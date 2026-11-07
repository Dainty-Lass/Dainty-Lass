// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The controller's [`Platform`] over Tauri (RD27 spec §2.4.1): dialogs, events to the main window, the art
//! scheme and the game's console reader. Everything else keeps launcher-kit's defaults.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use launcher_kit::bridge::types::{HostKit, HostServerView, Snapshot, TaskProgress};
use launcher_kit::controller::{ConsoleSink, FileFilter, FolderPurpose, Platform};
use launcher_kit::core::i18n::Language;
use launcher_kit::net::hostapi::ImageType;
use launcher_kit::platform::game_console;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::art::ArtStore;
use crate::bridge::backend::Backend;
use crate::dialogs;
use crate::window::MAIN;

/// The app as the controller sees it.
pub struct AppPlatform {
    app: AppHandle,
    art: Arc<ArtStore>,
    backend: Arc<Backend>,
}

impl AppPlatform {
    /// The platform over `app`.
    pub const fn new(app: AppHandle, art: Arc<ArtStore>, backend: Arc<Backend>) -> Self {
        Self { app, art, backend }
    }

    fn emit<T: Serialize + Clone>(&self, event: &str, payload: T) {
        let _ = self.app.emit_to(MAIN, event, payload);
    }
}

impl Platform for AppPlatform {
    fn choose_folder(
        &self,
        language: Language,
        purpose: FolderPurpose,
        start: Option<PathBuf>,
    ) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(dialogs::pick_folder(&self.app, language, purpose, start))
    }

    fn choose_save_file(
        &self,
        _language: Language,
        default_name: String,
        filter: FileFilter,
    ) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(dialogs::save_file(&self.app, default_name, filter))
    }

    fn choose_open_file(&self, _language: Language, filter: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(dialogs::open_file(&self.app, None, filter, None))
    }

    fn choose_dumper_dll(&self, language: Language) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(dialogs::dumper_dll(&self.app, language))
    }

    fn choose_gamedata_pack(&self, language: Language) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(dialogs::gamedata_pack(&self.app, language))
    }

    fn emit_snapshot(&self, snapshot: &Snapshot) {
        self.emit("snapshot", snapshot.clone());
    }

    fn emit_progress(&self, progress: &TaskProgress) {
        self.backend.progress(progress);
    }

    fn emit_host_kit(&self, kit: &HostKit) {
        self.emit("host-kit", kit.clone());
    }

    // Never logged: the view may carry the newest invite link
    fn emit_host_server(&self, view: &HostServerView) {
        self.emit("host-server", view.clone());
    }

    fn register_art(&self, id: &str, file: &Path, kind: ImageType) {
        self.art.insert(id, file, kind);
    }

    fn watch_game_console(&self, pid: u32, hide: bool, sink: ConsoleSink) {
        game_console::watch(pid, hide, Box::new(move |lines| sink(lines)));
    }

    fn emit_game_log(&self, lines: &[String]) {
        self.emit("game-log", lines.to_vec());
    }
}
