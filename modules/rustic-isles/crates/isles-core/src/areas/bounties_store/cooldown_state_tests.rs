//! `cooldownState.test.ts`, ported.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};

use super::authn::AuthData;
use super::cooldown_state::*;
use super::util::{iso, parse_ms};
use crate::ids::AccountId;
use crate::repo::bounties::{CooldownEntry, CooldownStateRecord};

const HOUR_MS: i64 = 3_600_000;
const NOW: &str = "2026-10-10T18:00:00.000Z";
const HOST_NOW: &str = "2026-10-13T18:00:00.000Z";

fn user() -> AccountId {
    "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@standalone".parse().unwrap()
}

fn ms(text: &str) -> i64 {
    parse_ms(text).unwrap()
}

/// A 32-hex gatherable node GUID, unique per index.
fn gatherable(index: usize) -> String {
    format!("58E91700491B00FA{index:016X}")
}

fn stored(entries: &[(&str, &str, Option<&str>)]) -> CooldownStateRecord {
    CooldownStateRecord {
        account: user(),
        entries: entries
            .iter()
            .map(|(id, started, updated)| CooldownEntry {
                id: (*id).into(),
                started_date: (*started).into(),
                updated_at: updated.unwrap_or(started).into(),
            })
            .collect(),
        update_version: 7,
        created_at: "2026-09-01T00:00:00.000Z".into(),
        updated_at: "2026-10-05T00:00:00.000Z".into(),
    }
}

fn owned(entries: Vec<(String, String, Option<String>)>) -> CooldownStateRecord {
    let borrowed: Vec<(&str, &str, Option<&str>)> = entries
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_deref()))
        .collect();
    stored(&borrowed)
}

fn body(entries: &[(&str, &str)]) -> Value {
    json!({ "cooldowns": entries.iter().map(|(id, date)| json!({ "cooldown_id": id, "cooldown_started_date": date })).collect::<Vec<_>>() })
}

fn merge(previous: Option<&CooldownStateRecord>, body: &Value, now: &str) -> CooldownStateRecord {
    let incoming = validate_cooldown_save(Some(body)).unwrap();
    merge_cooldown_state(&user(), previous, &incoming, now)
}

fn merge_at(previous: Option<&CooldownStateRecord>, body: &Value) -> CooldownStateRecord {
    merge(previous, body, "2026-10-10T08:41:49.822Z")
}

fn host_save(
    previous: Option<&CooldownStateRecord>,
    entries: &[(&str, &str)],
) -> (CooldownStateRecord, HostCooldownChanges) {
    merge_host_cooldown_state(&user(), previous, Some(&body(entries)), HOST_NOW).unwrap()
}

fn payload(record: &CooldownStateRecord) -> Value {
    cooldown_state_payload(Some(record))
}

fn host_runtime() -> AuthData {
    AuthData {
        user_id: Some("player-1".into()),
        is_gameserver: false,
        host_runtime: Some(rifts_seam::RuntimeAuth {
            capability_id: "c".into(),
            session: "01ARZ3NDEKTSV4RRFFQ69G5FAV@standalone".parse().unwrap(),
            generation: 1,
            host_account: rifts_seam::AccountId("host".into()),
            scopes: vec![],
            roster: vec![
                rifts_seam::AccountId("player-1".into()),
                rifts_seam::AccountId("player-2".into()),
            ],
        }),
    }
}

#[test]
fn cooldown_writes_are_dedicated_only_reads_are_server_or_same_account() {
    assert!(is_authorized_cooldown_writer(&AuthData::gameserver()));
    assert!(!is_authorized_cooldown_writer(&AuthData::player("player-1")));
    assert!(!is_authorized_cooldown_writer(&host_runtime()));
    assert!(is_authorized_cooldown_reader(&AuthData::gameserver(), "player-2", None));
    assert!(is_authorized_cooldown_reader(
        &AuthData::player("player-1"),
        "player-1",
        None
    ));
    assert!(!is_authorized_cooldown_reader(
        &AuthData::player("player-1"),
        "player-2",
        None
    ));
}

