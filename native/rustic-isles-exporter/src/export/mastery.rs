//! `mastery_1_14_7\mastery_objectives.json`: the only pretty file, written in one write with LF line ends (spec
//! §6.14).

use std::collections::{BTreeSet, HashSet};

use super::{Ctx, ROW_LIMIT};
use crate::engine::Engine;
use crate::json::escape;
use crate::sdk;
use crate::sdk_parse::{Kind, Member};

pub const FOLDER: &str = "mastery_1_14_7";
pub const FILE: &str = "mastery_objectives.json";
const PREFIX: &str = "[Mastery] ";
const CARD_STRUCT: &str = "MasteryCardTableData";
const ACHIEVEMENT_STRUCT: &str = "AchievementTableData";
const PLAYER_LEVEL_TRACK: &str = "MasteryTrack_PlayerLevel";
const MAX_OBJECTIVES: i64 = 2000;
const MAX_REWARD_TRACKS: i64 = 64;

/// The objective members of one row struct.
struct Objective {
    list: Member,
    id: Member,
    target: Member,
    data: Member,
    compound: Option<Member>,
    rank: Member,
    track: Member,
}

const CARD_OBJECTIVE: Objective = Objective {
    list: sdk::MC_OBJECTIVES,
    id: sdk::MC_OBJ_ID,
    target: sdk::MC_OBJ_TARGET,
    data: sdk::MC_OBJ_DATA,
    compound: Some(sdk::MC_OBJ_COMPOUND),
    rank: sdk::MC_OBJ_RANK,
    track: sdk::MC_OBJ_TRACK,
};

const ACHIEVEMENT_OBJECTIVE: Objective = Objective {
    list: sdk::AC_OBJECTIVES,
    id: sdk::AC_OBJ_ID,
    target: sdk::AC_OBJ_TARGET,
    data: sdk::AC_OBJ_DATA,
    compound: None,
    rank: sdk::AC_OBJ_RANK,
    track: sdk::AC_OBJ_TRACK,
};

pub fn export(ctx: &Ctx) -> i64 {
    export_dated(ctx, &crate::clock::Utc::now().date())
}

/// `date` is the `extracted` date (UTC).
pub fn export_dated(ctx: &Ctx, date: &str) -> i64 {
    let e = ctx.engine;
    let mut tables_read = Vec::new();
    let card_rows = unique_rows(ctx, CARD_STRUCT, &mut tables_read);
    let achievement_rows = unique_rows(ctx, ACHIEVEMENT_STRUCT, &mut tables_read);
    if card_rows.is_empty() && achievement_rows.is_empty() {
        ctx.status.line(PREFIX, "neither table is loaded; nothing written");
        return 0;
    }
    let mut ids = BTreeSet::new();
    let mut cards = Vec::new();
    for (name, at) in &card_rows {
        e.take_fault();
        let objectives = e.count(*at, &sdk::MC_OBJECTIVES);
        let tracks = e.count(*at, &sdk::MC_REWARD_TRACKS);
        if !(0..=MAX_OBJECTIVES).contains(&objectives) || !(0..=MAX_REWARD_TRACKS).contains(&tracks) {
            ctx.status.line(
                PREFIX,
                &format!("card {name}: {objectives} objectives, {tracks} reward tracks - skipped"),
            );
            continue;
        }
        let mut card_ids = Vec::new();
        let card = card_text(e, name, *at, &mut card_ids);
        if e.take_fault() {
            ctx.status.line(PREFIX, &format!("card {name}: unreadable row skipped"));
            continue;
        }
        ids.extend(card_ids);
        cards.push(card);
    }
    let mut achievements = Vec::new();
    for (name, at) in &achievement_rows {
        e.take_fault();
        let objectives = e.count(*at, &sdk::AC_OBJECTIVES);
        if !(0..=MAX_OBJECTIVES).contains(&objectives) {
            ctx.status.line(
                PREFIX,
                &format!("achievement {name}: {objectives} objectives - skipped"),
            );
            continue;
        }
        let mut achievement_ids = Vec::new();
        let list = objectives_text(e, *at, &ACHIEVEMENT_OBJECTIVE, &mut achievement_ids);
        if e.take_fault() {
            ctx.status
                .line(PREFIX, &format!("achievement {name}: unreadable row skipped"));
            continue;
        }
        ids.extend(achievement_ids);
        achievements.push(format!(
            "    {{\n      \"id\": {},\n{}    }}",
            escape(name),
            list.block("", "\n")
        ));
    }

    let source = format!(
        "Dauntless 1.14.7 CL 647472: mastery_card_table, achievements_table (CatalogExporter live export of {}), \
         extracted {date}",
        tables_read.join(", ")
    );
    let mut text = format!("{{\n  \"schema\": 1,\n  \"source\": {},\n", escape(&source));
    let id_lines: Vec<String> = ids.iter().map(|id| format!("    {}", escape(id))).collect();
    text.push_str(&list_block("  ", "objectiveIds", &id_lines, ",\n"));
    text.push_str(&list_block("  ", "cards", &cards, ",\n"));
    text.push_str(&list_block("  ", "achievements", &achievements, "\n"));
    text.push_str("}\n");

    let folder = crate::paths::subfolder(&ctx.out, FOLDER);
    if std::fs::write(folder.join(FILE), text.as_bytes()).is_err() {
        ctx.status.line(PREFIX, &format!("cannot write {FILE}"));
        return -1;
    }
    let written = (cards.len() + achievements.len()) as i64;
    ctx.status.line(
        PREFIX,
        &format!(
            "wrote {} cards, {} achievements, {} objective ids -> {FILE}",
            cards.len(),
            achievements.len(),
            ids.len()
        ),
    );
    written
}

