// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The launcher's state machine (RD27 spec §2.4): the page sends intents through the bridge's commands, and the
//! controller publishes a [`Snapshot`] after every step.
//!
//! All mutable state sits behind one mutex that is never held across an await, so concurrent commands interleave
//! only between steps. The settings store and the hash cache have async locks of their own.
//!
//! The Host page is `host` with the `host` feature and `host_off`'s refusals without it.

mod account;
pub mod cli;
mod connect;
pub(crate) mod crash;
mod game;
pub mod guarded;
#[cfg(feature = "host")]
mod host;
#[cfg(not(feature = "host"))]
mod host_off;
pub mod install;
mod join;
pub mod loader;
pub mod platform;
mod play;
mod playtime;
#[cfg(feature = "realms")]
pub mod realms;
#[cfg(not(feature = "realms"))]
mod realms_off;
pub mod region;
pub mod revived;
mod runtime;
pub mod stored;
pub mod versions;

#[cfg(not(feature = "realms"))]
use realms_off as realms;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use dainty_protocol::invite::{ServerMode, is_loopback_host, played_versions};
use dainty_protocol::launcher_version::below_minimum;
use dainty_protocol::status::ServerStatus;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::bridge::types::{
    ActionResult, Background, Branding, ConnectProblem, ErrorCode, ExternalTarget, Game, GameInfo, GameLanguage,
    GameProblem, LauncherError, NewsItem, NoticeCode, Phase, RuntimeInfo, RuntimeProfile, RuntimeState,
    SavedServerInfo, ServerGame, ServerInfo, Settings, SettingsPatch, Snapshot, SnapshotAccount, SnapshotApp,
    SnapshotConnect, SnapshotRunning, TaskProgress,
};
use crate::core::i18n::Language;
use crate::core::pins::PINNED_EXE_SHA256;
use crate::net::hostapi;
use crate::net::relay::Relay;
use crate::net::runtime_files::RuntimeManifest;
use crate::net::verify::{Abort, HashCache};
use crate::platform::keystore::os::Encryptor;
use crate::platform::keystore::{KeyStore, KeyStoreError, legacy::LegacyReader};
use crate::platform::launch::GameProcess;
use crate::platform::loader_session::LoaderSession;
use crate::platform::log;
use crate::platform::paths::{Migrated, Paths};
use crate::platform::settings::{Os, SettingsStore};

pub use platform::{ConsoleSink, FileFilter, FolderPurpose, Platform, PreparedGame, RelayOwner, Seams, ServerProbe};
pub use stored::{Stored, StoredServer};

/// The hash cache's file in `userData`.
pub const HASH_CACHE_FILE: &str = "file-hashes.json";
/// The art cache's folder in `userData`.
pub const ART_CACHE_DIR: &str = "art-cache";
/// The newest Game log lines kept.
pub const GAME_LOG_LINES: usize = 5000;
/// News is asked for at most this often.
pub const NEWS_INTERVAL: Duration = Duration::from_secs(60);
/// The default status poll interval.
pub const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(15);
/// The most poll ticks skipped between checks of a server that stays unreachable: 5 minutes at 15 s.
pub const UNREACHABLE_MAX_SKIP: u32 = 19;

/// What the app hands the controller (§2.1.2 step 9).
pub struct ControllerConfig {
    /// This PC's folders.
    pub paths: Paths,
    /// The app's resources folder.
    pub resources: PathBuf,
    /// The relay's port on 127.0.0.1.
    pub relay_port: u16,
    /// SHA-256 of the one game executable PLAY accepts.
    pub exe_pin: String,
    /// How often the status is polled.
    pub poll_interval: Duration,
    /// The OS's language, the settings' default.
    pub os_language: Language,
    /// The OS locale, for the game's `auto` language.
    pub os_locale: String,
    /// The game's config folder on Windows (§2.7.6).
    pub game_config_dir: PathBuf,
    /// Which folder rules apply, and the snapshot's `platform`.
    pub os: Os,
    /// Seals the key files.
    pub encryptor: Arc<dyn Encryptor>,
    /// Reads the old launcher's key files.
    pub legacy_reader: Option<Arc<dyn LegacyReader>>,
    /// The app's version.
    pub version: String,
    /// An installed release rather than a development run.
    pub packaged: bool,
    /// A development build: never installs Wine on its own.
    pub development_build: bool,
}

