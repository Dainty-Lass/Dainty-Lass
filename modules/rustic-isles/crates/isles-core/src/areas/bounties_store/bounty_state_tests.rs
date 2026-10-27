//! `bountyState.test.ts`, ported: the same cases over the same helpers (`Season`, `Daily`, `Chal`, `Seed`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};

use super::authn::AuthData;
use super::bounty_state::*;
use super::util::{iso, parse_ms};
use crate::game_data::season_window::{ACTIVE_SEASON_LENGTH_MS, active_season_window_ms};
use crate::ids::AccountId;
use crate::repo::bounties::{BountyGroup, BountyStateRecord, DraftData, RewardCredit, StoredBounty};

const NOW: &str = "2026-10-10T06:00:00.000Z";
const WEEK: &str = "2026-07-30T17:00:00.000Z";

fn user() -> AccountId {
    "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@standalone".parse().unwrap()
}

fn ms(text: &str) -> i64 {
    parse_ms(text).unwrap()
}

fn season(id: &str, progress: i64, claimed: bool, drafted: &str) -> Value {
    // id.replace(/^\d+_\d+_\d+_/, "").replace(/-season.*$/, "")
    let rest = id.splitn(4, '_').nth(3).unwrap_or(id);
    let objective = rest.split("-season").next().unwrap();
    json!({
        "bounty_id": id, "premium_bounty": false, "slot_index": 0,
        "objectives": [{ "objective_id": objective, "progress": progress }],
        "drafted_timestamp": drafted, "update_version": 0, "claimed": claimed
    })
}

fn season0(id: &str) -> Value {
    season(id, 0, false, WEEK)
}

fn daily(id: &str, drafted: &str, progress: i64, claimed: bool) -> Value {
    json!({
        "bounty_id": id, "premium_bounty": false, "slot_index": 0,
        "objectives": [{ "objective_id": id, "progress": progress }],
        "drafted_timestamp": drafted, "update_version": 0, "claimed": claimed
    })
}

fn token(id: &str, slot: i64, drafted: &str, progress: i64) -> Value {
    json!({
        "bounty_id": id, "premium_bounty": true, "slot_index": slot,
        "objectives": [{ "objective_id": id, "progress": progress }],
        "drafted_timestamp": drafted, "update_version": 0, "claimed": false
    })
}

fn draft() -> Value {
    default_draft_data()
}

fn merge(previous: Option<&BountyStateRecord>, body: &Value, at: &str) -> (BountyStateRecord, BountyMergeSummary) {
    merge_bounty_state(&user(), previous, Some(body), at).unwrap()
}

fn apply(previous: Option<&BountyStateRecord>, body: &Value) -> BountyStateRecord {
    merge(previous, body, NOW).0
}

fn apply_at(previous: Option<&BountyStateRecord>, body: &Value, at: &str) -> BountyStateRecord {
    merge(previous, body, at).0
}

fn find<'a>(record: &'a BountyStateRecord, id: &str) -> Option<&'a StoredBounty> {
    record.bounties.iter().find(|e| e.bounty["bounty_id"] == id)
}

fn ids(record: &BountyStateRecord) -> Vec<String> {
    record
        .bounties
        .iter()
        .map(|e| e.bounty["bounty_id"].as_str().unwrap().to_owned())
        .collect()
}

fn group_ids(record: &BountyStateRecord, group: BountyGroup) -> Vec<String> {
    record
        .bounties
        .iter()
        .filter(|e| e.group == group)
        .map(|e| e.bounty["bounty_id"].as_str().unwrap().to_owned())
        .collect()
}

fn credit(amount: f64, transaction: &str) -> RewardCredit {
    RewardCredit {
        catalog_id: "CURRENCY_SEASONAL_COIN".into(),
        amount,
        transaction_id: transaction.into(),
        at: NOW.into(),
    }
}

fn host_auth() -> AuthData {
    let runtime = rifts_seam::RuntimeAuth {
        capability_id: "c".into(),
        session: "01ARZ3NDEKTSV4RRFFQ69G5FAV@standalone".parse().unwrap(),
        generation: 1,
        host_account: rifts_seam::AccountId("host".into()),
        scopes: vec![],
        roster: vec![
            rifts_seam::AccountId("player-1".into()),
            rifts_seam::AccountId("player-2".into()),
        ],
    };
    AuthData {
        user_id: Some("player-1".into()),
        is_gameserver: false,
        host_runtime: Some(runtime),
    }
}

#[test]
fn bounty_writes_are_dedicated_only_reads_are_server_or_same_account() {
    assert!(is_authorized_bounty_writer(&AuthData::gameserver()));
    assert!(!is_authorized_bounty_writer(&AuthData::player("player-1")));
    assert!(!is_authorized_bounty_writer(&host_auth()));
    assert!(is_authorized_bounty_reader(&AuthData::gameserver(), "player-2", None));
    assert!(is_authorized_bounty_reader(
        &AuthData::player("player-1"),
        "player-1",
        None
    ));
    assert!(!is_authorized_bounty_reader(
        &AuthData::player("player-1"),
        "player-2",
        None
    ));
}

#[test]
fn a_player_host_runtime_reads_its_hunt_roster_only_and_never_writes() {
    let host = host_auth();
    assert!(is_authorized_bounty_reader(&host, "player-2", None));
    assert!(!is_authorized_bounty_reader(&host, "player-3", None));
    assert!(!is_authorized_bounty_writer(&host));
    // a host runtime without a roster gets nothing
    let mut empty = host_auth();
    if let Some(runtime) = empty.host_runtime.as_mut() {
        runtime.roster.clear();
    }
    assert!(!is_authorized_bounty_reader(&empty, "player-1", None));
}

#[test]
fn a_host_runtime_is_a_host_writer_not_a_dedicated_one() {
    assert!(is_host_bounty_writer(&host_auth()));
    assert!(!is_host_bounty_writer(&AuthData::gameserver()));
    let mut both = host_auth();
    both.is_gameserver = true;
    assert!(!is_host_bounty_writer(&both));
    assert!(!is_host_bounty_writer(&AuthData::player("player-1")));
    assert!(!is_host_bounty_writer(&AuthData::default()));
}

#[test]
fn an_account_with_no_stored_state_is_served_the_old_stubs_empty_shape() {
    assert_eq!(
        bounty_state_payload(None),
        json!({
            "season_start_date": "2020-08-23T00:00:00.000Z",
            "season_end_date": "2099-01-01T00:00:00.000Z",
            "bounties": [],
            "draft_data": draft(),
            "draft_data_daily": draft(),
            "draft_data_weekly": draft(),
        })
    );
    // the wire key order, which a client's JSON reader can't tell but a parity replay can
    let keys: Vec<_> = bounty_state_payload(None)
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        [
            "season_start_date",
            "season_end_date",
            "bounties",
            "draft_data",
            "draft_data_daily",
            "draft_data_weekly"
        ]
    );
}

#[test]
fn bounties_are_grouped_by_id() {
    assert_eq!(
        infer_bounty_group("Challenge_Daily_Bronze_KillBlazeWeapon"),
        BountyGroup::Daily
    );
    assert_eq!(
        infer_bounty_group("43_3_1_Challenge_Season_Spend_Notes-season43-2"),
        BountyGroup::Weekly
    );
    assert_eq!(infer_bounty_group("Bounty_Bronze_KillWithFriends"), BountyGroup::Base);
}

#[test]
fn the_weekly_seed_is_stored_and_served_back_with_its_draft_data() {
    let seed = vec![
        season0("43_1_1_Challenge_Season_BountiesAny_15-season43-0"),
        season0("43_1_2_Challenge_Season_LanternDamage-season43-0"),
    ];
    let record = apply(None, &json!({ "bounties": seed, "draft_data_weekly": draft() }));
    assert_eq!(record.update_version, 1);
    assert_eq!(
        bounty_group_counts(Some(&record)),
        json!({ "base": 0, "daily": 0, "weekly": 2, "unassigned": 0, "claimed": 0, "credited": 0 })
    );
    let payload = bounty_state_payload(Some(&record));
    assert_eq!(payload["bounties"], json!(seed));
    assert_eq!(payload["draft_data_weekly"], draft());
    assert_eq!(payload["draft_data_daily"], draft());
}

#[test]
fn a_reseed_at_progress_0_cannot_erase_progress_or_unclaim_a_claim() {
    let id = "43_3_7_Challenge_Season_Break_Heads-season43-2";
    let mut record = apply(
        None,
        &json!({ "bounties": [season0(id)], "draft_data_weekly": draft() }),
    );
    record = apply(Some(&record), &json!({ "bounties": [season(id, 7, false, WEEK)] }));
    record = apply(Some(&record), &json!({ "bounties": [season(id, 10, true, WEEK)] }));
    record = apply(
        Some(&record),
        &json!({ "bounties": [season0(id)], "draft_data_weekly": draft() }),
    );
    let stored = &record.bounties[0].bounty;
    assert_eq!(stored["objectives"][0]["progress"], 10);
    assert_eq!(stored["claimed"], true);
    assert_eq!(record.update_version, 4);
}

#[test]
fn an_empty_list_with_draft_data_updates_the_draft_but_deletes_nothing() {
    let list: Vec<Value> = [
        "43_1_1_Challenge_Season_BountiesAny_15-season43-0",
        "43_1_2_Challenge_Season_LanternDamage-season43-0",
    ]
    .iter()
    .map(|id| season(id, 3, false, WEEK))
    .collect();
    let mut record = apply(None, &json!({ "bounties": list, "draft_data_weekly": draft() }));
    let mut next = draft();
    next["gold_count"] = json!(4);
    record = apply(Some(&record), &json!({ "bounties": [], "draft_data_weekly": next }));
    assert_eq!(record.bounties.len(), 2);
    assert_eq!(record.draft_data.weekly, Some(next));
}

