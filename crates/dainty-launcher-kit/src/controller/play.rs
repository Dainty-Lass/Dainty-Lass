// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! PLAY (RD27 spec §2.4.7): every check, then the relay, the game's config, its arguments and environment, and
//! the game itself. The key stays in the launcher where the server gives game tickets: the game logs in with a
//! one-time ticket instead. A runtime that carries the relay runs it inside the game, which then needs nothing
//! from the launcher once started; for an older runtime the launcher runs the relay as before.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dainty_protocol::invite::ServerMode;
use tokio_util::sync::CancellationToken;

use super::platform::{PreparedGame, RelayOwner};
use super::stored::{StoredServer, mode_name};
use super::{Controller, game_language_code};
use crate::bridge::types::{ActionResult, ErrorCode, LauncherError, RuntimeState};
use crate::core::game_language;
use crate::core::pins::EXE_NAME;
use crate::net::hostapi::{self, QueueReply, UserInfo};
use crate::net::relay::{ALTERNATE_PORTS, LogSink, PinSink, Relay, RelayConfig, RelayError};
use crate::net::relay_owner::game_owner_check;
use crate::net::runtime_files::{
    AGPL_RUNTIME_DLL, LEGACY_RUNTIME_DLL, MIT_RUNTIME_DLL, RuntimeErrorCode, install_local_runtime, set_file_slot,
};
use crate::net::verify::Abort;
use crate::platform::game_config::{ConfigRequest, write_game_config};
use crate::platform::game_folder;
use crate::platform::inject::{self, InjectDll};
use crate::platform::launch::{
    CONSOLE_CLOSED_EXIT, GameEvent, LaunchArgs, LaunchPlan, add_economy_env, exit_code_hex, launch_args, launch_env,
};
use crate::platform::loader_session::SessionKind;
use crate::platform::log;

fn error(code: ErrorCode) -> LauncherError {
    LauncherError::new(code)
}

/// What the runtime logs when it starts its relay; a runtime file without it predates the in-game relay.
const IN_GAME_RELAY_MARKER: &[u8] = b"dainty-lass:in-game-relay:v1";
/// The variables naming the server to an in-game relay (`rustic_isles_runtime_core::config::relay`).
const SERVER_HOST_VAR: &str = "DAINTY_SERVER_HOST";
const SERVER_PORT_VAR: &str = "DAINTY_SERVER_PORT";
const SERVER_FP_VAR: &str = "DAINTY_SERVER_FP";

/// Whether the runtime's bytes carry the in-game relay's marker.
fn carries_relay(bytes: &[u8]) -> bool {
    bytes
        .windows(IN_GAME_RELAY_MARKER.len())
        .any(|window| window == IN_GAME_RELAY_MARKER)
}

/// Who carries the game's connection to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayHome {
    /// The runtime, inside the game, listening on this loopback port.
    Game(u16),
    /// The launcher's own relay.
    Launcher,
}

impl Controller {
    /// `play`.
    pub async fn play(&self) -> ActionResult {
        self.play_with(None).await
    }

    /// PLAY, with CatalogExporter beside the loader for the Host page's export session.
    pub(crate) async fn play_with(&self, exporter: Option<&Path>) -> ActionResult {
        {
            let state = self.lock();
            if state.shutting_down || state.busy {
                return ActionResult::err(ErrorCode::Busy);
            }
        }
        if self.shared.game.is_running() {
            return ActionResult::err(ErrorCode::AlreadyRunning);
        }
        let Some(server) = self.server() else {
            return ActionResult::err(ErrorCode::ServerUnreachable);
        };
        if let Some(minimum) = self.launcher_too_old() {
            return ActionResult::Err(LauncherError::with_detail(ErrorCode::LauncherTooOld, &minimum));
        }
        if let Err(refused) = self.begin_busy() {
            return ActionResult::Err(refused);
        }
        let result = self.play_steps(&server, exporter).await;
        self.finish(result)
    }

