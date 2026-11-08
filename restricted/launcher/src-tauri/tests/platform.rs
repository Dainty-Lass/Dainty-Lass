//! dainty-launcher-kit's folders, external links and system checks (spec §2.7.3, §2.7.9, §2.7.14), as the shell's
//! `platform/{paths,links,system}.rs` tested them, Reforged's `links.test.ts` included.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};

use dainty_launcher_kit::bridge::spec::EXTERNAL_TARGETS;
use dainty_launcher_kit::bridge::types::ExternalTarget;
use dainty_launcher_kit::platform::links::{check, fixed_url, fixed_urls};
use dainty_launcher_kit::platform::paths::{
    APP_FOLDER, DOCUMENTS_FOLDER, LEGACY_APP_FOLDER, LEGACY_DOCUMENTS_FOLDER, Paths,
};
use dainty_launcher_kit::platform::system::{free_bytes, normalize_device_signals, tailscale_candidates};

// ---------------------------------------------------------------- paths.rs

#[test]
fn the_electron_folders() {
    let paths = Paths::resolve().unwrap();
    assert!(paths.user_data.ends_with(APP_FOLDER));
    assert_eq!(paths.keys(), paths.user_data.join("keys"));
    assert_eq!(paths.logs(), paths.user_data.join("logs"));
    assert!(
        paths
            .recovery()
            .ends_with(Path::new(DOCUMENTS_FOLDER).join("Account Keys"))
    );
    if cfg!(windows) && std::env::var_os(dainty_launcher_kit::platform::paths::DEV_PROFILE_VAR).is_none() {
        let appdata = PathBuf::from(std::env::var_os("APPDATA").unwrap());
        assert_eq!(paths.user_data, appdata.join(APP_FOLDER));
    }
}

#[test]
fn migrates_legacy_user_data_and_documents_folders_without_losing_or_overwriting_data() {
    let temp = tempfile::tempdir().unwrap();
    let config_root = temp.path().join("Roaming");
    let docs_root = temp.path().join("Documents");
    let paths = Paths {
        user_data: config_root.join(APP_FOLDER),
        documents: docs_root.clone(),
        home: temp.path().to_path_buf(),
    };

    let legacy_user_data = config_root.join(LEGACY_APP_FOLDER);
    let new_user_data = paths.user_data.clone();
    std::fs::create_dir_all(legacy_user_data.join("keys")).unwrap();
    std::fs::create_dir_all(legacy_user_data.join("Server/.workspace/config")).unwrap();
    std::fs::write(legacy_user_data.join("settings.json"), b"{\"language\":\"en\"}").unwrap();
    std::fs::write(legacy_user_data.join("keys/abc.key2"), b"encrypted-key-1").unwrap();
    std::fs::write(
        legacy_user_data.join("Server/.workspace/config/host.toml"),
        b"mode = \"private\"\n",
    )
    .unwrap();

    let legacy_keys = docs_root.join(LEGACY_DOCUMENTS_FOLDER).join("Account Keys");
    std::fs::create_dir_all(&legacy_keys).unwrap();
    std::fs::write(legacy_keys.join("abc.txt"), b"DLK_legacy_1").unwrap();

    // 1. Fresh migration moves legacy folders into DaintyLass names
    paths.migrate_legacy();
    assert!(new_user_data.join("settings.json").is_file());
    assert_eq!(
        std::fs::read_to_string(new_user_data.join("keys/abc.key2")).unwrap(),
        "encrypted-key-1"
    );
    assert!(new_user_data.join("Server/.workspace/config/host.toml").is_file());
    let new_keys = paths.recovery();
    assert_eq!(
        std::fs::read_to_string(new_keys.join("abc.txt")).unwrap(),
        "DLK_legacy_1"
    );

    // 2. If both legacy and new folders exist, missing files merge without overwriting newer files
    std::fs::create_dir_all(legacy_user_data.join("keys")).unwrap();
    std::fs::write(legacy_user_data.join("keys/abc.key2"), b"stale-key").unwrap();
    std::fs::write(legacy_user_data.join("keys/def.key2"), b"encrypted-key-2").unwrap();
    std::fs::create_dir_all(&legacy_keys).unwrap();
    std::fs::write(legacy_keys.join("abc.txt"), b"DLK_stale").unwrap();
    std::fs::write(legacy_keys.join("def.txt"), b"DLK_legacy_2").unwrap();

    paths.migrate_legacy();
    for (file, text) in [
        (new_user_data.join("keys/abc.key2"), "encrypted-key-1"),
        (new_user_data.join("keys/def.key2"), "encrypted-key-2"),
        (new_keys.join("abc.txt"), "DLK_legacy_1"),
        (new_keys.join("def.txt"), "DLK_legacy_2"),
    ] {
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "{}", file.display());
    }
}

