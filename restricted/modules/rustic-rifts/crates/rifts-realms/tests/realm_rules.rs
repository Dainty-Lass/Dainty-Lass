//! `realms/realmRules.test.ts`, `realmInput.test.ts` and `realmHubRules.test.ts`. Cases only JavaScript can express (a
//! name that is a number or `undefined`, an unreadable date, an unknown key in a patch) are ruled out by the Rust
//! types and left out; the JSON bodies keep theirs.
#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use rifts_realms::input::{
    CreateRealmInput, EntryRealmInput, HeartbeatInput, HostingInput, InviteInput, JoinInput, SettingsInput,
    parse_account_id_param, parse_ban_body, parse_create_realm_body, parse_entry_realm, parse_heartbeat_body,
    parse_host_public_key, parse_invite_body, parse_invite_code_param, parse_join_body, parse_known_host_key_query,
    parse_list_query, parse_name_param, parse_settings_body, parse_username_body, parse_version,
};
use rifts_realms::realm::{
    AccountAt, DEFAULT_REALM_LIMITS, HEARTBEAT_ONLINE_MS, HostingPatch, HubState, INVITE_ALPHABET, INVITE_CODE_LENGTH,
    InviteState, JoinDenial, JoinPolicy, JoinVia, LAST_ONLINE_RELEASE_MS, MemberRole, NEVER_ONLINE_RELEASE_MS,
    NameRefusal, Patch, RESERVED_REALM_NAMES, RealmBan, RealmHosting, RealmInviteRecord, RealmLimits, RealmMember,
    RealmRecord, apply_heartbeat, apply_hosting_patch, build_invite, decide_join, detect_ownership_change,
    generate_invite_code, hub_room_id, hub_room_ids_of, invite_code_from, invite_state, is_realm_hub_ready,
    is_realm_name_releasable, is_realm_online, is_realm_serving, is_reserved_realm_name, normalize_hub_room_id,
    normalize_invite_code, normalize_realm_name, realm_name_for_username, realm_name_release_at, realms_module_enabled,
    sort_newest_first, validate_hosting_patch, validate_realm_name,
};
use rifts_realms::time::parse_iso;
use rifts_seam::{HubKind, UnixMillis};
use serde_json::{Value, json};

const T0: UnixMillis = UnixMillis(1_791_892_800_000); // 2026-10-13T12:00:00.000Z
const DAY: i64 = 24 * 3_600_000;
const KEY: &str = "Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZg==";

fn at(offset: i64) -> UnixMillis {
    UnixMillis(T0.0 + offset)
}

#[test]
fn t0_is_the_node_constant() {
    assert_eq!(parse_iso("2026-10-13T12:00:00.000Z"), Some(T0));
}

fn realm(policy: JoinPolicy, members: Vec<RealmMember>, bans: Vec<RealmBan>) -> RealmRecord {
    RealmRecord {
        realm_id: "realm-1".to_owned(),
        name: "friends".to_owned(),
        host_account_id: "host-1".to_owned(),
        host_public_key: "KEY-ONE-0123456789".to_owned(),
        join_policy: policy,
        listed: false,
        members,
        bans,
        created_at: T0,
        last_online_at: None,
        online_until: None,
        version: None,
        hosting: None,
        hub_room_ids: Vec::new(),
        offline_notified_for: None,
        join_requests: Vec::new(),
        player_invites: Vec::new(),
    }
}

fn host() -> RealmMember {
    RealmMember {
        account_id: "host-1".to_owned(),
        role: MemberRole::Host,
        joined_at: T0,
    }
}

fn friend() -> RealmMember {
    RealmMember {
        account_id: "friend-1".to_owned(),
        role: MemberRole::Member,
        joined_at: T0,
    }
}

fn invite() -> RealmInviteRecord {
    build_invite("ABCDEFGHJK".to_owned(), "realm-1", "host-1", T0, 2, 24)
}

// ------------------------------------------------------------------ realmRules.test.ts: names

#[test]
fn a_realm_name_from_a_username() {
    assert_eq!(realm_name_for_username("MysticFox").as_deref(), Some("mysticfox"));
    assert_eq!(realm_name_for_username("  Pal42 ").as_deref(), Some("pal42"));
    assert_eq!(realm_name_for_username("Old_Name.Two").as_deref(), Some("old-name-two"));
    assert_eq!(realm_name_for_username("__edge__").as_deref(), Some("edge"));
    // The username is the host's own, so a reserved word is not refused here
    assert_eq!(realm_name_for_username("Mystic").as_deref(), Some("mystic"));
    // No username yet: the placeholder is the account id, too long for a realm name
    assert_eq!(realm_name_for_username("00000000-0000-4000-8000-000000000000"), None);
    for bad in ["ab", "", "___", &"x".repeat(25)] {
        assert_eq!(realm_name_for_username(bad), None, "{bad}");
    }
}

#[test]
fn accepts_3_to_24_lower_case_letters_digits_and_inner_hyphens() {
    for name in [
        "abc",
        "ramsgate-friends",
        "a1b2c3",
        &"x".repeat(24),
        "a-b",
        "a--b",
        "123",
    ] {
        assert_eq!(validate_realm_name(name), Ok(name.to_owned()), "{name}");
    }
    assert_eq!(validate_realm_name("  Friends-Realm  "), Ok("friends-realm".to_owned()));
    assert_eq!(normalize_realm_name(" AbC ").as_deref(), Some("abc"));
}

