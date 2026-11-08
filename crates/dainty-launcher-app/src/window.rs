// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The one window (RD27 spec §2.2): its size, the navigation lock, the close guard and the tray settings.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dainty_launcher_kit::controller::{Controller, KeepOpen};
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::{Flavour, dialogs, tray};

/// The window's label; every event goes to it alone.
pub const MAIN: &str = "main";
/// Shown even when the page never says it loaded.
const SHOW_AFTER: Duration = Duration::from_secs(10);
const SIZE: (f64, f64) = (1280.0, 760.0);
const MIN_SIZE: (f64, f64) = (1024.0, 640.0);
// Matches the page's background, so nothing flashes before it draws
const BACKGROUND: Color = Color(0x03, 0x08, 0x0F, 0xFF);

/// The window's size and minimum for a work area in logical pixels; the defaults with no monitor.
pub fn fit(work_area: Option<(f64, f64)>) -> ((f64, f64), (f64, f64)) {
    match work_area {
        Some((width, height)) => (
            (width.min(SIZE.0), height.min(SIZE.1)),
            (width.min(MIN_SIZE.0), height.min(MIN_SIZE.1)),
        ),
        None => (SIZE, MIN_SIZE),
    }
}

/// Whether the webview may go to `url`: the app's own page only, or the dev server in a development build.
pub fn navigation_allowed(url: &Url, dev_url: Option<&Url>) -> bool {
    if let Some(dev) = dev_url {
        return url.origin() == dev.origin();
    }
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("tauri.localhost"),
        _ => false,
    }
}

/// What a close request does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Let it close.
    Close,
    /// Keep the window, hidden to the tray; the relay keeps running.
    HideToTray,
    /// Keep it and ask (the keep-open dialog).
    Ask(KeepOpen),
}

/// The close guard's choice. A close the tray's Quit asked for skips the tray but still faces the dialog.
pub const fn on_close(
    quitting: bool,
    close_to_tray: bool,
    confirmed: bool,
    keep_open: Option<KeepOpen>,
) -> CloseDecision {
    if !quitting && close_to_tray {
        return CloseDecision::HideToTray;
    }
    match keep_open {
        Some(reason) if !confirmed => CloseDecision::Ask(reason),
        _ => CloseDecision::Close,
    }
}

/// Whether a resize that left the window `minimized` hides it to the tray (and emits nothing else).
pub const fn hides_on_resize(minimized: bool, minimize_to_tray: bool) -> bool {
    minimized && minimize_to_tray
}

/// The icon a build handed in, managed by Tauri; none keeps the config's.
pub struct Icon(pub Option<tauri::image::Image<'static>>);

/// The icon for the window and the tray.
pub fn icon(app: &AppHandle) -> Option<tauri::image::Image<'static>> {
    app.try_state::<Icon>()
        .and_then(|icon| icon.0.clone())
        .or_else(|| app.default_window_icon().map(|icon| icon.clone().to_owned()))
}

/// The close guard's state, managed by Tauri.
#[derive(Default)]
pub struct CloseGuard {
    confirmed: AtomicBool,
    quitting: AtomicBool,
    controller: OnceLock<Controller>,
}

impl CloseGuard {
    /// Wires the guard to the controller; until then the tray settings are off and nothing keeps the window.
    pub fn attach(&self, controller: Controller) {
        let _ = self.controller.set(controller);
    }

    /// The tray's Quit: the next close skips the tray (read once).
    pub fn set_quitting(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }

    fn take_quitting(&self) -> bool {
        self.quitting.swap(false, Ordering::SeqCst)
    }

    fn tray(&self) -> (bool, bool) {
        self.controller.get().map_or((false, false), Controller::tray)
    }

    fn decide(&self) -> CloseDecision {
        let quitting = self.take_quitting();
        let (_, close_to_tray) = self.tray();
        let keep_open = self.controller.get().and_then(Controller::keep_open);
        on_close(
            quitting,
            close_to_tray,
            self.confirmed.load(Ordering::SeqCst),
            keep_open,
        )
    }
}

