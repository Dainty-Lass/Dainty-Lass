// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The Host page in a player-only build (no `host` feature): `host.rs`'s methods with nothing behind
//! them, so the bridge, its generated files and the controller are the same in both builds. The snapshot says
//! `hostToolsAvailable: false`, so the page never offers the Host tools; a call that comes anyway is
//! refused.

use super::Controller;
use crate::bridge::types::{
    ActionResult, ErrorCode, GameDataState, HostAction, HostBuilds, HostExports, HostFolder, HostGameData, HostKit,
    HostMultipliers, HostSdk, HostServerConfig, HostServerState, HostServerView, HostSetupRequest, HostToolchain,
    HostUpstream, LauncherError,
};

impl Controller {
    pub(crate) async fn host_shutdown(&self) {}

    /// Makes sure a Host workspace is chosen; `auto` never fails for a missing one.
    pub async fn ensure_host_workspace(&self, _auto: bool) -> ActionResult {
        ActionResult::Ok
    }

    fn host_off(&self) -> ActionResult {
        ActionResult::Err(self.refuse(LauncherError::with_detail(
            ErrorCode::HostFailed,
            "host_tools_unavailable",
        )))
    }

    /// The export session, refused like every Host command.
    pub async fn host_export(&self, _exporter: Option<&std::path::Path>) -> ActionResult {
        self.host_off()
    }

    /// No Host workspace, so no CatalogExporter.
    pub fn host_exporter_dll(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// No Host tools, so no exports.
    pub fn game_exports_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// No Host tools, so nowhere to look for exports.
    pub fn export_candidates(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// No Host tools, so no exports.
    pub fn latest_exports(&self, _since: Option<std::time::SystemTime>) -> Option<std::path::PathBuf> {
        None
    }

    /// `host_get`.
    pub async fn host_kit(&self) -> HostKit {
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

    /// `host_choose_workspace`.
    pub async fn host_choose_workspace(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_choose_dumper`.
    pub async fn host_choose_dumper(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_import_gamedata`.
    pub async fn host_import_gamedata(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_import_sdk_folder`.
    pub async fn host_import_sdk_folder(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_download_project`.
    pub async fn host_download_project(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_action`.
    pub async fn host_action(&self, _action: HostAction) -> ActionResult {
        self.host_off()
    }

    /// `host_open`.
    pub fn host_open(&self, _folder: HostFolder) -> ActionResult {
        self.host_off()
    }

    /// `host_server_get`.
    #[must_use]
    pub fn host_server_view(&self) -> HostServerView {
        HostServerView {
            state: HostServerState::Stopped,
            message: None,
            lines: Vec::new(),
            invite: None,
            embedded: false,
            components: Vec::new(),
        }
    }

    /// `host_server_start`.
    pub async fn host_server_start(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_server_stop`.
    pub async fn host_server_stop(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_server_restart`.
    pub async fn host_server_restart(&self, _name: &str) -> ActionResult {
        self.host_off()
    }

    /// `host_setup`.
    pub async fn host_setup(&self, _request: HostSetupRequest) -> ActionResult {
        self.host_off()
    }

    /// `host_create_invite`.
    pub async fn host_create_invite(&self, _for_name: Option<String>) -> ActionResult {
        self.host_off()
    }

    /// `host_update`.
    pub async fn host_update(&self) -> ActionResult {
        self.host_off()
    }

    /// `host_set_multipliers`.
    pub async fn host_set_multipliers(&self, _multipliers: Option<HostMultipliers>) -> ActionResult {
        self.host_off()
    }
}