#[test]
fn a_daily_draft_replaces_the_previous_daily_and_leaves_season_challenges_alone() {
    let season_id = "43_2_1_Challenge_Season_GatherablesAny-season43-1";
    let mut record = apply(
        None,
        &json!({ "bounties": [season(season_id, 5, false, WEEK)], "draft_data_weekly": draft() }),
    );
    let mut first_draft = draft();
    first_draft["previous_draft_selections"] = json!(["Challenge_Daily_Bronze_WoundDamageReduce"]);
    record = apply(
        Some(&record),
        &json!({
            "bounties": [daily("Challenge_Daily_Bronze_WoundDamageReduce", "2026-10-09T05:27:52.688Z", 1, true)],
            "draft_data_daily": first_draft
        }),
    );
    let mut second_draft = draft();
    second_draft["previous_draft_selections"] = json!([
        "Challenge_Daily_Bronze_WoundDamageReduce",
        "Challenge_Daily_Bronze_KillBlazeWeapon"
    ]);
    record = apply(
        Some(&record),
        &json!({
            "bounties": [daily("Challenge_Daily_Bronze_KillBlazeWeapon", "2026-10-10T05:28:08.000Z", 0, false)],
            "draft_data_daily": second_draft
        }),
    );
    let mut got = ids(&record);
    got.sort();
    assert_eq!(
        got,
        [
            "43_2_1_Challenge_Season_GatherablesAny-season43-1",
            "Challenge_Daily_Bronze_KillBlazeWeapon"
        ]
    );
    let weekly = record.bounties.iter().find(|e| e.group == BountyGroup::Weekly).unwrap();
    assert_eq!(weekly.bounty["objectives"][0]["progress"], 5);
    assert_eq!(
        record.draft_data.daily.as_ref().unwrap()["previous_draft_selections"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn drafting_a_second_bounty_token_keeps_the_first_active() {
    let season_id = "43_2_1_Challenge_Season_GatherablesAny-season43-1";
    let mut record = apply(
        None,
        &json!({ "bounties": [season(season_id, 5, false, WEEK)], "draft_data_weekly": draft() }),
    );
    let mut base = draft();
    base["current_draft_choices"] = json!(["a", "b", "c"]);
    base["bronze_count"] = json!(2);
    record = apply(Some(&record), &json!({ "bounties": [], "draft_data": base }));
    let mut after_first = draft();
    after_first["previous_draft_selections"] = json!(["Bounty_Bronze_PhysicalDodges"]);
    after_first["bronze_count"] = json!(1);
    record = apply(
        Some(&record),
        &json!({
            "bounties": [token("Bounty_Bronze_PhysicalDodges", 0, "2026-10-19T03:00:00.000Z", 0)],
            "draft_data": after_first
        }),
    );
    record = apply(
        Some(&record),
        &json!({ "bounties": [token("Bounty_Bronze_PhysicalDodges", 0, "2026-10-19T03:00:00.000Z", 4)] }),
    );
    let mut after_second = draft();
    after_second["previous_draft_selections"] =
        json!(["Bounty_Bronze_PhysicalDodges", "Bounty_Silver_StaggerDamageReduce"]);
    let (second, summary) = merge(
        Some(&record),
        &json!({
            "bounties": [token("Bounty_Silver_StaggerDamageReduce", 1, "2026-10-19T03:01:00.000Z", 0)],
            "draft_data": after_second
        }),
        NOW,
    );
    assert_eq!(summary.removed, 0);
    assert_eq!(summary.mode, "upsert");
    record = second;
    let mut base_ids = group_ids(&record, BountyGroup::Base);
    base_ids.sort();
    assert_eq!(
        base_ids,
        ["Bounty_Bronze_PhysicalDodges", "Bounty_Silver_StaggerDamageReduce"]
    );
    assert_eq!(
        find(&record, "Bounty_Bronze_PhysicalDodges").unwrap().bounty["objectives"][0]["progress"],
        4
    );
    assert_eq!(group_ids(&record, BountyGroup::Weekly).len(), 1);
    // what the hunt server loads after the map change carries both tokens
    let served = bounty_state_payload(Some(&record))["bounties"].clone();
    assert!(
        served
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["bounty_id"] == "Bounty_Silver_StaggerDamageReduce")
    );
    // claiming one removes only that one
    let (deleted, _) = delete_bounties(
        &user(),
        Some(&record),
        Some(&json!({ "bounty_ids": ["Bounty_Bronze_PhysicalDodges"] })),
        NOW,
    )
    .unwrap();
    assert_eq!(
        group_ids(&deleted, BountyGroup::Base),
        ["Bounty_Silver_StaggerDamageReduce"]
    );
}

#[test]
fn a_new_drafted_timestamp_is_a_new_instance() {
    let id = "Challenge_Daily_Bronze_KillBlazeWeapon";
    let mut record = apply(
        None,
        &json!({ "bounties": [daily(id, "2026-10-09T05:00:00.000Z", 1, true)], "draft_data_daily": draft() }),
    );
    record = apply(
        Some(&record),
        &json!({ "bounties": [daily(id, "2026-10-09T05:00:00.000Z", 0, false)] }),
    );
    assert_eq!(
        record.bounties[0].bounty["claimed"], true,
        "same instance stays claimed"
    );
    record = apply(
        Some(&record),
        &json!({ "bounties": [daily(id, "2026-10-10T05:00:00.000Z", 0, false)] }),
    );
    assert_eq!(record.bounties.len(), 1);
    assert_eq!(
        record.bounties[0].bounty["claimed"], false,
        "a later draft starts unclaimed"
    );
    assert_eq!(
        record.bounties[0].bounty["drafted_timestamp"],
        "2026-10-10T05:00:00.000Z"
    );
    assert_eq!(record.bounties[0].group, BountyGroup::Daily);
}

#[test]
fn an_objective_the_incoming_save_omits_is_kept() {
    let two = json!({ "bounty_id": "Bounty_Bronze_Two", "drafted_timestamp": WEEK, "claimed": false,
        "objectives": [{ "objective_id": "A", "progress": 2 }, { "objective_id": "B", "progress": 4 }] });
    let mut record = apply(None, &json!({ "bounties": [two] }));
    let mut again = two.clone();
    again["objectives"] = json!([{ "objective_id": "A", "progress": 3 }]);
    record = apply(Some(&record), &json!({ "bounties": [again] }));
    assert_eq!(
        record.bounties[0].bounty["objectives"],
        json!([{ "objective_id": "A", "progress": 3 }, { "objective_id": "B", "progress": 4 }])
    );
}

#[test]
fn malformed_saves_are_rejected_before_anything_is_stored() {
    let bad = |body: Option<Value>| merge_bounty_state(&user(), None, body.as_ref(), NOW).is_err();
    assert!(bad(Some(Value::Null)));
    assert!(bad(None));
    assert!(bad(Some(json!([]))));
    assert!(bad(Some(json!({ "bounties": {} }))));
    assert!(bad(Some(json!({ "bounties": [{ "objectives": [] }] }))));
    assert!(bad(Some(
        json!({ "bounties": [{ "bounty_id": "X", "objectives": {} }] })
    )));
    assert!(bad(Some(json!({ "bounties": [], "draft_data_daily": [] }))));
    assert!(bad(Some(json!({ "bounties": vec![json!({ "bounty_id": "X" }); 513] }))));
    assert!(bad(Some(json!({ "bounties": [{ "bounty_id": "" }] }))));
    assert!(bad(Some(json!({ "bounties": [{ "bounty_id": "x".repeat(201) }] }))));
    assert!(bad(Some(json!({ "bounties": ["x"] }))));
    // a null bounties key is "no bounties"
    assert!(!bad(Some(json!({ "bounties": null }))));
}

#[test]
fn the_stored_list_is_capped_oldest_claimed_first() {
    let many: Vec<Value> = (0..MAX_STORED_BOUNTIES)
        .map(|i| {
            daily(
                &format!("Bounty_Old_{i}"),
                &format!("2025-01-01T00:00:{:02}.000Z", i % 60),
                1,
                i < 5,
            )
        })
        .collect();
    let mut record: Option<BountyStateRecord> = None;
    for chunk in many.chunks(500) {
        record = Some(apply(record.as_ref(), &json!({ "bounties": chunk })));
    }
    let record = apply(
        record.as_ref(),
        &json!({ "bounties": [daily("Bounty_New", NOW, 0, false)] }),
    );
    assert_eq!(record.bounties.len(), MAX_STORED_BOUNTIES);
    assert!(find(&record, "Bounty_New").is_some());
    assert_eq!(
        bounty_group_counts(Some(&record))["claimed"],
        4,
        "one claimed bounty was dropped to make room"
    );
}

// ---- season-challenge rewards

const AXE: &str = "43_5_5_Challenge_Season_BigHit_Axe-season43-4";
const PARTS: &str = "43_11_2_Challenge_Season_Part_Damage_1Fight-season43-10";

fn seeded() -> BountyStateRecord {
    apply_at(
        None,
        &json!({ "bounties": [season0(AXE), season0(PARTS)], "draft_data_weekly": draft() }),
        "2026-10-10T08:00:00.000Z",
    )
}

#[test]
fn a_claim_is_stamped_when_first_seen_and_keeps_that_time() {
    let mut record = apply_at(
        Some(&seeded()),
        &json!({ "bounties": [season(AXE, 1, true, WEEK)] }),
        "2026-10-10T08:56:04.304Z",
    );
    record = apply_at(
        Some(&record),
        &json!({ "bounties": [season(AXE, 1, true, WEEK)] }),
        "2026-10-10T09:10:00.000Z",
    );
    assert_eq!(
        find(&record, AXE).unwrap().claimed_at.as_deref(),
        Some("2026-10-10T08:56:04.304Z")
    );
    assert_eq!(find(&record, PARTS).unwrap().claimed_at, None);
}

#[test]
fn each_claimed_challenge_funds_exactly_one_reward_newest_first() {
    let now_ms = ms(NOW);
    let mut record = apply_at(
        Some(&seeded()),
        &json!({ "bounties": [season(AXE, 1, true, WEEK)] }),
        "2026-10-10T08:56:04.304Z",
    );
    record = apply_at(
        Some(&record),
        &json!({ "bounties": [season(PARTS, 1, true, WEEK)] }),
        "2026-10-10T08:56:04.378Z",
    );
    assert_eq!(
        find_fundable_season_claim(Some(&record), now_ms).unwrap().bounty["bounty_id"],
        PARTS
    );
    record = with_reward_credit(&record, PARTS, credit(400.0, "T1"), NOW).unwrap();
    assert_eq!(
        find_fundable_season_claim(Some(&record), now_ms).unwrap().bounty["bounty_id"],
        AXE
    );
    record = with_reward_credit(&record, AXE, credit(400.0, "T2"), NOW).unwrap();
    assert!(find_fundable_season_claim(Some(&record), now_ms).is_none());
    assert!(with_reward_credit(&record, AXE, credit(400.0, "T3"), NOW).is_err());
    assert!(with_reward_credit(&record, "nope", credit(400.0, "T3"), NOW).is_err());
    assert_eq!(bounty_group_counts(Some(&record))["credited"], 2);
}

#[test]
fn a_reseed_keeps_a_challenge_claimed_and_credited_so_it_cannot_pay_twice() {
    let mut record = apply_at(
        Some(&seeded()),
        &json!({ "bounties": [season(AXE, 1, true, WEEK)] }),
        "2026-10-10T08:56:04.304Z",
    );
    record = with_reward_credit(&record, AXE, credit(400.0, "T1"), NOW).unwrap();
    record = apply_at(
        Some(&record),
        &json!({ "bounties": [season0(AXE), season0(PARTS)], "draft_data_weekly": draft() }),
        "2026-10-10T09:30:00.000Z",
    );
    let axe = find(&record, AXE).unwrap();
    assert_eq!(axe.bounty["claimed"], true);
    assert_eq!(axe.reward_credit.as_ref().unwrap().transaction_id, "T1");
    assert!(find_fundable_season_claim(Some(&record), ms(NOW)).is_none());
    let served = bounty_state_payload(Some(&record));
    let served_axe = served["bounties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["bounty_id"] == AXE)
        .unwrap();
    assert_eq!(
        served_axe["claimed"], true,
        "the game is served the challenge as claimed"
    );
}

#[test]
fn a_new_drafted_instance_of_a_challenge_starts_unclaimed_and_uncredited() {
    let mut record = apply_at(
        Some(&seeded()),
        &json!({ "bounties": [season(AXE, 1, true, WEEK)] }),
        "2026-10-10T08:56:04.304Z",
    );
    record = with_reward_credit(&record, AXE, credit(400.0, "T1"), NOW).unwrap();
    record = apply_at(
        Some(&record),
        &json!({ "bounties": [season(AXE, 0, false, "2026-10-17T17:00:00.000Z")] }),
        "2026-10-17T18:00:00.000Z",
    );
    let axe = find(&record, AXE).unwrap();
    assert!(axe.reward_credit.is_none());
    assert!(axe.claimed_at.is_none());
}

#[test]
fn a_claimed_daily_funds_one_reward_per_game_day_from_17_utc() {
    let day_one = ms(NOW);
    let frost = "Challenge_Daily_Bronze_KillBlazeFrost";
    let mut record = apply_at(
        None,
        &json!({ "bounties": [daily(frost, NOW, 1, true)], "draft_data_daily": draft() }),
        NOW,
    );
    assert_eq!(
        find_fundable_season_claim(Some(&record), day_one).unwrap().bounty["bounty_id"],
        frost
    );
    record = with_reward_credit(&record, frost, credit(100.0, "T1"), NOW).unwrap();
    assert_eq!(
        record.daily_reward_credited_on.as_deref(),
        Some("2026-10-09T17:00:00.000Z"),
        "a morning instant belongs to the game day that began the day before"
    );
    // a re-drafted daily claimed the same game day is not paid again
    record = apply_at(
        Some(&record),
        &json!({ "bounties": [daily("Challenge_Daily_Bronze_KillBlazeWeapon", NOW, 1, true)] }),
        "2026-10-10T07:00:00.000Z",
    );
    assert_eq!(
        record.daily_reward_credited_on.as_deref(),
        Some("2026-10-09T17:00:00.000Z"),
        "a save keeps the day"
    );
    let last_second = ms("2026-10-10T16:59:59.000Z");
    assert!(find_fundable_season_claim(Some(&record), last_second).is_none());
    assert!(daily_challenge_reward_limited(Some(&record), last_second));
    // at 17:00 UTC the next game day pays the next claimed daily
    let next_day = ms("2026-10-10T17:00:01.000Z");
    assert_eq!(
        find_fundable_season_claim(Some(&record), next_day).unwrap().bounty["bounty_id"],
        "Challenge_Daily_Bronze_KillBlazeWeapon"
    );
    assert!(!daily_challenge_reward_limited(Some(&record), next_day));
}

#[test]
fn the_game_day_runs_17_to_17_utc_and_the_old_utc_date_form_never_matches() {
    assert_eq!(
        game_daily_window_key(ms("2026-10-16T16:59:59.999Z")),
        "2026-10-15T17:00:00.000Z"
    );
    assert_eq!(
        game_daily_window_key(ms("2026-10-16T17:00:00.000Z")),
        "2026-10-16T17:00:00.000Z"
    );
    assert_eq!(
        game_daily_window_key(ms("2026-10-16T18:30:00.000Z")),
        "2026-10-16T17:00:00.000Z"
    );
    let old = BountyStateRecord {
        account: user(),
        bounties: vec![],
        draft_data: DraftData::default(),
        daily_reward_credited_on: Some("2026-10-16".into()),
        daily_premium_draft_granted_on: None,
        update_version: 1,
        created_at: NOW.into(),
        updated_at: NOW.into(),
    };
    assert!(!daily_challenge_reward_paid(Some(&old), ms("2026-10-16T12:00:00.000Z")));
    assert_eq!(utc_day(ms(NOW)), "2026-10-10");
}

#[test]
fn an_older_record_keeps_the_hosts_premium_token_day_through_saves() {
    let old = BountyStateRecord {
        account: user(),
        bounties: vec![],
        draft_data: DraftData::default(),
        daily_reward_credited_on: None,
        daily_premium_draft_granted_on: Some("2026-10-10".into()),
        update_version: 1,
        created_at: NOW.into(),
        updated_at: NOW.into(),
    };
    assert_eq!(bounty_state_payload(Some(&old)), bounty_state_payload(None));
    let record = apply_at(
        Some(&old),
        &json!({ "bounties": [daily("Challenge_Daily_Bronze_KillBlazeWeapon", NOW, 1, true)] }),
        "2026-10-10T07:00:00.000Z",
    );
    assert_eq!(record.daily_premium_draft_granted_on.as_deref(), Some("2026-10-10"));
    assert_eq!(record.bounties.len(), 1);
    assert_eq!(
        record.daily_reward_credited_on, None,
        "the daily reward day is a different key"
    );
}

// ---- host writes

#[test]
fn the_claim_stamp_tells_which_ids_a_save_turned_into_claimed() {
    let (t0, t1, t2) = (
        "2026-10-13T08:00:00.000Z",
        "2026-10-13T08:05:00.000Z",
        "2026-10-13T08:10:00.000Z",
    );
    let first = apply_at(
        None,
        &json!({ "bounties": [season0(AXE), season0(PARTS)], "draft_data_weekly": draft() }),
        t0,
    );
    assert!(newly_claimed_bounty_ids(&first).is_empty());
    let progress = apply_at(Some(&first), &json!({ "bounties": [season(AXE, 3, false, WEEK)] }), t1);
    assert!(newly_claimed_bounty_ids(&progress).is_empty());
    let claim = apply_at(
        Some(&progress),
        &json!({ "bounties": [season(AXE, 3, true, WEEK)] }),
        t2,
    );
    assert_eq!(newly_claimed_bounty_ids(&claim), [AXE]);
    let again = apply_at(
        Some(&claim),
        &json!({ "bounties": [season(AXE, 3, true, WEEK), season(PARTS, 1, true, WEEK)] }),
        "2026-10-13T08:15:00.000Z",
    );
    assert_eq!(newly_claimed_bounty_ids(&again), [PARTS]);
    let quiet = apply_at(
        Some(&again),
        &json!({ "bounties": [season(AXE, 3, true, WEEK)] }),
        "2026-10-13T08:20:00.000Z",
    );
    assert!(newly_claimed_bounty_ids(&quiet).is_empty());
}

#[test]
fn a_claim_stored_before_claimed_at_existed_is_never_reported() {
    let seeded = apply_at(
        None,
        &json!({ "bounties": [season(AXE, 1, true, WEEK)] }),
        "2026-10-09T00:00:00.000Z",
    );
    let mut old = seeded.clone();
    for entry in &mut old.bounties {
        entry.claimed_at = None;
    }
    let next = apply_at(
        Some(&old),
        &json!({ "bounties": [season(AXE, 1, true, WEEK), season(PARTS, 2, true, WEEK)], "draft_data_weekly": draft() }),
        "2026-10-13T09:00:00.000Z",
    );
    assert_eq!(newly_claimed_bounty_ids(&next), [PARTS]);
}

#[test]
fn a_redrafted_challenge_claimed_again_is_reported_again() {
    let mut record = apply_at(
        Some(&seeded()),
        &json!({ "bounties": [season(AXE, 1, true, WEEK)] }),
        "2026-10-13T08:00:00.000Z",
    );
    record = with_reward_credit(&record, AXE, credit(400.0, "T1"), NOW).unwrap();
    let redraft = apply_at(
        Some(&record),
        &json!({ "bounties": [season(AXE, 1, true, "2026-10-13T08:30:00.000Z")] }),
        "2026-10-13T08:31:00.000Z",
    );
    assert_eq!(newly_claimed_bounty_ids(&redraft), [AXE]);
}

#[test]
fn the_host_write_summary_names_mode_group_counts_claims_and_version() {
    let (record, summary) = merge(
        None,
        &json!({ "bounties": [season(AXE, 1, true, WEEK), season0(PARTS)], "draft_data_weekly": draft() }),
        "2026-10-13T08:00:00.000Z",
    );
    assert_eq!(
        host_bounty_write_summary(&summary, &record),
        json!({ "mode": "replace", "group": "weekly", "incoming": 2, "removed": 0, "newlyClaimedCount": 1,
                "newlyClaimed": [AXE], "version": 1 })
    );
    let (record2, summary2) = merge(
        Some(&record),
        &json!({ "bounties": [season(PARTS, 4, false, WEEK)] }),
        "2026-10-13T08:01:00.000Z",
    );
    assert_eq!(
        host_bounty_write_summary(&summary2, &record2),
        json!({ "mode": "upsert", "incoming": 1, "removed": 0, "newlyClaimedCount": 0, "newlyClaimed": [], "version": 2 })
    );
}

#[test]
fn the_host_write_summary_caps_the_claimed_id_list_but_not_the_count() {
    let list: Vec<String> = (0..40)
        .map(|i| format!("43_{i}_1_Challenge_Season_Test_{i}-season43-0"))
        .collect();
    let bounties: Vec<Value> = list.iter().map(|id| season(id, 1, true, WEEK)).collect();
    let (record, summary) = merge(
        None,
        &json!({ "bounties": bounties, "draft_data_weekly": draft() }),
        "2026-10-13T08:00:00.000Z",
    );
    let out = host_bounty_write_summary(&summary, &record);
    assert_eq!(out["newlyClaimedCount"], 40);
    assert_eq!(out["newlyClaimed"].as_array().unwrap().len(), 24);
    assert_eq!(out["newlyClaimed"], json!(list[..24]));
}

// ---- host limits

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
const H_NOW: &str = "2026-10-13T18:00:00.000Z";

fn h_now_ms() -> i64 {
    ms(H_NOW)
}

fn window_start() -> i64 {
    active_season_window_ms(h_now_ms()).0
}

fn roll_ms() -> i64 {
    active_season_window_ms(h_now_ms()).1
}

fn schedule_stamp(week: i64, start: i64) -> String {
    iso(weekly_schedule_stamp_ms(start, week))
}

fn stamp(week: i64) -> String {
    schedule_stamp(week, window_start())
}

fn roll_plus(minutes: i64) -> String {
    iso(roll_ms() + minutes * 60_000)
}

fn weekly_id(week: i64, slot: i64) -> String {
    weekly_named(week, slot, &format!("Test{slot}"))
}

fn weekly_named(week: i64, slot: i64, name: &str) -> String {
    format!("43_{week}_{slot}_Challenge_Season_{name}-season43-{}", week - 1)
}

#[derive(Clone)]
struct Chal {
    week: i64,
    slot: i64,
    progress: i64,
    claimed: bool,
    stamp: Option<String>,
    name: Option<&'static str>,
}

fn chal(week: i64, slot: i64) -> Chal {
    Chal {
        week,
        slot,
        progress: 0,
        claimed: false,
        stamp: None,
        name: None,
    }
}

impl Chal {
    fn progress(mut self, progress: i64) -> Self {
        self.progress = progress;
        self
    }

    fn claimed(mut self) -> Self {
        self.claimed = true;
        self
    }

    fn at(mut self, stamp: &str) -> Self {
        self.stamp = Some(stamp.to_owned());
        self
    }

    fn named(mut self, name: &'static str) -> Self {
        self.name = Some(name);
        self
    }

    fn json(&self) -> Value {
        let id = match self.name {
            Some(name) => weekly_named(self.week, self.slot, name),
            None => weekly_id(self.week, self.slot),
        };
        json!({
            "bounty_id": id, "premium_bounty": false, "slot_index": 0,
            "objectives": [{ "objective_id": format!("Obj{}", self.slot), "progress": self.progress }],
            "drafted_timestamp": self.stamp.clone().unwrap_or_else(|| stamp(self.week)),
            "update_version": 0, "claimed": self.claimed
        })
    }
}

/// All 110 instances of the window that starts at `start`, unclaimed, on schedule.
fn seed_list(start: i64) -> Vec<Value> {
    let mut list = Vec::new();
    for week in 1..=11 {
        for slot in 1..=10 {
            list.push(chal(week, slot).at(&schedule_stamp(week, start)).json());
        }
    }
    list
}

fn host(previous: Option<&BountyStateRecord>, body: &Value, at: &str) -> (BountyStateRecord, BountyMergeSummary) {
    merge_host_bounty_state(&user(), previous, Some(body), at).unwrap()
}

fn seeded_host() -> BountyStateRecord {
    host(
        None,
        &json!({ "bounties": seed_list(window_start()), "draft_data_weekly": draft() }),
        H_NOW,
    )
    .0
}

fn adjustments(summary: &BountyMergeSummary) -> &HostBountyAdjustments {
    summary.host.as_ref().unwrap()
}

#[test]
fn the_schedule_hangs_on_the_served_pass_window() {
    assert_eq!(iso(window_start()), "2026-07-30T17:00:00.000Z");
    assert_eq!(iso(roll_ms()), "2026-10-22T17:00:00.000Z");
    assert_eq!(stamp(1), "2026-07-30T17:00:00.000Z");
    assert_eq!(stamp(5), "2026-08-27T17:00:00.000Z");
    assert_eq!(stamp(11), "2026-10-08T17:00:00.000Z");
}

#[test]
fn every_real_weekly_id_parses_and_week_slot_and_index_must_agree() {
    for id in [
        "43_1_1_Challenge_Season_BountiesAny_15-season43-0",
        "43_2_10_Challenge_Season_Gather_Marks_Steel-season43-1",
        "43_5_5_Challenge_Season_BigHit_Axe-season43-4",
        "43_6_10_Challenge_Season_Rock_Alpha_Breaks_Armour-season43-5",
        "43_8_10_Challenge_Season_Crowbear_Alpha_Damage_Stagger_Penalty-season43-7",
        "43_10_5_Challenge_Season_Damage_ChainBlades_SingleFight_Penalty-season43-9",
        "43_11_6_Challenge_Season_KillBehemoth_Blaze_FrostWepx2-season43-10",
        "43_11_10_Challenge_Season_Trials_Break_All-season43-10",
    ] {
        assert!(parse_weekly_bounty_id(id).is_some(), "{id}");
    }
    assert_eq!(
        parse_weekly_bounty_id("43_11_10_Challenge_Season_Trials_Break_All-season43-10"),
        Some(WeeklyBountyId {
            season: 43,
            week: 11,
            slot: 10
        })
    );
    assert_eq!(
        parse_weekly_bounty_id(&weekly_id(1, 1)),
        Some(WeeklyBountyId {
            season: 43,
            week: 1,
            slot: 1
        })
    );
}

#[test]
fn ids_that_cannot_be_a_real_weekly_instance_do_not_parse() {
    for id in [
        "Challenge_Season_Fake",
        "Challenge_Season_X-season43-4",
        "26_11_7_Challenge_Season_BreakParts_Firesacs_Aether_Sally_Terra",
        "43_5_5_Challenge_Season_BigHit_Axe-season43-5",
        "43_5_5_Challenge_Season_BigHit_Axe-season43-3",
        "43_12_1_Challenge_Season_X-season43-11",
        "43_0_1_Challenge_Season_X-season43-0",
        "43_5_11_Challenge_Season_X-season43-4",
        "43_5_0_Challenge_Season_X-season43-4",
        "043_05_5_Challenge_Season_X-season43-4",
        "43_5_5_Challenge_Season_X-season43-04",
        "43_5_5_Challenge_Season_Big-Hit-season43-4",
        "43_5_5_Challenge_Season_-season43-4",
        "43_5_5_Challenge_Season_X-season43-4 ",
        "43_5_5_Challenge_Season_X-season43-4\n",
        "x43_5_5_Challenge_Season_X-season43-4",
        "43_5_5_challenge_season_X-season43-4",
        "43_5_5_Challenge_Season_X-season-4",
        "43_5_5_Challenge_Season_\u{dc}n\u{ef}-season43-4",
    ] {
        assert_eq!(parse_weekly_bounty_id(id), None, "{id:?}");
    }
}

#[test]
fn accepted_stamps_are_the_current_schedule_and_the_previous_for_two_hours_after_a_roll() {
    assert_eq!(accepted_weekly_stamps_ms(1, h_now_ms()), [window_start()]);
    assert_eq!(
        accepted_weekly_stamps_ms(11, h_now_ms()),
        [window_start() + 70 * DAY_MS]
    );
    assert_eq!(
        accepted_weekly_stamps_ms(3, roll_ms() - 1),
        [window_start() + 14 * DAY_MS]
    );
    let both = [roll_ms() + 14 * DAY_MS, window_start() + 14 * DAY_MS];
    assert_eq!(accepted_weekly_stamps_ms(3, roll_ms()), both);
    assert_eq!(
        accepted_weekly_stamps_ms(3, roll_ms() + WEEKLY_PREVIOUS_WINDOW_GRACE_MS - 1),
        both
    );
    assert_eq!(
        accepted_weekly_stamps_ms(3, roll_ms() + WEEKLY_PREVIOUS_WINDOW_GRACE_MS),
        [roll_ms() + 14 * DAY_MS]
    );
    assert_eq!(WEEKLY_PREVIOUS_WINDOW_GRACE_MS, 2 * HOUR_MS);
}

#[test]
fn only_a_claimed_weekly_on_an_accepted_stamp_is_protected_from_being_dropped() {
    let entry = |bounty: Value, group| StoredBounty {
        group,
        bounty,
        claimed_at: None,
        reward_credit: None,
    };
    let weekly = |c: Chal| entry(c.json(), BountyGroup::Weekly);
    assert!(is_protected_claimed_weekly(&weekly(chal(5, 5).claimed()), h_now_ms()));
    assert!(
        !is_protected_claimed_weekly(&weekly(chal(5, 5)), h_now_ms()),
        "an unclaimed one may be dropped"
    );
    let old = schedule_stamp(5, window_start() - ACTIVE_SEASON_LENGTH_MS);
    assert!(!is_protected_claimed_weekly(
        &weekly(chal(5, 5).claimed().at(&old)),
        h_now_ms()
    ));
    assert!(is_protected_claimed_weekly(
        &weekly(chal(5, 5).claimed()),
        roll_ms() + HOUR_MS
    ));
    assert!(!is_protected_claimed_weekly(
        &weekly(chal(5, 5).claimed()),
        roll_ms() + 3 * HOUR_MS
    ));
    let mut legacy = chal(5, 5).claimed().json();
    legacy["bounty_id"] = json!("Challenge_Season_Legacy");
    assert!(!is_protected_claimed_weekly(
        &entry(legacy, BountyGroup::Weekly),
        h_now_ms()
    ));
    assert!(!is_protected_claimed_weekly(
        &entry(daily("Challenge_Daily_Bronze_X", H_NOW, 1, true), BountyGroup::Daily),
        h_now_ms()
    ));
}

#[test]
fn host_progress_is_clamped_at_100_million() {
    assert_eq!(MAX_HOST_OBJECTIVE_PROGRESS, 100_000_000.0);
    let real = chal(2, 1).progress(500_752).json();
    assert_eq!(clamp_host_objective_progress(&real), (real.clone(), 0));
    assert_eq!(
        clamp_host_objective_progress(&chal(2, 1).progress(100_000_000).json()).1,
        0
    );
    let over = chal(2, 1).progress(100_000_001).json();
    let (clamped, count) = clamp_host_objective_progress(&over);
    assert_eq!(count, 1);
    assert_eq!(clamped["objectives"][0]["progress"], 100_000_000);
    assert_eq!(
        over["objectives"][0]["progress"], 100_000_001,
        "the input is not modified"
    );
    // the merge reads progress as Number(progress), so a numeric string is clamped the same way
    let mixed = json!({ "bounty_id": "B", "objectives": [
        { "objective_id": "A", "progress": "5e9" }, { "objective_id": "C", "progress": 7 },
        { "objective_id": "D", "progress": -1_000_000_000_000_i64 }, { "objective_id": "E" }, "junk" ] });
    let (out, count) = clamp_host_objective_progress(&mixed);
    assert_eq!(count, 1);
    assert_eq!(
        out["objectives"],
        json!([{ "objective_id": "A", "progress": 100_000_000 }, { "objective_id": "C", "progress": 7 },
                { "objective_id": "D", "progress": -1_000_000_000_000_i64 }, { "objective_id": "E" }, "junk"])
    );
    assert_eq!(clamp_host_objective_progress(&json!({ "bounty_id": "B" })).1, 0);
    assert_eq!(
        clamp_host_objective_progress(&json!({ "bounty_id": "B", "objectives": {} })).1,
        0
    );
}

#[test]
fn a_hosts_weekly_seed_on_schedule_is_stored_like_a_dedicated_servers() {
    let body = json!({ "bounties": seed_list(window_start()), "draft_data_weekly": draft() });
    let (host_record, host_summary) = host(None, &body, H_NOW);
    let (dedicated, dedicated_summary) = merge(None, &body, H_NOW);
    assert_eq!(host_record.bounties.len(), 110);
    assert_eq!(host_record, dedicated);
    assert_eq!(host_summary.incoming, 110);
    assert_eq!(adjustments(&host_summary), &HostBountyAdjustments::default());
    assert!(
        dedicated_summary.host.is_none(),
        "a dedicated save carries no host adjustments"
    );
    let (claim, claim_summary) = host(
        Some(&host_record),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        "2026-10-13T18:05:00.000Z",
    );
    assert_eq!(newly_claimed_bounty_ids(&claim), [weekly_id(5, 5)]);
    assert_eq!(
        find_fundable_season_claim(Some(&claim), ms(H_NOW)).unwrap().bounty["bounty_id"],
        weekly_id(5, 5).as_str()
    );
    assert_eq!(adjustments(&claim_summary).kept_claimed_weekly, 0);
}

#[test]
fn a_host_cannot_restamp_a_claimed_weekly_instance() {
    let id = weekly_id(5, 5);
    let mut record = host(
        Some(&seeded_host()),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        "2026-10-13T18:05:00.000Z",
    )
    .0;
    record = with_reward_credit(&record, &id, credit(400.0, "T1"), NOW).unwrap();
    let stamp5 = ms(&stamp(5));
    let attempts: Vec<Value> = vec![
        json!(stamp(4)),
        json!(stamp(6)),
        json!(schedule_stamp(5, window_start() - ACTIVE_SEASON_LENGTH_MS)),
        json!(schedule_stamp(5, roll_ms())),
        json!(H_NOW),
        json!(iso(stamp5 + 1)),
        json!(iso(stamp5 - 1)),
        json!(iso(stamp5 + DAY_MS)),
        json!("2099-01-01T00:00:00.000Z"),
        json!("not a date"),
        json!(""),
        Value::Null,
        json!(1_760_000_000_000_i64),
    ];
    for attempt in attempts.iter().map(Some).chain(std::iter::once(None)) {
        let mut entry = chal(5, 5).json();
        match attempt {
            Some(value) => entry["drafted_timestamp"] = value.clone(),
            None => {
                entry.as_object_mut().unwrap().remove("drafted_timestamp");
            }
        }
        let (result, summary) = host(
            Some(&record),
            &json!({ "bounties": [entry] }),
            "2026-10-13T18:10:00.000Z",
        );
        let label = format!("{attempt:?}");
        let stored = find(&result, &id).unwrap();
        assert_eq!(stored.bounty["claimed"], true, "{label}");
        assert_eq!(stored.bounty["drafted_timestamp"], stamp(5).as_str(), "{label}");
        assert_eq!(stored.reward_credit.as_ref().unwrap().transaction_id, "T1", "{label}");
        assert_eq!(
            stored.claimed_at.as_deref(),
            Some("2026-10-13T18:05:00.000Z"),
            "{label}"
        );
        assert_eq!(adjustments(&summary).kept_claimed_weekly, 1, "{label}");
        assert_eq!(
            adjustments(&summary).kept_claimed_weekly_ids,
            std::slice::from_ref(&id),
            "{label}"
        );
        assert_eq!(adjustments(&summary).ignored_weekly, 0, "{label}");
        assert!(
            find_fundable_season_claim(Some(&result), ms(H_NOW)).is_none(),
            "{label}"
        );
        assert_eq!(result.bounties.len(), 110, "{label}");
        assert_eq!(
            result.update_version,
            record.update_version + 1,
            "the save still applied"
        );
    }
    // the dedicated merge is unchanged: a different stamp is a new instance, claimable again
    let dedicated = apply_at(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).at(&schedule_stamp(5, roll_ms())).json()] }),
        "2026-10-13T18:10:00.000Z",
    );
    assert_eq!(find(&dedicated, &id).unwrap().bounty["claimed"], false);
    assert!(find(&dedicated, &id).unwrap().reward_credit.is_none());
}

