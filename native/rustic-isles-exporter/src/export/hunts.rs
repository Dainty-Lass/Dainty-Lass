//! `hunts_1_14_7\`: eight tables and `hunt_export_manifest.json` (spec §6.10).

use super::{Ctx, Row, TableExport, TextFile, export_tables, tag_query, within};
use crate::engine::Engine;
use crate::json::{Json, escape};
use crate::sdk;

pub const FOLDER: &str = "hunts_1_14_7";
pub const MANIFEST: &str = "hunt_export_manifest.json";
const PREFIX: &str = "[Hunts] ";

pub const NOTE: &str = "Read-only live table export. Tag-routed query dictionaries and raw token streams are \
preserved. FText display fields are deliberately omitted (FText is a shared-ref to FTextData, not an FString; reading \
it faults). map_metadata_table is omitted: Blueprint-defined row struct with no verified layout. Do not replace \
backend tables until importer validation is complete.";

type Writer = fn(&mut Row);

/// `(file, row struct, manifest key, writer)`, in the order they are written.
const TABLES: [(&str, &str, &str, Writer); 8] = [
    (
        "player_hunts.jsonl",
        "PlayerHuntTableData",
        "playerHuntRows",
        player_hunt,
    ),
    (
        "matchmaker_hunts.jsonl",
        "MatchmakerHuntTableData",
        "matchmakerHuntRows",
        matchmaker_hunt,
    ),
    ("hunt_regions.jsonl", "Hunt_Region", "huntRegionRows", hunt_region),
    (
        "escalation_mode_specs.jsonl",
        "EscalationModeSpecification",
        "escalationModeSpecRows",
        escalation_spec,
    ),
    (
        "hunt_modifiers.jsonl",
        "HuntModifierTableRow",
        "huntModifierRows",
        hunt_modifier,
    ),
    (
        "game_activities.jsonl",
        "GameActivityTableData",
        "gameActivityRows",
        game_activity,
    ),
    (
        "activity_unlock_criteria.jsonl",
        "ProgressionUnlockCriteria",
        "unlockCriteriaRows",
        unlock_criteria,
    ),
    ("schedule_data.jsonl", "ScheduleData", "scheduleRows", schedule),
];

/// The run total: -1 when either of the first two failed, else the sum with the others' failures as 0.
pub fn export(ctx: &Ctx) -> i64 {
    ctx.status.line(PREFIX, "starting read-only 1.12 hunt export");
    let folder = crate::paths::subfolder(&ctx.out, FOLDER);
    let mut counts = [0i64; 8];
    for (count, (file, row_struct, _, writer)) in counts.iter_mut().zip(TABLES) {
        let spec = TableExport {
            prefix: PREFIX,
            row_struct,
            file: folder.join(file),
            inclusive: false,
            missing: format!("no loaded table with RowStruct={row_struct}; open the hunt/map UI and re-inject"),
            folder: None,
        };
        *count = export_tables(ctx, &spec, |j, e, table, row_name, at| {
            j.obj().kstr("sourceTable", table).kstr("rowName", row_name);
            writer(&mut Row { j: &mut *j, e, at });
            j.end_obj();
        });
    }
    if let Some(mut file) = TextFile::create(&folder.join(MANIFEST)) {
        file.text(&manifest(&counts));
        file.finish();
    }
    if counts[0] < 0 || counts[1] < 0 {
        -1
    } else {
        counts.iter().map(|count| (*count).max(0)).sum()
    }
}

pub fn manifest(counts: &[i64; 8]) -> String {
    let mut text =
        String::from("{\n  \"gameVersion\": \"1.14.7\",\n  \"changelist\": 647472,\n  \"format\": \"jsonl\",\n");
    for ((_, _, key, _), count) in TABLES.iter().zip(counts) {
        text.push_str(&format!("  \"{key}\": {count},\n"));
    }
    text.push_str(&format!("  \"notes\": {}\n}}\n", escape(NOTE)));
    text
}

fn object(j: &mut Json, e: &Engine, at: u64, fields: impl FnOnce(&mut Row)) {
    j.obj();
    fields(&mut Row { j: &mut *j, e, at });
    j.end_obj();
}

