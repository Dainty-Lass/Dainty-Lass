// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! What the controller needs from the app (RD27 spec §2.4.1). No Tauri type crosses it: the shell implements it,
//! the tests fake it.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use url::Url;

use crate::bridge::types::{HostKit, HostServerView, Snapshot, TaskProgress};
use crate::core::i18n::Language;
use crate::net::hostapi::ImageType;
use crate::net::relay_owner::OwnerCheck;
use crate::platform::game_folder::StoreFolders;
use crate::platform::hostserver::HostTool;
use crate::platform::launch::{MissingFiles, Spawner};
use crate::platform::linux_runtime::PreparedLaunch;

/// What a folder picker is for; the shell titles it with the matching `dialog_*` key (§2.8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderPurpose {
    /// The game folder.
    Game,
    /// A local runtime folder.
    Runtime,
    /// The Host workspace.
    Workspace,
    /// A Dumper-7 SDK dump.
    Sdk,
    /// The folder Epic installs go into, one version folder each.
    GameInstall,
}

/// A file picker's one filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFilter {
    /// Its label.
    pub name: &'static str,
    /// The extension, without the dot.
    pub extension: &'static str,
}

/// The filter of the key backup's save and open pickers.
pub const TEXT_FILTER: FileFilter = FileFilter {
    name: "Text",
    extension: "txt",
};

/// How the game starts on this OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedGame {
    /// Directly (Windows).
    Native,
    /// Through Proton or Wine (Linux, §2.7.13).
    Runner(PreparedLaunch),
    /// Linux with neither.
    Missing,
}

/// Where the game's console lines go, a batch at a time.
pub type ConsoleSink = Arc<dyn Fn(Vec<String>) + Send + Sync>;

/// The app around the controller. Dialogs answer `None` on cancel; the defaults use dainty-launcher-kit's own helpers.
pub trait Platform: Send + Sync + 'static {
    /// A folder picker starting at `start` when it exists.
    fn choose_folder(
        &self,
        language: Language,
        purpose: FolderPurpose,
        start: Option<PathBuf>,
    ) -> BoxFuture<'_, Option<PathBuf>>;

    /// A save picker in Documents with `default_name`.
    fn choose_save_file(
        &self,
        language: Language,
        default_name: String,
        filter: FileFilter,
    ) -> BoxFuture<'_, Option<PathBuf>>;

    /// An open picker in Documents.
    fn choose_open_file(&self, language: Language, filter: FileFilter) -> BoxFuture<'_, Option<PathBuf>>;

    /// The Dumper-7 DLL picker (`DLL`/`dll`).
    fn choose_dumper_dll(&self, language: Language) -> BoxFuture<'_, Option<PathBuf>> {
        let _ = language;
        Box::pin(async { None })
    }

    /// The game data pack picker (`Game data pack`/`gz`).
    fn choose_gamedata_pack(&self, language: Language) -> BoxFuture<'_, Option<PathBuf>> {
        let _ = language;
        Box::pin(async { None })
    }

    /// Opens a URL that passed the link check.
    fn open_url(&self, url: &Url) -> io::Result<()> {
        crate::platform::links::open_url(url)
    }

    /// Opens a folder; false when it is not one.
    fn open_folder(&self, dir: &Path) -> io::Result<bool> {
        crate::platform::links::open_folder(dir)
    }

    /// Event `snapshot`.
    fn emit_snapshot(&self, snapshot: &Snapshot);

    /// The progress channel (§2.3.5 throttles it).
    fn emit_progress(&self, progress: &TaskProgress) {
        let _ = progress;
    }

    /// Event `host-kit`.
    fn emit_host_kit(&self, kit: &HostKit) {
        let _ = kit;
    }

    /// Event `host-server`; it may carry an invite link, so it is never logged.
    fn emit_host_server(&self, view: &HostServerView) {
        let _ = view;
    }

    /// Serves `file` as `rr-art://bg/<id>`.
    fn register_art(&self, id: &str, file: &Path, kind: ImageType) {
        let _ = (id, file, kind);
    }

    /// Tailscale's executable.
    fn find_tailscale(&self) -> Option<PathBuf> {
        crate::platform::system::find_tailscale()
    }

    /// Game clients running on this PC (§2.7.5).
    fn running_clients(&self) -> BoxFuture<'_, Vec<u32>> {
        Box::pin(crate::platform::launch::running_clients())
    }

    /// Readies the game's start for this OS.
    fn prepare_launch<'a>(&'a self, user_data: &'a Path, development_build: bool) -> BoxFuture<'a, PreparedGame> {
        Box::pin(async move {
            if cfg!(windows) {
                return PreparedGame::Native;
            }
            let env = crate::platform::linux_runtime::current_env();
            let options = crate::platform::linux_runtime::PrepareOptions { development_build };
            match crate::platform::linux_runtime::prepare(&env, user_data, options).await {
                Ok(prepared) => PreparedGame::Runner(prepared),
                Err(_) => PreparedGame::Missing,
            }
        })
    }

    /// Visual C++ and DirectX files missing from System32.
    fn missing_files(&self) -> MissingFiles {
        crate::platform::launch::missing_files()
    }

    /// Starts reading the game's console (§2.7.16); `hide` hides its window.
    fn watch_game_console(&self, pid: u32, hide: bool, sink: ConsoleSink) {
        let _ = (pid, hide, sink);
    }

    /// Event `game-log`: new lines, redacted.
    fn emit_game_log(&self, lines: &[String]) {
        let _ = lines;
    }
}

/// A probe for "a Host page server answers on 61000" (the moved Host module uses it).
pub type ServerProbe = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

/// Who may use the relay's game port.
#[derive(Clone, Default)]
pub enum RelayOwner {
    /// The launcher's game alone, on Windows (§2.6.5).
    #[default]
    Game,
    /// Any loopback client that passes the local rules.
    Anyone,
    /// The test's own check.
    Custom(Arc<dyn OwnerCheck>),
}

/// Stand-ins the tests use (§2.4.1); every field's default is the real thing.
#[derive(Clone)]
pub struct Seams {
    /// Starts the game.
    pub spawner: Option<Arc<dyn Spawner>>,
    /// The game's base environment; the process's when none.
    pub game_env: Option<BTreeMap<String, String>>,
    /// Where store installs are looked for; the OS's when none.
    pub store_folders: Option<StoreFolders>,
    /// The relay's QoS-only listener (80); none turns it off.
    pub qos_port: Option<u16>,
    /// Whether a Host page server answers on 61000.
    pub host_server_probe: Option<ServerProbe>,
    /// Where Dumper-7 writes its dumps.
    pub dumper_root: PathBuf,
    /// The relay's owner check.
    pub relay_owner: RelayOwner,
    /// The host kit to run instead of the workspace's `dainty-host`.
    pub host_tool: Option<HostTool>,
    /// How long a stopped Host page server gets before its processes are ended.
    pub host_stop_timeout: Duration,
}

impl Default for Seams {
    fn default() -> Self {
        Self {
            spawner: None,
            game_env: None,
            store_folders: None,
            qos_port: Some(crate::net::relay::DEFAULT_QOS_PORT),
            host_server_probe: None,
            dumper_root: PathBuf::from(r"C:\Dumper-7"),
            relay_owner: RelayOwner::Game,
            host_tool: None,
            host_stop_timeout: crate::platform::hostserver::STOP_TIMEOUT,
        }
    }
}
