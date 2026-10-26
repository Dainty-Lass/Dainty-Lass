//! The Host page in a player-only build (no `host` feature): `host.rs`'s methods with nothing behind
//! them, so the bridge, its generated files and `mod.rs` are the same in both builds. The snapshot says
//! `hostToolsAvailable: false`, so the page never offers the Host tools; a call that comes anyway is
//! refused.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use super::Controller;
use crate::bridge::types::{
    ActionResult, ErrorCode, GameDataState, HostAction, HostBuilds, HostExports, HostFolder, HostGameData, HostKit,
    HostMultipliers, HostSdk, HostServerConfig, HostServerState, HostServerView, HostSetupRequest, HostToolchain,
    HostUpstream,
};

/// `hostserver::Probe`'s shape, for the controller's test seams.
pub type Probe = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

#[derive(Default)]
pub struct HostState;

impl HostState {
    pub fn with(_probe: Option<Probe>, _dump_root: Option<PathBuf>) -> Self {
        Self
    }
}

impl Controller {
    pub(super) fn host_attach(&self) {}

    pub(super) async fn host_shutdown(&self) {}

    pub(super) async fn ensure_host_workspace(&self, _auto: bool) -> ActionResult {
        ActionResult::Ok
    }

    fn host_off(&self) -> ActionResult {
        ActionResult::Err(self.fail(ErrorCode::HostFailed, Some("host_tools_unavailable".to_owned())))
    }

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

    pub async fn host_choose_workspace(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_choose_dumper(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_import_gamedata(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_import_sdk_folder(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_download_project(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_action(&self, _action: HostAction) -> ActionResult {
        self.host_off()
    }

    pub fn host_open(&self, _folder: HostFolder) -> ActionResult {
        self.host_off()
    }

    pub fn host_server_view(&self) -> HostServerView {
        HostServerView {
            state: HostServerState::Stopped,
            message: None,
            lines: Vec::new(),
            invite: None,
        }
    }

    pub async fn host_server_start(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_server_stop(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_setup(&self, _request: HostSetupRequest) -> ActionResult {
        self.host_off()
    }

    pub async fn host_create_invite(&self, _for_name: Option<String>) -> ActionResult {
        self.host_off()
    }

    pub async fn host_update(&self) -> ActionResult {
        self.host_off()
    }

    pub async fn host_set_multipliers(&self, _multipliers: Option<HostMultipliers>) -> ActionResult {
        self.host_off()
    }
}
