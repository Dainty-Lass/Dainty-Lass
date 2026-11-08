//! dainty-launcher-kit's game config and game folder rules (spec §2.6, §2.7.6, §2.7.7), as the shell's
//! `platform/game_config_tests.rs` tested them: DR's recorded table (`test/cases/game-config.json`), and
//! Reforged's `launch.test.ts` (Engine.ini and Game.ini) and `display-repair.test.ts` cases, with the ini files in
//! temporary folders.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    reason = "a panic is how a test fails"
)]

mod common;

use dainty_launcher_kit::bridge::types::GraphicsPreset;
use dainty_launcher_kit::platform::{game_config as kit, game_folder as kit_folder};
use serde_json::Value;

/// The shell's `repair_display_settings`: the window repair alone, the graphics left to the game.
fn display(lines: &[String], force: bool) -> (Vec<String>, bool) {
    let edit = kit::repair_user_settings(lines, force, GraphicsPreset::UNSET);
    (edit.lines, edit.repaired)
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn graphics(value: &Value) -> GraphicsPreset {
    GraphicsPreset::try_from(value.as_i64().unwrap()).unwrap()
}

fn lines(list: &[&str]) -> Vec<String> {
    list.iter().map(|line| (*line).to_owned()).collect()
}

fn preset(level: i8) -> GraphicsPreset {
    GraphicsPreset::try_from(i64::from(level)).unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect()
}

/// DR's recorded table (`test/cases/game-config.json`: GameUserSettings.ini, the encodings, the display repair and
/// the game folder rules on both platforms) against dainty-launcher-kit.
#[test]
fn the_shared_table() {
    let cases = common::cases("game-config.json");
    let mut wrong: Vec<String> = Vec::new();
    let mut check = |ok: bool, case: &Value, got: String| {
        if !ok {
            wrong.push(format!(
                "{case}
  got: {got}"
            ));
        }
    };
    for case in cases["gus"].as_array().unwrap() {
        let edit = kit::repair_user_settings(&strings(&case["lines"]), false, graphics(&case["graphics"]));
        check(edit.lines == strings(&case["out"]), case, format!("{:?}", edit.lines));
    }
    for case in cases["split"].as_array().unwrap() {
        let got = kit::split_lines(case["text"].as_str().unwrap());
        check(got == strings(&case["lines"]), case, format!("{got:?}"));
    }
    for case in cases["encode"].as_array().unwrap() {
        let got = kit::encode(&kit::join_lines(&strings(&case["lines"])));
        check(got == bytes(&case["bytes"]), case, format!("{got:?}"));
    }
    for case in cases["decode"].as_array().unwrap() {
        let got = kit::decode(&bytes(&case["bytes"]));
        check(got == case["text"].as_str().unwrap(), case, format!("{got:?}"));
    }
    for case in cases["display"].as_array().unwrap() {
        let edit = kit::repair_user_settings(
            &strings(&case["lines"]),
            case["force"].as_bool().unwrap(),
            GraphicsPreset::UNSET,
        );
        let want = (
            strings(&case["out"]["lines"]),
            case["out"]["repaired"].as_bool().unwrap(),
        );
        check((edit.lines.clone(), edit.repaired) == want, case, format!("{edit:?}"));
    }
    for case in cases["folders"].as_array().unwrap() {
        let windows = case["platform"] == "win32";
        let input = case["input"].as_str().unwrap();
        let clean = kit_folder::normalize(input, windows);
        check(clean.as_deref() == case["clean"].as_str(), case, format!("{clean:?}"));
        if let Some(clean) = case["clean"].as_str() {
            let got = kit_folder::candidates(clean, windows);
            check(got == Some(strings(&case["candidates"])), case, format!("{got:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} cases differ:
{}",
        wrong.len(),
        wrong.join(
            "
"
        )
    );
}

/// `launch.test.ts`: "Engine.ini: chat goes to the relay, the owned sections are replaced and everything
/// else is kept"
#[test]
fn engine_ini_points_chat_at_the_relay_and_keeps_the_rest() {
    let existing = lines(&[
        "[Core.Log]",
        "LogOnline=Verbose",
        "",
        "[OnlineSubsystemMcp.XMPP]",
        "ServerAddr=\"wss://xmpp-service-prod.ol.epicgames.com\"",
        "ServerPort=443",
        "bUseSSL=true",
        "",
        "[SystemSettings]",
        "sg.ShadowQuality=0",
        "[/Script/Engine.Engine]",
        "bSmoothFrameRate=true",
    ]);
    let out = kit::edit_engine_ini(&existing, "127.0.0.1", 61100, GraphicsPreset::UNSET).unwrap();
    assert_eq!(
        out[..4],
        lines(&[
            "[OnlineSubsystemMcp.XMPP]",
            "ServerAddr=\"ws://127.0.0.1\"",
            "ServerPort=61100",
            "bUseSSL=false"
        ])
    );
    assert!(!out.iter().any(|line| line.contains("epicgames")));
    assert!(
        !out.iter().any(|line| line == "[SystemSettings]"),
        "graphics left to the game"
    );
    assert!(out.iter().any(|line| line == "LogOnline=Verbose"));
    assert!(out.iter().any(|line| line == "bSmoothFrameRate=true"));
    let forced = kit::edit_engine_ini(&existing, "127.0.0.1", 61100, preset(2)).unwrap();
    assert_eq!(forced[0], "[SystemSettings]");
    assert!(forced.iter().any(|line| line == "sg.ShadowQuality=2"));
    assert!(matches!(
        kit::edit_engine_ini(&existing, "bad host!", 1, GraphicsPreset::UNSET),
        Err(kit::ConfigError::InvalidHost)
    ));
    assert!(matches!(
        kit::edit_engine_ini(&existing, "127.0.0.1", 0, GraphicsPreset::UNSET),
        Err(kit::ConfigError::InvalidPort)
    ));
    assert_eq!(
        kit::repair_user_settings(&lines(&["sg.TextureQuality=0", "Other=1"]), false, preset(3)).lines,
        lines(&["sg.TextureQuality=3", "Other=1"])
    );
}

#[test]
fn system_settings_hold_the_forced_level_only() {
    let system_settings = |graphics| {
        let out = kit::edit_engine_ini(&[] as &[&str], "127.0.0.1", 61100, graphics).unwrap();
        let end = out.iter().position(|line| line.is_empty()).unwrap_or(0);
        let block = out[..end].to_vec();
        if block.first().is_some_and(|line| line == "[SystemSettings]") {
            block
        } else {
            Vec::new()
        }
    };
    assert!(system_settings(GraphicsPreset::UNSET).is_empty());
    assert_eq!(
        system_settings(preset(4)),
        lines(&[
            "[SystemSettings]",
            "sg.ViewDistanceQuality=4",
            "sg.AntiAliasingQuality=4",
            "sg.ShadowQuality=4",
            "sg.PostProcessQuality=4",
            "sg.TextureQuality=4",
            "sg.EffectsQuality=4",
            "sg.FoliageQuality=4",
            "sg.ShadingQuality=4",
            "sg.ResolutionQuality=100",
            "r.ScreenPercentage=100",
        ])
    );
    // A file with only owned sections on top: no blank lines are left above what is kept
    let out = kit::edit_engine_ini(
        &lines(&["[SystemSettings]", "x=1", "", "", "[Kept]", "y=2"]),
        "127.0.0.1",
        61100,
        GraphicsPreset::UNSET,
    )
    .unwrap();
    assert_eq!(
        out,
        lines(&[
            "[OnlineSubsystemMcp.XMPP]",
            "ServerAddr=\"ws://127.0.0.1\"",
            "ServerPort=61100",
            "bUseSSL=false",
            "",
            "[Kept]",
            "y=2",
        ])
    );
}

/// The shell's `ApplyConfig::new`: the relay's address, the game's graphics, Game.ini's endpoints removed.
fn config() -> kit::ConfigRequest<'static> {
    kit::ConfigRequest {
        host: "127.0.0.1",
        port: 61100,
        graphics: GraphicsPreset::UNSET,
        safe_window: false,
        clean_phoenix: true,
    }
}

fn read(path: &std::path::Path) -> String {
    kit::decode(&std::fs::read(path).unwrap())
}

/// `launch.test.ts`: "applyGameConfig writes Engine.ini and keeps a UTF-16 file readable"
#[test]
fn apply_keeps_a_utf16_file_readable() {
    let dir = tempfile::tempdir().unwrap();
    let mut data = vec![0xff, 0xfe];
    data.extend("[Custom]\r\nName=Jörð\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(dir.path().join("Engine.ini"), data).unwrap();
    let applied = kit::write_game_config(dir.path(), &config()).unwrap();
    let text = read(&applied.engine_ini);
    assert!(text.contains("ServerPort=61100"));
    assert!(text.contains("Name=Jörð"));
    assert_eq!(applied.phoenix_keys_removed, 0);
}

/// What Dauntless Revived's launcher leaves in Game.ini (local-server mode), between player sections.
const REVIVED_GAME_INI: [&str; 11] = [
    "[/Script/Archon.ArchonGameSettings]",
    "bShowHints=False",
    "",
    "[OnlineSubsystemPhoenix]",
    "AuthEndpoint=\"http://127.0.0.1:61000/auth\"",
    "CheckCrossPlayProgressionEndpoint=\"http://127.0.0.1:61000/features/platform/{platform}\"",
    "; a comment",
    "PhoenixStatusMessageEndpoint=\"http://127.0.0.1:61000/dauntless-status\"",
    "",
    "[Custom]",
    "Name=Kept",
];

/// `launch.test.ts`: "removeIniSection drops one section (any case), counts its keys and keeps the rest"
#[test]
fn remove_ini_section_drops_one_section() {
    let (out, removed) = kit::remove_phoenix(&lines(&REVIVED_GAME_INI)).unwrap();
    assert_eq!(removed, 3);
    assert_eq!(
        out,
        lines(&[
            "[/Script/Archon.ArchonGameSettings]",
            "bShowHints=False",
            "",
            "[Custom]",
            "Name=Kept"
        ])
    );
    // None: nothing to remove, the file stays as it is
    assert_eq!(kit::remove_phoenix(&lines(&["[A]", "x=1"])), None);
    assert_eq!(
        kit::remove_phoenix(&lines(&["  [OnlineSubsystemPhoenix]  ", "#x=1", "  y = 2", "[B]"])),
        Some((lines(&["[B]"]), 1))
    );
}

/// Spaces inside the brackets still name the Phoenix section (spec §2.7.6, RL9 3d58e9e2).
#[test]
fn a_phoenix_header_with_inner_spaces_is_removed() {
    assert_eq!(
        kit::remove_phoenix(&lines(&["  [ OnlineSubsystemPhoenix ]  ", "#x=1", "  y = 2", "[B]"])),
        Some((lines(&["[B]"]), 1))
    );
}

/// `launch.test.ts`: "applyGameConfig (1.14.7): Revived's 127.0.0.1 endpoints leave Game.ini, with a copy;
/// nothing else changes"
#[test]
fn apply_removes_revived_endpoints_from_game_ini() {
    let dir = tempfile::tempdir().unwrap();
    let game_ini = dir.path().join("Game.ini");
    let original: String = REVIVED_GAME_INI.iter().map(|line| format!("{line}\r\n")).collect();
    std::fs::write(&game_ini, &original).unwrap();
    let kept = kit::write_game_config(
        dir.path(),
        &kit::ConfigRequest {
            clean_phoenix: false,
            ..config()
        },
    )
    .unwrap();
    assert_eq!(kept.phoenix_keys_removed, 0);
    assert_eq!(read(&game_ini), original, "untouched when not asked");

    let applied = kit::write_game_config(dir.path(), &config()).unwrap();
    assert_eq!(applied.phoenix_keys_removed, 3);
    let text = read(&game_ini);
    assert!(!text.contains("127.0.0.1"), "no endpoint left");
    assert!(text.contains("bShowHints=False") && text.contains("Name=Kept"));
    assert!(read(&dir.path().join("Game.ini.before-reforged")).contains("CheckCrossPlayProgressionEndpoint"));

    // Already clean: not written again, and no Game.ini is made where there is none
    let again = kit::write_game_config(dir.path(), &config()).unwrap();
    assert_eq!(again.phoenix_keys_removed, 0);
    let empty = tempfile::tempdir().unwrap();
    kit::write_game_config(empty.path(), &config()).unwrap();
    assert!(!empty.path().join("Game.ini").exists());
    // No temporary file is left behind, whatever its name
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|name| !name.contains("tmp")), "{names:?}");
}

#[test]
fn apply_writes_both_files_and_keeps_a_backup_of_a_repaired_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Engine.ini"),
        "[Core.System]\r\nPaths=x\r\n[SystemSettings]\r\nold=1\r\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("GameUserSettings.ini"),
        "[/Script/Engine.GameUserSettings]\r\nResolutionSizeX=40000\r\nsg.ShadowQuality=1\r\n",
    )
    .unwrap();
    let options = kit::ConfigRequest {
        graphics: preset(2),
        ..config()
    };
    let applied = kit::write_game_config(dir.path(), &options).unwrap();
    assert!(applied.display_repaired);
    let text = std::fs::read_to_string(applied.engine_ini).unwrap();
    assert!(
        text.starts_with("[SystemSettings]\r\n")
            && text.contains("ServerAddr=\"ws://127.0.0.1\"\r\nServerPort=61100\r\n")
            && text.ends_with("[Core.System]\r\nPaths=x\r\n"),
        "{text}"
    );
    assert!(!text.contains("old=1"));
    let settings = std::fs::read_to_string(dir.path().join("GameUserSettings.ini")).unwrap();
    assert!(
        settings.contains("ResolutionSizeX=1280") && settings.contains("sg.ShadowQuality=2"),
        "{settings}"
    );
    let backups = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("before-display-repair")
        })
        .count();
    assert_eq!(backups, 1);
}

