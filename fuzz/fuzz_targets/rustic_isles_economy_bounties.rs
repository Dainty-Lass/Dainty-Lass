//! What Rustic Isles' bounty state reads from game servers and hosts (spec-rustic-isles-economy-bounties §13): an
//! arbitrary body and stored list through validation, the dedicated merge, the host plan and merge (any clock) and the
//! delete. Nothing panics; a save never stores more than 1000 entries, never unclaims a claimed entry, always moves the
//! version forward; a host merge keeps every protected entry and stores no weekly id that does not parse; the payload
//! always has its six keys.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_economy::bounties::state::{
    bounty_id, bounty_state_payload, delete_bounties, infer_group, is_claimed, is_protected, merge_bounty_state,
    merge_host_bounty_state, parse_delete, parse_weekly_id, validate_bounty_save,
};
use rustic_isles_store::AccountId;
use rustic_isles_store::repo::bounties::{BountyGroup, BountyStateRecord, DraftData, StoredBounty};
use serde_json::Value;

/// One drafted instance: the same `drafted_timestamp` by `===`, absent and `null` alike.
fn same_stamp(a: &Value, b: &Value) -> bool {
    let stamp = |v: &Value| v.get("drafted_timestamp").cloned().unwrap_or(Value::Null);
    dainty_service_kit::js::strict_eq(Some(&stamp(a)), Some(&stamp(b)))
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (body_text, stored_text) = text.split_once('\u{1}').unwrap_or((&text, ""));
    let body: Option<Value> = serde_json::from_str(body_text).ok();
    let Ok(account) = "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse::<AccountId>() else {
        return;
    };
    let stored_list: Vec<Value> = serde_json::from_str::<Value>(stored_text)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let stored = BountyStateRecord {
        account: account.clone(),
        bounties: stored_list
            .into_iter()
            .take(1000)
            .map(|bounty| StoredBounty {
                group: BountyGroup::Unassigned,
                claimed_at: Some(String::from("T0")),
                reward_credit: None,
                bounty,
            })
            .collect(),
        draft_data: DraftData::default(),
        daily_reward_credited_on: None,
        daily_premium_draft_granted_on: None,
        update_version: 3,
        created_at: "T0".into(),
        updated_at: "T0".into(),
    };
    let now_ms = i64::from(data.first().copied().unwrap_or(0)) * 86_400_000 * 7 + 1_727_370_000_000;
    if let Ok(save) = validate_bounty_save(body.as_ref()) {
        let dedicated = merge_bounty_state(Some(&stored), &account, &save, "T1");
        let host = merge_host_bounty_state(Some(&stored), &account, &save, "T1", now_ms);
        for merged in [&dedicated, &host] {
            assert!(merged.record.bounties.len() <= 1000);
            assert!(merged.record.update_version > stored.update_version);
            for entry in stored.bounties.iter().filter(|e| is_claimed(e)) {
                let id = bounty_id(entry);
                let once = stored.bounties.iter().filter(|e| bounty_id(e) == id).count() == 1;
                if let Some(kept) = merged.record.bounties.iter().find(|e| bounty_id(e) == id)
                    && once
                    && same_stamp(&kept.bounty, &entry.bounty)
                {
                    assert!(is_claimed(kept));
                }
            }
        }
        for entry in stored.bounties.iter().filter(|e| is_protected(e, now_ms)) {
            assert!(host.record.bounties.iter().any(|e| bounty_id(e) == bounty_id(entry)));
        }
        let before: Vec<Option<&str>> = stored.bounties.iter().map(bounty_id).collect();
        for entry in &host.record.bounties {
            let id = bounty_id(entry).unwrap_or("");
            if infer_group(id) == BountyGroup::Weekly && !before.contains(&Some(id)) {
                assert!(parse_weekly_id(id).is_some(), "{id}");
            }
        }
        let payload = bounty_state_payload(Some(&host.record));
        assert_eq!(payload.as_object().map(|m| m.len()), Some(6));
    }
    if let Ok(request) = parse_delete(body.as_ref()) {
        let done = delete_bounties(Some(&stored), &account, &request, "T2");
        assert!(done.record.update_version > stored.update_version);
    }
});