#[test]
fn the_rest_of_a_save_applies_when_one_entry_is_refused() {
    let id = weekly_id(5, 5);
    let record = host(
        Some(&seeded_host()),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        "2026-10-13T18:05:00.000Z",
    )
    .0;
    let (result, summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).at(H_NOW).json(), chal(5, 6).progress(9).json(), daily("Challenge_Daily_Bronze_X", H_NOW, 2, false)] }),
        "2026-10-13T18:10:00.000Z",
    );
    assert_eq!(adjustments(&summary).kept_claimed_weekly, 1);
    assert_eq!(find(&result, &id).unwrap().bounty["claimed"], true);
    assert_eq!(
        find(&result, &weekly_id(5, 6)).unwrap().bounty["objectives"][0]["progress"],
        9
    );
    assert_eq!(
        find(&result, "Challenge_Daily_Bronze_X").unwrap().group,
        BountyGroup::Daily
    );
}

#[test]
fn an_unclaimed_instance_cannot_be_restamped_but_counts_as_ignored() {
    let id = weekly_id(5, 5);
    let record = host(
        Some(&seeded_host()),
        &json!({ "bounties": [chal(5, 5).progress(2).json()] }),
        "2026-10-13T18:05:00.000Z",
    )
    .0;
    let (result, summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).at(&schedule_stamp(5, roll_ms())).claimed().progress(50).json()] }),
        "2026-10-13T18:10:00.000Z",
    );
    assert_eq!(adjustments(&summary).ignored_weekly, 1);
    assert_eq!(adjustments(&summary).kept_claimed_weekly, 0);
    assert_eq!(adjustments(&summary).ignored_weekly_ids, std::slice::from_ref(&id));
    let stored = find(&result, &id).unwrap();
    assert_eq!(stored.bounty["claimed"], false);
    assert_eq!(stored.bounty["objectives"][0]["progress"], 2);
    assert_eq!(stored.bounty["drafted_timestamp"], stamp(5).as_str());
}

