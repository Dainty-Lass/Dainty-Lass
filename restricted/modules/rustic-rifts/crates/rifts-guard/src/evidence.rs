//! The Hunt Guard evidence chain intake (RF6), in observe mode. Mystic's anticheat module is private (DD14), so the
//! batch and completion contract here is the one the host's transport speaks (`admission_client.cpp`
//! `GuardBatchBody`, `ValidGuardBatch`, `request_guard_completion`): batches chained by hash, a completion naming the
//! head. Guard checks the chain, counts it and records what it would refuse as findings. A finding flags the hunt
//! and nothing else: no verdict refuses, no progression or reward depends on it.

use std::collections::BTreeMap;

use rifts_seam::{AccountId, UnixMillis};
use serde_json::{Map, Value};

use crate::digest::Sha256;
use crate::errors::GuardError;
use crate::json::{js_string, safe_integer, stringify};
use crate::store::{
    BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition, EncounterState, FindingRecord,
};

pub const BATCH_SCHEMA: i64 = 1;
/// The transport's `kMaximumGuardBatchEvents`.
pub const MAX_BATCH_EVENTS: usize = 128;
/// "each payload at most 8 KiB" (the transport's `GuardEventBatchRequest`).
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024;
/// "64 KiB per batch": the events as `JSON.stringify` writes them.
pub const MAX_BATCH_EVENT_BYTES: u64 = 64 * 1024;
/// Per hunt generation. Rifts' own bound, far above a two-hour hunt at the session traffic limit's request rate.
pub const MAX_CHAIN_EVENTS: u64 = 262_144;
pub const MAX_CHAIN_EVENT_BYTES: u64 = 64 * 1024 * 1024;