#[test]
fn rejects_names_of_the_wrong_size_alphabet_or_hyphen_placement() {
    for name in [
        "ab",
        &"x".repeat(25),
        "",
        "   ",
        "-abc",
        "abc-",
        "-",
        "---",
        "my_realm",
        "my realm",
        "my.realm",
        "my/realm",
        "café",
        "рара",
        "a\nb",
        "ab\u{0}c",
        "<script>",
        "%24ne",
        &"a".repeat(5000),
    ] {
        assert_eq!(validate_realm_name(name), Err(NameRefusal::Invalid), "{name:?}");
    }
}

#[test]
fn refuses_reserved_names_in_any_case_and_with_hyphens_inside_them() {
    for name in [
        "official",
        "OFFICIAL",
        "paradox",
        "mysticparadox",
        "mystic-paradox",
        "o-fficial",
        "ad-min",
        "support",
        "mod",
        "admin",
    ] {
        assert_eq!(validate_realm_name(name), Err(NameRefusal::Reserved), "{name}");
    }
    assert!(!is_reserved_realm_name("my-official"));
    assert_eq!(
        validate_realm_name("official-friends"),
        Ok("official-friends".to_owned())
    );
}

#[test]
fn every_reserved_name_is_itself_well_formed() {
    for name in RESERVED_REALM_NAMES {
        assert!(
            (3..=24).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
            "{name}"
        );
    }
    for required in ["official", "paradox", "mysticparadox", "admin", "support", "mod"] {
        assert!(RESERVED_REALM_NAMES.contains(&required), "{required}");
    }
}

#[test]
fn a_realm_that_was_never_online_frees_its_name_3_days_after_it_was_claimed() {
    let r = realm(JoinPolicy::InviteOnly, vec![host()], Vec::new());
    assert_eq!(NEVER_ONLINE_RELEASE_MS, 3 * DAY);
    assert!(!is_realm_name_releasable(&r, T0));
    assert!(!is_realm_name_releasable(&r, at(3 * DAY - 1)));
    assert!(is_realm_name_releasable(&r, at(3 * DAY)));
    assert_eq!(realm_name_release_at(r.created_at, r.last_online_at), at(3 * DAY));
}

#[test]
fn a_realm_that_was_online_keeps_its_name_for_30_days_after_it_was_last_online() {
    let mut r = realm(JoinPolicy::InviteOnly, vec![host()], Vec::new());
    r.created_at = at(-100 * DAY);
    r.last_online_at = Some(T0);
    assert_eq!(LAST_ONLINE_RELEASE_MS, 30 * DAY);
    assert!(!is_realm_name_releasable(&r, at(29 * DAY)));
    assert!(!is_realm_name_releasable(&r, at(30 * DAY - 1)));
    assert!(is_realm_name_releasable(&r, at(30 * DAY)));
    assert_eq!(realm_name_release_at(r.created_at, r.last_online_at), at(30 * DAY));
    // Going online resets the clock even for a realm older than 3 days
    r.created_at = at(-5 * DAY);
    r.last_online_at = Some(at(-DAY));
    assert!(!is_realm_name_releasable(&r, T0));
}

#[test]
fn the_same_name_under_a_different_host_key_is_an_ownership_change() {
    let mut current = realm(JoinPolicy::InviteOnly, vec![host()], Vec::new());
    current.host_public_key = "KEY-TWO-0123456789".to_owned();
    assert!(detect_ownership_change("friends", "KEY-ONE-0123456789", &current));
    current.host_public_key = "KEY-ONE-0123456789".to_owned();
    assert!(!detect_ownership_change("friends", "KEY-ONE-0123456789", &current));
    current.host_public_key = "KEY-TWO-0123456789".to_owned();
    current.name = "other".to_owned();
    assert!(!detect_ownership_change("friends", "KEY-ONE-0123456789", &current));
    current.name = "FRIENDS".to_owned();
    assert!(detect_ownership_change("friends", "KEY-ONE-0123456789", &current));
}

#[test]
fn invite_codes_are_10_characters_of_an_alphabet_without_look_alikes() {
    assert_eq!(INVITE_CODE_LENGTH, 10);
    assert_eq!(INVITE_ALPHABET.len(), 32);
    for ambiguous in *b"IO01" {
        assert!(!INVITE_ALPHABET.contains(&ambiguous));
    }
    assert_eq!(INVITE_ALPHABET.iter().collect::<BTreeSet<_>>().len(), 32);
    let mut seen = BTreeSet::new();
    for _ in 0..2000 {
        let code = generate_invite_code();
        assert!(
            code.len() == 10 && code.bytes().all(|b| INVITE_ALPHABET.contains(&b)),
            "{code}"
        );
        seen.insert(code);
    }
    assert_eq!(seen.len(), 2000, "2000 random codes must not collide");
}

