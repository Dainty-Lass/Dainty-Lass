// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The host's Guard evidence publisher: events are chained into batches (each naming the previous batch's hash) and
//! sent from a thread of their own, so enqueueing never waits on the network. A refused batch ends the hunt's
//! evidence; a lost one is sent again unchanged.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use rand::TryRng as _;

use crate::backend::{BackendClient, BatchAnswer, MAX_BATCH_BODY};
use crate::text::safe_str;

pub const COMPLETION_REQUESTED: &str = "COMPLETION_REQUESTED";
pub const HUNT_ABORTED: &str = "HUNT_ABORTED";
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_EVENTS_HARD: usize = 128;
const EVENT_OVERHEAD: usize = 112;
const BATCH_OVERHEAD: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublisherConfig {
    pub window: Duration,
    pub first_retry: Duration,
    pub retry_ceiling: Duration,
    pub max_events: usize,
    pub body_estimate: usize,
    pub queue: usize,
    pub completion_attempts: u32,
    pub completion_first_wait: Duration,
}

impl Default for PublisherConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_millis(2000),
            first_retry: Duration::from_millis(250),
            retry_ceiling: Duration::from_millis(4000),
            max_events: 96,
            body_estimate: MAX_BATCH_BODY,
            queue: 4096,
            completion_attempts: 6,
            completion_first_wait: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Event {
    kind: String,
    actor: String,
    payload: String,
    monotonic_ms: u64,
    queued_at: Instant,
}

fn is_terminal(kind: &str) -> bool {
    kind == COMPLETION_REQUESTED || kind == HUNT_ABORTED
}

#[derive(Debug, Clone)]
struct Batch {
    id: String,
    first: u64,
    events: Vec<Event>,
    previous: String,
    body: String,
    retry_at: Option<Instant>,
}

