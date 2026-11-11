// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hunt-table importer (spec §10, §14.2), authored: handles use `table`, as the CatalogExporter writes them.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};

use rustic_isles_director::hunt_tables::{ImportHuntTablesOptions, import_hunt_tables};
use serde_json::{Value, json};

fn lines(rows: &[Value]) -> String {
    rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
}

fn player(name: &str, game_type: Value) -> Value {
    json!({
        "rowName": name, "sourceTable": "player_hunts_table", "region": {"table": "regions", "rowName": "Moss"},
        "matchmakerHuntIDs": [{"table": "matchmaker_hunts_table", "rowName": "MM_New"}], "matchmakerHuntsByTag": [],
        "matchmakingType": 1, "matchmakingGameType": game_type, "huntTags": ["Hunt.A"], "hasPortals": false,
        "recommendedWeaponSkillLevel": 3
    })
}

fn matchmaker(name: &str, map: &str) -> Value {
    json!({
        "rowName": name, "huntTags": ["Hunt.A"], "gameModeOverride": "",
        "specificBehemoth": {"behemothName": "b", "behemothAssetPath": "/Game/Monsters/b.b_C"},
        "mapList": [{"mapName": "m", "mapAssetPath": map, "biome": "EHuntBiome::Moss", "weighting": 1}],
        "gatherableTier": "EGatherableTier::Tier1", "maxPlayers": 4
    })
}

fn export(root: &Path, players: &[Value], matchmakers: &[Value], version: &str) -> PathBuf {
    let dir = root.join("hunts_1_14_7");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("hunt_export_manifest.json"),
        json!({"gameVersion": version, "changelist": 647472}).to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("player_hunts.jsonl"), lines(players)).unwrap();
    std::fs::write(dir.join("matchmaker_hunts.jsonl"), lines(matchmakers)).unwrap();
    dir
}

fn vendor(root: &Path) -> PathBuf {
    let dir = root.join("vendor");
    std::fs::create_dir_all(&dir).unwrap();
    let player = json!([{"Name": "player_hunts_table", "Type": "DataTable", "Rows": {
        "Old_A": {"HuntName": {"CultureInvariantString": "x"}, "MatchmakingGameType": 3, "MatchmakerHuntIDs": []},
        "Old_C": {"HuntName": {"CultureInvariantString": "x"}, "HuntTags": ["Hunt.A"], "_importedFrom": "CatalogExporter EXPORT_HUNTS 1.12.0"}
    }}]);
    std::fs::write(dir.join("player_hunts_table.json"), player.to_string()).unwrap();
    let main = json!([{"Name": "matchmaker_hunts_table", "Type": "DataTable", "Rows": {}}]);
    std::fs::write(dir.join("matchmaker_hunts_table.json"), main.to_string()).unwrap();
    dir
}

fn options(export_dir: PathBuf, vendor_dir: PathBuf) -> ImportHuntTablesOptions {
    ImportHuntTablesOptions {
        export_dir,
        vendor_dir,
        repo_root: None,
        apply: true,
        update_existing: false,
        verbose: false,
        backup_stamp: Some("2026-10-28T00-00-00-000Z".into()),
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn an_export_is_merged_and_written() {
    let root = tempfile::tempdir().unwrap();
    let players = [player("Old_C", json!(3)), player("New_B", json!(9))];
    let matchmakers = [matchmaker("MM_New", "/Game/Maps/new.new"), {
        let mut row = matchmaker("Arena_X", "/Game/Maps/a.a");
        row["sourceTable"] = json!("arena_easy_matchmaker_hunts");
        row
    }];
    let export_dir = export(root.path(), &players, &matchmakers, "1.14.7");
    let vendor_dir = vendor(root.path());
    let report = import_hunt_tables(&options(export_dir, vendor_dir.clone())).unwrap();
    assert!(
        report.conversion_problems.is_empty(),
        "{:?}",
        report.conversion_problems
    );
    let names: Vec<&str> = report.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "player_hunts_table",
            "matchmaker_hunts_table",
            "arena_easy_matchmaker_hunts"
        ]
    );
    assert!(report.output.contains("Export: "), "{}", report.output);
    assert!(
        report
            .output
            .contains("(1.14.7, CL 647472): 2 player rows, 2 matchmaker rows")
    );
    assert!(
        report
            .output
            .contains("Backed up: player_hunts_table.json.2026-10-28T00-00-00-000Z.bak")
    );
    assert!(report.output.contains("  all direct MatchmakerHuntIDs resolve"));
    assert!(
        vendor_dir
            .join("player_hunts_table.json.2026-10-28T00-00-00-000Z.bak")
            .is_file()
    );
    let written = read(&vendor_dir.join("player_hunts_table.json"));
    let rows = written[0]["Rows"].as_object().unwrap();
    // Added after the nearest earlier row of the export that the file has
    let order: Vec<&String> = rows.keys().collect();
    assert_eq!(order, ["Old_A", "Old_C", "New_B"]);
    assert_eq!(rows["New_B"]["MatchmakingGameType"], "EMatchmakingGameType::Gauntlet");
    assert_eq!(rows["New_B"]["MatchmakingType"], 1);
    assert_eq!(rows["New_B"]["RecomendedWeaponSkillLevel"], 3);
    let keys: Vec<&String> = rows["New_B"].as_object().unwrap().keys().collect();
    assert_eq!(keys.first().map(|k| k.as_str()), Some("HuntName"));
    assert_eq!(keys.last().map(|k| k.as_str()), Some("_importedFrom"));
    // Changed but not written without --update-existing
    assert!(rows["Old_C"].get("Region").is_none());
    assert!(vendor_dir.join("arena_easy_matchmaker_hunts.json").is_file());
    let text = std::fs::read_to_string(vendor_dir.join("player_hunts_table.json")).unwrap();
    assert!(text.starts_with("[\n  {\n    \"Name\": \"player_hunts_table\"") && !text.ends_with('\n'));

    let mut update = options(root.path().join("hunts_1_14_7"), vendor_dir.clone());
    update.update_existing = true;
    update.backup_stamp = Some("second".into());
    let report = import_hunt_tables(&update).unwrap();
    assert!(report.output.contains("game-type names 1"), "{}", report.output);
    let rows = read(&vendor_dir.join("player_hunts_table.json"))[0]["Rows"].clone();
    assert_eq!(rows["Old_C"]["_importedFrom"], "CatalogExporter EXPORT_HUNTS 1.14.7");
    assert!(rows["Old_C"].get("Region").is_some());
    assert_eq!(rows["Old_C"]["HuntName"]["CultureInvariantString"], "x");
    assert_eq!(
        rows["Old_A"]["MatchmakingGameType"],
        "EMatchmakingGameType::HuntingGround"
    );
}

