// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Training Grounds' player count from the runtime's occupancy log, and the idle stop's policy (spec §7.3,
//! §7.4, Director patch 0033). Under Wine the pid in a line is not the Director's, so position decides what
//! belongs to the current process.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

const MAX_READ: u64 = 4 * 1024 * 1024;
const MAX_PARTIAL: usize = 65_536;

fn signed(text: &str) -> Option<(i64, &str)> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let (number, after) = rest.split_at(digits);
    let value: i64 = number.parse().ok()?;
    Some((if negative { -value } else { value }, after))
}

/// An edge line's new count: `Some(None)` when it is negative (unknown), none when the line is not an edge.
pub fn parse_edge(line: &str) -> Option<Option<i64>> {
    const MARK: &str = "[NetConnEdge] tick=";
    let at = line.find(MARK)?;
    let rest = &line[at + MARK.len()..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = rest[digits..].strip_prefix(" connCount ")?;
    let (_, rest) = signed(rest)?;
    let rest = rest.strip_prefix(" -> ")?;
    let (count, _) = signed(rest)?;
    Some((count >= 0).then_some(count))
}

/// Follows `{GAMESERVER_RUNTIME_LOG_DIR}/rustic_isles_dll_port{port}.log`.
#[derive(Debug)]
pub struct RuntimeLogOccupancy {
    path: PathBuf,
    position: u64,
    partial: String,
    count: Option<i64>,
}

impl RuntimeLogOccupancy {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            position: 0,
            partial: String::new(),
            count: None,
        }
    }

    /// Re-based just before the hub's process starts: what is in the file now is an earlier process's.
    pub fn begin(&mut self) {
        self.position = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        self.partial.clear();
        self.count = None;
    }

    /// The count after the new lines; a read error keeps the last one.
    pub fn poll(&mut self) -> Option<i64> {
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return self.count;
        };
        let Ok(size) = file.metadata().map(|m| m.len()) else {
            return self.count;
        };
        if size < self.position {
            self.position = 0;
            self.partial.clear();
        }
        if size == self.position {
            return self.count;
        }
        let mut start = self.position;
        if size - start > MAX_READ {
            start = size - MAX_READ;
            self.partial.clear();
        }
        let mut bytes = Vec::new();
        let read = file
            .seek(SeekFrom::Start(start))
            .and_then(|_| file.by_ref().take(size - start).read_to_end(&mut bytes));
        if read.is_err() {
            return self.count;
        }
        self.position = start + bytes.len() as u64;
        // Latin-1: one char per byte
        let mut text = std::mem::take(&mut self.partial);
        text.extend(bytes.iter().map(|&b| char::from(b)));
        let mut pieces: Vec<&str> = text.split('\n').collect();
        let last = pieces.pop().unwrap_or("");
        for line in pieces {
            if let Some(count) = parse_edge(line) {
                self.count = count;
            }
        }
        if last.chars().count() <= MAX_PARTIAL {
            self.partial = last.to_owned();
        }
        self.count
    }

    pub fn partial_len(&self) -> usize {
        self.partial.len()
    }
}

/// When an empty Training Grounds should stop.
#[derive(Debug, Clone)]
pub struct IdleStopPolicy {
    idle_ms: u64,
    idle_start: Option<u64>,
}

impl IdleStopPolicy {
    /// 0 never stops.
    pub fn new(idle_ms: u64) -> Self {
        Self {
            idle_ms,
            idle_start: None,
        }
    }

    pub fn reset(&mut self) {
        self.idle_start = None;
    }

    /// A player on the way in has no connection yet, so the last request also counts as activity.
    pub fn should_stop(&mut self, now: u64, count: Option<i64>, ready_at: u64, last_request_at: Option<u64>) -> bool {
        if count != Some(0) {
            self.idle_start = None;
            return false;
        }
        let start = *self.idle_start.get_or_insert(now);
        if self.idle_ms == 0 {
            return false;
        }
        let since = start.max(ready_at).max(last_request_at.unwrap_or(0));
        now.saturating_sub(since) >= self.idle_ms
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn edge(from: i64, to: i64) -> String {
        format!("[12:00:01.234] pid=292 [NetConnEdge] tick=1234 connCount {from} -> {to} connMax=16 NetDriver=0x1\n")
    }

    fn append(path: &std::path::Path, text: &str) {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn follows_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustic_isles_dll_port8776.log");
        let mut follower = RuntimeLogOccupancy::new(&path);
        assert_eq!(follower.poll(), None);
        append(&path, &(edge(0, 1) + &edge(1, 0)));
        follower.begin();
        assert_eq!(follower.poll(), None);
        append(&path, &format!("LogNet: something\n{}", edge(-2, 0)));
        assert_eq!(follower.poll(), Some(0));
        append(&path, &(edge(0, 1) + &edge(1, 2)));
        assert_eq!(follower.poll(), Some(2));
        let half = edge(1, 0);
        append(&path, &format!("{}{}", edge(2, 1), &half[..30]));
        assert_eq!(follower.poll(), Some(1));
        append(&path, &half[30..]);
        assert_eq!(follower.poll(), Some(0));
        assert_eq!(follower.poll(), Some(0));
        append(&path, &format!("{}\n{}", "x".repeat(200), edge(0, 2)));
        assert_eq!(follower.poll(), Some(2));
        std::fs::write(&path, edge(-2, 0)).unwrap();
        assert_eq!(follower.poll(), Some(0));
        append(&path, &edge(0, -1));
        assert_eq!(follower.poll(), None);
        assert_eq!(RuntimeLogOccupancy::new(dir.path().join("none")).poll(), None);
    }

    #[test]
    fn edges() {
        assert_eq!(parse_edge(&edge(0, 3)), Some(Some(3)));
        assert_eq!(parse_edge("[NetConnEdge] tick= connCount 0 -> 1"), None);
        assert_eq!(parse_edge("[NetConnEdge] tick=1 connCount x -> 1"), None);
        assert_eq!(parse_edge("[NetConnEdge] tick=1 connCount 0 -> -4trailing"), Some(None));
    }

    #[test]
    fn idle_policy() {
        let min = 60_000;
        let mut p = IdleStopPolicy::new(5 * min);
        assert!(!p.should_stop(0, Some(0), 0, None));
        assert!(!p.should_stop(4 * min, Some(0), 0, None));
        assert!(p.should_stop(5 * min, Some(0), 0, None));
        p.reset();
        assert!(!p.should_stop(12 * min, Some(0), 0, Some(12 * min)));
        assert!(!p.should_stop(16 * min, Some(0), 0, Some(12 * min)));
        assert!(p.should_stop(17 * min, Some(0), 0, Some(12 * min)));
        assert!(!p.should_stop(20 * min, Some(1), 0, Some(12 * min)));
        assert!(!p.should_stop(25 * min, None, 0, Some(12 * min)));
        assert!(!p.should_stop(31 * min, Some(0), 0, Some(12 * min)));
        assert!(p.should_stop(36 * min, Some(0), 0, Some(12 * min)));
        let mut never = IdleStopPolicy::new(0);
        assert!(!never.should_stop(0, Some(0), 0, None));
        assert!(!never.should_stop(u64::MAX, Some(0), 0, None));
    }
}