// ---------------------------------------------------------------- links.rs

/// `links.test.ts`: "every fixed link is plain https"
#[test]
fn every_fixed_link_is_plain_https() {
    let mut fixed = 0;
    for name in EXTERNAL_TARGETS {
        let target: ExternalTarget = serde_json::from_value(serde_json::Value::from(*name)).unwrap();
        let Some(url) = fixed_url(target) else {
            assert!(["tailscale_share", "server_source"].contains(name), "{name}");
            continue;
        };
        fixed += 1;
        let parsed = url::Url::parse(url).unwrap();
        assert_eq!(parsed.scheme(), "https", "{name}");
        assert!(parsed.username().is_empty() && parsed.password().is_none(), "{name}");
        assert!(check(url, None).is_some(), "{name}");
    }
    assert_eq!(fixed, fixed_urls().count());
}

/// `links.test.ts`: "external URLs: fixed ones, Tailscale share links and the host's own https source link only"
#[test]
fn external_urls() {
    let allowed = |url: &str, source: Option<&str>| check(url, source).is_some();
    assert!(allowed("https://login.tailscale.com/admin/invite/abc", None));
    assert!(!allowed("http://login.tailscale.com/admin/invite/abc", None));
    assert!(!allowed("https://evil.example/", None));
    let source = Some("https://git.example/me/server");
    assert!(allowed("https://git.example/me/server", source));
    assert!(!allowed("https://git.example/me/other", source));
    assert!(!allowed("javascript:alert(1)", Some("javascript:alert(1)")));
    assert!(!allowed(
        "https://user:pw@git.example/",
        Some("https://user:pw@git.example/")
    ));
    assert!(allowed(
        "https://git.example:8443/me",
        Some("https://git.example:8443/me")
    ));
    assert!(!allowed("https://github.com/SilentSpud/DaintyLass/x", None));
}

// ---------------------------------------------------------------- system.rs

#[test]
fn device_signals_keep_known_formats_and_drop_placeholders() {
    let uuid = "12345678-1234-1234-1234-123456789abc";
    let tpm = "ab".repeat(32);
    let signals = |uuid: &str, tpm: &str| normalize_device_signals(Some(uuid), Some(tpm));
    assert_eq!(
        signals(&format!(" {}\r\n", uuid.to_uppercase()), &tpm.to_uppercase()),
        (Some(uuid.into()), Some(tpm.clone()))
    );
    for bad in [
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
        "03000200-0400-0500-0006-000700080009",
        "Default string",
        "unknown",
        "not-a-uuid",
    ] {
        assert_eq!(signals(bad, "").0, None, "{bad}");
    }
    for bad in [
        "0".repeat(64),
        "f".repeat(64),
        "g".repeat(64),
        "ab".into(),
        format!("{tpm}0"),
    ] {
        assert_eq!(signals("", &bad).1, None, "{bad}");
    }
    assert_eq!(signals(uuid, ""), (Some(uuid.into()), None));
}

#[test]
fn free_space_of_a_missing_folder_is_its_parents() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(&dir.path().join("not").join("yet")).is_some_and(|free| free > 0));
}

#[test]
fn tailscale_is_looked_for_in_the_usual_places() {
    let candidates = tailscale_candidates(&|name| std::env::var_os(name));
    if cfg!(windows) {
        assert!(
            candidates
                .iter()
                .any(|c| c.ends_with(Path::new("Tailscale").join("tailscale.exe")))
        );
    } else {
        assert_eq!(candidates[0], PathBuf::from("/usr/bin/tailscale"));
    }
}
