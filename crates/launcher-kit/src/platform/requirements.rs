// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Host page prerequisite checks (Spec §2.5 & §4.2).

use std::ffi::OsString;
use std::ops::AsyncFnOnce;
use std::path::{Path, PathBuf};

use crate::bridge::types::{Requirement, RequirementId, RequirementState};
use crate::platform::hostkit::{Toolchain, any_visual_studio, system_env};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    MongoDb,
    Postgres,
    Sqlite,
}

pub const DATABASE: Database = Database::Sqlite;
pub const POSTGRES_MINIMUM: u32 = 17;

pub struct Probes<'a> {
    pub env: &'a (dyn Fn(&str) -> Option<OsString> + Sync),
    pub exists: &'a (dyn Fn(&Path) -> bool + Sync),
    pub list_dir: &'a (dyn Fn(&Path) -> Vec<String> + Sync),
}

fn system_exists(path: &Path) -> bool {
    path.is_file()
}

impl Probes<'static> {
    #[must_use]
    pub fn system() -> Self {
        Self {
            env: &system_env,
            exists: &system_exists,
            list_dir: &list_dir,
        }
    }
}

pub struct Inputs<'a> {
    pub game: (bool, Option<String>),
    pub toolchain: &'a Toolchain,
    pub dumper_dll: Option<&'a Path>,
    pub tailscale: bool,
    pub missing_vc_runtime: usize,
}

/// Lists entry names in `dir`, sorted ascending.
#[must_use]
pub fn list_dir(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

fn version_parts(text: &str) -> Vec<u32> {
    text.split('.').map(|part| part.parse::<u32>().unwrap_or(0)).collect()
}

fn program_roots(probes: &Probes<'_>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["ProgramW6432", "ProgramFiles"] {
        if let Some(val) = (probes.env)(var).filter(|v| !v.is_empty()) {
            let path = PathBuf::from(val);
            if !roots.contains(&path) {
                roots.push(path);
            }
        }
    }
    let fallback = PathBuf::from(r"C:\Program Files");
    if !roots.contains(&fallback) {
        roots.push(fallback);
    }
    roots
}

