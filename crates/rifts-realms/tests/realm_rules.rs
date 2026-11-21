// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The realm rules and the strict input parsers.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use rifts_realms::input::{
    ParseFailure, parse_account_id_param, parse_ban_body, parse_create_realm_body, parse_entry_realm,
    parse_heartbeat_body, parse_host_public_key, parse_invite_body, parse_invite_code_param, parse_join_body,
    parse_known_host_key_query, parse_list_query, parse_name_param, parse_settings_body, parse_username_body,
    parse_version, whole_number,
};
use rifts_realms::realm::{
    AccountAt, DEFAULT_REALM_LIMITS, HostingPatch, INVITE_ALPHABET, InviteState, JoinDenial, JoinPolicy, JoinVia,
    MemberRole, NameRefusal, Patch, RESERVED_REALM_NAMES, RealmBan, RealmInviteRecord, RealmLimits, RealmMember,
    RealmRecord, apply_heartbeat, build_invite, decide_join, detect_ownership_change, hub_room_id, hub_room_ids_of,
    invite_code_from, invite_state, is_realm_name_releasable, matches_name_pattern, normalize_hub_room_id,
    normalize_invite_code, presence_touch_needs_write, realm_name_for_username, realm_name_release_at,
    realms_module_enabled, sort_newest_first, validate_hosting_patch, validate_realm_name,
};
use rifts_seam::{HubKind, UnixMillis};
use serde_json::{Value, json};

const T0: i64 = 1_791_892_800_000;

fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn realm(policy: JoinPolicy) -> RealmRecord {
    RealmRecord {
        realm_id: "realm-1".into(),
        name: "hosty".into(),
        host_account_id: "host".into(),
        host_public_key: "k".repeat(32),
        join_policy: policy,
        listed: false,
        members: vec![RealmMember {
            account_id: "host".into(),
            role: MemberRole::Host,
            joined_at: UnixMillis(T0),
        }],
        bans: Vec::new(),
        created_at: UnixMillis(T0),
        last_online_at: None,
        online_until: None,
        version: None,
        hosting: None,
        hub_room_ids: hub_room_ids_of("realm-1"),
        offline_notified_for: None,
        join_requests: Vec::new(),
        player_invites: Vec::new(),
    }
}

fn invite(realm_id: &str) -> RealmInviteRecord {
    build_invite("ABCDEFGHJK", realm_id, "host", UnixMillis(T0), 2, 1)
}

fn fails(code: &'static str, message: &str) -> ParseFailure {
    ParseFailure {
        code,
        message: message.to_owned(),
    }
}

#[test]
fn names_follow_the_pattern_and_the_reserved_list() {
    assert_eq!(validate_realm_name("  My-Realm "), Ok("my-realm".to_owned()));
    for bad in [
        "ab",
        "a".repeat(25).as_str(),
        "-realm",
        "realm-",
        "has space",
        "ünï",
        "",
    ] {
        assert_eq!(validate_realm_name(bad), Err(NameRefusal::Invalid), "{bad}");
    }
    assert_eq!(validate_realm_name(&"x".repeat(257)), Err(NameRefusal::Invalid));
    for reserved in ["ADMIN", "ad-min", "r-e-a-l-m", "Official"] {
        assert_eq!(validate_realm_name(reserved), Err(NameRefusal::Reserved), "{reserved}");
    }
    for name in RESERVED_REALM_NAMES {
        assert!(matches_name_pattern(name), "{name}");
    }
    // Format is checked before the reserved list.
    assert_eq!(validate_realm_name("-admin"), Err(NameRefusal::Invalid));
}

#[test]
fn usernames_become_realm_names() {
    assert_eq!(
        realm_name_for_username("  Cool Hunter!! 99 "),
        Some("cool-hunter-99".into())
    );
    assert_eq!(realm_name_for_username("__ab__c"), Some("ab-c".into()));
    assert_eq!(realm_name_for_username("ab"), None);
    assert_eq!(realm_name_for_username("admin"), Some("admin".into()));
    assert_eq!(realm_name_for_username("00000000-0000-4000-8000-000000000000"), None);
    assert_eq!(realm_name_for_username(&"a".repeat(257)), None);
}

