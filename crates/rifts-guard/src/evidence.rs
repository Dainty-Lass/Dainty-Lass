// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Hunt Guard evidence: the batch and completion bodies the host's transport posts, and how each moves a chain.

use std::collections::BTreeSet;

use rifts_seam::{AccountId, Json, SessionId, UnixMillis};
use serde_json::{Map, Value};

use crate::digest::{hex_of_parts, is_hex64};
use crate::errors::GuardError;
use crate::json::{js_string, non_negative, safe_integer, stringify};
use crate::store::{
    BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition, EncounterState, FindingRecord,
};

pub const BATCH_SCHEMA: i64 = 1;
pub const MAX_BATCH_EVENTS: usize = 128;
pub const MAX_PAYLOAD_BYTES: usize = 8192;
pub const MAX_BATCH_EVENT_BYTES: u64 = 65_536;
pub const MAX_CHAIN_EVENTS: u64 = 262_144;
pub const MAX_CHAIN_EVENT_BYTES: u64 = 67_108_864;

/// Prefixed to the body's canonical text before hashing; a database's chains carry hashes made with it.
const BATCH_HASH_LABEL: &[u8] = b"rifts-guard-batch-v1\n";

const BATCH_KEYS: [&str; 6] = [
    "schema",
    "batchId",
    "firstSequence",
    "lastSequence",
    "previousHash",
    "events",
];
const EVENT_KEYS: [&str; 5] = ["sequence", "monotonicMs", "type", "actorAccountId", "payload"];
const COMPLETION_KEYS: [&str; 2] = ["finalSequence", "finalHash"];

pub(crate) const BATCH_INVALID: &str = "The Guard event batch is invalid.";
pub(crate) const TOO_LARGE: &str = "The Guard request body exceeds its bounded limit.";
const FROZEN: &str = "The Guard chain of this hunt generation is closed.";
pub(crate) const CONFLICT: &str = "The Guard batch does not continue this hunt generation's chain.";
const LIMIT: &str = "This hunt generation's Guard evidence limit is reached.";
const COMPLETION_INVALID: &str = "The Guard completion request is invalid.";
pub(crate) const NOT_REQUESTED: &str = "The Guard chain has not requested completion.";
const NO_COMPLETION: &str = "This hunt generation has no Guard completion.";
const COMPLETION_CONFLICT: &str = "The Guard completion does not match this hunt generation's chain.";

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceEvent {
    pub sequence: u64,
    pub monotonic_ms: u64,
    pub kind: String,
    pub actor: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceBatch {
    pub batch_id: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub previous_hash: String,
    pub events: Vec<EvidenceEvent>,
    pub event_bytes: u64,
    pub batch_hash: String,
}

/// Why a batch body was refused; the text is a diagnostic only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchProblem {
    TooLarge,
    Invalid(&'static str),
}

impl BatchProblem {
    pub fn into_error(self) -> GuardError {
        match self {
            Self::TooLarge => GuardError::refused("GUARD_BATCH_TOO_LARGE", TOO_LARGE),
            Self::Invalid(_) => GuardError::refused("GUARD_BATCH_INVALID", BATCH_INVALID),
        }
    }
}

/// A refusal of a batch, with the finding it leaves on the chain when it leaves one: `(code, detail)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRefusal {
    pub error: GuardError,
    pub finding: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    pub final_sequence: u64,
    pub final_hash: String,
}

fn keys_within(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

fn token(value: &Value, max: usize, extra: &[u8]) -> Option<String> {
    let text = value.as_str()?;
    let ok = (1..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b));
    ok.then(|| text.to_owned())
}

fn invalid<T>(why: &'static str) -> Result<T, BatchProblem> {
    Err(BatchProblem::Invalid(why))
}

fn is_terminal_type(kind: &str) -> bool {
    matches!(kind, "COMPLETION_REQUESTED" | "HUNT_ABORTED")
}