const TERMINAL_TYPES: &[&str] = &["COMPLETION_REQUESTED", "HUNT_ABORTED"];

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceEvent {
    pub sequence: u64,
    pub monotonic_ms: u64,
    pub kind: String,
    pub actor: Option<String>,
    pub payload: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceBatch {
    pub batch_id: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub previous_hash: String,
    pub events: Vec<EvidenceEvent>,
    pub event_bytes: u64,
    /// SHA-256 over the batch's canonical text, which includes `previousHash`: the next batch's `previousHash`.
    pub batch_hash: String,
}

/// Why a batch body was refused: `GUARD_BATCH_TOO_LARGE` or `GUARD_BATCH_INVALID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchProblem {
    TooLarge,
    Invalid(&'static str),
}

impl BatchProblem {
    pub fn into_error(self) -> GuardError {
        match self {
            Self::TooLarge => GuardError::refused(
                "GUARD_BATCH_TOO_LARGE",
                "The Guard request body exceeds its bounded limit.",
            ),
            Self::Invalid(_) => GuardError::refused("GUARD_BATCH_INVALID", "The Guard event batch is invalid."),
        }
    }
}

fn token(text: &str, max: usize, extra: &[u8]) -> bool {
    (1..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
}

fn lower_hex_64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn non_negative(value: Option<&Value>) -> Option<u64> {
    safe_integer(value?).and_then(|n| u64::try_from(n).ok())
}

fn only_keys(object: &Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

/// Reads one batch body (`{schema:1,batchId,firstSequence,lastSequence,previousHash,events:[...]}`). Strict: unknown
/// fields, gaps, a terminal event before the last, payloads that are not objects all refuse the batch.
pub fn parse_batch(body: Option<&Value>) -> Result<EvidenceBatch, BatchProblem> {
    let object = body
        .and_then(Value::as_object)
        .ok_or(BatchProblem::Invalid("not an object"))?;
    if !only_keys(
        object,
        &[
            "schema",
            "batchId",
            "firstSequence",
            "lastSequence",
            "previousHash",
            "events",
        ],
    ) {
        return Err(BatchProblem::Invalid("unknown field"));
    }
    if object.get("schema").and_then(safe_integer) != Some(BATCH_SCHEMA) {
        return Err(BatchProblem::Invalid("schema"));
    }
    let batch_id = object
        .get("batchId")
        .and_then(Value::as_str)
        .filter(|id| token(id, 64, b"-"))
        .ok_or(BatchProblem::Invalid("batchId"))?;
    let previous_hash = object
        .get("previousHash")
        .and_then(Value::as_str)
        .filter(|hash| lower_hex_64(hash))
        .ok_or(BatchProblem::Invalid("previousHash"))?;
    let first_sequence = non_negative(object.get("firstSequence"))
        .filter(|n| *n >= 1)
        .ok_or(BatchProblem::Invalid("firstSequence"))?;
    let last_sequence = non_negative(object.get("lastSequence")).ok_or(BatchProblem::Invalid("lastSequence"))?;
    let raw_events = object
        .get("events")
        .and_then(Value::as_array)
        .ok_or(BatchProblem::Invalid("events"))?;
    if raw_events.is_empty() {
        return Err(BatchProblem::Invalid("no events"));
    }
    if raw_events.len() > MAX_BATCH_EVENTS {
        return Err(BatchProblem::TooLarge);
    }
    let mut events = Vec::with_capacity(raw_events.len());
    let mut event_bytes = 0u64;
    for (index, raw) in raw_events.iter().enumerate() {
        let event = raw.as_object().ok_or(BatchProblem::Invalid("event"))?;
        if !only_keys(event, &["sequence", "monotonicMs", "type", "actorAccountId", "payload"]) {
            return Err(BatchProblem::Invalid("unknown event field"));
        }
        let sequence = non_negative(event.get("sequence")).ok_or(BatchProblem::Invalid("sequence"))?;
        if Some(sequence) != first_sequence.checked_add(index as u64) {
            return Err(BatchProblem::Invalid("sequence gap"));
        }
        let monotonic_ms = non_negative(event.get("monotonicMs")).ok_or(BatchProblem::Invalid("monotonicMs"))?;
        if events
            .last()
            .is_some_and(|previous: &EvidenceEvent| monotonic_ms < previous.monotonic_ms)
        {
            return Err(BatchProblem::Invalid("monotonicMs went back"));
        }
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| token(kind, 64, b"_"))
            .ok_or(BatchProblem::Invalid("type"))?;
        let actor = match event.get("actorAccountId") {
            None => None,
            Some(Value::String(actor)) if token(actor, 96, b"._:-") => Some(actor.clone()),
            Some(_) => return Err(BatchProblem::Invalid("actorAccountId")),
        };
        let payload = event
            .get("payload")
            .and_then(Value::as_object)
            .ok_or(BatchProblem::Invalid("payload"))?;
        if stringify(&Value::Object(payload.clone())).len() > MAX_PAYLOAD_BYTES {
            return Err(BatchProblem::Invalid("payload too large"));
        }
        if TERMINAL_TYPES.contains(&kind) && index + 1 != raw_events.len() {
            return Err(BatchProblem::Invalid("terminal event not last"));
        }
        event_bytes += stringify(raw).len() as u64;
        events.push(EvidenceEvent {
            sequence,
            monotonic_ms,
            kind: kind.to_owned(),
            actor,
            payload: payload.clone(),
        });
    }
    if event_bytes > MAX_BATCH_EVENT_BYTES {
        return Err(BatchProblem::TooLarge);
    }
    if events.last().map(|event| event.sequence) != Some(last_sequence) {
        return Err(BatchProblem::Invalid("lastSequence"));
    }
    let mut hasher = Sha256::default();
    hasher
        .update(b"rifts-guard-batch-v1\n")
        .update(stringify(&Value::Object(object.clone())).as_bytes());
    Ok(EvidenceBatch {
        batch_id: batch_id.to_owned(),
        first_sequence,
        last_sequence,
        previous_hash: previous_hash.to_owned(),
        events,
        event_bytes,
        batch_hash: hasher.finish_hex(),
    })
}

/// A refusal of a well-formed batch by the chain, with the finding it leaves behind, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRefusal {
    pub error: GuardError,
    pub finding: Option<(String, String)>,
}

fn frozen() -> GuardError {
    GuardError::refused(
        "GUARD_CHAIN_FROZEN",
        "The Guard chain of this hunt generation is closed.",
    )
}

fn finding(chain: &ChainRecord, sequence: Option<u64>, code: &str, detail: String, at: UnixMillis) -> FindingRecord {
    FindingRecord {
        session: chain.session.clone(),
        generation: chain.generation,
        sequence,
        code: code.to_owned(),
        detail,
        at,
    }
}

/// The encounter state an event moves the chain to, forward only (a terminal event always applies).
fn encounter_after(current: EncounterState, kind: &str) -> EncounterState {
    let next = match kind {
        "SESSION_STARTED" => EncounterState::SessionStarted,
        "WORLD_READY" => EncounterState::WorldReady,
        "ENCOUNTER_STARTED" | "DAMAGE" => EncounterState::EncounterActive,
        "BEHEMOTH_DEFEATED" => EncounterState::BehemothDefeated,
        "COMPLETION_REQUESTED" => return EncounterState::CompletionRequested,
        "HUNT_ABORTED" => return EncounterState::Aborted,
        _ => return current,
    };
    current.max(next)
}

/// Applies a parsed batch to the chain it claims to continue. `roster` is the hunt's frozen roster; observe mode
/// turns what it would refuse into findings on the update instead.
pub fn apply_batch(
    chain: &ChainRecord,
    batch: &EvidenceBatch,
    roster: &[AccountId],
    now: UnixMillis,
) -> Result<ChainUpdate, ChainRefusal> {
    if chain.completion.is_some() || chain.encounter_state.is_terminal() {
        return Err(ChainRefusal {
            error: frozen(),
            finding: None,
        });
    }
    if batch.previous_hash != chain.head_hash || Some(batch.first_sequence) != chain.last_sequence.checked_add(1) {
        return Err(ChainRefusal {
            error: GuardError::refused(
                "GUARD_CHAIN_CONFLICT",
                "The Guard batch does not continue this hunt generation's chain.",
            ),
            finding: Some((
                "CHAIN_CONFLICT".to_owned(),
                format!(
                    "expected={}:{} got={}:{}",
                    chain.last_sequence.saturating_add(1),
                    chain.head_hash,
                    batch.first_sequence,
                    batch.previous_hash
                ),
            )),
        });
    }
    let events = batch.events.len() as u64;
    if chain.event_count.saturating_add(events) > MAX_CHAIN_EVENTS
        || chain.event_bytes.saturating_add(batch.event_bytes) > MAX_CHAIN_EVENT_BYTES
    {
        return Err(ChainRefusal {
            error: GuardError::refused(
                "GUARD_EVIDENCE_LIMIT",
                "This hunt generation's Guard evidence limit is reached.",
            ),
            finding: Some(("EVIDENCE_LIMIT".to_owned(), format!("events={}", chain.event_count))),
        });
    }

    let mut next = chain.clone();
    let mut findings = Vec::new();
    if let (Some(last), Some(first)) = (chain.last_monotonic_ms, batch.events.first())
        && first.monotonic_ms < last
    {
        findings.push(finding(
            chain,
            Some(first.sequence),
            "MONOTONIC_REGRESSION",
            format!("from={last} to={}", first.monotonic_ms),
            now,
        ));
    }
    let mut batch_types: BTreeMap<&str, ()> = BTreeMap::new();
    for event in &batch.events {
        *next.event_types.entry(event.kind.clone()).or_insert(0) += 1;
        batch_types.insert(&event.kind, ());
        next.encounter_state = encounter_after(next.encounter_state, &event.kind);
        if event.kind == "EVIDENCE_LOSS" {
            findings.push(finding(
                chain,
                Some(event.sequence),
                "EVIDENCE_LOSS",
                String::new(),
                now,
            ));
        }
        match &event.actor {
            Some(actor) if !roster.iter().any(|member| &member.0 == actor) => {
                findings.push(finding(
                    chain,
                    Some(event.sequence),
                    "ACTOR_NOT_ROSTERED",
                    format!("actor={actor}"),
                    now,
                ));
            }
            None if event.kind.contains("DAMAGE") && !next.has_unattributed_damage => {
                next.has_unattributed_damage = true;
                findings.push(finding(
                    chain,
                    Some(event.sequence),
                    "UNATTRIBUTED_DAMAGE",
                    String::new(),
                    now,
                ));
            }
            _ => {}
        }
    }
    next.revision = chain.revision + 1;
    next.last_sequence = batch.last_sequence;
    next.head_hash = batch.batch_hash.clone();
    next.batch_count += 1;
    next.event_count += events;
    next.event_bytes += batch.event_bytes;
    next.last_monotonic_ms = batch.events.last().map(|event| event.monotonic_ms);
    next.last_received_at = Some(now);
    next.finding_count += findings.len() as u64;
    if next.finding_count > 0 {
        next.disposition = Disposition::ObserveFlagged;
    }
    Ok(ChainUpdate {
        batch: Some(BatchRecord {
            session: chain.session.clone(),
            generation: chain.generation,
            batch_id: batch.batch_id.clone(),
            previous_hash: batch.previous_hash.clone(),
            batch_hash: batch.batch_hash.clone(),
            first_sequence: batch.first_sequence,
            last_sequence: batch.last_sequence,
            event_count: events,
            event_bytes: batch.event_bytes,
            event_types: batch_types.keys().map(|kind| (*kind).to_owned()).collect(),
            received_at: now,
        }),
        chain: next,
        findings,
    })
}

/// The update that records a refused batch's finding, and nothing else.
pub fn finding_update(chain: &ChainRecord, code: String, detail: String, now: UnixMillis) -> ChainUpdate {
    let mut next = chain.clone();
    next.revision = chain.revision + 1;
    next.finding_count += 1;
    next.disposition = Disposition::ObserveFlagged;
    ChainUpdate {
        findings: vec![finding(chain, None, &code, detail, now)],
        chain: next,
        batch: None,
    }
}

fn accepted_reply(chain: &ChainRecord, batch_hash: &str, last_sequence: u64, duplicate: bool) -> String {
    format!(
        concat!(
            r#"{{"accepted":true,"batchHash":"{}","sessionId":{},"generation":{},"lastSequence":{},"#,
            r#""encounterState":"{}","disposition":"{}","duplicate":{},"observeOnly":true}}"#
        ),
        batch_hash,
        js_string(&chain.session.to_string()),
        chain.generation,
        last_sequence,
        chain.encounter_state.as_str(),
        chain.disposition.as_str(),
        duplicate,
    )
}

/// `{"accepted":true,"batchHash":...}`: the transport reads `accepted` and the hash it chains the next batch to.
pub fn batch_reply(chain: &ChainRecord) -> String {
    accepted_reply(chain, &chain.head_hash, chain.last_sequence, false)
}

/// The reply to a resent batch that was already taken: its own hash, whatever came after it.
pub fn duplicate_reply(chain: &ChainRecord, batch: &BatchRecord) -> String {
    accepted_reply(chain, &batch.batch_hash, batch.last_sequence, true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    pub final_sequence: u64,
    pub final_hash: String,
}

fn completion_invalid(message: &str) -> GuardError {
    GuardError::refused("GUARD_COMPLETION_INVALID", message)
}

/// `{finalSequence, finalHash}`, nothing else.
pub fn parse_completion(body: Option<&Value>) -> Result<CompletionRequest, GuardError> {
    let invalid = || completion_invalid("The Guard completion request is invalid.");
    let object = body.and_then(Value::as_object).ok_or_else(invalid)?;
    if !only_keys(object, &["finalSequence", "finalHash"]) {
        return Err(invalid());
    }
    let final_sequence = non_negative(object.get("finalSequence"))
        .filter(|n| *n >= 1)
        .ok_or_else(invalid)?;
    let final_hash = object
        .get("finalHash")
        .and_then(Value::as_str)
        .filter(|hash| lower_hex_64(hash))
        .ok_or_else(invalid)?;
    Ok(CompletionRequest {
        final_sequence,
        final_hash: final_hash.to_owned(),
    })
}

/// The chain after an accepted completion, or `None` when the same completion was already taken.
pub fn apply_completion(
    chain: &ChainRecord,
    request: &CompletionRequest,
    now: UnixMillis,
) -> Result<Option<ChainUpdate>, GuardError> {
    let conflict = || {
        GuardError::refused(
            "GUARD_COMPLETION_CONFLICT",
            "The Guard completion does not match this hunt generation's chain.",
        )
    };
    if let Some(done) = &chain.completion {
        return if done.final_sequence == request.final_sequence && done.final_hash == request.final_hash {
            Ok(None)
        } else {
            Err(conflict())
        };
    }
    if chain.completion_mode == Some(CompletionMode::NoCompletion) {
        return Err(completion_invalid("This hunt generation has no Guard completion."));
    }
    if chain.encounter_state != EncounterState::CompletionRequested {
        return Err(completion_invalid("The Guard chain has not requested completion."));
    }
    if request.final_sequence != chain.last_sequence || request.final_hash != chain.head_hash {
        return Err(conflict());
    }
    let mut next = chain.clone();
    next.revision = chain.revision + 1;
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

/// The transport reads `accepted` and `"progressionApplied":false`; observe mode applies neither progression nor
/// rewards from evidence.
pub fn completion_reply(chain: &ChainRecord) -> String {
    let (sequence, hash) = chain
        .completion
        .as_ref()
        .map_or((chain.last_sequence, chain.head_hash.as_str()), |done| {
            (done.final_sequence, done.final_hash.as_str())
        });
    format!(
        concat!(
            r#"{{"accepted":true,"sessionId":{},"generation":{},"finalSequence":{},"finalHash":"{}","#,
            r#""disposition":"{}","findingCount":{},"observeOnly":true,"progressionApplied":false,"#,
            r#""rewardsApplied":false}}"#
        ),
        js_string(&chain.session.to_string()),
        chain.generation,
        sequence,
        hash,
        chain.disposition.as_str(),
        chain.finding_count,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::store::GENESIS_HASH;
    use rifts_seam::SessionId;
    use serde_json::json;

    pub(crate) fn session() -> SessionId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap()
    }

    pub(crate) fn batch_body(id: &str, first: u64, previous: &str, kinds: &[&str]) -> Value {
        let events: Vec<Value> = kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| {
                json!({"sequence": first + index as u64, "monotonicMs": 100 * (first + index as u64), "type": kind,
                    "actorAccountId": "host", "payload": {"n": index}})
            })
            .collect();
        json!({"schema": 1, "batchId": id, "firstSequence": first, "lastSequence": first + kinds.len() as u64 - 1,
            "previousHash": previous, "events": events})
    }

    fn roster() -> Vec<AccountId> {
        vec![AccountId("host".to_owned()), AccountId("friend".to_owned())]
    }

    #[test]
    fn the_transports_batch_parses() {
        // The body GuardBatchBody writes, byte for byte
        let text = format!(
            concat!(
                r#"{{"schema":1,"batchId":"6f1c-2a","firstSequence":1,"lastSequence":2,"previousHash":"{}","#,
                r#""events":[{{"sequence":1,"monotonicMs":5,"type":"SESSION_STARTED","payload":{{}}}},"#,
                r#"{{"sequence":2,"monotonicMs":9,"type":"DAMAGE","actorAccountId":"host","#,
                r#""payload":{{"amount":12.5}}}}]}}"#
            ),
            GENESIS_HASH
        );
        let value: Value = serde_json::from_str(&text).unwrap();
        let batch = parse_batch(Some(&value)).unwrap();
        assert_eq!(
            (batch.first_sequence, batch.last_sequence, batch.events.len()),
            (1, 2, 2)
        );
        assert_eq!(batch.events[0].actor, None);
        assert_eq!(batch.batch_hash.len(), 64);
        assert_eq!(
            parse_batch(Some(&value)).unwrap().batch_hash,
            batch.batch_hash,
            "deterministic"
        );
    }

    #[test]
    fn malformed_batches_are_refused() {
        let good = batch_body("b-1", 1, GENESIS_HASH, &["SESSION_STARTED", "WORLD_READY"]);
        assert!(parse_batch(Some(&good)).is_ok());
        let broken = |edit: &dyn Fn(&mut Value)| {
            let mut body = good.clone();
            edit(&mut body);
            parse_batch(Some(&body))
        };
        assert!(parse_batch(None).is_err());
        assert!(parse_batch(Some(&json!([]))).is_err());
        assert!(broken(&|b| b["schema"] = json!(2)).is_err());
        assert!(broken(&|b| b["extra"] = json!(1)).is_err());
        assert!(broken(&|b| b["batchId"] = json!("bad id")).is_err());
        assert!(broken(&|b| b["previousHash"] = json!("ABC")).is_err());
        assert!(broken(&|b| b["firstSequence"] = json!(0)).is_err());
        assert!(broken(&|b| b["lastSequence"] = json!(5)).is_err());
        assert!(broken(&|b| b["events"] = json!([])).is_err());
        assert!(broken(&|b| b["events"][1]["sequence"] = json!(3)).is_err());
        assert!(broken(&|b| b["events"][1]["monotonicMs"] = json!(1)).is_err());
        assert!(broken(&|b| b["events"][0]["payload"] = json!([])).is_err());
        assert!(broken(&|b| b["events"][0]["type"] = json!("no spaces")).is_err());
        assert!(broken(&|b| b["events"][0]["actorAccountId"] = json!(7)).is_err());
        assert!(broken(&|b| b["events"][0]["sequence"] = json!(1.5)).is_err());
        assert!(
            broken(&|b| b["events"][0]["type"] = json!("HUNT_ABORTED")).is_err(),
            "terminal not last"
        );
        let big = "x".repeat(MAX_PAYLOAD_BYTES);
        assert!(broken(&|b| b["events"][0]["payload"] = json!({ "p": big })).is_err());
        let many: Vec<&str> = std::iter::repeat_n("TICK", MAX_BATCH_EVENTS + 1).collect();
        assert_eq!(
            parse_batch(Some(&batch_body("b", 1, GENESIS_HASH, &many))),
            Err(BatchProblem::TooLarge)
        );
    }

    #[test]
    fn batches_chain_by_hash_and_sequence() {
        let chain = ChainRecord::new(session(), 2, Some(CompletionMode::FirstDefeat));
        let first = parse_batch(Some(&batch_body(
            "b-1",
            1,
            GENESIS_HASH,
            &["SESSION_STARTED", "WORLD_READY"],
        )))
        .unwrap();
        let update = apply_batch(&chain, &first, &roster(), UnixMillis(10)).unwrap();
        let after = update.chain;
        assert_eq!(
            (
                after.revision,
                after.last_sequence,
                after.batch_count,
                after.event_count
            ),
            (1, 2, 1, 2)
        );
        assert_eq!(after.head_hash, first.batch_hash);
        assert_eq!(after.encounter_state, EncounterState::WorldReady);
        assert_eq!(after.disposition, Disposition::ObserveClear);
        assert!(update.findings.is_empty());

        let second = parse_batch(Some(&batch_body(
            "b-2",
            3,
            &first.batch_hash,
            &["DAMAGE", "BEHEMOTH_DEFEATED"],
        )))
        .unwrap();
        let after2 = apply_batch(&after, &second, &roster(), UnixMillis(20)).unwrap().chain;
        assert_eq!(after2.encounter_state, EncounterState::BehemothDefeated);

        let stale = parse_batch(Some(&batch_body("b-3", 3, &first.batch_hash, &["DAMAGE"]))).unwrap();
        let refusal = apply_batch(&after2, &stale, &roster(), UnixMillis(30)).unwrap_err();
        assert_eq!(refusal.error.code(), Some("GUARD_CHAIN_CONFLICT"));
        assert_eq!(refusal.finding.unwrap().0, "CHAIN_CONFLICT");
    }

    #[test]
    fn what_observe_would_refuse_becomes_a_finding() {
        let chain = ChainRecord::new(session(), 2, None);
        let mut body = batch_body("b-1", 1, GENESIS_HASH, &["DAMAGE", "DAMAGE", "EVIDENCE_LOSS"]);
        body["events"][0]["actorAccountId"] = json!("stranger");
        body["events"][1].as_object_mut().unwrap().remove("actorAccountId");
        let batch = parse_batch(Some(&body)).unwrap();
        let update = apply_batch(&chain, &batch, &roster(), UnixMillis(10)).unwrap();
        let codes: Vec<&str> = update.findings.iter().map(|finding| finding.code.as_str()).collect();
        assert_eq!(codes, ["ACTOR_NOT_ROSTERED", "UNATTRIBUTED_DAMAGE", "EVIDENCE_LOSS"]);
        assert_eq!(update.chain.disposition, Disposition::ObserveFlagged);
        assert_eq!(update.chain.finding_count, 3);
        assert!(update.chain.has_unattributed_damage);
        assert_eq!(update.batch.unwrap().event_types, ["DAMAGE", "EVIDENCE_LOSS"]);
    }

    #[test]
    fn a_closed_chain_takes_nothing_more() {
        let chain = ChainRecord::new(session(), 2, None);
        let last = parse_batch(Some(&batch_body("b-1", 1, GENESIS_HASH, &["COMPLETION_REQUESTED"]))).unwrap();
        let closed = apply_batch(&chain, &last, &roster(), UnixMillis(1)).unwrap().chain;
        assert_eq!(closed.encounter_state, EncounterState::CompletionRequested);
        let more = parse_batch(Some(&batch_body("b-2", 2, &last.batch_hash, &["DAMAGE"]))).unwrap();
        assert_eq!(
            apply_batch(&closed, &more, &roster(), UnixMillis(2))
                .unwrap_err()
                .error
                .code(),
            Some("GUARD_CHAIN_FROZEN")
        );
    }

    #[test]
    fn completion_names_the_head_once() {
        let chain = ChainRecord::new(session(), 2, Some(CompletionMode::FirstDefeat));
        let early = CompletionRequest {
            final_sequence: 1,
            final_hash: GENESIS_HASH.to_owned(),
        };
        assert_eq!(
            apply_completion(&chain, &early, UnixMillis(1)).unwrap_err().code(),
            Some("GUARD_COMPLETION_INVALID")
        );
        let batch = parse_batch(Some(&batch_body(
            "b-1",
            1,
            GENESIS_HASH,
            &["BEHEMOTH_DEFEATED", "COMPLETION_REQUESTED"],
        )))
        .unwrap();
        let requested = apply_batch(&chain, &batch, &roster(), UnixMillis(1)).unwrap().chain;
        let wrong = CompletionRequest {
            final_sequence: 2,
            final_hash: GENESIS_HASH.to_owned(),
        };
        assert_eq!(
            apply_completion(&requested, &wrong, UnixMillis(2)).unwrap_err().code(),
            Some("GUARD_COMPLETION_CONFLICT")
        );
        let right = CompletionRequest {
            final_sequence: 2,
            final_hash: batch.batch_hash.clone(),
        };
        let done = apply_completion(&requested, &right, UnixMillis(2))
            .unwrap()
            .unwrap()
            .chain;
        assert!(done.completion.is_some());
        assert_eq!(
            apply_completion(&done, &right, UnixMillis(3)),
            Ok(None),
            "the same completion again"
        );
        assert_eq!(
            apply_completion(&done, &wrong, UnixMillis(3)).unwrap_err().code(),
            Some("GUARD_COMPLETION_CONFLICT")
        );
        let reply = completion_reply(&done);
        assert!(reply.contains(r#""accepted":true"#) && reply.contains(r#""progressionApplied":false"#));

        let hub_chain = ChainRecord::new(session(), 2, Some(CompletionMode::NoCompletion));
        assert_eq!(
            apply_completion(&hub_chain, &right, UnixMillis(1)).unwrap_err().code(),
            Some("GUARD_COMPLETION_INVALID")
        );
    }

    #[test]
    fn completion_bodies_are_strict() {
        let hash = "a".repeat(64);
        assert!(parse_completion(Some(&json!({"finalSequence": 3, "finalHash": hash}))).is_ok());
        assert!(parse_completion(Some(&json!({"finalSequence": 0, "finalHash": hash}))).is_err());
        assert!(parse_completion(Some(&json!({"finalSequence": 3, "finalHash": "A".repeat(64)}))).is_err());
        assert!(parse_completion(Some(&json!({"finalSequence": 3, "finalHash": hash, "x": 1}))).is_err());
        assert!(parse_completion(None).is_err());
    }

    #[test]
    fn replies_carry_what_the_transport_reads() {
        let chain = ChainRecord::new(session(), 2, None);
        let batch = parse_batch(Some(&batch_body("b-1", 1, GENESIS_HASH, &["SESSION_STARTED"]))).unwrap();
        let after = apply_batch(&chain, &batch, &roster(), UnixMillis(1)).unwrap().chain;
        let reply = batch_reply(&after);
        assert!(reply.starts_with(&format!(r#"{{"accepted":true,"batchHash":"{}""#, batch.batch_hash)));
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["sessionId"], json!(session().to_string()));
        assert_eq!(parsed["observeOnly"], json!(true));
    }
}
