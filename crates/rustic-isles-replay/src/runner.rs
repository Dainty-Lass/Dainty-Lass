// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The run (spec §9): targets, the HTTP and chat passes, classification and the report (§10.6).

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use axum::Router;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::ReplayError;
use crate::capture::{CaptureBody, CaptureRecord, ChatRecord, HttpRecord};
use crate::compare::{DiffKind, compare_chat_stanzas, compare_http_responses};
use crate::credentials::{ReplayTargetAuth, substitute_str};
use crate::ids::{IdMap, UUID, apply_ids_str, learn_ids};
use crate::normalize::{IdNormalizer, NormalizerConfig};
use crate::request::{FALLBACK_NOW_MS, PreparedHttpRequest, ReplayHttpResponse, prepare_http_request};
use crate::transport::{dispatch_router_request, send_http_request};

/// Where a side's answers come from (§6.2).
#[derive(Clone, Default)]
pub enum ReplayEndpoint {
    /// The capture's own answers
    #[default]
    Recorded,
    Http {
        base_url: String,
        director_url: Option<String>,
        /// `host:port` of a plain XMPP-over-TCP listener
        chat_addr: Option<String>,
    },
    /// In-process routers, each served on a fresh loopback listener per request
    Router { backend: Router, director: Option<Router> },
}

impl std::fmt::Debug for ReplayEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recorded => f.write_str("Recorded"),
            Self::Http {
                base_url,
                director_url,
                chat_addr,
            } => f
                .debug_struct("Http")
                .field("base_url", base_url)
                .field("director_url", director_url)
                .field("chat_addr", chat_addr)
                .finish(),
            Self::Router { director, .. } => f
                .debug_struct("Router")
                .field("director", &director.is_some())
                .finish_non_exhaustive(),
        }
    }
}

impl ReplayEndpoint {
    fn is_recorded(&self) -> bool {
        matches!(self, Self::Recorded)
    }