impl ControllerConfig {
    /// The defaults of §2.1.2 for `paths`, sealing with `encryptor`.
    #[must_use]
    pub fn new(paths: Paths, encryptor: Arc<dyn Encryptor>) -> Self {
        Self {
            paths,
            resources: PathBuf::new(),
            relay_port: crate::net::relay::DEFAULT_PORT,
            exe_pin: PINNED_EXE_SHA256.to_owned(),
            poll_interval: STATUS_POLL_INTERVAL,
            os_language: Language::En,
            os_locale: "en-US".to_owned(),
            game_config_dir: crate::platform::game_config::default_config_dir(),
            os: Os::current(),
            encryptor,
            legacy_reader: None,
            version: crate::VERSION.to_owned(),
            packaged: false,
            development_build: false,
        }
    }
}

/// Why closing the window should be questioned (§2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepOpen {
    /// The relay carries a running game.
    Relay,
}

/// Everything that changes, behind the one mutex (§2.4.2).
pub(crate) struct State {
    pub(crate) stored: Stored,
    pub(crate) reachable: Option<bool>,
    pub(crate) checking: bool,
    pub(crate) problem: Option<ConnectProblem>,
    pub(crate) tailscale_installed: bool,
    pub(crate) last_checked_at: Option<String>,
    pub(crate) failed_polls: u32,
    /// Checks in a row that did not reach the server, and the poll ticks to skip before the next one.
    pub(crate) failed_checks: u32,
    pub(crate) skip_ticks: u32,
    pub(crate) status: Option<ServerStatus>,
    pub(crate) status_unsupported: bool,
    pub(crate) has_key: bool,
    pub(crate) key_refused: bool,
    /// Bumped whenever the server or the account changes, so answers to older requests are dropped.
    pub(crate) generation: u64,
    pub(crate) busy: bool,
    pub(crate) game: GameInfo,
    pub(crate) manifest: Option<RuntimeManifest>,
    pub(crate) runtime: RuntimeInfo,
    pub(crate) runtime_playable: bool,
    pub(crate) task: Option<TaskProgress>,
    pub(crate) verify_abort: Option<Abort>,
    pub(crate) runtime_cancel: Option<CancellationToken>,
    /// Ends `sign_in`'s wait for the browser.
    pub(crate) sign_in_cancel: Option<CancellationToken>,
    /// Whether the account is linked to the server's sign-in, as `GetUserInfo` last said.
    pub(crate) oauth_linked: Option<bool>,
    pub(crate) last_error: Option<LauncherError>,
    pub(crate) notice: Option<NoticeCode>,
    pub(crate) update_ready: bool,
    pub(crate) poll: Option<JoinHandle<()>>,
    pub(crate) news: Vec<NewsItem>,
    pub(crate) news_at: Option<Instant>,
    pub(crate) news_loading: bool,
    pub(crate) branding: Option<Branding>,
    pub(crate) relay: Option<Relay>,
    pub(crate) shutting_down: bool,
    pub(crate) game_log: VecDeque<String>,
    pub(crate) host: HostFlags,
    /// PLAY's place in the server's login queue, while it waits there.
    pub(crate) queue_position: Option<u32>,
    /// Ends that wait (`leave_queue`).
    pub(crate) queue_leave: Option<CancellationToken>,
    /// When the running game was started, so only its own crash folders count.
    pub(crate) game_started: Option<std::time::SystemTime>,
    pub(crate) crash: Option<crash::Pending>,
}

/// The Host page's flags the controller keeps (§2.4.2).
#[derive(Debug, Default)]
pub struct HostFlags {
    /// The Host page's server is starting, running, stopping or answers from outside.
    pub server_up: bool,
    /// The game runs for a Dumper-7 dump, so its exit code is no launch failure.
    pub dumping: bool,
    /// The loader session placed for a dump or an export.
    pub loader_session: Option<LoaderSession>,
}