fn player_hunt(row: &mut Row) {
    let (e, at) = (row.e, row.at);
    row.s("playerHuntId", &sdk::PH_ID)
        .handle("region", &sdk::PH_REGION)
        .n("matchmakingType", &sdk::PH_MATCHMAKING_TYPE)
        .n("matchmakingGameType", &sdk::PH_MATCHMAKING_GAME_TYPE)
        .tags("huntTags", &sdk::PH_HUNT_TAGS)
        .handles("matchmakerHuntIDs", &sdk::PH_MATCHMAKER_HUNT_IDS);
    row.j.key("matchmakerHuntsByTag").arr();
    for list in e.items(at, &sdk::PH_BY_TAG) {
        object(row.j, e, list, |list_row| {
            list_row.s("matchmakerTable", &sdk::PH_BY_TAG_TABLE);
            list_row.j.key("queries").arr();
            for query in e.items(list_row.at, &sdk::PH_BY_TAG_QUERIES) {
                tag_query(list_row.j, e, query);
            }
            list_row.j.end_arr();
        });
    }
    row.j.end_arr();
    row.handle("escalationModeSpecification", &sdk::PH_ESCALATION_SPEC)
        .n("escalationPatrolInitialChallengeLevel", &sdk::PH_PATROL_INITIAL)
        .n("escalationPatrolMaxChallengeLevel", &sdk::PH_PATROL_MAX)
        .n("recommendedEscalationLevel", &sdk::PH_RECOMMENDED_ESCALATION)
        .b("hasPortals", &sdk::PH_HAS_PORTALS)
        .b("hasGlitterEvent", &sdk::PH_HAS_GLITTER)
        .b("hasPhaelanxEvent", &sdk::PH_HAS_PHAELANX)
        .s("huntSuccessReward", &sdk::PH_SUCCESS_REWARD)
        .s("firstCompletionSuccessReward", &sdk::PH_FIRST_REWARD)
        .s("targetedHuntSuccessReward", &sdk::PH_TARGETED_SUCCESS_REWARD)
        .s("huntFailureReward", &sdk::PH_FAILURE_REWARD)
        .s("targetedHuntFailureReward", &sdk::PH_TARGETED_FAILURE_REWARD)
        .n("minWeaponSkillLevel", &sdk::PH_MIN_SKILL)
        .n("recommendedWeaponSkillLevel", &sdk::PH_RECOMMENDED_SKILL)
        .b("visibleWhileLocked", &sdk::PH_VISIBLE_LOCKED);
}

/// `{"behemothName","behemothAsset Path","behemothAssetResolvedName","powerOverride","weighting"}`, with the
/// members' offsets taken inside the spawn struct.
fn behemoth(j: &mut Json, e: &Engine, at: u64) {
    let asset = within(sdk::MH_BEHEMOTH_ASSET, &sdk::MH_BEHEMOTH);
    let resolved = e.name_of(e.weak_object(at.wrapping_add(u64::from(asset.offset))));
    object(j, e, at, |row| {
        row.s("behemothName", &within(sdk::MH_BEHEMOTH_NAME, &sdk::MH_BEHEMOTH))
            .soft("behemothAssetPath", &asset);
        row.j.kstr("behemothAssetResolvedName", &resolved);
        row.n("powerOverride", &within(sdk::MH_BEHEMOTH_POWER, &sdk::MH_BEHEMOTH))
            .n("weighting", &within(sdk::MH_BEHEMOTH_WEIGHT, &sdk::MH_BEHEMOTH));
    });
}

fn matchmaker_hunt(row: &mut Row) {
    let (e, at) = (row.e, row.at);
    row.handle("region", &sdk::MH_REGION)
        .tags("huntTags", &sdk::MH_HUNT_TAGS)
        .n("huntThreatLevel", &sdk::MH_THREAT)
        .s("gameModeOverride", &sdk::MH_GAME_MODE)
        .n("dangerPerSecOverride", &sdk::MH_DANGER);
    row.j.key("specificBehemoth");
    behemoth(row.j, e, at.wrapping_add(u64::from(sdk::MH_BEHEMOTH.offset)));
    row.j.key("additionalSpecificBehemoths").arr();
    for spawn in e.items(at, &sdk::MH_EXTRA_BEHEMOTHS) {
        behemoth(row.j, e, spawn);
    }
    row.j.end_arr();
    row.handle("mapMetaData", &sdk::MH_MAP_METADATA);
    row.j.key("mapList").arr();
    for map in e.items(at, &sdk::MH_MAP_LIST) {
        let resolved = e.name_of(e.weak_object(map.wrapping_add(u64::from(sdk::MH_MAP_ASSET.offset))));
        object(row.j, e, map, |map_row| {
            map_row
                .s("mapName", &sdk::MH_MAP_NAME)
                .s("mapAssetName", &sdk::MH_MAP_ASSET_NAME)
                .soft("mapAssetPath", &sdk::MH_MAP_ASSET);
            map_row.j.kstr("mapAssetResolvedName", &resolved);
            map_row
                .n("biome", &sdk::MH_MAP_BIOME)
                .n("weighting", &sdk::MH_MAP_WEIGHT);
        });
    }
    row.j.end_arr();
    row.s("specificModifier", &sdk::MH_SPECIFIC_MODIFIER)
        .list("modifiers", &sdk::MH_MODIFIERS)
        .n("gatherableTier", &sdk::MH_GATHERABLE_TIER)
        .n("maxPlayers", &sdk::MH_MAX_PLAYERS)
        .b("isGeneratedEncounter", &sdk::MH_GENERATED)
        .handle("gameModeSpecificData", &sdk::MH_MODE_DATA);
}

fn hunt_region(row: &mut Row) {
    row.s("matchmakingId", &sdk::HR_MATCHMAKING_ID)
        .s("tokenId", &sdk::HR_TOKEN_ID)
        .n("numTokensRequired", &sdk::HR_TOKENS_REQUIRED)
        .n("numHuntPassBehemothRewards", &sdk::HR_PASS_REWARDS)
        .s("targetedBehemothList", &sdk::HR_TARGETED_LIST)
        .softs("behemothPresets", &sdk::HR_PRESETS);
}

