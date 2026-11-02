// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Guard publisher against a fake backend (spec §5.1 "Publisher"), with a 200 ms window and 20→80 ms retries.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rifts_transport::backend::BackendClient;
use rifts_transport::guard::{Publisher, PublisherConfig};
use serde_json::Value;

use common::CAPABILITY;
use common::backend::{Call, FakeBackend};

fn config() -> PublisherConfig {
    PublisherConfig {
        window: Duration::from_millis(200),
        first_retry: Duration::from_millis(20),
        retry_ceiling: Duration::from_millis(80),
        ..PublisherConfig::default()
    }
}

struct Rig {
    backend: FakeBackend,
    _runtime: tokio::runtime::Runtime,
    publisher: Publisher,
    lines: Arc<Mutex<Vec<String>>>,
}

fn rig(config: PublisherConfig) -> Rig {
    let backend = FakeBackend::start();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let client = Arc::new(
        BackendClient::new(
            &backend.url,
            CAPABILITY,
            "3f1c2a4e-1b2c-4d3e-8f40-000000000001",
            runtime.handle().clone(),
        )
        .unwrap(),
    );
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink_lines = lines.clone();
    let publisher = Publisher::start(
        client,
        1,
        config,
        Arc::new(move |line| sink_lines.lock().unwrap().push(line)),
    );
    Rig {
        backend,
        _runtime: runtime,
        publisher,
        lines,
    }
}

fn hash_of(batch: usize) -> String {
    let n = batch.to_string();
    format!("{}{n}", "b".repeat(64 - n.len()))
}

/// Checks the chain over the accepted batches and returns its events.
fn chain(batches: &[Call]) -> Vec<Value> {
    let mut events = Vec::new();
    let mut previous = "0".repeat(64);
    let mut next = 1;
    let mut last_ms = 0;
    for (index, call) in batches.iter().enumerate() {
        let body: Value = serde_json::from_str(&call.body).unwrap();
        assert_eq!(body["schema"], 1);
        assert_eq!(body["previousHash"], previous.as_str());
        assert_eq!(body["firstSequence"], next);
        let batch_events = body["events"].as_array().unwrap();
        assert!((1..=128).contains(&batch_events.len()));
        for (i, event) in batch_events.iter().enumerate() {
            assert_eq!(event["sequence"], next);
            next += 1;
            let ms = event["monotonicMs"].as_u64().unwrap();
            assert!(ms >= last_ms);
            last_ms = ms;
            let kind = event["type"].as_str().unwrap();
            if kind == "COMPLETION_REQUESTED" || kind == "HUNT_ABORTED" {
                assert_eq!(i, batch_events.len() - 1, "a terminal event ends its batch");
            }
            events.push(event.clone());
        }
        assert_eq!(body["lastSequence"], next - 1);
        assert_eq!(call.request_id, format!("guard-1-{}", body["firstSequence"]));
        previous = hash_of(index + 1);
    }
    events
}

#[test]
fn a_long_hunt_arrives_in_one_chain() {
    let mut rig = rig(config());
    rig.backend.set_delay(Duration::from_millis(300));
    let started = Instant::now();
    for n in 0..250 {
        assert!(rig.publisher.enqueue("TEST_EVENT", "acc-1", &format!("{{\"n\":{n}}}")));
    }
    assert!(started.elapsed() < Duration::from_millis(50));
    assert!(rig.publisher.enqueue("COMPLETION_REQUESTED", "acc-1", "{}"));
    assert!(rig.publisher.enqueue("PLAYER_LOST", "acc-1", "{\"transport\":\"ice\"}"));
    rig.publisher.flush_and_stop(Duration::from_secs(20));
    let log = rig.backend.log();
    assert!((3..=8).contains(&log.batches.len()), "{} batches", log.batches.len());
    let events = chain(&log.batches);
    assert_eq!(events.len(), 252);
    assert_eq!(log.completions.len(), 1);
    let completion: Value = serde_json::from_str(&log.completions[0].body).unwrap();
    assert!(completion["finalSequence"].as_u64().unwrap() >= 251);
    let stats = rig.publisher.stats();
    assert!(stats.completed);
    assert_eq!((stats.publish_failures, stats.evidence_loss), (0, 0));
}

#[test]
fn a_lost_batch_is_sent_again_unchanged() {
    let mut rig = rig(config());
    rig.backend.script(&[429, 0, 503]);
    for n in 0..5 {
        assert!(rig.publisher.enqueue("TEST_EVENT", "", &format!("{{\"n\":{n}}}")));
    }
    rig.publisher.flush_and_stop(Duration::from_secs(10));
    let log = rig.backend.log();
    assert_eq!(log.batches.len(), 4);
    let first: Value = serde_json::from_str(&log.batches[0].body).unwrap();
    for call in &log.batches {
        let body: Value = serde_json::from_str(&call.body).unwrap();
        assert_eq!(body["batchId"], first["batchId"]);
        assert_eq!(body["previousHash"], first["previousHash"]);
        assert_eq!(body["firstSequence"], first["firstSequence"]);
        assert_eq!(call.body.len(), log.batches[0].body.len());
    }
    assert_eq!(chain(&log.batches[3..]).len(), 5);
    let stats = rig.publisher.stats();
    assert_eq!((stats.publish_failures, stats.evidence_loss), (3, 0));
    let lines = rig.lines.lock().unwrap().clone();
    assert!(lines.iter().any(|l| l.contains("sending the same batch again")));
    assert!(lines.iter().any(|l| l.contains("was taken after 3 failed send(s)")));
}

#[test]
fn a_refused_batch_ends_the_evidence() {
    let mut rig = rig(config());
    rig.backend.script(&[409]);
    for n in 0..3 {
        assert!(rig.publisher.enqueue("TEST_EVENT", "", &format!("{{\"n\":{n}}}")));
    }
    assert!(rig.publisher.enqueue("COMPLETION_REQUESTED", "", "{}"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !rig.publisher.stats().refused && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!rig.publisher.enqueue("TEST_EVENT", "", "{}"));
    rig.publisher.flush_and_stop(Duration::from_secs(3));
    let log = rig.backend.log();
    assert_eq!(log.batches.len(), 1);
    assert!(log.completions.is_empty());
    let stats = rig.publisher.stats();
    assert!(stats.incomplete && !stats.completed);
    assert_eq!(stats.accepted_batches, 0);
    assert_eq!(stats.evidence_loss, 5);
    let lines = rig.lines.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|l| l == "GUARD_EVIDENCE_LOSS action=completion_fenced reason=batch_refused status=409")
    );
}

#[test]
fn an_overflow_is_recorded_in_the_chain() {
    let mut rig = rig(PublisherConfig {
        window: Duration::from_secs(10),
        queue: 8,
        ..config()
    });
    let taken = (0..12)
        .filter(|n| rig.publisher.enqueue("TEST_EVENT", "", &format!("{{\"n\":{n}}}")))
        .count();
    assert_eq!(taken, 8);
    let stats = rig.publisher.stats();
    assert_eq!(stats.evidence_loss, 4);
    assert!(stats.incomplete);
    rig.publisher.flush_and_stop(Duration::from_secs(10));
    let events = chain(&rig.backend.log().batches);
    assert_eq!(events.len(), 9);
    let loss = events.iter().find(|e| e["type"] == "EVIDENCE_LOSS").unwrap();
    assert_eq!(loss["payload"]["droppedEvents"], 4);
    assert_eq!(loss["payload"]["reason"], "publisher_queue_full");
    let lines = rig.lines.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|l| l == "GUARD_EVIDENCE_LOSS action=completion_fenced reason=publisher_queue_full")
    );
}
