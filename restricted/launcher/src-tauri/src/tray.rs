//! The tray icon, there only while the window is hidden to it: minimising or closing with the player's
//! `minimizeToTray` / `closeToTray` settings hides the window here, and the icon goes when it comes back.

use std::sync::atomic::Ordering;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::core::i18n::{self, translate};
use crate::window::{CloseGuard, MAIN};

const ID: &str = "main";
const OPEN: &str = "tray-open";
const QUIT: &str = "tray-quit";

/// Hides the window to the tray, making the icon if it isn't there.
pub fn hide(window: &WebviewWindow) {
    let app = window.app_handle();
    if app.tray_by_id(ID).is_none()
        && let Err(error) = build(app)
    {
        // No tray (a Linux desktop without one): leave the window where the player can find it
        crate::platform::log::info(&format!("no tray icon: {error}"));
        return;
    }
    let _ = window.hide();
}

/// Brings the window back from the tray, or forward if it never left.
pub fn restore(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    let _ = app.remove_tray_by_id(ID);
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let language = app.state::<crate::controller::ControllerHandle>().language();
    let open = MenuItem::with_id(app, OPEN, translate(language, i18n::TRAY_OPEN, &[]), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, translate(language, i18n::TRAY_QUIT, &[]), true, None::<&str>)?;
    let mut builder = TrayIconBuilder::with_id(ID)
        .tooltip(translate(language, i18n::APP_TITLE, &[]))
        .menu(&Menu::with_items(app, &[&open, &quit])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN => restore(app),
            QUIT => quit_from_tray(app),
            _ => {}
        })
        .on_tray_icon_event(|icon, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                restore(icon.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Quit closes for real: past close-to-tray, but still through the running game's confirmation (row L5),
/// which needs the window on screen.
fn quit_from_tray(app: &AppHandle) {
    app.state::<CloseGuard>().quitting.store(true, Ordering::SeqCst);
    restore(app);
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.close();
    }
}