#[test]
fn a_claimed_instance_cannot_be_dropped_by_omission_and_re_added_unclaimed() {
    let id = weekly_id(5, 5);
    let mut record = host(
        Some(&seeded_host()),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        "2026-10-13T18:05:00.000Z",
    )
    .0;
    record = with_reward_credit(&record, &id, credit(400.0, "T1"), NOW).unwrap();
    let omit_body = json!({ "bounties": [chal(5, 6).progress(2).json()], "draft_data_weekly": draft() });
    let (omit, summary) = host(Some(&record), &omit_body, "2026-10-13T18:10:00.000Z");
    assert_eq!(summary.mode, "replace");
    assert_eq!(summary.removed, 108);
    assert_eq!(omit.bounties.len(), 2);
    assert_eq!(find(&omit, &id).unwrap().bounty["claimed"], true);
    assert_eq!(
        find(&omit, &id).unwrap().reward_credit.as_ref().unwrap().transaction_id,
        "T1"
    );
    assert_eq!(adjustments(&summary).kept_claimed_weekly, 1);
    assert_eq!(adjustments(&summary).kept_claimed_weekly_ids, std::slice::from_ref(&id));
    // a later save adds it back unclaimed: still the claimed, credited instance
    let (back, back_summary) = host(
        Some(&omit),
        &json!({ "bounties": [chal(5, 5).json()] }),
        "2026-10-13T18:15:00.000Z",
    );
    assert_eq!(find(&back, &id).unwrap().bounty["claimed"], true);
    assert!(find_fundable_season_claim(Some(&back), ms(H_NOW)).is_none());
    assert_eq!(adjustments(&back_summary).kept_claimed_weekly, 0);
    // the dedicated merge drops it, it comes back unclaimed and its next claim funds a grant again
    let mut dedicated = apply_at(Some(&record), &omit_body, "2026-10-13T18:10:00.000Z");
    assert!(find(&dedicated, &id).is_none());
    dedicated = apply_at(
        Some(&dedicated),
        &json!({ "bounties": [chal(5, 5).json()] }),
        "2026-10-13T18:11:00.000Z",
    );
    dedicated = apply_at(
        Some(&dedicated),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        "2026-10-13T18:12:00.000Z",
    );
    assert_eq!(
        find_fundable_season_claim(Some(&dedicated), ms(H_NOW)).unwrap().bounty["bounty_id"],
        id.as_str()
    );
}

