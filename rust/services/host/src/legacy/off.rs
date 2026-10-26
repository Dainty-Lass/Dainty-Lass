//! A build without the `legacy` feature: every command refuses a legacy server with the same message.

use std::path::PathBuf;

use crate::Outcome;
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

pub fn start(_paths: &Paths, _config: &HostConfig, _external_allowlist: bool) -> Outcome {
    Err(unsupported())
}

pub fn create_invite_code(_paths: &Paths) -> Result<String, HostError> {
    Err(unsupported())
}

pub fn doctor_checks(_p: &Paths, _config: &HostConfig, _hash_exe: bool) -> Result<Vec<Check>, HostError> {
    Err(unsupported())
}
