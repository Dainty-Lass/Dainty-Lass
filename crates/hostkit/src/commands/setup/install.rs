// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Installing the services setup runs: checking or repairing a release's files, or building a checkout's.

use std::path::{Path, PathBuf};

use crate::commands::setup::paradox::Layout;
use crate::error::HostError;
use crate::paths::Paths;
use crate::run::{RunOptions, run};
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

/// One Node service to install in a checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInstall {
    /// Its name.
    pub name: &'static str,
    /// Its folder.
    pub dir: PathBuf,
    /// What its build writes, checked after it.
    pub build_output: Option<PathBuf>,
}

/// The reference backend's and Director's folders, with their names.
#[must_use]
pub fn services(l: &Layout) -> [(&Path, &'static str); 2] {
    [(&l.backend_dir, "backend"), (&l.director_dir, "director")]
}

/// A release when the server folder has a release.json, else a checkout.
pub fn service_source(p: &Paths) -> Result<ServiceSource, HostError> {
    match read_manifest(&p.repo)? {
        Some(m) => Ok(ServiceSource::Release(m)),
        None => Ok(ServiceSource::Checkout),
    }
}

/// The reference backend and Director, as a `paradox` build installs them.
#[must_use]
pub fn services_to_install(p: &Paths) -> Vec<ServiceInstall> {
    vec![
        ServiceInstall {
            name: "backend",
            dir: p.backend_dir.clone(),
            build_output: Some(p.backend_dir.join("build").join("server.js")),
        },
        ServiceInstall {
            name: "director",
            dir: p.director_dir.clone(),
            build_output: Some(p.director_dir.join("dist").join("server.js")),
        },
    ]
}

fn npm_program() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

/// Runs `npm ci` (or `npm install`) and the build in the service's folder, logging to `log_file`.
pub fn install_service(svc: &ServiceInstall, log_file: &Path) -> Result<(), HostError> {
    let opts = RunOptions {
        cwd: svc.dir.clone(),
        env: None,
        log_file: Some(log_file.to_path_buf()),
        quiet: true,
    };
    let install_subcmd = if svc.dir.join("package-lock.json").is_file() {
        "ci"
    } else {
        "install"
    };
    let install_res = run(npm_program(), &[install_subcmd, "--no-audit", "--no-fund"], &opts)?;
    if install_res.code != 0 {
        return Err(HostError::message(format!(
            "npm {install_subcmd} failed in {} ({})",
            svc.name,
            install_res
                .last_line
                .unwrap_or_else(|| format!("exit {}", install_res.code))
        )));
    }

    if let Some(build_out) = &svc.build_output {
        let build_res = run(npm_program(), &["run", "build"], &opts)?;
        if build_res.code != 0 {
            return Err(HostError::message(format!(
                "npm run build failed in {} ({})",
                svc.name,
                build_res
                    .last_line
                    .unwrap_or_else(|| format!("exit {}", build_res.code))
            )));
        }
        if !build_out.is_file() {
            return Err(HostError::message(format!(
                "{} build completed without producing {}",
                svc.name,
                build_out.display()
            )));
        }
    }
    Ok(())
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