#[test]
fn invite_code_generation_maps_random_bytes_without_bias() {
    assert_eq!(invite_code_from([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), "ABCDEFGHJK");
    // Only the low 5 bits are used: 255 and 31 both pick the last symbol, 32 the first
    assert_eq!(invite_code_from([255, 31, 32, 63, 64, 0, 0, 0, 0, 0]), "99A9AAAAAA");
}

#[test]
fn a_typed_or_pasted_invite_code_is_normalised() {
    assert_eq!(normalize_invite_code("abcde-fghjk").as_deref(), Some("ABCDEFGHJK"));
    assert_eq!(normalize_invite_code("  ABCDE FGHJK ").as_deref(), Some("ABCDEFGHJK"));
    assert_eq!(
        normalize_invite_code("a-b-c-d-e-f-g-h-j-k").as_deref(),
        Some("ABCDEFGHJK")
    );
    for bad in [
        "ABCDEFGHJ",
        "ABCDEFGHJKL",
        "ABCDEFGHIK",
        "ABCDEFGHOK",
        "ABCDE0GHJK",
        "ABCDE1GHJK",
        "ABCDE_GHJK",
        "",
        &"x".repeat(41),
    ] {
        assert_eq!(normalize_invite_code(bad), None, "{bad}");
    }
}

#[test]
fn a_fresh_invite_is_usable_until_it_expires_is_revoked_or_is_used_up() {
    let fresh = invite();
    assert_eq!(fresh.uses, 0);
    assert_eq!(fresh.created_at, T0);
    assert_eq!(fresh.expires_at, at(DAY));
    assert_eq!(invite_state(&fresh, T0), InviteState::Usable);
    assert_eq!(invite_state(&fresh, at(DAY - 1)), InviteState::Usable);
    assert_eq!(invite_state(&fresh, at(DAY)), InviteState::Expired);
    let used = |uses| RealmInviteRecord { uses, ..invite() };
    assert_eq!(invite_state(&used(1), T0), InviteState::Usable);
    assert_eq!(invite_state(&used(2), T0), InviteState::Exhausted);
    assert_eq!(invite_state(&used(3), T0), InviteState::Exhausted);
    let revoked = RealmInviteRecord {
        revoked_at: Some(T0),
        ..invite()
    };
    assert_eq!(invite_state(&revoked, T0), InviteState::Revoked);
    // Revoked outranks every other reason
    assert_eq!(
        invite_state(&RealmInviteRecord { uses: 2, ..revoked }, at(2 * DAY)),
        InviteState::Revoked
    );
}

#[test]
fn a_banned_account_is_refused_whatever_else_is_true() {
    let bans = vec![RealmBan {
        account_id: "bad-1".to_owned(),
        reason: None,
        at: T0,
    }];
    let invite_only = realm(JoinPolicy::InviteOnly, vec![host()], bans.clone());
    assert_eq!(decide_join(&invite_only, "bad-1", None, T0), Err(JoinDenial::Banned));
    let public = realm(JoinPolicy::Public, vec![host()], bans.clone());
    assert_eq!(decide_join(&public, "bad-1", None, T0), Err(JoinDenial::Banned));
    assert_eq!(
        decide_join(&invite_only, "bad-1", Some(&invite()), T0),
        Err(JoinDenial::Banned)
    );
    // Even a leftover membership does not outrank the ban
    let leftover = RealmMember {
        account_id: "bad-1".to_owned(),
        ..friend()
    };
    let with_member = realm(JoinPolicy::InviteOnly, vec![host(), leftover], bans);
    assert_eq!(decide_join(&with_member, "bad-1", None, T0), Err(JoinDenial::Banned));
}

#[test]
fn a_public_realm_lets_anyone_in_an_invite_only_one_needs_a_membership_or_an_invite() {
    let public = realm(JoinPolicy::Public, vec![host()], Vec::new());
    assert_eq!(decide_join(&public, "stranger", None, T0), Ok(JoinVia::Public));
    let closed = realm(JoinPolicy::InviteOnly, vec![host()], Vec::new());
    assert_eq!(
        decide_join(&closed, "stranger", None, T0),
        Err(JoinDenial::InviteRequired)
    );
    assert_eq!(
        decide_join(&closed, "stranger", Some(&invite()), T0),
        Ok(JoinVia::Invite)
    );
    let other = RealmInviteRecord {
        realm_id: "realm-2".to_owned(),
        ..invite()
    };
    assert_eq!(
        decide_join(&closed, "stranger", Some(&other), T0),
        Err(JoinDenial::InviteInvalid)
    );
    let revoked = RealmInviteRecord {
        revoked_at: Some(T0),
        ..invite()
    };
    assert_eq!(
        decide_join(&closed, "stranger", Some(&revoked), T0),
        Err(JoinDenial::InviteExpired)
    );
    let used = RealmInviteRecord { uses: 2, ..invite() };
    assert_eq!(
        decide_join(&closed, "stranger", Some(&used), T0),
        Err(JoinDenial::InviteExpired)
    );
    assert_eq!(
        decide_join(&closed, "stranger", Some(&invite()), at(2 * DAY)),
        Err(JoinDenial::InviteExpired)
    );
}

#[test]
fn the_host_and_members_are_already_in_with_no_invite_needed() {
    let r = realm(JoinPolicy::InviteOnly, vec![host(), friend()], Vec::new());
    assert_eq!(decide_join(&r, "host-1", None, T0), Ok(JoinVia::Member));
    assert_eq!(decide_join(&r, "friend-1", None, T0), Ok(JoinVia::Member));
    let used = RealmInviteRecord { uses: 2, ..invite() };
    assert_eq!(decide_join(&r, "friend-1", Some(&used), T0), Ok(JoinVia::Member));
}

#[test]
fn a_heartbeat_keeps_the_realm_online_for_90_seconds() {
    assert_eq!(HEARTBEAT_ONLINE_MS, 90_000);
    let update = apply_heartbeat(T0, None, None);
    assert_eq!(
        (update.last_online_at, update.online_until, update.version),
        (T0, at(90_000), None)
    );
    let mut r = realm(JoinPolicy::InviteOnly, vec![host()], Vec::new());
    r.online_until = Some(update.online_until);
    assert!(is_realm_online(&r, T0));
    assert!(is_realm_online(&r, at(89_999)));
    assert!(!is_realm_online(&r, at(90_000)));
}

#[test]
fn a_heartbeat_records_the_version_only_when_sent_and_online_false_ends_it_now() {
    assert_eq!(apply_heartbeat(T0, None, None).version, None);
    assert_eq!(
        apply_heartbeat(T0, None, Some("0.2.128".to_owned())).version.as_deref(),
        Some("0.2.128")
    );
    let closing = apply_heartbeat(T0, Some(false), Some("0.2.128".to_owned()));
    assert_eq!(closing.online_until, T0);
    assert_eq!(
        closing.last_online_at, T0,
        "closing on purpose still counts as online at least once"
    );
    assert_eq!(apply_heartbeat(T0, Some(true), None).online_until, at(90_000));
    let r = realm(JoinPolicy::InviteOnly, vec![host()], Vec::new());
    assert!(!is_realm_online(&r, T0), "no heartbeat");
}

#[test]
fn join_requests_are_listed_newest_first_with_stored_order_for_ties() {
    let requests: Vec<AccountAt> = [("a", 0), ("b", 2_000), ("c", 0), ("d", 1_000)]
        .iter()
        .map(|(id, offset)| AccountAt {
            account_id: (*id).to_owned(),
            at: at(*offset),
        })
        .collect();
    let sorted: Vec<String> = sort_newest_first(&requests).into_iter().map(|r| r.account_id).collect();
    assert_eq!(sorted, ["b", "d", "a", "c"]);
    assert!(sort_newest_first(&[]).is_empty());
}

// ------------------------------------------------------------------ realmInput.test.ts

fn code<T: std::fmt::Debug>(result: Result<T, rifts_realms::input::ParseFailure>) -> &'static str {
    result.err().map_or("ok", |failure| failure.code)
}