#[test]
fn release_times_and_ownership() {
    let mut r = realm(JoinPolicy::Public);
    assert_eq!(
        realm_name_release_at(UnixMillis(T0), None),
        UnixMillis(T0 + 259_200_000)
    );
    assert_eq!(
        realm_name_release_at(UnixMillis(T0), Some(UnixMillis(T0 + 5))),
        UnixMillis(T0 + 5 + 2_592_000_000)
    );
    assert!(!is_realm_name_releasable(&r, UnixMillis(T0 + 259_199_999)));
    assert!(is_realm_name_releasable(&r, UnixMillis(T0 + 259_200_000)));
    r.last_online_at = Some(UnixMillis(T0));
    assert!(!is_realm_name_releasable(&r, UnixMillis(T0 + 259_200_000)));
    assert!(detect_ownership_change("HOSTY", "other", &r));
    assert!(!detect_ownership_change("hosty", &r.host_public_key.clone(), &r));
    assert!(!detect_ownership_change("else", "other", &r));
}

#[test]
fn invite_codes() {
    assert_eq!(INVITE_ALPHABET.len(), 32);
    for c in *b"IO01" {
        assert!(!INVITE_ALPHABET.contains(&c));
    }
    // Only the low five bits count, so every byte maps evenly.
    assert_eq!(invite_code_from([32; 10]), invite_code_from([0; 10]));
    assert_eq!(normalize_invite_code(" abcd-efgh jk "), Some("ABCDEFGHJK".into()));
    assert_eq!(normalize_invite_code("ABCDEFGHJ"), None);
    assert_eq!(normalize_invite_code("ABCDEFGHJ1"), None);
    assert_eq!(normalize_invite_code(&format!("ABCDEFGHJK{}", " ".repeat(31))), None);
}

#[test]
fn invite_states_in_order() {
    let mut i = invite("realm-1");
    assert_eq!(i.expires_at, UnixMillis(T0 + 3_600_000));
    assert_eq!(invite_state(&i, UnixMillis(T0)), InviteState::Usable);
    assert_eq!(invite_state(&i, UnixMillis(T0 + 3_600_000)), InviteState::Expired);
    i.uses = 2;
    assert_eq!(invite_state(&i, UnixMillis(T0)), InviteState::Exhausted);
    i.revoked_at = Some(UnixMillis(T0));
    assert_eq!(invite_state(&i, UnixMillis(T0 + 3_600_000)), InviteState::Revoked);
}

#[test]
fn the_join_decision_order() {
    let mut r = realm(JoinPolicy::Public);
    r.bans.push(RealmBan {
        account_id: "bad".into(),
        reason: None,
        at: UnixMillis(T0),
    });
    r.members.push(RealmMember {
        account_id: "bad".into(),
        role: MemberRole::Member,
        joined_at: UnixMillis(T0),
    });
    assert_eq!(
        decide_join(&r, "bad", Some(&invite("realm-1")), UnixMillis(T0)),
        Err(JoinDenial::Banned)
    );
    assert_eq!(decide_join(&r, "host", None, UnixMillis(T0)), Ok(JoinVia::Member));
    assert_eq!(decide_join(&r, "x", None, UnixMillis(T0)), Ok(JoinVia::Public));
    r.join_policy = JoinPolicy::Solo;
    assert_eq!(decide_join(&r, "x", None, UnixMillis(T0)), Err(JoinDenial::Solo));
    assert_eq!(decide_join(&r, "host", None, UnixMillis(T0)), Ok(JoinVia::Member));
    r.join_policy = JoinPolicy::InviteOnly;
    assert_eq!(
        decide_join(&r, "x", None, UnixMillis(T0)),
        Err(JoinDenial::InviteRequired)
    );
    assert_eq!(
        decide_join(&r, "x", Some(&invite("realm-2")), UnixMillis(T0)),
        Err(JoinDenial::InviteInvalid)
    );
    assert_eq!(
        decide_join(&r, "x", Some(&invite("realm-1")), UnixMillis(T0 + 3_600_000)),
        Err(JoinDenial::InviteExpired)
    );
    assert_eq!(
        decide_join(&r, "x", Some(&invite("realm-1")), UnixMillis(T0)),
        Ok(JoinVia::Invite)
    );
    assert_eq!(JoinDenial::InviteExpired.code(), "REALM_INVITE_EXPIRED");
}