#[test]
fn an_ignored_entry_a_replace_save_names_stays_as_stored_and_incoming_counts_what_was_sent() {
    let stored = seeded_host();
    let (result, summary) = host(
        Some(&stored),
        &json!({ "bounties": [chal(5, 5).at(H_NOW).claimed().json(), chal(5, 6).progress(4).json()], "draft_data_weekly": draft() }),
        H_NOW,
    );
    assert_eq!(summary.incoming, 2);
    assert_eq!(summary.removed, 108);
    assert_eq!(adjustments(&summary).ignored_weekly, 1);
    let kept = find(&result, &weekly_id(5, 5)).unwrap();
    assert_eq!(kept.bounty["drafted_timestamp"], stamp(5).as_str());
    assert_eq!(kept.bounty["claimed"], false);
    assert_eq!(
        find(&result, &weekly_id(5, 6)).unwrap().bounty["objectives"][0]["progress"],
        4
    );
    assert_eq!(result.bounties.len(), 2);
}

#[test]
fn entries_are_judged_in_order_against_what_earlier_entries_left() {
    let first = chal(5, 5).progress(1).claimed().json();
    let sent = vec![
        first.clone(),
        chal(5, 5).at(H_NOW).json(),
        chal(5, 5).at(&schedule_stamp(5, roll_ms())).json(),
        chal(5, 5).progress(2).json(),
    ];
    let (planned, adjustments) = plan_host_bounty_save(None, &sent, h_now_ms());
    assert_eq!(planned.len(), 2, "the first entry and the repeat on the same stamp");
    assert_eq!(
        adjustments.kept_claimed_weekly, 2,
        "two re-stamps of an instance this very save claimed"
    );
    assert_eq!(adjustments.ignored_weekly, 0);
    assert_eq!(planned[0], first);
}

