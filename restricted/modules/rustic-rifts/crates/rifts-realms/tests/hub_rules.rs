//! `p2p/realm/realmHubRules.test.ts`: a realm hub's state machine, lease clock, presence table, ticket eligibility,
//! capacity and settings.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{hunt_session, t};
use rifts_realms::host_policy::EligibilityPolicy;
use rifts_realms::hub::{
    ENTRY_LIFETIME_MS, EntryDecision, HUB_LIVE_STATES, HubEvent, HubTick, HubTicketCheck, HubTicketDenial,
    PRESENCE_ALL_STATES, PRESENCE_LIVE_STATES, PresenceEvent, PresenceState, RealmCapacity, RealmCapacityInput,
    RealmHubPolicy, RealmRequest, classify_realm_request, decide_entry_request, decide_hub_tick, decide_realm_capacity,
    hub_end_counts_as_host_failure, hub_is_full, hub_manifest, hub_party_key, hub_ticket_denial,
    hub_ticket_denial_error, is_hub_transition_allowed, is_presence_expired, next_hub_state, next_presence_state,
    parse_hub_start_body, parse_hub_stop_body, realm_host_ineligibility_reason, realm_hosting_enabled,
};
use rifts_realms::hunt::{ArtifactHashes, HostPresenceRecord, HuntSession, HuntState};
use rifts_seam::{HubKind, UnixMillis};
use serde_json::json;

use HuntState::*;

fn now() -> UnixMillis {
    t("2026-10-14T12:00:00.000Z")
}

fn at(offset: i64) -> UnixMillis {
    UnixMillis(now().0 + offset)
}

#[test]
fn classify_realm_request_city_and_return_to_ramsgate_are_ramsgate_the_dojo_is_training() {
    use RealmRequest::{Hub, Hunt};
    assert_eq!(classify_realm_request(Some("CITY"), None), Hub(HubKind::Ramsgate));
    assert_eq!(classify_realm_request(Some("CITY"), Some("")), Hub(HubKind::Ramsgate));
    assert_eq!(
        classify_realm_request(Some("CITY"), Some("ShatteredIsles_ReturnToRamsgate")),
        Hub(HubKind::Ramsgate)
    );
    assert_eq!(
        classify_realm_request(Some("SHARED"), Some("ShatteredIsles_ReturnToRamsgate")),
        Hub(HubKind::Ramsgate)
    );
    assert_eq!(
        classify_realm_request(Some("ISLAND"), Some("ShatteredIsles_ReturnToRamsgate")),
        Hub(HubKind::Ramsgate)
    );
    assert_eq!(
        classify_realm_request(Some("SHARED"), Some("ShatteredIsles_TrainingDojo")),
        Hub(HubKind::Training)
    );
    let training = hub_manifest(HubKind::Training).hunt_definition_id;
    assert_eq!(
        classify_realm_request(Some("SHARED"), Some(training)),
        Hub(HubKind::Training)
    );
    assert_eq!(
        classify_realm_request(Some("ISLAND"), Some("ShatteredIsles_IslandA")),
        Hunt
    );
    assert_eq!(
        classify_realm_request(Some("SHARED"), Some("ShatteredIsles_Embermane")),
        Hunt
    );
    assert_eq!(classify_realm_request(Some("ISLAND"), None), Hunt);
    assert_eq!(classify_realm_request(None, None), Hunt);
    assert_eq!(
        classify_realm_request(Some("SHARED"), Some("shatteredisles_trainingdojo")),
        Hunt,
        "matched exactly"
    );
}