impl State {
    fn new(stored: Stored) -> Self {
        Self {
            stored,
            reachable: None,
            checking: false,
            problem: None,
            tailscale_installed: false,
            last_checked_at: None,
            failed_polls: 0,
            failed_checks: 0,
            skip_ticks: 0,
            status: None,
            status_unsupported: false,
            has_key: false,
            key_refused: false,
            generation: 0,
            busy: false,
            game: GameInfo {
                root: None,
                problem: Some(GameProblem::NotSet),
                version_text: None,
                version_ok: false,
                exe_verified: false,
                detected: Vec::new(),
            },
            manifest: None,
            runtime: RuntimeInfo {
                state: RuntimeState::Unavailable,
                version: None,
                files: Vec::new(),
                local_dir: None,
            },
            runtime_playable: false,
            task: None,
            verify_abort: None,
            runtime_cancel: None,
            sign_in_cancel: None,
            oauth_linked: None,
            last_error: None,
            notice: None,
            update_ready: false,
            poll: None,
            news: Vec::new(),
            news_at: None,
            news_loading: false,
            branding: None,
            relay: None,
            shutting_down: false,
            game_log: VecDeque::new(),
            host: HostFlags::default(),
            queue_position: None,
            queue_leave: None,
            game_started: None,
            crash: None,
        }
    }

    /// Forgets what the last server said (§2.4.2).
    fn reset_server(&mut self) {
        self.reachable = None;
        self.status = None;
        self.status_unsupported = false;
        self.problem = None;
        self.news.clear();
        self.news_at = None;
        self.branding = None;
        self.key_refused = false;
        self.failed_polls = 0;
        self.failed_checks = 0;
        self.skip_ticks = 0;
        self.manifest = None;
        self.generation += 1;
    }
}

pub(crate) struct Shared {
    pub(crate) config: ControllerConfig,
    pub(crate) seams: Seams,
    pub(crate) platform: Arc<dyn Platform>,
    state: Mutex<State>,
    pub(crate) settings: tokio::sync::Mutex<SettingsStore>,
    pub(crate) hashes: tokio::sync::Mutex<HashCache>,
    pub(crate) keys: Arc<KeyStore>,
    pub(crate) game: GameProcess,
    /// How many games have exited and the last one's code, once its clean-up is done (`--wait`).
    pub(crate) exits: tokio::sync::watch::Sender<(u64, Option<i32>)>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    // Their own locks, outside the state's
    pub(crate) installs: install::Installs,
    #[cfg_attr(
        not(feature = "realms"),
        allow(dead_code, reason = "a build without realms keeps nothing")
    )]
    pub(crate) realms: realms::RealmsState,
    #[cfg(feature = "host")]
    pub(crate) host: host::HostState,
}

/// The one controller of an app; clones share it.
#[derive(Clone)]
pub struct Controller {
    pub(crate) shared: Arc<Shared>,
}

impl Controller {
    /// Builds the controller (§2.4.1): migrates the old folders and loads the settings, keys and hash cache.
    #[must_use]
    pub fn new(config: ControllerConfig, platform: Arc<dyn Platform>, seams: Seams) -> Self {
        for (from, to, outcome) in config.paths.migrate_legacy() {
            match outcome {
                Ok(Migrated::Nothing) => {}
                Ok(done) => log::info(format!("moved {} to {} ({done:?})", from.display(), to.display())),
                Err(error) => log::warn(format!("could not move {}: {error}", from.display())),
            }
        }
        let settings = SettingsStore::load(&config.paths.user_data, config.os_language, config.os);
        let keys = KeyStore::new(
            config.paths.keys(),
            config.paths.recovery(),
            Arc::clone(&config.encryptor),
            config.legacy_reader.clone(),
        );
        let hashes = HashCache::load(config.paths.user_data.join(HASH_CACHE_FILE));
        let game = seams
            .spawner
            .clone()
            .map_or_else(GameProcess::new, GameProcess::with_spawner);
        let state = State::new(Stored(settings.map().clone()));
        #[cfg(feature = "host")]
        let host = host::HostState::new(&seams);
        let controller = Self {
            shared: Arc::new(Shared {
                config,
                seams,
                platform,
                state: Mutex::new(state),
                settings: tokio::sync::Mutex::new(settings),
                hashes: tokio::sync::Mutex::new(hashes),
                keys: Arc::new(keys),
                game,
                exits: tokio::sync::watch::Sender::new((0, None)),
                tasks: Mutex::new(Vec::new()),
                installs: install::Installs::default(),
                realms: realms::RealmsState::default(),
                #[cfg(feature = "host")]
                host,
            }),
        };
        #[cfg(feature = "host")]
        controller.attach_host_server();
        controller
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn weak(&self) -> Weak<Shared> {
        Arc::downgrade(&self.shared)
    }

    pub(crate) fn from_weak(weak: &Weak<Shared>) -> Option<Self> {
        weak.upgrade().map(|shared| Self { shared })
    }

    pub(crate) fn config(&self) -> &ControllerConfig {
        &self.shared.config
    }

    pub(crate) fn platform(&self) -> &dyn Platform {
        self.shared.platform.as_ref()
    }

    /// Runs `work` in the background; shutdown waits for it.
    pub(crate) fn spawn(&self, work: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = tokio::spawn(work);
        let mut tasks = self.shared.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle);
    }

