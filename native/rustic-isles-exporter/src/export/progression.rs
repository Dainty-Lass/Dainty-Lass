//! `progression_1_14_7\`: eight files (spec §6.6).

use std::path::Path;

use super::{Ctx, ROW_LIMIT, Row, TextFile};
use crate::engine::{Engine, sorted_rows};
use crate::facts::{
    B_EN_DURATION, B_EN_ID, B_FREE, B_GATING, B_MULTIPLIER, B_OR_ID, B_OR_PRIORITY, B_PREMIUM, B_REQ_RANK, B_REQ_XP,
    B_REQUIREMENTS, B_RESET, B_RW_BUFFS, B_RW_CUSTOM, B_RW_ENTITLEMENTS, B_RW_INSTANCED, B_RW_ORDERED, B_RW_RANK,
    B_RW_STACKED, B_ST_ID, B_ST_PRIORITY, B_ST_QUANTITY, B_TRACK, BASE_LEN,
};
use crate::json::Json;
use crate::sdk;
use crate::sdk_parse::Member;

pub const FOLDER: &str = "progression_1_14_7";
const PREFIX: &str = "[Progression] ";
const AWARDS_FILE: &str = "experience-awards.jsonl";
const AWARDS_ROW_STRUCT: &str = "ExperienceTableData";

type Base = [Member; BASE_LEN];
type Writer = fn(&mut Json, &Engine, &str, u64);

/// `(file, table name, row struct, writer)`: the tables found by name. The awards go between the hunt pass and the
/// progression tracks.
const NAMED: [(&str, &str, &str, Writer); 7] = [
    (
        "player-experience-tracks.jsonl",
        "player_experience_track_table",
        "ExperienceTrackTableData",
        experience_track,
    ),
    (
        "weapon-experience-tracks.jsonl",
        "weapon_experience_track_table",
        "ExperienceTrackTableData",
        experience_track,
    ),
    (
        "experience-tracks.jsonl",
        "experience_track_table",
        "ExperienceTrackTableData",
        experience_track,
    ),
    (
        "huntpass-season-table.jsonl",
        "hunt_pass_season_table",
        "HuntPassSeasonDataTable",
        hunt_pass_season,
    ),
    (
        "progression-tracks.jsonl",
        "progression_track_table",
        "ProgressionTrackTableData",
        progression_track,
    ),
    (
        "online-track-info.jsonl",
        "online_progression_track_info_table",
        "OnlineProgressionTrackInfo",
        online_track,
    ),
    (
        "mastery-tracks.jsonl",
        "mastery_track_table",
        "MasteryTrackTableData",
        mastery_track,
    ),
];

/// The rows written in all eight files.
pub fn export(ctx: &Ctx) -> i64 {
    let folder = crate::paths::subfolder(&ctx.out, FOLDER);
    let mut total = 0;
    for (index, (file, table, row_struct, writer)) in NAMED.iter().enumerate() {
        if index == 4 {
            total += awards(ctx, &folder).max(0);
        }
        total += named(ctx, &folder, file, table, row_struct, *writer).max(0);
    }
    total
}

fn named(ctx: &Ctx, folder: &Path, file: &str, name: &str, row_struct: &str, writer: Writer) -> i64 {
    let e = ctx.engine;
    let Some(table) = e.table_by_name(name) else {
        ctx.status.line(PREFIX, &format!("{name}: table not found, skipping"));
        return 0;
    };
    match e.row_struct_name(table) {
        None => {
            ctx.status
                .line(PREFIX, &format!("{name}: RowStruct is null - skipping"));
            return 0;
        }
        Some(found) if found != row_struct => {
            ctx.status.line(
                PREFIX,
                &format!(
                    "RowStruct mismatch on {name}: expected {row_struct} got {found} - skipping (never guessing a \
                     wrong cast)"
                ),
            );
            return 0;
        }
        Some(_) => {}
    }
    let (count, rows) = e.rows(table);
    if !(0..ROW_LIMIT).contains(&count) {
        ctx.status
            .line(PREFIX, &format!("{name}: row count {count} out of range, skipping"));
        return 0;
    }
    let Some(mut out) = TextFile::create(&folder.join(file)) else {
        ctx.status.line(PREFIX, &format!("cannot open {file}"));
        return -1;
    };
    let written = write_rows(ctx, &mut out, name, &rows, writer);
    out.finish();
    ctx.status
        .line(PREFIX, &format!("wrote {written}/{count} rows -> {file} from {name}"));
    written
}

