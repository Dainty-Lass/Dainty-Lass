//! `slayers_path_1_14_7\player_journey_nodes.jsonl` (spec §6.9). No `FText` is read.

use super::{Ctx, Row, TableExport, export_tables};
use crate::engine::Engine;
use crate::json::Json;
use crate::sdk;

pub const FOLDER: &str = "slayers_path_1_14_7";
pub const FILE: &str = "player_journey_nodes.jsonl";
const PREFIX: &str = "[SlayersPath] ";
const ROW_STRUCT: &str = "PlayerJourneyNodeData";

pub fn export(ctx: &Ctx) -> i64 {
    let folder = ctx.out.join(FOLDER);
    let spec = TableExport {
        prefix: PREFIX,
        row_struct: ROW_STRUCT,
        file: folder.join(FILE),
        inclusive: false,
        missing: format!(
            "no tables found with RowStruct={ROW_STRUCT} (is the Slayer's Path screen loaded? open it once, then re-run)"
        ),
        folder: Some(folder),
    };
    export_tables(ctx, &spec, node)
}

fn node(j: &mut Json, e: &Engine, table: &str, row_name: &str, at: u64) {
    j.obj().kstr("nodeId", row_name).kstr("table", table);
    let mut row = Row { j, e, at };
    row.n("nodeType", &sdk::SP_NODE_TYPE)
        .n("nodeLevel", &sdk::SP_NODE_LEVEL)
        .list("childNodes", &sdk::SP_CHILD_NODES)
        .b("autoUnlockIfParentUnlocked", &sdk::SP_AUTO_UNLOCK);

    row.j.key("currencyCosts").arr();
    for cost in e.items(at, &sdk::SP_CURRENCY_COSTS) {
        row.j.obj();
        Row {
            j: &mut *row.j,
            e,
            at: cost,
        }
        .s("currency", &sdk::SP_CURRENCY_COST_CURRENCY)
        .n("amount", &sdk::SP_CURRENCY_COST_AMOUNT);
        row.j.end_obj();
    }
    row.j.end_arr().key("experienceCosts").arr();
    for cost in e.items(at, &sdk::SP_EXPERIENCE_COSTS) {
        row.j.obj();
        Row {
            j: &mut *row.j,
            e,
            at: cost,
        }
        .n("experienceType", &sdk::SP_EXPERIENCE_COST_TYPE)
        .n("amount", &sdk::SP_EXPERIENCE_COST_AMOUNT);
        row.j.end_obj();
    }
    row.j.end_arr().key("rewards").arr();
    for reward in e.items(at, &sdk::SP_REWARDS) {
        row.j.obj();
        Row {
            j: &mut *row.j,
            e,
            at: reward,
        }
        .s("itemId", &sdk::SP_REWARD_ITEM_ID)
        .n("amount", &sdk::SP_REWARD_AMOUNT)
        .s("entitlementId", &sdk::SP_REWARD_ENTITLEMENT_ID)
        .n("duration", &sdk::SP_REWARD_DURATION)
        .s("passPrefix", &sdk::SP_REWARD_PASS_PREFIX)
        .n("experienceType", &sdk::SP_REWARD_EXPERIENCE_TYPE)
        .b("autoEquip", &sdk::SP_REWARD_AUTO_EQUIP);
        row.j.end_obj();
    }
    row.j.end_arr();
    row.count("systemRewardCount", &sdk::SP_SYSTEM_REWARDS)
        .s("activationEventId", &sdk::SP_ACTIVATION_EVENT_ID)
        .list("questIds", &sdk::SP_QUEST_IDS)
        .s("questIdToNotifyOnUnlock", &sdk::SP_QUEST_TO_NOTIFY)
        .s("tutorialToShowWhenUnlocked", &sdk::SP_TUTORIAL)
        .count("objectiveCount", &sdk::SP_OBJECTIVES);

    let map = sdk::SP_ATTRIBUTE_VALUES;
    let (_, entries) = e.map_elements(at.wrapping_add(u64::from(map.offset)), map.stride);
    row.j.key("gameplayAttributeValues").obj();
    for entry in entries {
        let name = e.text_at(entry.wrapping_add(u64::from(map.key_offset)), map.elem);
        let value = e.number_at(entry.wrapping_add(u64::from(map.value_offset)), map.value);
        row.j.key(&name).num(value);
    }
    row.j.end_obj();
    j.end_obj();
}