#[test]
fn a_host_runtime_reads_its_roster_only_and_fails_closed_without_one() {
    let host = host_runtime();
    assert!(is_authorized_cooldown_reader(&host, "player-1", None));
    assert!(is_authorized_cooldown_reader(&host, "player-2", None));
    assert!(!is_authorized_cooldown_reader(&host, "player-3", None));
    let mut bare = host_runtime();
    if let Some(runtime) = bare.host_runtime.as_mut() {
        runtime.roster.clear();
    }
    assert!(!is_authorized_cooldown_reader(&bare, "player-1", None));
    assert!(!is_authorized_cooldown_writer(&host));
    // the global account id names a roster member too
    let account = user();
    let mut by_global = host_runtime();
    if let Some(runtime) = by_global.host_runtime.as_mut() {
        runtime.roster = vec![rifts_seam::AccountId(account.to_string())];
    }
    assert!(is_authorized_cooldown_reader(&by_global, "someone", Some(&account)));
}

#[test]
fn an_account_with_no_cooldowns_is_served_an_empty_map() {
    assert_eq!(cooldown_state_payload(None), json!({}));
}

#[test]
fn a_batch_is_stored_verbatim_and_served_back_as_an_id_to_date_map() {
    let record = merge_at(
        None,
        &body(&[
            ("GRANT_PATROL_BOOST_TOKEN", "2026-10-10T00:00:00.000Z"),
            ("GRANT_DAILY_HEROICPLUS", "2026-10-10T00:00:00.000Z"),
        ]),
    );
    assert_eq!(record.update_version, 1);
    assert_eq!(
        payload(&record),
        json!({ "GRANT_PATROL_BOOST_TOKEN": "2026-10-10T00:00:00.000Z", "GRANT_DAILY_HEROICPLUS": "2026-10-10T00:00:00.000Z" })
    );
    assert_eq!(
        cooldown_list(Some(&record))[0],
        (
            "GRANT_PATROL_BOOST_TOKEN".to_owned(),
            "2026-10-10T00:00:00.000Z".to_owned()
        )
    );
    let text = service_kit::jsjson::stringify(&payload(&record));
    assert_eq!(
        text,
        r#"{"GRANT_PATROL_BOOST_TOKEN":"2026-10-10T00:00:00.000Z","GRANT_DAILY_HEROICPLUS":"2026-10-10T00:00:00.000Z"}"#
    );
}

#[test]
fn a_later_batch_replaces_the_cooldowns_it_names_and_keeps_the_rest() {
    let mut record = merge_at(
        None,
        &body(&[
            ("season43", "2026-10-22T17:00:00.000Z"),
            ("bounty_daily", "2026-10-09T17:00:00.000Z"),
        ]),
    );
    record = merge_at(Some(&record), &body(&[("bounty_daily", "2026-10-10T17:00:00.000Z")]));
    assert_eq!(
        payload(&record),
        json!({ "season43": "2026-10-22T17:00:00.000Z", "bounty_daily": "2026-10-10T17:00:00.000Z" })
    );
    assert_eq!(record.update_version, 2);
}

#[test]
fn an_id_that_collides_with_an_object_prototype_name_is_served_as_plain_data() {
    let record = merge_at(None, &body(&[("__proto__", "2026-10-10T00:00:00Z")]));
    assert_eq!(
        service_kit::jsjson::stringify(&payload(&record)),
        r#"{"__proto__":"2026-10-10T00:00:00Z"}"#
    );
}

#[test]
fn a_bare_array_body_is_accepted_too() {
    let incoming = validate_cooldown_save(Some(
        &json!([{ "cooldown_id": "X", "cooldown_started_date": "2026-10-10T00:00:00Z" }]),
    ))
    .unwrap();
    assert_eq!(merge_cooldown_state(&user(), None, &incoming, NOW).entries.len(), 1);
}

