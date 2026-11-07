// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The Host page (RD27 spec §2.5), answered as a build without the host kit does (§2.5.1). RL12b's Host page
//! controller replaces this module; [`HostFlags`] is what it fills.

use crate::bridge::checks::{HOST_TOOLS_UNAVAILABLE, host_failed};
use crate::bridge::types::{
    ActionResult, GameDataState, HostAction, HostBuilds, HostExports, HostFolder, HostGameData, HostKit,
    HostMultipliers, HostSdk, HostServerConfig, HostServerState, HostServerView, HostSetupRequest, HostToolchain,
    HostUpstream,
};
use crate::platform::loader_session::LoaderSession;

use super::Controller;

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

/// The kit of a launcher without Host tools: no workspace, nothing found, game data missing.
#[must_use]
pub fn empty_kit() -> HostKit {
    HostKit {
        workspace: None,
        workspace_ok: false,
        upstream: HostUpstream {
            present: false,
            patched: false,
        },
        sdk: HostSdk {
            present: false,
            function_files: 0,
        },
        dumps: Vec::new(),
        dumper_dll: None,
        builds: HostBuilds {
            runtime: false,
            loader: false,
            exporter: false,
            published: false,
        },
        toolchain: HostToolchain {
            visual_studio: None,
            msbuild: false,
            cargo: false,
        },
        game_data: HostGameData {
            backend: GameDataState::Missing,
            director: GameDataState::Missing,
        },
        exports: HostExports {
            present: false,
            in_game: false,
        },
        server_config: HostServerConfig {
            configured: false,
            name: None,
            mode: None,
            host: None,
            multipliers: None,
        },
        requirements: Vec::new(),
        job: None,
        update: None,
        project_target: None,
    }
}

/// A stopped server.
#[must_use]
pub fn stopped_server() -> HostServerView {
    HostServerView {
        state: HostServerState::Stopped,
        message: None,
        lines: Vec::new(),
        invite: None,
        embedded: false,
        components: Vec::new(),
    }
}

impl Controller {
    fn host_refused(&self) -> ActionResult {
        self.fail(host_failed(HOST_TOOLS_UNAVAILABLE))
    }

    /// `host_get`.
    #[must_use]
    pub fn host_kit(&self) -> HostKit {
        empty_kit()
    }

    /// Makes sure a Host workspace is chosen; `quiet` never fails for a missing one.
    pub async fn ensure_host_workspace(&self, quiet: bool) -> ActionResult {
        let _ = quiet;
        ActionResult::Ok
    }

    /// `host_choose_workspace`.
    pub async fn host_choose_workspace(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_choose_dumper`.
    pub async fn host_choose_dumper(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_import_gamedata`.
    pub async fn host_import_gamedata(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_import_sdk_folder`.
    pub async fn host_import_sdk_folder(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_download_project`.
    pub async fn host_download_project(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_action`.
    pub async fn host_action(&self, action: HostAction) -> ActionResult {
        let _ = action;
        self.host_refused()
    }

    /// `host_open`.
    pub fn host_open(&self, folder: HostFolder) -> ActionResult {
        let _ = folder;
        self.host_refused()
    }

    /// `host_server_get`.
    #[must_use]
    pub fn host_server_view(&self) -> HostServerView {
        stopped_server()
    }

    /// `host_server_start`.
    pub async fn host_server_start(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_server_stop`.
    pub async fn host_server_stop(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_server_restart`.
    pub async fn host_server_restart(&self, name: &str) -> ActionResult {
        let _ = name;
        self.host_refused()
    }

    /// `host_setup`.
    pub async fn host_setup(&self, request: HostSetupRequest) -> ActionResult {
        let _ = request;
        self.host_refused()
    }

    /// `host_create_invite`.
    pub async fn host_create_invite(&self, for_name: Option<String>) -> ActionResult {
        let _ = for_name;
        self.host_refused()
    }

    /// `host_update`.
    pub async fn host_update(&self) -> ActionResult {
        self.host_refused()
    }

    /// `host_set_multipliers`.
    pub async fn host_set_multipliers(&self, multipliers: Option<HostMultipliers>) -> ActionResult {
        let _ = multipliers;
        self.host_refused()
    }

    /// Stops the Host page's server if this launcher started it.
    pub(crate) async fn host_shutdown(&self) {}
}