    fn reaches_director(&self) -> bool {
        match self {
            Self::Recorded => true,
            Self::Http { director_url, .. } => director_url.is_some(),
            Self::Router { director, .. } => director.is_some(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplayTarget {
    pub name: String,
    pub endpoint: ReplayEndpoint,
    pub auth: ReplayTargetAuth,
    /// Send core's replay headers; always off for a recorded side
    pub pin_clock: bool,
}

impl ReplayTarget {
    pub fn recorded() -> Self {
        Self {
            name: "recorded".into(),
            endpoint: ReplayEndpoint::Recorded,
            auth: ReplayTargetAuth::default(),
            pin_clock: false,
        }
    }

    pub fn http(name: impl Into<String>, base_url: impl Into<String>, auth: ReplayTargetAuth) -> Self {
        Self {
            name: name.into(),
            endpoint: ReplayEndpoint::Http {
                base_url: base_url.into(),
                director_url: None,
                chat_addr: None,
            },
            auth,
            pin_clock: true,
        }
    }

    pub fn router(name: impl Into<String>, backend: Router, director: Option<Router>, auth: ReplayTargetAuth) -> Self {
        Self {
            name: name.into(),
            endpoint: ReplayEndpoint::Router { backend, director },
            auth,
            pin_clock: true,
        }
    }

    fn pins(&self) -> bool {
        self.pin_clock && !self.endpoint.is_recorded()
    }
}

#[derive(Debug, Clone)]
pub struct ReplayConfig {
    pub left: ReplayTarget,
    pub right: ReplayTarget,
    pub normalizer: NormalizerConfig,
    /// Keeps HTTP records whose raw target or route contains it
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Http,
    Chat,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayEntryResult {
    pub index: usize,
    pub kind: EntryKind,
    pub conn: u64,
    pub seq: u64,
    pub method: String,
    pub target: String,
    pub route: String,
    pub inexact: bool,
    pub left_status: Option<u16>,
    pub right_status: Option<u16>,
    pub diffs: Vec<DiffKind>,
}

/// §10.6, fields in the report's order.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReplayReport {
    pub total_records: usize,
    pub http_replayed: usize,
    pub chat_connections_replayed: usize,
    pub skipped_records: usize,
    pub exact_matched: usize,
    pub inexact_matched: usize,
    pub inexact_diffed: usize,
    pub unexplained_diffed: usize,
    pub status_diffs: usize,
    pub header_diffs: usize,
    pub value_diffs: usize,
    pub text_diffs: usize,
    pub chat_stanza_diffs: usize,
    pub duration_ms: u64,
    pub entries: Vec<ReplayEntryResult>,
    /// Printed to stderr as they happen; not part of the JSON report
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl ReplayReport {
    /// No entry differs without an explanation.
    pub fn passed(&self) -> bool {
        self.unexplained_diffed == 0
    }

    fn record(&mut self, entry: ReplayEntryResult) {
        match (entry.diffs.is_empty(), entry.inexact) {
            (true, false) => self.exact_matched += 1,
            (true, true) => self.inexact_matched += 1,
            (false, true) => self.inexact_diffed += 1,
            (false, false) => self.unexplained_diffed += 1,
        }
        for diff in &entry.diffs {
            match diff {
                DiffKind::StatusDiff { .. } => self.status_diffs += 1,
                DiffKind::HeaderDiff { .. } => self.header_diffs += 1,
                DiffKind::ValueDiff { .. } => self.value_diffs += 1,
                DiffKind::TextDiff { .. } => self.text_diffs += 1,
                DiffKind::ChatStanzaDiff { .. } => self.chat_stanza_diffs += 1,
            }
        }
        self.entries.push(entry);
    }

    /// Pretty JSON, two-space indent, no trailing newline.
    pub fn to_json_pretty(&self) -> Result<String, ReplayError> {
        serde_json::to_string_pretty(self).map_err(|e| ReplayError::Json(e.to_string()))
    }

    /// The text form of §10.6 under `prefix` (`isles-db replay:`, `reforged-host db replay:`).
    pub fn to_text(&self, prefix: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{prefix} {} (records={}, http={}, chat_conns={}, skipped={}, exact_matched={}, inexact_matched={}, \
             inexact_diffed={}, unexplained_diffed={})",
            if self.passed() { "PASS" } else { "DIFF" },
            self.total_records,
            self.http_replayed,
            self.chat_connections_replayed,
            self.skipped_records,
            self.exact_matched,
            self.inexact_matched,
            self.inexact_diffed,
            self.unexplained_diffed
        );
        let _ = writeln!(
            out,
            "  diffs by kind: status={}, header={}, value={}, text={}, chat_stanza={}",
            self.status_diffs, self.header_diffs, self.value_diffs, self.text_diffs, self.chat_stanza_diffs
        );
        for entry in self.entries.iter().filter(|e| !e.diffs.is_empty()) {
            let _ = writeln!(
                out,
                "  [{}] #{} {} {} (conn={}, seq={}, route={}): {} diff(s)",
                if entry.inexact { "INEXACT" } else { "DIFF" },
                entry.index,
                entry.method,
                entry.target,
                entry.conn,
                entry.seq,
                entry.route,
                entry.diffs.len()
            );
            for diff in &entry.diffs {
                let _ = writeln!(out, "    - {diff:?}");
            }
        }
        out
    }
}

/// Seeded ids a pinned target would mint for `seed` (core §9.1), to tell whether it honoured the headers.
fn seeded_ids(seed: &str, count: u64) -> HashSet<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"isles-replay-seed:");
    hasher.update(seed.as_bytes());
    let key: [u8; 32] = hasher.finalize().into();
    (0..count)
        .map(|n| {
            let mut hasher = Sha256::new();
            hasher.update(key);
            hasher.update(n.to_le_bytes());
            let digest: [u8; 32] = hasher.finalize().into();
            let mut b = [0u8; 16];
            b.copy_from_slice(&digest[..16]);
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            format!(
                "{}-{}-{}-{}-{}",
                &hex[0..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..32]
            )
        })
        .collect()
}

const SEEDED_IDS_CHECKED: u64 = 64;

struct Side<'a> {
    target: &'a ReplayTarget,
    ids: IdMap,
    normalizer: IdNormalizer,
    /// Pinning still to be confirmed by an answer that mints an id (decision on §16 question 1)
    pin_unconfirmed: bool,
}

impl<'a> Side<'a> {
    fn new(target: &'a ReplayTarget, config: NormalizerConfig) -> Self {
        Self {
            target,
            ids: IdMap::new(),
            normalizer: IdNormalizer::new(config),
            pin_unconfirmed: target.pins(),
        }
    }

    fn is_live(&self) -> bool {
        !self.target.endpoint.is_recorded()
    }