#[test]
fn malformed_cooldown_saves_are_rejected() {
    let bad = |value: Option<Value>| validate_cooldown_save(value.as_ref()).is_err();
    assert!(bad(Some(json!({}))));
    assert!(bad(None));
    assert!(bad(Some(Value::Null)));
    assert!(bad(Some(body(&[("", "2026-10-10T00:00:00Z")]))));
    assert!(bad(Some(body(&[("X", "not a date")]))));
    let many = vec![json!({ "cooldown_id": "X", "cooldown_started_date": "2026-10-10T00:00:00Z" }); 257];
    assert!(bad(Some(json!({ "cooldowns": many }))));
    assert!(bad(Some(
        json!({ "cooldowns": [{ "cooldown_id": 5, "cooldown_started_date": "2026-10-10T00:00:00Z" }] })
    )));
    assert!(bad(Some(
        json!({ "cooldowns": [{ "cooldown_id": "X", "cooldown_started_date": 5 }] })
    )));
    assert!(bad(Some(json!({ "cooldowns": [null] }))));
}

#[test]
fn only_32_hex_gatherable_ids_older_than_48_hours_count_as_expired() {
    let old = iso(ms(NOW) - 49 * HOUR_MS);
    let recent = iso(ms(NOW) - 47 * HOUR_MS);
    assert!(is_expired_gatherable_cooldown(
        "58E91700491B00FA7DB373809E250E45",
        &old,
        NOW
    ));
    assert!(is_expired_gatherable_cooldown(
        "58e91700491b00fa7db373809e250e45",
        &old,
        NOW
    ));
    assert!(!is_expired_gatherable_cooldown(
        "58E91700491B00FA7DB373809E250E45",
        &recent,
        NOW
    ));
    for id in [
        "season43",
        "bounty_daily",
        "GRANT_PATROL_BOOST_TOKEN",
        "58E91700491B00FA7DB373809E250E4",
        "58E91700491B00FA7DB373809E250E45A",
        "Z8E91700491B00FA7DB373809E250E45",
    ] {
        assert!(
            !is_expired_gatherable_cooldown(id, "2020-01-01T00:00:00.000Z", NOW),
            "{id}"
        );
    }
    assert!(!is_expired_gatherable_cooldown(
        "58E91700491B00FA7DB373809E250E45",
        "not a date",
        NOW
    ));
}

#[test]
fn old_gatherable_ids_are_pruned_at_write_time_so_they_cannot_evict_long_lived_cooldowns() {
    let mut entries = vec![
        (
            "season43".to_owned(),
            "2026-10-22T17:00:00.000Z".to_owned(),
            Some("2026-09-02T00:00:00.000Z".to_owned()),
        ),
        (
            "bounty_daily".to_owned(),
            "2026-10-01T17:00:00.000Z".to_owned(),
            Some("2026-09-02T00:00:00.000Z".to_owned()),
        ),
    ];
    entries.extend((0..MAX_STORED_COOLDOWNS).map(|i| (gatherable(i), "2026-10-05T12:00:00.000Z".to_owned(), None)));
    let previous = owned(entries);
    let next = merge(Some(&previous), &body(&[(&gatherable(5000), NOW)]), NOW);
    assert_eq!(
        payload(&next),
        json!({ "season43": "2026-10-22T17:00:00.000Z", "bounty_daily": "2026-10-01T17:00:00.000Z", gatherable(5000): NOW })
    );
    assert_eq!(next.update_version, 8);
    assert_eq!(next.created_at, "2026-09-01T00:00:00.000Z");
}

#[test]
fn gatherable_ids_harvested_within_48_hours_are_kept() {
    let yesterday = iso(ms(NOW) - 26 * HOUR_MS);
    let almost = iso(ms(NOW) - 47 * HOUR_MS);
    let previous = stored(&[
        (&gatherable(1), &yesterday, None),
        (&gatherable(2), &almost, None),
        (&gatherable(3), &iso(ms(NOW) - 49 * HOUR_MS), None),
    ]);
    let next = merge(
        Some(&previous),
        &body(&[("bounty_daily", "2026-10-10T17:00:00.000Z")]),
        NOW,
    );
    assert_eq!(
        payload(&next),
        json!({ gatherable(1): yesterday, gatherable(2): almost, "bounty_daily": "2026-10-10T17:00:00.000Z" })
    );
}

