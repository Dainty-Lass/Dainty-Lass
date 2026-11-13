// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The built helper against a real process: what Proton runs, minus Wine.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;

use dainty_inject::InjectDll;
use dainty_inject::helper::{EXIT_BAD_ARGS, EXIT_INJECT_FAILED};

fn system32() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into())).join("System32")
}

fn helper(dll: &InjectDll) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dainty-inject"));
    command
        .arg("--dll")
        .arg(&dll.path)
        .arg(&dll.sha256)
        .arg("--")
        .arg(system32().join("cmd.exe"))
        .args(["/c", "exit", "7"]);
    command
}

#[test]
fn the_helper_loads_the_dll_runs_the_program_and_passes_its_code_on() {
    let dll = InjectDll::as_found(&system32().join("dbghelp.dll")).unwrap();
    let status = helper(&dll).status().unwrap();
    assert_eq!(status.code(), Some(7));
}

#[test]
fn a_changed_dll_is_never_loaded_and_the_program_never_runs() {
    let mut dll = InjectDll::as_found(&system32().join("dbghelp.dll")).unwrap();
    dll.sha256 = "0".repeat(64);
    let status = helper(&dll).status().unwrap();
    assert_eq!(status.code().map(i32::cast_unsigned), Some(EXIT_INJECT_FAILED));
}

#[test]
fn a_line_that_is_not_the_helpers_is_refused() {
    let status = Command::new(env!("CARGO_BIN_EXE_dainty-inject"))
        .arg("game.exe")
        .status()
        .unwrap();
    assert_eq!(status.code().map(i32::cast_unsigned), Some(EXIT_BAD_ARGS));
}