    async fn play_steps(&self, server: &StoredServer, exporter: Option<&Path>) -> Result<(), LauncherError> {
        let missing = self.platform().missing_files();
        if !missing.visual_cpp.is_empty() {
            log::warn(format!("Visual C++ files missing: {}", missing.visual_cpp.join(", ")));
        }
        if !self.platform().running_clients().await.is_empty() {
            return Err(error(ErrorCode::AlreadyRunning));
        }

        let key = self.load_key().await.ok_or_else(|| error(ErrorCode::KeyInvalid))?;
        let user = match hostapi::fetch_user_info(&server.endpoint(), &key).await {
            Ok(user) => user,
            Err(refused) => {
                if matches!(refused.code, ErrorCode::KeyRejected | ErrorCode::AccountBanned) {
                    self.mark_refused();
                }
                return Err(refused);
            }
        };
        self.lock().key_refused = false;
        if !self.wait_in_line(server, &key).await? {
            return Ok(());
        }

        let win64 = self.checked_game().await?;

        let leftovers = self.loader_leftovers(&win64);
        if !leftovers.is_empty() {
            let files = leftovers.join(", ");
            log::warn(format!("refusing PLAY: loader files left beside the game: {files}"));
            return Err(LauncherError::with_detail(ErrorCode::LoaderLeftovers, &files));
        }

        if let Some(local) = self.stored().local_runtime_dir().map(PathBuf::from) {
            install_local_runtime(&local, &win64, self.runtime_files())
                .await
                .map_err(|failed| {
                    error(if failed.code == RuntimeErrorCode::InUse {
                        ErrorCode::RuntimeInUse
                    } else {
                        ErrorCode::LocalRuntimeInvalid
                    })
                })?;
        }

        self.look_at_runtime().await;
        let (playable, state) = {
            let state = self.lock();
            (state.runtime_playable, state.runtime.state)
        };
        if !playable {
            return Err(error(match state {
                RuntimeState::Untrusted => ErrorCode::RuntimeUntrusted,
                RuntimeState::Unavailable => ErrorCode::RuntimeUnavailable,
                _ => ErrorCode::RuntimeFailed,
            }));
        }

        let mut dlls = Vec::new();
        if !self.uses_loader() {
            dlls = self.runtime_dlls(&win64)?;
            if let Some(dll) = exporter {
                let found = InjectDll::as_found(dll).map_err(|failed| {
                    log::warn(format!("host: CatalogExporter not loaded: {failed}"));
                    LauncherError::with_detail(ErrorCode::HostFailed, "exporter")
                })?;
                log::info(format!("host: loading CatalogExporter, SHA-256 {}", found.sha256));
                dlls.push(found);
            }
        } else if let Some(dll) = exporter {
            let session = self.begin_loader_session(SessionKind::Export, &win64, dll)?;
            self.lock().host.loader_session = Some(session);
        }

        let home = if self.runtime_carries_relay(&win64).await {
            self.game_relay_port().await
        } else {
            self.start_relay(server).await.map(|()| RelayHome::Launcher)
        };
        let home = match home {
            Ok(home) => home,
            Err(failed) => {
                self.end_current_loader_session().await;
                return Err(failed);
            }
        };
        let login = match hostapi::fetch_game_ticket(&server.endpoint(), &key).await {
            Ok(Some(ticket)) => ticket,
            Ok(None) => {
                log::info("the server gives no game tickets: the game logs in with the account key");
                key.clone()
            }
            Err(failed) => {
                let relay = self.lock().relay.take();
                if let Some(relay) = relay {
                    relay.stop().await;
                }
                self.end_current_loader_session().await;
                return Err(failed);
            }
        };
        if let Err(failed) = self.launch(server, &win64, &login, &user, dlls, home).await {
            let relay = self.lock().relay.take();
            if let Some(relay) = relay {
                relay.stop().await;
            }
            self.end_current_loader_session().await;
            return Err(failed);
        }
        Ok(())
    }

    /// The server's login queue: true once admitted, false when the player left the line. A rate limit only
    /// skips a poll; the place is kept.
    async fn wait_in_line(&self, server: &StoredServer, key: &str) -> Result<bool, LauncherError> {
        let endpoint = server.endpoint();
        let leave = CancellationToken::new();
        self.lock().queue_leave = Some(leave.clone());
        let every = self.shared.seams.queue_poll;
        let outcome = loop {
            match hostapi::join_queue(&endpoint, key).await {
                Ok(QueueReply::Admitted) => break Ok(true),
                Ok(QueueReply::Waiting(position)) => {
                    let changed = self.lock().queue_position.replace(position) != Some(position);
                    if changed {
                        log::info(format!("waiting in the server's login queue at place {position}"));
                        self.publish();
                    }
                }
                Err(refused) if refused.code == ErrorCode::RateLimited => {}
                Err(refused) => break Err(refused),
            }
            tokio::select! {
                () = leave.cancelled() => {
                    if !hostapi::leave_queue(&endpoint, key).await {
                        log::warn("the server did not confirm leaving its login queue");
                    }
                    log::info("left the server's login queue");
                    break Ok(false);
                }
                () = tokio::time::sleep(every) => {}
            }
        };
        let mut state = self.lock();
        state.queue_position = None;
        state.queue_leave = None;
        outcome
    }

