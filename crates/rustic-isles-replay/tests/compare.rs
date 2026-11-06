// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Normalization and comparison (spec §13.2 N3-N9, R1-R2, C1-C5).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use rustic_isles_replay::{
    DiffKind, IdNormalizer, NormalizerConfig, ReplayHttpResponse, ReplayTargetAuth, compare_chat_stanzas,
    compare_http_responses, is_filtered_chat_stanza, mint_hs256_jwt, normalize_chat_stanza,
};
use serde_json::json;

fn answer(status: u16, headers: &[(&str, &str)], body: &str) -> ReplayHttpResponse {
    ReplayHttpResponse {
        status,
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        body_text: body.to_owned(),
    }
}

fn pair() -> (IdNormalizer, IdNormalizer) {
    (
        IdNormalizer::new(NormalizerConfig::default()),
        IdNormalizer::new(NormalizerConfig::default()),
    )
}

fn compare(left: &ReplayHttpResponse, right: &ReplayHttpResponse) -> Vec<DiffKind> {
    let (mut l, mut r) = pair();
    compare_http_responses(left, right, &mut l, &mut r)
}

#[test]
fn n3_n4_volatile_values_and_co_reference() {
    let rs = ReplayTargetAuth::with_test_keys("g")
        .mint_token("UID-p", 1_792_688_400_000)
        .unwrap();
    let hs = mint_hs256_jwt("UID-p", "s", 1_792_688_400_000);
    let body = |token: &str, created: &str, id: &str, iid: &str, party: &str, grant: &str, up: u32, pid: u32| {
        json!({"token": token, "createdAt": created, "id": id, "instanceId": iid, "partyId": party,
            "grantId": grant, "uptimeSeconds": up, "processId": pid})
        .to_string()
    };
    let left = answer(
        200,
        &[
            ("date", "Thu, 22 Oct 2026 17:00:00 GMT"),
            ("etag", "W/\"1\""),
            ("x-powered-by", "Express"),
            ("x-rustic-isles-request-id", "r-1"),
            ("content-type", "application/json; charset=UTF-8"),
            ("x-mystic-party-revision", "3"),
        ],
        &body(
            &rs,
            "2026-10-22T17:00:00.000Z",
            "0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69",
            "aaaa1111bbbb2222",
            &format!("local-{}_us-east-1", "ab12".repeat(6)),
            "gs-1c8f7b45-2d2e-4c1a-8b2d-0e1c2b3a4968",
            120,
            4412,
        ),
    );
    let right = answer(
        200,
        &[
            ("date", "Fri, 23 Oct 2026 09:00:00 GMT"),
            ("content-length", "321"),
            ("x-rustic-isles-request-id", "r-2"),
            ("content-type", "application/json; charset=utf-8"),
            ("x-mystic-party-revision", "3"),
        ],
        &body(
            &hs,
            "2026-11-05T08:30:00.500Z",
            "7d6c5b4a-3928-4716-a5b4-c3d2e1f00918",
            "cccc3333dddd4444",
            &format!("local-{}_us-east-1", "cd34".repeat(6)),
            "gs-2a1b0c9d-8e7f-4a6b-9c5d-4e3f2a1b0c9d",
            7,
            99,
        ),
    );
    let (mut l, mut r) = pair();
    assert_eq!(compare_http_responses(&left, &right, &mut l, &mut r), []);

    // N4, with the same normalizers: the left repeats its id, the right names its own and then a new one
    let u1 = "0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69";
    let left = answer(200, &[], &json!({"primary": u1, "secondary": u1}).to_string());
    let right = answer(
        200,
        &[],
        &json!({"primary": "7d6c5b4a-3928-4716-a5b4-c3d2e1f00918", "secondary": "66666666-7777-4888-9999-aaaaaaaaaaaa"})
            .to_string(),
    );
    let diffs = compare_http_responses(&left, &right, &mut l, &mut r);
    assert_eq!(
        diffs,
        [DiffKind::ValueDiff {
            path: "$.secondary".into(),
            left: json!("<uuid#1>"),
            right: json!("<uuid#2>"),
        }]
    );
}

#[test]
fn n4_fresh_normalizers() {
    let u1 = "0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69";
    let left = answer(200, &[], &json!({"primary": u1, "secondary": u1}).to_string());
    let right = answer(
        200,
        &[],
        &json!({"primary": "11111111-2222-4333-8444-555555555555", "secondary": "66666666-7777-4888-9999-aaaaaaaaaaaa"})
            .to_string(),
    );
    assert_eq!(
        compare(&left, &right),
        [DiffKind::ValueDiff {
            path: "$.secondary".into(),
            left: json!("<uuid#1>"),
            right: json!("<uuid#2>"),
        }]
    );
}

