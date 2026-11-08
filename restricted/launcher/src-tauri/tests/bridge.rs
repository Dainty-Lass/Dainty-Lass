//! dainty-launcher-kit's bridge (spec §2.3, rows L6–L9, RA2) and dainty-launcher-app's command handler, as the shell's
//! `bridge/{commands,spec,types,progress,backend,generate}.rs` tested them: the argument checks, the command table,
//! the wire types' JSON, the progress throttle and channel, and the generated files.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::time::{Duration, Instant};

use dainty_launcher_kit::bridge::checks;
use dainty_launcher_kit::bridge::progress::{ProgressChannel, Throttle};
use dainty_launcher_kit::bridge::spec::{
    self, ADMIN_ACTIONS, Build, COMMANDS, COPY_TEXT_MAX, EXTERNAL_TARGETS, Field, Group, HISTORY_CURSOR_MAX,
    HISTORY_WHO_MAX, HOST_ACTIONS, HOST_FOLDERS, INSTALL_MODES, PLANNED, REALM_ID_MAX, REALM_KINDS, SERVER_MODES,
    Shape,
};
use dainty_launcher_kit::bridge::types::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use ts_rs::TS;

// ---------------------------------------------------------------- commands.rs (the argument checks)

fn name<T: DeserializeOwned>(value: Value, names: &[&str]) -> Option<T> {
    checks::name::<T>(&value, names).ok()
}

#[test]
fn names_are_only_the_exact_strings() {
    assert_eq!(
        name::<HostFolder>(json!("dumps"), HOST_FOLDERS),
        Some(HostFolder::Dumps)
    );
    assert_eq!(
        name::<ExternalTarget>(json!("dumper7_zip"), EXTERNAL_TARGETS),
        Some(ExternalTarget::Dumper7Zip)
    );
    for bad in [
        json!("Dumps"),
        json!("dumps "),
        json!("__proto__"),
        json!("https://example.com"),
        json!(7),
        json!(null),
        json!(["dumps"]),
    ] {
        assert_eq!(name::<HostFolder>(bad.clone(), HOST_FOLDERS), None, "{bad}");
    }
}

#[test]
fn detected_games_by_index_below_16() {
    let index = |value: Value| checks::detected_index(&value).ok();
    assert_eq!(index(json!(0)), Some(0));
    assert_eq!(index(json!(15)), Some(15));
    assert_eq!(index(json!(15.0)), Some(15));
    for bad in [json!(16), json!(-1), json!(1.5), json!("1"), json!(null)] {
        assert_eq!(index(bad.clone()), None, "{bad}");
    }
}

#[test]
fn the_setup_form_as_ipc_validate_checks_it() {
    let form = |value: Value| checks::host_setup(&value).ok();
    assert_eq!(
        form(json!({ "name": "  Friends  ", "mode": "private", "publicHost": "x" })),
        Some(HostSetupRequest {
            name: "Friends".into(),
            mode: HostServerMode::Private,
            public_host: None
        })
    );
    assert_eq!(
        form(json!({ "name": "Hunts", "mode": "public", "publicHost": " play.example.org " }))
            .and_then(|form| form.public_host),
        Some("play.example.org".into())
    );
    let long_host = format!("a{}a", "b".repeat(251));
    assert!(form(json!({ "name": "x", "mode": "public", "publicHost": long_host })).is_some());
    for bad in [
        json!({ "name": "", "mode": "private", "publicHost": null }),
        json!({ "name": "   ", "mode": "private", "publicHost": null }),
        json!({ "name": "a".repeat(61), "mode": "private", "publicHost": null }),
        json!({ "name": "a\u{7}b", "mode": "private", "publicHost": null }),
        json!({ "name": "x", "mode": "lan", "publicHost": null }),
        json!({ "name": "x", "mode": "public", "publicHost": null }),
        json!({ "name": "x", "mode": "public", "publicHost": "-bad.example" }),
        json!({ "name": "x", "mode": "public", "publicHost": "a b" }),
        json!({ "name": "x", "mode": "public", "publicHost": format!("a{}a", "b".repeat(252)) }),
        json!([]),
        json!(null),
    ] {
        assert_eq!(form(bad.clone()), None, "{bad}");
    }
}

