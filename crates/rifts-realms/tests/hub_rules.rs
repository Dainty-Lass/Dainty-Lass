// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The realm hub rules: identity, manifests, the state machine and its tick, ticket eligibility, capacity.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;

use common::hunt;
use rifts_realms::host_policy::EligibilityPolicy;
use rifts_realms::hub::{
    HUB_CAPABILITY_SCOPES, HubEvent, HubTick, HubTicketCheck, HubTicketDenial, PresenceState, RealmCapacity,
    RealmCapacityInput, RealmHubPolicy, RealmRequest, classify_realm_request, decide_hub_tick, decide_realm_capacity,
    hub_end_counts_as_host_failure, hub_is_full, hub_manifest, hub_party_key, hub_ran, hub_ticket_denial,
    hub_ticket_denial_error, is_hub_transition_allowed, next_hub_state, parse_hub_start_body, parse_hub_stop_body,
    realm_host_ineligibility_reason, realm_hosting_enabled,
};
use rifts_realms::hunt::{ArtifactHashes, HostPresenceRecord, HuntSession, HuntState};
use rifts_seam::{CapabilityScope, HubKind, UnixMillis};
use serde_json::json;

use HuntState as H;

fn hub(state: HuntState) -> HuntSession {
    let mut h = hunt("hub-1");
    h.state = state;
    h.hub_kind = Some(HubKind::Ramsgate);
    h.expires_at = UnixMillis(1_000_000);
    h
}

#[test]
fn identity_and_manifests() {
    assert_eq!(hub_party_key("r1", HubKind::Ramsgate), "realmhub:r1:ramsgate");
    assert_eq!(hub_party_key("r1", HubKind::Training), "realmhub:r1:training");
    assert_eq!(
        classify_realm_request("CITY", "x"),
        RealmRequest::Hub(HubKind::Ramsgate)
    );
    assert_eq!(
        classify_realm_request("ISLAND", "ShatteredIsles_ReturnToRamsgate"),
        RealmRequest::Hub(HubKind::Ramsgate)
    );
    assert_eq!(
        classify_realm_request("SHARED", "ShatteredIsles_TrainingDojo"),
        RealmRequest::Hub(HubKind::Training)
    );
    assert_eq!(classify_realm_request("ISLAND", "Hunt_A"), RealmRequest::Hunt);
    let ramsgate = hub_manifest(HubKind::Ramsgate);
    assert_eq!(ramsgate.game_mode, "CITY");
    assert_eq!(ramsgate.map_path, "/Game/Maps/ramsgate/ramsgate_01_persistent");
    let training = hub_manifest(HubKind::Training);
    assert_eq!(training.hunt_definition_id, "ShatteredIsles_TrainingDojo");
    assert_eq!(training.guard_completion_mode, "NO_COMPLETION");
    assert!(!HUB_CAPABILITY_SCOPES.contains(&CapabilityScope::EventsWrite));
    assert!(!HUB_CAPABILITY_SCOPES.contains(&CapabilityScope::CompleteRequest));
}

#[test]
fn start_and_stop_bodies() {
    assert_eq!(
        parse_hub_start_body(&json!({ "hubKind": "RAMSGATE" })),
        Ok((HubKind::Ramsgate, 16))
    );
    assert_eq!(
        parse_hub_start_body(&json!({ "hubKind": "TRAINING", "maxPlayers": 64 })),
        Ok((HubKind::Training, 64))
    );
    assert_eq!(
        parse_hub_start_body(&json!([])),
        Err("The request body must be an object.")
    );
    assert_eq!(
        parse_hub_start_body(&json!({ "hubKind": "ramsgate" })),
        Err("hubKind must be RAMSGATE or TRAINING.")
    );
    assert_eq!(
        parse_hub_start_body(&json!({ "hubKind": "RAMSGATE", "maxPlayers": 65 })),
        Err("maxPlayers must be a whole number from 1 to 64.")
    );
    assert_eq!(
        parse_hub_stop_body(&json!({ "hubKind": "TRAINING" })),
        Ok(HubKind::Training)
    );
}