#[test]
fn refusals_and_dry_runs() {
    let root = tempfile::tempdir().unwrap();
    let vendor_dir = vendor(root.path());
    let error = import_hunt_tables(&options(root.path().join("none"), vendor_dir.clone())).unwrap_err();
    assert!(error.starts_with("No export found at ") && error.ends_with("inject into the CLIENT first."));
    let export_dir = export(root.path(), &[player("P", json!(3))], &[], "1.13.0");
    let error = import_hunt_tables(&options(export_dir.clone(), vendor_dir.clone())).unwrap_err();
    assert!(error.contains("declares gameVersion '1.13.0'."), "{error}");
    export(
        root.path(),
        &[player("P", json!(3))],
        &[matchmaker("MM_Bad", "Engine/x.x")],
        "1.14.7",
    );
    let error = import_hunt_tables(&options(export_dir.clone(), vendor_dir.clone())).unwrap_err();
    assert!(
        error.contains("MM_Bad: no /Game/ map path (got \"Engine/x.x\")"),
        "{error}"
    );
    assert!(error.contains("REFUSING TO WRITE: 1 matchmaker row(s) have no /Game/ map path."));
    let mut dry = options(export_dir.clone(), vendor_dir.clone());
    dry.apply = false;
    let before = std::fs::read_to_string(vendor_dir.join("player_hunts_table.json")).unwrap();
    let report = import_hunt_tables(&dry).unwrap();
    assert!(
        report
            .output
            .ends_with("Dry run complete. Nothing written. Re-run with --apply to commit.")
    );
    assert_eq!(
        std::fs::read_to_string(vendor_dir.join("player_hunts_table.json")).unwrap(),
        before
    );
    std::fs::remove_file(export_dir.join("matchmaker_hunts.jsonl")).unwrap();
    let error = import_hunt_tables(&options(export_dir, vendor_dir)).unwrap_err();
    assert_eq!(error, "Export is missing player_hunts.jsonl or matchmaker_hunts.jsonl.");
}

#[test]
fn seeded_example_rows_are_dropped_once_real_rows_arrive() {
    let root = tempfile::tempdir().unwrap();
    let export_dir = export(
        root.path(),
        &[player("New_B", json!(9))],
        &[matchmaker("MM_New", "/Game/Maps/new.new")],
        "1.14.7",
    );
    let vendor_dir = root.path().join("vendor");
    std::fs::create_dir_all(&vendor_dir).unwrap();
    let seeds = [
        ("player_hunts_table", "Example_Hunt_Easy"),
        ("matchmaker_hunts_table", "Example_MatchmakerHunt"),
        ("arena_easy_matchmaker_hunts", "Example_MatchmakerHunt"),
    ];
    for (table, row) in seeds {
        let file = json!([{"Name": table, "Type": "DataTable", "Rows": {row: {"MatchmakerHuntIDs": []}}}]).to_string();
        std::fs::write(vendor_dir.join(format!("{table}.example.json")), &file).unwrap();
        std::fs::write(vendor_dir.join(format!("{table}.json")), &file).unwrap();
    }
    let report = import_hunt_tables(&options(export_dir, vendor_dir.clone())).unwrap();
    let placeholders: Vec<&[String]> = report.tables.iter().map(|t| t.placeholders.as_slice()).collect();
    assert_eq!(
        placeholders,
        [
            &["Example_Hunt_Easy".to_owned()][..],
            &["Example_MatchmakerHunt".to_owned()][..]
        ]
    );
    assert!(report.output.contains("  placeholders    : 1 example row(s) dropped"));
    let rows = |table: &str| {
        let file = read(&vendor_dir.join(format!("{table}.json")));
        file[0]["Rows"].as_object().unwrap().keys().cloned().collect::<Vec<_>>()
    };
    assert_eq!(rows("player_hunts_table"), ["New_B"]);
    assert_eq!(rows("matchmaker_hunts_table"), ["MM_New"]);
    // The export brought no arena rows, so the placeholder stays the table's only content.
    assert_eq!(rows("arena_easy_matchmaker_hunts"), ["Example_MatchmakerHunt"]);
    assert!(vendor_dir.join("player_hunts_table.example.json").is_file());
}