#[test]
fn heartbeat_and_newest_first() {
    let on = apply_heartbeat(UnixMillis(T0), None, Some("1.0".into()));
    assert_eq!(
        (on.last_online_at, on.online_until),
        (UnixMillis(T0), UnixMillis(T0 + 90_000))
    );
    assert_eq!(
        apply_heartbeat(UnixMillis(T0), Some(false), None).online_until,
        UnixMillis(T0)
    );
    let entries = [("a", 1), ("b", 3), ("c", 1), ("d", 3)].map(|(id, at)| AccountAt {
        account_id: id.into(),
        at: UnixMillis(at),
    });
    let ids: Vec<String> = sort_newest_first(&entries).into_iter().map(|e| e.account_id).collect();
    assert_eq!(ids, ["b", "d", "a", "c"]);
}

#[test]
fn hosting_patches_are_validated_in_order() {
    let patch = |f: fn(&mut HostingPatch)| {
        let mut p = HostingPatch::default();
        f(&mut p);
        validate_hosting_patch(&p)
    };
    assert_eq!(patch(|_| {}), None);
    assert_eq!(patch(|p| p.max_servers = Some(9)), Some("maxServers"));
    assert_eq!(patch(|p| p.max_players = Some(0)), Some("maxPlayers"));
    assert_eq!(patch(|p| p.afk_seconds = Patch::Set(7201)), Some("afkSeconds"));
    assert_eq!(patch(|p| p.afk_seconds = Patch::Remove), None);
    assert_eq!(patch(|p| p.active_servers = Some(65)), Some("activeServers"));
    assert_eq!(
        patch(|p| p.hub_session_id = Patch::Set(String::new())),
        Some("hubSessionId")
    );
    assert_eq!(
        patch(|p| p.hub_session_id = Patch::Set("x".repeat(129))),
        Some("hubSessionId")
    );
    assert_eq!(
        patch(|p| {
            p.max_servers = Some(0);
            p.max_players = Some(0);
        }),
        Some("maxServers")
    );
}

#[test]
fn hub_room_ids_and_their_normalization() {
    let [ramsgate, training] = hub_room_ids_of("realm-1").try_into().unwrap();
    assert_eq!(ramsgate, hub_room_id("realm-1", HubKind::Ramsgate));
    assert_eq!(training, hub_room_id("realm-1", HubKind::Training));
    assert_eq!(normalize_hub_room_id(&ramsgate.to_uppercase()), Some(ramsgate.clone()));
    assert_eq!(normalize_hub_room_id("not-a-room"), None);
}

#[test]
fn presence_touch_rule() {
    // Touches 60 s apart never let a row lapse.
    assert!(presence_touch_needs_write(
        UnixMillis(0),
        UnixMillis(120_000),
        UnixMillis(60_000)
    ));
    assert!(!presence_touch_needs_write(
        UnixMillis(0),
        UnixMillis(120_000),
        UnixMillis(10_000)
    ));
    assert!(presence_touch_needs_write(
        UnixMillis(0),
        UnixMillis(120_000),
        UnixMillis(30_001)
    ));
    assert!(presence_touch_needs_write(
        UnixMillis(0),
        UnixMillis(70_000),
        UnixMillis(10_001)
    ));
}