#[test]
fn the_exact_transition_table() {
    let mut allowed = Vec::new();
    for from in HuntState::ALL {
        for to in HuntState::ALL {
            if is_hub_transition_allowed(from, to) {
                allowed.push((from, to));
            }
        }
    }
    let mut expected = vec![
        (H::HostOffered, H::Starting),
        (H::Starting, H::HostReady),
        (H::HostReady, H::HostReady),
        (H::HostReady, H::Admitting),
        (H::Admitting, H::Running),
        (H::HostReady, H::Suspect),
        (H::Admitting, H::Suspect),
        (H::Running, H::Suspect),
        (H::Suspect, H::HostReady),
        (H::Suspect, H::Admitting),
        (H::Suspect, H::Running),
        (H::Suspect, H::Aborted),
    ];
    for live in [
        H::HostOffered,
        H::Starting,
        H::HostReady,
        H::Admitting,
        H::Running,
        H::Suspect,
    ] {
        expected.push((live, H::Completed));
        expected.push((live, H::Aborted));
    }
    expected.sort_by_key(|(a, b)| (a.as_str(), b.as_str()));
    expected.dedup();
    allowed.sort_by_key(|(a, b)| (a.as_str(), b.as_str()));
    assert_eq!(allowed, expected);
    assert_eq!(
        next_hub_state(H::Suspect, HubEvent::LeaseBack, Some(H::Running)),
        Some(H::Running)
    );
    assert_eq!(next_hub_state(H::Suspect, HubEvent::LeaseBack, Some(H::Starting)), None);
    assert!(hub_ran(H::Running, None) && hub_ran(H::Suspect, Some(H::Running)));
    assert!(!hub_ran(H::Suspect, Some(H::Admitting)));
}

#[test]
fn the_tick() {
    let mut h = hub(H::Running);
    h.lease_expires_at = Some(UnixMillis(600));
    assert_eq!(decide_hub_tick(&h, UnixMillis(500), false, true), HubTick::None);
    assert_eq!(
        decide_hub_tick(&h, UnixMillis(600), false, true),
        HubTick::Suspect {
            reason: "HOST_LEASE_MISSED"
        }
    );
    assert_eq!(
        decide_hub_tick(&h, UnixMillis(500), true, true),
        HubTick::End {
            reason: "OPERATOR_P2P_STOP"
        }
    );
    assert_eq!(
        decide_hub_tick(&h, UnixMillis(500), false, false),
        HubTick::End {
            reason: "REALM_HOSTING_DISABLED"
        }
    );
    assert_eq!(
        decide_hub_tick(&h, UnixMillis(1_000_000), false, true),
        HubTick::End {
            reason: "HUB_SESSION_EXPIRED"
        }
    );
    let mut s = hub(H::Suspect);
    s.pre_suspect_state = Some(H::Running);
    s.lease_expires_at = Some(UnixMillis(700));
    s.suspect_deadline = Some(UnixMillis(800));
    assert_eq!(decide_hub_tick(&s, UnixMillis(650), false, true), HubTick::Recover);
    s.lease_expires_at = Some(UnixMillis(600));
    assert_eq!(decide_hub_tick(&s, UnixMillis(650), false, true), HubTick::None);
    assert_eq!(
        decide_hub_tick(&s, UnixMillis(800), false, true),
        HubTick::End {
            reason: "HOST_LEASE_LOST"
        }
    );
    let mut starting = hub(H::Starting);
    starting.startup_deadline = Some(UnixMillis(100));
    assert_eq!(
        decide_hub_tick(&starting, UnixMillis(100), false, true),
        HubTick::End {
            reason: "HUB_STARTUP_TIMEOUT"
        }
    );
    let mut ready = hub(H::HostReady);
    ready.startup_deadline = Some(UnixMillis(100));
    ready.lease_expires_at = Some(UnixMillis(50));
    // Still on the startup clock: the deadline beats the lease.
    assert_eq!(
        decide_hub_tick(&ready, UnixMillis(100), false, true),
        HubTick::End {
            reason: "HUB_STARTUP_TIMEOUT"
        }
    );
    let mut offered = hub(H::HostOffered);
    offered.offer_expires_at = Some(UnixMillis(10));
    assert_eq!(
        decide_hub_tick(&offered, UnixMillis(10), false, true),
        HubTick::End {
            reason: "HUB_OFFER_EXPIRED"
        }
    );
    assert_eq!(
        decide_hub_tick(&hub(H::Completed), UnixMillis(10), true, true),
        HubTick::None
    );
    assert!(hub_end_counts_as_host_failure("HUB_STARTUP_TIMEOUT"));
    assert!(!hub_end_counts_as_host_failure("HOST_STOPPED_HUB"));
}