impl Batch {
    fn last(&self) -> u64 {
        self.first + self.events.len() as u64 - 1
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublisherStats {
    pub publish_failures: u64,
    pub evidence_loss: u64,
    pub accepted_batches: u64,
    pub completed: bool,
    pub incomplete: bool,
    pub refused: bool,
}

struct State {
    config: PublisherConfig,
    queue: VecDeque<Event>,
    stopping: bool,
    flushing: bool,
    overflow_gap: u64,
    overflow_reported: bool,
    next_sequence: u64,
    head: String,
    batch: Option<Batch>,
    retry_delay: Duration,
    failures_in_row: u32,
    completion_due: bool,
    completion_attempt: u32,
    completion_at: Option<Instant>,
    stats: PublisherStats,
}

impl State {
    fn idle(&self) -> bool {
        self.stats.refused || (self.queue.is_empty() && self.batch.is_none() && !self.completion_due)
    }

    fn batch_due(&self, now: Instant) -> bool {
        let Some(oldest) = self.queue.front() else {
            return false;
        };
        self.flushing
            || self.queue.iter().any(|e| is_terminal(&e.kind))
            || self.queue.len() >= self.config.max_events
            || now.saturating_duration_since(oldest.queued_at) >= self.config.window
    }

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        if let Some(batch) = &self.batch {
            return batch.retry_at;
        }
        let window = self.queue.front().map(|e| e.queued_at + self.config.window);
        let completion = self.completion_due.then(|| self.completion_at.unwrap_or(now));
        match (window, completion) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

pub type Sink = Arc<dyn Fn(String) + Send + Sync>;

pub struct Publisher {
    shared: Arc<(Mutex<State>, Condvar)>,
    started: Instant,
    worker: Option<thread::JoinHandle<()>>,
    sink: Sink,
}

fn uuid_v4() -> Option<String> {
    let mut bytes = [0_u8; 16];
    rand::rngs::SysRng.try_fill_bytes(&mut bytes).ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Some(format_uuid(&bytes))
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex: String = bytes
        .iter()
        .flat_map(|&b| [b >> 4, b & 15])
        .filter_map(|n| char::from_digit(u32::from(n), 16))
        .collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn event_json(sequence: u64, event: &Event) -> String {
    let actor = if event.actor.is_empty() {
        String::new()
    } else {
        format!(",\"actorAccountId\":\"{}\"", event.actor)
    };
    format!(
        "{{\"sequence\":{sequence},\"monotonicMs\":{},\"type\":\"{}\"{actor},\"payload\":{}}}",
        event.monotonic_ms, event.kind, event.payload
    )
}

/// The client's checks before a batch is sent; a failure is an answered refusal.
fn check_batch(session_ok: bool, batch: &Batch) -> Result<(), &'static str> {
    let events_ok = (1..=MAX_EVENTS_HARD).contains(&batch.events.len())
        && batch.first >= 1
        && batch.events.windows(2).all(|w| w[0].monotonic_ms <= w[1].monotonic_ms)
        && batch.events.iter().all(|e| {
            safe_str(&e.kind, 64, b"_")
                && (e.actor.is_empty() || safe_str(&e.actor, 96, b"._:-"))
                && (1..=4096).contains(&e.payload.len())
                && e.payload.starts_with('{')
                && e.payload.ends_with('}')
        });
    let header_ok = session_ok
        && safe_str(&batch.id, 64, b"-")
        && batch.previous.len() == 64
        && batch.previous.bytes().all(|b| b.is_ascii_alphanumeric());
    if !events_ok || !header_ok {
        return Err("invalid Guard event batch");
    }
    if batch.body.len() > MAX_BATCH_BODY {
        return Err("Guard event batch exceeds its size limit");
    }
    Ok(())
}

impl Publisher {
    pub fn start(client: Arc<BackendClient>, generation: u32, config: PublisherConfig, sink: Sink) -> Self {
        let state = State {
            config,
            queue: VecDeque::new(),
            stopping: false,
            flushing: false,
            overflow_gap: 0,
            overflow_reported: false,
            next_sequence: 1,
            head: ZERO_HASH.to_owned(),
            batch: None,
            retry_delay: config.first_retry,
            failures_in_row: 0,
            completion_due: false,
            completion_attempt: 0,
            completion_at: None,
            stats: PublisherStats::default(),
        };
        let shared = Arc::new((Mutex::new(state), Condvar::new()));
        let worker_shared = shared.clone();
        let worker_sink = sink.clone();
        let worker = thread::Builder::new()
            .name("rifts-guard-publisher".to_owned())
            .spawn(move || run(&worker_shared, &client, generation, &worker_sink))
            .ok();
        Self {
            shared,
            started: Instant::now(),
            worker,
            sink,
        }
    }

    fn lock(&self) -> Option<MutexGuard<'_, State>> {
        self.shared.0.lock().ok()
    }

    /// Never waits on the network. `false` when stopping, after the evidence ended, or with the queue full.
    pub fn enqueue(&self, kind: &str, actor: &str, payload: &str) -> bool {
        let now = Instant::now();
        let monotonic_ms = u64::try_from(now.saturating_duration_since(self.started).as_millis()).unwrap_or(u64::MAX);
        let Some(mut state) = self.lock() else {
            return false;
        };
        if state.stopping || self.worker.is_none() {
            return false;
        }
        if state.stats.refused {
            state.stats.evidence_loss += 1;
            return false;
        }
        if state.queue.len() >= state.config.queue {
            state.stats.evidence_loss += 1;
            state.stats.incomplete = true;
            state.overflow_gap += 1;
            if !state.overflow_reported {
                state.overflow_reported = true;
                (self.sink)("GUARD_EVIDENCE_LOSS action=completion_fenced reason=publisher_queue_full".to_owned());
            }
            return false;
        }
        let event = |kind: &str, actor: &str, payload: String| Event {
            kind: kind.to_owned(),
            actor: actor.to_owned(),
            payload,
            monotonic_ms,
            queued_at: now,
        };
        if state.overflow_gap > 0 {
            let gap = std::mem::take(&mut state.overflow_gap);
            state.queue.push_back(event(
                "EVIDENCE_LOSS",
                "",
                format!("{{\"droppedEvents\":{gap},\"reason\":\"publisher_queue_full\"}}"),
            ));
        }
        state.queue.push_back(event(kind, actor, payload.to_owned()));
        drop(state);
        self.shared.1.notify_all();
        true
    }

    pub fn stats(&self) -> PublisherStats {
        self.lock().map(|s| s.stats).unwrap_or_default()
    }

    /// Makes every queued event due, waits up to `limit` until idle, then stops the thread.
    pub fn flush_and_stop(&mut self, limit: Duration) {
        let deadline = Instant::now() + limit;
        if let Some(mut state) = self.lock() {
            state.flushing = true;
        }
        self.shared.1.notify_all();
        let (lock, signal) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            while !state.idle() {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || self.worker.is_none() {
                    break;
                }
                state = match signal.wait_timeout(state, left) {
                    Ok((state, _)) => state,
                    Err(_) => break,
                };
            }
        }
        self.stop();
    }

    pub fn stop(&mut self) {
        if let Some(mut state) = self.lock() {
            state.stopping = true;
        }
        self.shared.1.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub fn sink(&self) -> Sink {
        self.sink.clone()
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn end_evidence(state: &mut State, lost: u64, reason: &str, sink: &Sink) {
    state.stats.refused = true;
    state.stats.incomplete = true;
    state.stats.evidence_loss += lost + state.queue.len() as u64;
    state.queue.clear();
    state.batch = None;
    state.completion_due = false;
    sink(format!("GUARD_EVIDENCE_LOSS action=completion_fenced reason={reason}"));
}

fn build_batch(state: &mut State, session_ok: bool, sink: &Sink) -> Option<Batch> {
    let Some(id) = uuid_v4() else {
        end_evidence(state, 0, "batch_id_generation", sink);
        return None;
    };
    let mut events = Vec::new();
    let mut total = BATCH_OVERHEAD;
    while let Some(next) = state.queue.front() {
        let size = EVENT_OVERHEAD + next.kind.len() + next.actor.len() + next.payload.len();
        if events.len() >= state.config.max_events.min(MAX_EVENTS_HARD)
            || (!events.is_empty() && total + size > state.config.body_estimate)
        {
            break;
        }
        total += size;
        let Some(event) = state.queue.pop_front() else {
            break;
        };
        let terminal = is_terminal(&event.kind);
        events.push(event);
        if terminal {
            break;
        }
    }
    let first = state.next_sequence;
    let body_events: Vec<String> = events
        .iter()
        .enumerate()
        .map(|(i, e)| event_json(first + i as u64, e))
        .collect();
    let batch = Batch {
        body: format!(
            "{{\"schema\":1,\"batchId\":\"{id}\",\"firstSequence\":{first},\"lastSequence\":{},\"previousHash\":\"{}\",\"events\":[{}]}}",
            first + events.len() as u64 - 1,
            state.head,
            body_events.join(",")
        ),
        id,
        first,
        events,
        previous: state.head.clone(),
        retry_at: None,
    };
    if state.overflow_gap > 0 && state.queue.len() < state.config.queue {
        let gap = std::mem::take(&mut state.overflow_gap);
        let stamp = batch.events.last().map_or(0, |e| e.monotonic_ms);
        state.queue.push_back(Event {
            kind: "EVIDENCE_LOSS".to_owned(),
            actor: String::new(),
            payload: format!("{{\"droppedEvents\":{gap},\"reason\":\"publisher_queue_full\"}}"),
            monotonic_ms: stamp,
            queued_at: Instant::now(),
        });
    }
    if let Err(error) = check_batch(session_ok, &batch) {
        state.stats.publish_failures += 1;
        let lost = batch.events.len() as u64;
        end_evidence(state, lost, "batch_refused status=0", sink);
        sink(format!(
            "HUNT GUARD OBSERVE WARN: {error} (no more evidence is sent for this hunt)"
        ));
        return None;
    }
    Some(batch)
}

fn run(shared: &Arc<(Mutex<State>, Condvar)>, client: &BackendClient, generation: u32, sink: &Sink) {
    let (lock, signal) = &**shared;
    let session_ok = client.session_ok();
    loop {
        enum Work {
            Send(Batch),
            Complete(u64, String),
        }
        let work = {
            let Ok(mut state) = lock.lock() else {
                return;
            };
            loop {
                if state.stopping {
                    return;
                }
                let now = Instant::now();
                if state.stats.refused {
                    state.queue.clear();
                } else if let Some(batch) = &state.batch {
                    if batch.retry_at.is_none_or(|at| at <= now) {
                        break Work::Send(batch.clone());
                    }
                } else if state.batch_due(now) {
                    if let Some(batch) = build_batch(&mut state, session_ok, sink) {
                        state.batch = Some(batch.clone());
                        break Work::Send(batch);
                    }
                    signal.notify_all();
                    continue;
                } else if state.completion_due && state.completion_at.is_none_or(|at| at <= now) {
                    if state.stats.incomplete {
                        state.completion_due = false;
                        sink(
                            "HUNT GUARD COMPLETION WARN: Guard evidence is incomplete; completion is not requested"
                                .to_owned(),
                        );
                        signal.notify_all();
                        continue;
                    }
                    break Work::Complete(state.next_sequence - 1, state.head.clone());
                }
                signal.notify_all();
                state = match state.next_wake(now) {
                    Some(at) => match signal.wait_timeout(state, at.saturating_duration_since(now)) {
                        Ok((state, _)) => state,
                        Err(_) => return,
                    },
                    None => match signal.wait(state) {
                        Ok(state) => state,
                        Err(_) => return,
                    },
                };
            }
        };
        match work {
            Work::Send(batch) => {
                let answer = client.event_batch(batch.body.clone(), &format!("guard-{generation}-{}", batch.first));
                let Ok(mut state) = lock.lock() else {
                    return;
                };
                match answer {
                    BatchAnswer::Accepted(hash) => {
                        state.head = hash;
                        state.next_sequence = batch.last() + 1;
                        if state.failures_in_row > 0 {
                            sink(format!(
                                "HUNT GUARD OBSERVE: the batch was taken after {} failed send(s)",
                                state.failures_in_row
                            ));
                        }
                        state.failures_in_row = 0;
                        state.retry_delay = state.config.first_retry;
                        state.batch = None;
                        state.stats.accepted_batches += 1;
                        if batch.events.iter().any(|e| e.kind == COMPLETION_REQUESTED) {
                            state.completion_due = true;
                            state.completion_at = None;
                        }
                    }
                    BatchAnswer::Retry(error) => {
                        state.stats.publish_failures += 1;
                        state.failures_in_row += 1;
                        if state.failures_in_row == 1 {
                            sink(format!(
                                "HUNT GUARD OBSERVE WARN: {error} (sending the same batch again)"
                            ));
                        }
                        let delay = state.retry_delay;
                        state.retry_delay = (delay * 2).min(state.config.retry_ceiling);
                        if let Some(out) = state.batch.as_mut() {
                            out.retry_at = Some(Instant::now() + delay);
                        }
                    }
                    BatchAnswer::Refused { status, error } => {
                        state.stats.publish_failures += 1;
                        let lost = batch.events.len() as u64;
                        end_evidence(&mut state, lost, &format!("batch_refused status={status}"), sink);
                        let error = if error.is_empty() {
                            "Guard event batch was rejected".to_owned()
                        } else {
                            error
                        };
                        sink(format!(
                            "HUNT GUARD OBSERVE WARN: {error} (no more evidence is sent for this hunt)"
                        ));
                    }
                }
                signal.notify_all();
            }
            Work::Complete(last, head) => {
                let answer = client.completion(last, &head, &format!("complete-{generation}-{last}"));
                let Ok(mut state) = lock.lock() else {
                    return;
                };
                match answer {
                    Ok(()) => {
                        state.completion_due = false;
                        state.stats.completed = true;
                    }
                    Err(error) => {
                        state.completion_attempt += 1;
                        let attempt = state.completion_attempt;
                        if attempt >= state.config.completion_attempts {
                            state.completion_due = false;
                            sink(format!(
                                "HUNT GUARD COMPLETION WARN: {error} (gave up after {attempt} attempts)"
                            ));
                        } else {
                            sink(format!("HUNT GUARD COMPLETION WARN: {error} (trying again)"));
                            let wait = state.config.completion_first_wait * 2_u32.pow(attempt - 1);
                            state.completion_at = Some(Instant::now() + wait);
                        }
                    }
                }
                signal.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_shape() {
        let id = uuid_v4().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(
            id.bytes()
                .all(|b| b == b'-' || b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn event_text() {
        let event = Event {
            kind: "PLAYER_LOST".to_owned(),
            actor: String::new(),
            payload: "{\"transport\":\"ice\"}".to_owned(),
            monotonic_ms: 5,
            queued_at: Instant::now(),
        };
        assert_eq!(
            event_json(3, &event),
            "{\"sequence\":3,\"monotonicMs\":5,\"type\":\"PLAYER_LOST\",\"payload\":{\"transport\":\"ice\"}}"
        );
    }
}