#[test]
fn multipliers_from_1_to_100_rounded_and_1_1_is_off() {
    let multipliers = |value: Value| checks::multipliers(&value).map_err(drop);
    assert_eq!(multipliers(json!(null)), Ok(None));
    assert_eq!(multipliers(json!({ "xp": 1, "rewards": 1 })), Ok(None));
    assert_eq!(
        multipliers(json!({ "xp": 2.345, "rewards": 100 })),
        Ok(Some(HostMultipliers {
            xp: 2.35,
            rewards: 100.0
        }))
    );
    for bad in [
        json!({ "xp": 0.5, "rewards": 1 }),
        json!({ "xp": 101, "rewards": 1 }),
        json!({ "xp": "2", "rewards": 1 }),
        json!({ "xp": 2 }),
        json!([]),
        json!("off"),
    ] {
        assert_eq!(multipliers(bad.clone()), Err(()), "{bad}");
    }
}

#[test]
fn who_an_invite_is_for() {
    let invite_for = |value: Option<Value>| checks::invite_for(value.as_ref()).map_err(drop);
    assert_eq!(invite_for(None), Ok(None));
    assert_eq!(invite_for(Some(json!(null))), Ok(None));
    assert_eq!(invite_for(Some(json!(""))), Ok(None));
    assert_eq!(invite_for(Some(json!(" Sam "))), Ok(Some("Sam".into())));
    assert_eq!(invite_for(Some(json!("a".repeat(41)))), Err(()));
    assert_eq!(invite_for(Some(json!("a\nb"))), Err(()));
    assert_eq!(invite_for(Some(json!(7))), Err(()));
}

/// Who an invite is for is trimmed, so a blank name is nobody (spec §2.3.4, RL8 2db97a7b).
#[test]
fn a_blank_name_is_nobody() {
    assert_eq!(checks::invite_for(Some(&json!("   "))).map_err(drop), Ok(None));
}

#[test]
fn copying_keeps_the_4096_limit() {
    assert!(!checks::copy_text(&"x".repeat(COPY_TEXT_MAX + 1)));
    assert!(checks::copy_text(&"x".repeat(COPY_TEXT_MAX)));
}

/// A value of the right shape for each argument, as the page would send it.
fn good(shape: &Shape) -> Value {
    match shape {
        Shape::Object { fields, .. } => Value::Object(
            fields
                .iter()
                .map(|field| (field.name.to_owned(), good(&field.shape)))
                .collect(),
        ),
        Shape::Text { .. } | Shape::OptionalText { .. } => json!("a"),
        Shape::Flag => json!(true),
        Shape::Index { max } => json!(max),
        Shape::Name(names) => json!(names[0]),
        Shape::Number { min, .. } => json!(min),
        Shape::SettingsPatch | Shape::Channel => json!(null),
    }
}

#[test]
fn planned_commands_are_refused_with_a_code() {
    for name in PLANNED {
        let command = spec::command(name).unwrap();
        let values: Vec<Value> = command.args.iter().map(|field: &Field| good(&field.shape)).collect();
        assert!(
            command
                .args
                .iter()
                .zip(&values)
                .all(|(f, v)| spec::fits_shape(&f.shape, Some(v))),
            "{name}"
        );
        let refused = if Build::current().serves(command.group) {
            checks::not_implemented(name)
        } else {
            checks::not_in_this_build(name)
        };
        assert_eq!(serde_json::to_value(&refused).unwrap()["detail"], *name);
    }
}

#[test]
fn planned_commands_check_their_arguments() {
    let long = "a".repeat(REALM_ID_MAX + 1);
    assert!(checks::realm_id(&long).is_err());
    assert!(checks::realm_id("").is_err());
    assert!(checks::realm_id("a\nb").is_err());
    assert_eq!(
        checks::realm_id(" 01ARZ3NDEKTSV4RRFFQ69G5FAV@local ").ok(),
        Some("01ARZ3NDEKTSV4RRFFQ69G5FAV@local")
    );
    let realm_join = spec::command("realm_join").unwrap();
    assert!(!spec::fits_shape(&realm_join.args[0].shape, Some(&json!(7))));
    assert!(!spec::fits_shape(&realm_join.args[0].shape, None));
    assert!(spec::command("no_such_command").is_none());
    assert_eq!(name::<RealmKind>(json!("hub"), REALM_KINDS), Some(RealmKind::Hub));
    assert_eq!(name::<RealmKind>(json!("hunt"), REALM_KINDS), Some(RealmKind::Hunt));
    assert_eq!(name::<RealmKind>(json!("raid"), REALM_KINDS), None);
    for refused in [
        checks::realm_id(&long).unwrap_err(),
        checks::name::<RealmKind>(&json!("raid"), REALM_KINDS).unwrap_err(),
    ] {
        assert_eq!(refused.code, ErrorCode::Unknown);
    }
}