    /// The runtimes to load into the game, the MIT one first (RT-5): the installed files, with the hashes of the
    /// offered set the runtime check just matched, or as found when a local runtime or no offer is in use. Each
    /// only when the offered set lists it, or, with no offer, when the folder holds it: an R21 set is the MIT
    /// runtime alone, a pre-R21 set the pair (spec-runtime-r21 L-2).
    pub(crate) fn runtime_dlls(&self, win64: &Path) -> Result<Vec<InjectDll>, LauncherError> {
        let agpl = [AGPL_RUNTIME_DLL, LEGACY_RUNTIME_DLL]
            .iter()
            .map(|name| win64.join(name))
            .find(|path| path.is_file());
        let offered = {
            let state = self.lock();
            let local = state.stored.local_runtime_dir().is_some();
            state.manifest.as_ref().filter(|_| !local).map(|m| {
                let sha = |slot: &str| {
                    m.files
                        .iter()
                        .find(|f| set_file_slot(&f.name) == Some(slot))
                        .map(|f| f.sha256.clone())
                };
                (sha(MIT_RUNTIME_DLL), sha(AGPL_RUNTIME_DLL))
            })
        };
        let as_found = |path: &Path| {
            InjectDll::as_found(path).map_err(|failed| {
                log::warn(format!("the runtime cannot be loaded: {failed}"));
                error(ErrorCode::RuntimeFailed)
            })
        };
        let mit = win64.join(MIT_RUNTIME_DLL);
        let mut dlls = Vec::with_capacity(2);
        match &offered {
            Some((Some(sha256), _)) => dlls.push(InjectDll {
                path: mit,
                sha256: sha256.clone(),
            }),
            None if mit.is_file() => dlls.push(as_found(&mit)?),
            _ => {}
        }
        match (offered, agpl) {
            (Some((_, Some(sha256))), Some(path)) => dlls.push(InjectDll { path, sha256 }),
            (Some((_, Some(_))), None) => return Err(error(ErrorCode::RuntimeFailed)),
            (None, Some(path)) => dlls.push(as_found(&path)?),
            _ => {}
        }
        if dlls.is_empty() {
            return Err(error(ErrorCode::RuntimeFailed));
        }
        Ok(dlls)
    }

    /// The game folder's Win64 folder, once its executable is the pinned one.
    async fn checked_game(&self) -> Result<PathBuf, LauncherError> {
        let not_found = || error(ErrorCode::GameFolderNotFound);
        let root = self.lock().game.root.clone().map(PathBuf::from).ok_or_else(not_found)?;
        let win64 = self.win64().ok_or_else(not_found)?;
        let exe = game_folder::exe_path(&root);
        let hashed = {
            let mut hashes = self.shared.hashes.lock().await;
            let hashed = hashes.hash(&exe, Abort::new(), Arc::new(|_| {})).await;
            if let Err(failed) = hashes.save().await {
                log::warn(format!("could not save the hash cache: {failed}"));
            }
            hashed
        };
        match hashed {
            Ok(hash) if hash.eq_ignore_ascii_case(&self.config().exe_pin) => Ok(win64),
            Ok(_) => Err(error(ErrorCode::GameWrongVersion)),
            Err(_) => {
                self.look_at_game(false).await;
                Err(not_found())
            }
        }
    }

