// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Dialog purpose identifiers, translation keys, and file-filter constants (Spec §2.6).

/// Purpose of a folder-picker dialog, mapped to its localization key via [`folder_title`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderPurpose {
    /// Picking the Dauntless install folder.
    Game,
    /// Picking a local runtime folder.
    Runtime,
    /// Picking a Dumper-7 SDK folder.
    Sdk,
    /// Picking the host workspace.
    Workspace,
    /// Picking the folder Epic installs go into, one version folder each.
    GameInstall,
}

/// Reason the launcher warns before closing the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepOpen {
    /// The local relay is running and would stop with the window.
    Relay,
}

/// Catalog key of the game folder picker's title.
pub const DIALOG_GAME_FOLDER: &str = "dialog_game_folder";
/// Catalog key of the runtime folder picker's title.
pub const DIALOG_RUNTIME_FOLDER: &str = "dialog_runtime_folder";
/// Catalog key of the SDK folder picker's title.
pub const DIALOG_SDK_FOLDER: &str = "dialog_sdk_folder";
/// Catalog key of the workspace folder picker's title.
pub const DIALOG_WORKSPACE_FOLDER: &str = "dialog_workspace_folder";
/// Catalog key of the Epic install folder picker's title.
pub const DIALOG_GAME_INSTALL_FOLDER: &str = "dialog_game_install_folder";
/// Catalog key of the Dumper-7 DLL picker's title.
pub const DIALOG_DUMPER_DLL: &str = "dialog_dumper_dll";
/// Catalog key of the game-data pack picker's title.
pub const DIALOG_GAMEDATA_PACK: &str = "dialog_gamedata_pack";
/// Catalog key of the close warning's title while the game runs.
pub const DIALOG_GAME_RUNNING_TITLE: &str = "close_title";
/// Catalog key of the close warning's message.
pub const DIALOG_GAME_RUNNING_MESSAGE: &str = "close_text";
/// Catalog key of the close warning's detail; the same text as the message.
pub const DIALOG_GAME_RUNNING_DETAIL: &str = "close_text";
/// Catalog key of the button that keeps the window open.
pub const DIALOG_KEEP_OPEN: &str = "close_keep";
/// Catalog key of the button that closes anyway.
pub const DIALOG_CLOSE_ANYWAY: &str = "close_anyway";

/// File filter for text files: label and extension.
pub const TEXT_FILES: (&str, &str) = ("Text", "txt");
/// File filter for DLLs: label and extension.
pub const DLL_FILES: (&str, &str) = ("DLL", "dll");
/// File filter for game-data packs: label and extension.
pub const GAMEDATA_PACKS: (&str, &str) = ("Game data pack", "gz");

/// Returns the localization string key for the given [`FolderPurpose`].
#[must_use]
pub const fn folder_title(purpose: FolderPurpose) -> &'static str {
    match purpose {
        FolderPurpose::Game => DIALOG_GAME_FOLDER,
        FolderPurpose::Runtime => DIALOG_RUNTIME_FOLDER,
        FolderPurpose::Sdk => DIALOG_SDK_FOLDER,
        FolderPurpose::Workspace => DIALOG_WORKSPACE_FOLDER,
        FolderPurpose::GameInstall => DIALOG_GAME_INSTALL_FOLDER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_titles_map_each_purpose_to_its_key() {
        assert_eq!(folder_title(FolderPurpose::Game), "dialog_game_folder");
        assert_eq!(folder_title(FolderPurpose::GameInstall), "dialog_game_install_folder");
        assert_eq!(folder_title(FolderPurpose::Runtime), "dialog_runtime_folder");
        assert_eq!(folder_title(FolderPurpose::Sdk), "dialog_sdk_folder");
        assert_eq!(folder_title(FolderPurpose::Workspace), "dialog_workspace_folder");
    }
}