#[test]
fn the_ids_a_save_writes_are_never_pruned_even_with_an_old_start_date() {
    let previous = stored(&[(&gatherable(1), "2026-10-01T00:00:00.000Z", None)]);
    let next = merge(
        Some(&previous),
        &body(&[
            (&gatherable(1), "2026-10-02T00:00:00.000Z"),
            (&gatherable(2), "2026-10-03T00:00:00.000Z"),
        ]),
        NOW,
    );
    assert_eq!(
        payload(&next),
        json!({ gatherable(1): "2026-10-02T00:00:00.000Z", gatherable(2): "2026-10-03T00:00:00.000Z" })
    );
}

// ---- host writes

#[test]
fn a_host_is_recognised_only_by_the_runtime_flag_without_gameserver() {
    assert!(is_host_cooldown_writer(&host_runtime()));
    assert!(!is_host_cooldown_writer(&AuthData::gameserver()));
    let mut both = host_runtime();
    both.is_gameserver = true;
    assert!(!is_host_cooldown_writer(&both));
    assert!(!is_host_cooldown_writer(&AuthData::player("player-1")));
    assert!(!is_host_cooldown_writer(&AuthData::default()));
}

#[test]
fn a_host_may_add_a_cooldown_repeat_one_and_move_one_later() {
    let previous = stored(&[
        ("bounty_daily", "2026-10-12T17:00:00.000Z", None),
        ("GRANT_DAILY_HEROICPLUS", "2026-10-12T00:00:00.000Z", None),
    ]);
    let (record, changes) = host_save(
        Some(&previous),
        &[
            ("bounty_daily", "2026-10-13T17:00:00.000Z"),
            ("GRANT_DAILY_HEROICPLUS", "2026-10-12T00:00:00.000Z"),
            ("GRANT_PATROL_BOOST_TOKEN", "2026-10-13T00:00:00.000Z"),
        ],
    );
    assert_eq!(
        payload(&record),
        json!({ "bounty_daily": "2026-10-13T17:00:00.000Z", "GRANT_DAILY_HEROICPLUS": "2026-10-12T00:00:00.000Z",
                "GRANT_PATROL_BOOST_TOKEN": "2026-10-13T00:00:00.000Z" })
    );
    assert_eq!(
        Value::Object(changes.to_json()),
        json!({
            "received": 3, "unchanged": 1,
            "added": [{ "id": "GRANT_PATROL_BOOST_TOKEN", "to": "2026-10-13T00:00:00.000Z" }], "addedCount": 1,
            "raised": [{ "id": "bounty_daily", "from": "2026-10-12T17:00:00.000Z", "to": "2026-10-13T17:00:00.000Z" }], "raisedCount": 1,
            "keptLater": [], "keptLaterCount": 0, "droppedNew": 0,
            "refusedFuture": [], "refusedFutureCount": 0,
        })
    );
    assert_eq!(record.update_version, 8);
    assert_eq!(record.created_at, "2026-09-01T00:00:00.000Z");
}

#[test]
fn a_host_cannot_move_a_cooldown_earlier() {
    let previous = stored(&[
        ("bounty_daily", "2026-10-13T17:00:00.000Z", None),
        ("GRANT_DAILY_HEROICPLUS", "2026-10-12T00:00:00.000Z", None),
    ]);
    let (record, changes) = host_save(
        Some(&previous),
        &[
            ("bounty_daily", "2026-10-01T00:00:00.000Z"),
            ("GRANT_DAILY_HEROICPLUS", "2026-10-12T23:59:59.999Z"),
            ("season43", "2026-10-22T17:00:00.000Z"),
        ],
    );
    assert_eq!(
        payload(&record),
        json!({ "bounty_daily": "2026-10-13T17:00:00.000Z", "GRANT_DAILY_HEROICPLUS": "2026-10-12T23:59:59.999Z",
                "season43": "2026-10-22T17:00:00.000Z" })
    );
    assert_eq!(
        changes.kept_later,
        [(
            "bounty_daily".to_owned(),
            "2026-10-13T17:00:00.000Z".to_owned(),
            "2026-10-01T00:00:00.000Z".to_owned()
        )]
    );
    assert_eq!(changes.kept_later_count, 1);
    let (one_ms, _) = host_save(Some(&previous), &[("bounty_daily", "2026-10-13T16:59:59.999Z")]);
    assert_eq!(payload(&one_ms)["bounty_daily"], "2026-10-13T17:00:00.000Z");
    // dates are compared as instants, whatever their notation
    let (earlier, earlier_changes) = host_save(Some(&previous), &[("bounty_daily", "2026-10-13T17:30:00+01:00")]);
    assert_eq!(payload(&earlier)["bounty_daily"], "2026-10-13T17:00:00.000Z");
    assert_eq!(earlier_changes.kept_later_count, 1);
    let (_, same_changes) = host_save(Some(&previous), &[("bounty_daily", "2026-10-13T18:00:00+01:00")]);
    assert_eq!((same_changes.unchanged, same_changes.kept_later_count), (1, 0));
}