/// Makes the window, hidden until the page loads (§2.2).
pub fn create(app: &AppHandle, flavour: Flavour, data_dir: Option<PathBuf>) -> tauri::Result<WebviewWindow> {
    let work_area = app.primary_monitor().ok().flatten().map(|monitor| {
        let size = monitor.work_area().size.to_logical::<f64>(monitor.scale_factor());
        (size.width, size.height)
    });
    let ((width, height), (min_width, min_height)) = fit(work_area);
    let dev_url = if tauri::is_dev() {
        app.config().build.dev_url.clone()
    } else {
        None
    };

    let mut builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title(flavour.window_title())
        .decorations(false)
        .visible(false)
        .background_color(BACKGROUND)
        .inner_size(width, height)
        .min_inner_size(min_width, min_height)
        .center()
        .on_navigation(move |url| navigation_allowed(url, dev_url.as_ref()))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = window.show();
            }
        });
    if let Some(dir) = data_dir {
        builder = builder.data_directory(dir);
    }
    if let Some(icon) = app.try_state::<Icon>().and_then(|icon| icon.0.clone()) {
        builder = builder.icon(icon)?;
    }
    let window = builder.build()?;

    let late = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_AFTER).await;
        if late.is_visible().is_ok_and(|visible| !visible) {
            let _ = late.show();
        }
    });

    let handle = window.clone();
    window.on_window_event(move |event| on_event(&handle, event));
    Ok(window)
}

fn on_event(window: &WebviewWindow, event: &WindowEvent) {
    let Some(guard) = window.try_state::<Arc<CloseGuard>>() else {
        return;
    };
    match event {
        WindowEvent::Resized(_) => {
            let (minimize_to_tray, _) = guard.tray();
            if hides_on_resize(window.is_minimized().unwrap_or(false), minimize_to_tray) {
                tray::hide(window);
                return;
            }
            let _ = window.emit_to(MAIN, "window-state", window.is_maximized().unwrap_or(false));
        }
        WindowEvent::CloseRequested { api, .. } => match guard.decide() {
            CloseDecision::Close => {}
            CloseDecision::HideToTray => {
                api.prevent_close();
                tray::hide(window);
            }
            CloseDecision::Ask(reason) => {
                api.prevent_close();
                let Some(controller) = guard.controller.get().cloned() else {
                    return;
                };
                let guard = Arc::clone(&guard);
                let target = window.clone();
                dialogs::ask_keep_open(window, controller.language(), move |close| {
                    if close {
                        guard.confirmed.store(true, Ordering::SeqCst);
                        let _ = target.close();
                    } else {
                        let _ = target.minimize();
                        controller.kept_open(reason);
                    }
                });
            }
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn fits_the_work_area() {
        assert_eq!(fit(None), ((1280.0, 760.0), (1024.0, 640.0)));
        assert_eq!(fit(Some((1920.0, 1040.0))), ((1280.0, 760.0), (1024.0, 640.0)));
        assert_eq!(fit(Some((1100.0, 700.0))), ((1100.0, 700.0), (1024.0, 640.0)));
        assert_eq!(fit(Some((800.0, 600.0))), ((800.0, 600.0), (800.0, 600.0)));
    }

    #[test]
    fn navigation_stays_on_the_app() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(navigation_allowed(&url("tauri://localhost/index.html"), None));
        assert!(navigation_allowed(&url("http://tauri.localhost/"), None));
        assert!(navigation_allowed(&url("https://tauri.localhost/x"), None));
        assert!(!navigation_allowed(&url("https://tauri.localhost.evil.com/"), None));
        assert!(!navigation_allowed(&url("https://example.com/"), None));
        assert!(!navigation_allowed(&url("tauri://other/"), None));
        assert!(!navigation_allowed(&url("http://localhost:4321/"), None));

        let dev = url("http://localhost:4321");
        assert!(navigation_allowed(&url("http://localhost:4321/launcher"), Some(&dev)));
        assert!(!navigation_allowed(&url("http://localhost:4322/"), Some(&dev)));
        assert!(!navigation_allowed(&url("tauri://localhost/"), Some(&dev)));
    }

    #[test]
    fn close_guard_decisions() {
        use CloseDecision::{Ask, Close, HideToTray};
        let relay = Some(KeepOpen::Relay);
        assert_eq!(on_close(false, false, false, None), Close);
        assert_eq!(on_close(false, false, false, relay), Ask(KeepOpen::Relay));
        assert_eq!(on_close(false, false, true, relay), Close);
        // Close to tray wins, and keeps the relay without asking
        assert_eq!(on_close(false, true, false, relay), HideToTray);
        assert_eq!(on_close(false, true, false, None), HideToTray);
        // The tray's Quit still faces the dialog
        assert_eq!(on_close(true, true, false, relay), Ask(KeepOpen::Relay));
        assert_eq!(on_close(true, true, false, None), Close);
    }

    #[test]
    fn quitting_is_read_once() {
        let guard = CloseGuard::default();
        assert_eq!(guard.tray(), (false, false));
        guard.set_quitting();
        assert!(guard.take_quitting());
        assert!(!guard.take_quitting());
        assert_eq!(guard.decide(), CloseDecision::Close);
    }

    #[test]
    fn only_a_minimised_window_hides_on_resize() {
        assert!(hides_on_resize(true, true));
        assert!(!hides_on_resize(true, false));
        assert!(!hides_on_resize(false, true));
    }
}