#[test]
fn history_commands_check_their_arguments() {
    let shape = |name: &str| &spec::command(name).unwrap().args[0].shape;
    let (filter, selection) = (shape("history_query"), shape("history_revert_dry_run"));
    let fine = json!({ "actor": "admin:ab12cd34", "since": "2026-10-01T00:00:00Z", "cursor": "node:5" });
    assert!(spec::fits_shape(filter, Some(&fine)));
    assert!(checks::history_filter(&fine).is_ok());
    assert!(spec::fits_shape(filter, Some(&json!({}))));
    assert!(spec::fits_shape(
        selection,
        Some(&json!({ "account": null, "until": "6h" }))
    ));
    for bad in [
        json!({ "actor": "key_01", "table": "accounts" }),
        json!({ "actor": 7 }),
        json!({ "actor": "a".repeat(HISTORY_WHO_MAX + 1) }),
        json!({ "cursor": "9".repeat(HISTORY_CURSOR_MAX + 1) }),
        json!(null),
        json!([]),
    ] {
        assert!(!spec::fits_shape(filter, Some(&bad)), "{bad}");
        if bad.is_object() {
            assert!(checks::history_filter(&bad).is_err(), "{bad}");
        }
    }
    // A revert's selection has no cursor: a dry run of one page is not a thing
    assert!(!spec::fits_shape(selection, Some(&fine)));
    assert!(checks::revert_selection(&fine).is_err());
}

/// A history filter or selection that is no object is refused (spec §2.3.4, RL8 2db97a7b).
#[test]
fn a_filter_that_is_no_object_is_refused() {
    for bad in [json!(null), json!([])] {
        assert!(checks::history_filter(&bad).is_err(), "{bad}");
        assert!(checks::revert_selection(&bad).is_err(), "{bad}");
    }
}

#[test]
fn the_player_launcher_refuses_history_as_not_in_this_build() {
    assert_eq!(
        serde_json::to_value(checks::not_in_this_build("history_query")).unwrap(),
        json!({ "code": "not_in_this_build", "detail": "history_query" })
    );
}

// ---------------------------------------------------------------- spec.rs (the command table)

#[test]
fn limits_count_like_javascript() {
    assert!(spec::fits("ab", 2));
    assert!(!spec::fits("abc", 2));
    // One code point outside the BMP is two UTF-16 units, four UTF-8 bytes
    assert!(spec::fits("\u{1F600}", 2));
    assert!(!spec::fits("\u{1F600}a", 2));
    assert!(spec::fits("ää", 2));
}

#[test]
fn server_ids_are_24_hex() {
    assert!(spec::is_server_id("0123456789abcdef01234567"));
    assert!(!spec::is_server_id("0123456789ABCDEF01234567"));
    assert!(!spec::is_server_id("0123456789abcdef0123456"));
}

/// The command names dainty-launcher-app's `generate_handler!` lists, in order.
fn served() -> Vec<&'static str> {
    let lib = include_str!("../../../../crates/dainty-launcher-app/src/lib.rs");
    lib.split_once("generate_handler![")
        .map_or("", |(_, rest)| rest)
        .split_once("])")
        .map_or("", |(list, _)| list)
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.trim_start_matches("bridge::commands::"))
        .collect()
}

#[test]
fn every_command_is_served_and_nothing_else() {
    let listed: Vec<&str> = COMMANDS.iter().map(|command| command.name).collect();
    assert_eq!(served(), listed);
}

#[test]
fn names_are_unique() {
    let mut names: Vec<_> = COMMANDS.iter().map(|command| command.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), COMMANDS.len());
}

/// A name list is exactly the enum's names: each one parses, and the TypeScript union ts-rs writes for the enum
/// has the same names in the same order.
fn same_names<T: TS + DeserializeOwned>(names: &[&str]) {
    for name in names {
        assert!(serde_json::from_value::<T>(Value::from(*name)).is_ok(), "{name}");
    }
    let union = T::inline(&ts_rs::Config::new());
    let declared: Vec<String> = union
        .split('|')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .collect();
    assert_eq!(declared, names, "{}", T::name(&ts_rs::Config::new()));
}