    /// A key store call on the blocking pool (D-Bus and DPAPI may block).
    pub(crate) async fn with_keys<T: Send + 'static>(
        &self,
        work: impl FnOnce(&KeyStore) -> Result<T, KeyStoreError> + Send + 'static,
    ) -> Result<T, KeyStoreError> {
        let keys = Arc::clone(&self.shared.keys);
        tokio::task::spawn_blocking(move || work(&keys))
            .await
            .unwrap_or(Err(KeyStoreError::Failed))
    }

    /// Edits the settings map, writes it and refreshes the view the state holds.
    pub(crate) async fn edit_settings(&self, edit: impl FnOnce(&mut serde_json::Map<String, Value>)) {
        let mut store = self.shared.settings.lock().await;
        if let Err(error) = store.update(edit) {
            log::warn(format!("could not write the settings: {error}"));
        }
        let map = store.map().clone();
        drop(store);
        self.lock().stored = Stored(map);
    }

    /// The stored settings view, as of now.
    pub(crate) fn stored(&self) -> Stored {
        self.lock().stored.clone()
    }

    /// The current server.
    pub(crate) fn server(&self) -> Option<StoredServer> {
        self.lock().stored.server()
    }

    /// The launcher's language.
    #[must_use]
    pub fn language(&self) -> Language {
        self.lock().stored.language()
    }

    // --- Publishing ---

    /// Emits the snapshot.
    pub fn publish(&self) {
        let snapshot = self.snapshot();
        self.platform().emit_snapshot(&snapshot);
    }

    /// Sets the last error and publishes; answers it.
    pub(crate) fn fail(&self, error: LauncherError) -> ActionResult {
        self.lock().last_error = Some(error.clone());
        self.publish();
        ActionResult::Err(error)
    }

    /// Sets the last error and publishes; answers the error, for callers that answer something else.
    pub(crate) fn refuse(&self, error: LauncherError) -> LauncherError {
        self.lock().last_error = Some(error.clone());
        self.publish();
        error
    }

    /// Marks busy unless something already runs; clears the last error and publishes.
    pub(crate) fn begin_busy(&self) -> Result<(), LauncherError> {
        {
            let mut state = self.lock();
            if state.busy || state.task.is_some() {
                return Err(LauncherError::new(ErrorCode::Busy));
            }
            state.busy = true;
            state.last_error = None;
        }
        self.publish();
        Ok(())
    }

    /// Ends a busy action with its result: a failure becomes the last error.
    pub(crate) fn finish(&self, result: Result<(), LauncherError>) -> ActionResult {
        let mut state = self.lock();
        state.busy = false;
        let answer = match result {
            Ok(()) => ActionResult::Ok,
            Err(error) => {
                state.last_error = Some(error.clone());
                ActionResult::Err(error)
            }
        };
        drop(state);
        self.publish();
        answer
    }

    /// A busy action, a task or the game: what most commands refuse.
    pub(crate) fn occupied(&self) -> bool {
        let state = self.lock();
        state.busy || state.task.is_some() || self.shared.game.is_running()
    }

    /// The snapshot of §2.4.2.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let state = self.lock();
        self.snapshot_of(&state)
    }

    fn phase_of(&self, state: &State, server: Option<&StoredServer>) -> Phase {
        if server.is_none() {
            Phase::Join
        } else if state.task.is_some() {
            Phase::Installing
        } else if self.shared.game.is_running() {
            Phase::Running
        } else if state.reachable != Some(true) {
            Phase::Connect
        } else if self.too_old_for(state).is_some() {
            Phase::LauncherTooOld
        } else if !state.has_key || state.key_refused {
            Phase::Register
        } else if state.game.problem.is_some() {
            Phase::Locate
        } else if !state.runtime_playable {
            Phase::Runtime
        } else {
            Phase::Ready
        }
    }

    fn snapshot_of(&self, state: &State) -> Snapshot {
        let stored = &state.stored;
        let server = stored.server();
        let current = server.as_ref().map(StoredServer::id);
        let saved_servers = stored
            .saved_servers()
            .into_iter()
            .map(|entry| {
                let id = entry.id();
                SavedServerInfo {
                    current: current.as_deref() == Some(id.as_str()),
                    username: stored.username(&id).map(str::to_owned),
                    unsupported: entry.game == ServerGame::Revived,
                    id,
                    game: entry.game,
                    mode: entry.mode,
                    host: entry.host,
                    port: entry.port,
                    name: entry.name,
                }
            })
            .collect();
        let has_key = state.has_key && !state.key_refused;
        let account = SnapshotAccount {
            username: current.as_deref().and_then(|id| stored.username(id)).map(str::to_owned),
            has_key,
            offer_backup: has_key && current.as_deref().is_some_and(|id| !stored.backup_offered(id)),
            oauth_linked: state.oauth_linked.filter(|_| has_key),
        };
        Snapshot {
            phase: self.phase_of(state, server.as_ref()),
            busy: state.busy,
            server: server.as_ref().map(|server| ServerInfo {
                game: server.game,
                mode: server.mode,
                host: server.host.clone(),
                port: server.port,
                name: server.name.clone(),
                has_share: server.share.is_some(),
                has_pending_invite: server.code.is_some(),
                loopback: is_loopback_host(&server.host),
                fingerprint: server.pin().map(str::to_owned),
            }),
            saved_servers,
            connect: SnapshotConnect {
                checking: state.checking,
                problem: state.problem,
                tailscale_installed: state.tailscale_installed,
                last_checked_at: state.last_checked_at.clone(),
            },
            account,
            game: state.game.clone(),
            revived: None,
            runtime: state.runtime.clone(),
            task: state.task.clone(),
            running: SnapshotRunning {
                game: self.shared.game.is_running(),
                relay_port: state.relay.as_ref().map(Relay::port),
            },
            settings: settings_of(stored),
            app: SnapshotApp {
                version: self.config().version.clone(),
                packaged: self.config().packaged,
                update_ready: state.update_ready,
                platform: platform_name(self.config().os).to_owned(),
                host_tools_available: cfg!(feature = "host"),
                legacy_available: false,
                game_versions: played_versions().iter().map(ToString::to_string).collect(),
                server_gui: cfg!(feature = "server-gui"),
            },
            status: state.status.clone(),
            status_unsupported: state.status_unsupported,
            last_error: state.last_error.clone(),
            notice: state.notice,
            queue_position: state.queue_position,
            crash: state.crash.as_ref().map(|c| c.offer.clone()),
        }
    }

    // --- Start and stop ---

    /// Start-up (§2.4.3): the key flag, a left loader session, the game, the runtime, the Host workspace, then a
    /// connection and a verify in the background.
    pub async fn init(&self) {
        self.start_up().await;
        if self.server().is_some() {
            let me = self.clone();
            self.spawn(async move {
                let _ = me.connect().await;
            });
        }
        if self.lock().game.problem == Some(GameProblem::Unverified) {
            let me = self.clone();
            self.spawn(async move {
                let _ = me.verify_game().await;
            });
        }
    }

    /// `init`'s awaited part, which the command line runs before its own connect.
    pub(crate) async fn start_up(&self) {
        self.refresh_key_flag().await;
        self.recover_loader_session().await;
        self.look_at_game(true).await;
        self.look_at_runtime().await;
        let _ = self.ensure_host_workspace(true).await;
        self.publish();
    }

    /// Exit (§2.4.12): PLAY refuses from now on; background work ends; the relay stops; the hash cache is saved.
    pub async fn shutdown(&self) {
        self.lock().shutting_down = true;
        self.host_shutdown().await;
        self.realms_shutdown().await;
        loop {
            let tasks: Vec<JoinHandle<()>> =
                std::mem::take(&mut *self.shared.tasks.lock().unwrap_or_else(PoisonError::into_inner));
            if tasks.is_empty() {
                break;
            }
            for task in tasks {
                let _ = task.await;
            }
        }
        self.set_status_polling(false);
        self.cancel_task();
        let relay = self.lock().relay.take();
        if let Some(relay) = relay {
            relay.stop().await;
        }
        if let Err(error) = self.shared.hashes.lock().await.save().await {
            log::warn(format!("could not save the hash cache: {error}"));
        }
    }

    // --- Settings, links and folders (§2.4.10) ---

    /// Applies a settings change and answers the settings.
    pub async fn set_settings(&self, patch: SettingsPatch) -> Settings {
        let host_on = cfg!(feature = "host") && patch.host_tools == Some(true) && !self.stored().chose_host_tools();
        self.edit_settings(|map| {
            if let Some(graphics) = patch.graphics {
                stored::edit::field(map, "graphics", json!(graphics.get()));
            }
            let flags = [
                ("windowed", patch.windowed),
                ("streamerMode", patch.streamer_mode),
                ("hostTools", patch.host_tools.filter(|_| cfg!(feature = "host"))),
                ("minimizeToTray", patch.minimize_to_tray),
                ("closeToTray", patch.close_to_tray),
                ("showGameConsole", patch.show_game_console),
                ("injectHelper", patch.inject_helper),
            ];
            for (name, value) in flags {
                if let Some(value) = value {
                    stored::edit::field(map, name, json!(value));
                }
            }
            if let Some(language) = patch.language {
                stored::edit::field(map, "language", json!(language.code()));
            }
            if let Some(language) = patch.game_language {
                stored::edit::field(map, "gameLanguage", json!(game_language_code(language)));
            }
            if let Some(profile) = patch.runtime_profile {
                stored::edit::field(map, "runtimeProfile", json!(runtime_profile_code(profile)));
            }
            if let Some(choice) = patch.crash_reports {
                stored::edit::field(map, "crashReports", json!(choice));
            }
        })
        .await;
        if host_on {
            let _ = self.ensure_host_workspace(true).await;
        }
        self.publish();
        settings_of(&self.stored())
    }

    /// (`minimizeToTray`, `closeToTray`) for the window.
    #[must_use]
    pub fn tray(&self) -> (bool, bool) {
        let state = self.lock();
        (state.stored.minimize_to_tray(), state.stored.close_to_tray())
    }

    /// The kept Game log lines, oldest first.
    #[must_use]
    pub fn game_log(&self) -> Vec<String> {
        self.lock().game_log.iter().cloned().collect()
    }

    /// Takes one batch of the game's console lines: redacted, kept (newest 5,000) and emitted.
    pub(crate) fn take_console_lines(&self, lines: Vec<String>) {
        let lines: Vec<String> = lines.iter().map(|line| log::redact(line)).collect();
        {
            let mut state = self.lock();
            state.game_log.extend(lines.iter().cloned());
            while state.game_log.len() > GAME_LOG_LINES {
                state.game_log.pop_front();
            }
        }
        self.platform().emit_game_log(&lines);
    }

    /// Opens an external page; the URL is fixed here or comes from the server and must pass the link check.
    pub fn open_external(&self, target: ExternalTarget) -> ActionResult {
        let (share, source) = {
            let state = self.lock();
            let share = state
                .stored
                .server()
                .filter(|server| server.mode == ServerMode::Private)
                .and_then(|server| server.share);
            let source = state.status.as_ref().and_then(|status| status.source_url.clone());
            (share, source)
        };
        let Some(url) = crate::platform::links::target_url(target, share.as_deref(), source.as_deref()) else {
            return ActionResult::err(ErrorCode::Unknown);
        };
        match self.platform().open_url(&url) {
            Ok(()) => ActionResult::Ok,
            Err(error) => {
                log::warn(format!("could not open a link: {error}"));
                ActionResult::err(ErrorCode::Unknown)
            }
        }
    }

    /// Opens `<userData>/logs` when it exists.
    pub fn open_logs_folder(&self) {
        let logs = self.config().paths.logs();
        if logs.is_dir()
            && let Err(error) = self.platform().open_folder(&logs)
        {
            log::warn(format!("could not open the logs folder: {error}"));
        }
    }

    /// Shows a notice.
    pub fn set_notice(&self, notice: NoticeCode) {
        self.lock().notice = Some(notice);
        self.publish();
    }

    /// Clears the notice.
    pub fn dismiss_notice(&self) {
        self.lock().notice = None;
        self.publish();
    }

    /// Clears the last error.
    pub fn dismiss_error(&self) {
        self.lock().last_error = None;
        self.publish();
    }

    /// The launcher version the server asks for, when this launcher is below it (`minLauncherVersion`).
    pub(crate) fn too_old_for(&self, state: &State) -> Option<String> {
        let minimum = state.status.as_ref()?.min_launcher_version.as_deref()?;
        below_minimum(&self.config().version, minimum).then(|| minimum.to_owned())
    }

    /// The version the server asks for, when this launcher must update before it plays.
    #[must_use]
    pub fn launcher_too_old(&self) -> Option<String> {
        self.too_old_for(&self.lock())
    }

    /// Nothing would be cut short by updating now: no task, no game, not closing.
    #[must_use]
    pub fn may_update_now(&self) -> bool {
        let state = self.lock();
        state.task.is_none() && !state.shutting_down && !self.shared.game.is_running()
    }

    /// Marks an update as downloaded and waiting.
    pub fn set_update_ready(&self, ready: bool) {
        self.lock().update_ready = ready;
        self.publish();
    }

    /// An update waits and nothing would be cut short by installing it.
    #[must_use]
    pub fn may_install_update(&self) -> bool {
        let state = self.lock();
        state.update_ready && state.task.is_none() && !state.shutting_down && !self.shared.game.is_running()
    }

    /// Why the window should not just close.
    #[must_use]
    pub fn keep_open(&self) -> Option<KeepOpen> {
        self.lock().relay.is_some().then_some(KeepOpen::Relay)
    }

    /// The window was kept open for `reason`.
    pub fn kept_open(&self, reason: KeepOpen) {
        match reason {
            KeepOpen::Relay => self.set_notice(NoticeCode::KeepOpen),
        }
    }

    /// The news, for `get_news`.
    #[must_use]
    pub fn news(&self) -> Vec<NewsItem> {
        self.lock().news.clone()
    }

    /// The server's look, for `get_branding`.
    #[must_use]
    pub fn branding(&self) -> Branding {
        self.lock().branding.clone().unwrap_or(Branding {
            backgrounds: Vec::new(),
            accent: None,
        })
    }

    pub(crate) fn user_data(&self) -> &Path {
        &self.config().paths.user_data
    }
}

