// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host update`: a newer release found, checked and installed in place, then setup rerun.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde_json::json;

use crate::cli::UpdateArgs;
use crate::error::HostError;
use crate::paths::Paths;
use crate::run::{RunOptions, run as run_cmd};
use crate::selfupdate::{
    Attestation, Candidate, Manifest, SUMS_FILE, Source, UpdateError, UpdateResult, Wanted, extract, fetch,
    find_release, install, installing_marker, read_manifest, this_platform, verify_attestation,
};

/// What `update` needs of the machine, so a test can stand in for it.
pub trait UpdateEnv {
    /// This machine's release platform; an error on one with no releases.
    fn platform(&self) -> Result<&'static str, HostError>;
    /// Where releases are looked for.
    fn source(&self, installed: Option<&Manifest>) -> Source;
    /// Checks the archive's build provenance, logging to `log`.
    fn verify_attestation(&self, archive: &Path, candidate: &Candidate, log: &Path) -> UpdateResult<Attestation>;
    /// The checkout's short commit, or `unknown`.
    fn checkout_commit(&self, root: &Path) -> String;
    /// Runs the new `program`'s `setup --skip-install` in `root`; returns its exit code.
    fn run_setup(&self, program: &Path, root: &Path) -> Result<i32, HostError>;
}

/// The real machine.
#[derive(Default)]
pub struct RealUpdateEnv;

impl UpdateEnv for RealUpdateEnv {
    fn platform(&self) -> Result<&'static str, HostError> {
        this_platform()
    }

    fn source(&self, installed: Option<&Manifest>) -> Source {
        Source::from_env(installed)
    }

    fn verify_attestation(&self, archive: &Path, candidate: &Candidate, log: &Path) -> UpdateResult<Attestation> {
        verify_attestation(archive, candidate, log)
    }

    fn checkout_commit(&self, root: &Path) -> String {
        crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(root))
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".into())
    }

    fn run_setup(&self, program: &Path, root: &Path) -> Result<i32, HostError> {
        let res = run_cmd(
            &program.to_string_lossy(),
            &["setup", "--skip-install"],
            &RunOptions {
                cwd: root.to_path_buf(),
                env: None,
                log_file: None,
                quiet: false,
            },
        )?;
        Ok(res.code)
    }
}