#[test]
fn creating_a_realm_defaults_to_invite_only_and_unlisted_and_ignores_unknown_keys() {
    let parsed = parse_create_realm_body(Some(
        &json!({ "name": " Friends-Realm ", "hostPublicKey": KEY, "isAdmin": true }),
    ));
    assert_eq!(
        parsed,
        Ok(CreateRealmInput {
            name: Some("friends-realm".to_owned()),
            host_public_key: KEY.to_owned(),
            join_policy: JoinPolicy::InviteOnly,
            listed: false,
            version: None,
        })
    );
    let full =
        json!({ "name": "abc", "hostPublicKey": KEY, "joinPolicy": "PUBLIC", "listed": true, "version": "0.2.128" });
    assert_eq!(
        parse_create_realm_body(Some(&full)),
        Ok(CreateRealmInput {
            name: Some("abc".to_owned()),
            host_public_key: KEY.to_owned(),
            join_policy: JoinPolicy::Public,
            listed: true,
            version: Some("0.2.128".to_owned()),
        })
    );
}

#[test]
fn creating_a_realm_checks_each_field_strictly() {
    let body = |v: Value| parse_create_realm_body(Some(&v));
    assert_eq!(
        code(body(json!({ "name": "ab", "hostPublicKey": KEY }))),
        "REALM_NAME_INVALID"
    );
    let unnamed = body(json!({ "hostPublicKey": KEY })).unwrap();
    assert_eq!(unnamed.name, None);
    assert_eq!(
        code(body(json!({ "name": "", "hostPublicKey": KEY }))),
        "REALM_NAME_INVALID"
    );
    assert_eq!(
        code(body(json!({ "name": null, "hostPublicKey": KEY }))),
        "REALM_NAME_INVALID"
    );
    assert_eq!(
        code(body(json!({ "name": "Official", "hostPublicKey": KEY }))),
        "REALM_NAME_RESERVED"
    );
    assert_eq!(code(body(json!({ "name": "abc" }))), "REALM_VALIDATION_FAILED");
    for key in [
        json!("short"),
        json!("x".repeat(513)),
        json!("has spaces in it 1234"),
        json!("<script>alert(1)</script>"),
        json!(12_345_678_901_234_567_890_u64),
        json!(null),
        json!({}),
        json!([KEY]),
    ] {
        assert_eq!(
            code(body(json!({ "name": "abc", "hostPublicKey": key }))),
            "REALM_VALIDATION_FAILED",
            "{key}"
        );
    }
    for policy in [
        json!("public"),
        json!("OPEN"),
        json!(""),
        json!(1),
        json!(true),
        json!(null),
        json!({}),
    ] {
        let v = json!({ "name": "abc", "hostPublicKey": KEY, "joinPolicy": policy });
        assert_eq!(code(body(v)), "REALM_VALIDATION_FAILED", "{policy}");
    }
    for listed in [json!("true"), json!(1), json!(0), json!("yes"), json!(null), json!({})] {
        let v = json!({ "name": "abc", "hostPublicKey": KEY, "listed": listed });
        assert_eq!(code(body(v)), "REALM_VALIDATION_FAILED", "{listed}");
    }
    for version in [json!(""), json!("1 2"), json!("x".repeat(65)), json!(1), json!("<b>")] {
        let v = json!({ "name": "abc", "hostPublicKey": KEY, "version": version });
        assert_eq!(code(body(v)), "REALM_VALIDATION_FAILED", "{version}");
    }
    for v in [json!([]), json!("abc"), json!(5), json!(true)] {
        assert_eq!(code(body(v)), "REALM_VALIDATION_FAILED");
    }
}