#[test]
fn a_host_save_never_deletes_a_stored_cooldown() {
    let previous = stored(&[
        ("season43", "2026-10-22T17:00:00.000Z", None),
        ("bounty_daily", "2026-10-12T17:00:00.000Z", None),
    ]);
    assert_eq!(payload(&host_save(Some(&previous), &[]).0), payload(&previous));
    assert_eq!(
        payload(&host_save(Some(&previous), &[("other", HOST_NOW)]).0),
        json!({ "season43": "2026-10-22T17:00:00.000Z", "bounty_daily": "2026-10-12T17:00:00.000Z", "other": HOST_NOW })
    );
}

#[test]
fn duplicates_inside_one_host_save_are_judged_in_order() {
    let previous = stored(&[("bounty_daily", "2026-10-12T17:00:00.000Z", None)]);
    let (record, changes) = host_save(
        Some(&previous),
        &[
            ("bounty_daily", "2026-10-14T00:00:00.000Z"),
            ("bounty_daily", "2026-10-13T00:00:00.000Z"),
            ("fresh", "2026-10-13T10:00:00.000Z"),
            ("fresh", "2026-10-13T09:00:00.000Z"),
        ],
    );
    assert_eq!(
        payload(&record),
        json!({ "bounty_daily": "2026-10-14T00:00:00.000Z", "fresh": "2026-10-13T10:00:00.000Z" })
    );
    assert_eq!(
        (changes.kept_later_count, changes.raised_count, changes.added_count),
        (2, 1, 1)
    );
}

#[test]
fn a_host_cannot_evict_stored_cooldowns_by_flooding_new_ids() {
    let mut entries = vec![(
        "bounty_daily".to_owned(),
        "2026-10-12T17:00:00.000Z".to_owned(),
        Some("2026-09-02T00:00:00.000Z".to_owned()),
    )];
    entries.extend((0..MAX_STORED_COOLDOWNS - 1).map(|i| {
        (
            format!("named_{i}"),
            "2026-10-12T00:00:00.000Z".to_owned(),
            Some("2026-10-12T00:00:00.000Z".to_owned()),
        )
    }));
    let full = owned(entries);
    assert_eq!(full.entries.len(), MAX_STORED_COOLDOWNS);
    let flood_ids: Vec<String> = (0..50).map(|i| format!("junk_{i}")).collect();
    let flood: Vec<(&str, &str)> = flood_ids.iter().map(|id| (id.as_str(), HOST_NOW)).collect();
    let (host_record, changes) = host_save(Some(&full), &flood);
    assert_eq!(host_record.entries.len(), MAX_STORED_COOLDOWNS);
    assert_eq!(payload(&host_record)["bounty_daily"], "2026-10-12T17:00:00.000Z");
    assert_eq!((changes.dropped_new, changes.added_count), (50, 0));
    // the dedicated path is unchanged: new ids push the oldest-written entry (the gate) out
    let dedicated = merge(Some(&full), &body(&flood), HOST_NOW);
    assert!(payload(&dedicated).get("bounty_daily").is_none());

    let mut nearly = vec![("bounty_daily".to_owned(), "2026-10-12T17:00:00.000Z".to_owned(), None)];
    nearly.extend(
        (0..MAX_STORED_COOLDOWNS - 4).map(|i| (format!("named_{i}"), "2026-10-12T00:00:00.000Z".to_owned(), None)),
    );
    let (some, some_changes) = host_save(Some(&owned(nearly)), &flood[..5]);
    assert_eq!((some_changes.added_count, some_changes.dropped_new), (3, 2));
    assert_eq!(some.entries.len(), MAX_STORED_COOLDOWNS);
    assert_eq!(payload(&some)["bounty_daily"], "2026-10-12T17:00:00.000Z");
}