/// `display-repair.test.ts`: "FuturePoet oversized viewport recovers to windowed 720p and preserves
/// unrelated preferences"
#[test]
fn an_oversized_viewport_recovers_to_a_720p_window() {
    let before = lines(&[
        "[/Script/Archon.ArchonGameUserSettings]",
        "ResolutionSizeX=33424",
        "ResolutionSizeY=828",
        "FullscreenMode=1",
        "Volume=0.7",
        "[Other]",
        "ResolutionSizeX=99999",
    ]);
    let (fixed, repaired) = display(&before, false);
    assert!(repaired);
    for line in [
        "ResolutionSizeX=1280",
        "FullscreenMode=2",
        "Volume=0.7",
        "ResolutionSizeX=99999",
    ] {
        assert!(fixed.iter().any(|l| l == line), "{line}");
    }
    assert!(!display(&fixed, false).1);
}

/// `display-repair.test.ts`: "valid widescreen preferences are unchanged"
#[test]
fn valid_widescreen_preferences_are_unchanged() {
    let before = lines(&[
        "[/Script/Engine.GameUserSettings]",
        "ResolutionSizeX=5120",
        "ResolutionSizeY=1440",
    ]);
    assert_eq!(display(&before, false), (before, false));
}

/// `display-repair.test.ts`: "Poet fix overrides valid or missing saved settings and stays idempotent"
#[test]
fn the_safe_window_overrides_any_saved_size_once() {
    for before in [
        Vec::new(),
        lines(&[
            "[/Script/Archon.ArchonGameUserSettings]",
            "ResolutionSizeX=1920",
            "ResolutionSizeY=1080",
            "FullscreenMode=1",
            "WindowPosX=33423",
            "Volume=0.7",
        ]),
    ] {
        let (fixed, repaired) = display(&before, true);
        assert!(repaired);
        for line in ["ResolutionSizeX=1280", "FullscreenMode=2", "WindowPosX=-1"] {
            assert!(fixed.iter().any(|l| l == line), "{line}");
        }
        if before.iter().any(|l| l == "Volume=0.7") {
            assert!(fixed.iter().any(|l| l == "Volume=0.7"));
        }
        assert_eq!(display(&fixed, true), (fixed.clone(), false));
    }
}