#[test]
fn settings_need_at_least_one_valid_setting() {
    let body = |v: Value| parse_settings_body(Some(&v));
    assert_eq!(
        body(json!({ "joinPolicy": "PUBLIC" })),
        Ok(SettingsInput {
            join_policy: Some(JoinPolicy::Public),
            listed: None
        })
    );
    assert_eq!(
        body(json!({ "listed": false })),
        Ok(SettingsInput {
            join_policy: None,
            listed: Some(false)
        })
    );
    assert_eq!(
        body(json!({ "joinPolicy": "INVITE_ONLY", "listed": true, "hostPublicKey": "ignored" })),
        Ok(SettingsInput {
            join_policy: Some(JoinPolicy::InviteOnly),
            listed: Some(true)
        })
    );
    assert_eq!(code(body(json!({}))), "REALM_VALIDATION_FAILED");
    assert_eq!(code(parse_settings_body(None)), "REALM_VALIDATION_FAILED");
    assert_eq!(code(body(json!({ "name": "other" }))), "REALM_VALIDATION_FAILED");
    assert_eq!(code(body(json!({ "joinPolicy": "NOPE" }))), "REALM_VALIDATION_FAILED");
    assert_eq!(code(body(json!({ "listed": "yes" }))), "REALM_VALIDATION_FAILED");
}

#[test]
fn invite_options_default_to_10_uses_and_7_days() {
    let defaults = InviteInput {
        max_uses: 10,
        ttl_hours: 168,
    };
    assert_eq!(parse_invite_body(None), Ok(defaults));
    assert_eq!(parse_invite_body(Some(&json!({}))), Ok(defaults));
    assert_eq!(
        parse_invite_body(Some(&json!({ "maxUses": 1, "expiresInHours": 1 }))),
        Ok(InviteInput {
            max_uses: 1,
            ttl_hours: 1
        })
    );
    assert_eq!(
        parse_invite_body(Some(&json!({ "maxUses": 100, "expiresInHours": 720 }))),
        Ok(InviteInput {
            max_uses: 100,
            ttl_hours: 720
        })
    );
    for max in [
        json!(0),
        json!(101),
        json!(-1),
        json!(1.5),
        json!("5"),
        json!(null),
        json!(9_007_199_254_740_993_u64),
    ] {
        assert_eq!(
            code(parse_invite_body(Some(&json!({ "maxUses": max })))),
            "REALM_VALIDATION_FAILED",
            "{max}"
        );
    }
    for hours in [json!(0), json!(721), json!(-5), json!(0.5), json!("24"), json!(null)] {
        let v = json!({ "expiresInHours": hours });
        assert_eq!(code(parse_invite_body(Some(&v))), "REALM_VALIDATION_FAILED", "{hours}");
    }
}

#[test]
fn joining_takes_a_name_and_or_an_invite_code_plus_an_optional_known_host_key() {
    let body = |v: Value| parse_join_body(Some(&v));
    let join = |name: Option<&str>, code: Option<&str>, key: Option<&str>| JoinInput {
        name: name.map(str::to_owned),
        invite_code: code.map(str::to_owned),
        known_host_key: key.map(str::to_owned),
    };
    assert_eq!(
        body(json!({ "name": "Friends" })),
        Ok(join(Some("friends"), None, None))
    );
    assert_eq!(
        body(json!({ "inviteCode": "abcde-fghjk" })),
        Ok(join(None, Some("ABCDEFGHJK"), None))
    );
    assert_eq!(
        body(json!({ "name": "friends", "inviteCode": "ABCDEFGHJK", "knownHostKey": KEY })),
        Ok(join(Some("friends"), Some("ABCDEFGHJK"), Some(KEY)))
    );
    assert_eq!(code(body(json!({}))), "REALM_VALIDATION_FAILED");
    assert_eq!(code(body(json!({ "knownHostKey": KEY }))), "REALM_VALIDATION_FAILED");
    assert_eq!(code(body(json!({ "name": "-bad-" }))), "REALM_NAME_INVALID");
    assert_eq!(
        code(body(json!({ "inviteCode": "ABCDEFGHIK" }))),
        "REALM_VALIDATION_FAILED"
    );
    assert_eq!(
        code(body(json!({ "name": "friends", "knownHostKey": "bad key" }))),
        "REALM_VALIDATION_FAILED"
    );
    assert_eq!(code(body(json!({ "name": { "$ne": "x" } }))), "REALM_NAME_INVALID");
    assert_eq!(
        code(body(json!({ "inviteCode": { "$ne": "x" } }))),
        "REALM_VALIDATION_FAILED"
    );
    // A join names an existing realm: a well-formed reserved name passes
    assert_eq!(
        body(json!({ "name": "MysticFox " })),
        Ok(join(Some("mysticfox"), None, None))
    );
    assert_eq!(
        body(json!({ "name": "mysticfox", "knownHostKey": KEY })),
        Ok(join(Some("mysticfox"), None, Some(KEY)))
    );
    assert_eq!(code(body(json!({ "name": "-mysticfox" }))), "REALM_NAME_INVALID");
    assert_eq!(
        code(parse_create_realm_body(Some(&json!({ "name": "mysticfox" })))),
        "REALM_NAME_RESERVED"
    );
}

#[test]
fn heartbeat_takes_an_optional_online_flag_and_version() {
    assert_eq!(parse_heartbeat_body(None), Ok(HeartbeatInput::default()));
    assert_eq!(
        parse_heartbeat_body(Some(&json!({ "online": false, "version": "0.2.128" }))),
        Ok(HeartbeatInput {
            online: Some(false),
            version: Some("0.2.128".to_owned()),
            hosting: None
        })
    );
    assert_eq!(
        code(parse_heartbeat_body(Some(&json!({ "online": "false" })))),
        "REALM_VALIDATION_FAILED"
    );
    assert_eq!(
        code(parse_heartbeat_body(Some(&json!({ "version": "bad version" })))),
        "REALM_VALIDATION_FAILED"
    );
}

