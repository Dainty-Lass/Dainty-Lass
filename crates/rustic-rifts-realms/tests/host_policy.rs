// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Host policy: artifact hashes, launcher versions, eligibility and the score.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;

use rustic_rifts_realms::host_policy::{
    ArtifactSources, EligibilityPolicy, artifact_hashes_match, host_ineligibility_reason, hunt_drain_cap_ms,
    read_artifact_policy, score_host, version_at_least,
};
use rustic_rifts_realms::hunt::{ArtifactHashes, HostPresenceRecord, HostReliabilityRecord};
use rustic_rifts_seam::UnixMillis;
use serde_json::json;

fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

const GAME_EXE: &str = "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b";

fn feed() -> ArtifactSources {
    ArtifactSources {
        runtime_manifest: Some(json!({
            "targetChangelist": 647472,
            "sha256": sha('B'),
            "extraFiles": [{ "name": "WINMM.dll", "sha256": sha('c') }],
        })),
        guard_artifacts: Some(vec![
            ("TRANSPORT_MODULE".to_owned(), sha('d')),
            ("ADMISSION_KEY".to_owned(), sha('e')),
        ]),
    }
}

#[test]
fn the_six_hashes_from_the_feeds() {
    let policy = read_artifact_policy(&settings(&[("TARGET_CHANGELIST", "647472")]), &feed()).unwrap();
    let h = policy.hashes;
    assert_eq!(h.game_exe, GAME_EXE);
    assert_eq!(h.internal_server, sha('b'));
    assert_eq!(h.winmm_proxy, sha('c'));
    // The ICE transport has no runtime of its own.
    assert_eq!((h.steam_bridge.clone(), h.steam_runtime.clone()), (sha('d'), sha('d')));
    assert_eq!(h.admission_public_key, sha('e'));
    // Unpinned, the MIT runtime is expected as the zero hash (RT-5)
    assert_eq!(h.mit_runtime, "0".repeat(64));
    assert!(policy.stale_pins.is_empty());
    let mut pinned = feed();
    pinned
        .guard_artifacts
        .as_mut()
        .unwrap()
        .push(("TRANSPORT_RUNTIME_MIT".to_owned(), sha('F')));
    let policy = read_artifact_policy(&settings(&[("TARGET_CHANGELIST", "647472")]), &pinned).unwrap();
    assert_eq!(policy.hashes.mit_runtime, sha('f'));
}

#[test]
fn pins_win_and_older_names_count() {
    let pinned = settings(&[
        ("TARGET_CHANGELIST", "647472"),
        ("P2P_STEAM_BRIDGE_SHA256", &sha('A')),
        ("P2P_INTERNAL_SERVER_SHA256", &sha('b')),
    ]);
    let policy = read_artifact_policy(&pinned, &feed()).unwrap();
    assert_eq!(policy.hashes.steam_bridge, sha('a'));
    assert_eq!(policy.stale_pins.len(), 1);
    assert!(policy.stale_pins[0].starts_with("[P2PArtifacts] P2P_STEAM_BRIDGE_SHA256 pins aaaaaaaaaaaa..."));
    assert_eq!(
        read_artifact_policy(
            &settings(&[("TARGET_CHANGELIST", "647472"), ("P2P_WINMM_SHA256", "nope")]),
            &feed()
        ),
        Err("Missing or invalid P2P_WINMM_SHA256.".to_owned())
    );
    // Without a target the game exe has no known hash; the first missing artifact is the error.
    assert_eq!(
        read_artifact_policy(&settings(&[]), &feed()),
        Err("Missing or invalid P2P_GAME_EXE_SHA256.".to_owned())
    );
}

#[test]
fn another_changelist_leaves_the_runtime_feed_out() {
    let s = settings(&[("TARGET_CHANGELIST", "647472")]);
    let mut other = feed();
    other.runtime_manifest = Some(json!({ "targetChangelist": 1, "sha256": sha('b') }));
    assert_eq!(
        read_artifact_policy(&s, &other),
        Err("Missing or invalid P2P_INTERNAL_SERVER_SHA256.".to_owned())
    );
    let a = ArtifactHashes {
        game_exe: sha('a'),
        internal_server: sha('a'),
        winmm_proxy: sha('a'),
        steam_bridge: sha('a'),
        steam_runtime: sha('a'),
        admission_public_key: sha('a'),
        mit_runtime: sha('a'),
    };
    let upper = ArtifactHashes {
        game_exe: sha('A'),
        ..a.clone()
    };
    assert!(artifact_hashes_match(&upper, &a));
    assert!(!artifact_hashes_match(
        &ArtifactHashes::default(),
        &ArtifactHashes::default()
    ));
}