#[test]
fn the_same_instant_written_another_way_is_the_same_instance() {
    let id = weekly_id(11, 1);
    let record = host(
        None,
        &json!({ "bounties": [chal(11, 1).progress(3).claimed().json()] }),
        "2026-10-13T18:05:00.000Z",
    )
    .0;
    assert_eq!(
        find(&record, &id).unwrap().bounty["drafted_timestamp"],
        "2026-10-08T17:00:00.000Z"
    );
    for notation in [
        "2026-10-08T17:00:00+00:00",
        "2026-10-08T18:00:00+01:00",
        "2026-10-08T17:00:00Z",
        "2026-10-08T17:00:00.000000Z",
    ] {
        let (result, summary) = host(
            Some(&record),
            &json!({ "bounties": [chal(11, 1).at(notation).json()] }),
            "2026-10-13T18:10:00.000Z",
        );
        let entry = find(&result, &id).unwrap();
        assert_eq!(
            entry.bounty["drafted_timestamp"], "2026-10-08T17:00:00.000Z",
            "{notation}"
        );
        assert_eq!(entry.bounty["claimed"], true, "{notation}");
        assert_eq!(
            entry.claimed_at.as_deref(),
            Some("2026-10-13T18:05:00.000Z"),
            "{notation}"
        );
        assert_eq!(adjustments(&summary).kept_claimed_weekly, 0, "{notation}");
        assert_eq!(adjustments(&summary).ignored_weekly, 0, "{notation}");
    }
}

#[test]
fn the_window_rolls_once_and_a_stale_servers_stamp_cannot_bring_the_claim_back() {
    let id = weekly_id(5, 5);
    let new_stamp = schedule_stamp(5, roll_ms());
    let mut record = apply_at(
        None,
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()], "draft_data_weekly": draft() }),
        "2026-08-28T00:00:00.000Z",
    );
    record = with_reward_credit(&record, &id, credit(400.0, "T1"), NOW).unwrap();
    let (stale, stale_summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        &roll_plus(30),
    );
    assert_eq!(adjustments(&stale_summary), &HostBountyAdjustments::default());
    assert_eq!(
        find(&stale, &id)
            .unwrap()
            .reward_credit
            .as_ref()
            .unwrap()
            .transaction_id,
        "T1"
    );

    let (fresh, fresh_summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).at(&new_stamp).json()] }),
        &roll_plus(31),
    );
    let entry = find(&fresh, &id).unwrap();
    assert_eq!(entry.bounty["drafted_timestamp"], new_stamp.as_str());
    assert_eq!(entry.bounty["claimed"], false);
    assert!(entry.reward_credit.is_none() && entry.claimed_at.is_none());
    assert_eq!(adjustments(&fresh_summary), &HostBountyAdjustments::default());

    let (replay, replay_summary) = host(
        Some(&fresh),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        &roll_plus(32),
    );
    assert_eq!(
        find(&replay, &id).unwrap().bounty["drafted_timestamp"],
        new_stamp.as_str()
    );
    assert_eq!(find(&replay, &id).unwrap().bounty["claimed"], false);
    assert_eq!(adjustments(&replay_summary).ignored_weekly, 1);
    assert!(find_fundable_season_claim(Some(&replay), ms(&roll_plus(32))).is_none());

    let (claim, _) = host(
        Some(&fresh),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().at(&new_stamp).json()] }),
        &roll_plus(40),
    );
    assert_eq!(newly_claimed_bounty_ids(&claim), std::slice::from_ref(&id));
    assert_eq!(
        find_fundable_season_claim(Some(&claim), ms(&roll_plus(40)))
            .unwrap()
            .bounty["bounty_id"],
        id.as_str()
    );
    let (again, again_summary) = host(
        Some(&claim),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        &roll_plus(41),
    );
    assert_eq!(adjustments(&again_summary).kept_claimed_weekly, 1);
    assert_eq!(
        find(&again, &id).unwrap().bounty["drafted_timestamp"],
        new_stamp.as_str()
    );
    assert_eq!(find(&again, &id).unwrap().bounty["claimed"], true);
}

#[test]
fn the_previous_windows_schedule_is_tolerated_for_two_hours_and_not_a_minute_longer() {
    let id = weekly_id(5, 5);
    let record = apply_at(
        None,
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        "2026-08-28T00:00:00.000Z",
    );
    let old = json!({ "bounties": [chal(5, 5).progress(2).claimed().json()] });
    let (inside, inside_summary) = host(Some(&record), &old, &roll_plus(119));
    assert_eq!(adjustments(&inside_summary).kept_claimed_weekly, 0);
    assert_eq!(find(&inside, &id).unwrap().bounty["objectives"][0]["progress"], 2);
    let (outside, outside_summary) = host(Some(&record), &old, &roll_plus(120));
    assert_eq!(adjustments(&outside_summary).kept_claimed_weekly, 1);
    assert_eq!(find(&outside, &id).unwrap().bounty["objectives"][0]["progress"], 1);
    let new_stamp = schedule_stamp(5, roll_ms());
    let (early, early_summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).at(&new_stamp).json()] }),
        &iso(roll_ms() - 1),
    );
    assert_eq!(adjustments(&early_summary).kept_claimed_weekly, 1);
    assert_eq!(
        find(&early, &id).unwrap().bounty["drafted_timestamp"],
        stamp(5).as_str()
    );
    let (on_time, on_time_summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).at(&new_stamp).json()] }),
        &roll_plus(0),
    );
    assert_eq!(adjustments(&on_time_summary).kept_claimed_weekly, 0);
    assert_eq!(
        find(&on_time, &id).unwrap().bounty["drafted_timestamp"],
        new_stamp.as_str()
    );
}