#[test]
fn ban_reason_is_optional_trimmed_200_characters_at_most_no_control_characters() {
    assert_eq!(parse_ban_body(None), Ok(None));
    assert_eq!(
        parse_ban_body(Some(&json!({ "reason": "  griefing  " }))),
        Ok(Some("griefing".to_owned()))
    );
    assert_eq!(parse_ban_body(Some(&json!({ "reason": "   " }))), Ok(None));
    assert_eq!(
        parse_ban_body(Some(&json!({ "reason": "x".repeat(200) }))),
        Ok(Some("x".repeat(200)))
    );
    for bad in [
        json!({ "reason": "x".repeat(201) }),
        json!({ "reason": "line\nbreak" }),
        json!({ "reason": "nul\u{0}byte" }),
        json!({ "reason": 5 }),
        json!([]),
    ] {
        assert_eq!(code(parse_ban_body(Some(&bad))), "REALM_VALIDATION_FAILED", "{bad}");
    }
}

#[test]
fn listing_query_is_digits_only_limit_1_to_50_offset_0_to_10000() {
    assert_eq!(parse_list_query(None, None), Ok((25, 0)));
    assert_eq!(parse_list_query(Some("50"), Some("10000")), Ok((50, 10_000)));
    for limit in ["0", "51", "-1", "1.5", "abc", "", " 5", "1e1", "0x10"] {
        assert_eq!(
            code(parse_list_query(Some(limit), None)),
            "REALM_VALIDATION_FAILED",
            "{limit}"
        );
    }
    for offset in ["10001", "-1", "x", ""] {
        assert_eq!(
            code(parse_list_query(None, Some(offset))),
            "REALM_VALIDATION_FAILED",
            "{offset}"
        );
    }
}

#[test]
fn path_and_query_values_names_invite_codes_account_ids_and_host_keys() {
    assert_eq!(parse_name_param(Some("Friends")), Ok("friends".to_owned()));
    // A reserved name is well-formed: looking it up is "not found", not "invalid"
    assert_eq!(parse_name_param(Some("official")), Ok("official".to_owned()));
    assert_eq!(code(parse_name_param(Some("a b"))), "REALM_NAME_INVALID");
    assert_eq!(code(parse_name_param(None)), "REALM_NAME_INVALID");
    assert_eq!(parse_invite_code_param("abcde-fghjk"), Ok("ABCDEFGHJK".to_owned()));
    assert_eq!(code(parse_invite_code_param("nope")), "REALM_VALIDATION_FAILED");
    let account = "0d0f7e2c-5f3a-4a55-9f3e-6b1d2c3a4b5c";
    assert_eq!(parse_account_id_param(account).as_deref(), Some(account));
    assert_eq!(parse_account_id_param("dev:user_1.a").as_deref(), Some("dev:user_1.a"));
    for bad in ["", "$ne", "a b", "a/b", &"x".repeat(129), "a%00b", "../etc"] {
        assert_eq!(parse_account_id_param(bad), None, "{bad}");
    }
    assert_eq!(parse_host_public_key(&json!(KEY)).as_deref(), Some(KEY));
    assert_eq!(
        parse_host_public_key(&json!(format!("  {KEY}  "))).as_deref(),
        Some(KEY)
    );
    assert_eq!(parse_host_public_key(&json!("tooshort")), None);
    assert_eq!(parse_version(&json!("0.2.128+rc1")).as_deref(), Some("0.2.128+rc1"));
    assert_eq!(parse_version(&json!("0.2 128")), None);
    assert_eq!(parse_known_host_key_query(None), Ok(None));
    assert_eq!(parse_known_host_key_query(Some(KEY)), Ok(Some(KEY.to_owned())));
    assert_eq!(
        code(parse_known_host_key_query(Some("bad key"))),
        "REALM_VALIDATION_FAILED"
    );
}

#[test]
fn the_module_switch_is_on_only_for_on_true_or_1() {
    let env = |v: &str| BTreeMap::from([("MYSTICPARADOX_REALMS_MODULE".to_owned(), v.to_owned())]);
    for value in ["on", "ON", "true", "True", "1", " on "] {
        assert!(realms_module_enabled(&env(value)), "{value}");
    }
    for value in ["", "off", "false", "0", "yes", "enabled", "2", "onn"] {
        assert!(!realms_module_enabled(&env(value)), "{value}");
    }
    assert!(!realms_module_enabled(&BTreeMap::new()));
}

#[test]
fn adding_a_player_needs_a_trimmed_username_of_1_to_64_plain_characters() {
    assert_eq!(
        parse_username_body(Some(&json!({ "username": "  Friendly " }))),
        Ok("Friendly".to_owned())
    );
    assert_eq!(
        parse_username_body(Some(&json!({ "username": "x", "accountId": "someone-else" }))),
        Ok("x".to_owned())
    );
    assert_eq!(
        parse_username_body(Some(&json!({ "username": "x".repeat(64) }))),
        Ok("x".repeat(64))
    );
    assert_eq!(code(parse_username_body(None)), "REALM_VALIDATION_FAILED");
    for body in [
        json!(null),
        json!({}),
        json!({ "username": 7 }),
        json!({ "username": ["a"] }),
        json!({ "username": "" }),
        json!({ "username": "   " }),
        json!({ "username": "x".repeat(65) }),
        json!({ "username": "a\tb" }),
        json!({ "username": "a\u{7f}b" }),
        json!([]),
    ] {
        assert_eq!(
            code(parse_username_body(Some(&body))),
            "REALM_VALIDATION_FAILED",
            "{body}"
        );
    }
}

