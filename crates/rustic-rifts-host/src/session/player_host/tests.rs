// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rustic_isles_runtime_core::player_host as wire;
use rustic_rifts_seam::{Secret, SessionId};

use super::*;
use crate::session::{HostedRole, LaunchEnvironmentSpec, build_launch_environment, host_server_arguments};

const PORT: u16 = 8791;

fn session() -> SessionId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap()
}

fn token() -> String {
    // A fresh suffix per test, so tests running at once never share a pipe
    crate::session::folder::random_hex::<16>().unwrap()
}

/// What `LocalRealmHosting::start_host` builds for a hunt, with `x` as the random part.
fn launch(x: &str) -> (PathBuf, Vec<String>, BTreeMap<String, String>) {
    let spec = LaunchEnvironmentSpec {
        capability: Secret::new(format!("cap_{}", "A".repeat(39))),
        role: HostedRole::Hunt,
        session_id: session(),
        generation: 7,
        launch_id: format!("host-{x}"),
        ready_pipe: format!(r"\\.\pipe\RusticInternalServer.ready.{x}"),
        control_pipe: format!(r"\\.\pipe\RusticInternalServer.control.{x}"),
        guard_pipe: format!(r"\\.\pipe\RusticHuntGuard.events.{x}"),
        hunt_evidence: false,
        guard_completion_mode: "FIRST_DEFEAT".into(),
        runtime_profile: "production".into(),
        launcher_pid: std::process::id(),
        max_players: None,
        hub_afk_seconds: None,
        trials_week: None,
        backend_url: "https://127.0.0.1:8443".into(),
    };
    let arguments = host_server_arguments(PORT, "/Game/Maps/HuntMap", "Behemoth_Gnasher", "Arena_Hunt", "1");
    let program = PathBuf::from(r"C:\Games\Dauntless\Dauntless.exe");
    (program, arguments, build_launch_environment(&spec, []))
}

fn pipes(x: &str) -> HostedPipes {
    let (program, arguments, environment) = launch(x);
    HostedPipes::from_launch(&program, &arguments, &environment).unwrap()
}