#[test]
fn the_change_lists_are_capped_but_the_counts_are_not() {
    let ids: Vec<String> = (0..30).map(|i| format!("gate_{i}")).collect();
    let previous = owned(
        ids.iter()
            .map(|id| (id.clone(), "2026-10-12T00:00:00.000Z".to_owned(), None))
            .collect(),
    );
    let entries: Vec<(&str, &str)> = ids.iter().map(|id| (id.as_str(), "2026-10-13T00:00:00.000Z")).collect();
    let (_, changes) = host_save(Some(&previous), &entries);
    assert_eq!(changes.raised_count, 30);
    assert_eq!(changes.raised.len(), 20);
    assert_eq!(
        changes.raised[0],
        (
            "gate_0".to_owned(),
            "2026-10-12T00:00:00.000Z".to_owned(),
            "2026-10-13T00:00:00.000Z".to_owned()
        )
    );
}

#[test]
fn a_malformed_host_save_is_rejected_like_a_dedicated_one() {
    assert!(merge_host_cooldown_state(&user(), None, Some(&json!({})), HOST_NOW).is_err());
    assert!(merge_host_cooldown_state(&user(), None, Some(&body(&[("X", "not a date")])), HOST_NOW).is_err());
}

#[test]
fn a_host_save_over_an_unparsable_stored_date_replaces_it() {
    let previous = stored(&[("odd", "garbage", None)]);
    let (record, _) = host_save(Some(&previous), &[("odd", "2026-10-13T00:00:00.000Z")]);
    assert_eq!(payload(&record)["odd"], "2026-10-13T00:00:00.000Z");
}

// ---- the future limit

const WINDOW_END: &str = "2026-10-22T17:00:00.000Z";
const NEXT_WINDOW_END: &str = "2027-01-14T17:00:00.000Z";
const HOURS_48: i64 = 48 * HOUR_MS;

fn ahead_of(offset: i64) -> String {
    iso(ms(HOST_NOW) + offset)
}

#[test]
fn season_ids_are_recognised_by_shape_and_the_limits_follow() {
    for id in ["season43", "season44", "SEASON43", "season10a"] {
        assert!(is_season_cooldown_id(id), "{id}");
    }
    for id in [
        "season",
        "seasonal",
        "season43_extra",
        "xseason43",
        "season43\n",
        "season43 ",
        "bounty_daily",
        "GRANT_SEASON43",
        "",
    ] {
        assert!(!is_season_cooldown_id(id), "{id:?}");
    }
    assert_eq!(HOST_COOLDOWN_MAX_FUTURE_MS, HOURS_48);
    let now = ms(HOST_NOW);
    assert_eq!(host_cooldown_latest_ms("bounty_daily", now), now + HOURS_48);
    assert_eq!(host_cooldown_latest_ms("GRANT_DAILY_HEROICPLUS", now), now + HOURS_48);
    assert_eq!(host_cooldown_latest_ms("season43", now), ms(WINDOW_END));
    assert_eq!(host_cooldown_latest_ms("season43", ms(WINDOW_END) - 1), ms(WINDOW_END));
    assert_eq!(host_cooldown_latest_ms("season43", ms(WINDOW_END)), ms(NEXT_WINDOW_END));
}

