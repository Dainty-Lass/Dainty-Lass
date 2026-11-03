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
    match raw_events.len() {
        0 => return invalid("no events"),
        n if n > MAX_BATCH_EVENTS => return Err(BatchProblem::TooLarge),
        _ => {}
    }
    let mut events: Vec<EvidenceEvent> = Vec::with_capacity(raw_events.len());
    let mut event_bytes = 0_u64;
    for (index, raw) in raw_events.iter().enumerate() {
        let sequence = first_sequence.checked_add(index as u64);
        let last = index + 1 == raw_events.len();
        let event = parse_event(raw, sequence, events.last().map(|e| e.monotonic_ms), last)?;
        event_bytes += stringify(raw).len() as u64;
        events.push(event);
    }
    if event_bytes > MAX_BATCH_EVENT_BYTES {
        return Err(BatchProblem::TooLarge);
    }
    if events.last().map(|e| e.sequence) != Some(last_sequence) {
        return invalid("lastSequence");
    }
    // The hash is over the whole body as parsed, keys in document order (spec-rd27-rifts §2 D5).
    let canonical = stringify(&Value::Object(object.clone()));
    let batch_hash = hex_of_parts(&[BATCH_HASH_LABEL, canonical.as_bytes()]);
    Ok(EvidenceBatch {
        batch_id,
        first_sequence,
        last_sequence,
        previous_hash,
        events,
        event_bytes,
        batch_hash,
    })
}

/// One event of a batch: `sequence` is where it must sit, `floor` the previous event's clock.
fn parse_event(
    raw: &Value,
    sequence: Option<u64>,
    floor: Option<u64>,
    last: bool,
) -> Result<EvidenceEvent, BatchProblem> {
    let Value::Object(event) = raw else {
        return invalid("event not an object");
    };
    if !keys_within(event, &EVENT_KEYS) {
        return invalid("unknown event field");
    }
    let get = |name: &str| event.get(name).unwrap_or(&Value::Null);
    let sequence = match (non_negative(get("sequence")), sequence) {
        (Some(given), Some(wanted)) if given == wanted => given,
        _ => return invalid("sequence"),
    };
    let monotonic_ms = non_negative(get("monotonicMs")).ok_or(BatchProblem::Invalid("monotonicMs"))?;
    if floor.is_some_and(|floor| monotonic_ms < floor) {
        return invalid("monotonicMs regressed");
    }
    let kind = token(get("type"), 64, b"_").ok_or(BatchProblem::Invalid("type"))?;
    let actor = event
        .get("actorAccountId")
        .map(|value| token(value, 96, b"._:-").ok_or(BatchProblem::Invalid("actorAccountId")))
        .transpose()?;
    let payload = event
        .get("payload")
        .filter(|payload| payload.is_object() && stringify(payload).len() <= MAX_PAYLOAD_BYTES)
        .cloned()
        .ok_or(BatchProblem::Invalid("payload"))?;
    if is_terminal_type(&kind) && !last {
        return invalid("terminal event not last");
    }
    Ok(EvidenceEvent {
        sequence,
        monotonic_ms,
        kind,
        actor,
        payload,
    })
}

fn chain_refusal(code: &str, message: &str, finding: Option<(&str, String)>) -> ChainRefusal {
    let error = GuardError::refused(code, message);
    let finding = finding.map(|(code, detail)| (code.to_owned(), detail));
    ChainRefusal { error, finding }
}

/// Where an event moves the encounter; terminal events move it even backwards in the order.
fn moved_state(kind: &str, current: EncounterState) -> EncounterState {
    use EncounterState::*;
    let forward = match kind {
        "COMPLETION_REQUESTED" => return CompletionRequested,
        "HUNT_ABORTED" => return Aborted,
        "SESSION_STARTED" => SessionStarted,
        "WORLD_READY" => WorldReady,
        "ENCOUNTER_STARTED" | "DAMAGE" => EncounterActive,
        "BEHEMOTH_DEFEATED" => BehemothDefeated,
        _ => current,
    };
    forward.max(current)
}

/// Findings of one chain write, stamped with its chain and time.
struct Findings<'a> {
    chain: &'a ChainRecord,
    now: UnixMillis,
    list: Vec<FindingRecord>,
}

