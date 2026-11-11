//! The AGPL half of the DaintyLass launcher (RD27 spec §4.2, §4.4). The app is dainty-launcher-app's (MIT); this crate
//! hands it today's icons (L2) and, with `server-gui`, the MIT embedded server (`dainty-manager-server`). Its tests
//! hold dainty-launcher-kit to DR's recorded tables (§3.2), drive the kit's controller, relay and modules as the
//! shell's own tests did, and are the oracle for the manager's embedded server (spec-tauri-shell §12.1).

/// What this build hands dainty-launcher-app: the icons and, in the manager, the embedded server.
pub fn options() -> dainty_launcher_app::Options {
    #[cfg(feature = "server-gui")]
    {
        let logs = dainty_manager_server::init_telemetry();
        dainty_launcher_app::Options {
            embedded_server: Some(std::sync::Arc::new(dainty_manager_server::EmbeddedServerState::new(
                logs,
            ))),
            icon: Some(tauri::include_image!("icons/server/32x32.png")),
        }
    }
    #[cfg(not(feature = "server-gui"))]
    {
        dainty_launcher_app::Options {
            icon: Some(tauri::include_image!("icons/32x32.png")),
        }
    }
}