#[test]
fn limits_and_the_module_switch() {
    assert_eq!(RealmLimits::from_settings(&settings(&[])), DEFAULT_REALM_LIMITS);
    let limits = RealmLimits::from_settings(&settings(&[
        ("RUSTIC_RIFTS_REALMS_MAX_MEMBERS", " 2 "),
        ("RUSTIC_RIFTS_REALMS_MAX_BANS", "5001"),
    ]));
    assert_eq!((limits.max_members, limits.max_bans), (2, 1000));
    for bad in ["1", "+3", "1e3", "1000000"] {
        let l = RealmLimits::from_settings(&settings(&[("RUSTIC_RIFTS_REALMS_MAX_MEMBERS", bad)]));
        assert_eq!(l.max_members, 1000, "{bad}");
    }
    assert!(realms_module_enabled(&settings(&[(
        "RUSTIC_RIFTS_REALMS_MODULE",
        " On "
    )])));
    assert!(!realms_module_enabled(&settings(&[(
        "RUSTIC_RIFTS_REALMS_MODULE",
        "yes"
    )])));
}

#[test]
fn create_realm_body() {
    let key = "A".repeat(16);
    let ok = parse_create_realm_body(Some(&json!({ "hostPublicKey": key, "name": "My-Realm" }))).unwrap();
    assert_eq!(ok.name.as_deref(), Some("my-realm"));
    assert_eq!(
        (ok.join_policy, ok.listed, ok.version),
        (JoinPolicy::InviteOnly, false, None)
    );
    assert_eq!(
        parse_create_realm_body(Some(&json!({ "hostPublicKey": key, "name": "admin" }))),
        Err(fails("REALM_NAME_RESERVED", "That realm name is reserved."))
    );
    assert_eq!(
        parse_create_realm_body(Some(&json!({ "hostPublicKey": key, "name": 7 })))
            .unwrap_err()
            .code,
        "REALM_NAME_INVALID"
    );
    assert_eq!(
        parse_create_realm_body(None),
        Err(fails(
            "REALM_VALIDATION_FAILED",
            "hostPublicKey must be the host's public key as text."
        ))
    );
    assert_eq!(
        parse_create_realm_body(Some(&json!([]))),
        Err(fails(
            "REALM_VALIDATION_FAILED",
            "The request body must be a JSON object."
        ))
    );
    let bad_policy = parse_create_realm_body(Some(&json!({ "hostPublicKey": key, "joinPolicy": "public" })));
    assert_eq!(
        bad_policy.unwrap_err().message,
        "joinPolicy must be SOLO, INVITE_ONLY or PUBLIC."
    );
    let null_version = parse_create_realm_body(Some(&json!({ "hostPublicKey": key, "version": null })));
    assert_eq!(null_version.unwrap_err().message, "version is malformed.");
}