#[test]
fn the_hub_manifests_are_the_directors_persistent_hub_layout() {
    let ramsgate = hub_manifest(HubKind::Ramsgate);
    let training = hub_manifest(HubKind::Training);
    assert_eq!(ramsgate.map_path, "/Game/Maps/ramsgate/ramsgate_01_persistent");
    assert_eq!(ramsgate.hunt_definition_id, "ShatteredIsles_ReturnToRamsgate");
    assert_eq!(training.map_path, "/Game/Maps/islands/dojo/training_dojo_persistent");
    assert_eq!(training.hunt_definition_id, "ShatteredIsles_TrainingDojo");
    for manifest in [ramsgate, training] {
        assert_eq!(manifest.behemoth_path, "NO_BEHEMOTH");
        assert_eq!(manifest.matchmaker_hunt_id, "NO_MM_HUNTID");
        assert_eq!(manifest.expected_player_string, "NO_EXPECTED_PLAYERS");
        assert_eq!(manifest.guard_completion_mode, "NO_COMPLETION");
    }
}

#[test]
fn a_hubs_party_key_is_unique_per_realm_and_kind() {
    let mut hub = hunt_session("s", Running);
    assert!(!hub.is_realm_hub());
    hub.hub_kind = Some(HubKind::Ramsgate);
    assert!(hub.is_realm_hub());
    assert_eq!(hub_party_key("r1", HubKind::Ramsgate), "realmhub:r1:ramsgate");
    assert_eq!(hub_party_key("r1", HubKind::Training), "realmhub:r1:training");
    assert_ne!(
        hub_party_key("r1", HubKind::Ramsgate),
        hub_party_key("r2", HubKind::Ramsgate)
    );
}

#[test]
fn hubs_start_body_takes_a_kind_and_1_to_64_players() {
    assert_eq!(
        parse_hub_start_body(&json!({ "hubKind": "RAMSGATE" })),
        Ok((HubKind::Ramsgate, 16))
    );
    assert_eq!(
        parse_hub_start_body(&json!({ "hubKind": "TRAINING", "maxPlayers": 64 })),
        Ok((HubKind::Training, 64))
    );
    assert_eq!(
        parse_hub_start_body(&json!({ "hubKind": "RAMSGATE", "maxPlayers": 1 })),
        Ok((HubKind::Ramsgate, 1))
    );
    for bad in [
        json!(null),
        json!([]),
        json!("RAMSGATE"),
        json!({}),
        json!({ "hubKind": "ramsgate" }),
        json!({ "hubKind": "CITY" }),
        json!({ "hubKind": "RAMSGATE", "maxPlayers": 0 }),
        json!({ "hubKind": "RAMSGATE", "maxPlayers": 65 }),
        json!({ "hubKind": "RAMSGATE", "maxPlayers": 2.5 }),
        json!({ "hubKind": "RAMSGATE", "maxPlayers": "8" }),
    ] {
        assert!(parse_hub_start_body(&bad).is_err(), "{bad}");
    }
    assert_eq!(
        parse_hub_stop_body(&json!({ "hubKind": "TRAINING" })),
        Ok(HubKind::Training)
    );
    assert!(parse_hub_stop_body(&json!({})).is_err());
    assert!(parse_hub_stop_body(&json!(null)).is_err());
}

