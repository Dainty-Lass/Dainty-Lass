// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Starting a realm's hosted game server (spec-runtime-r10 12.2 row 2): suspended, the runtime loaded into it by
//! absolute path with checked hashes, and killed rather than resumed half-loaded (RD9). The pipes around it are
//! `rustic_rifts_host::PlayerHostSpawner`'s.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rustic_rifts_host::{HostError, HostedProcess, LAUNCH_VARIABLES, ProcessStarter};

use crate::platform::inject::{self, InjectDll};
use crate::platform::launch::STRIPPED_PREFIXES;

/// Picks the DLLs for the game folder `win64`: the runtimes PLAY would load, with the offered hashes.
pub type RuntimeDlls = dyn Fn(&Path) -> Result<Vec<InjectDll>, HostError> + Send + Sync;

/// The hosted server's environment: `environment` from `rifts-host` without any other server prefix's variables
/// the launcher inherited, plus the loader's stand-down. Only `rifts-host`'s own `RUSTIC_ISLES_` names survive.
pub fn hosted_environment(environment: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let stale = |name: &str| {
        let upper = name.to_ascii_uppercase();
        STRIPPED_PREFIXES.iter().any(|prefix| upper.starts_with(prefix)) && !LAUNCH_VARIABLES.contains(&name)
    };
    let mut env: BTreeMap<String, String> = environment
        .iter()
        .filter(|(name, _)| !stale(name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    env.insert(inject::INJECTED_VAR.to_owned(), "1".to_owned());
    env
}

/// The launcher's [`ProcessStarter`].
pub struct InjectingStarter {
    dlls: Box<RuntimeDlls>,
}

impl std::fmt::Debug for InjectingStarter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InjectingStarter").finish_non_exhaustive()
    }
}

impl InjectingStarter {
    /// A starter loading what `dlls` picks.
    pub fn new(dlls: Box<RuntimeDlls>) -> Self {
        Self { dlls }
    }
}

fn failed(what: &str) -> impl Fn(std::io::Error) -> HostError + '_ {
    move |error| HostError::Io(format!("{what}: {error}"))
}