#[test]
fn name_lists_are_the_enums() {
    same_names::<ExternalTarget>(EXTERNAL_TARGETS);
    same_names::<HostAction>(HOST_ACTIONS);
    same_names::<HostFolder>(HOST_FOLDERS);
    same_names::<HostServerMode>(SERVER_MODES);
    same_names::<InstallMode>(INSTALL_MODES);
    same_names::<RealmKind>(REALM_KINDS);
    same_names::<AdminActionKind>(ADMIN_ACTIONS);
}

fn group(name: &str) -> Group {
    spec::command(name)
        .unwrap_or_else(|| panic!("{name} is not listed"))
        .group
}

/// The shell's `SERVER_GUI_COMMANDS` and `REALM_COMMANDS`: each listed, in its group.
#[test]
fn planned_and_server_gui_commands_are_listed() {
    for name in [
        "module_list",
        "module_get",
        "module_run_command",
        "module_set_settings",
        "module_load",
        "module_unload",
        "history_query",
        "history_revert_dry_run",
        "history_revert_apply",
        "guarded_list",
        "guarded_approve",
        "guarded_cancel",
        "admin_action",
    ] {
        assert_eq!(group(name), Group::ServerGui, "{name}");
    }
    for name in ["realm_host", "realm_join", "realm_stop", "realm_get"] {
        assert_eq!(group(name), Group::Realms, "{name}");
    }
    for name in PLANNED {
        assert!(spec::command(name).is_some(), "{name}");
    }
}

#[test]
fn each_command_names_its_build() {
    assert_eq!(group("host_get"), Group::Host);
    assert_eq!(group("module_load"), Group::ServerGui);
    assert_eq!(group("history_revert_apply"), Group::ServerGui);
    assert_eq!(group("epic_sign_in_start"), Group::Every);
    assert_eq!(group("realm_host"), Group::Realms);
    for command in COMMANDS {
        if command.group == Group::ServerGui {
            assert!(!command.name.starts_with("host_"), "{}", command.name);
        }
    }
    let build = Build::current();
    assert!(build.serves(group("play")));
    assert_eq!(build.serves(group("module_list")), cfg!(feature = "server-gui"));
    assert_eq!(build.serves(group("realm_host")), cfg!(feature = "realms"));
}

// ---------------------------------------------------------------- types.rs (the answers' JSON)

#[test]
fn guarded_outcomes_read_as_the_page_expects() {
    let info = GuardedActionInfo {
        id: "0123456789abcdef".into(),
        kind: "ban".into(),
        summary: "ban Ada: griefing".into(),
        state: GuardedState::Pending,
        mode: GuardedMode::Delay,
        initiated_by: "58ef5419".into(),
        needs_role: "moderator".into(),
        created_at: "2026-10-27T12:00:00Z".into(),
        release_at: Some("2026-10-27T12:10:00Z".into()),
        expires_at: "2026-10-29T12:10:00Z".into(),
        approved_by: None,
        canceled_by: None,
        ended_at: None,
        result: None,
    };
    let listed = serde_json::to_value(GuardedList::Ok(vec![info.clone()])).unwrap();
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["actions"][0]["initiatedBy"], "58ef5419");
    assert_eq!(listed["actions"][0]["state"], "pending");
    assert_eq!(listed["actions"][0]["releaseAt"], "2026-10-27T12:10:00Z");
    let decided = serde_json::to_value(GuardedOutcome::Ok(info)).unwrap();
    assert_eq!(decided["action"]["id"], "0123456789abcdef");
    assert_eq!(
        serde_json::to_value(GuardedOutcome::Err(LauncherError::new(ErrorCode::GuardedNotFound))).unwrap(),
        json!({ "ok": false, "error": { "code": "guarded_not_found" } })
    );
    assert_eq!(
        serde_json::to_value(GuardedList::Err(LauncherError::new(ErrorCode::GuardedNotWaiting))).unwrap(),
        json!({ "ok": false, "error": { "code": "guarded_not_waiting" } })
    );
}

