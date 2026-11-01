//! Self-update (row L46, RD7). Every bundle (the Windows installer, the AppImage, the deb and the rpm) updates
//! itself with `tauri-plugin-updater` from the `launcher-updates` release's feed, and a bundle whose ed25519
//! signature is not this launcher's key (the `pubkey` in tauri.conf.json) is refused. DR
//! fetched deb and rpm updates unsigned from GitHub's releases API; the plugin installs them through `pkexec`
//! as DR did, but signed (RA14). Packaged builds look once at start and then every hour, as the Electron
//! launcher did; nothing installs until the player presses the update button (`installUpdate`).

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::controller::ControllerHandle;
use crate::platform::log;

const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// The update found last, waiting for the player.
#[derive(Default)]
pub struct Updates {
    pending: tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>,
}

/// Whether this build can update itself: a bundle (a bare `cargo build` binary has no bundle type on Linux,
/// and the plugin would replace it as an AppImage) built with the updater key.
fn updatable(app: &AppHandle) -> bool {
    let bundled = cfg!(windows) || tauri::utils::platform::bundle_type().is_some();
    let keyed = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|key| !key.trim().is_empty());
    bundled && keyed
}

async fn check(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    app.updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())
}

/// Looks now and then every hour; marks the controller's snapshot when an update waits.
pub fn start(app: AppHandle, controller: ControllerHandle) {
    if !updatable(&app) {
        log::info("update: this build has no updater key or bundle; not checking for updates");
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            match check(&app).await {
                Ok(Some(update)) => {
                    log::info(&format!("update: launcher {} is ready to install", update.version));
                    *app.state::<Arc<Updates>>().pending.lock().await = Some(update);
                    controller.set_update_ready();
                }
                Ok(None) => {}
                Err(error) => log::warn(&format!("update: check failed: {error}")),
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// The player pressed the update button: installs the waiting update, then the launcher restarts (the Windows
/// installer closes it itself; deb and rpm ask for the password through `pkexec` first).
pub async fn install(app: AppHandle, controller: ControllerHandle) {
    if !controller.may_install_update() {
        return;
    }
    let Some(update) = app.state::<Arc<Updates>>().pending.lock().await.take() else {
        return;
    };
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            log::info(&format!("update: launcher {} installed; restarting", update.version));
            app.restart();
        }
        Err(error) => log::error(&format!("update: install failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use crate::platform::links::PROJECT_URL;

    const CLIENT: &str = include_str!("../../tauri.conf.json");
    const MANAGER: &str = include_str!("../../tauri.server-gui.conf.json");

    fn endpoints(config: &str) -> Vec<String> {
        let config: serde_json::Value = serde_json::from_str(config).unwrap();
        config["plugins"]["updater"]["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|endpoint| endpoint.as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn each_app_reads_its_own_feed_from_the_project_release() {
        let feed = format!("{PROJECT_URL}/releases/download/launcher-updates/");
        let (client, manager) = (endpoints(CLIENT), endpoints(MANAGER));
        assert_eq!(client, [format!("{feed}latest.json")]);
        // The manager must never take the client's installer for itself
        assert_eq!(manager, [format!("{feed}latest-manager.json")]);
    }
}
