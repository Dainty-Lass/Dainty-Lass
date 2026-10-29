#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use isles_director::hunt_tables::{ImportHuntTablesOptions, import_hunt_tables};
use serde_json::json;
use tempfile::TempDir;

fn director_ref_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("1.14.7")
        .join("ParadoxDirector")
}

fn setup_director_tree(root: &Path) {
    let scripts_dir = root.join("ParadoxDirector").join("scripts");
    let game_data_dir = root.join("ParadoxDirector").join("game-data");
    let exports_dir = root.join("Items_Analysis").join("hunts_1_14_7");
    std::fs::create_dir_all(&scripts_dir).unwrap();
    std::fs::create_dir_all(&game_data_dir).unwrap();
    std::fs::create_dir_all(&exports_dir).unwrap();

    let ref_dir = director_ref_dir();
    std::fs::copy(
        ref_dir.join("scripts").join("import_hunt_tables.cjs"),
        scripts_dir.join("import_hunt_tables.cjs"),
    )
    .unwrap();

    for entry in std::fs::read_dir(ref_dir.join("game-data")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(".example.json") {
            std::fs::copy(entry.path(), game_data_dir.join(format!("{stem}.json"))).unwrap();
        }
    }

    std::fs::write(
        exports_dir.join("hunt_export_manifest.json"),
        json!({
            "gameVersion": "1.14.7",
            "changelist": 647472
        })
        .to_string(),
    )
    .unwrap();

    let player_rows = [
        json!({
            "rowName": "ShatteredIsles_IslandS",
            "region": { "dataTable": "regions_table", "rowName": "Region_IslandS" },
            "matchmakerHuntIDs": [
                { "dataTable": "matchmaker_hunts_table", "rowName": "MM_IslandS_01" }
            ],
            "matchmakerHuntsByTag": [],
            "matchmakingType": 1,
            "matchmakingGameType": 3,
            "huntTags": ["Hunt.IslandS"],
            "escalationModeSpecification": { "dataTable": null, "rowName": "None" },
            "escalationPatrolInitialChallengeLevel": 0,
            "escalationPatrolMaxChallengeLevel": 0,
            "recommendedEscalationLevel": 0,
            "hasPortals": false,
            "hasGlitterEvent": false,
            "hasPhaelanxEvent": true,
            "huntSuccessReward": "Reward_IslandS_Success",
            "firstCompletionSuccessReward": "None",
            "targetedHuntSuccessReward": "None",
            "huntFailureReward": "None",
            "targetedHuntFailureReward": "None",
            "minWeaponSkillLevel": 14,
            "recommendedWeaponSkillLevel": 18,
            "visibleWhileLocked": true
        }),
        json!({
            "rowName": "Gauntlet_Season_01",
            "region": { "dataTable": "regions_table", "rowName": "Region_Gauntlet" },
            "matchmakerHuntIDs": [
                { "dataTable": "matchmaker_hunts_table", "rowName": "MM_Gauntlet_01" }
            ],
            "matchmakerHuntsByTag": [],
            "matchmakingType": 0,
            "matchmakingGameType": 9,
            "huntTags": ["Hunt.Gauntlet"],
            "escalationModeSpecification": { "dataTable": null, "rowName": "None" },
            "hasPortals": false,
            "hasGlitterEvent": false,
            "hasPhaelanxEvent": false,
            "minWeaponSkillLevel": 1,
            "recommendedWeaponSkillLevel": 10,
            "visibleWhileLocked": true
        }),
    ];
    let player_jsonl = player_rows
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(exports_dir.join("player_hunts.jsonl"), player_jsonl).unwrap();

    let mm_rows = [
        json!({
            "rowName": "MM_IslandS_01",
            "sourceTable": "matchmaker_hunts_table",
            "region": { "dataTable": "regions_table", "rowName": "Region_IslandS" },
            "huntTags": ["Hunt.IslandS"],
            "huntThreatLevel": 18,
            "gameModeOverride": "None",
            "dangerPerSecOverride": 0.0,
            "specificBehemoth": {
                "behemothName": "Phaelanx",
                "behemothAssetPath": "/Game/Behemoths/Phaelanx/BP_Phaelanx.BP_Phaelanx_C",
                "powerOverride": 550.0,
                "weighting": 1.0
            },
            "additionalSpecificBehemoths": [],
            "mapMetaData": { "dataTable": null, "rowName": "None" },
            "mapList": [
                {
                    "mapName": "Island_S",
                    "mapAssetPath": "/Game/Maps/Islands/Island_S/Island_S_Main.Island_S_Main",
                    "biome": 2,
                    "weighting": 1.0
                }
            ],
            "modifiers": ["Modifier_Fire"],
            "gatherableTier": 3,
            "maxPlayers": 6,
            "isGeneratedEncounter": true,
            "gameModeSpecificData": null
        }),
        json!({
            "rowName": "MM_Gauntlet_01",
            "sourceTable": "matchmaker_hunts_table",
            "region": { "dataTable": "regions_table", "rowName": "Region_Gauntlet" },
            "huntTags": ["Hunt.Gauntlet"],
            "huntThreatLevel": 12,
            "gameModeOverride": "None",
            "dangerPerSecOverride": 0.5,
            "specificBehemoth": {
                "behemothName": "",
                "behemothAssetPath": "None",
                "powerOverride": 0.0,
                "weighting": 1.0
            },
            "additionalSpecificBehemoths": [],
            "mapMetaData": { "dataTable": null, "rowName": "None" },
            "mapList": [
                {
                    "mapName": "Gauntlet_Arena",
                    "mapAssetPath": "/Game/Maps/Gauntlet/Gauntlet_Arena.Gauntlet_Arena",
                    "biome": 0,
                    "weighting": 1.0
                }
            ],
            "modifiers": [],
            "gatherableTier": 0,
            "maxPlayers": 4,
            "isGeneratedEncounter": false,
            "gameModeSpecificData": null
        }),
        json!({
            "rowName": "Arena_Hard_New_01",
            "sourceTable": "arena_hard_matchmaker_hunts_new",
            "region": { "dataTable": "regions_table", "rowName": "Region_Arena" },
            "huntTags": ["Hunt.Trials.Normal"],
            "huntThreatLevel": 17,
            "gameModeOverride": "None",
            "dangerPerSecOverride": 0.0,
            "specificBehemoth": {
                "behemothName": "Shrowd",
                "behemothAssetPath": "/Game/Behemoths/Shrowd/BP_Shrowd.BP_Shrowd_C",
                "powerOverride": 500.0,
                "weighting": 1.0
            },
            "additionalSpecificBehemoths": [],
            "mapMetaData": { "dataTable": null, "rowName": "None" },
            "mapList": [
                {
                    "mapName": "Arena_Island",
                    "mapAssetPath": "/Game/Maps/Arena/Arena_Island.Arena_Island",
                    "biome": 0,
                    "weighting": 1.0
                }
            ],
            "modifiers": [],
            "gatherableTier": 0,
            "maxPlayers": 4,
            "isGeneratedEncounter": false,
            "gameModeSpecificData": null
        }),
    ];
    let mm_jsonl = mm_rows.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(exports_dir.join("matchmaker_hunts.jsonl"), mm_jsonl).unwrap();
}

