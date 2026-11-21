// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! One player's travel attempt (spec §5.1) and Node's object for it, which the realms side exchanges.

use serde_json::{Map, Value};

/// Where a candidate came from; only the realms' hubs are marked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSource {
    /// A hub a realm runs, whose status is read from its hub session.
    RealmHub,
}

impl CandidateSource {
    /// The value the candidate's JSON carries (`REALM_HUB`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RealmHub => "REALM_HUB",
        }
    }
}

/// One player's matchmaking search, as the status route answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchCandidate {
    /// Internal server readiness; the client also waits for `ready_at`
    pub ready: bool,
    /// Whether the search was cancelled; `status_reason` says why.
    pub canceled: bool,
    /// Why the search ended without a server, such as `PLAYER_CANCELED`.
    pub status_reason: Option<String>,
    /// The hunt searched for.
    pub hunt_id: String,
    /// This client's travel attempt; party members sharing an instance each get their own.
    pub candidate_id: String,
    /// The game session the client joins.
    pub game_session_id: String,
    /// The game server's address; empty until it is ready.
    pub host: String,
    /// The game server's port; 0 until it is ready.
    pub port: i64,
    /// Unix ms of the Go Now release; 0 is none
    pub ready_at: i64,
    /// Set only for a realm hub.
    pub source: Option<CandidateSource>,
    /// The hub session a realm hub's status is read from
    pub source_session_id: Option<String>,
}

impl MatchCandidate {
    /// A search for `hunt_id` under `id` (candidate and game session alike).
    pub fn matching(hunt_id: &str, id: &str) -> Self {
        Self {
            ready: false,
            canceled: false,
            status_reason: None,
            hunt_id: hunt_id.to_owned(),
            candidate_id: id.to_owned(),
            game_session_id: id.to_owned(),
            host: String::new(),
            port: 0,
            ready_at: 0,
            source: None,
            source_session_id: None,
        }
    }

    /// Whether the candidate is a realm's hub.
    pub fn is_realm_hub(&self) -> bool {
        self.source == Some(CandidateSource::RealmHub)
    }

    /// Whether the client may travel at `now` (Unix ms): the server is ready and the Go Now release has passed.
    pub fn is_client_ready(&self, now: i64) -> bool {
        self.ready && (self.ready_at == 0 || now >= self.ready_at)
    }

    /// A copy whose `ready` is what the client may see now.
    pub fn client_visible(&self, now: i64) -> Self {
        Self {
            ready: self.is_client_ready(now),
            ..self.clone()
        }
    }

    /// 10 s once released: the client otherwise waits its default 10 s before its first poll.
    pub fn status_period_millis(&self, now: i64) -> i64 {
        if !self.ready || self.ready_at > now {
            1000
        } else {
            10_000
        }
    }

    /// Whether the ring placed it (DN6, DN9).
    pub fn is_ring_assigned(&self) -> bool {
        self.game_session_id.starts_with("hunt:") || self.game_session_id.starts_with("hub_instance:")
    }