/// Searches standard Windows installation paths and `PATH` for `mongod.exe` / `mongod`.
#[must_use]
pub fn find_mongod(probes: &Probes<'_>) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) { "mongod.exe" } else { "mongod" };
    if let Some(path_var) = (probes.env)("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !dir.as_os_str().is_empty() {
                let candidate = dir.join(exe_name);
                if (probes.exists)(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }

    for root in program_roots(probes) {
        let server_dir = root.join("MongoDB").join("Server");
        let mut versions = (probes.list_dir)(&server_dir);
        versions.sort_by_key(|a| std::cmp::Reverse(version_parts(a)));
        for ver in versions {
            let candidate = server_dir.join(ver).join("bin").join("mongod.exe");
            if (probes.exists)(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

/// Searches standard installation paths and `PATH` for PostgreSQL (`pg_ctl` + `initdb`),
/// returning the `pg_ctl` path and detected major version.
#[must_use]
pub fn find_postgres(probes: &Probes<'_>) -> Option<(PathBuf, Option<u32>)> {
    for root in program_roots(probes) {
        let pg_root = root.join("PostgreSQL");
        let mut versions: Vec<(Option<u32>, String)> = (probes.list_dir)(&pg_root)
            .into_iter()
            .map(|name| {
                let major = name.split('.').next().and_then(|s| s.parse::<u32>().ok());
                (major, name)
            })
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        for (major, ver) in versions {
            let bin = pg_root.join(ver).join("bin");
            let pg_ctl = bin.join("pg_ctl.exe");
            let initdb = bin.join("initdb.exe");
            if (probes.exists)(&pg_ctl) && (probes.exists)(&initdb) {
                return Some((pg_ctl, major));
            }
        }
    }

    let (ctl_name, init_name) = if cfg!(windows) {
        ("pg_ctl.exe", "initdb.exe")
    } else {
        ("pg_ctl", "initdb")
    };
    if let Some(path_var) = (probes.env)("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !dir.as_os_str().is_empty() {
                let pg_ctl = dir.join(ctl_name);
                let initdb = dir.join(init_name);
                if (probes.exists)(&pg_ctl) && (probes.exists)(&initdb) {
                    return Some((pg_ctl, None));
                }
            }
        }
    }

    None
}

/// Evaluates the 6 host prerequisites in order (`Dauntless`, `Tailscale`, `VisualStudio`, `Rust`, `Dumper7`, `VcRedist`).
pub async fn check_requirements(
    inputs: &Inputs<'_>,
    probes: &Probes<'_>,
    any_vs: impl AsyncFnOnce() -> Option<String>,
) -> Vec<Requirement> {
    let dauntless = Requirement {
        id: RequirementId::Dauntless,
        state: if inputs.game.0 {
            RequirementState::Ok
        } else {
            RequirementState::Missing
        },
        detail: inputs.game.1.clone(),
        problem: None,
    };

    let tailscale = Requirement {
        id: RequirementId::Tailscale,
        state: if inputs.tailscale {
            RequirementState::Ok
        } else {
            RequirementState::Missing
        },
        detail: None,
        problem: None,
    };

    let visual_studio = if let Some(vs) = &inputs.toolchain.visual_studio {
        Requirement {
            id: RequirementId::VisualStudio,
            state: RequirementState::Ok,
            detail: Some(vs.clone()),
            problem: None,
        }
    } else if let Some(vs_name) = any_vs().await {
        Requirement {
            id: RequirementId::VisualStudio,
            state: RequirementState::Problem,
            detail: Some(vs_name),
            problem: Some("no_cpp".to_owned()),
        }
    } else {
        Requirement {
            id: RequirementId::VisualStudio,
            state: RequirementState::Missing,
            detail: None,
            problem: None,
        }
    };

    let rust = Requirement {
        id: RequirementId::Rust,
        state: if inputs.toolchain.cargo.is_some() {
            RequirementState::Ok
        } else {
            RequirementState::Missing
        },
        detail: None,
        problem: None,
    };

    let dumper7 = if let Some(dumper) = inputs.dumper_dll.filter(|p| (probes.exists)(p)) {
        let detail = dumper
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .or_else(|| Some(dumper.to_string_lossy().into_owned()));
        Requirement {
            id: RequirementId::Dumper7,
            state: RequirementState::Ok,
            detail,
            problem: None,
        }
    } else {
        Requirement {
            id: RequirementId::Dumper7,
            state: RequirementState::Missing,
            detail: None,
            problem: None,
        }
    };

    let vc_redist = Requirement {
        id: RequirementId::VcRedist,
        state: if inputs.missing_vc_runtime == 0 {
            RequirementState::Ok
        } else {
            RequirementState::Missing
        },
        detail: None,
        problem: None,
    };

    vec![dauntless, tailscale, visual_studio, rust, dumper7, vc_redist]
}

/// Evaluates the 6 host prerequisites against the current system.
pub async fn check_requirements_here(inputs: &Inputs<'_>) -> Vec<Requirement> {
    check_requirements(inputs, &Probes::system(), any_visual_studio).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_six_requirements_in_order_without_mongodb_or_git() {
        let toolchain = Toolchain {
            visual_studio: None,
            major_version: None,
            msbuild: None,
            cargo: Some(PathBuf::from(r"C:\Users\Host\.cargo\bin\cargo.exe")),
        };
        let dumper = PathBuf::from(r"C:\Tools\Dumper-7.dll");
        let inputs = Inputs {
            game: (false, Some("wrong_version".to_owned())),
            toolchain: &toolchain,
            dumper_dll: Some(&dumper),
            tailscale: true,
            missing_vc_runtime: 2,
        };
        let probes = Probes {
            env: &|_| None,
            exists: &|p| p == Path::new(r"C:\Tools\Dumper-7.dll"),
            list_dir: &|_| Vec::new(),
        };

        let list = check_requirements(&inputs, &probes, async || {
            Some("Visual Studio Community 2022".to_owned())
        })
        .await;

        assert_eq!(list.len(), 6);
        assert_eq!(list[0].id, RequirementId::Dauntless);
        assert_eq!(list[0].state, RequirementState::Missing);
        assert_eq!(list[0].detail.as_deref(), Some("wrong_version"));

        assert_eq!(list[1].id, RequirementId::Tailscale);
        assert_eq!(list[1].state, RequirementState::Ok);

        assert_eq!(list[2].id, RequirementId::VisualStudio);
        assert_eq!(list[2].state, RequirementState::Problem);
        assert_eq!(list[2].problem.as_deref(), Some("no_cpp"));

        assert_eq!(list[3].id, RequirementId::Rust);
        assert_eq!(list[3].state, RequirementState::Ok);

        assert_eq!(list[4].id, RequirementId::Dumper7);
        assert_eq!(list[4].state, RequirementState::Ok);
        assert_eq!(list[4].detail.as_deref(), Some("Dumper-7.dll"));

        assert_eq!(list[5].id, RequirementId::VcRedist);
        assert_eq!(list[5].state, RequirementState::Missing);
    }

    #[test]
    fn finds_newest_mongod_and_postgres_versions() {
        let probes = Probes {
            env: &|k| (k == "ProgramW6432").then(|| OsString::from(r"C:\Program Files")),
            exists: &|p| {
                matches!(
                    p.to_str(),
                    Some(
                        r"C:\Program Files\MongoDB\Server\7.0\bin\mongod.exe"
                            | r"C:\Program Files\MongoDB\Server\8.0\bin\mongod.exe"
                            | r"C:\Program Files\PostgreSQL\16\bin\pg_ctl.exe"
                            | r"C:\Program Files\PostgreSQL\16\bin\initdb.exe"
                            | r"C:\Program Files\PostgreSQL\17\bin\pg_ctl.exe"
                            | r"C:\Program Files\PostgreSQL\17\bin\initdb.exe"
                    )
                )
            },
            list_dir: &|p| match p.to_str() {
                Some(r"C:\Program Files\MongoDB\Server") => {
                    vec!["7.0".to_owned(), "8.0".to_owned()]
                }
                Some(r"C:\Program Files\PostgreSQL") => {
                    vec!["16".to_owned(), "17".to_owned()]
                }
                _ => Vec::new(),
            },
        };

        assert_eq!(
            find_mongod(&probes),
            Some(PathBuf::from(r"C:\Program Files\MongoDB\Server\8.0\bin\mongod.exe"))
        );
        assert_eq!(
            find_postgres(&probes),
            Some((
                PathBuf::from(r"C:\Program Files\PostgreSQL\17\bin\pg_ctl.exe"),
                Some(17)
            ))
        );
    }
}
