// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Key maps and the command line (spec §13.2 K1-K7, U8).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::net::SocketAddr;
use std::path::PathBuf;

use rustic_isles_replay::cli::{CliCommand, USAGE, parse_args, run_with_output};
use rustic_isles_replay::{ReplayEndpoint, ReplayOptions, load_key_map};
use serde_json::json;
use sha2::{Digest, Sha256};

fn hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

fn options(line: &str) -> (ReplayOptions, bool) {
    match parse_args(&args(line)).unwrap() {
        CliCommand::Run(cli) => (cli.options, cli.json),
        CliCommand::Help => panic!("help"),
    }
}

#[test]
fn k1_k2_k3_key_maps() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path
    };
    let k1 = load_key_map(&write("k1.json", r#"["DLK_alpha_key","UUK_beta_key",7]"#)).unwrap();
    assert_eq!(k1.len(), 2);
    assert_eq!(k1[&hex("DLK_alpha_key")], "DLK_alpha_key");
    assert_eq!(k1[&hex("UUK_beta_key")], "UUK_beta_key");

    let upper = "AB".repeat(32);
    let k2 = load_key_map(&write(
        "k2.json",
        &json!({upper.clone(): "DLK_a", "name": "DLK_b", "n": 1}).to_string(),
    ))
    .unwrap();
    assert_eq!(k2.len(), 2);
    assert_eq!(k2[&upper.to_ascii_lowercase()], "DLK_a");
    assert_eq!(k2[&hex("DLK_b")], "DLK_b");

    let k3 = write("k3.json", "42");
    assert_eq!(
        load_key_map(&k3).unwrap_err().to_string(),
        format!(
            "invalid replay configuration: --key-map {} must be a JSON object or array of strings",
            k3.display()
        )
    );
    let bad = write("bad.json", "{");
    assert!(load_key_map(&bad).unwrap_err().to_string().starts_with(&format!(
        "invalid replay configuration: invalid JSON in --key-map {}: ",
        bad.display()
    )));
    let missing = dir.path().join("none.json");
    assert!(load_key_map(&missing).unwrap_err().to_string().starts_with(&format!(
        "invalid replay configuration: failed to read --key-map {}: ",
        missing.display()
    )));
}

#[test]
fn k4_full_command_line() {
    let (o, json) = options(
        "replay --corpus c.jsonl --left recorded --right http://127.0.0.1:61000 --right-director \
         http://127.0.0.1:61001 --gs-key g --jwt-secret j --gateway-secret w --filter /character --out r.json --json",
    );
    assert!(json);
    assert_eq!(o.corpus, PathBuf::from("c.jsonl"));
    assert_eq!(
        (o.left.as_str(), o.right.as_str()),
        ("recorded", "http://127.0.0.1:61000")
    );
    assert_eq!(o.right_director.as_deref(), Some("http://127.0.0.1:61001"));
    assert_eq!(
        (o.left_gs_key.as_deref(), o.right_gs_key.as_deref()),
        (Some("g"), Some("g"))
    );
    assert_eq!(
        (o.left_jwt_secret.as_deref(), o.right_jwt_secret.as_deref()),
        (Some("j"), Some("j"))
    );
    assert_eq!(o.gateway_secret.as_deref(), Some("w"));
    assert_eq!(o.filter.as_deref(), Some("/character"));
    assert_eq!(o.out_path, Some(PathBuf::from("r.json")));
    assert!(o.pin_clock);
    let (o, json) = options("--corpus c --left a --right b --no-pin-clock");
    assert!(!o.pin_clock && !json);
    let (o, _) = options("--corpus c --left a --right b --no-pin-clock --pin-clock");
    assert!(o.pin_clock);
    assert!(matches!(
        parse_args(&args("--corpus c --bogus -h")),
        Ok(CliCommand::Help)
    ));
}

#[test]
fn k5_argument_order_wins() {
    let base = "--corpus c --left a --right b";
    let (o, _) = options(&format!("{base} --left-jwt-secret a --jwt-secret b"));
    assert_eq!(
        (o.left_jwt_secret.as_deref(), o.right_jwt_secret.as_deref()),
        (Some("b"), Some("b"))
    );
    let (o, _) = options(&format!("{base} --jwt-secret b --left-jwt-secret a"));
    assert_eq!(
        (o.left_jwt_secret.as_deref(), o.right_jwt_secret.as_deref()),
        (Some("a"), Some("b"))
    );
    let (o, _) = options(&format!("{base} --signing-key s --right-signing-key t --left-gs-key x"));
    assert_eq!(
        (o.left_signing_key.as_deref(), o.right_signing_key.as_deref()),
        (Some("s"), Some("t"))
    );
    assert_eq!((o.left_gs_key.as_deref(), o.right_gs_key.as_deref()), (Some("x"), None));
}

fn run(line: &str) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut argv = vec!["/usr/bin/isles-db".to_owned()];
    argv.extend(args(line));
    let code = run_with_output(&argv, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn k6_usage_errors() {
    for (line, message) in [
        ("replay --corpus c --left recorded", "missing required --right <url>"),
        ("replay --left recorded --right x", "missing required --corpus <path>"),
        ("replay --corpus c --right x", "missing required --left <url|recorded>"),
        ("replay --bogus", "unexpected flag \"--bogus\""),
        (
            "replay --corpus c --left l --right r --out",
            "--out requires a file path",
        ),
        ("replay --left", "--left requires a URL or 'recorded'"),
        ("replay --right-chat", "--right-chat requires a host:port"),
        ("replay --left-signing-key", "--left-signing-key requires a base64 key"),
        ("replay --gs-key", "--gs-key requires a key"),
        ("replay --gateway-secret", "--gateway-secret requires a secret"),
        ("replay --filter", "--filter requires a substring"),
        ("replay --key-map", "--key-map requires a path"),
        ("replay --left-director", "--left-director requires a URL"),
    ] {
        let (code, out, err) = run(line);
        assert_eq!(code, 2, "{line}");
        assert!(out.is_empty());
        assert_eq!(err, format!("error: {message}\n\n{USAGE}\n"), "{line}");
    }
    let (code, out, _) = run("replay --help");
    assert_eq!((code, out.as_str()), (0, USAGE));
    let (code, _, err) = run("replay --corpus /definitely/missing.jsonl --left recorded --right http://127.0.0.1:1");
    assert_eq!(code, 1);
    assert!(
        err.starts_with("isles-db replay failed: invalid replay configuration: I/O error at "),
        "{err}"
    );
}

/// A target on its own thread and runtime: the CLI blocks on a runtime of its own.
fn spawn_target() -> SocketAddr {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let server = rustic_isles_replay::serve_ephemeral(common::target(true))
                .await
                .unwrap();
            tx.send(server.addr).unwrap();
            std::future::pending::<()>().await;
        });
    });
    rx.recv().unwrap()
}