    /// Node's object, keys in its order.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("Ready".into(), self.ready.into());
        if self.canceled {
            out.insert("Canceled".into(), true.into());
        }
        if let Some(reason) = &self.status_reason {
            out.insert("StatusReason".into(), reason.clone().into());
        }
        out.insert("HuntId".into(), self.hunt_id.clone().into());
        out.insert("CandidateId".into(), self.candidate_id.clone().into());
        out.insert("GameSessionId".into(), self.game_session_id.clone().into());
        out.insert("Host".into(), self.host.clone().into());
        out.insert("Port".into(), self.port.into());
        out.insert("ReadyAt".into(), self.ready_at.into());
        if let Some(source) = self.source {
            out.insert("Source".into(), source.as_str().into());
        }
        if let Some(session) = &self.source_session_id {
            out.insert("SourceSessionId".into(), session.clone().into());
        }
        Value::Object(out)
    }

    /// Node's object as text, as `JSON.stringify` writes it.
    pub fn to_json_text(&self) -> String {
        dainty_service_kit::jsjson::stringify(&self.to_json())
    }

    /// None unless an object with string `HuntId`, `CandidateId` and `GameSessionId`.
    pub fn from_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let text = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_owned);
        let flag = |name: &str| object.get(name).and_then(Value::as_bool).unwrap_or(false);
        // `7777.0` is not an integer here, so it reads as 0; a huge one rounds as `JSON.parse` would, so it writes back
        let integer = |name: &str| {
            let n = object.get(name).and_then(Value::as_i64).unwrap_or(0) as f64;
            if n.abs() < 9.223_372_036_854_775e18 {
                n as i64
            } else {
                0
            }
        };
        Some(Self {
            ready: flag("Ready"),
            canceled: flag("Canceled"),
            status_reason: text("StatusReason"),
            hunt_id: text("HuntId")?,
            candidate_id: text("CandidateId")?,
            game_session_id: text("GameSessionId")?,
            host: text("Host").unwrap_or_default(),
            port: integer("Port"),
            ready_at: integer("ReadyAt"),
            source: (object.get("Source").and_then(Value::as_str) == Some("REALM_HUB"))
                .then_some(CandidateSource::RealmHub),
            source_session_id: text("SourceSessionId"),
        })
    }

    /// The realms side's text: none when it does not parse or is not a candidate.
    pub fn from_json_text(text: &str) -> Option<Self> {
        Self::from_json(&serde_json::from_str(text).ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_object_round_trips() {
        let mut candidate = MatchCandidate::matching("ShatteredIsles_IslandA", "c1");
        candidate.ready = true;
        candidate.host = "203.0.113.10".into();
        candidate.port = 7777;
        candidate.ready_at = 5;
        candidate.source = Some(CandidateSource::RealmHub);
        candidate.source_session_id = Some("s".into());
        let text = candidate.to_json_text();
        assert_eq!(
            text,
            r#"{"Ready":true,"HuntId":"ShatteredIsles_IslandA","CandidateId":"c1","GameSessionId":"c1","Host":"203.0.113.10","Port":7777,"ReadyAt":5,"Source":"REALM_HUB","SourceSessionId":"s"}"#
        );
        assert_eq!(MatchCandidate::from_json_text(&text), Some(candidate));
        assert_eq!(MatchCandidate::from_json_text(r#"{"Ready":true}"#), None);
        let cancelled = MatchCandidate {
            canceled: true,
            status_reason: Some("WHY".into()),
            ..MatchCandidate::matching("h", "c")
        };
        assert_eq!(
            cancelled.to_json_text(),
            r#"{"Ready":false,"Canceled":true,"StatusReason":"WHY","HuntId":"h","CandidateId":"c","GameSessionId":"c","Host":"","Port":0,"ReadyAt":0}"#
        );
    }

    #[test]
    fn loose_fields_take_their_defaults() {
        let read = MatchCandidate::from_json_text(
            r#"{"HuntId":"h","CandidateId":"c","GameSessionId":"g","Port":7777.0,"ReadyAt":"5","Ready":1,"Source":"realm_hub","Host":3}"#,
        )
        .unwrap();
        assert_eq!((read.port, read.ready_at, read.ready, read.source), (0, 0, false, None));
        assert_eq!(read.host, "");
        let huge = MatchCandidate::from_json_text(
            r#"{"HuntId":"h","CandidateId":"c","GameSessionId":"g","Port":1152921504606846977}"#,
        )
        .unwrap();
        assert_eq!(huge.port, 1_152_921_504_606_846_976);
        assert_eq!(MatchCandidate::from_json_text(&huge.to_json_text()), Some(huge));
        assert_eq!(MatchCandidate::from_json_text("[1]"), None);
        assert_eq!(MatchCandidate::from_json_text("{"), None);
    }

    #[test]
    fn periods_and_release() {
        let mut c = MatchCandidate::matching("h", "c");
        assert_eq!(c.status_period_millis(100), 1000);
        c.ready = true;
        c.ready_at = 200;
        assert_eq!(c.status_period_millis(100), 1000);
        assert!(!c.client_visible(100).ready);
        assert_eq!(c.status_period_millis(200), 10_000);
        assert!(c.client_visible(200).ready);
        c.ready_at = 0;
        assert_eq!(c.status_period_millis(0), 10_000);
    }
}