#[test]
fn install_outcomes_read_as_the_page_expects() {
    let info = InstallInfo {
        version: "1.14.7".into(),
        state: InstallState::Downloading,
        folder: Some("D:/Games/Dauntless/1.14.7".into()),
        progress: None,
        error: None,
    };
    assert_eq!(
        serde_json::to_value(InstallOutcome::Ok(info)).unwrap(),
        json!({ "ok": true, "install": {
            "version": "1.14.7", "state": "downloading", "folder": "D:/Games/Dauntless/1.14.7",
            "progress": null, "error": null,
        } })
    );
    assert_eq!(
        serde_json::to_value(InstallOutcome::Err(LauncherError::new(ErrorCode::EpicNotOwned))).unwrap(),
        json!({ "ok": false, "error": { "code": "epic_not_owned" } })
    );
    assert_eq!(serde_json::to_value(TaskKind::Install).unwrap(), json!("install"));
}

#[test]
fn results_read_as_the_page_expects() {
    assert_eq!(serde_json::to_value(ActionResult::Ok).unwrap(), json!({ "ok": true }));
    assert_eq!(
        serde_json::to_value(ActionResult::err(ErrorCode::KeyInvalid)).unwrap(),
        json!({ "ok": false, "error": { "code": "key_invalid" } })
    );
    let check = InviteCheck::Ok {
        certificate_changed: true,
        previous_fingerprint: None,
        moved_from: Some("100.64.0.1:61000".into()),
    };
    assert_eq!(
        serde_json::to_value(check).unwrap(),
        json!({ "ok": true, "certificateChanged": true, "previousFingerprint": null, "movedFrom": "100.64.0.1:61000" })
    );
    let registered = RegisterOutcome::Ok {
        username: "Slayer".into(),
    };
    assert_eq!(
        serde_json::to_value(registered).unwrap(),
        json!({ "ok": true, "username": "Slayer" })
    );
    let pinged = RegionPing::Ok {
        regions: vec![
            RegionRtt {
                region: "uk".into(),
                rtt_ms: Some(18.4),
                samples: 5,
                problem: None,
            },
            RegionRtt {
                region: "oregon".into(),
                rtt_ms: None,
                samples: 0,
                problem: Some(RegionProblem::CertMismatch),
            },
        ],
        reported: true,
    };
    assert_eq!(
        serde_json::to_value(pinged).unwrap(),
        json!({ "ok": true, "regions": [
            { "region": "uk", "rttMs": 18.4, "samples": 5, "problem": null },
            { "region": "oregon", "rttMs": null, "samples": 0, "problem": "cert_mismatch" },
        ], "reported": true })
    );
    assert_eq!(
        serde_json::to_value(RegionPing::Err(LauncherError::new(ErrorCode::ServerUnreachable))).unwrap(),
        json!({ "ok": false, "error": { "code": "server_unreachable" } })
    );
    let listed = GameVersionList {
        versions: vec![
            GameVersionEntry {
                version: "1.14.7".into(),
                display_name: "Rustic Isles 1.14.7".into(),
                origin: GameVersionOrigin::Launcher,
                install: GameVersionInstall::Epic,
                epic: Some(EpicVersionInfo {
                    build_version: "1.14.7.647472".into(),
                    manifest: EpicManifestState::Ready,
                    download_bytes: Some(15_985_792_608),
                    install_bytes: Some(16_521_629_423),
                }),
            },
            GameVersionEntry {
                version: "1.4.4".into(),
                display_name: "Dauntless Revived".into(),
                origin: GameVersionOrigin::Server,
                install: GameVersionInstall::Locate,
                epic: None,
            },
        ],
    };
    assert_eq!(
        serde_json::to_value(listed).unwrap(),
        json!({ "ok": true, "versions": [
            { "version": "1.14.7", "displayName": "Rustic Isles 1.14.7", "origin": "launcher", "install": "epic",
              "epic": { "buildVersion": "1.14.7.647472", "manifest": "ready", "downloadBytes": 15_985_792_608_u64,
                        "installBytes": 16_521_629_423_u64 } },
            { "version": "1.4.4", "displayName": "Dauntless Revived", "origin": "server", "install": "locate",
              "epic": null },
        ] })
    );
    let epic = EpicView {
        signed_in: true,
        display_name: Some("Slayer".into()),
        signing_in: false,
    };
    assert_eq!(
        serde_json::to_value(epic).unwrap(),
        json!({ "signedIn": true, "displayName": "Slayer", "signingIn": false })
    );
    assert_eq!(
        serde_json::to_value(ErrorCode::EpicCodeInvalid).unwrap(),
        json!("epic_code_invalid")
    );
    let error = LauncherError::with_detail(ErrorCode::HostFailed, "setup_form");
    assert_eq!(
        serde_json::to_value(RegisterOutcome::Err(error)).unwrap(),
        json!({ "ok": false, "error": { "code": "host_failed", "detail": "setup_form" } })
    );
}