#[test]
fn realm_limits_default_bounded_overrides_garbage_ignored() {
    let env = |k: &str, v: &str| BTreeMap::from([(k.to_owned(), v.to_owned())]);
    assert_eq!(RealmLimits::from_settings(&BTreeMap::new()), DEFAULT_REALM_LIMITS);
    assert_eq!(
        RealmLimits::from_settings(&env("MYSTICPARADOX_REALMS_MAX_MEMBERS", "50")).max_members,
        50
    );
    assert_eq!(
        RealmLimits::from_settings(&env("MYSTICPARADOX_REALMS_MAX_MEMBERS", "1")).max_members,
        1000
    );
    assert_eq!(
        RealmLimits::from_settings(&env("MYSTICPARADOX_REALMS_MAX_MEMBERS", "99999")).max_members,
        1000
    );
    assert_eq!(
        RealmLimits::from_settings(&env("MYSTICPARADOX_REALMS_MAX_BANS", "abc")).max_bans,
        1000
    );
}

// ------------------------------------------------------------------ realms/realmHubRules.test.ts

fn hosting(state: HubState) -> RealmHosting {
    RealmHosting {
        max_servers: 2,
        max_players: 16,
        afk_seconds: None,
        hub_session_id: None,
        hub_state: state,
        training_state: HubState::None,
        active_servers: 1,
        updated_at: T0,
    }
}

#[test]
fn a_realm_is_serving_only_while_its_host_is_online_and_its_hub_is_running() {
    let mut r = realm(JoinPolicy::InviteOnly, vec![host()], Vec::new());
    r.online_until = Some(at(1000));
    r.hosting = Some(hosting(HubState::Running));
    assert!(is_realm_serving(&r, T0));
    for state in [HubState::Starting, HubState::Stopping, HubState::None] {
        r.hosting = Some(hosting(state));
        assert!(!is_realm_serving(&r, T0), "{state:?}");
    }
    r.hosting = None;
    assert!(!is_realm_serving(&r, T0), "no hosting record at all");
    r.hosting = Some(hosting(HubState::Running));
    r.online_until = Some(T0);
    assert!(!is_realm_serving(&r, T0), "online state lapsed exactly now");
    r.online_until = None;
    assert!(!is_realm_serving(&r, T0), "never online");
    // The hub itself can be ready while the host's heartbeat has lapsed
    assert!(is_realm_hub_ready(&r));
    r.hosting = None;
    assert!(!is_realm_hub_ready(&r));
}

#[test]
fn a_hub_room_id_is_deterministic_uuid_shaped_and_different_per_realm_and_kind() {
    let id = hub_room_id("realm-1", HubKind::Ramsgate);
    let b = id.as_bytes();
    assert_eq!(id.len(), 36);
    assert_eq!(b[14], b'5');
    assert!(matches!(b[19], b'8' | b'9' | b'a' | b'b'), "{id}");
    assert_eq!(hub_room_id("realm-1", HubKind::Ramsgate), id);
    assert_ne!(hub_room_id("realm-1", HubKind::Training), id);
    assert_ne!(hub_room_id("realm-2", HubKind::Ramsgate), id);
    assert_eq!(
        hub_room_ids_of("realm-1"),
        [id, hub_room_id("realm-1", HubKind::Training)]
    );
}

#[test]
fn a_hub_room_id_from_a_request_is_accepted_only_in_its_uuid_form() {
    let id = hub_room_id("realm-1", HubKind::Training);
    assert_eq!(normalize_hub_room_id(&id.to_uppercase()), Some(id.clone()));
    let bad = [
        String::new(),
        "not-a-uuid".to_owned(),
        format!("{id}0"),
        id[1..].to_owned(),
        id.replace('-', ""),
        format!("{}{}", "g".repeat(8), &id[8..]),
    ];
    for bad in bad {
        assert_eq!(normalize_hub_room_id(&bad), None, "{bad}");
    }
}

#[test]
fn a_hosting_patch_changes_only_what_it_names() {
    let starting = HostingPatch {
        hub_state: Some(HubState::Starting),
        hub_session_id: Patch::Set("sess-1".to_owned()),
        ..HostingPatch::default()
    };
    let fresh = apply_hosting_patch(None, &starting, T0);
    assert_eq!(
        fresh,
        RealmHosting {
            max_servers: 1,
            max_players: 16,
            afk_seconds: None,
            hub_session_id: Some("sess-1".to_owned()),
            hub_state: HubState::Starting,
            training_state: HubState::None,
            active_servers: 0,
            updated_at: T0,
        }
    );
    let afk = HostingPatch {
        max_servers: Some(4),
        afk_seconds: Patch::Set(600),
        ..HostingPatch::default()
    };
    let with_afk = apply_hosting_patch(Some(fresh), &afk, at(1));
    assert_eq!((with_afk.max_servers, with_afk.afk_seconds), (4, Some(600)));
    assert_eq!(
        with_afk.hub_state,
        HubState::Starting,
        "an unnamed field keeps its value"
    );
    assert_eq!(with_afk.updated_at, at(1));
    let clear = HostingPatch {
        afk_seconds: Patch::Remove,
        hub_session_id: Patch::Remove,
        hub_state: Some(HubState::None),
        ..HostingPatch::default()
    };
    let cleared = apply_hosting_patch(Some(with_afk), &clear, at(2));
    assert_eq!((cleared.afk_seconds, cleared.hub_session_id.as_deref()), (None, None));
    assert_eq!(cleared.hub_state, HubState::None);
    assert_eq!(cleared.updated_at, at(2), "the patch cannot set updatedAt");
}

