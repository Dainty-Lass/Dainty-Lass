// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::process::ExitCode;

use crate::cli::DoctorArgs;
use crate::doctor::{DoctorOptions, print_checks, run_doctor};
use crate::error::HostError;
use crate::paths::Paths;

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
    #![allow(clippy::unwrap_used)]

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
        assert!(lines.iter().any(|l| l.contains("host.toml is missing")));
    }
}
