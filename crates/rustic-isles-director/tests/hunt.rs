// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hunt tables and the resolver (spec §9, §14.2): picks fixed to the first candidate.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::{Arc, Mutex};

use rustic_isles_director::hub::Level;
use rustic_isles_director::hunt::{self, HuntResolver, ResolverOptions, Tables};
use serde_json::{Map, Value, json};

fn rows(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn handle(row: &str) -> Value {
    json!({"DataTable": {"ObjectName": "DataTable'matchmaker_hunts_table'", "ObjectPath": "x"}, "RowName": row})
}

fn mm(map: &str, behemoth: &str) -> Value {
    json!({"HuntTags": [], "MapList": [{"MapAssetName": map}], "SpecificBehemoth": {"BehemothAsset": {"AssetPathName": behemoth}}})
}

fn arena(map: &str) -> Value {
    json!({"HuntTags": ["Hunt.Arena.Easy.Week"], "MapList": [{"MapAssetName": map}]})
}

fn tables() -> Tables {
    let trials = json!({"MatchmakerHuntIDs": [{"DataTable": null, "RowName": "None"}], "MatchmakerHuntsByTag": [{
        "MatchmakerTable": {"ObjectName": "DataTable'arena_easy_matchmaker_hunts'", "ObjectPath": "y"},
        "HuntTags": [{"QueryTokenStream": [0, 1, 1, 1, 0], "TagDictionary": [{"TagName": "Hunt.Arena.Easy"}]}]
    }]});
    Tables {
        player_rows: rows(json!({
            "Plain_Hunt": {"MatchmakerHuntIDs": [handle("MM_Plain")]},
            "Boss_Hunt": {"MatchmakerHuntIDs": [handle("MM_Boss")]},
            "Dangling_Hunt": {"MatchmakerHuntIDs": [handle("MM_Nowhere")]},
            "NoMap_Hunt": {"MatchmakerHuntIDs": [handle("MM_NoMap")]},
            "Trials_Hunt": trials,
        })),
        matchmaker_sources: vec![
            (
                "matchmaker_hunts_table".into(),
                rows(json!({
                    "MM_Plain": mm("/Game/Maps/test/plain.plain", "None"),
                    "MM_Boss": mm("/Game/Maps/test/boss.boss", "/Game/Monsters/test/boss_bp.boss_bp_C"),
                    "MM_NoMap": mm("Engine/Maps/none", "None"),
                })),
            ),
            (
                "arena_easy_matchmaker_hunts".into(),
                rows(json!({
                    "Arena_MatchmakerHunt_Easy_001": arena("/Game/Maps/arena/a1.a1"),
                    "Arena_MatchmakerHunt_Easy_002": arena("/Game/Maps/arena/a2.a2"),
                    "Arena_MatchmakerHunt_Hard_001": {"HuntTags": ["Hunt.Arena.Hard"], "MapList": [{"MapAssetName": "/Game/Maps/arena/h.h"}]},
                })),
            ),
        ],
    }
}

type Logged = Arc<Mutex<Vec<(Level, String)>>>;

fn resolver(now: i64) -> (HuntResolver, Logged, Arc<Mutex<Vec<String>>>) {
    let logged: Logged = Arc::default();
    let missing: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut options = ResolverOptions::new(tables());
    let sink = Arc::clone(&logged);
    options.log = Arc::new(move |level, line| sink.lock().unwrap().push((level, line.to_owned())));
    options.now = Arc::new(move || now);
    options.random_int = Arc::new(|min, _| min);
    let report = Arc::clone(&missing);
    options.on_missing_hunt = Some(Arc::new(move |id| report.lock().unwrap().push(id.to_owned())));
    (HuntResolver::new(options), logged, missing)
}

const WEEK: i64 = 10_080 * 60_000;

#[test]
fn manifests_and_errors() {
    let (r, _, missing) = resolver(0);
    let manifest = r
        .resolve_manifest(Some(&json!("Plain_Hunt")), Some(&json!(["acc-1"])))
        .unwrap();
    assert_eq!(
        service_kit::jsjson::stringify(&manifest),
        r#"{"huntId":"Plain_Hunt","matchmakerHuntId":"MM_Plain","mapPath":"/Game/Maps/test/plain","behemothPath":"NO_BEHEMOTH","expectedPlayerString":"acc-1:Plain_Hunt"}"#
    );
    let boss = r
        .resolve_manifest(Some(&json!("Boss_Hunt")), Some(&json!(["acc-1", "acc-2"])))
        .unwrap();
    assert_eq!(boss["expectedPlayerString"], "acc-1:Boss_Hunt,acc-2:Boss_Hunt");
    assert_eq!(boss["behemothPath"], "/Game/Monsters/test/boss_bp.boss_bp_C");
    for id in ["No_Such_Hunt", "constructor", "__proto__"] {
        let error = r.resolve_manifest(Some(&json!(id)), Some(&json!(["a"]))).unwrap_err();
        assert_eq!(
            error,
            format!("player_hunts_table has no usable row for HuntId '{id}' (missing row or empty MatchmakerHuntIDs)")
        );
    }
    assert_eq!(*missing.lock().unwrap(), ["No_Such_Hunt", "constructor", "__proto__"]);
    assert_eq!(
        r.matchmaker_hunt_id_for("Dangling_Hunt").unwrap_err(),
        "player_hunts_table row 'Dangling_Hunt' has no resolvable 1.12 MatchmakerHuntIDs"
    );
    assert_eq!(
        r.resolve_manifest(Some(&json!("NoMap_Hunt")), Some(&json!(["a"])))
            .unwrap_err(),
        "matchmaker_hunts_table row 'MM_NoMap' has no valid /Game/ map asset path"
    );
    for party in [
        json!([]),
        json!(["1", "2", "3", "4", "5"]),
        json!([""]),
        json!(["a", 7]),
        Value::Null,
        json!("acc-1"),
    ] {
        assert_eq!(
            r.resolve_manifest(Some(&json!("No_Such_Hunt")), Some(&party))
                .unwrap_err(),
            "ExpectedPlayers must contain between one and four account IDs"
        );
    }
    assert_eq!(missing.lock().unwrap().len(), 3);
}

#[test]
fn trials_rotation() {
    let (r, _, _) = resolver(WEEK);
    let manifest = r
        .resolve_manifest(Some(&json!("Trials_Hunt")), Some(&json!(["a"])))
        .unwrap();
    assert_eq!(manifest["matchmakerHuntId"], "Arena_MatchmakerHunt_Easy_002");
    assert_eq!(manifest["mapPath"], "/Game/Maps/arena/a2");
    let (r, logged, _) = resolver(2 * WEEK);
    assert_eq!(
        r.matchmaker_hunt_id_for("Trials_Hunt").unwrap(),
        "Arena_MatchmakerHunt_Easy_001"
    );
    let logged = logged.lock().unwrap();
    assert!(logged.iter().any(|(l, t)| *l == Level::Warn && t.contains("_003")));
    assert!(
        logged
            .iter()
            .any(|(_, t)| t == "[HuntTagRouting] Trials_Hunt resolved to 2 matchmaker row(s) via tag query")
    );
}

#[test]
fn coverage_is_quiet() {
    let (r, logged, _) = resolver(0);
    let coverage = r.audit_coverage();
    assert_eq!((coverage.resolvable(), coverage.total), (4, 5));
    assert_eq!(coverage.unresolved, ["Dangling_Hunt"]);
    assert!(logged.lock().unwrap().is_empty());
    let audit = r.validate_tables();
    assert_eq!(
        audit.problems,
        [
            "Dangling_Hunt -> missing MatchmakerHunt 'MM_Nowhere'",
            "NoMap_Hunt -> MM_NoMap has no valid map"
        ]
    );
    assert_eq!((audit.direct_rows, audit.tag_routed_refs), (5, 1));
}

fn write(dir: &std::path::Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
}

#[test]
fn loading() {
    let dir = tempfile::tempdir().unwrap();
    let files = hunt::table_files(true);
    let core: Vec<String> = rustic_isles::director_files(true)
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(files, core);
    let core: Vec<String> = rustic_isles::director_files(false)
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(hunt::table_files(false), core);
    for (n, file) in files.iter().enumerate() {
        let rows = if n == 0 { r#"{"Plain_Hunt":{}}"# } else { "{}" };
        let bom = if n == 0 { "\u{feff}" } else { "" };
        write(
            dir.path(),
            file,
            &format!(r#"{bom}[{{"Name":"t","Type":"DataTable","Rows":{rows}}}]"#),
        );
    }
    let tables = hunt::load_tables(dir.path()).unwrap();
    assert_eq!((tables.player_rows.len(), tables.matchmaker_sources.len()), (1, 6));
    std::fs::remove_file(dir.path().join(&files[3])).unwrap();
    let error = hunt::load_tables(dir.path()).unwrap_err();
    assert!(error.starts_with("[hunt-manifest] Required hunt table is missing: "));
    assert!(error.ends_with("arena_hard_matchmaker_hunts.json"), "{error}");
    write(dir.path(), &files[3], r#"{"Rows":{}}"#);
    let error = hunt::load_tables(dir.path()).unwrap_err();
    assert!(error.ends_with("is not a DataTable export (no Rows)"), "{error}");
}

/// Restricted's real-table counts, only on a machine that has the game data.
#[test]
fn real_tables_when_present() {
    let Some(dir) = std::env::var_os("RUSTIC_ISLES_REAL_HUNT_TABLES") else {
        return;
    };
    let tables = hunt::load_tables(std::path::Path::new(&dir)).unwrap();
    let r = HuntResolver::new(ResolverOptions::new(tables));
    let coverage = r.audit_coverage();
    assert_eq!((coverage.resolvable(), coverage.total), (161, 168));
    assert_eq!(coverage.max_players, [4, 12]);
}