#[test]
fn a_hosting_patch_from_the_p2p_side_is_checked() {
    assert_eq!(validate_hosting_patch(&HostingPatch::default()), None);
    let full = HostingPatch {
        max_servers: Some(8),
        max_players: Some(64),
        afk_seconds: Patch::Set(7200),
        active_servers: Some(8),
        hub_state: Some(HubState::Running),
        training_state: Some(HubState::Stopping),
        hub_session_id: Patch::Set("s".to_owned()),
    };
    assert_eq!(validate_hosting_patch(&full), None);
    let removal = HostingPatch {
        afk_seconds: Patch::Remove,
        hub_session_id: Patch::Remove,
        ..HostingPatch::default()
    };
    assert_eq!(validate_hosting_patch(&removal), None);
    let check = |patch: HostingPatch| validate_hosting_patch(&patch);
    let d = HostingPatch::default;
    assert_eq!(
        check(HostingPatch {
            max_servers: Some(0),
            ..d()
        }),
        Some("maxServers")
    );
    assert_eq!(
        check(HostingPatch {
            max_servers: Some(9),
            ..d()
        }),
        Some("maxServers")
    );
    assert_eq!(
        check(HostingPatch {
            max_players: Some(65),
            ..d()
        }),
        Some("maxPlayers")
    );
    assert_eq!(
        check(HostingPatch {
            afk_seconds: Patch::Set(7201),
            ..d()
        }),
        Some("afkSeconds")
    );
    assert_eq!(
        check(HostingPatch {
            active_servers: Some(-1),
            ..d()
        }),
        Some("activeServers")
    );
    assert_eq!(
        check(HostingPatch {
            hub_session_id: Patch::Set(String::new()),
            ..d()
        }),
        Some("hubSessionId")
    );
    assert_eq!(
        check(HostingPatch {
            hub_session_id: Patch::Set("x".repeat(129)),
            ..d()
        }),
        Some("hubSessionId")
    );
}

#[test]
fn the_host_heartbeat_hosting_block() {
    let body = |v: Value| parse_heartbeat_body(Some(&v));
    assert_eq!(
        body(json!({ "hosting": { "maxServers": 3, "maxPlayers": 16 } }))
            .unwrap()
            .hosting,
        Some(HostingInput {
            max_servers: 3,
            max_players: 16,
            afk_seconds: None
        })
    );
    let full = body(
        json!({ "online": true, "hosting": { "maxServers": 8, "maxPlayers": 64, "afkSeconds": 0, "isAdmin": true } }),
    );
    assert_eq!(
        full,
        Ok(HeartbeatInput {
            online: Some(true),
            version: None,
            hosting: Some(HostingInput {
                max_servers: 8,
                max_players: 64,
                afk_seconds: Some(0)
            })
        })
    );
    assert_eq!(
        body(json!({ "online": false })).unwrap().hosting,
        None,
        "hosting stays optional"
    );
    for hosting in [
        json!("x"),
        json!([]),
        json!(null),
        json!({}),
        json!({ "maxServers": 3 }),
        json!({ "maxPlayers": 16 }),
        json!({ "maxServers": 0, "maxPlayers": 16 }),
        json!({ "maxServers": 9, "maxPlayers": 16 }),
        json!({ "maxServers": 3, "maxPlayers": 0 }),
        json!({ "maxServers": 3, "maxPlayers": 65 }),
        json!({ "maxServers": "3", "maxPlayers": 16 }),
        json!({ "maxServers": 1.5, "maxPlayers": 16 }),
        json!({ "maxServers": 3, "maxPlayers": 16, "afkSeconds": 7201 }),
        json!({ "maxServers": 3, "maxPlayers": 16, "afkSeconds": -1 }),
        json!({ "maxServers": 3, "maxPlayers": 16, "afkSeconds": "60" }),
    ] {
        assert_eq!(
            code(body(json!({ "hosting": hosting }))),
            "REALM_VALIDATION_FAILED",
            "{hosting}"
        );
    }
}

#[test]
fn the_realm_of_a_game_session_needs_a_name() {
    let entry = |name: &str, code: Option<&str>, key: Option<&str>| EntryRealmInput {
        name: name.to_owned(),
        invite_code: code.map(str::to_owned),
        known_host_key: key.map(str::to_owned),
    };
    assert_eq!(
        parse_entry_realm(&json!({ "name": " Friends " })),
        Ok(entry("friends", None, None))
    );
    assert_eq!(
        parse_entry_realm(&json!({ "name": "friends", "inviteCode": "abcde-fghjk", "knownHostKey": KEY, "extra": 1 })),
        Ok(entry("friends", Some("ABCDEFGHJK"), Some(KEY)))
    );
    assert_eq!(
        parse_entry_realm(&json!({ "name": "official" })),
        Ok(entry("official", None, None))
    );
    assert_eq!(code(parse_entry_realm(&json!({ "name": "ab" }))), "REALM_NAME_INVALID");
    assert_eq!(
        code(parse_entry_realm(&json!({ "name": { "$ne": "x" } }))),
        "REALM_NAME_INVALID"
    );
    assert_eq!(code(parse_entry_realm(&json!({}))), "REALM_VALIDATION_FAILED");
    assert_eq!(
        code(parse_entry_realm(
            &json!({ "name": "friends", "inviteCode": "ABCDEFGHIK" })
        )),
        "REALM_VALIDATION_FAILED"
    );
    assert_eq!(
        code(parse_entry_realm(
            &json!({ "name": "friends", "inviteCode": { "$ne": "x" } })
        )),
        "REALM_VALIDATION_FAILED"
    );
    assert_eq!(
        code(parse_entry_realm(
            &json!({ "name": "friends", "knownHostKey": "short" })
        )),
        "REALM_VALIDATION_FAILED"
    );
    for bad in [json!("friends"), json!(5), json!(true), json!([]), json!(["friends"])] {
        assert_eq!(code(parse_entry_realm(&bad)), "REALM_VALIDATION_FAILED", "{bad}");
    }
}
