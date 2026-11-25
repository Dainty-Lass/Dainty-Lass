// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Installing the services setup runs: checking or repairing a release's files, or building a checkout's.

use std::path::Path;

use crate::error::HostError;
use crate::paths::Paths;
use crate::selfupdate::{
    Manifest, Source, Wanted, damaged_files, extract, fetch, find_release, install, read_manifest, this_platform,
    verify_attestation,
};

/// Where the services come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceSource {
    /// A checkout: built here.
    Checkout,
    /// A release, with its manifest.
    Release(Manifest),
}

/// The 1.14.7 services a checkout builds into the server binary.
pub const BUILT_IN: [&str; 2] = ["backend", "director"];

/// A release when the server folder has a release.json, else a checkout.
pub fn service_source(p: &Paths) -> Result<ServiceSource, HostError> {
    match read_manifest(&p.repo)? {
        Some(m) => Ok(ServiceSource::Release(m)),
        None => Ok(ServiceSource::Checkout),
    }
}

/// Checks a release's files and puts back from `source` any that are missing or changed; does nothing without one.
pub fn install_from_release(
    root: &Path,
    manifest: Option<&Manifest>,
    platform: &str,
    source: &Source,
    log: &Path,
    out: &mut dyn FnMut(&str),
) -> Result<(), HostError> {
    let Some(m) = manifest else {
        return Ok(());
    };
    let damaged = damaged_files(root, m)?;
    let short = m.short_commit();
    if damaged.is_empty() {
        out(&format!(
            "- servers {} ({platform}, {short}) from the release: every file checked",
            m.version
        ));
        return Ok(());
    }

    out(&format!(
        "- {} files of servers {} are missing or changed ({}); putting them back from {}",
        damaged.len(),
        m.version,
        damaged.join(", "),
        source.describe()
    ));

    let candidate =
        find_release(source, platform, Wanted::Exactly(&m.version)).map_err(|e| HostError::message(e.to_string()))?;

    let tmp_dir = root.join(".workspace").join("update").join("repair");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| HostError::io(&tmp_dir, e))?;
    let fetch_dir = tmp_dir.join("download");
    let extract_dir = tmp_dir.join("staged");

    let archive = fetch(&candidate, &fetch_dir).map_err(|e| HostError::message(e.to_string()))?;
    out("- SHA-256 checked against SHA256SUMS.txt");
    let _ = verify_attestation(&archive, &candidate, log).map_err(|e| HostError::message(e.to_string()))?;
    let staged = extract(&archive, &extract_dir).map_err(|e| HostError::message(e.to_string()))?;
    let _ = install(&staged, root, None).map_err(|e| HostError::message(e.to_string()))?;
    let _ = std::fs::remove_dir_all(&tmp_dir);
    out(&format!("- servers {} installed", m.version));
    Ok(())
}

/// Puts back the release's damaged files; returns how many there were.
pub fn repair_release(p: &Paths, manifest: &Manifest, log_file: &Path) -> Result<usize, HostError> {
    let damaged = damaged_files(&p.repo, manifest)?;
    if damaged.is_empty() {
        return Ok(0);
    }
    let platform = this_platform()?;
    let source = Source::from_env(Some(manifest));
    install_from_release(&p.repo, Some(manifest), platform, &source, log_file, &mut |_| {})?;
    Ok(damaged.len())
}