#[test]
fn a_date_more_than_48_hours_ahead_is_refused() {
    let previous = stored(&[("bounty_daily", "2026-10-12T17:00:00.000Z", None)]);
    let far = ahead_of(30 * 24 * HOUR_MS);
    let past = ahead_of(HOURS_48 + 1);
    let (record, changes) = host_save(
        Some(&previous),
        &[("bounty_daily", &far), ("GRANT_LOCK", &past), ("GRANT_FINE", HOST_NOW)],
    );
    assert_eq!(
        payload(&record),
        json!({ "bounty_daily": "2026-10-12T17:00:00.000Z", "GRANT_FINE": HOST_NOW })
    );
    assert_eq!(changes.refused_future_count, 2);
    assert_eq!(
        changes.refused_future,
        [
            (
                "bounty_daily".to_owned(),
                far,
                ahead_of(HOURS_48),
                Some("2026-10-12T17:00:00.000Z".to_owned())
            ),
            ("GRANT_LOCK".to_owned(), past, ahead_of(HOURS_48), None),
        ]
    );
    assert_eq!(changes.added_count, 1);
    assert_eq!(changes.kept_later_count, 0);
    assert_eq!(record.update_version, 8);
}

#[test]
fn exactly_48_hours_ahead_is_accepted_and_real_near_dates_never_refused() {
    let previous = stored(&[("bounty_daily", "2026-10-12T17:00:00.000Z", None)]);
    let edge = ahead_of(HOURS_48);
    let (record, changes) = host_save(
        Some(&previous),
        &[
            ("bounty_daily", "2026-10-14T17:00:00.000Z"),
            ("edge", &edge),
            ("GRANT_DAILY_HEROICPLUS", "2026-10-13T00:00:00.000Z"),
            ("58E91700491B00FA7DB373809E250E45", HOST_NOW),
        ],
    );
    assert_eq!(changes.refused_future_count, 0);
    assert_eq!(payload(&record)["bounty_daily"], "2026-10-14T17:00:00.000Z");
    assert_eq!(payload(&record)["edge"], edge.as_str());
    // the limit is on the instant, not the notation
    let (_, notation) = host_save(
        Some(&previous),
        &[
            ("edge", "2026-10-15T19:00:00+01:00"),
            ("over", "2026-10-15T19:00:01+01:00"),
        ],
    );
    assert_eq!(notation.refused_future_count, 1);
    assert_eq!(notation.refused_future[0].0, "over");
}

#[test]
fn a_season_id_may_reach_the_end_of_the_window_and_other_ids_stay_at_48_hours() {
    let mid = ahead_of(5 * 24 * HOUR_MS);
    let (record, changes) = host_save(
        None,
        &[
            ("season43", WINDOW_END),
            ("season44", &mid),
            ("season45", "2026-10-22T17:00:00.001Z"),
            ("season46", NEXT_WINDOW_END),
            ("bounty_daily", &mid),
        ],
    );
    assert_eq!(payload(&record), json!({ "season43": WINDOW_END, "season44": mid }));
    let refused: Vec<&str> = changes.refused_future.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(refused, ["season45", "season46", "bounty_daily"]);
    assert_eq!(changes.refused_future[0].2, WINDOW_END);
    assert_eq!(changes.refused_future[2].2, ahead_of(HOURS_48));
}

#[test]
fn the_season_limit_follows_the_roll() {
    let one = body(&[("season43", NEXT_WINDOW_END)]);
    let before = merge_host_cooldown_state(&user(), None, Some(&one), "2026-10-22T16:59:59.999Z").unwrap();
    assert_eq!(before.1.refused_future_count, 1);
    let after = merge_host_cooldown_state(&user(), None, Some(&one), WINDOW_END).unwrap();
    assert_eq!(after.1.refused_future_count, 0);
    assert_eq!(payload(&after.0)["season43"], NEXT_WINDOW_END);
    let old_end = merge_host_cooldown_state(
        &user(),
        None,
        Some(&body(&[("season43", WINDOW_END)])),
        "2026-10-23T00:00:00.000Z",
    )
    .unwrap();
    assert_eq!(old_end.1.refused_future_count, 0);
}