#[test]
fn hub_state_machine_has_the_4_2_transitions_and_no_others() {
    assert_eq!(
        next_hub_state(HostOffered, HubEvent::HostBootstrap, None),
        Some(Starting)
    );
    assert_eq!(next_hub_state(Starting, HubEvent::HostReady, None), Some(HostReady));
    assert_eq!(
        next_hub_state(HostReady, HubEvent::HostRouteReady, None),
        Some(Admitting)
    );
    assert_eq!(next_hub_state(Admitting, HubEvent::HostAdmitted, None), Some(Running));
    assert_eq!(next_hub_state(Running, HubEvent::LeaseLapsed, None), Some(Suspect));
    assert_eq!(
        next_hub_state(Suspect, HubEvent::LeaseBack, Some(Running)),
        Some(Running)
    );
    assert_eq!(
        next_hub_state(Suspect, HubEvent::LeaseBack, Some(HostReady)),
        Some(HostReady)
    );
    assert_eq!(
        next_hub_state(Suspect, HubEvent::LeaseBack, None),
        None,
        "no way back without the pre-suspect state"
    );
    assert_eq!(next_hub_state(Suspect, HubEvent::LeaseBack, Some(Starting)), None);
    assert_eq!(next_hub_state(Suspect, HubEvent::SuspectTimeout, None), Some(Aborted));
    assert_eq!(next_hub_state(Running, HubEvent::Stop, None), Some(Completed));
    assert_eq!(next_hub_state(HostOffered, HubEvent::Abort, None), Some(Aborted));
    assert_eq!(next_hub_state(Running, HubEvent::HostBootstrap, None), None);
    assert_eq!(next_hub_state(Starting, HubEvent::HostRouteReady, None), None);
    assert_eq!(next_hub_state(Aborted, HubEvent::Stop, None), None);
    assert_eq!(next_hub_state(Completed, HubEvent::Abort, None), None);
    assert_eq!(next_hub_state(Running, HubEvent::SuspectTimeout, None), None);
    assert_eq!(next_hub_state(Starting, HubEvent::LeaseLapsed, None), None);

    let states = [
        Requested,
        HostSelecting,
        HostOffered,
        Starting,
        HostReady,
        Admitting,
        Running,
        Suspect,
        Fenced,
        Aborting,
        Recovering,
        Completed,
        Aborted,
    ];
    let mut allowed: BTreeSet<String> = [
        "HOST_OFFERED>STARTING",
        "STARTING>HOST_READY",
        "HOST_READY>HOST_READY",
        "HOST_READY>ADMITTING",
        "ADMITTING>RUNNING",
        "HOST_READY>SUSPECT",
        "ADMITTING>SUSPECT",
        "RUNNING>SUSPECT",
        "SUSPECT>HOST_READY",
        "SUSPECT>ADMITTING",
        "SUSPECT>RUNNING",
        "SUSPECT>ABORTED",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    for from in HUB_LIVE_STATES {
        allowed.insert(format!("{}>COMPLETED", from.as_str()));
        allowed.insert(format!("{}>ABORTED", from.as_str()));
    }
    for from in states {
        for to in states {
            let key = format!("{}>{}", from.as_str(), to.as_str());
            assert_eq!(is_hub_transition_allowed(from, to), allowed.contains(&key), "{key}");
        }
    }
}

fn lease(state: HuntState) -> HuntSession {
    let mut hub = hunt_session("hub", state);
    hub.hub_kind = Some(HubKind::Ramsgate);
    hub.expires_at = at(7 * 86_400_000);
    hub.offer_expires_at = Some(at(60_000));
    hub.startup_deadline = Some(at(60_000));
    hub.lease_expires_at = Some(at(8_000));
    hub
}

fn tick(hub: &HuntSession) -> HubTick {
    decide_hub_tick(hub, now(), false, true)
}

fn end(reason: &'static str) -> HubTick {
    HubTick::End { reason }
}

#[test]
fn monitor_a_healthy_hub_does_nothing_and_a_lapsed_lease_makes_it_suspect() {
    for state in [HostOffered, Starting, HostReady, Admitting, Running] {
        assert_eq!(tick(&lease(state)), HubTick::None, "{state:?}");
    }
    for state in [HostReady, Admitting, Running] {
        let mut hub = lease(state);
        hub.lease_expires_at = Some(at(-1));
        assert_eq!(
            tick(&hub),
            HubTick::Suspect {
                reason: "HOST_LEASE_MISSED"
            },
            "{state:?}"
        );
    }
    // A lapsed lease before the host was ready is the startup deadline's business
    let mut starting = lease(Starting);
    starting.lease_expires_at = Some(at(-1));
    assert_eq!(tick(&starting), HubTick::None);
    let mut running = lease(Running);
    running.lease_expires_at = None;
    assert_eq!(tick(&running), HubTick::None);
}

#[test]
fn monitor_a_suspect_hub_comes_back_with_the_lease_or_is_aborted_when_the_window_ends() {
    let mut suspect = lease(Suspect);
    suspect.pre_suspect_state = Some(Running);
    suspect.suspect_deadline = Some(at(10_000));
    suspect.lease_expires_at = Some(at(-5_000));
    assert_eq!(tick(&suspect), HubTick::None);
    let back = HuntSession {
        lease_expires_at: Some(at(5_000)),
        ..suspect.clone()
    };
    assert_eq!(tick(&back), HubTick::Recover);
    let lost = HuntSession {
        suspect_deadline: Some(at(-1)),
        ..suspect.clone()
    };
    assert_eq!(tick(&lost), end("HOST_LEASE_LOST"));
    // A lease that is back wins over a deadline that has also passed
    let both = HuntSession {
        lease_expires_at: Some(at(5_000)),
        suspect_deadline: Some(at(-1)),
        ..suspect.clone()
    };
    assert_eq!(tick(&both), HubTick::Recover);
    // Nothing to return to: wait for the deadline
    let orphan = HuntSession {
        lease_expires_at: Some(at(5_000)),
        pre_suspect_state: None,
        ..suspect
    };
    assert_eq!(tick(&orphan), HubTick::None);
}

#[test]
fn monitor_deadlines_expiry_operator_stop_and_hosting_switch_end_a_hub() {
    let with = |state, change: fn(&mut HuntSession)| {
        let mut hub = lease(state);
        change(&mut hub);
        hub
    };
    assert_eq!(
        tick(&with(HostOffered, |h| h.offer_expires_at = Some(at(-1)))),
        end("HUB_OFFER_EXPIRED")
    );
    for state in [Starting, HostReady, Admitting] {
        assert_eq!(
            tick(&with(state, |h| h.startup_deadline = Some(at(-1)))),
            end("HUB_STARTUP_TIMEOUT"),
            "{state:?}"
        );
    }
    // A running hub has no startup deadline any more (a stale one is ignored)
    assert_eq!(
        tick(&with(Running, |h| h.startup_deadline = Some(at(-1)))),
        HubTick::None
    );
    assert_eq!(
        tick(&with(Running, |h| h.expires_at = at(-1))),
        end("HUB_SESSION_EXPIRED")
    );
    assert_eq!(
        decide_hub_tick(&lease(Running), now(), true, true),
        end("OPERATOR_P2P_STOP")
    );
    assert_eq!(
        decide_hub_tick(&lease(HostOffered), now(), true, true),
        end("OPERATOR_P2P_STOP")
    );
    assert_eq!(
        decide_hub_tick(&lease(Running), now(), false, false),
        end("REALM_HOSTING_DISABLED")
    );
    // A hub that is not live is never touched
    assert_eq!(decide_hub_tick(&lease(Completed), now(), true, true), HubTick::None);
    assert_eq!(tick(&lease(Aborted)), HubTick::None);
}

#[test]
fn only_a_lost_host_is_a_failure_of_the_host() {
    for reason in ["HOST_LEASE_LOST", "HOST_PROCESS_UNHEALTHY", "HUB_STARTUP_TIMEOUT"] {
        assert!(hub_end_counts_as_host_failure(reason), "{reason}");
    }
    for reason in [
        "HOST_STOPPED",
        "OPERATOR_P2P_STOP",
        "REALM_RELEASED",
        "HUB_OFFER_EXPIRED",
        "REALM_HOSTING_DISABLED",
    ] {
        assert!(!hub_end_counts_as_host_failure(reason), "{reason}");
    }
}

#[test]
fn presence_transitions_follow_the_4_3_table() {
    use PresenceEvent as E;
    use PresenceState as S;
    let next = next_presence_state;
    assert_eq!(next(S::Pending, E::TicketIssued), Some(S::Ticketed));
    assert_eq!(next(S::Ticketed, E::TicketIssued), Some(S::Ticketed));
    assert_eq!(next(S::Ticketed, E::Admitted), Some(S::Admitted));
    assert_eq!(next(S::Pending, E::Admitted), Some(S::Admitted));
    assert_eq!(next(S::Admitted, E::Admitted), Some(S::Admitted));
    assert_eq!(next(S::Admitted, E::Disconnected), Some(S::Lost));
    assert_eq!(next(S::Admitted, E::Travelled), Some(S::Left));
    assert_eq!(next(S::Pending, E::Expired), Some(S::Expired));
    assert_eq!(next(S::Ticketed, E::Expired), Some(S::Expired));
    for from in PRESENCE_LIVE_STATES {
        assert_eq!(next(*from, E::Removed), Some(S::Removed), "{from:?}");
        assert_eq!(next(*from, E::SessionEnded), Some(S::Lost), "{from:?}");
    }
    for from in PRESENCE_ALL_STATES {
        assert_eq!(next(*from, E::EntryStarted), Some(S::Pending), "{from:?}");
    }
    assert_eq!(next(S::Lost, E::Admitted), None, "a lost link needs a fresh entry");
    assert_eq!(next(S::Lost, E::TicketIssued), None);
    assert_eq!(next(S::Left, E::Admitted), None);
    assert_eq!(next(S::Removed, E::Admitted), None);
    assert_eq!(next(S::Expired, E::TicketIssued), None);
    assert_eq!(next(S::Pending, E::Disconnected), None);
    assert_eq!(next(S::Ticketed, E::Travelled), None);
    assert_eq!(next(S::Admitted, E::Expired), None, "a connected member never expires");
    assert_eq!(next(S::Lost, E::Removed), None);
}

#[test]
fn an_entry_expires_only_while_it_waits() {
    assert!(is_presence_expired(PresenceState::Pending, at(-1), at(0)));
    assert!(is_presence_expired(PresenceState::Pending, at(0), at(0)));
    assert!(!is_presence_expired(PresenceState::Ticketed, at(1), at(0)));
    assert!(!is_presence_expired(PresenceState::Admitted, at(-86_400_000), at(0)));
    assert!(!is_presence_expired(PresenceState::Lost, at(-86_400_000), at(0)));
    assert_eq!(ENTRY_LIFETIME_MS, 90_000);
}

#[test]
fn an_entry_request_creates_reuses_a_waiting_row_or_starts_over() {
    assert_eq!(decide_entry_request(None, at(0)), EntryDecision::Create);
    assert_eq!(
        decide_entry_request(Some((PresenceState::Pending, at(30_000))), at(0)),
        EntryDecision::Reuse
    );
    assert_eq!(
        decide_entry_request(Some((PresenceState::Ticketed, at(30_000))), at(0)),
        EntryDecision::Reuse
    );
    assert_eq!(
        decide_entry_request(Some((PresenceState::Pending, at(-1))), at(0)),
        EntryDecision::Reenter
    );
    for state in [
        PresenceState::Admitted,
        PresenceState::Lost,
        PresenceState::Left,
        PresenceState::Removed,
        PresenceState::Expired,
    ] {
        assert_eq!(
            decide_entry_request(Some((state, at(30_000))), at(0)),
            EntryDecision::Reenter,
            "{state:?}"
        );
    }
}

struct Ticket {
    hub: HuntSession,
    generation: Option<i64>,
    account: &'static str,
    presence: Option<(PresenceState, UnixMillis, &'static str)>,
    member: bool,
    banned: bool,
    link: Option<(&'static str, &'static str)>,
}

impl Ticket {
    fn new() -> Self {
        let mut hub = hunt_session("hub", Running);
        hub.expires_at = at(86_400_000);
        hub.host_account_id = "host-account".to_owned();
        Self {
            hub,
            generation: None,
            account: "member-account",
            presence: Some((PresenceState::Pending, at(60_000), "4242")),
            member: true,
            banned: false,
            link: Some(("VERIFIED", "4242")),
        }
    }

    fn denial(&self) -> Option<HubTicketDenial> {
        hub_ticket_denial(&HubTicketCheck {
            hub: &self.hub,
            generation: self.generation,
            account_id: self.account,
            presence: self.presence,
            is_member: self.member,
            is_banned: self.banned,
            link: self.link,
            now: now(),
        })
    }
}

#[test]
fn ticket_eligibility_needs_a_running_hub_an_open_entry_a_member_and_the_verified_link() {
    use HubTicketDenial as D;
    assert_eq!(Ticket::new().denial(), None);
    let mut t = Ticket::new();
    t.presence = Some((PresenceState::Ticketed, at(60_000), "4242"));
    assert_eq!(t.denial(), None);
    let mut t = Ticket::new();
    t.generation = Some(1);
    assert_eq!(t.denial(), None);
    for state in [HostOffered, Starting, HostReady, Admitting, Suspect, Completed, Aborted] {
        let mut t = Ticket::new();
        t.hub.state = state;
        assert_eq!(t.denial(), Some(D::HubNotRunning), "{state:?}");
    }
    let mut t = Ticket::new();
    t.hub.expires_at = at(-1);
    assert_eq!(t.denial(), Some(D::HubExpired));
    let mut t = Ticket::new();
    t.generation = Some(2);
    assert_eq!(t.denial(), Some(D::GenerationMismatch));
    let mut t = Ticket::new();
    t.account = "host-account";
    assert_eq!(t.denial(), Some(D::IsHost));
    let mut t = Ticket::new();
    t.presence = None;
    assert_eq!(t.denial(), Some(D::NotExpected));
    for state in [
        PresenceState::Admitted,
        PresenceState::Lost,
        PresenceState::Left,
        PresenceState::Removed,
        PresenceState::Expired,
    ] {
        let mut t = Ticket::new();
        t.presence = Some((state, at(60_000), "4242"));
        assert_eq!(t.denial(), Some(D::NotExpected), "{state:?}");
    }
    let mut t = Ticket::new();
    t.presence = Some((PresenceState::Pending, at(-1), "4242"));
    assert_eq!(t.denial(), Some(D::EntryExpired));
    let mut t = Ticket::new();
    t.banned = true;
    assert_eq!(t.denial(), Some(D::Banned));
    t.member = false;
    assert_eq!(
        t.denial(),
        Some(D::Banned),
        "a ban is reported before a missing membership"
    );
    let mut t = Ticket::new();
    t.member = false;
    assert_eq!(t.denial(), Some(D::NotMember));
    let mut t = Ticket::new();
    t.link = None;
    assert_eq!(t.denial(), Some(D::PeerLinkMissing));
    t.link = Some(("REVOKED", "4242"));
    assert_eq!(t.denial(), Some(D::PeerLinkMissing));
    t.link = Some(("VERIFIED", "9999"));
    assert_eq!(t.denial(), Some(D::PeerIdMismatch));
}

#[test]
fn every_ticket_denial_maps_to_a_launcher_error() {
    use HubTicketDenial as D;
    for (reason, code) in [
        (D::HubNotRunning, "P2P_SESSION_NOT_READY"),
        (D::HubExpired, "P2P_SESSION_NOT_READY"),
        (D::GenerationMismatch, "P2P_GENERATION_FENCED"),
        (D::IsHost, "P2P_NOT_ROSTERED"),
        (D::NotExpected, "P2P_NOT_ROSTERED"),
        (D::EntryExpired, "P2P_NOT_ROSTERED"),
        (D::Banned, "REALM_BANNED"),
        (D::NotMember, "REALM_NOT_MEMBER"),
        (D::PeerLinkMissing, "P2P_STEAM_LINK_REQUIRED"),
        (D::PeerIdMismatch, "P2P_STEAM_LINK_REQUIRED"),
    ] {
        let (actual, message) = hub_ticket_denial_error(reason);
        assert_eq!(actual, code, "{reason:?}");
        assert!(!message.is_empty());
    }
}

#[test]
fn capacity_is_the_server_limit_and_commit_headroom_and_low_free_memory_is_only_reported() {
    let base = RealmCapacityInput {
        active_servers: 1,
        max_servers: 3,
        free_physical_memory_mib: 8192.0,
        commit_headroom_mib: 8192.0,
        minimum_free_memory_mib: 3072.0,
        minimum_commit_headroom_mib: 4096.0,
    };
    let decide = |change: fn(&mut RealmCapacityInput)| {
        let mut input = base;
        change(&mut input);
        decide_realm_capacity(&input)
    };
    assert_eq!(decide_realm_capacity(&base), RealmCapacity::Ok);
    assert_eq!(decide(|i| i.active_servers = 2), RealmCapacity::Ok);
    assert_eq!(decide(|i| i.active_servers = 3), RealmCapacity::ServerLimit);
    assert_eq!(
        decide(|i| i.free_physical_memory_mib = 3071.0),
        RealmCapacity::OkLowFreeMemory
    );
    assert_eq!(
        decide(|i| {
            i.free_physical_memory_mib = 2006.0;
            i.commit_headroom_mib = 15_460.0;
        }),
        RealmCapacity::OkLowFreeMemory,
        "little free memory, plenty of commit headroom"
    );
    assert_eq!(decide(|i| i.free_physical_memory_mib = 3072.0), RealmCapacity::Ok);
    assert_eq!(decide(|i| i.commit_headroom_mib = 4095.0), RealmCapacity::CommitLow);
    assert_eq!(
        decide(|i| {
            i.free_physical_memory_mib = 0.0;
            i.commit_headroom_mib = 4095.0;
        }),
        RealmCapacity::CommitLow
    );
    assert_eq!(
        decide(|i| {
            i.active_servers = 3;
            i.free_physical_memory_mib = 0.0;
        }),
        RealmCapacity::ServerLimit
    );
}

#[test]
fn a_hub_is_full_at_max_peers_but_never_for_a_member_already_in_it() {
    assert!(!hub_is_full(15, 16, false));
    assert!(hub_is_full(16, 16, false));
    assert!(hub_is_full(17, 16, false));
    assert!(!hub_is_full(16, 16, true));
    assert!(!hub_is_full(0, 1, false));
    assert!(hub_is_full(1, 1, false));
}

#[test]
fn the_realm_hosting_switch_and_the_realm_hub_thresholds() {
    let env = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    };
    for on in ["on", "ON", "true", "1", " on "] {
        assert!(
            realm_hosting_enabled(&env(&[("MYSTICPARADOX_REALMS_HOSTING", on)])),
            "{on}"
        );
    }
    for off in ["", "off", "false", "0", "yes", "enabled"] {
        assert!(
            !realm_hosting_enabled(&env(&[("MYSTICPARADOX_REALMS_HOSTING", off)])),
            "{off}"
        );
    }
    assert!(!realm_hosting_enabled(&env(&[])));
    let defaults = RealmHubPolicy::from_settings(&env(&[]));
    assert_eq!(defaults.capability_ttl_ms, 24 * 3_600_000);
    assert_eq!(defaults.minimum_free_memory_mib, 3072.0);
    assert_eq!(defaults.minimum_commit_headroom_mib, 4096.0);
    let tuned = RealmHubPolicy::from_settings(&env(&[
        ("P2P_REALM_HUB_CAPABILITY_TTL_HOURS", "12"),
        ("P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER", "2048"),
    ]));
    assert_eq!(tuned.capability_ttl_ms, 12 * 3_600_000);
    assert_eq!(tuned.minimum_free_memory_mib, 2048.0);
    let bad = RealmHubPolicy::from_settings(&env(&[
        ("P2P_REALM_HUB_CAPABILITY_TTL_HOURS", "0"),
        ("P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER", "lots"),
    ]));
    assert_eq!(bad.capability_ttl_ms, 24 * 3_600_000);
    assert_eq!(bad.minimum_free_memory_mib, 3072.0);
    let long = RealmHubPolicy::from_settings(&env(&[("P2P_REALM_HUB_CAPABILITY_TTL_HOURS", "1000")]));
    assert_eq!(
        long.capability_ttl_ms,
        24 * 3_600_000,
        "the session record lives 7 days at most"
    );
}

fn host_presence() -> HostPresenceRecord {
    let hash = "a".repeat(64);
    HostPresenceRecord {
        account_id: "host-account".to_owned(),
        revision: 1,
        consent_to_host: true,
        observed_at: at(-1_000),
        expires_at: at(29_000),
        free_physical_memory_mib: 8192.0,
        commit_headroom_mib: 8192.0,
        logical_cpu_count: 8.0,
        cpu_load_percent: 20.0,
        power_saver: false,
        runtime_supported: true,
        steam_runtime_ready: true,
        steam_id: Some("4242".to_owned()),
        backend_rtt_ms: 40.0,
        worst_peer_rtt_ms: 40.0,
        jitter_ms: 5.0,
        packet_loss_percent: 0.0,
        artifacts: ArtifactHashes {
            game_exe: hash.clone(),
            internal_server: hash.clone(),
            winmm_proxy: hash.clone(),
            steam_bridge: hash.clone(),
            steam_runtime: hash.clone(),
            admission_public_key: hash,
        },
        artifacts_verified: true,
        launcher_version: "0.1.55".to_owned(),
        guard_session_id: None,
        guard_risk_score: None,
        guard_verified: Some(true),
        guard_roles: Vec::new(),
        realm_host_sessions: None,
    }
}

#[test]
fn realm_host_eligibility_keeps_the_integrity_rules_and_drops_the_network_ones() {
    let policy = EligibilityPolicy::from_settings(&BTreeMap::new());
    let reason = |change: fn(&mut HostPresenceRecord), guard: bool| {
        let mut presence = host_presence();
        change(&mut presence);
        realm_host_ineligibility_reason(&presence, &policy, 3072.0, 4096.0, guard, now())
    };
    assert_eq!(reason(|_| {}, true), None);
    assert_eq!(
        reason(
            |p| {
                p.worst_peer_rtt_ms = 1900.0;
                p.jitter_ms = 900.0;
                p.packet_loss_percent = 60.0;
            },
            true
        ),
        None
    );
    assert_eq!(
        reason(|p| p.consent_to_host = false, true),
        Some("HOST_CONSENT_REQUIRED")
    );
    assert_eq!(
        reason(|p| p.launcher_version = "0.1.10".to_owned(), true),
        Some("HOST_LAUNCHER_OUTDATED")
    );
    assert_eq!(
        reason(|p| p.artifacts_verified = false, true),
        Some("HOST_RUNTIME_UNVERIFIED")
    );
    assert_eq!(
        reason(|p| p.steam_runtime_ready = false, true),
        Some("HOST_RUNTIME_UNVERIFIED")
    );
    assert_eq!(
        reason(|p| p.guard_verified = Some(false), true),
        Some("HOST_GUARD_UNVERIFIED")
    );
    assert_eq!(reason(|p| p.guard_verified = Some(false), false), None);
    assert_eq!(reason(|p| p.expires_at = at(-1), true), Some("HOST_PRESENCE_STALE"));
    assert_eq!(reason(|p| p.power_saver = true, true), Some("HOST_CPU_UNSUITABLE"));
    // The realm's memory floors replace the official ones
    assert_eq!(
        reason(
            |p| {
                p.free_physical_memory_mib = 3100.0;
                p.commit_headroom_mib = 4100.0;
            },
            true
        ),
        None
    );
    assert_eq!(
        reason(|p| p.free_physical_memory_mib = 3000.0, true),
        Some("HOST_MEMORY_LOW")
    );
    assert_eq!(
        reason(|p| p.commit_headroom_mib = 4000.0, true),
        Some("HOST_MEMORY_LOW")
    );
}