fn escalation_spec(row: &mut Row) {
    row.n("initialChallengeLevel", &sdk::ES_INITIAL)
        .n("maxChallengeLevel", &sdk::ES_MAX)
        .count("roundStructureCount", &sdk::ES_ROUNDS)
        .handle("lootTable", &sdk::ES_LOOT_TABLE)
        .n("optionalPowerOverride", &sdk::ES_POWER_OVERRIDE)
        .tags("escalationModeTags", &sdk::ES_TAGS)
        .s("backupEncountersTable", &sdk::ES_BACKUP_TABLE)
        .list("completionRewards", &sdk::ES_COMPLETION_REWARDS)
        .nums("escalationAmountPerScoreValue", &sdk::ES_AMOUNT_PER_SCORE);
}

fn hunt_modifier(row: &mut Row) {
    row.soft("modifierAssetPath", &sdk::HM_ASSET)
        .n("huntComplexityModifier", &sdk::HM_COMPLEXITY)
        .n("threatLevel", &sdk::HM_THREAT)
        .n("modifierApplicationType", &sdk::HM_APPLICATION)
        .tags("modifierMetaTags", &sdk::HM_META_TAGS)
        .n("behemothElementCondition", &sdk::HM_ELEMENT);
}

fn game_activity(row: &mut Row) {
    row.b("enabled", &sdk::GA_ENABLED)
        .s("playerHunt", &sdk::GA_PLAYER_HUNT)
        .list("matchmakerHunts", &sdk::GA_MATCHMAKER_HUNTS)
        .n("unlockRequirement", &sdk::GA_UNLOCK_REQUIREMENT)
        .handles("unlockCriteria", &sdk::GA_UNLOCK_CRITERIA);
}

fn unlock_criteria(row: &mut Row) {
    let (e, at) = (row.e, row.at);
    row.n("featureFlagsOperation", &sdk::UC_FLAGS_OP)
        .softs("featureFlags", &sdk::UC_FLAGS)
        .n("itemIdsOperation", &sdk::UC_ITEMS_OP)
        .list("itemIds", &sdk::UC_ITEMS)
        .n("progressionTrackRanksOperation", &sdk::UC_RANKS_OP);
    row.j.key("progressionTrackRanks").arr();
    for rank in e.items(at, &sdk::UC_RANKS) {
        object(row.j, e, rank, |rank_row| {
            rank_row.s("track", &sdk::UC_RANK_TRACK).n("rank", &sdk::UC_RANK_RANK);
        });
    }
    row.j.end_arr();
    row.n("questRequirementsOperation", &sdk::UC_QUESTS_OP)
        .n("questRequirementsCondition", &sdk::UC_QUESTS_CONDITION);
    row.j.key("questRequirements").arr();
    for quest in e.items(at, &sdk::UC_QUESTS) {
        object(row.j, e, quest, |quest_row| {
            quest_row
                .s("questName", &sdk::UC_QUEST_NAME)
                .n("statusRequired", &sdk::UC_QUEST_STATUS);
        });
    }
    row.j.end_arr();
    row.n("playerJourneyNodesOperation", &sdk::UC_NODES_OP)
        .handles("playerJourneyNodes", &sdk::UC_NODES);
}

fn schedule(row: &mut Row) {
    let (e, at) = (row.e, row.at);
    row.n("startTimeTicks", &sdk::SD_START)
        .n("endTimeTicks", &sdk::SD_END)
        .b("isRepeatable", &sdk::SD_REPEATABLE)
        .b("eachDay", &sdk::SD_EACH_DAY)
        .b("eachWeek", &sdk::SD_EACH_WEEK)
        .b("shouldQueue", &sdk::SD_SHOULD_QUEUE);
    row.j.key("scheduledItems").arr();
    for item in e.items(at, &sdk::SD_ITEMS) {
        object(row.j, e, item, |item_row| {
            item_row
                .s("id", &sdk::SD_ITEM_ID)
                .n("maxCompletionPerInterval", &sdk::SD_ITEM_MAX);
        });
    }
    row.j.end_arr();
    row.nums("itemsLeftOut", &sdk::SD_LEFT_OUT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_manifest() {
        let text = manifest(&[3, -1, 0, 1, 2, 0, 0, 5]);
        assert!(text.starts_with(
            "{\n  \"gameVersion\": \"1.14.7\",\n  \"changelist\": 647472,\n  \"format\": \"jsonl\",\n  \
             \"playerHuntRows\": 3,\n  \"matchmakerHuntRows\": -1,\n  \"huntRegionRows\": 0,\n"
        ));
        assert!(text.ends_with(&format!("  \"scheduleRows\": 5,\n  \"notes\": \"{NOTE}\"\n}}\n")));
        assert!(NOTE.contains("(FText is a shared-ref to FTextData, not an FString; reading it faults)"));
    }
}