#[test]
fn invented_weekly_ids_are_not_stored() {
    let valid = chal(5, 5).json();
    let with = |key: &str, value: Value| {
        let mut entry = valid.clone();
        entry[key] = value;
        entry
    };
    let mut bad: Vec<Value> = vec![
        with("bounty_id", json!("Challenge_Season_Fake")),
        with("bounty_id", json!("43_5_5_Challenge_Season_BigHit_Axe")),
        with("bounty_id", json!("43_5_5_Challenge_Season_BigHit_Axe-season43-5")),
        {
            let mut e = with("bounty_id", json!("43_12_1_Challenge_Season_X-season43-11"));
            e["drafted_timestamp"] = json!(stamp(12));
            e
        },
        with("bounty_id", json!("43_5_11_Challenge_Season_X-season43-4")),
        with("bounty_id", json!("43_5_5_Challenge_Season_Big-Hit-season43-4")),
        with("bounty_id", json!("xx43_5_6_Challenge_Season_X-season43-4")),
    ];
    let mut now_stamped = chal(5, 6).json();
    now_stamped["drafted_timestamp"] = json!(H_NOW);
    bad.push(now_stamped);
    bad.push(chal(5, 7).at(&stamp(4)).json());
    let mut missing = chal(5, 8).json();
    missing.as_object_mut().unwrap().remove("drafted_timestamp");
    bad.push(missing);
    bad.push(chal(5, 9).at("2099-01-01T00:00:00.000Z").json());
    for entry in &bad {
        let mut claimed = entry.clone();
        claimed["claimed"] = json!(true);
        let (result, summary) = host(None, &json!({ "bounties": [claimed] }), H_NOW);
        assert_eq!(result.bounties.len(), 0, "{}", entry["bounty_id"]);
        assert_eq!(adjustments(&summary).ignored_weekly, 1, "{}", entry["bounty_id"]);
        assert!(find_fundable_season_claim(Some(&result), ms(H_NOW)).is_none());
    }
    let mut all: Vec<Value> = bad
        .iter()
        .map(|e| {
            let mut c = e.clone();
            c["claimed"] = json!(true);
            c
        })
        .collect();
    all.push(valid);
    let (many, summary) = host(None, &json!({ "bounties": all }), H_NOW);
    assert_eq!(ids(&many), [weekly_id(5, 5)]);
    assert_eq!(adjustments(&summary).ignored_weekly, bad.len());
    assert_eq!(
        adjustments(&summary).ignored_weekly_ids.len(),
        10,
        "the sample is capped"
    );
    // the dedicated merge stores whatever the server sends
    let mut fake = bad[0].clone();
    fake["claimed"] = json!(true);
    let dedicated = apply_at(None, &json!({ "bounties": [fake] }), H_NOW);
    assert_eq!(
        find_fundable_season_claim(Some(&dedicated), ms(H_NOW)).unwrap().bounty["bounty_id"],
        "Challenge_Season_Fake"
    );
}

#[test]
fn a_bounty_is_filed_by_its_id_for_a_host() {
    let daily1 = daily("Challenge_Daily_Bronze_Fake", H_NOW, 1, true);
    let base1 = json!({ "bounty_id": "Bounty_Bronze_Fake", "premium_bounty": false, "slot_index": 0, "objectives": [],
        "drafted_timestamp": H_NOW, "update_version": 0, "claimed": true });
    let body = json!({ "bounties": [daily1, base1], "draft_data_weekly": draft() });
    let (host_record, _) = host(None, &body, H_NOW);
    assert_eq!(
        host_record.bounties.iter().map(|e| e.group).collect::<Vec<_>>(),
        [BountyGroup::Daily, BountyGroup::Base]
    );
    let found = find_fundable_season_claim(Some(&host_record), ms(H_NOW)).unwrap();
    assert_eq!(found.group, BountyGroup::Daily);
    assert_eq!(found.bounty["bounty_id"], "Challenge_Daily_Bronze_Fake");
    let mut paid = host_record.clone();
    paid.daily_reward_credited_on = Some(game_daily_window_key(ms(H_NOW)));
    assert!(find_fundable_season_claim(Some(&paid), ms(H_NOW)).is_none());
    let dedicated = apply_at(None, &body, H_NOW);
    assert_eq!(
        dedicated.bounties.iter().map(|e| e.group).collect::<Vec<_>>(),
        [BountyGroup::Weekly, BountyGroup::Weekly]
    );
}

#[test]
fn a_window_holds_110_weekly_instances() {
    let full = seeded_host();
    assert_eq!(full.bounties.len(), WEEKLY_INSTANCES_PER_WINDOW);
    let invented = chal(1, 1).named("Invented");
    let mut claimed = invented.clone().claimed();
    claimed.progress = 0;
    let (refused, summary) = host(Some(&full), &json!({ "bounties": [claimed.json()] }), H_NOW);
    assert_eq!(refused.bounties.len(), 110);
    assert_eq!(adjustments(&summary).ignored_weekly, 1);
    assert!(find(&refused, invented.json()["bounty_id"].as_str().unwrap()).is_none());
    let (update, update_summary) = host(
        Some(&full),
        &json!({ "bounties": [chal(1, 1).progress(5).json()] }),
        H_NOW,
    );
    assert_eq!(adjustments(&update_summary).ignored_weekly, 0);
    assert_eq!(
        find(&update, &weekly_id(1, 1)).unwrap().bounty["objectives"][0]["progress"],
        5
    );
    let (nearly, _) = host(None, &json!({ "bounties": seed_list(window_start())[..109] }), H_NOW);
    let (added, added_summary) = host(Some(&nearly), &json!({ "bounties": [invented.json()] }), H_NOW);
    assert_eq!(added.bounties.len(), 110);
    assert_eq!(adjustments(&added_summary).ignored_weekly, 0);
    let (extra, extra_summary) = host(
        Some(&added),
        &json!({ "bounties": [chal(2, 1).named("Invented2").json()] }),
        H_NOW,
    );
    assert_eq!(extra.bounties.len(), 110);
    assert_eq!(adjustments(&extra_summary).ignored_weekly, 1);
}

#[test]
fn the_cap_is_per_window() {
    let old = seeded_host();
    let (invented, summary) = host(
        Some(&old),
        &json!({ "bounties": [chal(1, 1).named("Invented").json()] }),
        &roll_plus(30),
    );
    assert_eq!(adjustments(&summary).ignored_weekly, 1);
    assert_eq!(invented.bounties.len(), 110);
    let (rolled, rolled_summary) = host(
        Some(&old),
        &json!({ "bounties": seed_list(roll_ms()), "draft_data_weekly": draft() }),
        &roll_plus(31),
    );
    assert_eq!(rolled.bounties.len(), 110);
    assert_eq!(adjustments(&rolled_summary).ignored_weekly, 0);
    assert_eq!(adjustments(&rolled_summary).kept_claimed_weekly, 0);
    assert!(
        rolled
            .bounties
            .iter()
            .all(|e| ms(e.bounty["drafted_timestamp"].as_str().unwrap()) >= roll_ms())
    );
    let extra = chal(1, 1).named("Invented").at(&schedule_stamp(1, roll_ms()));
    let (full, extra_summary) = host(Some(&rolled), &json!({ "bounties": [extra.json()] }), &roll_plus(32));
    assert_eq!(adjustments(&extra_summary).ignored_weekly, 1);
    assert_eq!(full.bounties.len(), 110);
}

#[test]
fn a_host_flooding_new_bounties_cannot_evict_claimed_weekly_instances() {
    let protected = chal(5, 5).progress(1).claimed().json();
    let mut entries = vec![StoredBounty {
        group: BountyGroup::Weekly,
        bounty: protected,
        claimed_at: None,
        reward_credit: None,
    }];
    for index in 0..MAX_STORED_BOUNTIES - 1 {
        entries.push(StoredBounty {
            group: BountyGroup::Daily,
            bounty: daily(
                &format!("Challenge_Daily_Old_{index}"),
                "2026-10-01T00:00:00.000Z",
                1,
                true,
            ),
            claimed_at: None,
            reward_credit: None,
        });
    }
    let stored = BountyStateRecord {
        account: user(),
        bounties: entries,
        draft_data: DraftData::default(),
        daily_reward_credited_on: None,
        daily_premium_draft_granted_on: None,
        update_version: 9,
        created_at: H_NOW.into(),
        updated_at: H_NOW.into(),
    };
    let flood: Vec<Value> = (0..5)
        .map(|i| daily(&format!("Challenge_Daily_New_{i}"), H_NOW, 0, false))
        .collect();
    let (host_record, _) = host(Some(&stored), &json!({ "bounties": flood }), H_NOW);
    assert_eq!(host_record.bounties.len(), MAX_STORED_BOUNTIES);
    assert!(
        find(&host_record, &weekly_id(5, 5)).is_some(),
        "the claimed weekly instance stays"
    );
    assert!(
        flood
            .iter()
            .all(|f| find(&host_record, f["bounty_id"].as_str().unwrap()).is_some())
    );
    let dedicated = apply_at(Some(&stored), &json!({ "bounties": flood }), H_NOW);
    assert_eq!(dedicated.bounties.len(), MAX_STORED_BOUNTIES);
    assert!(
        find(&dedicated, &weekly_id(5, 5)).is_none(),
        "the dedicated cap order is unchanged"
    );
}