    /// Whether the MIT runtime the game will load runs the relay itself.
    async fn runtime_carries_relay(&self, win64: &Path) -> bool {
        match tokio::fs::read(win64.join(MIT_RUNTIME_DLL)).await {
            Ok(bytes) => tokio::task::spawn_blocking(move || carries_relay(&bytes))
                .await
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// A loopback port the in-game relay can take: the configured one, else the first alternate that binds, as the
    /// launcher's relay would choose. Any relay of the launcher's still running is stopped first.
    async fn game_relay_port(&self) -> Result<RelayHome, LauncherError> {
        let running = self.lock().relay.take();
        if let Some(relay) = running {
            relay.stop().await;
        }
        let configured = self.config().relay_port;
        let free = |port: u16| std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok();
        std::iter::once(configured)
            .chain(ALTERNATE_PORTS)
            .find(|port| free(*port))
            .map(RelayHome::Game)
            .ok_or_else(|| LauncherError::with_detail(ErrorCode::RelayPortBusy, &configured.to_string()))
    }

    async fn start_relay(&self, server: &StoredServer) -> Result<(), LauncherError> {
        let running = self.lock().relay.take();
        if let Some(relay) = running {
            relay.stop().await;
        }
        let port = self.config().relay_port;
        let failed = || LauncherError::with_detail(ErrorCode::RelayFailed, &port.to_string());
        if server.mode == ServerMode::Public && server.fp.is_none() {
            return Err(failed());
        }
        let game = self.shared.game.clone();
        let owner = match &self.shared.seams.relay_owner {
            RelayOwner::Game => game_owner_check(move || game.pid()),
            RelayOwner::Anyone => None,
            RelayOwner::Custom(check) => Some(Arc::clone(check)),
        };
        let log_sink: LogSink = Arc::new(|level, line| log::write(level.into(), &format!("relay: {line}")));
        let me = self.weak();
        let on_pin: PinSink = Arc::new(move |_seen| {
            if let Some(me) = Controller::from_weak(&me) {
                me.lock().last_error = Some(error(ErrorCode::CertMismatch));
                me.publish();
            }
        });
        let config = RelayConfig {
            port,
            qos_port: self.shared.seams.qos_port,
            target: server.endpoint(),
            owner,
            log: log_sink,
            on_pin_mismatch: on_pin,
        };
        match Relay::start(config).await {
            Ok(relay) => {
                self.lock().relay = Some(relay);
                Ok(())
            }
            Err(RelayError::PortBusy(busy)) => {
                Err(LauncherError::with_detail(ErrorCode::RelayPortBusy, &busy.to_string()))
            }
            Err(other) => {
                log::warn(format!("the relay did not start: {other:?}"));
                Err(failed())
            }
        }
    }

    pub(crate) fn base_environment(&self) -> BTreeMap<String, String> {
        self.shared
            .seams
            .game_env
            .clone()
            .unwrap_or_else(|| std::env::vars().collect())
    }

    async fn launch(
        &self,
        server: &StoredServer,
        win64: &Path,
        login: &str,
        user: &UserInfo,
        dlls: Vec<InjectDll>,
        home: RelayHome,
    ) -> Result<(), LauncherError> {
        let prepared = self
            .platform()
            .prepare_launch(self.user_data(), self.config().development_build)
            .await;
        let runner = match prepared {
            PreparedGame::Native => None,
            PreparedGame::Runner(runner) => Some(runner),
            PreparedGame::Missing => return Err(error(ErrorCode::CompatRuntimeMissing)),
        };
        let config_dir = runner.as_ref().map_or_else(
            || self.config().game_config_dir.clone(),
            |runner| runner.config_dir.clone(),
        );
        let stored = self.stored();
        // The relay's own port, which is not the configured one when Windows had lent that out
        let port = match home {
            RelayHome::Game(port) => port,
            RelayHome::Launcher => self.lock().relay.as_ref().map_or(self.config().relay_port, Relay::port),
        };
        let written = write_game_config(
            &config_dir,
            &ConfigRequest {
                host: "127.0.0.1",
                port,
                graphics: stored.graphics(),
                safe_window: stored.windowed(),
                clean_phoenix: true,
            },
        )
        .map_err(|failed| {
            log::warn(format!("could not write the game's config: {failed:?}"));
            error(ErrorCode::ConfigFailed)
        })?;
        if written.phoenix_keys_removed > 0 {
            log::info(format!(
                "removed {} Phoenix endpoints from Game.ini",
                written.phoenix_keys_removed
            ));
        }
        if written.engine_leftovers_removed > 0 {
            log::info(format!(
                "removed {} redirect and proxy settings another launcher left in Engine.ini",
                written.engine_leftovers_removed
            ));
        }

        let setting = game_language_code(stored.game_language());
        let locale = game_language::resolve(setting, Some(&self.config().os_locale));
        let launch_failed = |why: &dyn std::fmt::Debug| {
            log::warn(format!("could not start the game: {why:?}"));
            error(ErrorCode::LaunchFailed)
        };
        let args = launch_args(&LaunchArgs {
            key: login,
            user_id: &user.user_id,
            username: &user.username,
            windowed: stored.windowed() || written.display_repaired,
            locale,
        })
        .map_err(|failed| launch_failed(&failed))?;
        let mut env = launch_env(
            self.base_environment(),
            &format!("http://127.0.0.1:{port}"),
            stored.runtime_profile(),
        )
        .map_err(|failed| launch_failed(&failed))?;
        let economy = self.lock().status.as_ref().and_then(|status| status.economy.clone());
        if let Some(economy) = &economy {
            // The names are the protocol's own; the refused value itself is never logged
            for field in &economy.refused {
                let field: String = field.chars().take(32).collect();
                log::warn(format!("the server's {field} is invalid; the game keeps its default"));
            }
            log::info(format!(
                "the server's supply caps kept: {:?}, cell fusion minutes: {:?}",
                economy.keep_supply_stack_caps, economy.cell_fusion_minutes
            ));
        }
        add_economy_env(&mut env, economy.as_ref());
        for var in [SERVER_HOST_VAR, SERVER_PORT_VAR, SERVER_FP_VAR] {
            env.remove(var);
        }
        if matches!(home, RelayHome::Game(_)) {
            env.insert(SERVER_HOST_VAR.to_owned(), server.host.clone());
            env.insert(SERVER_PORT_VAR.to_owned(), server.port.to_string());
            if let Some(pin) = server.fp.as_deref().and_then(dainty_relay::pins::normalize_pin) {
                env.insert(SERVER_FP_VAR.to_owned(), pin);
            }
        }
        for dll in &dlls {
            log::info(format!(
                "loading {} into the game, SHA-256 {}",
                dll.path.display(),
                dll.sha256
            ));
        }
        let plan = match runner.as_ref() {
            // Proton with the inject helper: Wine runs the helper, which starts the game and loads the DLLs
            Some(runner) if !dlls.is_empty() => {
                let helper = self.config().resources.join(inject::HELPER_EXE);
                if !helper.is_file() {
                    log::warn(format!("the inject helper is missing: {}", helper.display()));
                    return Err(error(ErrorCode::InjectHelperMissing));
                }
                let game_args: Vec<OsString> = args.iter().map(OsString::from).collect();
                let line = inject::helper_line(&win64.join(EXE_NAME), &dlls, &game_args)
                    .map_err(|failed| launch_failed(&failed))?;
                LaunchPlan::through_helper(win64, runner, &helper, line, env)
            }
            _ => {
                let plan = LaunchPlan::new(win64, runner.as_ref(), &args, env);
                if runner.is_none() {
                    plan.with_inject(dlls.clone())
                } else {
                    plan
                }
            }
        };
        log::info(format!(
            "starting {} for {}:{} ({}, relay in the {})",
            plan.describe(),
            server.host,
            server.port,
            mode_name(server.mode),
            if matches!(home, RelayHome::Game(_)) {
                "game"
            } else {
                "launcher"
            }
        ));

        self.lock().game_log.clear();
        self.note_game_started();
        let me = self.clone();
        self.shared
            .game
            .start(&plan, move |event| me.on_game_event(event))
            .map_err(|failed| launch_failed(&failed))?;
        Ok(())
    }

    pub(crate) fn on_game_event(&self, event: GameEvent) {
        match event {
            GameEvent::Started(pid) => {
                if let Some(pid) = pid {
                    let hide = !self.stored().show_game_console();
                    let me = self.weak();
                    self.platform().watch_game_console(
                        pid,
                        hide,
                        Arc::new(move |lines| {
                            if let Some(me) = Controller::from_weak(&me) {
                                me.take_console_lines(lines);
                            }
                        }),
                    );
                }
                self.publish();
            }
            GameEvent::Exited(code) => {
                let me = self.clone();
                self.spawn(async move { me.on_game_exit(code).await });
            }
        }
    }

    async fn on_game_exit(&self, code: Option<i32>) {
        log::info(format!("the game exited with code {code:?}"));
        let dumping = std::mem::take(&mut self.lock().host.dumping);
        if let Some(code) = code
            && code != 0
            && code.cast_unsigned() != CONSOLE_CLOSED_EXIT
            && !dumping
        {
            self.lock().last_error = Some(LauncherError::with_detail(
                ErrorCode::LaunchFailed,
                &exit_code_hex(code),
            ));
        }
        self.end_current_loader_session().await;
        let relay = self.lock().relay.take();
        if let Some(relay) = relay {
            let counts = relay.stop().await;
            log::info(format!(
                "relay stopped: {} requests, {} upgrades, {} failures, {} QoS",
                counts.requests, counts.upgrades, counts.failures, counts.qos
            ));
        }
        self.look_at_runtime().await;
        if dumping {
            self.lock().game_started = None;
        } else {
            self.look_for_crash(code).await;
        }
        self.publish();
        self.shared.exits.send_modify(|(count, last)| {
            *count += 1;
            *last = code;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_runtime_with_the_marker_carries_the_relay() {
        assert!(carries_relay(b"\0\0dainty-lass:in-game-relay:v1: relay on"));
        assert!(!carries_relay(b"dainty-lass:in-game-relay:v"));
        assert!(!carries_relay(b""));
    }
}
