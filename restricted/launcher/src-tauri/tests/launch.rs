//! dainty-launcher-kit's game launch (spec §2.7.5, rows L31, RA9), as the shell's `platform/launch.rs` tested it: Reforged's
//! `launch.test.ts` cases (the command line, the masked key, the environment), the exit codes, and the game process.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dainty_launcher_kit::bridge::types::RuntimeProfile;
use dainty_launcher_kit::platform::launch::{
    CONSOLE_CLOSED_EXIT, GameChild, GameEvent, GameProcess, LaunchArgs, LaunchError, LaunchPlan, Spawner, describe,
    exit_code_hex, launch_args, launch_env, mask_args, missing_files, missing_files_in,
};
use dainty_launcher_kit::platform::linux_runtime::PreparedLaunch;
use futures_util::future::BoxFuture;
use tokio::sync::Notify;

const KEY: &str = concat!("DLK_", "cccccccccccccccc", "cccccccccccccccc", "cccccccccccccccc");
const USER_ID: &str = "0b7c2f6e-1d2a-4c55-9d6f-7a1e2b3c4d5e";

/// The shell's `LaunchSettings`, whose missing locale is English.
fn settings<'a>(key: &'a str, user_id: &'a str, username: &'a str, windowed: bool) -> LaunchArgs<'a> {
    LaunchArgs {
        key,
        user_id,
        username,
        windowed,
        locale: "en-US",
    }
}

/// `launch.test.ts`: "launch arguments: the 1.14.7 command line with the key as the exchange code"
#[test]
fn the_1_14_7_command_line_with_the_key_as_the_exchange_code() {
    let args = launch_args(&LaunchArgs {
        locale: "pt-BR",
        ..settings(KEY, USER_ID, "Slayer_42", false)
    })
    .unwrap();
    assert_eq!(
        args,
        [
            "-DisableEOS".to_owned(),
            "-EpicPortal".to_owned(),
            "-NoEAC".to_owned(),
            "-AUTH_TYPE=exchangecode".to_owned(),
            format!("-AUTH_LOGIN={USER_ID}"),
            format!("-AUTH_PASSWORD={KEY}"),
            "-epicapp=Archon".to_owned(),
            "-epicenv=Prod".to_owned(),
            "-epicusername=Slayer_42".to_owned(),
            format!("-epicuserid={USER_ID}"),
            format!("-epicaccountid={USER_ID}"),
            "-epicsandboxid=jackal".to_owned(),
            "-epicdeploymentid=53565ba467df4edbb6f5a3d939a8b4f2".to_owned(),
            "-epiclocale=pt-BR".to_owned(),
        ]
    );
    let windowed = launch_args(&LaunchArgs {
        locale: "xx-YY",
        ..settings(KEY, "u1", "abc", true)
    })
    .unwrap();
    assert_eq!(
        &windowed[windowed.len() - 3..],
        ["-windowed", "-ResX=1280", "-ResY=720"]
    );
    assert!(
        windowed.iter().any(|arg| arg == "-epiclocale=en-US"),
        "an unknown locale falls back to English"
    );
    assert!(launch_args(&settings("nope", "u1", "abc", false)).is_err());
    assert!(launch_args(&settings(KEY, "a b", "abc", false)).is_err());
    assert!(launch_args(&settings(KEY, "", "abc", false)).is_err());
    assert!(launch_args(&settings(KEY, &"u".repeat(129), "abc", false)).is_err());
    assert!(launch_args(&settings(KEY, "u1", "a\"b", false)).is_err());
    assert!(launch_args(&settings(KEY, "u1", "a-b", false)).is_err());
    assert!(launch_args(&settings(KEY, "u1", &"a".repeat(33), false)).is_err());
}

