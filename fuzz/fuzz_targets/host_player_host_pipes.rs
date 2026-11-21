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
use rustic_rifts_host::HostedPipes;

const SESSION: &str = "01582ee3-5dbc-de96-764d-4ff1866b78ab";
const X: &str = "0123456789abcdef0123456789abcdef";

fn launch() -> (Vec<String>, BTreeMap<String, String>) {
    let arguments = ["-", "8791", "/Game/Maps/HuntMap", "Behemoth_Gnasher", "Arena", "1", "127.0.0.1:8791"]
        .into_iter()
        .chain(["-server", "-log", "-nullrhi", "-MULTIHOME=127.0.0.1"])
        .map(str::to_owned)
        .collect();
    let pid = std::process::id().to_string();
    let environment = [
        ("RUSTIC_ISLES_AUTH_MODE", "HUNT_CAPABILITY".to_owned()),
        (wire::BIND_ADDRESS, "127.0.0.1".to_owned()),
        (wire::SESSION_ID, SESSION.to_owned()),
        (wire::SESSION_GENERATION, "7".to_owned()),
        (wire::CAPABILITY, format!("cap_{}", "A".repeat(39))),
        (wire::CONTROL_PIPE, format!(r"\.\pipe\RusticInternalServer.control.{X}")),
        (wire::READY_PIPE, format!(r"\.\pipe\RusticInternalServer.ready.{X}")),
        (wire::LAUNCH_ID, format!("host-{X}")),
        (wire::LAUNCHER_PID, pid),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    (arguments, environment)
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
    let ready = wire::ready_message(SESSION, "7", &format!("host-{X}"), 8791);
    assert_eq!(pipes.check_ready(rest).is_ok(), rest == ready.as_bytes());
    assert_eq!(pipes.is_fence_ack(rest), rest == wire::fence_ack(SESSION, "7").as_bytes());
});