#[test]
fn a_hosts_progress_above_100_million_is_clamped_and_counted_in_every_group() {
    let string_progress = json!({ "bounty_id": "Bounty_Bronze_Str", "drafted_timestamp": H_NOW, "claimed": false,
        "objectives": [{ "objective_id": "A", "progress": "5e9" }, { "objective_id": "B", "progress": 7 }] });
    let body = json!({ "bounties": [
        chal(2, 1).progress(500_752).json(), chal(2, 2).progress(100_000_000).json(),
        chal(2, 3).progress(100_000_001).json(), chal(2, 4).progress(1_000_000_000_000).json(),
        daily("Challenge_Daily_Bronze_Big", H_NOW, 9_000_000_000_000_000, false), string_progress ] });
    let (host_record, summary) = host(None, &body, H_NOW);
    let progress = |id: &str| -> Vec<Value> {
        find(&host_record, id).unwrap().bounty["objectives"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["progress"].clone())
            .collect()
    };
    assert_eq!(progress(&weekly_id(2, 1)), [json!(500_752)]);
    assert_eq!(progress(&weekly_id(2, 2)), [json!(100_000_000)]);
    assert_eq!(progress(&weekly_id(2, 3)), [json!(100_000_000)]);
    assert_eq!(progress(&weekly_id(2, 4)), [json!(100_000_000)]);
    assert_eq!(progress("Challenge_Daily_Bronze_Big"), [json!(100_000_000)]);
    assert_eq!(progress("Bounty_Bronze_Str"), [json!(100_000_000), json!(7)]);
    assert_eq!(adjustments(&summary).clamped_progress, 4);
    assert_eq!(host_record.bounties.len(), 6, "clamped entries are kept, not refused");
    let (again, again_summary) = host(Some(&host_record), &body, H_NOW);
    assert_eq!(
        find(&again, &weekly_id(2, 4)).unwrap().bounty["objectives"][0]["progress"],
        100_000_000
    );
    assert_eq!(adjustments(&again_summary).clamped_progress, 4);
    let dedicated = apply_at(None, &body, H_NOW);
    assert_eq!(
        find(&dedicated, &weekly_id(2, 4)).unwrap().bounty["objectives"][0]["progress"],
        1_000_000_000_000_i64
    );
}

#[test]
fn daily_and_base_bounties_merge_for_a_host_exactly_as_for_a_dedicated_server() {
    let mut selections = draft();
    selections["previous_draft_selections"] = json!(["Challenge_Daily_Bronze_KillBlazeWeapon"]);
    let first = json!({ "bounties": [daily("Challenge_Daily_Bronze_KillBlazeWeapon", "2026-10-13T05:28:08.000Z", 1, true)], "draft_data_daily": selections });
    let second = json!({ "bounties": [daily("Challenge_Daily_Bronze_WoundDamageReduce", "2026-10-14T05:28:08.000Z", 0, false)], "draft_data_daily": selections });
    let third = json!({ "bounties": [daily("Challenge_Daily_Bronze_WoundDamageReduce", "2099-01-01T00:00:00.000Z", 3, false)] });
    let (mut h, mut d): (Option<BountyStateRecord>, Option<BountyStateRecord>) = (None, None);
    for body in [first, second, third] {
        h = Some(host(h.as_ref(), &body, H_NOW).0);
        d = Some(apply_at(d.as_ref(), &body, H_NOW));
        assert_eq!(h, d);
    }
}

#[test]
fn the_host_write_tag_carries_counts_and_capped_samples() {
    let id = weekly_id(5, 5);
    let record = host(
        Some(&seeded_host()),
        &json!({ "bounties": [chal(5, 5).progress(1).claimed().json()] }),
        "2026-10-13T18:05:00.000Z",
    )
    .0;
    let mut fake = chal(5, 6).json();
    fake["bounty_id"] = json!("Challenge_Season_Fake");
    let (result, summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 5).at(H_NOW).json(), fake, chal(5, 7).progress(1_000_000_000).json()] }),
        "2026-10-13T18:10:00.000Z",
    );
    assert_eq!(
        host_bounty_write_summary(&summary, &result),
        json!({ "mode": "upsert", "incoming": 3, "removed": 0, "newlyClaimedCount": 0, "newlyClaimed": [],
                "keptClaimedWeekly": 1, "ignoredWeekly": 1, "clampedProgress": 1,
                "keptClaimedWeeklyIds": [id], "ignoredWeeklyIds": ["Challenge_Season_Fake"],
                "version": record.update_version + 1 })
    );
    let (quiet, quiet_summary) = host(
        Some(&record),
        &json!({ "bounties": [chal(5, 6).progress(2).json()] }),
        "2026-10-13T18:11:00.000Z",
    );
    assert_eq!(
        host_bounty_write_summary(&quiet_summary, &quiet),
        json!({ "mode": "upsert", "incoming": 1, "removed": 0, "newlyClaimedCount": 0, "newlyClaimed": [],
                "keptClaimedWeekly": 0, "ignoredWeekly": 0, "clampedProgress": 0, "version": record.update_version + 1 })
    );
}

#[test]
fn a_malformed_host_save_is_rejected_like_a_dedicated_one() {
    let bad = |body: Option<Value>| merge_host_bounty_state(&user(), None, body.as_ref(), H_NOW).is_err();
    assert!(bad(Some(Value::Null)));
    assert!(bad(Some(json!({ "bounties": {} }))));
    assert!(bad(Some(json!({ "bounties": [{ "objectives": [] }] }))));
    assert!(bad(Some(json!({ "bounties": [], "draft_data_daily": [] }))));
    assert!(bad(Some(json!({ "bounties": vec![json!({ "bounty_id": "X" }); 513] }))));
}

// ---- POST /bounty/delete/:userId

#[test]
fn a_claimed_bounty_is_deleted_by_id_and_season_challenges_never() {
    let base = json!({ "bounty_id": "Bounty_Bronze_PhysicalDodges", "premium_bounty": false, "slot_index": 0,
        "objectives": [{ "objective_id": "Bounty_Bronze_PhysicalDodges", "progress": 10 }], "drafted_timestamp": NOW,
        "update_version": 7, "claimed": false });
    let mut record = apply(None, &json!({ "bounties": [base], "draft_data": draft() }));
    record = apply(
        Some(&record),
        &json!({ "bounties": [season("43_2_5_Challenge_Season_Staggers-season43-1", 5, true, WEEK)], "draft_data_weekly": draft() }),
    );
    record = apply(
        Some(&record),
        &json!({ "bounties": [daily("Challenge_Daily_Bronze_KillASChain", NOW, 3, true)], "draft_data_daily": draft() }),
    );
    let (deleted, summary) = delete_bounties(
        &user(),
        Some(&record),
        Some(&json!({ "bounty_ids": ["Bounty_Bronze_PhysicalDodges", "43_2_5_Challenge_Season_Staggers-season43-1", "Unknown"] })),
        NOW,
    )
    .unwrap();
    assert_eq!(summary.deleted, ["Bounty_Bronze_PhysicalDodges"]);
    assert_eq!(summary.kept, ["43_2_5_Challenge_Season_Staggers-season43-1", "Unknown"]);
    assert_eq!(
        ids(&deleted),
        [
            "43_2_5_Challenge_Season_Staggers-season43-1",
            "Challenge_Daily_Bronze_KillASChain"
        ]
    );
    assert_eq!(deleted.update_version, record.update_version + 1);
    assert_eq!(deleted.draft_data, record.draft_data);
    assert_eq!(
        bounty_state_payload(Some(&deleted))["bounties"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (flag, flag_summary) = delete_bounties(
        &user(),
        Some(&record),
        Some(&json!({ "delete_claimed_bounties": true })),
        NOW,
    )
    .unwrap();
    assert_eq!(flag_summary.deleted, ["Challenge_Daily_Bronze_KillASChain"]);
    assert_eq!(flag.bounties.len(), 2);
}

#[test]
fn a_delete_that_names_no_bounty_is_the_games_reset() {
    for body in [None, Some(Value::Null), Some(json!({})), Some(json!({ "other": 1 }))] {
        assert!(bounty_delete_names_nothing(body.as_ref()), "{body:?}");
    }
    for body in [
        json!([]),
        json!({ "bounty_ids": [] }),
        json!({ "bounty_ids": "A" }),
        json!({ "bounties": ["A"] }),
        json!({ "delete_claimed_bounties": false }),
    ] {
        assert!(!bounty_delete_names_nothing(Some(&body)), "{body}");
    }
}

#[test]
fn the_delete_body_accepts_ids_or_bounty_objects_and_must_name_something() {
    let parse = |body: Value| parse_bounty_delete(Some(&body));
    assert_eq!(
        parse(json!({ "bounty_ids": ["A", "A"] })).unwrap(),
        BountyDeleteRequest {
            bounty_ids: vec!["A".into()],
            delete_claimed: false
        }
    );
    assert_eq!(
        parse(json!({ "bounties": [{ "bounty_id": "B" }, "C"] })).unwrap(),
        BountyDeleteRequest {
            bounty_ids: vec!["B".into(), "C".into()],
            delete_claimed: false
        }
    );
    assert_eq!(
        parse(json!({ "delete_claimed_bounties": true })).unwrap(),
        BountyDeleteRequest {
            bounty_ids: vec![],
            delete_claimed: true
        }
    );
    assert!(parse_bounty_delete(None).is_err());
    for body in [
        json!([]),
        json!({}),
        json!({ "bounty_ids": [] }),
        json!({ "bounty_ids": "A" }),
        json!({ "bounty_ids": [1] }),
        json!({ "bounty_ids": [""] }),
    ] {
        assert!(parse(body.clone()).is_err(), "{body}");
    }
}
