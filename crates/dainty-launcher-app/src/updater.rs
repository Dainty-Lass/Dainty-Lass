// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Self-update (RD27 spec §2.7.12) from the `launcher-updates` release (L7). Release builds supply the public
//! key; without one, or outside a bundle, nothing is ever checked.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use dainty_launcher_kit::bridge::types::{ActionResult, ErrorCode, LauncherError};
use dainty_launcher_kit::controller::Controller;
use dainty_launcher_kit::platform::log;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// Whether this executable can update itself: a bundle (any Windows build, or a Linux bundle type) whose config
/// carries a public key.
pub fn updatable(windows: bool, bundle: Option<BundleType>, pubkey: Option<&str>) -> bool {
    let bundled = windows || matches!(bundle, Some(BundleType::Deb | BundleType::Rpm | BundleType::AppImage));
    bundled && pubkey.is_some_and(|key| !key.trim().is_empty())
}

fn configured_pubkey(app: &AppHandle) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The update found and not yet installed, managed by Tauri.
#[derive(Default)]
pub struct Updates {
    waiting: Mutex<Option<Update>>,
}

impl Updates {
    fn put(&self, update: Update) {
        *self.waiting.lock().unwrap_or_else(PoisonError::into_inner) = Some(update);
    }

    fn take(&self) -> Option<Update> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// Checks now and every hour, marking the snapshot when an update waits.
pub fn start(app: &AppHandle, controller: Controller) {
    let pubkey = configured_pubkey(app);
    if !updatable(cfg!(windows), tauri::utils::platform::bundle_type(), pubkey.as_deref()) {
        log::info("self-update is off: not a bundle, or no updater key in this build");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            check(&app, &controller).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

async fn check(app: &AppHandle, controller: &Controller) {
    let found = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };
    match found {
        Ok(Some(update)) => {
            log::info(format!("launcher update {} is ready", update.version));
            app.state::<Updates>().put(update);
            controller.set_update_ready(true);
        }
        Ok(None) => {}
        Err(error) => log::warn(format!("could not check for a launcher update: {error}")),
    }
}

/// `install_update`: installs the waiting update when the controller allows it, then restarts.
pub async fn install(app: &AppHandle, controller: &Controller) {
    if !controller.may_install_update() {
        return;
    }
    let Some(update) = app.state::<Updates>().take() else {
        return;
    };
    let version = update.version.clone();
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            log::info(format!("installed launcher update {version}; restarting"));
            app.restart();
        }
        Err(error) => {
            log::warn(format!("could not install launcher update {version}: {error}"));
            controller.set_update_ready(false);
        }
    }
}

/// `update_launcher`: the server asks for a newer launcher, so check and install now rather than on the hour,
/// then restart. Any failure answers `launcher_update_failed`, and the page tells the player to update by hand.
pub async fn update_now(app: &AppHandle, controller: &Controller) -> ActionResult {
    let Some(minimum) = controller.launcher_too_old() else {
        return ActionResult::Ok;
    };
    let failed = || ActionResult::Err(LauncherError::with_detail(ErrorCode::LauncherUpdateFailed, &minimum));
    if !controller.may_update_now() {
        return ActionResult::err(ErrorCode::Busy);
    }
    let pubkey = configured_pubkey(app);
    if !updatable(cfg!(windows), tauri::utils::platform::bundle_type(), pubkey.as_deref()) {
        log::warn(format!(
            "the server asks for launcher {minimum}, and this build cannot update itself"
        ));
        return failed();
    }
    let found = match app.state::<Updates>().take() {
        Some(update) => Ok(Some(update)),
        None => match app.updater() {
            Ok(updater) => updater.check().await,
            Err(error) => Err(error),
        },
    };
    let update = match found {
        Ok(Some(update)) => update,
        Ok(None) => {
            log::warn(format!(
                "the server asks for launcher {minimum}, and the update feed has nothing newer"
            ));
            return failed();
        }
        Err(error) => {
            log::warn(format!("could not check for a launcher update: {error}"));
            return failed();
        }
    };
    let version = update.version.clone();
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            log::info(format!(
                "installed launcher update {version} the server asks for; restarting"
            ));
            app.restart();
        }
        Err(error) => {
            log::warn(format!("could not install launcher update {version}: {error}"));
            controller.set_update_ready(false);
            failed()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_only_a_signed_bundle() {
        assert!(updatable(true, None, Some("key")));
        assert!(updatable(false, Some(BundleType::AppImage), Some("key")));
        assert!(updatable(false, Some(BundleType::Deb), Some("key")));
        assert!(updatable(false, Some(BundleType::Rpm), Some("key")));
        assert!(!updatable(false, None, Some("key")));
        assert!(!updatable(false, Some(BundleType::Msi), Some("key")));
        assert!(!updatable(true, None, Some("")));
        assert!(!updatable(true, None, Some("  ")));
        assert!(!updatable(true, Some(BundleType::Nsis), None));
    }

    #[test]
    fn the_repository_config_has_no_key_and_the_l7_feed() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap_or(serde_json::Value::Null);
        let updater = &config["plugins"]["updater"];
        assert_eq!(updater["pubkey"], "");
        assert_eq!(
            updater["endpoints"],
            serde_json::json!([
                "https://github.com/SilentSpud/DaintyLass/releases/download/launcher-updates/latest.json"
            ])
        );
        assert_eq!(config["bundle"]["license"], "MIT");
    }
}
