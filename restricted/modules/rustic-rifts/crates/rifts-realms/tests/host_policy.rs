//! `p2p/config/p2pHostPolicy.test.ts`: artifact hashes, eligibility and the host score.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use rifts_realms::host_policy::{
    ArtifactSources, EligibilityPolicy, KNOWN_GAME_EXE_SHA256, artifact_hashes_match, host_ineligibility_reason,
    hunt_drain_cap_ms, read_artifact_policy, score_host,
};
use rifts_realms::hunt::{ArtifactHashes, HostPresenceRecord, HostReliabilityRecord};
use rifts_seam::UnixMillis;
use serde_json::json;

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn hashes() -> ArtifactHashes {
    ArtifactHashes {
        game_exe: "11".repeat(32),
        internal_server: "22".repeat(32),
        winmm_proxy: "33".repeat(32),
        steam_bridge: "44".repeat(32),
        steam_runtime: "55".repeat(32),
        admission_public_key: "66".repeat(32),
    }
}

const NOW: UnixMillis = UnixMillis(1_792_000_000_000);

fn presence() -> HostPresenceRecord {
    HostPresenceRecord {
        account_id: "host-a".to_owned(),
        revision: 1,
        consent_to_host: true,
        observed_at: NOW,
        expires_at: UnixMillis(NOW.0 + 30_000),
        free_physical_memory_mib: 16_384.0,
        commit_headroom_mib: 24_576.0,
        logical_cpu_count: 12.0,
        cpu_load_percent: 20.0,
        power_saver: false,
        runtime_supported: true,
        steam_runtime_ready: true,
        steam_id: None,
        backend_rtt_ms: 30.0,
        worst_peer_rtt_ms: 50.0,
        jitter_ms: 4.0,
        packet_loss_percent: 0.0,
        artifacts: hashes(),
        artifacts_verified: true,
        launcher_version: "0.1.35".to_owned(),
        guard_session_id: None,
        guard_risk_score: None,
        guard_verified: None,
        guard_roles: Vec::new(),
        realm_host_sessions: None,
    }
}

fn pins() -> Vec<(&'static str, String)> {
    let h = hashes();
    vec![
        ("P2P_GAME_EXE_SHA256", h.game_exe),
        ("P2P_INTERNAL_SERVER_SHA256", h.internal_server),
        ("P2P_WINMM_SHA256", h.winmm_proxy),
        ("P2P_TRANSPORT_MODULE_SHA256", h.steam_bridge),
        ("P2P_TRANSPORT_RUNTIME_SHA256", h.steam_runtime),
        ("P2P_ADMISSION_PUBLIC_XY_SHA256", h.admission_public_key),
    ]
}

