// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the launcher reads from a hosted server's pipes (spec-runtime-r10 12.2 row 2): its launch checked against
//! the runtime's own verdict, then one ready-pipe and one control-pipe message, both untrusted. Only the exact bytes
//! the runtime's codec writes for this launch are accepted.
#![no_main]

use std::collections::BTreeMap;
use std::path::Path;

use libfuzzer_sys::fuzz_target;
use rustic_isles_runtime_core::player_host as wire;
use rustic_rifts_host::{
    HostedPipes, HostedRole, LaunchEnvironmentSpec, build_launch_environment, host_server_arguments, wire_session_id,
};
use rustic_rifts_seam::{Secret, SessionId};

const X: &str = "0123456789abcdef0123456789abcdef";
const PORT: u16 = 8791;

fn session() -> SessionId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().expect("a session id")
}

/// The launch realm hosting builds for a hunt: a hand-written one drifted from it and was refused before any input
fn launch() -> (Vec<String>, BTreeMap<String, String>) {
    let spec = LaunchEnvironmentSpec {
        capability: Secret::new(format!("cap_{}", "A".repeat(39))),
        role: HostedRole::Hunt,
        session_id: session(),
        generation: 7,
        launch_id: format!("host-{X}"),
        ready_pipe: format!(r"\\.\pipe\RusticInternalServer.ready.{X}"),
        control_pipe: format!(r"\\.\pipe\RusticInternalServer.control.{X}"),
        guard_pipe: format!(r"\\.\pipe\RusticHuntGuard.events.{X}"),
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
    (arguments, build_launch_environment(&spec, []))
}

fuzz_target!(|data: &[u8]| {
    let Some((&split, rest)) = data.split_first() else {
        return;
    };
    let (arguments, mut environment) = launch();
    // Half the inputs also overwrite one launch variable, which must never panic and never pass a bad name
    if split & 1 == 1
        && let Some((name, value)) = String::from_utf8_lossy(rest).split_once('=')
    {
        environment.insert(name.to_owned(), value.to_owned());
        if let Ok(pipes) = HostedPipes::from_launch(Path::new("Dauntless.exe"), &arguments, &environment) {
            let ready = pipes.ready_message();
            assert!(ready.starts_with("RIS1 READY ") && ready.ends_with(" bind=127.0.0.1\n"));
            assert!(ready.len() <= wire::MAX_MESSAGE);
        }
        return;
    }
    let Ok(pipes) = HostedPipes::from_launch(Path::new("Dauntless.exe"), &arguments, &environment) else {
        panic!("the fixed launch is refused");
    };
    let wire_session = wire_session_id(&session());
    let ready = wire::ready_message(&wire_session, "7", &format!("host-{X}"), PORT);
    assert_eq!(pipes.check_ready(rest).is_ok(), rest == ready.as_bytes());
    assert_eq!(pipes.is_fence_ack(rest), rest == wire::fence_ack(&wire_session, "7").as_bytes());
});
