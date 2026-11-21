// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The live set: canonical addresses with expiries, and its state file, so a restart restores who may play
//! but never more than one TTL from now.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::ip::{AllowlistCheck, ParsedIp, check_allowlist_ip, parse_ip};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Added,
    Refreshed,
    Full,
}

#[derive(Debug, Clone)]
struct Entry {
    expires_at: i64,
    version: u8,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AllowlistState {
    ttl_ms: i64,
    max: usize,
    entries: HashMap<String, Entry>,
}

impl AllowlistState {
    pub fn new(ttl_ms: i64, max: usize) -> Self {
        Self {
            ttl_ms,
            max,
            entries: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// An existing entry's expiry becomes now plus the TTL; a new one is added unless the set is full.
    pub fn refresh(&mut self, ip: &ParsedIp, now: i64) -> Refresh {
        let expires_at = now + self.ttl_ms;
        if let Some(entry) = self.entries.get_mut(&ip.canonical) {
            entry.expires_at = expires_at;
            return Refresh::Refreshed;
        }
        if self.entries.len() >= self.max {
            return Refresh::Full;
        }
        self.insert(ip, expires_at);
        Refresh::Added
    }

    fn insert(&mut self, ip: &ParsedIp, expires_at: i64) {
        self.entries.insert(
            ip.canonical.clone(),
            Entry {
                expires_at,
                version: ip.version,
                bytes: ip.bytes.clone(),
            },
        );
    }

    /// Removes and returns, sorted as text, every entry whose expiry is at or before `now`.
    pub fn expire(&mut self, now: i64) -> Vec<String> {
        let mut gone: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.expires_at <= now)
            .map(|(ip, _)| ip.clone())
            .collect();
        gone.sort();
        for ip in &gone {
            self.entries.remove(ip);
        }
        gone
    }

    fn ordered(&self) -> Vec<(&String, &Entry)> {
        let mut all: Vec<(&String, &Entry)> = self.entries.iter().collect();
        all.sort_by(|a, b| (a.1.version, &a.1.bytes).cmp(&(b.1.version, &b.1.bytes)));
        all
    }

    /// IPv4 before IPv6, each by the address's bytes, so the same set always gives the same rule.
    pub fn list(&self) -> Vec<String> {
        self.ordered().into_iter().map(|(ip, _)| ip.clone()).collect()
    }

    /// `(address, expiry)` in list order.
    pub fn entries(&self) -> Vec<(String, i64)> {
        self.ordered()
            .into_iter()
            .map(|(ip, e)| (ip.clone(), e.expires_at))
            .collect()
    }

    pub fn contains(&self, canonical: &str) -> bool {
        self.entries.contains_key(canonical)
    }

    pub fn next_expiry(&self) -> Option<i64> {
        self.entries.values().map(|e| e.expires_at).min()
    }

    pub fn to_json(&self) -> Value {
        let entries: Vec<Value> = self
            .entries()
            .into_iter()
            .map(|(ip, at)| json!({ "ip": ip, "expiresAt": at }))
            .collect();
        json!({ "version": 1, "entries": entries })
    }

    /// Written to `<file>.tmp` (0600 where there are modes) and renamed into place.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let mut tmp = file.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        std::io::Write::write_all(&mut options.open(&tmp)?, self.to_json().to_string().as_bytes())?;
        std::fs::rename(&tmp, file)
    }

    /// The entries of a saved state that still hold: a parsable address that passes the check, expiring after
    /// now, at most `max` of them, none later than now plus one TTL.
    pub fn restore(&mut self, saved: &Value, now: i64, allow_private: bool) -> usize {
        let Some(entries) = saved.get("entries").and_then(Value::as_array) else {
            return 0;
        };
        let mut restored = 0;
        for item in entries {
            if self.entries.len() >= self.max {
                break;
            }
            let Some(text) = item.get("ip").and_then(Value::as_str) else {
                continue;
            };
            let Some(expires_at) = item.get("expiresAt").and_then(Value::as_f64).filter(|v| v.is_finite()) else {
                continue;
            };
            if expires_at <= now as f64 || parse_ip(text).is_none() {
                continue;
            }
            let AllowlistCheck::Allowed(ip) = check_allowlist_ip(Some(text), allow_private) else {
                continue;
            };
            let expires_at = (expires_at as i64).min(now + self.ttl_ms);
            if !self.entries.contains_key(&ip.canonical) {
                restored += 1;
            }
            self.insert(&ip, expires_at);
        }
        restored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> ParsedIp {
        parse_ip(text).unwrap()
    }

    #[test]
    fn refresh_and_expire() {
        let mut s = AllowlistState::new(600_000, 2);
        assert_eq!(s.refresh(&ip("203.0.113.7"), 0), Refresh::Added);
        assert_eq!(s.refresh(&ip("203.0.113.7"), 1000), Refresh::Refreshed);
        assert_eq!(s.refresh(&ip("2001:db8::1"), 0), Refresh::Added);
        assert_eq!(s.refresh(&ip("198.51.100.2"), 0), Refresh::Full);
        assert!(s.expire(600_999).iter().all(|g| g != "203.0.113.7"));
        assert!(s.contains("203.0.113.7"));
        assert_eq!(s.expire(601_000), vec!["203.0.113.7".to_owned()]);
    }

    #[test]
    fn list_order() {
        let mut s = AllowlistState::new(1000, 10);
        for a in ["2001:db8::2", "203.0.113.7", "10.0.0.1", "2001:db8::1", "9.0.0.1"] {
            s.refresh(&ip(a), 0);
        }
        assert_eq!(
            s.list(),
            ["9.0.0.1", "10.0.0.1", "203.0.113.7", "2001:db8::1", "2001:db8::2"]
        );
    }

    #[test]
    fn restore_example() {
        let saved = json!({"version":1,"entries":[
            {"ip":"203.0.113.7","expiresAt":5_000_000},
            {"ip":"198.51.100.2","expiresAt":999},
            {"ip":"10.0.0.1","expiresAt":5_000_000},
            {"ip":"0.0.0.0/0","expiresAt":5_000_000},
            5,
            null
        ]});
        let mut s = AllowlistState::new(600_000, 256);
        assert_eq!(s.restore(&saved, 1000, false), 1);
        assert_eq!(s.entries(), vec![("203.0.113.7".to_owned(), 601_000)]);
        assert_eq!(AllowlistState::new(1, 1).restore(&json!({"entries":{}}), 0, true), 0);
    }

    #[test]
    fn save_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state.json");
        let mut s = AllowlistState::new(600_000, 10);
        s.refresh(&ip("203.0.113.7"), 0);
        s.save(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            text,
            r#"{"version":1,"entries":[{"ip":"203.0.113.7","expiresAt":600000}]}"#
        );
        assert!(!dir.path().join("state.json.tmp").exists());
    }
}