/// `launch.test.ts`: "the key never reaches a log line"
#[test]
fn the_key_never_reaches_a_log_line() {
    let args = launch_args(&settings(KEY, "u1", "abc", false)).unwrap();
    assert!(mask_args(&args).iter().any(|arg| arg == "-AUTH_PASSWORD=<hidden>"));
    let line = describe(Path::new("game.exe"), &args);
    assert!(!line.contains(KEY));
    assert!(line.starts_with(
        "game.exe -DisableEOS -EpicPortal -NoEAC -AUTH_TYPE=exchangecode -AUTH_LOGIN=u1 -AUTH_PASSWORD=<hidden>"
    ));
    assert_eq!(mask_args(&["-auth_password=x"]), ["-AUTH_PASSWORD=<hidden>"]);
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

/// `launch.test.ts`: "launchEnvironment: the runtime is pointed at the local relay and nothing else"
#[test]
fn the_runtime_is_pointed_at_the_local_relay_and_nothing_else() {
    let out = launch_env(
        env(&[
            ("PATH", "x"),
            ("DAINTY_BACKEND_URL", "https://evil.example"),
            ("RUSTIC_ISLES_LOG", "1"),
            ("MYSTICPARADOX_LOG", "1"),
            ("mysticparadox_backend_url", "https://evil.example"),
            ("RUSTIC_RIFTS_BIND_ADDRESS", "0.0.0.0"),
            ("reforged_other", "y"),
        ]),
        "http://127.0.0.1:61100",
        RuntimeProfile::Development,
    )
    .unwrap();
    let get = |name: &str| out.get(name).map(String::as_str);
    assert_eq!(get("DAINTY_BACKEND_URL"), Some("http://127.0.0.1:61100"));
    assert_eq!(get("RUSTIC_ISLES_RUNTIME_PROFILE"), Some("development"));
    assert_eq!(get("MYSTICPARADOX_RUNTIME_PROFILE"), Some("development"));
    assert_eq!(get("PATH"), Some("x"));
    for gone in [
        "RUSTIC_ISLES_LOG",
        "MYSTICPARADOX_LOG",
        "mysticparadox_backend_url",
        "RUSTIC_RIFTS_BIND_ADDRESS",
        "reforged_other",
    ] {
        assert_eq!(get(gone), None, "{gone}");
    }
    assert_eq!(out.len(), 4, "the inherited DAINTY_BACKEND_URL is gone, not doubled");
    for bad in [
        "http://192.168.1.2:61100",
        "https://127.0.0.1:61100",
        "http://127.0.0.1:0",
        "http://127.0.0.1:061100",
        "http://127.0.0.1:611000",
        "http://127.0.0.1:61100/",
        "http://127.0.0.1",
    ] {
        assert!(launch_env(env(&[]), bad, RuntimeProfile::Production).is_err(), "{bad}");
    }
    let production = launch_env(env(&[]), "http://127.0.0.1:1", RuntimeProfile::Production).unwrap();
    assert_eq!(
        production.get("RUSTIC_ISLES_RUNTIME_PROFILE").map(String::as_str),
        Some("production")
    );
}

#[test]
fn exit_codes() {
    assert_eq!(exit_code_hex(-1_073_741_515), "0xC0000135");
    #[allow(clippy::cast_possible_wrap, reason = "the code as Windows reports it")]
    let closed = CONSOLE_CLOSED_EXIT as i32;
    assert_eq!(exit_code_hex(closed), "0xC000013A");
    assert_eq!(exit_code_hex(1), "0x00000001");
    // The DirectX files: only XINPUT1_3 there
    let system32 = tempfile::tempdir().unwrap();
    std::fs::write(system32.path().join("XINPUT1_3.dll"), b"x").unwrap();
    assert_eq!(
        missing_files_in(system32.path()).directx,
        ["X3DAudio1_7.dll", "XAPOFX1_5.dll"]
    );
    if !cfg!(windows) {
        assert!(missing_files().is_empty(), "Wine or Proton supplies them");
    }
}

/// A stand-in game that records its plan and runs until told to exit.
struct StandIn {
    seen: Arc<Mutex<Option<LaunchPlan>>>,
    exit: Arc<Notify>,
}

struct Child(Arc<Notify>);

impl GameChild for Child {
    fn id(&self) -> Option<u32> {
        Some(42)
    }

    fn wait(self: Box<Self>) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
        Box::pin(async move {
            self.0.notified().await;
            Ok(Some(0))
        })
    }
}

impl Spawner for StandIn {
    fn spawn(&self, plan: &LaunchPlan) -> std::io::Result<Box<dyn GameChild>> {
        *self.seen.lock().unwrap() = Some(plan.clone());
        Ok(Box::new(Child(Arc::clone(&self.exit))))
    }
}

