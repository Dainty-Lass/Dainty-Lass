// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::paths::Paths;
use crate::run::{RunOptions, run};
use crate::selfupdate::{
    Manifest, Source, Wanted, damaged_files, extract, fetch, find_release, install, read_manifest,
    this_platform, verify_attestation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceSource {
    Checkout,
    Release(Manifest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInstall {
    pub name: &'static str,
    pub dir: PathBuf,
    pub build_output: Option<PathBuf>,
}

pub fn service_source(p: &Paths) -> Result<ServiceSource, HostError> {
    match read_manifest(&p.repo)? {
        Some(m) => Ok(ServiceSource::Release(m)),
        None => Ok(ServiceSource::Checkout),
    }
}

#[must_use]
pub fn services_to_install(p: &Paths) -> Vec<ServiceInstall> {
    vec![
        ServiceInstall {
            name: "ParadoxBackend",
            dir: p.backend_dir.clone(),
            build_output: Some(p.backend_dir.join("build").join("server.js")),
        },
        ServiceInstall {
            name: "ParadoxDirector",
            dir: p.director_dir.clone(),
            build_output: Some(p.director_dir.join("dist").join("server.js")),
        },
    ]
}

fn npm_program() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

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
    let install_res = run(
        npm_program(),
        &[install_subcmd, "--no-audit", "--no-fund"],
        &opts,
    )?;
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

pub fn repair_release(
    p: &Paths,
    manifest: &Manifest,
    log_file: &Path,
) -> Result<usize, HostError> {
    let damaged = damaged_files(&p.repo, manifest)?;
    if damaged.is_empty() {
        return Ok(0);
    }

    let platform = this_platform()?;
    let source = Source::from_env(Some(manifest));
    let candidate = find_release(&source, platform, Wanted::Exactly(&manifest.version))
        .map_err(|e| HostError::message(e.to_string()))?;

    let tmp_dir = p.workspace.join("tmp-repair");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| HostError::io(&tmp_dir, e))?;
    let fetch_dir = tmp_dir.join("download");
    let extract_dir = tmp_dir.join("staged");

    let archive = fetch(&candidate, &fetch_dir).map_err(|e| HostError::message(e.to_string()))?;
    let _ = verify_attestation(&archive, &candidate, log_file)
        .map_err(|e| HostError::message(e.to_string()))?;
    let staged = extract(&archive, &extract_dir).map_err(|e| HostError::message(e.to_string()))?;
    let _ = install(&staged, &p.repo, None).map_err(|e| HostError::message(e.to_string()))?;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let still_damaged = damaged_files(&p.repo, manifest)?;
    if !still_damaged.is_empty() {
        return Err(HostError::message(format!(
            "release repair completed but {} file(s) are still damaged: {}",
            still_damaged.len(),
            still_damaged.join(", ")
        )));
    }
    Ok(damaged.len())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn services_to_install_returns_backend_and_director() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let svcs = services_to_install(&paths);
        assert_eq!(svcs.len(), 2);
        assert_eq!(svcs[0].name, "ParadoxBackend");
        assert_eq!(svcs[1].name, "ParadoxDirector");
    }

    #[test]
    fn service_source_detects_checkout_vs_release() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        assert_eq!(service_source(&paths).unwrap(), ServiceSource::Checkout);
    }
}
