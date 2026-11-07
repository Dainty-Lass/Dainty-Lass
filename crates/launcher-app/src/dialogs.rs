// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The backend's own dialogs (RD27 spec §2.7.11): the pickers, parented to the main window, and the keep-open
//! question. Their text is launcher-kit's MIT catalog (L1).

use std::path::PathBuf;

use launcher_kit::controller::{FileFilter, FolderPurpose};
use launcher_kit::core::i18n::{self, Language};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::{
    DialogExt, FileDialogBuilder, FilePath, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};
use tokio::sync::oneshot;

use crate::window::MAIN;

/// The Dumper-7 DLL picker's filter.
pub const DLL_FILTER: FileFilter = FileFilter {
    name: "DLL",
    extension: "dll",
};
/// The game data pack picker's filter.
pub const GAMEDATA_FILTER: FileFilter = FileFilter {
    name: "Game data pack",
    extension: "gz",
};

/// The catalog key titling a folder picker.
pub const fn folder_title_key(purpose: FolderPurpose) -> &'static str {
    match purpose {
        FolderPurpose::Game => i18n::DIALOG_GAME_FOLDER,
        FolderPurpose::Runtime => i18n::DIALOG_RUNTIME_FOLDER,
        FolderPurpose::Workspace => i18n::DIALOG_WORKSPACE_FOLDER,
        FolderPurpose::Sdk => i18n::DIALOG_SDK_FOLDER,
    }
}

/// The keep-open dialog's text in one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepOpenText {
    /// The window title.
    pub title: String,
    /// The title line, a blank line and the question.
    pub message: String,
    /// The button that keeps the launcher open.
    pub keep: String,
    /// The button that closes it.
    pub anyway: String,
}

impl KeepOpenText {
    /// The text in `language`.
    pub fn new(language: Language) -> Self {
        let t = |key| i18n::translate(language, key, &[]);
        Self {
            title: t(i18n::APP_TITLE),
            message: format!("{}\n\n{}", t(i18n::CLOSE_TITLE), t(i18n::CLOSE_TEXT)),
            keep: t(i18n::CLOSE_KEEP),
            anyway: t(i18n::CLOSE_ANYWAY),
        }
    }

    /// Whether the player chose to close. Only the close button closes: Esc, the dialog's own close button and
    /// anything unexpected keep the launcher open.
    pub fn closes(&self, result: &MessageDialogResult) -> bool {
        match result {
            MessageDialogResult::Custom(label) => *label == self.anyway,
            // rfd answers Ok for the first custom button on some desktops, which is "close anyway" (`buttons`)
            MessageDialogResult::Ok => true,
            _ => false,
        }
    }

    /// The buttons. "Close anyway" is the first so that every way of dismissing the dialog lands on "keep":
    /// on Linux rfd maps Cancel to the second label.
    pub fn buttons(&self) -> MessageDialogButtons {
        MessageDialogButtons::OkCancelCustom(self.anyway.clone(), self.keep.clone())
    }
}

/// Asks whether to close while the relay carries a game; `then(true)` closes.
pub fn ask_keep_open(window: &WebviewWindow, language: Language, then: impl FnOnce(bool) + Send + 'static) {
    let text = KeepOpenText::new(language);
    window
        .dialog()
        .message(text.message.clone())
        .title(text.title.clone())
        .kind(MessageDialogKind::Warning)
        .buttons(text.buttons())
        .parent(window)
        .show_with_result(move |result| then(text.closes(&result)));
}

fn builder(app: &AppHandle, title: &str) -> FileDialogBuilder<tauri::Wry> {
    let builder = app.dialog().file().set_title(title);
    match app.get_webview_window(MAIN) {
        Some(window) => builder.set_parent(&window),
        None => builder,
    }
}

// Sync, so the builder (not `Send` on every OS) is gone before anything awaits
fn ask(pick: impl FnOnce(oneshot::Sender<Option<FilePath>>)) -> oneshot::Receiver<Option<FilePath>> {
    let (tx, rx) = oneshot::channel();
    pick(tx);
    rx
}

async fn answer(rx: oneshot::Receiver<Option<FilePath>>) -> Option<PathBuf> {
    rx.await.ok().flatten().and_then(|path| path.into_path().ok())
}