/// Runs `dainty-host update`, printing to stdout; an error is printed and gives a failing exit code.
#[must_use]
pub fn run(paths: &Paths, args: &UpdateArgs) -> ExitCode {
    let env = RealUpdateEnv;
    match run_with(paths, args, &env, &mut |line| println!("{line}")) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Finds the release, downloads and checks it, installs it and reruns setup unless `--no-setup`, printing through
/// `out`.
/// With `--check` it only says whether a newer release is out.
pub fn run_with(
    p: &Paths,
    args: &UpdateArgs,
    env: &dyn UpdateEnv,
    out: &mut dyn FnMut(&str),
) -> Result<ExitCode, HostError> {
    let installed = read_manifest(&p.repo)?;
    let interrupted = installing_marker(&p.repo).is_file();
    let platform = env.platform()?;
    let source = env.source(installed.as_ref());
    let installed_ver = if interrupted {
        None
    } else {
        installed.as_ref().map(|m| m.version.as_str())
    };

    let map_update_err = |err: UpdateError, staging: Option<&Path>, out: &mut dyn FnMut(&str)| -> HostError {
        if let Some(s) = staging {
            let _ = fs::remove_dir_all(s);
        }
        match err {
            UpdateError::Problem(prob) => {
                if args.json {
                    let line = json!({
                        "updated": false,
                        "error": prob.code(),
                        "message": prob.message(),
                    });
                    out(&line.to_string());
                }
                HostError::message(prob.message())
            }
            UpdateError::Host(he) => he,
        }
    };

    let candidate = match find_release(&source, platform, Wanted::Newest(installed_ver)) {
        Ok(c) => c,
        Err(e) => return Err(map_update_err(e, None, out)),
    };

    if args.check {
        if interrupted {
            out(&format!(
                "The last update stopped before it was complete: run dainty-host update to finish installing servers {} ({platform}).",
                candidate.version
            ));
            if args.json {
                out(&json!({
                    "behind": 1,
                    "head": installed.as_ref().map(|m| m.version.clone()),
                    "target": candidate.version,
                })
                .to_string());
            }
            return Ok(ExitCode::SUCCESS);
        }
        if let Some(current) = &installed {
            let up_to_date = candidate.version == current.version;
            if up_to_date {
                out(&format!("Up to date: servers {} ({platform}).", current.version));
            } else {
                out(&format!(
                    "Update available: servers {} -> {} ({platform}).",
                    current.version, candidate.version
                ));
            }
            if args.json {
                out(&json!({
                    "behind": usize::from(!up_to_date),
                    "head": current.version,
                    "target": candidate.version,
                })
                .to_string());
            }
            return Ok(ExitCode::SUCCESS);
        }
        let commit = env.checkout_commit(&p.repo);
        out(&format!(
            "This server runs from a git checkout ({commit}): update moves it to the server release {} ({platform}).",
            candidate.version
        ));
        if args.json {
            out(&json!({
                "behind": 1,
                "head": commit,
                "target": candidate.version,
            })
            .to_string());
        }
        return Ok(ExitCode::SUCCESS);
    }

    if !interrupted
        && let Some(current) = &installed
        && candidate.version == current.version
    {
        out(&format!("Up to date: servers {} ({platform}).", current.version));
        if args.json {
            out(&json!({
                "updated": false,
                "to": current.version,
            })
            .to_string());
        }
        return Ok(ExitCode::SUCCESS);
    }

    let update_dir = p.workspace.join("update");
    let staging_dir = update_dir.join("staging");
    let _ = fs::remove_dir_all(&staging_dir);
    fs::create_dir_all(&update_dir).map_err(|e| HostError::io(&update_dir, e))?;
    let log_file = p.logs.join("update.log");

    let archive = match fetch(&candidate, &update_dir) {
        Ok(a) => a,
        Err(e) => return Err(map_update_err(e, Some(&staging_dir), out)),
    };
    if let Err(e) = env.verify_attestation(&archive, &candidate, &log_file) {
        let _ = fs::remove_file(&archive);
        let _ = fs::remove_file(update_dir.join(SUMS_FILE));
        return Err(map_update_err(e, Some(&staging_dir), out));
    }
    let staged = match extract(&archive, &staging_dir) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::remove_file(&archive);
            let _ = fs::remove_file(update_dir.join(SUMS_FILE));
            return Err(map_update_err(e, Some(&staging_dir), out));
        }
    };

    let prev_for_install = if interrupted { None } else { installed.as_ref() };
    let (_manifest, info) = match install(&staged, &p.repo, prev_for_install) {
        Ok(r) => r,
        Err(e) => {
            let _ = fs::remove_file(&archive);
            let _ = fs::remove_file(update_dir.join(SUMS_FILE));
            return Err(map_update_err(e, Some(&staging_dir), out));
        }
    };
    let _ = fs::remove_file(&archive);
    let _ = fs::remove_file(update_dir.join(SUMS_FILE));
    let _ = fs::remove_dir_all(&staging_dir);

    if let Some(from) = &info.from {
        out(&format!(
            "Updated to servers {} from {from} ({} files written, {} removed).",
            info.to, info.files, info.removed
        ));
    } else {
        out(&format!(
            "Updated to servers {} ({} files written).",
            info.to, info.files
        ));
    }
    if info.runtime_changed == Some(true) {
        out("The game runtime changed: players should update their game patch before joining.");
    }
    if args.json {
        out(&json!({
            "updated": true,
            "from": info.from,
            "to": info.to,
            "files": info.files,
            "removed": info.removed,
            "runtimeChanged": info.runtime_changed,
        })
        .to_string());
    }

    if !args.no_setup {
        let configured = p.settings_toml.is_file();
        if configured {
            let host_bin = p
                .repo
                .join("bin")
                .join(format!("dainty-host{}", std::env::consts::EXE_SUFFIX));
            let code = env.run_setup(&host_bin, &p.repo)?;
            if code != 0 {
                return Err(HostError::message(format!("dainty-host setup exited with code {code}")));
            }
        } else {
            out("Now run dainty-host setup.");
        }
    }

    Ok(ExitCode::SUCCESS)
}