pub(crate) fn platform_name(os: Os) -> &'static str {
    match os {
        Os::Windows => "win32",
        Os::Linux => "linux",
    }
}

pub(crate) fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) const fn game_language_code(language: GameLanguage) -> &'static str {
    match language {
        GameLanguage::Auto => "auto",
        GameLanguage::EnUs => "en-US",
        GameLanguage::DeDe => "de-DE",
        GameLanguage::EsEs => "es-ES",
        GameLanguage::FrFr => "fr-FR",
        GameLanguage::ItIt => "it-IT",
        GameLanguage::JaJp => "ja-JP",
        GameLanguage::PtBr => "pt-BR",
        GameLanguage::RuRu => "ru-RU",
    }
}

pub(crate) const fn runtime_profile_code(profile: RuntimeProfile) -> &'static str {
    match profile {
        RuntimeProfile::Production => "production",
        RuntimeProfile::Development => "development",
    }
}

fn settings_of(stored: &Stored) -> Settings {
    Settings {
        graphics: stored.graphics(),
        windowed: stored.windowed(),
        language: stored.language(),
        game_language: stored.game_language(),
        runtime_profile: stored.runtime_profile(),
        streamer_mode: stored.streamer_mode(),
        host_tools: cfg!(feature = "host") && stored.host_tools(),
        game_edition: Game::Reborn,
        minimize_to_tray: stored.minimize_to_tray(),
        close_to_tray: stored.close_to_tray(),
        show_game_console: stored.show_game_console(),
        inject_helper: stored.inject_helper(),
        crash_reports: stored.crash_reports(),
    }
}

/// The bridge's news from the host API's.
pub(crate) fn news_items(items: Vec<hostapi::NewsItem>) -> Vec<NewsItem> {
    items
        .into_iter()
        .map(|item| NewsItem {
            date: item.date,
            title: item.title,
            body: item.body,
        })
        .collect()
}

pub(crate) fn background(url: String, credit: Option<String>) -> Background {
    Background { url, credit }
}
