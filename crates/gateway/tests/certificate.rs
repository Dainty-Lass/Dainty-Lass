// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `make-cert` makes a certificate the gateway loads, reads a fingerprint, and never replaces one without
//! `--force`. (The Node host kit's certificate, key and invite vectors stay in dainty-restricted until the
//! integration pass copies them in; docs/port/clean-room/gateway.md.)
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::ExitCode;

use gateway::cert;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn make_cert_writes_a_pair_and_refuses_to_replace_it() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tls");
    let out_text = out.display().to_string();
    let made = cert::make_cert_main(&args(&[
        "--host",
        "203.0.113.7",
        "--host",
        "Play.Example.org",
        "--out",
        &out_text,
    ]));
    assert_eq!(made, ExitCode::SUCCESS);
    let (cert_file, key_file) = (out.join("gateway-cert.pem"), out.join("gateway-key.pem"));
    let loaded = cert::load(&cert_file, &key_file).unwrap();
    assert_eq!(
        loaded.info.subject_alt_name,
        "IP Address:203.0.113.7, DNS:play.example.org"
    );
    let before = std::fs::read(&cert_file).unwrap();
    let again = cert::make_cert_main(&args(&["--host", "203.0.113.7", "--out", &out_text]));
    assert_eq!(again, ExitCode::from(2));
    assert_eq!(std::fs::read(&cert_file).unwrap(), before);
    let forced = cert::make_cert_main(&args(&["--host", "203.0.113.7", "--out", &out_text, "--force"]));
    assert_eq!(forced, ExitCode::SUCCESS);
    let replaced = cert::load(&cert_file, &key_file).unwrap();
    assert_ne!(replaced.info.fingerprint, loaded.info.fingerprint);
    let file = cert_file.display().to_string();
    assert_eq!(
        cert::make_cert_main(&args(&["--fingerprint", &file, "--json"])),
        ExitCode::SUCCESS
    );
    assert_eq!(
        cert::make_cert_main(&args(&["--fingerprint", "missing.pem"])),
        ExitCode::from(2)
    );
    assert_eq!(cert::make_cert_main(&args(&["--bogus"])), ExitCode::from(2));
    assert_eq!(cert::make_cert_main(&args(&["--help"])), ExitCode::from(2));
}

#[test]
fn the_served_certificate_is_the_one_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let (c, k) = (dir.path().join("c.pem"), dir.path().join("k.pem"));
    let (loaded, _) = cert::load_or_generate(&c, &k, &[]).unwrap();
    assert_eq!(
        cert::describe(&loaded.chain[0]).unwrap().fingerprint,
        loaded.info.fingerprint
    );
    assert!(cert::server_config(&loaded).unwrap().alpn_protocols == vec![b"http/1.1".to_vec()]);
}
