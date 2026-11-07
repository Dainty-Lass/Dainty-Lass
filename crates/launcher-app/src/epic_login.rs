// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Stand-in for the Epic sign-in window (Category A, moved in later): no window, so the page offers the paste
//! fallback (`epic_sign_in_code`).

use std::sync::Arc;

use launcher_kit::bridge::checks::not_in_this_build;
use launcher_kit::bridge::types::ActionResult;
use launcher_kit::core::i18n::Language;
use launcher_kit::platform::epic_session::Epic;
use tauri::AppHandle;

/// Opens the sign-in window, titled `epic_sign_in_title`.
pub fn open(app: &AppHandle, epic: &Arc<Epic>, language: Language) -> ActionResult {
    let _ = (app, epic, language);
    ActionResult::Err(not_in_this_build("epic_sign_in_start"))
}

/// Closes the sign-in window, if open.
pub fn close(app: &AppHandle, epic: &Arc<Epic>) {
    let _ = (app, epic);
}