#[test]
fn ticket_denials_and_errors() {
    let running = hub(H::Running);
    let base = HubTicketCheck {
        hub: &running,
        generation: Some(1),
        account_id: "member",
        presence: Some((PresenceState::Pending, UnixMillis(999_999), "42")),
        is_member: true,
        is_banned: false,
        link: Some(("VERIFIED", "42")),
        now: UnixMillis(10),
    };
    assert_eq!(hub_ticket_denial(&base), None);
    let starting = hub(H::Starting);
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck { hub: &starting, ..base }),
        Some(HubTicketDenial::HubNotRunning)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            now: UnixMillis(1_000_000),
            ..base
        }),
        Some(HubTicketDenial::HubExpired)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            generation: Some(2),
            ..base
        }),
        Some(HubTicketDenial::GenerationMismatch)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            account_id: "host",
            ..base
        }),
        Some(HubTicketDenial::IsHost)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            presence: Some((PresenceState::Admitted, UnixMillis(0), "42")),
            ..base
        }),
        Some(HubTicketDenial::NotExpected)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            presence: Some((PresenceState::Ticketed, UnixMillis(10), "42")),
            ..base
        }),
        Some(HubTicketDenial::EntryExpired)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            is_banned: true,
            ..base
        }),
        Some(HubTicketDenial::Banned)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            is_member: false,
            ..base
        }),
        Some(HubTicketDenial::NotMember)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck {
            link: Some(("VERIFIED", "7")),
            ..base
        }),
        Some(HubTicketDenial::PeerIdMismatch)
    );
    assert_eq!(
        hub_ticket_denial(&HubTicketCheck { link: None, ..base }),
        Some(HubTicketDenial::PeerLinkMissing)
    );
    assert_eq!(
        hub_ticket_denial_error(HubTicketDenial::HubExpired),
        ("P2P_SESSION_NOT_READY", "The realm's hub is not accepting players.")
    );
    assert_eq!(
        hub_ticket_denial_error(HubTicketDenial::EntryExpired).0,
        "P2P_NOT_ROSTERED"
    );
    assert_eq!(
        hub_ticket_denial_error(HubTicketDenial::PeerLinkMissing).0,
        "P2P_STEAM_LINK_REQUIRED"
    );
}

fn presence() -> HostPresenceRecord {
    HostPresenceRecord {
        account_id: "host".into(),
        revision: 1,
        consent_to_host: true,
        observed_at: UnixMillis(1000),
        expires_at: UnixMillis(100_000),
        free_physical_memory_mib: 100.0,
        commit_headroom_mib: 100.0,
        logical_cpu_count: 8.0,
        cpu_load_percent: 10.0,
        power_saver: false,
        runtime_supported: true,
        steam_runtime_ready: true,
        steam_id: None,
        backend_rtt_ms: 10.0,
        worst_peer_rtt_ms: 5000.0,
        jitter_ms: 900.0,
        packet_loss_percent: 99.0,
        artifacts: ArtifactHashes::default(),
        artifacts_verified: true,
        launcher_version: "0.1.70".into(),
        guard_session_id: None,
        guard_risk_score: None,
        guard_verified: None,
        guard_roles: Vec::new(),
        realm_host_sessions: None,
    }
}

#[test]
fn capacity_and_realm_hosting_settings() {
    let input = RealmCapacityInput {
        active_servers: 1,
        max_servers: 6,
        free_physical_memory_mib: 4000.0,
        commit_headroom_mib: 5000.0,
        minimum_free_memory_mib: 3072.0,
        minimum_commit_headroom_mib: 4096.0,
    };
    assert_eq!(decide_realm_capacity(&input), RealmCapacity::Ok);
    assert_eq!(
        decide_realm_capacity(&RealmCapacityInput {
            active_servers: 6,
            ..input
        }),
        RealmCapacity::ServerLimit
    );
    assert_eq!(
        decide_realm_capacity(&RealmCapacityInput {
            commit_headroom_mib: 1.0,
            ..input
        }),
        RealmCapacity::CommitLow
    );
    assert_eq!(
        decide_realm_capacity(&RealmCapacityInput {
            free_physical_memory_mib: 1.0,
            ..input
        }),
        RealmCapacity::OkLowFreeMemory
    );
    assert!(hub_is_full(16, 16, false) && !hub_is_full(16, 16, true) && !hub_is_full(15, 16, false));
    let set = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    };
    assert!(realm_hosting_enabled(&set(&[("RUSTIC_RIFTS_REALMS_HOSTING", "TRUE")])));
    assert!(!realm_hosting_enabled(&set(&[])));
    let policy = RealmHubPolicy::from_settings(&set(&[
        ("P2P_REALM_HUB_CAPABILITY_TTL_HOURS", "2"),
        ("P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER", "100"),
    ]));
    assert_eq!(policy.capability_ttl_ms, 7_200_000);
    assert_eq!(policy.minimum_free_memory_mib, 3072.0);
    assert_eq!(
        (policy.minimum_commit_headroom_mib, policy.default_max_servers),
        (4096.0, 6)
    );
    // A realm host is not held to the network bounds or the hunt memory floors.
    let eligibility = EligibilityPolicy::default();
    assert_eq!(
        realm_host_ineligibility_reason(&presence(), &eligibility, 0.0, 0.0, false, UnixMillis(2000)),
        None
    );
    let mut low = presence();
    low.commit_headroom_mib = 10.0;
    assert_eq!(
        realm_host_ineligibility_reason(&low, &eligibility, 0.0, 50.0, false, UnixMillis(2000)),
        Some("HOST_MEMORY_LOW")
    );
    assert_eq!(
        realm_host_ineligibility_reason(&presence(), &eligibility, 0.0, 0.0, true, UnixMillis(2000)),
        Some("HOST_GUARD_UNVERIFIED")
    );
}
