// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! CLEAN-ROOM STUB (RD27): stands in for the Category A `moderation.rs` (the staff dashboard's filter, chat log and
//! reports), which the integration pass copies in unchanged. Its API is the rewrite's reading of spec-rd27-chat
//! §2.10.3, §2.11 and §2.13.8. The stub's dashboard is a local queue, so the rewrite's tests can see what reached it.

use std::sync::{Arc, Mutex, PoisonError};

use module_api::chat::RoomKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardSettings {
    pub url: String,
    pub ingest_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Direct,
    City,
    Party,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::City => "city",
            Self::Party => "party",
        }
    }
}

pub fn channel_of(kind: &RoomKind) -> Channel {
    match kind {
        RoomKind::Network | RoomKind::Region(_) => Channel::City,
        RoomKind::Party(_) | RoomKind::Hunt(_) => Channel::Party,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Masked,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub user_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    pub at: i64,
    pub channel: &'static str,
    pub room_id: Option<String>,
    pub sender: Person,
    pub recipient: Option<Person>,
    pub body: String,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPerson {
    pub user_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportContext {
    pub room_id: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: String,
    pub source: String,
    pub reporter: Person,
    pub target: TargetPerson,
    pub category: String,
    pub description: String,
    pub context: ReportContext,
}

/// A `/report` command's parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReport {
    pub target: String,
    pub category: String,
    pub description: String,
}

/// None: not a report command. `Err`: one, badly formed.
pub fn parse_report(body: &str) -> Option<Result<ParsedReport, String>> {
    let rest = body.trim().strip_prefix("/report")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut words = rest.split_whitespace();
    let (Some(target), Some(first)) = (words.next(), words.next()) else {
        return Some(Err(String::from("usage")));
    };
    let rest: Vec<&str> = words.collect();
    if rest.is_empty() {
        return Some(Ok(ParsedReport {
            target: target.to_owned(),
            category: String::from("other"),
            description: first.to_owned(),
        }));
    }
    Some(Ok(ParsedReport {
        target: target.to_owned(),
        category: first.to_owned(),
        description: rest.join(" "),
    }))
}

#[derive(Default)]
struct Queue {
    lines: Vec<Line>,
    reports: Vec<Report>,
    /// Words the stub's filter blocks
    blocked: Vec<String>,
}

#[derive(Clone, Default)]
pub struct Moderation {
    queue: Option<Arc<Mutex<Queue>>>,
}

impl Moderation {
    pub fn off() -> Self {
        Self::default()
    }

    pub fn start(_settings: DashboardSettings) -> Self {
        Self {
            queue: Some(Arc::default()),
        }
    }

    pub fn is_on(&self) -> bool {
        self.queue.is_some()
    }

    fn with<T>(&self, f: impl FnOnce(&mut Queue) -> T) -> Option<T> {
        self.queue
            .as_ref()
            .map(|q| f(&mut q.lock().unwrap_or_else(PoisonError::into_inner)))
    }

    /// The verdict and the text shown.
    pub fn rate(&self, body: &str, _channel: Channel) -> (Verdict, String) {
        let blocked = self
            .with(|q| q.blocked.iter().any(|word| body.contains(word.as_str())))
            .unwrap_or(false);
        if blocked {
            (Verdict::Blocked, body.to_owned())
        } else {
            (Verdict::Allowed, body.to_owned())
        }
    }

    pub fn record(&self, line: Line) {
        self.with(|q| q.lines.push(line));
    }

    pub async fn report(&self, report: Report) -> bool {
        self.with(|q| q.reports.push(report)).is_some()
    }

    /// The stub's own, for tests.
    pub fn block_word(&self, word: &str) {
        self.with(|q| q.blocked.push(word.to_owned()));
    }

    pub fn lines(&self) -> Vec<Line> {
        self.with(|q| q.lines.clone()).unwrap_or_default()
    }

    pub fn reports(&self) -> Vec<Report> {
        self.with(|q| q.reports.clone()).unwrap_or_default()
    }
}
