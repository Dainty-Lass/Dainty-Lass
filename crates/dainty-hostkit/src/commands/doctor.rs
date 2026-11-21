// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host doctor`: every check printed by area, with a summary; fails when a check fails.

use std::process::ExitCode;

use crate::cli::DoctorArgs;
use crate::doctor::{DoctorOptions, print_checks, run_doctor};
use crate::error::HostError;
use crate::paths::Paths;

/// Runs every check and prints them through `say`; returns a failing exit code when a check failed.
pub fn run_with(paths: &Paths, args: &DoctorArgs, mut say: impl FnMut(&str)) -> Result<ExitCode, HostError> {
    let opts = DoctorOptions {
        hash_exe: !args.no_hash,
    };
    let checks = run_doctor(paths, &opts)?;
    let (fails, warns) = print_checks(&checks, &mut say);
    say(&format!("\nSummary: {fails} failure(s), {warns} warning(s)."));
    if fails == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

/// Runs `dainty-host doctor`, printing to stdout; an error is printed and gives a failing exit code.
#[must_use]
pub fn run(paths: &Paths, args: &DoctorArgs) -> ExitCode {
    match run_with(paths, args, |line| println!("{line}")) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn doctor_returns_failure_when_unconfigured() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let mut lines = Vec::new();
        let code = run_with(&paths, &DoctorArgs { no_hash: true }, |l| {
            lines.push(l.to_owned());
        })
        .unwrap();
        assert_eq!(code, ExitCode::FAILURE);
        assert!(lines.iter().any(|l| l.contains("settings.toml is missing")));
    }
}