impl Findings<'_> {
    fn add(&mut self, sequence: Option<u64>, code: &str, detail: String) {
        self.list.push(FindingRecord {
            session: self.chain.session.clone(),
            generation: self.chain.generation,
            sequence,
            code: code.into(),
            detail,
            at: self.now,
        });
    }
}

/// Why `batch` may not follow `chain`, before any of it is read.
fn batch_gate(chain: &ChainRecord, batch: &EvidenceBatch) -> Result<(), ChainRefusal> {
    if chain.completion.is_some() || chain.encounter_state.is_terminal() {
        return Err(chain_refusal("GUARD_CHAIN_FROZEN", FROZEN, None));
    }
    let expected = chain.last_sequence.checked_add(1);
    if expected != Some(batch.first_sequence) || batch.previous_hash != chain.head_hash {
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
    let events_over = chain.event_count.saturating_add(batch.events.len() as u64) > MAX_CHAIN_EVENTS;
    let bytes_over = chain.event_bytes.saturating_add(batch.event_bytes) > MAX_CHAIN_EVENT_BYTES;
    if events_over || bytes_over {
        let detail = format!("events={}", chain.event_count);
        return Err(chain_refusal(
            "GUARD_EVIDENCE_LIMIT",
            LIMIT,
            Some(("EVIDENCE_LIMIT", detail)),
        ));
    }
    Ok(())
}

/// The chain after `batch`, or why it may not follow.
pub fn apply_batch(
    chain: &ChainRecord,
    batch: &EvidenceBatch,
    roster: &[AccountId],
    now: UnixMillis,
) -> Result<ChainUpdate, ChainRefusal> {
    batch_gate(chain, batch)?;
    let mut findings = Findings {
        chain,
        now,
        list: vec![],
    };
    let mut next = chain.clone();
    let regressed = batch
        .events
        .first()
        .zip(chain.last_monotonic_ms)
        .filter(|(first, last)| first.monotonic_ms < *last);
    if let Some((first, last)) = regressed {
        findings.add(
            Some(first.sequence),
            "MONOTONIC_REGRESSION",
            format!("from={last} to={}", first.monotonic_ms),
        );
    }
    let mut types = BTreeSet::new();
    for event in &batch.events {
        let (kind, at) = (event.kind.as_str(), Some(event.sequence));
        *next.event_types.entry(kind.to_owned()).or_default() += 1;
        types.insert(kind);
        next.encounter_state = moved_state(kind, next.encounter_state);
        if kind == "EVIDENCE_LOSS" {
            findings.add(at, "EVIDENCE_LOSS", String::new());
        }
        match &event.actor {
            Some(actor) if roster.iter().all(|member| member.0 != *actor) => {
                findings.add(at, "ACTOR_NOT_ROSTERED", format!("actor={actor}"));
            }
            // Only the first unattributed damage of a chain is a finding.
            None if kind.contains("DAMAGE") && !next.has_unattributed_damage => {
                next.has_unattributed_damage = true;
                findings.add(at, "UNATTRIBUTED_DAMAGE", String::new());
            }
            _ => {}
        }
    }
    let count = batch.events.len() as u64;
    let found = findings.list;
    next.revision += 1;
    next.batch_count += 1;
    next.event_count += count;
    next.event_bytes += batch.event_bytes;
    next.finding_count += found.len() as u64;
    (next.last_sequence, next.head_hash) = (batch.last_sequence, batch.batch_hash.clone());
    next.last_monotonic_ms = batch.events.last().map(|e| e.monotonic_ms);
    next.last_received_at = Some(now);
    if next.finding_count != 0 {
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
        event_types: types.into_iter().map(String::from).collect(),
        received_at: now,
    };
    Ok(ChainUpdate {
        chain: next,
        batch: Some(record),
        findings: found,
    })
}

/// A refusal's finding as its own chain write, with no sequence and no batch.
pub fn finding_update(chain: &ChainRecord, code: &str, detail: &str, now: UnixMillis) -> ChainUpdate {
    let mut findings = Findings {
        chain,
        now,
        list: vec![],
    };
    findings.add(None, code, detail.into());
    let next = ChainRecord {
        revision: chain.revision + 1,
        finding_count: chain.finding_count + 1,
        disposition: Disposition::ObserveFlagged,
        ..chain.clone()
    };
    ChainUpdate {
        chain: next,
        batch: None,
        findings: findings.list,
    }
}

pub fn parse_completion(body: Option<&Value>) -> Result<CompletionRequest, GuardError> {
    let read = || {
        let object = body?
            .as_object()
            .filter(|object| keys_within(object, &COMPLETION_KEYS))?;
        let final_sequence = object.get("finalSequence").and_then(non_negative).filter(|n| *n != 0)?;
        let final_hash = object
            .get("finalHash")
            .and_then(Value::as_str)
            .filter(|hash| is_hex64(hash))?;
        Some(CompletionRequest {
            final_sequence,
            final_hash: final_hash.into(),
        })
    };
    read().ok_or_else(|| GuardError::refused("GUARD_COMPLETION_INVALID", COMPLETION_INVALID))
}

/// The chain completed, `None` when it already is with the same answer.
pub fn apply_completion(
    chain: &ChainRecord,
    request: &CompletionRequest,
    now: UnixMillis,
) -> Result<Option<ChainUpdate>, GuardError> {
    let matches = |sequence: u64, hash: &str| sequence == request.final_sequence && hash == request.final_hash;
    let conflict = || GuardError::refused("GUARD_COMPLETION_CONFLICT", COMPLETION_CONFLICT);
    if let Some(done) = &chain.completion {
        return if matches(done.final_sequence, &done.final_hash) {
            Ok(None)
        } else {
            Err(conflict())
        };
    }
    let invalid = |text: &str| Err(GuardError::refused("GUARD_COMPLETION_INVALID", text));
    if chain.completion_mode == Some(CompletionMode::NoCompletion) {
        return invalid(NO_COMPLETION);
    }
    if chain.encounter_state != EncounterState::CompletionRequested {
        return invalid(NOT_REQUESTED);
    }
    if !matches(chain.last_sequence, &chain.head_hash) {
        return Err(conflict());
    }
    let completion = CompletionRecord {
        final_sequence: request.final_sequence,
        final_hash: request.final_hash.clone(),
        at: now,
    };
    let next = ChainRecord {
        revision: chain.revision + 1,
        completion: Some(completion),
        ..chain.clone()
    };
    Ok(Some(ChainUpdate {
        chain: next,
        batch: None,
        findings: vec![],
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
    let (hash, session) = (js_string(batch_hash), js_string(&session.to_string()));
    let (state, disposition) = (chain.encounter_state.as_str(), chain.disposition.as_str());
    Json(format!(
        concat!(
            r#"{{"accepted":true,"batchHash":{hash},"sessionId":{session},"generation":{generation},"#,
            r#""lastSequence":{last},"encounterState":"{state}","disposition":"{disposition}","#,
            r#""duplicate":{duplicate},"observeOnly":true}}"#
        ),
        hash = hash,
        session = session,
        generation = generation,
        last = last_sequence,
        state = state,
        disposition = disposition,
        duplicate = duplicate,
    ))
}

pub(crate) fn completion_reply(chain: &ChainRecord) -> Json {
    let (sequence, hash) = match &chain.completion {
        Some(done) => (done.final_sequence, done.final_hash.as_str()),
        None => (chain.last_sequence, chain.head_hash.as_str()),
    };
    let (session, hash) = (js_string(&chain.session.to_string()), js_string(hash));
    let (generation, disposition, findings) = (chain.generation, chain.disposition.as_str(), chain.finding_count);
    Json(format!(
        concat!(
            r#"{{"accepted":true,"sessionId":{session},"generation":{generation},"finalSequence":{sequence},"#,
            r#""finalHash":{hash},"disposition":"{disposition}","findingCount":{findings},"observeOnly":true,"#,
            r#""progressionApplied":false,"rewardsApplied":false}}"#
        ),
        session = session,
        generation = generation,
        sequence = sequence,
        hash = hash,
        disposition = disposition,
        findings = findings,
    ))
}
