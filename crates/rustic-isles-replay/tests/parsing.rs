// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Capture parsing (spec §13.2 P1-P7).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use rustic_isles_replay::{CaptureBody, CaptureRecord, load_corpus, parse_capture_line, parse_capture_str};
use serde_json::{Value, json};

const HEX: &str = "5f0c2e8d4b1a39776e2d9c0b8a7f6e5d4c3b2a1908f7e6d5c4b3a29180706f5e";

fn p1_lines() -> String {
    [
        json!({"kind":"http","t":"2026-10-22T17:00:00.000Z","conn":3,"seq":2,"method":"POST",
            "target":"/account/api/oauth/token","reqHeaders":{"authorization":"basic <redacted>"},
            "reqBody":{"form":[["grant_type","exchange_code"],["exchange_code",format!("<key:{HEX}>")]]},
            "status":200,"resBody":{"json":{"access_token":"<token:UID-alice>"}}})
        .to_string(),
        json!({"kind":"http","t":"2026-10-22T17:00:01.000Z","conn":3,"seq":3,"method":"GET","target":"/login",
            "reqBody":null,"status":204,"resBody":null})
        .to_string(),
        json!({"kind":"chat","t":"2026-10-22T17:00:02.000Z","conn":4,"event":"open"}).to_string(),
        json!({"kind":"chat","t":"2026-10-22T17:00:03.000Z","conn":4,"dir":"in",
            "text":"<auth mechanism='PLAIN'><plain:UID-alice></auth>"})
        .to_string(),
        json!({"kind":"chat","t":"2026-10-22T17:00:04.000Z","conn":4,"dir":"out","text":"<success/>"}).to_string(),
        json!({"kind":"http","t":"2026-10-22T17:00:05.000Z","conn":5,"method":"PUT","target":"/upload",
            "reqBody":{"binary":128},"resBody":{"omitted":65536}})
        .to_string(),
        r#"{"kind":"http","t":"2026-10-22T17:00:06.000Z","conn":6,"meth"#.to_owned(),
    ]
    .join("\n")
}

#[test]
fn p1_mixed_lines() {
    let records = parse_capture_str(&p1_lines());
    assert_eq!(records.len(), 6);
    let CaptureRecord::Http(first) = &records[0] else {
        panic!()
    };
    assert_eq!(
        first.req_body,
        CaptureBody::Form(vec![
            ("grant_type".into(), "exchange_code".into()),
            ("exchange_code".into(), format!("<key:{HEX}>")),
        ])
    );
    assert!(!first.req_body.is_lossy());
    assert_eq!(first.req_headers["authorization"], "basic <redacted>");
    let CaptureRecord::Http(second) = &records[1] else {
        panic!()
    };
    assert_eq!(second.req_body, CaptureBody::Null);
    assert_eq!(
        (second.src.as_str(), second.ip.as_str(), second.route.as_str()),
        ("gateway", "127.0.0.1", "backend")
    );
    let CaptureRecord::Chat(open) = &records[2] else {
        panic!()
    };
    assert_eq!((open.conn, open.event.as_deref()), (4, Some("open")));
    let CaptureRecord::Http(upload) = &records[5] else {
        panic!()
    };
    assert_eq!(upload.req_body, CaptureBody::Binary(128));
    assert_eq!(upload.res_body, CaptureBody::Omitted(65536));
    assert!(upload.req_body.is_lossy() && upload.res_body.is_lossy());
    assert_eq!((upload.seq, upload.status, upload.ms, upload.aborted), (0, 0, 0, false));
}

#[test]
fn p2_text_bodies() {
    let line = r#"{"kind":"http","conn":1,"method":"POST","target":"/x","req_body":{"text":"hello=world"},"res_body":{"text":"<pre>Bad Request</pre>"}}"#;
    let Some(CaptureRecord::Http(r)) = parse_capture_line(line) else {
        panic!()
    };
    assert_eq!(r.req_body, CaptureBody::Text("hello=world".into()));
    assert_eq!(r.res_body, CaptureBody::Text("<pre>Bad Request</pre>".into()));
}

#[test]
fn p3_skipped_lines() {
    for line in [
        r#"{"kind":"unknown","conn":1}"#,
        r#"{"kind":"http","method":"GET","target":"/x"}"#,
        r#"{"kind":"http","conn":1}"#,
        r#"{"kind":"http","conn":-1,"method":"GET","target":"/x"}"#,
        r#"[1,2]"#,
        "   ",
    ] {
        assert!(parse_capture_line(line).is_none(), "{line}");
    }
}

