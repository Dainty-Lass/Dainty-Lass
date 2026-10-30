// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Dialog purpose identifiers, translation keys, and file-filter constants (Spec §2.6).

/// Purpose of a folder-picker dialog, mapped to its localization key via [`folder_title`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderPurpose {
    Game,
    Runtime,
    Sdk,
    Workspace,
    #[cfg(feature = "legacy")]
    Revived,
}

/// Reason the launcher warns before closing the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepOpen {
    Relay,
}

pub const DIALOG_GAME_FOLDER: &str = "dialog_game_folder";
pub const DIALOG_LEGACY_FOLDER: &str = "dialog_legacy_folder";
pub const DIALOG_REVIVED_FOLDER: &str = DIALOG_LEGACY_FOLDER;
pub const DIALOG_RUNTIME_FOLDER: &str = "dialog_runtime_folder";
pub const DIALOG_SDK_FOLDER: &str = "dialog_sdk_folder";
pub const DIALOG_WORKSPACE_FOLDER: &str = "dialog_workspace_folder";
pub const DIALOG_DUMPER_DLL: &str = "dialog_dumper_dll";
pub const DIALOG_GAMEDATA_PACK: &str = "dialog_gamedata_pack";
pub const DIALOG_GAME_RUNNING_TITLE: &str = "close_title";
pub const DIALOG_GAME_RUNNING_MESSAGE: &str = "close_text";
pub const DIALOG_GAME_RUNNING_DETAIL: &str = "close_text";
pub const DIALOG_KEEP_OPEN: &str = "close_keep";
pub const DIALOG_CLOSE_ANYWAY: &str = "close_anyway";

pub const TEXT_FILES: (&str, &str) = ("Text", "txt");
pub const DLL_FILES: (&str, &str) = ("DLL", "dll");
pub const GAMEDATA_PACKS: (&str, &str) = ("Game data pack", "gz");

/// Returns the localization string key for the given [`FolderPurpose`].
#[must_use]
pub const fn folder_title(purpose: FolderPurpose) -> &'static str {
    match purpose {
        FolderPurpose::Game => DIALOG_GAME_FOLDER,
        FolderPurpose::Runtime => DIALOG_RUNTIME_FOLDER,
        FolderPurpose::Sdk => DIALOG_SDK_FOLDER,
        FolderPurpose::Workspace => DIALOG_WORKSPACE_FOLDER,
        #[cfg(feature = "legacy")]
        FolderPurpose::Revived => DIALOG_REVIVED_FOLDER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_titles_map_each_purpose_to_its_key() {
        assert_eq!(folder_title(FolderPurpose::Game), "dialog_game_folder");
        assert_eq!(folder_title(FolderPurpose::Runtime), "dialog_runtime_folder");
        assert_eq!(folder_title(FolderPurpose::Sdk), "dialog_sdk_folder");
        assert_eq!(folder_title(FolderPurpose::Workspace), "dialog_workspace_folder");
        #[cfg(feature = "legacy")]
        assert_eq!(folder_title(FolderPurpose::Revived), "dialog_legacy_folder");
    }
}