    async fn send(
        &self,
        record: &HttpRecord,
        request: &PreparedHttpRequest,
    ) -> Result<ReplayHttpResponse, ReplayError> {
        let director = record.route == "director";
        match &self.target.endpoint {
            ReplayEndpoint::Recorded => Ok(ReplayHttpResponse::from_recorded(record)),
            ReplayEndpoint::Http {
                base_url, director_url, ..
            } => {
                let base = director.then_some(director_url.as_ref()).flatten().unwrap_or(base_url);
                send_http_request(base, request).await
            }
            ReplayEndpoint::Router { backend, director: d } => {
                let router = director.then_some(d.as_ref()).flatten().unwrap_or(backend);
                dispatch_router_request(router, request).await
            }
        }
    }

    /// Answers one record; a live answer teaches ids from the recorded one.
    async fn answer(
        &mut self,
        record: &HttpRecord,
        report: &mut ReplayReport,
    ) -> Result<(ReplayHttpResponse, bool), ReplayError> {
        if !self.is_live() {
            let inexact = record.aborted || record.req_body.is_lossy() || record.res_body.is_lossy();
            return Ok((ReplayHttpResponse::from_recorded(record), inexact));
        }
        let request = prepare_http_request(record, &self.target.auth, &self.ids, self.target.pins());
        let answer = self.send(record, &request).await?;
        if self.pin_unconfirmed {
            self.check_pinning(record, &request, &answer, report);
        }
        if let CaptureBody::Json(recorded) = &record.res_body
            && let Ok(live) = serde_json::from_str::<Value>(&answer.body_text)
        {
            learn_ids(recorded, &live, &mut self.ids);
        }
        Ok((answer, request.inexact))
    }

    /// The first answer carrying an id this request could have minted shows whether the target pinned it: a
    /// seeded id means yes; only unknown ids mean it ignored the headers (`RUSTIC_ISLES_REPLAY_HEADERS` off).
    fn check_pinning(
        &mut self,
        record: &HttpRecord,
        request: &PreparedHttpRequest,
        answer: &ReplayHttpResponse,
        report: &mut ReplayReport,
    ) {
        let mut known = String::new();
        known.push_str(&request.target);
        for (_, v) in &request.headers {
            known.push_str(v);
        }
        known.push_str(&String::from_utf8_lossy(&request.body));
        known.push_str(&ReplayHttpResponse::from_recorded(record).body_text);
        for live in self.ids.values() {
            known.push_str(live);
        }
        let known = known.to_ascii_lowercase();
        let mut answer_text = answer.body_text.clone();
        for v in answer.headers.values() {
            answer_text.push_str(v);
        }
        let fresh: Vec<String> = UUID
            .find_iter(&answer_text)
            .map(|m| m.as_str().to_ascii_lowercase())
            .filter(|id| !known.contains(id.as_str()))
            .collect();
        if fresh.is_empty() {
            return;
        }
        self.pin_unconfirmed = false;
        let seed = format!("{}:{}:{}:{}", record.conn, record.seq, record.method, record.target);
        let seeded = seeded_ids(&seed, SEEDED_IDS_CHECKED);
        if fresh.iter().any(|id| seeded.contains(id)) {
            return;
        }
        let warning = format!(
            "warning: replay target {} answered with ids its replay seed does not make; it appears to ignore the \
             x-isles-replay-* headers (is RUSTIC_ISLES_REPLAY_HEADERS on?), so its times and ids are not pinned",
            self.target.name
        );
        eprintln!("{warning}");
        report.warnings.push(warning);
    }

    /// One chat connection's server stanzas on this side, and whether they are inexact (§9.3).
    async fn chat(&self, records: &[&ChatRecord]) -> Result<(Vec<String>, bool), ReplayError> {
        let mut inexact = records.iter().any(|r| r.omitted.is_some() || r.compressed.is_some());
        let chat_addr = match &self.target.endpoint {
            ReplayEndpoint::Http {
                chat_addr: Some(addr), ..
            } => Some(addr),
            _ => None,
        };
        let substitute = |record: &ChatRecord, text: &str| {
            let now = record.timestamp_ms().unwrap_or(FALLBACK_NOW_MS);
            substitute_str(&apply_ids_str(text, &self.ids), &self.target.auth, now)
        };
        fn incoming(r: &&&ChatRecord) -> bool {
            r.dir.as_deref() == Some("in") && r.text.is_some()
        }
        let Some(addr) = chat_addr else {
            for record in records.iter().filter(incoming) {
                inexact |= substitute(record, record.text.as_deref().unwrap_or_default()).inexact;
            }
            let stanzas = records
                .iter()
                .filter(|r| r.dir.as_deref() == Some("out"))
                .filter_map(|r| r.text.as_deref())
                .map(|t| apply_ids_str(t, &self.ids))
                .collect();
            return Ok((stanzas, inexact));
        };
        let fail = |e: std::io::Error| ReplayError::Transport {
            target: format!("chat://{addr}"),
            message: e.to_string(),
        };
        let mut stream = TcpStream::connect(addr.as_str()).await.map_err(fail)?;
        let mut stanzas = Vec::new();
        let mut buffer = vec![0u8; 8192];
        for record in records.iter().filter(incoming) {
            let sub = substitute(record, record.text.as_deref().unwrap_or_default());
            inexact |= sub.inexact;
            stream.write_all(sub.value.as_bytes()).await.map_err(fail)?;
            // One read per frame: a server that answers in two writes, or slowly, loses stanzas (§15 item 17)
            if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(50), stream.read(&mut buffer)).await
                && n > 0
            {
                stanzas.push(String::from_utf8_lossy(&buffer[..n]).into_owned());
            }
        }
        Ok((stanzas, inexact))
    }
}