#[test]
fn launcher_versions() {
    assert!(version_at_least("0.1.35", "0.1.35"));
    assert!(version_at_least(" 0.2.0 ", "0.1.35"));
    assert!(!version_at_least("0.1.34", "0.1.35"));
    assert!(!version_at_least("0.1.35-beta", "0.1.35"));
    assert!(version_at_least("0.1.35+build.1", "0.1.35"));
    assert!(version_at_least("1.0.0-beta.11", "1.0.0-beta.2"));
    assert!(version_at_least("1.0.0-beta", "1.0.0-alpha"));
    assert!(!version_at_least("1.0.0-alpha", "1.0.0-alpha.1"));
    assert!(!version_at_least("1.0.0-1", "1.0.0-alpha"));
    assert!(!version_at_least("unknown", "0.1.35"));
    assert!(!version_at_least("1.0", "0.1.35"));
}

#[test]
fn eligibility_settings_and_the_drain_cap() {
    let p = EligibilityPolicy::from_settings(&settings(&[
        ("P2P_HOST_MIN_LAUNCHER_VERSION", "0.2"),
        ("P2P_HOST_LEASE_LIFETIME_MS", "4500.9"),
        ("P2P_MAX_RECONNECTS", "11"),
        ("P2P_HOST_MAX_CPU_PERCENT", "0x32"),
    ]));
    assert_eq!(p.minimum_launcher_version, "0.1.35");
    assert_eq!(p.lease_lifetime_ms, 4500);
    assert_eq!(p.maximum_reconnects, 3);
    assert_eq!(p.maximum_cpu_load_percent, 50.0);
    assert_eq!(EligibilityPolicy::default().suspect_lifetime_ms, 15_000);
    assert_eq!(hunt_drain_cap_ms(&settings(&[])), None);
    assert_eq!(
        hunt_drain_cap_ms(&settings(&[("P2P_HUNT_DRAIN_GRACE_MS", "60000.5")])),
        Some(60_000)
    );
    assert_eq!(hunt_drain_cap_ms(&settings(&[("P2P_HUNT_DRAIN_GRACE_MS", "10")])), None);
}

fn presence() -> HostPresenceRecord {
    HostPresenceRecord {
        account_id: "host".into(),
        revision: 1,
        consent_to_host: true,
        observed_at: UnixMillis(1000),
        expires_at: UnixMillis(100_000),
        free_physical_memory_mib: 8192.0,
        commit_headroom_mib: 12_288.0,
        logical_cpu_count: 8.0,
        cpu_load_percent: 20.0,
        power_saver: false,
        runtime_supported: true,
        steam_runtime_ready: true,
        steam_id: None,
        backend_rtt_ms: 10.0,
        worst_peer_rtt_ms: 50.0,
        jitter_ms: 8.0,
        packet_loss_percent: 1.0,
        artifacts: ArtifactHashes::default(),
        artifacts_verified: true,
        launcher_version: "0.1.70".into(),
        guard_session_id: None,
        guard_risk_score: None,
        guard_verified: Some(true),
        guard_roles: Vec::new(),
        realm_host_sessions: None,
    }
}

#[test]
fn eligibility_in_order() {
    let policy = EligibilityPolicy::default();
    let check = |f: fn(&mut HostPresenceRecord)| {
        let mut p = presence();
        f(&mut p);
        host_ineligibility_reason(&p, None, 0, &policy, true, UnixMillis(2000))
    };
    assert_eq!(check(|_| {}), None);
    assert_eq!(
        check(|p| {
            p.consent_to_host = false;
            p.launcher_version = "0.0.1".into();
        }),
        Some("HOST_CONSENT_REQUIRED")
    );
    assert_eq!(
        check(|p| p.launcher_version = "0.0.1".into()),
        Some("HOST_LAUNCHER_OUTDATED")
    );
    assert_eq!(
        check(|p| p.steam_runtime_ready = false),
        Some("HOST_RUNTIME_UNVERIFIED")
    );
    assert_eq!(check(|p| p.guard_verified = None), Some("HOST_GUARD_UNVERIFIED"));
    assert_eq!(
        check(|p| p.observed_at = UnixMillis(-30_000)),
        Some("HOST_PRESENCE_STALE")
    );
    assert_eq!(check(|p| p.commit_headroom_mib = 100.0), Some("HOST_MEMORY_LOW"));
    assert_eq!(check(|p| p.power_saver = true), Some("HOST_CPU_UNSUITABLE"));
    assert_eq!(check(|p| p.jitter_ms = 81.0), Some("HOST_NETWORK_UNSUITABLE"));
    let p = presence();
    assert_eq!(
        host_ineligibility_reason(&p, None, 1, &policy, true, UnixMillis(2000)),
        Some("HOST_ALREADY_ACTIVE")
    );
    let mut cooled = HostReliabilityRecord::new("host", UnixMillis(0));
    cooled.cooldown_until = Some(UnixMillis(2001));
    assert_eq!(
        host_ineligibility_reason(&p, Some(&cooled), 0, &policy, true, UnixMillis(2000)),
        Some("HOST_COOLDOWN")
    );
}

