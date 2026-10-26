//! A build without the `legacy` feature: every command refuses a server that runs 1.4.4 with the same message.

use std::path::PathBuf;

use crate::commands::start::Planned;
use crate::commands::start::binaries::Programs;
use crate::doctor::Check;
use crate::error::HostError;
use crate::hostconfig::{HostConfig, LEGACY_UNSUPPORTED, Platform, Secrets};
use crate::paths::Paths;

fn unsupported() -> HostError {
    HostError::message(LEGACY_UNSUPPORTED)
}

pub fn write_env_files(
    _c: &HostConfig,
    _s: &Secrets,
    _p: &Paths,
    _source_commit: &str,
    _platform: Platform,
) -> Result<Vec<PathBuf>, HostError> {
    Err(unsupported())
}

pub fn env_file_names(_c: &HostConfig) -> Vec<String> {
    Vec::new()
}

pub fn legacy_service_plan(
    _paths: &Paths,
    _config: &HostConfig,
    _external_allowlist: bool,
    _programs: &Programs,
) -> Result<Vec<Planned>, HostError> {
    Err(unsupported())
}

pub fn prepare(_paths: &Paths, _config: &HostConfig) -> Result<(), HostError> {
    Err(unsupported())
}

pub fn create_invite_code(_paths: &Paths) -> Result<String, HostError> {
    Err(unsupported())
}

pub fn doctor_checks(_p: &Paths, _config: &HostConfig, _hash_exe: bool) -> Result<Vec<Check>, HostError> {
    Err(unsupported())
}
