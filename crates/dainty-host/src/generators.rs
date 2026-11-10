// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! hostkit's `gamedata` steps, run in process by the 1.14.7 module's generators (spec-rd27-host §2.1).

use dainty_hostkit::gamedata::Step;

use crate::HostError;
use crate::paths::Paths;
use crate::run::RunResult;

/// Runs `step`'s generator (spec-rd27-host §2.1).
pub(crate) fn run_generator(_paths: &Paths, step: &Step<'_>) -> Result<RunResult, HostError> {
    Err(HostError::message(format!(
        "{}: not written yet (spec-rd27-host §2.1)",
        step.generator.label()
    )))
}