#[test]
fn a_far_future_date_already_stored_is_not_a_refusal_when_written_back() {
    let previous = stored(&[("locked", "2027-01-01T00:00:00.000Z", None)]);
    let (_, same) = host_save(Some(&previous), &[("locked", "2027-01-01T01:00:00+01:00")]);
    assert_eq!((same.refused_future_count, same.unchanged), (0, 1));
    let (other_record, other) = host_save(Some(&previous), &[("locked", "2027-06-01T00:00:00.000Z")]);
    assert_eq!(other.refused_future_count, 1);
    assert_eq!(payload(&other_record)["locked"], "2027-01-01T00:00:00.000Z");
    let (_, earlier) = host_save(Some(&previous), &[("locked", HOST_NOW)]);
    assert_eq!((earlier.refused_future_count, earlier.kept_later_count), (0, 1));
}

#[test]
fn a_refused_new_id_costs_no_room_in_a_full_store() {
    let mut entries = vec![("bounty_daily".to_owned(), "2026-10-12T17:00:00.000Z".to_owned(), None)];
    entries.extend(
        (0..MAX_STORED_COOLDOWNS - 1).map(|i| (format!("named_{i}"), "2026-10-12T00:00:00.000Z".to_owned(), None)),
    );
    let when = ahead_of(HOURS_48 + HOUR_MS);
    let (record, changes) = host_save(
        Some(&owned(entries)),
        &[("junk_0", &when), ("junk_1", &when), ("junk_2", &when)],
    );
    assert_eq!((changes.refused_future_count, changes.dropped_new), (3, 0));
    assert_eq!(record.entries.len(), MAX_STORED_COOLDOWNS);
}

#[test]
fn duplicates_inside_one_host_save_are_judged_in_order_for_the_future_limit_too() {
    let previous = stored(&[("bounty_daily", "2026-10-12T17:00:00.000Z", None)]);
    let far = ahead_of(10 * 24 * HOUR_MS);
    let (one, two) = (ahead_of(HOUR_MS), ahead_of(2 * HOUR_MS));
    let (record, changes) = host_save(
        Some(&previous),
        &[
            ("bounty_daily", &far),
            ("bounty_daily", &one),
            ("fresh", &two),
            ("fresh", &far),
        ],
    );
    assert_eq!(payload(&record), json!({ "bounty_daily": one, "fresh": two }));
    assert_eq!(
        (changes.refused_future_count, changes.raised_count, changes.added_count),
        (2, 1, 1)
    );
}

#[test]
fn the_refusal_list_is_capped_but_the_count_is_not() {
    let when = ahead_of(HOURS_48 + HOUR_MS);
    let ids: Vec<String> = (0..30).map(|i| format!("lock_{i}")).collect();
    let flood: Vec<(&str, &str)> = ids.iter().map(|id| (id.as_str(), when.as_str())).collect();
    let (record, changes) = host_save(None, &flood);
    assert_eq!(changes.refused_future_count, 30);
    assert_eq!(changes.refused_future.len(), 20);
    assert!(record.entries.is_empty());
}

#[test]
fn the_dedicated_merge_has_no_future_limit_and_an_unreadable_clock_falls_back_to_the_real_one() {
    let far = "2099-01-01T00:00:00.000Z";
    assert_eq!(
        payload(&merge(None, &body(&[("bounty_daily", far)]), HOST_NOW))["bounty_daily"],
        far
    );
    let real = super::util::now_ms();
    let near = iso(real + HOUR_MS);
    let farther = iso(real + 72 * HOUR_MS);
    let (record, changes) = merge_host_cooldown_state(
        &user(),
        None,
        Some(&body(&[("near", &near), ("far", &farther)])),
        "garbage",
    )
    .unwrap();
    let keys: Vec<String> = payload(&record).as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["near"]);
    assert_eq!(changes.refused_future_count, 1);
}

#[test]
fn start_ids_are_validated() {
    assert!(is_valid_cooldown_id("58E91700491B00FA7DB373809E250E45"));
    assert!(is_valid_cooldown_id("a:b.c-d_e"));
    assert!(!is_valid_cooldown_id(""));
    assert!(!is_valid_cooldown_id("a b"));
    assert!(!is_valid_cooldown_id("a/b"));
    assert!(!is_valid_cooldown_id(&"x".repeat(201)));
    assert!(is_valid_cooldown_id(&"x".repeat(200)));
}