#[test]
fn the_other_bodies() {
    assert_eq!(
        parse_settings_body(Some(&json!({}))).unwrap_err().message,
        "Nothing to change: send joinPolicy and/or listed."
    );
    assert_eq!(
        parse_settings_body(Some(&json!({ "listed": true }))).unwrap().listed,
        Some(true)
    );
    let invite = parse_invite_body(None).unwrap();
    assert_eq!((invite.max_uses, invite.ttl_hours), (10, 168));
    assert_eq!(
        parse_invite_body(Some(&json!({ "maxUses": 101 }))).unwrap_err().message,
        "maxUses must be a whole number from 1 to 100."
    );
    assert_eq!(
        parse_invite_body(Some(&json!({ "expiresInHours": 1.5 })))
            .unwrap_err()
            .message,
        "expiresInHours must be a whole number from 1 to 720."
    );
    assert_eq!(
        parse_join_body(Some(&json!({}))).unwrap_err().message,
        "Send a realm name and/or an invite code."
    );
    let join = parse_join_body(Some(&json!({ "name": "Admin", "inviteCode": "abcd-efgh-jk" }))).unwrap();
    assert_eq!(join.name.as_deref(), Some("admin"));
    assert_eq!(join.invite_code.as_deref(), Some("ABCDEFGHJK"));
    assert_eq!(
        parse_join_body(Some(&json!({ "name": "hosty", "knownHostKey": "short" })))
            .unwrap_err()
            .message,
        "knownHostKey is malformed."
    );
    let beat = parse_heartbeat_body(Some(&json!({ "hosting": { "maxServers": 2, "maxPlayers": 8 } }))).unwrap();
    assert_eq!(beat.hosting.unwrap().afk_seconds, None);
    assert_eq!(
        parse_heartbeat_body(Some(
            &json!({ "hosting": { "maxServers": 2, "maxPlayers": 8, "afkSeconds": null } })
        ))
        .unwrap_err()
        .message,
        "hosting.afkSeconds must be a whole number from 0 to 7200."
    );
    assert_eq!(
        parse_heartbeat_body(Some(&json!({ "hosting": [] })))
            .unwrap_err()
            .message,
        "hosting must be an object."
    );
    assert_eq!(parse_ban_body(Some(&json!({ "reason": "  " }))), Ok(None));
    assert_eq!(
        parse_ban_body(Some(&json!({ "reason": 1 }))).unwrap_err().message,
        "reason must be text."
    );
    assert_eq!(
        parse_ban_body(Some(&json!({ "reason": "a\u{7}" })))
            .unwrap_err()
            .message,
        "reason must be at most 200 characters of plain text."
    );
    assert_eq!(
        parse_username_body(Some(&json!({ "username": " Pal " }))),
        Ok("Pal".into())
    );
    assert_eq!(parse_username_body(None).unwrap_err().message, "username is required.");
    assert_eq!(
        parse_username_body(Some(&json!({ "username": "" })))
            .unwrap_err()
            .message,
        "username must be 1 to 64 characters of plain text."
    );
}

#[test]
fn the_entry_realm_and_parameters() {
    assert_eq!(
        parse_entry_realm(&json!("hosty")).unwrap_err().message,
        "realm must be an object with a name."
    );
    assert_eq!(
        parse_entry_realm(&json!({})),
        Err(fails("REALM_VALIDATION_FAILED", "realm.name is required."))
    );
    assert_eq!(
        parse_entry_realm(&json!({ "name": "-x-" })),
        Err(fails("REALM_NAME_INVALID", "That is not a realm name."))
    );
    assert_eq!(parse_entry_realm(&json!({ "name": "Hosty" })).unwrap().name, "hosty");
    assert_eq!(parse_list_query(None, None), Ok((25, 0)));
    assert_eq!(parse_list_query(Some("50"), Some("10000")), Ok((50, 10_000)));
    for bad in ["0", "51", "+1", " 1", ""] {
        assert!(parse_list_query(Some(bad), None).is_err(), "{bad}");
    }
    assert!(parse_list_query(None, Some("10001")).is_err());
    assert_eq!(parse_account_id_param("acc:1.x_y-z"), Some("acc:1.x_y-z".into()));
    assert_eq!(parse_account_id_param("a b"), None);
    assert_eq!(parse_known_host_key_query(None), Ok(None));
    assert!(parse_known_host_key_query(Some("tiny")).is_err());
    assert_eq!(parse_name_param(Some("Admin")), Ok("admin".into()));
    assert!(parse_name_param(None).is_err());
    assert!(parse_invite_code_param("nope").is_err());
    assert_eq!(
        parse_host_public_key(&json!("  AAAA_BBBB-CCCC=DD  ")),
        Some("AAAA_BBBB-CCCC=DD".into())
    );
    assert_eq!(parse_host_public_key(&Value::Null), None);
    assert_eq!(parse_version(&json!("0.1.70+abc")), Some("0.1.70+abc".into()));
    assert_eq!(parse_version(&json!("0.1 70")), None);
    assert_eq!(whole_number(&json!(2.0)), Some(2));
}
