// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

// Nothing here logs: `log`'s own test reads the shared log file whole. The store's logic is tested in
// `tests/keystore.rs`, a process of its own.

use super::*;

#[test]
fn slot_ids_from_the_spec() {
    assert_eq!(
        slot_id(ServerMode::Private, "100.64.0.1", 61000),
        "2f1974fb8b73463e94a88aed"
    );
    assert_eq!(
        slot_id(ServerMode::Public, "203.0.113.10", 443),
        "4513d283ab25060967c23596"
    );
    assert_eq!(
        Slot::new(ServerMode::Public, "203.0.113.10", 443).id(),
        "4513d283ab25060967c23596"
    );
}

#[test]
fn slot_ids_differ_by_mode_and_are_24_hex() {
    let private = slot_id(ServerMode::Private, "example.ts.net", 61000);
    let public = slot_id(ServerMode::Public, "example.ts.net", 61000);
    assert_ne!(private, public);
    for id in [private, public] {
        assert_eq!(id.len(), 24);
        assert!(id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }
}

fn backup(fingerprint: Option<&'static str>, username: Option<&'static str>) -> Backup<'static> {
    Backup {
        server_name: "Ramsgate",
        host: "play.example.com",
        port: 443,
        fingerprint,
        username,
        saved: NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
        key: "RRK_abcdefghijklmnopqrstuvwx",
    }
}

#[test]
fn backup_text_public() {
    let fp = "ab".repeat(32);
    let fp: &'static str = Box::leak(fp.into_boxed_str());
    let text = backup_text(&backup(Some(fp), Some("Slayer")));
    let expected = [
        "DaintyLass account key",
        "",
        "Server: Ramsgate (play.example.com:443)",
        &format!("Certificate: {fp}"),
        "Username: Slayer",
        "Saved: 2026-11-06",
        "Key: RRK_abcdefghijklmnopqrstuvwx",
        "",
        BACKUP_PRIVACY,
        BACKUP_OTHER_PC,
        "",
    ]
    .join("\r\n");
    assert_eq!(text, expected);
    assert!(text.ends_with(".\r\n"));
    assert_eq!(
        protocol::account::extract_account_key(&text),
        Some("RRK_abcdefghijklmnopqrstuvwx")
    );
}

#[test]
fn backup_text_private_without_username() {
    let text = backup_text(&backup(None, None));
    assert!(!text.contains("Certificate:"));
    assert!(text.contains("\r\nUsername: -\r\n"));
    assert!(!text.replace("\r\n", "").contains('\n'));
    assert_eq!(
        protocol::account::extract_account_key(&text),
        Some("RRK_abcdefghijklmnopqrstuvwx")
    );
}

#[test]
fn backup_debug_hides_the_key() {
    assert!(!format!("{:?}", backup(None, None)).contains("RRK_"));
}

#[test]
fn write_private_replaces_and_leaves_no_tmp() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested").join("a.txt");
    write_private(&path, b"one").unwrap();
    write_private(&path, b"two").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"two");
    assert!(!tmp.path().join("nested").join("a.txt.tmp").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
