//! What the database tool reads out of a host's MongoDB (spec-rustic-isles-db §7, §8, §15): arbitrary JSON
//! documents as every collection of a fake source, through extraction and the RB11 repairs.
//! Nothing panics, every JSON text column parses, every loadout keeps at least one slot, and every repaired
//! item id is 26 characters of `A`-`Z` and `0`-`9`.
#![no_main]

use std::collections::BTreeMap;

use libfuzzer_sys::fuzz_target;
use rustic_isles_db::LegacyStarter;
use rustic_isles_db::fuzzing::{extract_documents, rows};
use rustic_isles_db::ids::is_instance_id;
use serde_json::Value;

const COLLECTIONS: &[&str] = &[
    "accounts",
    "authIdentities",
    "refreshSessions",
    "gameExchangeCodes",
    "discordOAuthTransactions",
    "emailCodes",
    "gameServerApiKeys",
    "userApiKeys",
    "playerJourney",
    "progressionTracks",
    "progressionObjectives",
    "progressionObjectiveEvents",
    "progressionTransactions",
    "escalationProgress",
    "breadcrumbs",
    "encounteredContent",
    "characters",
    "inventories",
    "loadouts",
    "inventoryTransactions",
    "bountyStates",
    "cooldownStates",
    "entitlements",
    "wallets",
    "economyDaily",
    "partyMemberships",
    "parties",
    "partyInvites",
    "partyOutbox",
    "friendships",
    "reforgedInvites",
    "adminSessions",
    "adminAudit",
    "launcherNotices",
    "operationsPolicy",
    "launcherGuardSessions",
    "guardFindings",
];

const JSON_COLUMNS: &[(&str, &str)] = &[
    ("launcher_accounts", "roles"),
    ("characters", "data"),
    ("loadouts", "slots_json"),
    ("loadouts", "persistent"),
    ("inventories", "instanced_items"),
    ("inventories", "stacked_items"),
    ("inventory_transactions", "result"),
    ("inventory_transactions", "extra"),
    ("bounty_states", "bounties"),
    ("bounty_states", "draft_data"),
    ("cooldown_states", "entries"),
    ("progression_tracks", "reforge"),
    ("progression_objective_events", "raw_body"),
    ("progression_grants", "result"),
    ("escalation_progress", "talents_progress"),
    ("escalation_progress", "unlock_progress"),
    ("player_journey", "nodes"),
    ("breadcrumbs", "items"),
    ("encountered_content", "content"),
    ("party_outbox", "recipients"),
    ("party_outbox", "payload"),
    ("admin_audit", "old_state"),
    ("admin_audit", "new_state"),
    ("launcher_notices", "announcements"),
    ("launcher_notices", "tags"),
    ("launcher_guard_sessions", "detail"),
    ("guard_findings", "account_ids"),
];

fuzz_target!(|data: &[u8]| {
    let Ok(Value::Array(items)) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    let docs: Vec<_> = items.into_iter().filter_map(|v| v.as_object().cloned()).collect();
    let collections: BTreeMap<String, Vec<_>> = COLLECTIONS.iter().map(|c| ((*c).to_owned(), docs.clone())).collect();
    let starter = LegacyStarter {
        instanced: vec!["W".into()],
        equipment: vec![("weapon".into(), "W".into())],
        banner: "B".into(),
        glider: "G".into(),
        default_dye: "D".into(),
        default_sigil: "S".into(),
    };
    // An empty user id aborts by design (P2); anything else is checked
    let Ok(state) = extract_documents(collections, true, &starter) else {
        return;
    };
    for (table, column) in JSON_COLUMNS {
        for row in rows(&state, table) {
            match row.get(*column) {
                Some(Value::String(text)) => {
                    assert!(serde_json::from_str::<Value>(text).is_ok(), "{table}.{column}");
                }
                Some(Value::Null) | None => {}
                Some(other) => panic!("{table}.{column} is not text: {other}"),
            }
        }
    }
    for loadout in rows(&state, "loadouts") {
        assert!(loadout["unlocked_total_slots"].as_i64().is_some_and(|n| n >= 1));
    }
    for inventory in rows(&state, "inventories") {
        let items: Value =
            serde_json::from_str(inventory["instanced_items"].as_str().unwrap_or("[]")).unwrap_or_default();
        for item in items.as_array().into_iter().flatten() {
            if item.get("catalogId").is_some_and(Value::is_string) {
                assert!(item["instanceId"].as_str().is_some_and(is_instance_id), "{item}");
            }
        }
    }
});