/// Reads one batch body; the hash covers the whole body as parsed.
pub fn parse_batch(body: Option<&Value>) -> Result<EvidenceBatch, BatchProblem> {
    let Some(Value::Object(object)) = body else {
        return invalid("not an object");
    };
    if !keys_within(object, &BATCH_KEYS) {
        return invalid("unknown batch field");
    }
    let field = |name: &str| object.get(name).unwrap_or(&Value::Null);
    if safe_integer(field("schema")) != Some(BATCH_SCHEMA) {
        return invalid("schema");
    }
    let Some(batch_id) = token(field("batchId"), 64, b"-") else {
        return invalid("batchId");
    };
    let previous_hash = match field("previousHash").as_str() {
        Some(hash) if is_hex64(hash) => hash.to_owned(),
        _ => return invalid("previousHash"),
    };
    let Some(first_sequence) = non_negative(field("firstSequence")).filter(|n| *n >= 1) else {
        return invalid("firstSequence");
    };
    let Some(last_sequence) = non_negative(field("lastSequence")) else {
        return invalid("lastSequence");
    };
    let Value::Array(raw_events) = field("events") else {
        return invalid("events");
    };
    if raw_events.is_empty() {
        return invalid("no events");
    }
    if raw_events.len() > MAX_BATCH_EVENTS {
        return Err(BatchProblem::TooLarge);
    }
    let mut events = Vec::with_capacity(raw_events.len());
    let mut event_bytes: u64 = 0;
    for (index, raw) in raw_events.iter().enumerate() {
        let Value::Object(event) = raw else {
            return invalid("event not an object");
        };
        if !keys_within(event, &EVENT_KEYS) {
            return invalid("unknown event field");
        }
        let get = |name: &str| event.get(name).unwrap_or(&Value::Null);
        let sequence = non_negative(get("sequence"));
        if sequence != first_sequence.checked_add(index as u64) {
            return invalid("sequence");
        }
        let Some(monotonic_ms) = non_negative(get("monotonicMs")) else {
            return invalid("monotonicMs");
        };
        if events
            .last()
            .is_some_and(|prev: &EvidenceEvent| monotonic_ms < prev.monotonic_ms)
        {
            return invalid("monotonicMs regressed");
        }
        let Some(kind) = token(get("type"), 64, b"_") else {
            return invalid("type");
        };
        let actor = match event.get("actorAccountId") {
            None => None,
            Some(value) => match token(value, 96, b"._:-") {
                Some(actor) => Some(actor),
                None => return invalid("actorAccountId"),
            },
        };
        let payload = match event.get("payload") {
            Some(payload @ Value::Object(_)) if stringify(payload).len() <= MAX_PAYLOAD_BYTES => payload.clone(),
            _ => return invalid("payload"),
        };
        if is_terminal_type(&kind) && index + 1 != raw_events.len() {
            return invalid("terminal event not last");
        }
        event_bytes += stringify(raw).len() as u64;
        events.push(EvidenceEvent {
            sequence: first_sequence + index as u64,
            monotonic_ms,
            kind,
            actor,
            payload,
        });
    }
    if event_bytes > MAX_BATCH_EVENT_BYTES {
        return Err(BatchProblem::TooLarge);
    }
    if events.last().map(|e| e.sequence) != Some(last_sequence) {
        return invalid("lastSequence");
    }
    let canonical = stringify(&Value::Object(object.clone()));
    Ok(EvidenceBatch {
        batch_id,
        first_sequence,
        last_sequence,
        previous_hash,
        events,
        event_bytes,
        batch_hash: hex_of_parts(&[BATCH_HASH_LABEL, canonical.as_bytes()]),
    })
}

fn chain_refusal(code: &str, message: &str, finding: Option<(&str, String)>) -> ChainRefusal {
    ChainRefusal {
        error: GuardError::refused(code, message),
        finding: finding.map(|(code, detail)| (code.to_owned(), detail)),
    }
}

fn target_state(kind: &str) -> Option<EncounterState> {
    match kind {
        "SESSION_STARTED" => Some(EncounterState::SessionStarted),
        "WORLD_READY" => Some(EncounterState::WorldReady),
        "ENCOUNTER_STARTED" | "DAMAGE" => Some(EncounterState::EncounterActive),
        "BEHEMOTH_DEFEATED" => Some(EncounterState::BehemothDefeated),
        _ => None,
    }
}