#[tokio::test]
async fn a_stand_in_gets_the_arguments_and_the_whole_environment() {
    let seen = Arc::new(Mutex::new(None));
    let exit = Arc::new(Notify::new());
    let game = GameProcess::with_spawner(Arc::new(StandIn {
        seen: Arc::clone(&seen),
        exit: Arc::clone(&exit),
    }));
    let environment = launch_env(
        env(&[("PATH", "x")]),
        "http://127.0.0.1:61100",
        RuntimeProfile::Production,
    )
    .unwrap();
    let plan = LaunchPlan::new(Path::new("win64"), None, &["-NoEAC".to_owned()], environment.clone());
    assert_eq!(game.start(&plan, |_| {}).unwrap(), Some(42));
    assert_eq!(game.pid(), Some(42));
    let again = LaunchPlan::new(Path::new("win64"), None, &[], environment.clone());
    assert!(
        matches!(game.start(&again, |_| {}), Err(LaunchError::AlreadyRunning)),
        "one game at a time"
    );
    let plan = seen.lock().unwrap().take().unwrap();
    assert_eq!(plan.working_dir, Path::new("win64"));
    assert_eq!(plan.args, ["-NoEAC"]);
    assert_eq!(plan.env, environment);
    exit.notify_one();
    for _ in 0..100 {
        if !game.is_running() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the stand-in never ended");
}

#[tokio::test]
async fn the_game_process_is_tracked_and_started_once() {
    let dir = tempfile::tempdir().unwrap();
    // Any program stands in for the game: the runtime runs it instead of the exe. It exits with the code the
    // launch environment gives it
    let (program, before): (std::path::PathBuf, Vec<&str>) = if cfg!(windows) {
        (
            dainty_launcher_kit::platform::system::system_root()
                .join("System32")
                .join("cmd.exe"),
            vec!["/C", "exit %RR_TEST_CODE%", "rem"],
        )
    } else {
        ("sh".into(), vec!["-c", "exit $RR_TEST_CODE"])
    };
    let mut environment = launch_env(std::env::vars(), "http://127.0.0.1:61100", RuntimeProfile::Production).unwrap();
    environment.insert("RR_TEST_CODE".into(), "3".into());
    let runtime = PreparedLaunch {
        program,
        args_before_exe: before.into_iter().map(Into::into).collect(),
        env: BTreeMap::new(),
        kind: dainty_launcher_kit::platform::linux_runtime::RuntimeKind::Wine,
        prefix: dir.path().to_path_buf(),
        config_dir: dir.path().to_path_buf(),
        label: "stand-in".into(),
    };
    let plan = LaunchPlan::new(dir.path(), Some(&runtime), &[], environment);
    let game = GameProcess::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    game.start(&plan, move |event| {
        let _ = tx.send(event);
    })
    .unwrap();
    assert!(
        game.start(&plan, |_| {}).is_err() || !game.is_running(),
        "refuses to start twice while running"
    );
    assert!(matches!(rx.recv().await, Some(GameEvent::Started(_))));
    assert_eq!(rx.recv().await, Some(GameEvent::Exited(Some(3))));
    assert!(!game.is_running());
}

/// The shell ran these launches through its own code and dainty-launcher-kit's side by side and found them equal; with its
/// half gone, dainty-launcher-kit's answers are held to the rules both followed.
#[test]
fn launcher_kit_builds_the_launch() {
    let key = concat!("DLK_", "0123456789abcdef", "0123456789abcdef");
    let users = [("4f1e2d3c4b5a69788796a5b4", "Hunter_01"), ("", "x"), ("abc", "")];
    for (user_id, username) in users {
        for windowed in [false, true] {
            for locale in ["en-US", "pt-BR", "xx-YY", "ja-JP"] {
                let args = launch_args(&LaunchArgs {
                    key,
                    user_id,
                    username,
                    windowed,
                    locale,
                });
                assert_eq!(
                    args.is_ok(),
                    !user_id.is_empty() && !username.is_empty(),
                    "{user_id:?} {username:?} {windowed} {locale:?}"
                );
                if let Ok(args) = args {
                    assert_eq!(args.iter().any(|a| a == "-windowed"), windowed);
                    let line = describe(Path::new("Dauntless-Win64-Shipping.exe"), &args);
                    assert!(
                        !line.contains(key) && line.contains("-AUTH_PASSWORD=<hidden>"),
                        "{line}"
                    );
                }
            }
        }
    }
    let base = [
        ("PATH", "/usr/bin"),
        ("DAINTY_BACKEND_URL", "http://evil.example"),
        ("rustic_isles_log", "1"),
        ("RUSTIC_RIFTS_BIND_ADDRESS", "0.0.0.0"),
        ("MysticParadox_x", "y"),
        ("HOME", "/home/a"),
    ];
    for url in ["http://127.0.0.1:61100", "http://127.0.0.1:0", "http://localhost:61100"] {
        for profile in [RuntimeProfile::Production, RuntimeProfile::Development] {
            let out = launch_env(base, url, profile);
            assert_eq!(out.is_ok(), url == "http://127.0.0.1:61100", "{url}");
            if let Ok(out) = out {
                let names: Vec<&str> = out.keys().map(String::as_str).collect();
                assert_eq!(
                    names,
                    [
                        "HOME",
                        "MYSTICPARADOX_RUNTIME_PROFILE",
                        "PATH",
                        "DAINTY_BACKEND_URL",
                        "RUSTIC_ISLES_RUNTIME_PROFILE"
                    ],
                    "{url}"
                );
            }
        }
    }
}