#[test]
fn scores_order_hosts() {
    let policy = EligibilityPolicy::default();
    let (good, breakdown) = score_host(&presence(), None, &policy, 0);
    assert_eq!(breakdown.stability_risk, 25.0);
    assert_eq!(breakdown.integrity_risk, 0.0);
    assert_eq!(good, (good * 1000.0).round() / 1000.0);
    let mut laggy = presence();
    laggy.worst_peer_rtt_ms = 240.0;
    assert!(score_host(&laggy, None, &policy, 0).0 > good);
    let mut failing = HostReliabilityRecord::new("host", UnixMillis(0));
    failing.starts_failed = 3;
    assert_eq!(
        score_host(&presence(), Some(&failing), &policy, 0).1.stability_risk,
        100.0
    );
    assert_eq!(score_host(&presence(), None, &policy, 1).1.host_load_risk, 100.0);
    let mut unverified = presence();
    unverified.artifacts_verified = false;
    assert_eq!(score_host(&unverified, None, &policy, 0).1.integrity_risk, 100.0);
}

#[test]
fn an_unpublished_transport_expects_the_zero_hash() {
    let zero = sha('0');
    let mut unpublished = feed();
    unpublished.guard_artifacts = Some(vec![("ADMISSION_KEY".to_owned(), sha('e'))]);
    let s = settings(&[("TARGET_CHANGELIST", "647472")]);
    let h = read_artifact_policy(&s, &unpublished).unwrap().hashes;
    // The launcher links the transport in and reports zeros for both roles (spec-paradox-transport C2).
    assert_eq!((h.steam_bridge, h.steam_runtime), (zero.clone(), zero));
    // A pin still wins, and a bad pin is still refused.
    let pinned = settings(&[
        ("TARGET_CHANGELIST", "647472"),
        ("P2P_TRANSPORT_MODULE_SHA256", &sha('d')),
    ]);
    assert_eq!(
        read_artifact_policy(&pinned, &unpublished).unwrap().hashes.steam_bridge,
        sha('d')
    );
    let bad = settings(&[("TARGET_CHANGELIST", "647472"), ("P2P_TRANSPORT_MODULE_SHA256", "nope")]);
    assert_eq!(
        read_artifact_policy(&bad, &unpublished),
        Err("Missing or invalid P2P_TRANSPORT_MODULE_SHA256.".to_owned())
    );
}

/// A launcher from before RT-5 reports no MIT runtime: read as the zero hash, and not written back.
#[test]
fn the_mit_runtime_is_optional_in_a_report() {
    let mut report = json!({
        "gameExe": sha('a'), "internalServer": sha('a'), "winmmProxy": sha('a'),
        "transportModule": sha('a'), "transportRuntime": sha('a'), "admissionPublicKey": sha('a'),
    });
    let parsed = rustic_rifts_realms::selection::parse_artifacts(Some(&report)).unwrap();
    assert_eq!(parsed.mit_runtime, "0".repeat(64));
    assert!(serde_json::to_value(&parsed).unwrap().get("mitRuntime").is_none());
    report["mitRuntime"] = json!(sha('b'));
    let parsed = rustic_rifts_realms::selection::parse_artifacts(Some(&report)).unwrap();
    assert_eq!(parsed.mit_runtime, sha('b'));
    report["mitRuntime"] = json!("nope");
    assert!(rustic_rifts_realms::selection::parse_artifacts(Some(&report)).is_none());
}