/// The chain after `batch`, or why it may not follow.
pub fn apply_batch(
    chain: &ChainRecord,
    batch: &EvidenceBatch,
    roster: &[AccountId],
    now: UnixMillis,
) -> Result<ChainUpdate, ChainRefusal> {
    if chain.completion.is_some() || chain.encounter_state.is_terminal() {
        return Err(chain_refusal("GUARD_CHAIN_FROZEN", FROZEN, None));
    }
    if batch.previous_hash != chain.head_hash || Some(batch.first_sequence) != chain.last_sequence.checked_add(1) {
        let detail = format!(
            "expected={}:{} got={}:{}",
            chain.last_sequence.saturating_add(1),
            chain.head_hash,
            batch.first_sequence,
            batch.previous_hash
        );
        return Err(chain_refusal(
            "GUARD_CHAIN_CONFLICT",
            CONFLICT,
            Some(("CHAIN_CONFLICT", detail)),
        ));
    }
    let count = batch.events.len() as u64;
    if chain.event_count.saturating_add(count) > MAX_CHAIN_EVENTS
        || chain.event_bytes.saturating_add(batch.event_bytes) > MAX_CHAIN_EVENT_BYTES
    {
        let detail = format!("events={}", chain.event_count);
        return Err(chain_refusal(
            "GUARD_EVIDENCE_LIMIT",
            LIMIT,
            Some(("EVIDENCE_LIMIT", detail)),
        ));
    }

    let mut next = chain.clone();
    let mut findings = Vec::new();
    let mut finding = |sequence: u64, code: &str, detail: String| {
        findings.push(FindingRecord {
            session: chain.session.clone(),
            generation: chain.generation,
            sequence: Some(sequence),
            code: code.to_owned(),
            detail,
            at: now,
        });
    };
    if let (Some(last), Some(first)) = (chain.last_monotonic_ms, batch.events.first())
        && first.monotonic_ms < last
    {
        finding(
            first.sequence,
            "MONOTONIC_REGRESSION",
            format!("from={last} to={}", first.monotonic_ms),
        );
    }
    let mut types = BTreeSet::new();
    for event in &batch.events {
        *next.event_types.entry(event.kind.clone()).or_insert(0) += 1;
        types.insert(event.kind.clone());
        match event.kind.as_str() {
            "COMPLETION_REQUESTED" => next.encounter_state = EncounterState::CompletionRequested,
            "HUNT_ABORTED" => next.encounter_state = EncounterState::Aborted,
            kind => {
                if let Some(state) = target_state(kind)
                    && state > next.encounter_state
                {
                    next.encounter_state = state;
                }
            }
        }
        if event.kind == "EVIDENCE_LOSS" {
            finding(event.sequence, "EVIDENCE_LOSS", String::new());
        }
        match &event.actor {
            Some(actor) if !roster.iter().any(|member| &member.0 == actor) => {
                finding(event.sequence, "ACTOR_NOT_ROSTERED", format!("actor={actor}"));
            }
            None if event.kind.contains("DAMAGE") && !next.has_unattributed_damage => {
                next.has_unattributed_damage = true;
                finding(event.sequence, "UNATTRIBUTED_DAMAGE", String::new());
            }
            _ => {}
        }
    }
    next.revision += 1;
    next.last_sequence = batch.last_sequence;
    next.head_hash = batch.batch_hash.clone();
    next.batch_count += 1;
    next.event_count += count;
    next.event_bytes += batch.event_bytes;
    next.last_monotonic_ms = batch.events.last().map(|e| e.monotonic_ms);
    next.last_received_at = Some(now);
    next.finding_count += findings.len() as u64;
    if next.finding_count > 0 {
        next.disposition = Disposition::ObserveFlagged;
    }
    let record = BatchRecord {
        session: chain.session.clone(),
        generation: chain.generation,
        batch_id: batch.batch_id.clone(),
        previous_hash: batch.previous_hash.clone(),
        batch_hash: batch.batch_hash.clone(),
        first_sequence: batch.first_sequence,
        last_sequence: batch.last_sequence,
        event_count: count,
        event_bytes: batch.event_bytes,
        event_types: types.into_iter().collect(),
        received_at: now,
    };
    Ok(ChainUpdate {
        chain: next,
        batch: Some(record),
        findings,
    })
}