fn write_rows(
    ctx: &Ctx,
    out: &mut TextFile,
    table: &str,
    rows: &[(String, u64)],
    mut writer: impl FnMut(&mut Json, &Engine, &str, u64),
) -> i64 {
    let e = ctx.engine;
    let mut written = 0;
    for (row_name, at) in sorted_rows(rows) {
        e.take_fault();
        let mut j = Json::new();
        if at != 0 {
            writer(&mut j, e, &row_name, at);
        }
        if at == 0 || e.take_fault() {
            ctx.status
                .line(PREFIX, &format!("{table}/{row_name}: unreadable row skipped"));
            continue;
        }
        out.line(&j.into_bytes());
        written += 1;
    }
    written
}

/// Every table by row struct, else the first live experience component's table (spec §6.6).
fn awards(ctx: &Ctx, folder: &Path) -> i64 {
    let e = ctx.engine;
    let mut tables = e.tables_by_row_struct(AWARDS_ROW_STRUCT);
    if tables.is_empty() {
        let class = e.find_class("PlayerExperienceComponent");
        let fallback = e
            .objects()
            .into_iter()
            .map(|(_, object)| object)
            .filter(|&object| class != 0 && e.is_a(object, class) && !e.is_default(object))
            .map(|component| e.ptr(component.wrapping_add(u64::from(sdk::PEC_TABLE.offset))))
            .find(|&table| table != 0 && e.row_struct_name(table).as_deref() == Some(AWARDS_ROW_STRUCT));
        e.take_fault();
        match fallback {
            Some(table) => {
                ctx.status.line(
                    PREFIX,
                    &format!(
                        "experience awards: using UPlayerExperienceComponent::ExperienceTable composite fallback ({})",
                        e.name_of(table)
                    ),
                );
                tables.push(table);
            }
            None => {
                ctx.status.line(
                    PREFIX,
                    &format!("experience awards: no table with RowStruct={AWARDS_ROW_STRUCT}"),
                );
                return 0;
            }
        }
    }
    let Some(mut out) = TextFile::create(&folder.join(AWARDS_FILE)) else {
        ctx.status.line(PREFIX, &format!("cannot open {AWARDS_FILE}"));
        return -1;
    };
    let mut written = 0;
    for table in tables {
        let name = e.name_of(table);
        let (count, rows) = e.rows(table);
        if !(0..ROW_LIMIT).contains(&count) {
            ctx.status
                .line(PREFIX, &format!("{name}: row count {count} out of range, skipping"));
            continue;
        }
        let table_written = write_rows(ctx, &mut out, &name, &rows, |j, e, row_name, at| {
            j.obj().kstr("rowName", row_name);
            Row { j: &mut *j, e, at }
                .n("experienceType", &sdk::AW_TYPE)
                .n("amount", &sdk::AW_AMOUNT)
                .count("objectiveCount", &sdk::AW_OBJECTIVES);
            j.kstr("table", &name).end_obj();
        });
        ctx.status.line(
            PREFIX,
            &format!("wrote {table_written}/{count} rows -> {AWARDS_FILE} from {name}"),
        );
        written += table_written;
    }
    out.finish();
    written
}

fn base(j: &mut Json, e: &Engine, at: u64, b: &Base) {
    j.key("base").obj();
    let mut row = Row { j: &mut *j, e, at };
    row.s("progressionTrack", &b[B_TRACK])
        .s("premiumGatingEntitlement", &b[B_GATING])
        .b("resetWithCharacter", &b[B_RESET])
        .n("progressionMultiplier", &b[B_MULTIPLIER])
        .count("requirementCount", &b[B_REQUIREMENTS]);
    j.key("requirements").arr();
    for requirement in e.items(at, &b[B_REQUIREMENTS]) {
        j.obj();
        Row {
            j: &mut *j,
            e,
            at: requirement,
        }
        .n("rankId", &b[B_REQ_RANK])
        .n("requiredXP", &b[B_REQ_XP]);
        j.end_obj();
    }
    j.end_arr();
    for (key, list) in [("freeRewards", B_FREE), ("premiumRewards", B_PREMIUM)] {
        j.key(key).arr();
        for reward_at in e.items(at, &b[list]) {
            reward(j, e, reward_at, b);
        }
        j.end_arr();
    }
    j.end_obj();
}

fn reward(j: &mut Json, e: &Engine, at: u64, b: &Base) {
    j.obj();
    Row { j: &mut *j, e, at }
        .n("rankId", &b[B_RW_RANK])
        .s("customRewardString", &b[B_RW_CUSTOM]);
    j.key("stackedItems").arr();
    for item in e.items(at, &b[B_RW_STACKED]) {
        j.obj();
        Row {
            j: &mut *j,
            e,
            at: item,
        }
        .s("catalogId", &b[B_ST_ID])
        .n("quantity", &b[B_ST_QUANTITY])
        .n("priority", &b[B_ST_PRIORITY]);
        j.end_obj();
    }
    j.end_arr();
    Row { j: &mut *j, e, at }.list("instancedItems", &b[B_RW_INSTANCED]);
    j.key("orderedInstancedItems").arr();
    for item in e.items(at, &b[B_RW_ORDERED]) {
        j.obj();
        Row {
            j: &mut *j,
            e,
            at: item,
        }
        .s("catalogId", &b[B_OR_ID])
        .n("priority", &b[B_OR_PRIORITY]);
        j.end_obj();
    }
    j.end_arr().key("entitlements").arr();
    for entitlement in e.items(at, &b[B_RW_ENTITLEMENTS]) {
        j.obj();
        Row {
            j: &mut *j,
            e,
            at: entitlement,
        }
        .s("entitlementId", &b[B_EN_ID])
        .n("duration", &b[B_EN_DURATION]);
        j.end_obj();
    }
    j.end_arr();
    Row { j: &mut *j, e, at }.count("buffCount", &b[B_RW_BUFFS]);
    j.end_obj();
}