/// `"<key>": [` lines `]` (or `[]` on one line), indented by `indent`, followed by `after`.
fn list_block(indent: &str, key: &str, lines: &[String], after: &str) -> String {
    if lines.is_empty() {
        format!("{indent}\"{key}\": []{after}")
    } else {
        format!("{indent}\"{key}\": [\n{}\n{indent}]{after}", lines.join(",\n"))
    }
}

/// The rows of every table with `row_struct` (array order, then map order), first of each name kept, empty names
/// skipped, then sorted by name.
fn unique_rows(ctx: &Ctx, row_struct: &str, tables_read: &mut Vec<String>) -> Vec<(String, u64)> {
    let e = ctx.engine;
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for table in e.tables_by_row_struct(row_struct) {
        let table_name = e.name_of(table);
        let (count, entries) = e.rows(table);
        if !(0..ROW_LIMIT).contains(&count) {
            ctx.status.line(
                PREFIX,
                &format!("{table_name}: row count {count} out of range, skipping"),
            );
            continue;
        }
        tables_read.push(table_name.clone());
        for (name, at) in entries {
            if name.is_empty() || at == 0 {
                continue;
            }
            if !seen.insert(name.clone()) {
                ctx.status
                    .line(PREFIX, &format!("{table_name}: duplicate row {name} dropped"));
                continue;
            }
            rows.push((name, at));
        }
    }
    rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    rows
}

struct Objectives {
    lines: Vec<String>,
}

impl Objectives {
    /// The `objectives` member: one line per objective, or `[]` on one line.
    fn block(&self, after_close: &str, after: &str) -> String {
        if self.lines.is_empty() {
            format!("      \"objectives\": []{after_close}{after}")
        } else {
            let lines: Vec<String> = self.lines.iter().map(|line| format!("        {line}")).collect();
            format!(
                "      \"objectives\": [\n{}\n      ]{after_close}{after}",
                lines.join(",\n")
            )
        }
    }
}

fn useful(text: &str) -> bool {
    !text.is_empty() && text != "None"
}

fn objectives_text(e: &Engine, at: u64, members: &Objective, ids: &mut Vec<String>) -> Objectives {
    let mut lines = Vec::new();
    for objective in e.items(at, &members.list) {
        let id = e.text(objective, &members.id);
        if id.is_empty() {
            continue;
        }
        let mut line = format!(
            "{{\"id\":{},\"target\":{}",
            escape(&id),
            e.int(objective, &members.target)
        );
        let class = data_class(e, objective, &members.data);
        let kind = class.strip_prefix("ObjectiveData_").unwrap_or(&class);
        if !kind.is_empty() {
            line.push_str(&format!(",\"type\":{}", escape(kind)));
        }
        if let Some(compound) = &members.compound {
            let compound = e.text(objective, compound);
            if useful(&compound) {
                line.push_str(&format!(",\"compoundId\":{}", escape(&compound)));
            }
        }
        let rank = e.int(objective, &members.rank);
        if rank > 0 {
            line.push_str(&format!(",\"rank\":{rank}"));
            let track = e.text(objective, &members.track);
            if useful(&track) {
                line.push_str(&format!(",\"progressTrack\":{}", escape(&track)));
            }
        }
        line.push('}');
        ids.push(id);
        lines.push(line);
    }
    Objectives { lines }
}

/// The objective data's class name: the class itself for a class member, the object's class otherwise.
fn data_class(e: &Engine, objective: u64, member: &Member) -> String {
    let pointer = e.ptr(objective.wrapping_add(u64::from(member.offset)));
    if pointer == 0 {
        return String::new();
    }
    match member.kind {
        Kind::ClassPtr => e.name_of(pointer),
        _ => e.class_name_of(pointer),
    }
}

fn card_text(e: &Engine, name: &str, at: u64, ids: &mut Vec<String>) -> String {
    let track = e
        .strings(at, &sdk::MC_REWARD_TRACKS)
        .into_iter()
        .find(|track| useful(track) && track != PLAYER_LEVEL_TRACK)
        .map_or_else(|| "null".to_owned(), |track| escape(&track));
    let objectives = objectives_text(e, at, &CARD_OBJECTIVE, ids);
    let card_type = e.int(at, &sdk::MC_CARD_TYPE);
    let mut extras = vec![format!(
        "\"type\": {}",
        if card_type == sdk::CARD_TYPE_WEAPON {
            "\"Weapon\""
        } else if card_type == sdk::CARD_TYPE_BEHEMOTH {
            "\"Behemoth\""
        } else {
            "null"
        }
    )];
    let game_object = e.text(at, &sdk::MC_GAME_OBJECT);
    if !game_object.is_empty() {
        extras.push(format!("\"gameObjectId\": {}", escape(&game_object)));
    }
    for (key, member) in [
        ("baseXpObjectiveId", sdk::MC_BASE_XP),
        ("prestigeXpObjectiveId", sdk::MC_PRESTIGE_XP),
    ] {
        let id = e.text(at, &member);
        if useful(&id) {
            extras.push(format!("\"{key}\": {}", escape(&id)));
            ids.push(id);
        }
    }
    format!(
        "    {{\n      \"cardId\": {}, \"track\": {track},\n{}      {}\n    }}",
        escape(name),
        objectives.block(",", "\n"),
        extras.join(", ")
    )
}