#[test]
fn n5_status_and_content_type() {
    let diffs = compare(
        &answer(200, &[("content-type", "application/json")], "{}"),
        &answer(500, &[("content-type", "text/html")], "{}"),
    );
    assert_eq!(
        diffs,
        [
            DiffKind::StatusDiff { left: 200, right: 500 },
            DiffKind::HeaderDiff {
                header: "content-type".into(),
                left: Some("application/json".into()),
                right: Some("text/html".into()),
            },
        ]
    );
    let one_sided = compare(
        &answer(200, &[("cache-control", "no-store")], ""),
        &answer(200, &[], ""),
    );
    assert_eq!(
        one_sided,
        [DiffKind::HeaderDiff {
            header: "cache-control".into(),
            left: Some("no-store".into()),
            right: None
        }]
    );
}

#[test]
fn n6_n7_n8_raw_json_text() {
    let n6 = compare(
        &answer(200, &[], r#"{"alpha":1,"beta":2}"#),
        &answer(200, &[], r#"{"beta":2,"alpha":1}"#),
    );
    assert_eq!(
        n6,
        [DiffKind::TextDiff {
            left: r#"{"alpha":1,"beta":2}"#.into(),
            right: r#"{"beta":2,"alpha":1}"#.into(),
        }]
    );
    let n7 = compare(&answer(200, &[], r#"{"a":1}"#), &answer(200, &[], r#"{ "a": 1 }"#));
    assert_eq!(
        n7,
        [DiffKind::TextDiff {
            left: r#"{"a":1}"#.into(),
            right: r#"{ "a": 1 }"#.into(),
        }]
    );
    let n8 = compare(&answer(200, &[], r#"{"n":1}"#), &answer(200, &[], r#"{"n":1.0}"#));
    assert_eq!(
        n8,
        [DiffKind::TextDiff {
            left: r#"{"n":1}"#.into(),
            right: r#"{"n":1.0}"#.into(),
        }]
    );
    // With the switch off, only values count
    let mut off = IdNormalizer::new(NormalizerConfig {
        check_raw_json_text: false,
        ..NormalizerConfig::default()
    });
    let mut right = IdNormalizer::default();
    let quiet = compare_http_responses(
        &answer(200, &[], r#"{"a":1}"#),
        &answer(200, &[], r#"{ "a": 1 }"#),
        &mut off,
        &mut right,
    );
    assert_eq!(quiet, []);
}

#[test]
fn value_paths_and_raw_bodies() {
    let diffs = compare(
        &answer(200, &[], r#"{"list":[1,2],"o":{"x":"a"},"t":true}"#),
        &answer(200, &[], r#"{"list":[1],"o":{"x":1},"z":null,"t":"true"}"#),
    );
    assert_eq!(
        diffs,
        [
            DiffKind::ValueDiff {
                path: "$.list[1]".into(),
                left: json!(2),
                right: json!(null)
            },
            DiffKind::ValueDiff {
                path: "$.o.x".into(),
                left: json!("a"),
                right: json!(1)
            },
            DiffKind::ValueDiff {
                path: "$.t".into(),
                left: json!(true),
                right: json!("true")
            },
        ]
    );
    assert_eq!(compare(&answer(204, &[], ""), &answer(204, &[], "")), []);
    let text = compare(
        &answer(200, &[], "<pre>uptime 2026-10-22T17:00:00Z</pre>"),
        &answer(200, &[], "<pre>uptime 2026-11-01T00:00:00Z</pre>"),
    );
    assert_eq!(text, []);
    let differs = compare(&answer(200, &[], "ok"), &answer(200, &[], r#"{"ok":true}"#));
    assert_eq!(
        differs,
        [DiffKind::TextDiff {
            left: "ok".into(),
            right: r#"{"ok":true}"#.into()
        }]
    );
}

#[test]
fn r1_gateway_redactions_align() {
    let recorded = json!({
        "game": {"executableSha256": "<redacted>"},
        "token_type": "<redacted>",
        "refresh_token": "<redacted>",
        "payload": {"huntpass_id": "<redacted>", "bounty_token_id": "<redacted>"},
    });
    let live = json!({
        "game": {"executableSha256": "9f".repeat(32)},
        "token_type": "bearer",
        "refresh_token": "opaque-refresh-value-0001",
        "payload": {"huntpass_id": "season43", "bounty_token_id": "TOKEN_BOUNTY_DRAFT"},
    });
    let diffs = compare(
        &answer(200, &[], &recorded.to_string()),
        &answer(200, &[], &live.to_string()),
    );
    assert_eq!(diffs, []);
    // Either side
    let diffs = compare(
        &answer(200, &[], &live.to_string()),
        &answer(200, &[], &recorded.to_string()),
    );
    assert_eq!(diffs, []);
}

#[test]
fn r2_redacted_under_another_name_differs() {
    let diffs = compare(
        &answer(200, &[], r#"{"displayName":"<redacted>"}"#),
        &answer(200, &[], r#"{"displayName":"CanarySlayer"}"#),
    );
    assert_eq!(diffs.len(), 1);
    let long = format!(r#"{{"displayName":"{}"}}"#, "a".repeat(100));
    let quiet = compare(
        &answer(200, &[], r#"{"displayName":"<redacted>"}"#),
        &answer(200, &[], &long),
    );
    assert_eq!(quiet, []);
}

#[test]
fn c1_c2_chat_filters_and_stream_ids() {
    assert!(is_filtered_chat_stanza(
        "<iq from='chat.steelyard.ca' id='srv-1' type='get'><ping xmlns='urn:xmpp:ping'/></iq>"
    ));
    assert!(is_filtered_chat_stanza("<iq type='result' id='replay-barrier-1'/>"));
    assert!(is_filtered_chat_stanza("  \n"));
    assert!(!is_filtered_chat_stanza("<presence/>"));
    assert_eq!(
        normalize_chat_stanza("<stream:stream from='chat.steelyard.ca' id='abc-123' version='1.0'>"),
        "<stream:stream from='chat.steelyard.ca' id='<stream>' version='1.0'>"
    );
    assert_eq!(
        normalize_chat_stanza(r#" <stream:stream id="x9" xmlns="jabber:client"> "#),
        r#"<stream:stream id="<stream>" xmlns="jabber:client">"#
    );
    assert_eq!(
        normalize_chat_stanza(r#"<message id="srv-ab12-cd" stamp='2026-10-22T17:00:00Z'/>"#),
        r#"<message id="srv-<id>" stamp='<date>'/>"#
    );
    assert_eq!(
        normalize_chat_stanza("<x t='2099-01-01T00:00:00Z'/>"),
        "<x t='<date>'/>"
    );
}

#[test]
fn c3_c4_c5_chat_comparison() {
    let left = [
        "<stream:stream from='chat' id='s1' version='1.0'>".to_owned(),
        "<iq id='srv-1' type='get'><ping xmlns='urn:xmpp:ping'/></iq>".to_owned(),
        "<iq id='srv-222' type='result'/>".to_owned(),
    ];
    let right = [
        "<stream:stream from='chat' id='s2' version='1.0'>".to_owned(),
        "<iq id='srv-999' type='result'/>".to_owned(),
    ];
    assert_eq!(compare_chat_stanzas(1, &left, &right), []);
    assert_eq!(
        compare_chat_stanzas(
            9,
            &["<presence from='a'/>".to_owned()],
            &["<presence from='b'/>".to_owned()]
        ),
        [DiffKind::ChatStanzaDiff {
            conn: 9,
            index: 0,
            left: Some("<presence from='a'/>".into()),
            right: Some("<presence from='b'/>".into()),
        }]
    );
    assert_eq!(
        compare_chat_stanzas(2, &["<a/>".to_owned(), "<b/>".to_owned()], &["<a/>".to_owned()]),
        [DiffKind::ChatStanzaDiff {
            conn: 2,
            index: 1,
            left: Some("<b/>".into()),
            right: None
        }]
    );
}

#[test]
fn diff_serialization_and_debug() {
    let status = DiffKind::StatusDiff { left: 200, right: 500 };
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"kind":"StatusDiff","left":200,"right":500}"#
    );
    let header = DiffKind::HeaderDiff {
        header: "content-type".into(),
        left: Some("application/json".into()),
        right: None,
    };
    assert_eq!(
        serde_json::to_string(&header).unwrap(),
        r#"{"kind":"HeaderDiff","header":"content-type","left":"application/json","right":null}"#
    );
    assert_eq!(format!("{status:?}"), "StatusDiff { left: 200, right: 500 }");
    let value = DiffKind::ValueDiff {
        path: "$.x".into(),
        left: json!("a"),
        right: json!(1),
    };
    assert_eq!(
        format!("{value:?}"),
        r#"ValueDiff { path: "$.x", left: String("a"), right: Number(1) }"#
    );
}