#[test]
fn module_outcomes_read_as_the_page_expects() {
    let error = LauncherError::with_detail(ErrorCode::ModuleBadState, "loaded");
    assert_eq!(
        serde_json::to_value(ModuleOutcome::Err(error.clone())).unwrap(),
        json!({ "ok": false, "error": { "code": "module_bad_state", "detail": "loaded" } })
    );
    assert_eq!(
        serde_json::to_value(ModuleListOutcome::Ok(Vec::new())).unwrap(),
        json!({ "ok": true, "modules": [] })
    );
    assert_eq!(
        serde_json::to_value(ModuleCommandOutcome::Ok {
            text: "hi".into(),
            data: Some("{\"a\":1}".into()),
        })
        .unwrap(),
        json!({ "ok": true, "text": "hi", "data": "{\"a\":1}" })
    );
    assert_eq!(
        serde_json::to_value(ModuleCommandOutcome::Ok {
            text: String::new(),
            data: None,
        })
        .unwrap(),
        json!({ "ok": true, "text": "", "data": null })
    );
    assert_eq!(
        serde_json::to_value([
            ModuleSettingValue::Flag(true),
            ModuleSettingValue::Number(2.5),
            ModuleSettingValue::Text("x".into())
        ])
        .unwrap(),
        json!([true, 2.5, "x"])
    );
    assert_eq!(
        serde_json::to_value(ModuleRunState::NotLoaded).unwrap(),
        json!("not_loaded")
    );
    assert_eq!(
        serde_json::to_value(ModuleOrigin::CompiledIn).unwrap(),
        json!("compiled_in")
    );
}

#[test]
fn names_match_the_typescript_ones() {
    assert_eq!(
        serde_json::to_value(ExternalTarget::Dumper7Zip).unwrap(),
        json!("dumper7_zip")
    );
    assert_eq!(serde_json::to_value(ExternalTarget::Dumper7).unwrap(), json!("dumper7"));
    assert_eq!(
        serde_json::to_value(ExternalTarget::ProjectDocsSdk).unwrap(),
        json!("project_docs_sdk")
    );
    assert_eq!(serde_json::to_value(GameLanguage::PtBr).unwrap(), json!("pt-BR"));
    assert_eq!(
        serde_json::to_value(ErrorCode::RuntimeDifferentOnServer).unwrap(),
        json!("runtime_different_on_server")
    );
    assert_eq!(
        serde_json::to_value(NoticeCode::RuntimeInstalledRestart).unwrap(),
        json!("runtime_installed_restart")
    );
    assert_eq!(
        serde_json::to_value(HostFolder::GamedataPacks).unwrap(),
        json!("gamedata_packs")
    );
    assert_eq!(
        serde_json::to_value(HostUpdateState::UpToDate).unwrap(),
        json!("up_to_date")
    );
    let invite = HostInvite {
        link: "x".into(),
        for_name: None,
    };
    assert_eq!(
        serde_json::to_value(invite).unwrap(),
        json!({ "link": "x", "for": null })
    );
}

#[test]
fn a_settings_patch_is_checked_whole() {
    let patch: SettingsPatch =
        serde_json::from_value(json!({ "runtimeProfile": "development", "graphics": -1, "hostTools": true })).unwrap();
    assert_eq!(patch.runtime_profile, Some(RuntimeProfile::Development));
    assert_eq!(patch.graphics, Some(GraphicsPreset::UNSET));
    assert_eq!(patch.host_tools, Some(true));
    for bad in [
        json!({ "graphics": 5 }),
        json!({ "unexpected": true }),
        json!({ "huntRegion": "us" }),
        json!({ "exposure": "basic" }),
        json!({ "language": "xx" }),
        json!({ "windowed": "yes" }),
        json!({ "runtimeProfile": "debug" }),
        json!([]),
    ] {
        assert!(serde_json::from_value::<SettingsPatch>(bad.clone()).is_err(), "{bad}");
    }
}

