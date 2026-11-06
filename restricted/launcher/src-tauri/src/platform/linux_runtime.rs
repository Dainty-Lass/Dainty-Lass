//! Proton or Wine for the game on Linux (row L32) over launcher-kit's `platform::linux_runtime` (RL3), which
//! finds the runner, makes its prefix and installs what is missing. Here: the launch in the game process's
//! shape (`launch::LaunchRuntime`).

use std::path::{Path, PathBuf};

use launcher_kit::platform::linux_runtime as kit;

use super::launch::LaunchRuntime;

pub use kit::{DependencyKind, Env, InstallPlan, RuntimeMissing, current_env};

#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    pub runtime: LaunchRuntime,
    pub config_dir: PathBuf,
    pub runtime_name: String,
}

/// `auto_install_runtime` false (a development build) never installs Wine.
pub async fn prepare_linux_game_launch(
    user_data: &Path,
    env: &Env,
    _home: &Path,
    auto_install_runtime: bool,
) -> Result<PreparedLaunch, RuntimeMissing> {
    let options = kit::PrepareOptions {
        development_build: !auto_install_runtime,
    };
    let prepared = kit::prepare(env, user_data, options).await?;
    Ok(PreparedLaunch {
        runtime: LaunchRuntime {
            command: prepared.program.to_string_lossy().into_owned(),
            args_prefix: prepared
                .args_before_exe
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            env: prepared.env.into_iter().collect(),
        },
        config_dir: prepared.config_dir,
        runtime_name: prepared.label,
    })
}

/// Wine, or gnome-keyring and libsecret, through the distribution's package manager.
#[cfg_attr(
    not(target_os = "linux"),
    allow(dead_code, reason = "the secure-storage bootstrap is Linux only")
)]
pub fn install_plan(kind: DependencyKind, env: &Env) -> Option<InstallPlan> {
    kit::install_plan(env, kind)
}

#[cfg_attr(
    not(target_os = "linux"),
    allow(dead_code, reason = "the secure-storage bootstrap is Linux only")
)]
pub async fn run_install(plan: &InstallPlan, env: &Env, _cwd: &Path) -> std::io::Result<()> {
    kit::run_install(plan, &env.vars).await.map_err(std::io::Error::other)
}