/// A refusal's finding as its own chain write, with no sequence and no batch.
pub fn finding_update(chain: &ChainRecord, code: &str, detail: &str, now: UnixMillis) -> ChainUpdate {
    let mut next = chain.clone();
    next.revision += 1;
    next.finding_count += 1;
    next.disposition = Disposition::ObserveFlagged;
    ChainUpdate {
        chain: next,
        batch: None,
        findings: vec![FindingRecord {
            session: chain.session.clone(),
            generation: chain.generation,
            sequence: None,
            code: code.to_owned(),
            detail: detail.to_owned(),
            at: now,
        }],
    }
}

pub fn parse_completion(body: Option<&Value>) -> Result<CompletionRequest, GuardError> {
    let refused = || GuardError::refused("GUARD_COMPLETION_INVALID", COMPLETION_INVALID);
    let Some(Value::Object(object)) = body else {
        return Err(refused());
    };
    if !keys_within(object, &COMPLETION_KEYS) {
        return Err(refused());
    }
    let final_sequence = object
        .get("finalSequence")
        .and_then(non_negative)
        .filter(|n| *n >= 1)
        .ok_or_else(refused)?;
    let final_hash = match object.get("finalHash").and_then(Value::as_str) {
        Some(hash) if is_hex64(hash) => hash.to_owned(),
        _ => return Err(refused()),
    };
    Ok(CompletionRequest {
        final_sequence,
        final_hash,
    })
}

/// The chain completed, `None` when it already is with the same answer.
pub fn apply_completion(
    chain: &ChainRecord,
    request: &CompletionRequest,
    now: UnixMillis,
) -> Result<Option<ChainUpdate>, GuardError> {
    let conflict = || GuardError::refused("GUARD_COMPLETION_CONFLICT", COMPLETION_CONFLICT);
    if let Some(done) = &chain.completion {
        return if done.final_sequence == request.final_sequence && done.final_hash == request.final_hash {
            Ok(None)
        } else {
            Err(conflict())
        };
    }
    if chain.completion_mode == Some(CompletionMode::NoCompletion) {
        return Err(GuardError::refused("GUARD_COMPLETION_INVALID", NO_COMPLETION));
    }
    if chain.encounter_state != EncounterState::CompletionRequested {
        return Err(GuardError::refused("GUARD_COMPLETION_INVALID", NOT_REQUESTED));
    }
    if request.final_sequence != chain.last_sequence || request.final_hash != chain.head_hash {
        return Err(conflict());
    }
    let mut next = chain.clone();
    next.revision += 1;
    next.completion = Some(CompletionRecord {
        final_sequence: request.final_sequence,
        final_hash: request.final_hash.clone(),
        at: now,
    });
    Ok(Some(ChainUpdate {
        chain: next,
        batch: None,
        findings: Vec::new(),
    }))
}

/// The transport reads `"accepted":true` and the first `"batchHash"`, so those two lead.
pub(crate) fn batch_reply(
    session: &SessionId,
    generation: u64,
    batch_hash: &str,
    last_sequence: u64,
    chain: &ChainRecord,
    duplicate: bool,
) -> Json {
    Json(format!(
        r#"{{"accepted":true,"batchHash":{},"sessionId":{},"generation":{generation},"lastSequence":{last_sequence},"encounterState":"{}","disposition":"{}","duplicate":{duplicate},"observeOnly":true}}"#,
        js_string(batch_hash),
        js_string(&session.to_string()),
        chain.encounter_state.as_str(),
        chain.disposition.as_str(),
    ))
}

pub(crate) fn completion_reply(chain: &ChainRecord) -> Json {
    let (sequence, hash) = chain
        .completion
        .as_ref()
        .map_or((chain.last_sequence, chain.head_hash.as_str()), |c| {
            (c.final_sequence, c.final_hash.as_str())
        });
    Json(format!(
        r#"{{"accepted":true,"sessionId":{},"generation":{},"finalSequence":{sequence},"finalHash":{},"disposition":"{}","findingCount":{},"observeOnly":true,"progressionApplied":false,"rewardsApplied":false}}"#,
        js_string(&chain.session.to_string()),
        chain.generation,
        js_string(hash),
        chain.disposition.as_str(),
        chain.finding_count,
    ))
}
