//! The Epic sign-in window (DD22, DE2): a second webview on `epic::auth::LOGIN_URL`. Once the player has signed
//! in, Epic shows a JSON page with the authorization code; a script on that page alone hands the code over by
//! navigating to `https://epic-sign-in.invalid/#<code>`, a navigation the hook below reads and cancels, so the code
//! never leaves the process. The window is private (no cookies kept), has no bridge (the capability is the main
//! window's only), opens no popups and loads nothing but HTTPS.

use std::sync::{Arc, Mutex};

use ::epic::auth::{AuthorizationCode, LOGIN_URL, REDIRECT_ORIGIN, REDIRECT_PATH};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use url::Url;

use super::epic_session::Epic;
use crate::bridge::types::{ActionResult, ErrorCode, LauncherError};
use crate::core::i18n::{self, Language};
use crate::platform::log;

pub const LABEL: &str = "epic-sign-in";
/// Never resolved (RFC 2606); the navigation to it is cancelled before anything is looked up.
const HANDOFF_HOST: &str = "epic-sign-in.invalid";

/// Runs on every page of the window; acts only on Epic's redirect page.
const SCRIPT: &str = r#"(() => {
  if (location.origin !== "https://www.epicgames.com" || location.pathname !== "/id/api/redirect") return;
  addEventListener("DOMContentLoaded", () => {
    let code = null;
    try { code = JSON.parse(document.body.innerText).authorizationCode; } catch (_) {}
    if (typeof code === "string" && /^[A-Za-z0-9]{1,256}$/.test(code)) location.replace("https://epic-sign-in.invalid/#" + code);
  });
})();"#;

/// What the navigation hook makes of one URL.
enum Step {
    Allow,
    Deny,
    Code(AuthorizationCode),
}

/// Whether the page now shown is Epic's redirect page: only that page may hand a code over, so a third-party
/// login page can't sign the player in to an account of its choosing.
#[derive(Default)]
struct Watch {
    on_redirect: bool,
}

impl Watch {
    fn step(&mut self, url: &Url) -> Step {
        if url.scheme() == "https" && url.host_str() == Some(HANDOFF_HOST) {
            let from_redirect = std::mem::take(&mut self.on_redirect);
            return match (from_redirect, url.fragment().map(AuthorizationCode::new)) {
                (true, Some(Ok(code))) => Step::Code(code),
                _ => Step::Deny,
            };
        }
        if url.scheme() != "https" && url.as_str() != "about:blank" {
            return Step::Deny;
        }
        self.on_redirect = url.origin().ascii_serialization() == REDIRECT_ORIGIN && url.path() == REDIRECT_PATH;
        Step::Allow
    }
}

enum Outcome {
    Code(AuthorizationCode),
    Closed,
}

/// Opens the window and waits: signed in once Epic's page gave a code and Epic took it, `cancelled` when the
/// player closed the window, `busy` while one is already open (it is brought forward).
pub async fn sign_in(app: &AppHandle, epic: Arc<Epic>, language: Language) -> ActionResult {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_focus();
        return ActionResult::err(ErrorCode::Busy);
    }
    if !epic.set_window_open(true) {
        return ActionResult::err(ErrorCode::Busy);
    }
    let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
    let handoff = sender.clone();
    let watch = Mutex::new(Watch::default());
    let built = Url::parse(LOGIN_URL).map_err(|_| ()).and_then(|url| {
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
            .title(i18n::translate(language, i18n::EPIC_SIGN_IN_TITLE, &[]))
            .inner_size(520.0, 760.0)
            .min_inner_size(400.0, 560.0)
            .center()
            .focused(true)
            .incognito(true)
            .initialization_script(SCRIPT)
            .on_navigation(move |url| {
                let mut watch = watch.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match watch.step(url) {
                    Step::Allow => true,
                    Step::Deny => false,
                    Step::Code(code) => {
                        let _ = handoff.send(Outcome::Code(code));
                        false
                    }
                }
            })
            .on_new_window(|_, _| NewWindowResponse::Deny)
            .build()
            .map_err(|_| ())
    });
    let Ok(window) = built else {
        epic.set_window_open(false);
        log::warn("the Epic sign-in window could not open");
        return ActionResult::Err(LauncherError::with_detail(ErrorCode::EpicFailed, "window"));
    };
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let _ = sender.send(Outcome::Closed);
        }
    });
    let outcome = outcomes.recv().await;
    let _ = window.destroy();
    epic.set_window_open(false);
    match outcome {
        Some(Outcome::Code(code)) => epic.sign_in(code).await,
        _ => ActionResult::err(ErrorCode::Cancelled),
    }
}

/// Closes the window, as when a pasted code signed the player in; the waiting `sign_in` answers `cancelled`.
pub fn close(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REDIRECT: &str =
        "https://www.epicgames.com/id/api/redirect?clientId=34a02cf8f4414e29b15921876da36f9a&responseType=code";
    const HANDOFF: &str = "https://epic-sign-in.invalid/#0123456789abcdef0123456789abcdef";

    fn step(watch: &mut Watch, url: &str) -> &'static str {
        match watch.step(&Url::parse(url).unwrap()) {
            Step::Allow => "allow",
            Step::Deny => "deny",
            Step::Code(code) => {
                assert_eq!(code.expose(), "0123456789abcdef0123456789abcdef");
                "code"
            }
        }
    }

    #[test]
    fn the_code_is_taken_only_from_epics_redirect_page() {
        let mut watch = Watch::default();
        assert_eq!(step(&mut watch, LOGIN_URL), "allow");
        assert_eq!(step(&mut watch, "https://accounts.google.com/o/oauth2/auth"), "allow");
        assert_eq!(
            step(&mut watch, HANDOFF),
            "deny",
            "a third-party page can't hand a code over"
        );
        assert_eq!(step(&mut watch, REDIRECT), "allow");
        assert_eq!(step(&mut watch, HANDOFF), "code");
        assert_eq!(step(&mut watch, HANDOFF), "deny", "once");
        assert_eq!(step(&mut watch, REDIRECT), "allow");
        assert_eq!(step(&mut watch, "https://epic-sign-in.invalid/#not%20a%20code"), "deny");
        assert_eq!(step(&mut watch, "https://epic-sign-in.invalid/"), "deny");
    }

    #[test]
    fn only_https_loads() {
        let mut watch = Watch::default();
        for url in [
            "http://www.epicgames.com/id/api/redirect",
            "file:///C:/Windows/win.ini",
            "com.epicgames.launcher://apps",
            "javascript:alert(1)",
            "data:text/html,x",
        ] {
            assert_eq!(step(&mut watch, url), "deny", "{url}");
        }
        assert_eq!(step(&mut watch, "about:blank"), "allow");
        assert_eq!(step(&mut watch, "http://www.epicgames.com/id/api/redirect"), "deny");
        assert_eq!(
            step(&mut watch, HANDOFF),
            "deny",
            "plain HTTP never counts as the redirect page"
        );
    }

    #[test]
    fn the_script_acts_on_the_redirect_page_only() {
        assert!(SCRIPT.contains(&format!("location.origin !== \"{REDIRECT_ORIGIN}\"")));
        assert!(SCRIPT.contains(&format!("location.pathname !== \"{REDIRECT_PATH}\"")));
        assert!(SCRIPT.contains(&format!("\"https://{HANDOFF_HOST}/#\"")));
    }
}