#[test]
fn p4_p5_directory_order() {
    let dir = tempfile::tempdir().unwrap();
    let line = |t: &str, target: &str| json!({"kind":"http","t":t,"conn":1,"method":"GET","target":target}).to_string();
    std::fs::write(
        dir.path().join("b.jsonl"),
        [
            line("2026-10-22T17:00:02.000Z", "/b"),
            line("2026-10-22T17:00:00.000Z", "/b-same"),
        ]
        .join("\n"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("a.jsonl"),
        [
            line("2026-10-22T17:00:01.000Z", "/a"),
            line("2026-10-22T17:00:00.000Z", "/a-same-1"),
            line("2026-10-22T17:00:00.000Z", "/a-same-2"),
        ]
        .join("\n"),
    )
    .unwrap();
    std::fs::write(dir.path().join("c.txt"), line("2026-10-22T16:00:00.000Z", "/c")).unwrap();
    std::fs::write(dir.path().join("d.JSONL"), line("2026-10-22T16:00:00.000Z", "/d")).unwrap();
    let targets: Vec<String> = load_corpus(dir.path())
        .unwrap()
        .into_iter()
        .map(|r| match r {
            CaptureRecord::Http(h) => h.target,
            CaptureRecord::Chat(_) => unreachable!(),
        })
        .collect();
    assert_eq!(targets, ["/a-same-1", "/a-same-2", "/b-same", "/a", "/b"]);

    let file = dir.path().join("c.txt");
    assert_eq!(load_corpus(&file).unwrap().len(), 1);
    let missing = dir.path().join("missing.jsonl");
    let error = load_corpus(&missing).unwrap_err().to_string();
    assert!(
        error.starts_with(&format!("I/O error at {}: ", missing.display())),
        "{error}"
    );
}

#[test]
fn p6_camel_and_snake_case() {
    let camel =
        r#"{"kind":"http","conn":1,"method":"GET","target":"/x","reqHeaders":{"a":"1"},"resBody":{"json":{"k":1}}}"#;
    let snake =
        r#"{"kind":"http","conn":1,"method":"GET","target":"/x","req_headers":{"a":"1"},"res_body":{"json":{"k":1}}}"#;
    assert_eq!(parse_capture_line(camel), parse_capture_line(snake));
    let both = r#"{"kind":"http","conn":1,"method":"GET","target":"/x","reqBody":{"text":"camel"},"req_body":{"text":"snake"}}"#;
    let Some(CaptureRecord::Http(r)) = parse_capture_line(both) else {
        panic!()
    };
    assert_eq!(r.req_body, CaptureBody::Text("snake".into()));
}

#[test]
fn p7_json_null_is_a_json_body() {
    assert_eq!(
        CaptureBody::from_wire(Some(&json!({"json": null}))),
        CaptureBody::Json(Value::Null)
    );
    assert_eq!(CaptureBody::from_wire(Some(&json!("text"))), CaptureBody::Null);
    assert_eq!(CaptureBody::from_wire(Some(&json!({"other": 1}))), CaptureBody::Null);
    assert_eq!(
        CaptureBody::from_wire(Some(&json!({"form": [["a", 1], ["b"], "c", ["d", "e"]]}))),
        CaptureBody::Form(vec![("a".into(), String::new()), ("d".into(), "e".into())])
    );
    for wire in [
        json!(null),
        json!({"json": {"a": [1]}}),
        json!({"form": [["k", "v"]]}),
        json!({"text": "t"}),
        json!({"binary": 3}),
        json!({"omitted": 4}),
    ] {
        assert_eq!(CaptureBody::from_wire(Some(&wire)).to_wire(), wire);
    }
}

#[test]
fn chat_fields_of_the_wrong_type_read_as_absent() {
    let Some(CaptureRecord::Chat(c)) =
        parse_capture_line(r#"{"kind":"chat","conn":2,"dir":5,"text":"x","omitted":"big","via":"ws"}"#)
    else {
        panic!()
    };
    assert_eq!(
        (c.dir.as_deref(), c.text.as_deref(), c.omitted, c.via.as_deref()),
        (None, Some("x"), None, Some("ws"))
    );
    assert_eq!(c.timestamp_ms(), None);
}