fn skip_http(record: &HttpRecord, config: &ReplayConfig) -> bool {
    if let Some(filter) = &config.filter
        && !record.target.contains(filter.as_str())
        && !record.route.contains(filter.as_str())
    {
        return true;
    }
    if record.aborted || record.route == "content" || record.route == "network" {
        return true;
    }
    // Only the right side's Director is checked (§15 item 13)
    record.route == "director" && !config.right.endpoint.reaches_director()
}

/// Replays `records` (in corpus order) and classifies every entry. A transport error ends the run (§15 item 21).
pub async fn run_replay(records: &[CaptureRecord], config: &ReplayConfig) -> Result<ReplayReport, ReplayError> {
    let started = Instant::now();
    let mut report = ReplayReport {
        total_records: records.len(),
        ..ReplayReport::default()
    };
    let mut left = Side::new(&config.left, config.normalizer);
    let mut right = Side::new(&config.right, config.normalizer);
    let mut chats: BTreeMap<u64, Vec<&ChatRecord>> = BTreeMap::new();

    for (index, record) in records.iter().enumerate() {
        let record = match record {
            CaptureRecord::Http(record) => record,
            CaptureRecord::Chat(chat) => {
                // Kept only when the filter text itself names chat (§15 item 11)
                if config.filter.as_ref().is_some_and(|f| !f.contains("chat")) {
                    report.skipped_records += 1;
                } else {
                    chats.entry(chat.conn).or_default().push(chat);
                }
                continue;
            }
        };
        if skip_http(record, config) {
            report.skipped_records += 1;
            continue;
        }
        report.http_replayed += 1;
        let (left_answer, left_inexact) = left.answer(record, &mut report).await?;
        let (right_answer, right_inexact) = right.answer(record, &mut report).await?;
        // Nothing recorded to learn from: the right follows the left's ids
        if left.is_live()
            && record.res_body == CaptureBody::Null
            && let (Ok(l), Ok(r)) = (
                serde_json::from_str::<Value>(&left_answer.body_text),
                serde_json::from_str::<Value>(&right_answer.body_text),
            )
        {
            learn_ids(&l, &r, &mut right.ids);
        }
        let diffs = compare_http_responses(&left_answer, &right_answer, &mut left.normalizer, &mut right.normalizer);
        report.record(ReplayEntryResult {
            index,
            kind: EntryKind::Http,
            conn: record.conn,
            seq: record.seq,
            method: record.method.clone(),
            target: record.target.clone(),
            route: record.route.clone(),
            inexact: left_inexact || right_inexact,
            left_status: Some(left_answer.status),
            right_status: Some(right_answer.status),
            diffs,
        });
    }

    for (conn, records) in &chats {
        report.chat_connections_replayed += 1;
        let (left_stanzas, left_inexact) = left.chat(records).await?;
        let (right_stanzas, right_inexact) = right.chat(records).await?;
        let index = report.entries.len();
        report.record(ReplayEntryResult {
            index,
            kind: EntryKind::Chat,
            conn: *conn,
            seq: 0,
            method: "XMPP".into(),
            target: format!("chat:conn:{conn}"),
            route: "chat".into(),
            inexact: left_inexact || right_inexact,
            left_status: None,
            right_status: None,
            diffs: compare_chat_stanzas(*conn, &left_stanzas, &right_stanzas),
        });
    }

    report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_ids_are_v4() {
        let ids = seeded_ids("1:1:GET:/x", 2);
        assert_eq!(ids.len(), 2);
        for id in ids {
            assert_eq!(id.len(), 36);
            assert_eq!(id.as_bytes()[14], b'4');
        }
    }
}