// ---------------------------------------------------------------- progress.rs and backend.rs

fn progress(kind: TaskKind, done: u64) -> TaskProgress {
    TaskProgress {
        kind,
        total_bytes: 100,
        done_bytes: done,
        files_total: 2,
        files_done: u32::from(done >= 100) * 2,
        bytes_per_second: 0,
        eta_seconds: None,
        current_file: None,
    }
}

#[test]
fn about_ten_a_second_but_never_a_change_that_matters() {
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    let mut throttle = Throttle::default();
    assert!(throttle.pass(&progress(TaskKind::Runtime, 1), at(0)));
    assert!(!throttle.pass(&progress(TaskKind::Runtime, 2), at(10)));
    assert!(!throttle.pass(&progress(TaskKind::Runtime, 3), at(99)));
    assert!(throttle.pass(&progress(TaskKind::Runtime, 4), at(100)));
    assert!(throttle.pass(&progress(TaskKind::Verify, 4), at(102)), "another task");
    assert!(throttle.pass(&progress(TaskKind::Verify, 100), at(103)), "the end");
    let passed = (0..1000)
        .filter(|ms| throttle.pass(&progress(TaskKind::Verify, 50), at(200 + ms)))
        .count();
    assert_eq!(passed, 10);
}

/// The shell's `Backend` over its channel, as dainty-launcher-app's is over dainty-launcher-kit's `ProgressChannel`.
#[test]
fn progress_reaches_the_subscribed_channel_throttled() {
    let mut channel: ProgressChannel<&'static str> = ProgressChannel::new();
    let now = Instant::now();
    let mut sent = 0;
    let mut push = |channel: &mut ProgressChannel<&'static str>, progress: &TaskProgress| {
        if channel.offer(progress, now).is_some() {
            sent += 1;
        }
    };
    let mut value = TaskProgress {
        kind: TaskKind::Runtime,
        total_bytes: 10,
        done_bytes: 1,
        files_total: 1,
        files_done: 0,
        bytes_per_second: 0,
        eta_seconds: None,
        current_file: None,
    };
    push(&mut channel, &value);
    assert!(!channel.is_subscribed());
    channel.subscribe("page");
    push(&mut channel, &value);
    value.done_bytes = 2;
    push(&mut channel, &value);
    value.kind = TaskKind::Verify;
    push(&mut channel, &value);
    assert_eq!(
        sent, 2,
        "nothing before the page subscribes, then throttled but never across tasks"
    );
}

// ---------------------------------------------------------------- generate.rs (the generated files)

fn generated(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../crates/dainty-launcher-kit/generated")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The generated files hold every command and type whatever the build: the manager's and the realms' too.
#[test]
fn generated_output_is_feature_independent() {
    let (ts, js, perms) = (
        generated("types.ts"),
        generated("commands.js"),
        generated("app-commands.toml"),
    );
    for command in COMMANDS {
        let name = command.name;
        assert!(ts.contains(&format!("\"{name}\"")), "types.ts missing command {name}");
        assert!(
            js.contains(&format!("\"{name}\":")),
            "commands.js missing command {name}"
        );
        assert!(
            perms.contains(&format!("\"allow-{}\"", name.replace('_', "-"))),
            "app-commands.toml missing command {name}"
        );
    }
    for ty in [
        "ModuleView",
        "HistoryEntryView",
        "RevertPlanView",
        "GuardedActionInfo",
        "RealmView",
        "RevivedInfo",
        "DoctorReportView",
        "MongoMigrationView",
        "ServerConfigView",
        "ServerInviteEntry",
        "AdminResultView",
        "ManagedInstallView",
        "RemoteStatusView",
        "RemoteInviteView",
        "RemoteExecView",
    ] {
        assert!(
            ts.contains(&format!("export type {ty} =")),
            "types.ts missing type {ty}"
        );
    }
}

#[test]
fn numbers_read_as_javascript_writes_them() {
    // The hook's limits are JavaScript numbers: no `.0`
    let js = generated("commands.js");
    assert!(
        !js.contains(".0,") && !js.contains(".0 }"),
        "a limit written as a float"
    );
    assert!(js.contains(&format!("{}", spec::KEY_TEXT_MAX)));
}