/// A folder picker titled for `purpose`, starting at `start` when it exists.
pub async fn pick_folder(
    app: &AppHandle,
    language: Language,
    purpose: FolderPurpose,
    start: Option<PathBuf>,
) -> Option<PathBuf> {
    let mut builder = builder(app, &i18n::translate(language, folder_title_key(purpose), &[]));
    if let Some(start) = start.filter(|dir| dir.is_dir()) {
        builder = builder.set_directory(start);
    }
    let rx = ask(|tx| {
        builder.pick_folder(move |path| {
            let _ = tx.send(path);
        });
    });
    answer(rx).await
}

/// A save picker in Documents with `default_name`.
pub async fn save_file(app: &AppHandle, default_name: String, filter: FileFilter) -> Option<PathBuf> {
    let mut builder = builder(app, "")
        .set_file_name(default_name)
        .add_filter(filter.name, &[filter.extension]);
    if let Ok(documents) = app.path().document_dir() {
        builder = builder.set_directory(documents);
    }
    let rx = ask(|tx| {
        builder.save_file(move |path| {
            let _ = tx.send(path);
        });
    });
    answer(rx).await
}

/// An open picker titled `title` (none for the key file), starting at `start` or else in Documents.
pub async fn open_file(
    app: &AppHandle,
    title: Option<String>,
    filter: FileFilter,
    start: Option<PathBuf>,
) -> Option<PathBuf> {
    let mut builder = builder(app, title.as_deref().unwrap_or_default()).add_filter(filter.name, &[filter.extension]);
    if let Some(dir) = start.or_else(|| app.path().document_dir().ok()) {
        builder = builder.set_directory(dir);
    }
    let rx = ask(|tx| {
        builder.pick_file(move |path| {
            let _ = tx.send(path);
        });
    });
    answer(rx).await
}

/// The Dumper-7 DLL picker.
pub async fn dumper_dll(app: &AppHandle, language: Language) -> Option<PathBuf> {
    let title = i18n::translate(language, i18n::DIALOG_DUMPER_DLL, &[]);
    open_file(app, Some(title), DLL_FILTER, None).await
}

/// The game data pack picker.
pub async fn gamedata_pack(app: &AppHandle, language: Language) -> Option<PathBuf> {
    let title = i18n::translate(language, i18n::DIALOG_GAMEDATA_PACK, &[]);
    open_file(app, Some(title), GAMEDATA_FILTER, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dialog_string_exists_in_every_language() {
        for language in Language::ALL {
            for key in i18n::KEYS {
                let text = i18n::translate(language, key, &[]);
                assert!(!text.is_empty() && text != key, "{key} in {}", language.code());
            }
            for purpose in [
                FolderPurpose::Game,
                FolderPurpose::Runtime,
                FolderPurpose::Workspace,
                FolderPurpose::Sdk,
            ] {
                assert!(i18n::KEYS.contains(&folder_title_key(purpose)));
            }
        }
    }

    #[test]
    fn keep_open_text_is_the_catalogs() {
        for language in Language::ALL {
            let text = KeepOpenText::new(language);
            let t = |key| i18n::translate(language, key, &[]);
            assert_eq!(text.title, t(i18n::APP_TITLE));
            assert_eq!(
                text.message,
                format!("{}\n\n{}", t(i18n::CLOSE_TITLE), t(i18n::CLOSE_TEXT))
            );
            assert_eq!(text.keep, t(i18n::CLOSE_KEEP));
            assert_eq!(text.anyway, t(i18n::CLOSE_ANYWAY));
            assert_ne!(text.keep, text.anyway);
        }
    }

    #[test]
    fn only_close_anyway_closes() {
        let text = KeepOpenText::new(Language::En);
        assert!(text.closes(&MessageDialogResult::Custom(text.anyway.clone())));
        assert!(text.closes(&MessageDialogResult::Ok));
        assert!(!text.closes(&MessageDialogResult::Custom(text.keep.clone())));
        assert!(!text.closes(&MessageDialogResult::Cancel));
        assert!(!text.closes(&MessageDialogResult::No));
        assert!(!text.closes(&MessageDialogResult::Custom("something else".to_owned())));
        assert!(matches!(
            text.buttons(),
            MessageDialogButtons::OkCancelCustom(first, second) if first == text.anyway && second == text.keep
        ));
    }
}