fn refusal(program: &Path, arguments: &[String], environment: &BTreeMap<String, String>) -> String {
    match HostedPipes::from_launch(program, arguments, environment) {
        Err(HostError::InvalidInput(text)) => text,
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn the_launch_hosting_builds_is_one_the_runtime_hosts() {
    let x = "0123456789abcdef0123456789abcdef";
    let pipes = pipes(x);
    assert_eq!(pipes.port(), PORT);
    let wire_session = crate::session::wire_session_id(&session());
    assert_eq!(
        pipes.ready_message(),
        format!(
            "RIS1 READY session={wire_session} generation=7 launch=host-{x} port={PORT} bind=127.0.0.1
"
        )
    );
    assert_eq!(
        pipes.fence_request(),
        format!(
            "RIS1 FENCE session={wire_session} generation=7
"
        )
    );
}

#[test]
fn the_wire_is_the_runtimes_codec_both_ways() {
    let x = token();
    let pipes = pipes(&x);
    let wire_session = crate::session::wire_session_id(&session());
    // What the runtime writes passes; what the launcher writes is what the runtime accepts
    let ready = wire::ready_message(&wire_session, "7", &format!("host-{x}"), PORT);
    assert_eq!(pipes.check_ready(ready.as_bytes()), Ok(()));
    assert!(wire::is_fence(pipes.fence_request().as_bytes(), &wire_session, "7"));
    assert!(pipes.is_fence_ack(wire::fence_ack(&wire_session, "7").as_bytes()));
    assert!(!pipes.is_fence_ack(wire::fence_ack(&wire_session, "8").as_bytes()));
    assert!(!pipes.is_fence_ack(pipes.fence_request().as_bytes()));
}

#[test]
fn ready_messages_are_refused_unless_exact() {
    let x = token();
    let pipes = pipes(&x);
    let good = pipes.ready_message();
    let refused = |bytes: &[u8]| pipes.check_ready(bytes).unwrap_err();
    assert_eq!(
        refused(format!("{}\r\n", good.trim_end()).as_bytes()),
        ReadyRefusal::Mismatch
    );
    assert_eq!(refused(good.trim_end().as_bytes()), ReadyRefusal::NotOneLine);
    assert_eq!(refused(format!("{good}{good}").as_bytes()), ReadyRefusal::NotOneLine);
    assert_eq!(
        refused(good.replace("generation=7", "generation=8").as_bytes()),
        ReadyRefusal::Mismatch
    );
    assert_eq!(
        refused(good.replace("port=8791", "port=8792").as_bytes()),
        ReadyRefusal::Mismatch
    );
    assert_eq!(
        refused(good.replace("bind=127.0.0.1", "bind=0.0.0.0").as_bytes()),
        ReadyRefusal::Mismatch
    );
    assert_eq!(refused(good.replace("RIS1", "MYS1").as_bytes()), ReadyRefusal::Mismatch);
    assert_eq!(refused(&[b'a'; 512]), ReadyRefusal::TooLong);
    assert_eq!(refused(b""), ReadyRefusal::NotOneLine);
    assert!(!pipes.is_fence_ack(&[b'a'; 512]));
}

#[test]
fn a_launch_the_runtime_would_refuse_is_never_started() {
    let x = token();
    let (program, arguments, environment) = launch(&x);
    let with = |name: &str, value: &str| {
        let mut changed = environment.clone();
        changed.insert(name.to_owned(), value.to_owned());
        changed
    };
    let without = |name: &str| {
        let mut changed = environment.clone();
        changed.remove(name);
        changed
    };
    let other_pid = (std::process::id() + 1).to_string();
    let cases = [
        (with(wire::BIND_ADDRESS, "0.0.0.0"), wire::BIND_ADDRESS),
        (with(wire::LAUNCHER_PID, &other_pid), wire::LAUNCHER_PID),
        (
            with(wire::READY_PIPE, &format!(r"\\.\pipe\MysticInternalServer.ready.{x}")),
            wire::READY_PIPE,
        ),
        (with(wire::CAPABILITY, "short"), wire::CAPABILITY),
        (without(wire::CONTROL_PIPE), wire::CONTROL_PIPE),
        (with(wire::HOSTED_ROLE, "hub"), wire::problems::ROLE_INVALID),
    ];
    for (changed, label) in cases {
        let text = refusal(&program, &arguments, &changed);
        assert!(text.ends_with(label), "{text}");
        // Labels only: the capability and pipe names never reach an error
        assert!(!text.contains(&x), "{text}");
    }
    let mut no_multihome = arguments.clone();
    no_multihome.retain(|argument| !argument.starts_with("-MULTIHOME"));
    assert!(refusal(&program, &no_multihome, &environment).ends_with("MULTIHOME"));
    let not_player_host = with("RUSTIC_ISLES_AUTH_MODE", "GLOBAL_GAMESERVER");
    assert!(refusal(&program, &arguments, &not_player_host).ends_with("not a player host"));
}

#[test]
fn the_launch_environment_is_exactly_what_r10_reads() {
    let x = token();
    let (_, _, environment) = launch(&x);
    let r10 = [
        wire::BIND_ADDRESS,
        wire::SESSION_ID,
        wire::SESSION_GENERATION,
        wire::CAPABILITY,
        wire::CONTROL_PIPE,
        wire::READY_PIPE,
        wire::LAUNCH_ID,
        wire::LAUNCHER_PID,
        wire::HOSTED_ROLE,
        wire::HUNT_GUARD_EVENTS,
    ];
    for name in r10 {
        assert!(environment.contains_key(name), "{name}");
    }
    // A hunt sets no hub numbers, and nothing under an old name
    assert!(!environment.contains_key(wire::MAX_PLAYERS));
    assert!(!environment.contains_key(wire::HUB_AFK_SECONDS));
    assert!(
        environment.keys().all(|name| name.starts_with("RUSTIC_ISLES_")),
        "{environment:?}"
    );
    assert!(environment.values().all(|value| !value.contains("Mystic")));
}

#[test]
fn the_debug_form_hides_the_pipes() {
    let x = token();
    let shown = format!("{:?}", pipes(&x));
    assert!(!shown.contains(&x), "{shown}");
}

#[cfg(not(windows))]
#[test]
fn off_windows_no_server_is_started() {
    struct Never;
    impl ProcessStarter for Never {
        fn start(
            &self,
            _: &Path,
            _: &[String],
            _: &Path,
            _: &BTreeMap<String, String>,
        ) -> Result<Box<dyn HostedProcess>, HostError> {
            panic!("started off Windows")
        }
    }
    let (program, arguments, environment) = launch(&token());
    let spawner = PlayerHostSpawner::new(Arc::new(Never));
    let refused = spawner.spawn(&program, &arguments, Path::new("."), &environment);
    assert!(matches!(refused, Err(HostError::Io(text)) if text.contains("Windows only")));
}

#[cfg(windows)]
mod on_windows;
