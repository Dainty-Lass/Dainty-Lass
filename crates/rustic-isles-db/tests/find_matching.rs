// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The filtered read the legacy importer sends for one player's documents.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use rustic_isles_db::mongo::{MockMongoServer, MongoClient, MongoUri};
use serde_json::{Map, Value, json};

fn filter(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn reads_only_the_matching_documents_across_batches() {
    let server = MockMongoServer::start(2).unwrap();
    let docs = (0..7).map(|i| json!({"userId": if i % 2 == 0 { "a" } else { "b" }, "n": i}));
    server.seed("characters", docs.collect());
    let mut client = MongoClient::connect(&MongoUri::parse(&server.uri("rr")).unwrap()).unwrap();

    let mine = client
        .find_matching("characters", &filter(json!({"userId": "a"})))
        .unwrap();
    let numbers: Vec<_> = mine.iter().map(|d| d["n"].as_i64().unwrap()).collect();
    assert_eq!(numbers, [0, 2, 4, 6]);

    let some = client
        .find_matching("characters", &filter(json!({"n": {"$in": [1, 4, 9]}})))
        .unwrap();
    assert_eq!(some.len(), 2);
    assert!(
        client
            .find_matching("missing", &filter(json!({"userId": "a"})))
            .unwrap()
            .is_empty()
    );
    assert_eq!(client.find_all("characters").unwrap().len(), 7);
}

#[test]
fn refuses_a_nul_in_a_key() {
    let server = MockMongoServer::start(2).unwrap();
    let mut client = MongoClient::connect(&MongoUri::parse(&server.uri("rr")).unwrap()).unwrap();
    let err = client.find_matching("c", &filter(json!({"a\u{0}b": 1}))).unwrap_err();
    assert!(err.to_string().contains("NUL"));
}