#[test]
fn u8_exit_codes_and_text() {
    let addr = spawn_target();
    let dir = tempfile::tempdir().unwrap();
    let corpus = |name: &str, ok: bool| {
        let path = dir.path().join(name);
        let line = json!({"kind":"http","t":"2026-10-22T17:00:00.000Z","conn":1,"seq":1,"method":"GET",
            "target":"/fixed","status":200,"resHeaders":{"content-type":"application/json"},
            "resBody":{"json":{"ok":ok}}});
        std::fs::write(&path, line.to_string()).unwrap();
        path.display().to_string()
    };
    let pass = corpus("pass.jsonl", true);
    let (code, out, _) = run(&format!("replay --corpus {pass} --left recorded --right http://{addr}"));
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out,
        "isles-db replay: PASS (records=1, http=1, chat_conns=0, skipped=0, exact_matched=1, inexact_matched=0, \
         inexact_diffed=0, unexplained_diffed=0)\n  diffs by kind: status=0, header=0, value=0, text=0, chat_stanza=0\n"
    );

    let fail = corpus("fail.jsonl", false);
    let out_file = dir.path().join("r.json");
    let (code, out, _) = run(&format!(
        "replay --corpus {fail} --left recorded --right http://{addr} --out {}",
        out_file.display()
    ));
    assert_eq!(code, 2);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].starts_with("isles-db replay: DIFF (records=1, http=1,"),
        "{out}"
    );
    assert_eq!(
        lines[1],
        "  diffs by kind: status=0, header=0, value=1, text=0, chat_stanza=0"
    );
    assert_eq!(
        lines[2],
        "  [DIFF] #0 GET /fixed (conn=1, seq=1, route=backend): 1 diff(s)"
    );
    assert_eq!(
        lines[3],
        "    - ValueDiff { path: \"$.ok\", left: Bool(false), right: Bool(true) }"
    );
    assert!(out_file.exists());

    let (code, out, _) = run(&format!(
        "replay --corpus {fail} --left recorded --right http://{addr} --json"
    ));
    assert_eq!(code, 2);
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["unexplained_diffed"], 1);
}

#[test]
fn k7_debug_shows_no_secrets() {
    let (o, _) = options(
        "--corpus c --left recorded --right http://127.0.0.1:61000 --jwt-secret JWTSECRET --signing-key SIGNKEY \
         --gs-key GSKEY --gateway-secret GWSECRET",
    );
    let shown = format!("{o:?}");
    for secret in ["JWTSECRET", "SIGNKEY", "GSKEY", "GWSECRET"] {
        assert!(!shown.contains(secret), "{secret} in {shown}");
    }
    assert!(shown.contains("http://127.0.0.1:61000"));
    let endpoint = ReplayEndpoint::Router {
        backend: common::target(true),
        director: None,
    };
    assert_eq!(format!("{endpoint:?}"), "Router { director: false, .. }");
}