fn experience_track(j: &mut Json, e: &Engine, row_name: &str, at: u64) {
    j.obj().kstr("rowName", row_name);
    base(j, e, at, &sdk::EXP_BASE);
    Row { j: &mut *j, e, at }
        .s("prestigeTrack", &sdk::EXP_PRESTIGE_TRACK)
        .s("expDisplayShortName", &sdk::EXP_SHORT_NAME)
        .s("expDisplayName", &sdk::EXP_NAME)
        .s("expDisplayDescription", &sdk::EXP_DESCRIPTION)
        .count("experienceBankingLevelCount", &sdk::EXP_BANKING_LEVELS);
    j.end_obj();
}

fn hunt_pass_season(j: &mut Json, e: &Engine, row_name: &str, at: u64) {
    j.obj().kstr("rowName", row_name);
    base(j, e, at, &sdk::HP_BASE);
    Row { j: &mut *j, e, at }
        .b("mustClaimRewards", &sdk::HP_MUST_CLAIM)
        .s("seasonTitle", &sdk::HP_TITLE)
        .s("seasonDescription", &sdk::HP_DESCRIPTION)
        .s("seasonDate", &sdk::HP_DATE)
        .s("huntRewardItemId", &sdk::HP_REWARD_ITEM)
        .n("huntRewardItemAmount", &sdk::HP_REWARD_AMOUNT)
        .s("nextProgressionTrack", &sdk::HP_NEXT_TRACK)
        .s("previewsProgressionTrack", &sdk::HP_PREVIOUS_TRACK)
        .b("buyLevelsUsingRank", &sdk::HP_BUY_WITH_RANK)
        .b("giveSeasonalCurrencies", &sdk::HP_SEASONAL_CURRENCIES)
        .s("overallSeasonName", &sdk::HP_OVERALL_NAME);
    j.end_obj();
}

fn progression_track(j: &mut Json, e: &Engine, row_name: &str, at: u64) {
    j.obj().kstr("rowName", row_name);
    Row { j: &mut *j, e, at }
        .s("displayName", &sdk::PT_DISPLAY_NAME)
        .n("currentVersion", &sdk::PT_VERSION)
        .b("repeats", &sdk::PT_REPEATS)
        .b("isPrestigeTrack", &sdk::PT_IS_PRESTIGE)
        .b("isCoreTrack", &sdk::PT_IS_CORE)
        .b("visibleInUI", &sdk::PT_VISIBLE)
        .s("prestigeTrack", &sdk::PT_PRESTIGE_TRACK)
        .s("rankTable", &sdk::PT_RANK_TABLE)
        .count("unlockConditionCount", &sdk::PT_UNLOCK_CONDITIONS)
        .count("challengeSlotCount", &sdk::PT_CHALLENGE_SLOTS)
        .n("craftItemRewardAmount", &sdk::PT_CRAFT_REWARD)
        .n("upgradeItemRewardAmount", &sdk::PT_UPGRADE_REWARD);
    j.end_obj();
}

fn online_track(j: &mut Json, e: &Engine, row_name: &str, at: u64) {
    j.obj().kstr("rowName", row_name);
    Row { j: &mut *j, e, at }
        .s("displayName", &sdk::OT_DISPLAY_NAME)
        .s("progressDisplaySingular", &sdk::OT_SINGULAR)
        .s("progressDisplayPlural", &sdk::OT_PLURAL)
        .s("progressDescription", &sdk::OT_DESCRIPTION);
    j.end_obj();
}

fn mastery_track(j: &mut Json, e: &Engine, row_name: &str, at: u64) {
    j.obj().kstr("rowName", row_name);
    base(j, e, at, &sdk::MT_BASE);
    Row { j: &mut *j, e, at }
        .s("displayName", &sdk::MT_DISPLAY_NAME)
        .s("shortDisplayName", &sdk::MT_SHORT_NAME)
        .b("isUIEnabled", &sdk::MT_UI_ENABLED)
        .n("category", &sdk::MT_CATEGORY);
    j.end_obj();
}