#[test]
fn import_hunt_tables_matches_node_script_byte_for_byte() {
    let rust_dir = TempDir::new().unwrap();
    let node_dir = TempDir::new().unwrap();
    setup_director_tree(rust_dir.path());
    setup_director_tree(node_dir.path());

    let report = import_hunt_tables(&ImportHuntTablesOptions {
        export_dir: rust_dir.path().join("Items_Analysis").join("hunts_1_14_7"),
        vendor_dir: rust_dir.path().join("ParadoxDirector").join("game-data"),
        repo_root: Some(rust_dir.path().to_path_buf()),
        apply: true,
        update_existing: false,
        verbose: false,
        backup_stamp: Some("2026-10-28T00-00-00-000Z".to_owned()),
    })
    .unwrap();
    assert!(report.conversion_problems.is_empty());

    let has_node = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_node {
        for table in &report.tables {
            let file_name = format!("{}.json", table.table_name);
            assert!(
                rust_dir
                    .path()
                    .join("ParadoxDirector/game-data")
                    .join(&file_name)
                    .exists()
            );
        }
        return;
    }

    let script = node_dir
        .path()
        .join("ParadoxDirector")
        .join("scripts")
        .join("import_hunt_tables.cjs");
    let out = Command::new("node").arg(&script).arg("--apply").output().unwrap();
    assert!(
        out.status.success(),
        "node import_hunt_tables.cjs failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    for table in &report.tables {
        let file_name = format!("{}.json", table.table_name);
        let rust_bytes = std::fs::read(rust_dir.path().join("ParadoxDirector/game-data").join(&file_name)).unwrap();
        let node_bytes = std::fs::read(node_dir.path().join("ParadoxDirector/game-data").join(&file_name)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&rust_bytes),
            String::from_utf8_lossy(&node_bytes),
            "mismatch in {file_name}"
        );
    }
}
