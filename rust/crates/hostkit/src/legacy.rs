// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Legacy edition (`1.4.4`) hook dispatch.

use std::path::PathBuf;

use crate::commands::start::Planned;
use crate::commands::start::binaries::Programs;
use crate::doctor::Check;
use crate::error::HostError;
use crate::hostconfig::{HostConfig, Platform, Secrets};
use crate::paths::Paths;

pub fn write_env_files(
    c: &HostConfig,
    s: &Secrets,
    p: &Paths,
    source_commit: &str,
    platform: Platform,
) -> Result<Vec<PathBuf>, HostError> {
    crate::hooks::get().legacy_write_env_files(c, s, p, source_commit, platform)
}

pub fn env_file_names(c: &HostConfig) -> Vec<String> {
    crate::hooks::get().legacy_env_file_names(c)
}

pub fn legacy_service_plan(
    paths: &Paths,
    config: &HostConfig,
    external_allowlist: bool,
    programs: &Programs,
) -> Result<Vec<Planned>, HostError> {
    crate::hooks::get().legacy_service_plan(paths, config, external_allowlist, programs)
}

pub fn prepare(paths: &Paths, config: &HostConfig) -> Result<(), HostError> {
    crate::hooks::get().legacy_prepare(paths, config)
}

pub fn create_invite_code(paths: &Paths) -> Result<String, HostError> {
    crate::hooks::get().legacy_create_invite_code(paths)
}

pub fn doctor_checks(p: &Paths, config: &HostConfig, hash_exe: bool) -> Result<Vec<Check>, HostError> {
    crate::hooks::get().legacy_doctor_checks(p, config, hash_exe)
}