fn owned(pairs: &[(&'static str, String)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

fn no_feed() -> ArtifactSources {
    ArtifactSources::default()
}

#[test]
fn artifact_policy_requires_all_six_exact_release_hashes() {
    let error = read_artifact_policy(&env(&[]), &no_feed()).unwrap_err();
    assert!(error.contains("Missing or invalid"), "{error}");
    let policy = read_artifact_policy(&owned(&pins()), &no_feed()).unwrap();
    assert!(artifact_hashes_match(&hashes(), &policy.hashes));
    let other = ArtifactHashes {
        steam_bridge: "77".repeat(32),
        ..hashes()
    };
    assert!(!artifact_hashes_match(&other, &policy.hashes));
}

#[test]
fn artifact_policy_still_reads_the_transport_hashes_under_their_older_key_names() {
    let legacy: Vec<(&'static str, String)> = pins()
        .into_iter()
        .map(|(k, v)| match k {
            "P2P_TRANSPORT_MODULE_SHA256" => ("P2P_STEAM_BRIDGE_SHA256", v),
            "P2P_TRANSPORT_RUNTIME_SHA256" => ("P2P_STEAM_RUNTIME_SHA256", v),
            _ => (k, v),
        })
        .collect();
    let policy = read_artifact_policy(&owned(&legacy), &no_feed()).unwrap();
    assert!(artifact_hashes_match(&hashes(), &policy.hashes));
}

#[test]
fn the_drain_cap_defaults_to_no_cap_and_30_s_to_10_min_sets_one() {
    let cap = |value: &str| hunt_drain_cap_ms(&env(&[("P2P_HUNT_DRAIN_GRACE_MS", value)]));
    assert_eq!(hunt_drain_cap_ms(&env(&[])), None, "unset: no cap");
    assert_eq!(cap(""), None, "blank: no cap");
    assert_eq!(cap("0"), None, "0 is explicitly no cap");
    assert_eq!(cap(" 60000 "), Some(60_000), "a cap in range is read, trimmed");
    assert_eq!(cap("30000"), Some(30_000));
    assert_eq!(cap("600000"), Some(600_000));
    assert_eq!(cap("5000"), None);
    assert_eq!(cap("900000"), None);
    assert_eq!(cap("-60000"), None);
    assert_eq!(cap("soon"), None);
    assert_eq!(cap("Infinity"), None);
}

fn reason(p: &HostPresenceRecord, active: i64, policy: &EligibilityPolicy, guard: bool) -> Option<&'static str> {
    host_ineligibility_reason(p, None, active, policy, guard, NOW)
}

#[test]
fn host_eligibility_fails_closed_for_consent_integrity_resources_and_concurrent_hosting() {
    let policy = EligibilityPolicy::from_settings(&env(&[]));
    let with = |change: fn(&mut HostPresenceRecord)| {
        let mut p = presence();
        change(&mut p);
        p
    };
    assert_eq!(
        reason(&with(|p| p.consent_to_host = false), 0, &policy, false),
        Some("HOST_CONSENT_REQUIRED")
    );
    assert_eq!(
        reason(&with(|p| p.launcher_version = "0.1.30".to_owned()), 0, &policy, false),
        Some("HOST_LAUNCHER_OUTDATED")
    );
    assert_eq!(
        reason(&with(|p| p.launcher_version = "invalid".to_owned()), 0, &policy, false),
        Some("HOST_LAUNCHER_OUTDATED")
    );
    assert_eq!(
        reason(&with(|p| p.artifacts_verified = false), 0, &policy, false),
        Some("HOST_RUNTIME_UNVERIFIED")
    );
    assert_eq!(
        reason(&with(|p| p.free_physical_memory_mib = 512.0), 0, &policy, false),
        Some("HOST_MEMORY_LOW")
    );
    assert_eq!(reason(&presence(), 1, &policy, false), Some("HOST_ALREADY_ACTIVE"));
    assert_eq!(
        reason(&with(|p| p.guard_verified = Some(false)), 0, &policy, true),
        Some("HOST_GUARD_UNVERIFIED")
    );
    assert_eq!(reason(&presence(), 0, &policy, false), None);
}

#[test]
fn automatic_hosting_requires_launcher_0_1_35_or_an_explicit_newer_floor() {
    assert_eq!(
        EligibilityPolicy::from_settings(&env(&[])).minimum_launcher_version,
        "0.1.35"
    );
    let newer = EligibilityPolicy::from_settings(&env(&[("P2P_HOST_MIN_LAUNCHER_VERSION", "0.2.0")]));
    let mut old = presence();
    old.launcher_version = "0.1.99".to_owned();
    assert_eq!(reason(&old, 0, &newer, false), Some("HOST_LAUNCHER_OUTDATED"));
    old.launcher_version = "0.2.0".to_owned();
    assert_eq!(reason(&old, 0, &newer, false), None);
}

#[test]
fn central_score_prefers_the_lower_latency_and_more_stable_eligible_host() {
    let policy = EligibilityPolicy::from_settings(&env(&[]));
    let history = |succeeded, failed, unexpected| HostReliabilityRecord {
        starts_succeeded: succeeded,
        starts_failed: failed,
        unexpected_failures: unexpected,
        ..HostReliabilityRecord::new("host", NOW)
    };
    let (reliable, _) = score_host(&presence(), Some(&history(20, 0, 0)), &policy, 0);
    let mut shaky = presence();
    shaky.worst_peer_rtt_ms = 220.0;
    shaky.jitter_ms = 60.0;
    shaky.packet_loss_percent = 8.0;
    let (risky, _) = score_host(&shaky, Some(&history(2, 4, 4)), &policy, 0);
    assert!(reliable < risky);
}

fn feed() -> ArtifactSources {
    let h = hashes();
    ArtifactSources {
        runtime_manifest: Some(json!({
            "schema": 1, "component": "ParadoxRuntime", "target": "client", "version": "0.2.128", "channel": "stable",
            "targetChangelist": 647472, "sha256": h.internal_server.to_uppercase(),
            "extraFiles": [{ "name": "WINMM.dll", "sha256": h.winmm_proxy }, { "name": "steam_api64.dll", "sha256": "99".repeat(32) }]
        })),
        guard_artifacts: Some(vec![
            ("GAME".to_owned(), "aa".repeat(32)),
            ("INTERNAL_SERVER".to_owned(), h.internal_server.clone()),
            ("TRANSPORT_MODULE".to_owned(), h.steam_bridge.clone()),
            ("TRANSPORT_RUNTIME".to_owned(), h.steam_runtime.clone()),
            ("ADMISSION_KEY".to_owned(), h.admission_public_key),
        ]),
    }
}

#[test]
fn artifact_policy_derives_the_feed_hashes_and_the_exe_from_the_changelist() {
    let policy = read_artifact_policy(&env(&[("TARGET_CHANGELIST", "647472")]), &feed())
        .unwrap()
        .hashes;
    let h = hashes();
    assert_eq!(policy.game_exe, KNOWN_GAME_EXE_SHA256[0].1);
    assert_eq!(
        policy.game_exe,
        "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b"
    );
    assert_eq!(policy.internal_server, h.internal_server);
    assert_eq!(policy.winmm_proxy, h.winmm_proxy);
    assert_eq!(policy.steam_bridge, h.steam_bridge);
    assert_eq!(policy.steam_runtime, h.steam_runtime);
    assert_eq!(policy.admission_public_key, h.admission_public_key);
    let reported = ArtifactHashes {
        game_exe: KNOWN_GAME_EXE_SHA256[0].1.to_owned(),
        ..hashes()
    };
    assert!(artifact_hashes_match(&reported, &policy));
}

#[test]
fn an_ice_manifest_without_a_transport_runtime_role_expects_the_module_for_both() {
    let mut ice = feed();
    if let Some(artifacts) = ice.guard_artifacts.as_mut() {
        artifacts.retain(|(role, _)| role != "TRANSPORT_RUNTIME");
    }
    let policy = read_artifact_policy(&env(&[("TARGET_CHANGELIST", "647472")]), &ice)
        .unwrap()
        .hashes;
    assert_eq!(policy.steam_bridge, hashes().steam_bridge);
    assert_eq!(policy.steam_runtime, hashes().steam_bridge);
}

#[test]
fn an_env_pin_still_overrides_the_feed_and_an_unknown_changelist_needs_its_exe_pin() {
    let h = hashes();
    let pinned = read_artifact_policy(
        &env(&[
            ("TARGET_CHANGELIST", "647472"),
            ("P2P_INTERNAL_SERVER_SHA256", &"77".repeat(32)),
            ("P2P_GAME_EXE_SHA256", &h.game_exe),
        ]),
        &feed(),
    )
    .unwrap();
    assert_eq!(pinned.hashes.internal_server, "77".repeat(32));
    assert_eq!(pinned.hashes.game_exe, h.game_exe);
    assert_eq!(pinned.hashes.winmm_proxy, h.winmm_proxy);
    assert_eq!(
        pinned.stale_pins.len(),
        2,
        "both pins differ from what the feed derives"
    );
    let mut older = feed();
    if let Some(manifest) = older.runtime_manifest.as_mut() {
        manifest["targetChangelist"] = json!(392819);
    }
    let error = read_artifact_policy(&env(&[("TARGET_CHANGELIST", "392819")]), &older).unwrap_err();
    assert!(error.contains("P2P_GAME_EXE_SHA256"), "{error}");
    let with_exe = env(&[("TARGET_CHANGELIST", "392819"), ("P2P_GAME_EXE_SHA256", &h.game_exe)]);
    assert_eq!(
        read_artifact_policy(&with_exe, &older).unwrap().hashes.game_exe,
        h.game_exe
    );
    // A blank KEY= line is no pin: the feed value is used
    let blank = env(&[
        ("TARGET_CHANGELIST", "647472"),
        ("P2P_INTERNAL_SERVER_SHA256", ""),
        ("P2P_GAME_EXE_SHA256", " "),
    ]);
    assert_eq!(
        read_artifact_policy(&blank, &feed()).unwrap().hashes.internal_server,
        h.internal_server
    );
    // A malformed pin is a configuration error, not a fallback to the feed
    let bad = env(&[("TARGET_CHANGELIST", "647472"), ("P2P_WINMM_SHA256", "not-a-hash")]);
    assert!(
        read_artifact_policy(&bad, &feed())
            .unwrap_err()
            .contains("P2P_WINMM_SHA256")
    );
}

#[test]
fn a_feed_for_another_changelist_or_a_guard_manifest_without_co_op_roles_supplies_nothing() {
    let changelist = env(&[("TARGET_CHANGELIST", "647472")]);
    let mut other = feed();
    if let Some(manifest) = other.runtime_manifest.as_mut() {
        manifest["targetChangelist"] = json!(392819);
    }
    assert!(
        read_artifact_policy(&changelist, &other)
            .unwrap_err()
            .contains("P2P_INTERNAL_SERVER_SHA256")
    );
    let mut bare = feed();
    if let Some(artifacts) = bare.guard_artifacts.as_mut() {
        artifacts.truncate(2);
    }
    assert!(
        read_artifact_policy(&changelist, &bare)
            .unwrap_err()
            .contains("P2P_TRANSPORT_MODULE_SHA256")
    );
    let h = hashes();
    let mut none = feed();
    none.guard_artifacts = Some(Vec::new());
    let pinned = env(&[
        ("TARGET_CHANGELIST", "647472"),
        ("P2P_TRANSPORT_MODULE_SHA256", &h.steam_bridge),
        ("P2P_TRANSPORT_RUNTIME_SHA256", &h.steam_runtime),
        ("P2P_ADMISSION_PUBLIC_XY_SHA256", &h.admission_public_key),
    ]);
    assert_eq!(
        read_artifact_policy(&pinned, &none).unwrap().hashes.steam_bridge,
        h.steam_bridge
    );
}