impl ProcessStarter for InjectingStarter {
    fn start(
        &self,
        program: &Path,
        arguments: &[String],
        working_dir: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<Box<dyn HostedProcess>, HostError> {
        if !inject::INJECTS {
            return Err(HostError::Transport(
                "realm hosting starts its server on Windows only".to_owned(),
            ));
        }
        let dlls = (self.dlls)(working_dir)?;
        if dlls.is_empty() {
            return Err(HostError::Transport(
                "no runtime to load into the hosted server".to_owned(),
            ));
        }
        let mut command = Command::new(program);
        command
            .args(arguments)
            .current_dir(working_dir)
            .env_clear()
            .envs(hosted_environment(environment))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Suspended for the DLLs (RD9); no console: its log is the runtime's file
            const CREATE_SUSPENDED: u32 = 0x4;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(failed("the hosted server did not start"))?;
        if let Err(error) = load(&child, &dlls) {
            // Never resumed: a server without its runtime must not run
            let _ = child.kill();
            let _ = child.wait();
            return Err(HostError::Transport(format!(
                "the runtime was not loaded into the hosted server: {error}"
            )));
        }
        Ok(Box::new(Hosted {
            pid: child.id(),
            child: Mutex::new(child),
        }))
    }
}

#[cfg(windows)]
fn load(child: &Child, dlls: &[InjectDll]) -> Result<(), inject::InjectError> {
    use std::os::windows::io::AsRawHandle;
    dainty_inject::load_and_resume(child.as_raw_handle(), child.id(), dlls)
}

#[cfg(not(windows))]
fn load(_: &Child, _: &[InjectDll]) -> Result<(), inject::InjectError> {
    Err(inject::InjectError::Unsupported)
}

struct Hosted {
    pid: u32,
    child: Mutex<Child>,
}

impl Hosted {
    fn child(&self) -> MutexGuard<'_, Child> {
        self.child.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl HostedProcess for Hosted {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn has_exited(&self) -> bool {
        !matches!(self.child().try_wait(), Ok(None))
    }

    fn wait_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.has_exited() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn kill(&self) {
        let mut child = self.child();
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosted_environment_keeps_only_rifts_host_names_and_the_stand_down() {
        let given: BTreeMap<String, String> = [
            ("PATH", "p"),
            ("SystemRoot", r"C:\Windows"),
            ("RUSTIC_ISLES_SESSION_ID", "s"),
            ("RUSTIC_ISLES_BACKEND_URL", "https://127.0.0.1:8443"),
            ("RUSTIC_ISLES_OTHER", "stale"),
            ("rustic_isles_session_id", "stale case"),
            ("DAINTY_BACKEND_URL", "stale"),
            ("DAINTY_RUNTIME_INJECTED", "0"),
            ("RUSTIC_RIFTS_ALLOW_DAMAGE_DEBUG", "1"),
            ("MYSTICPARADOX_RUNTIME_PROFILE", "development"),
            ("REFORGED_BACKEND_URL", "stale"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
        let env = hosted_environment(&given);
        let names: Vec<&str> = env.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "DAINTY_RUNTIME_INJECTED",
                "PATH",
                "RUSTIC_ISLES_BACKEND_URL",
                "RUSTIC_ISLES_SESSION_ID",
                "SystemRoot"
            ]
        );
        assert_eq!(env["DAINTY_RUNTIME_INJECTED"], "1");
    }

    #[test]
    fn a_hosted_launch_carries_exactly_what_the_runtime_reads() {
        use rustic_rifts_host::{
            HostedPipes, HostedRole, LaunchEnvironmentSpec, build_launch_environment, host_server_arguments,
        };
        use rustic_rifts_seam::Secret;

        let x = "0123456789abcdef0123456789abcdef";
        let spec = LaunchEnvironmentSpec {
            capability: Secret::new(format!("cap_{}", "A".repeat(39))),
            role: HostedRole::Hunt,
            session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap(),
            generation: 3,
            launch_id: format!("host-{x}"),
            ready_pipe: format!(r"\\.\pipe\RusticInternalServer.ready.{x}"),
            control_pipe: format!(r"\\.\pipe\RusticInternalServer.control.{x}"),
            guard_pipe: format!(r"\\.\pipe\RusticHuntGuard.events.{x}"),
            hunt_evidence: false,
            guard_completion_mode: "FIRST_DEFEAT".into(),
            runtime_profile: "production".into(),
            launcher_pid: std::process::id(),
            max_players: None,
            hub_afk_seconds: None,
            trials_week: None,
            backend_url: "https://127.0.0.1:8443".into(),
        };
        // The launcher's own shell, stale names included
        let inherited = [
            ("PATH", "p"),
            ("DAINTY_BACKEND_URL", "http://stale"),
            ("RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG", "1"),
            ("RUSTIC_ISLES_GAMESERVER_KEY", "stale"),
            ("MYSTICPARADOX_HOSTED_ROLE", "HUB"),
            ("RUSTIC_RIFTS_SESSION_ID", "stale"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        let env = hosted_environment(&build_launch_environment(&spec, inherited));
        let ours: Vec<&str> = env
            .keys()
            .map(String::as_str)
            .filter(|name| name.to_ascii_uppercase().starts_with("RUSTIC_") || name.starts_with("MYSTIC"))
            .collect();
        assert!(ours.iter().all(|name| LAUNCH_VARIABLES.contains(name)), "{ours:?}");
        assert!(!env.contains_key("DAINTY_BACKEND_URL"));
        assert_eq!(env["RUSTIC_ISLES_BACKEND_URL"], "https://127.0.0.1:8443");
        let arguments = host_server_arguments(8791, "/Game/Maps/HuntMap", "Behemoth_Gnasher", "Arena_Hunt", "1");
        let pipes = HostedPipes::from_launch(Path::new("Dauntless.exe"), &arguments, &env).unwrap();
        assert_eq!(pipes.port(), 8791);
    }

    #[cfg(not(windows))]
    #[test]
    fn off_windows_nothing_starts() {
        let starter = InjectingStarter::new(Box::new(|_| panic!("asked for DLLs off Windows")));
        let refused = starter.start(Path::new("/bin/true"), &[], Path::new("/"), &BTreeMap::new());
        assert!(refused.is_err());
    }
}
