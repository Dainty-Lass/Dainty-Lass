//! Epic sign-in behind the bridge's `epic_*` commands (DD22, DE2). A code from the login window (`epic_login`)
//! or the player's paste is exchanged through `epic::auth`; the session goes to the keystore (`keystore::epic`)
//! and stays here. Only [`EpicView`] leaves: whether someone is signed in, and their display name.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ::epic::auth::{self, Accounts, AuthError, AuthorizationCode, Session};

use crate::bridge::types::{ActionResult, EpicView, ErrorCode, LauncherError};
use crate::platform::keystore::epic::EpicSessionStore;
use crate::platform::log;

/// egs-api's client has no timeout of its own.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);
const SIGN_OUT_TIMEOUT: Duration = Duration::from_secs(10);
/// An access token this close to its expiry is refreshed before use (egs-api's own margin).
const REFRESH_MARGIN_SECONDS: i64 = 600;

pub struct Epic {
    store: EpicSessionStore,
    accounts: Arc<dyn Accounts>,
    /// Held across each exchange, refresh and sign out, so they never interleave
    session: tokio::sync::Mutex<Option<Session>>,
    view: Mutex<View>,
}

#[derive(Default)]
struct View {
    signed_in: bool,
    display_name: Option<String>,
    window_open: bool,
    exchanging: u32,
}

/// Marks an exchange in progress for [`EpicView::signing_in`] until dropped.
struct Exchanging<'a>(&'a Mutex<View>);

impl Drop for Exchanging<'_> {
    fn drop(&mut self) {
        let mut view = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        view.exchanging = view.exchanging.saturating_sub(1);
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn auth_error(error: &AuthError) -> LauncherError {
    LauncherError::new(match error {
        AuthError::BadCode => ErrorCode::EpicCodeInvalid,
        AuthError::Rejected => ErrorCode::EpicRejected,
        AuthError::Unreachable => ErrorCode::EpicUnreachable,
        _ => ErrorCode::EpicFailed,
    })
}

fn store_error() -> LauncherError {
    LauncherError::new(ErrorCode::KeyStorageUnavailable)
}

impl Epic {
    /// Restores the session the keystore holds, so a restart stays signed in. Nothing is asked of Epic here.
    pub fn new(store: EpicSessionStore, accounts: Arc<dyn Accounts>) -> Self {
        let session = store.load(now()).unwrap_or_else(|_| {
            log::warn("the Epic session could not be read: key storage unavailable");
            None
        });
        let epic = Self {
            store,
            accounts,
            session: tokio::sync::Mutex::new(None),
            view: Mutex::new(View::default()),
        };
        epic.show(session.as_ref());
        Self {
            session: tokio::sync::Mutex::new(session),
            ..epic
        }
    }

    fn view_lock(&self) -> MutexGuard<'_, View> {
        self.view.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn show(&self, session: Option<&Session>) {
        let mut view = self.view_lock();
        view.signed_in = session.is_some();
        view.display_name = session
            .map(Session::display_name)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
    }

    fn exchanging(&self) -> Exchanging<'_> {
        self.view_lock().exchanging += 1;
        Exchanging(&self.view)
    }

    pub fn view(&self) -> EpicView {
        let view = self.view_lock();
        EpicView {
            signed_in: view.signed_in,
            display_name: view.display_name.clone(),
            signing_in: view.window_open || view.exchanging > 0,
        }
    }

    /// Whether the login window is open; false refuses a second one.
    pub fn set_window_open(&self, open: bool) -> bool {
        let mut view = self.view_lock();
        let changed = view.window_open != open;
        view.window_open = open;
        changed
    }

    pub fn signed_in(&self) -> bool {
        self.view_lock().signed_in
    }

    /// The paste fallback: the code, or the whole JSON page Epic shows it in.
    pub async fn sign_in_pasted(&self, pasted: &str) -> ActionResult {
        match AuthorizationCode::from_pasted(pasted) {
            Ok(code) => self.sign_in(code).await,
            Err(error) => ActionResult::Err(auth_error(&error)),
        }
    }

    /// Exchanges the code and keeps the session, in place of any before it.
    pub async fn sign_in(&self, code: AuthorizationCode) -> ActionResult {
        let _exchanging = self.exchanging();
        let mut slot = self.session.lock().await;
        let session = match tokio::time::timeout(EXCHANGE_TIMEOUT, auth::sign_in(self.accounts.as_ref(), code)).await {
            Ok(Ok(session)) => session,
            Ok(Err(error)) => {
                log::warn(&format!("Epic sign-in failed: {error}"));
                return ActionResult::Err(auth_error(&error));
            }
            Err(_) => return ActionResult::Err(auth_error(&AuthError::Unreachable)),
        };
        if self.store.save(&session).is_err() {
            log::warn("the Epic session could not be stored: key storage unavailable");
            return ActionResult::Err(store_error());
        }
        log::info("signed in to Epic");
        self.show(Some(&session));
        *slot = Some(session);
        ActionResult::Ok
    }

    /// Forgets the session here first, then ends it at Epic as far as Epic answers in time.
    pub async fn sign_out(&self) -> ActionResult {
        let mut slot = self.session.lock().await;
        let session = slot.take();
        self.show(None);
        let cleared = self.store.clear();
        if let Some(session) = session {
            let ended = tokio::time::timeout(SIGN_OUT_TIMEOUT, auth::sign_out(self.accounts.as_ref(), session)).await;
            if !matches!(ended, Ok(Ok(()))) {
                log::warn("the Epic session could not be ended at Epic; it is forgotten here");
            }
        }
        log::info("signed out of Epic");
        match cleared {
            Ok(()) => ActionResult::Ok,
            Err(_) => ActionResult::Err(store_error()),
        }
    }

    /// The session with an access token good for a while: refreshed first when it is close to expiry, and the
    /// turned-over tokens stored. Epic refusing the refresh token signs out.
    #[cfg_attr(not(test), allow(dead_code, reason = "DE3's library check takes the session"))]
    pub async fn fresh_session(&self) -> Result<tokio::sync::MutexGuard<'_, Option<Session>>, LauncherError> {
        let mut slot = self.session.lock().await;
        let Some(session) = slot.as_ref() else {
            return Err(LauncherError::new(ErrorCode::EpicSignedOut));
        };
        if session.access_expires_at() - now() > REFRESH_MARGIN_SECONDS {
            return Ok(slot);
        }
        let refreshed =
            match tokio::time::timeout(EXCHANGE_TIMEOUT, auth::refresh(self.accounts.as_ref(), session)).await {
                Ok(Ok(refreshed)) => refreshed,
                Ok(Err(AuthError::Rejected)) => {
                    log::warn("Epic refused the stored session; signed out");
                    *slot = None;
                    self.show(None);
                    let _ = self.store.clear();
                    return Err(LauncherError::new(ErrorCode::EpicSignedOut));
                }
                Ok(Err(error)) => return Err(auth_error(&error)),
                Err(_) => return Err(auth_error(&AuthError::Unreachable)),
            };
        if self.store.save(&refreshed).is_err() {
            log::warn("the refreshed Epic session could not be stored: key storage unavailable");
        }
        self.show(Some(&refreshed));
        *slot = Some(refreshed);
        Ok(slot)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use ::epic::auth::{Grant, Pending, Token};

    use super::*;
    use crate::platform::keystore::epic::FILE_NAME;
    use crate::platform::keystore::epic::tests::Fake;

    const CODE: &str = "c0dec0dec0dec0dec0dec0dec0dec0de";

    /// Epic's account service as a double. Like egs-api, it also logs what it shouldn't, under `egs_api`, through
    /// both `tracing` and `log`: the launcher's log setup must drop those lines.
    struct Double {
        seen: StdMutex<Vec<String>>,
        issued: StdMutex<u32>,
        access_expires_in: i64,
        refuse_refresh: bool,
    }

    impl Double {
        fn new(access_expires_in: i64) -> Arc<Self> {
            Arc::new(Self {
                seen: StdMutex::new(Vec::new()),
                issued: StdMutex::new(0),
                access_expires_in,
                refuse_refresh: false,
            })
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Accounts for Double {
        fn token(&self, grant: Grant) -> Pending<'_, Session> {
            let (kind, secret) = match &grant {
                Grant::Code(code) => ("code", code.expose().to_owned()),
                Grant::Refresh(token) => ("refresh", token.expose().to_owned()),
            };
            self.seen.lock().unwrap().push(kind.to_owned());
            tracing::error!(target: "egs_api::api::login", "POST https://account/oauth/token code={secret}");
            tracing_log::log::warn!(target: "egs_api::api", "error sending request {secret}");
            let refused = (kind == "code" && secret == "used") || (kind == "refresh" && self.refuse_refresh);
            let mut issued = self.issued.lock().unwrap();
            *issued += 1;
            let n = *issued;
            Box::pin(async move {
                if refused {
                    return Err(AuthError::Rejected);
                }
                Ok(Session::new(
                    "8645b4947bbc4c0092a8b7236df169d1".into(),
                    "Slayer".into(),
                    Token::new(format!("eg1~access-token-{n}")),
                    now() + self.access_expires_in,
                    Token::new(format!("eg1~refresh-token-{n}")),
                    now() + 30 * 86_400,
                ))
            })
        }

        fn end_session(&self, access_token: Token) -> Pending<'_, ()> {
            self.seen.lock().unwrap().push("end".into());
            tracing_log::log::warn!(
                target: "egs_api::api::login",
                "Unable to invalidate session: error sending request for url (https://account/oauth/sessions/kill/{})",
                access_token.expose()
            );
            Box::pin(async { Ok(()) })
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    fn epic(dir: &std::path::Path, accounts: Arc<Double>) -> Epic {
        Epic::new(EpicSessionStore::new(dir, Arc::new(Fake)), accounts)
    }

    #[test]
    fn signed_in_after_a_restart_until_signed_out() {
        let dir = tempfile::tempdir().unwrap();
        let double = Double::new(7200);
        let first = epic(dir.path(), double.clone());
        assert_eq!(
            first.view(),
            EpicView {
                signed_in: false,
                display_name: None,
                signing_in: false
            }
        );
        let page = format!(
            r#"{{"redirectUrl":"https://localhost/launcher/authorized?code={CODE}","authorizationCode":"{CODE}","exchangeCode":null,"sid":null}}"#
        );
        assert_eq!(runtime().block_on(first.sign_in_pasted(&page)), ActionResult::Ok);
        assert_eq!(first.view().display_name.as_deref(), Some("Slayer"));
        assert!(dir.path().join(FILE_NAME).exists());
        drop(first);

        // The launcher starts again: the keystore entry is the session
        let again = epic(dir.path(), double.clone());
        assert_eq!(
            again.view(),
            EpicView {
                signed_in: true,
                display_name: Some("Slayer".into()),
                signing_in: false
            }
        );
        assert_eq!(double.seen(), ["code"], "a restart asks nothing of Epic");
        assert_eq!(runtime().block_on(again.sign_out()), ActionResult::Ok);
        assert!(
            !dir.path().join(FILE_NAME).exists(),
            "sign out clears the keystore entry"
        );
        assert!(!again.view().signed_in);
        assert_eq!(double.seen(), ["code", "end"]);
        assert!(!epic(dir.path(), double).view().signed_in);
    }

    #[test]
    fn bad_and_refused_codes_store_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let double = Double::new(7200);
        let epic = epic(dir.path(), double.clone());
        let rt = runtime();
        for bad in ["", "not a code", r#"{"authorizationCode":null}"#] {
            assert_eq!(
                rt.block_on(epic.sign_in_pasted(bad)),
                ActionResult::err(ErrorCode::EpicCodeInvalid),
                "{bad}"
            );
        }
        assert!(double.seen().is_empty(), "nothing sent to Epic");
        assert_eq!(
            rt.block_on(epic.sign_in_pasted("used")),
            ActionResult::err(ErrorCode::EpicRejected)
        );
        assert!(!dir.path().join(FILE_NAME).exists());
        assert!(!epic.view().signed_in);
        assert_eq!(
            rt.block_on(epic.sign_out()),
            ActionResult::Ok,
            "signing out twice is fine"
        );
        assert_eq!(
            rt.block_on(epic.fresh_session()).err(),
            Some(LauncherError::new(ErrorCode::EpicSignedOut))
        );
    }

    #[test]
    fn a_stale_access_token_is_refreshed_and_the_new_tokens_kept() {
        let dir = tempfile::tempdir().unwrap();
        let double = Double::new(60);
        let epic = epic(dir.path(), double.clone());
        let rt = runtime();
        assert_eq!(rt.block_on(epic.sign_in_pasted(CODE)), ActionResult::Ok);
        {
            let fresh = rt.block_on(epic.fresh_session()).unwrap();
            assert_eq!(fresh.as_ref().unwrap().refresh_token().expose(), "eg1~refresh-token-2");
        }
        let stored = EpicSessionStore::new(dir.path(), Arc::new(Fake))
            .load(now())
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.refresh_token().expose(),
            "eg1~refresh-token-2",
            "Epic turns the token over"
        );
        assert_eq!(double.seen(), ["code", "refresh"]);

        let refusing = Arc::new(Double {
            refuse_refresh: true,
            ..Arc::into_inner(Double::new(60)).unwrap()
        });
        let epic = super::Epic::new(EpicSessionStore::new(dir.path(), Arc::new(Fake)), refusing);
        assert!(epic.view().signed_in);
        assert_eq!(
            rt.block_on(epic.fresh_session()).err(),
            Some(LauncherError::new(ErrorCode::EpicSignedOut))
        );
        assert!(!epic.view().signed_in);
        assert!(!dir.path().join(FILE_NAME).exists(), "a refused session is forgotten");
    }

    /// Everything `tracing` (and `log`, forwarded) carries while signing in, refreshing and signing out, with the
    /// launcher's log layer in the subscriber, plus the launcher's log file: no code and no token anywhere.
    #[test]
    fn no_code_or_token_reaches_any_log() {
        use tracing_subscriber::layer::SubscriberExt as _;

        #[derive(Clone, Default)]
        struct Capture(Arc<StdMutex<String>>);

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
            fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
                struct All<'a>(&'a mut String);
                impl tracing::field::Visit for All<'_> {
                    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                        self.0.push_str(&format!(" {}={value:?}", field.name()));
                    }
                }
                let mut line = self.0.lock().unwrap();
                line.push_str(event.metadata().target());
                event.record(&mut All(&mut line));
                line.push('\n');
            }
        }

        let _file = log::FILE_TESTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let logs = tempfile::tempdir().unwrap();
        log::log_to_file(logs.path().to_path_buf());
        let _ = tracing_log::LogTracer::init();
        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry().with(log::Layer).with(capture.clone());
        let dir = tempfile::tempdir().unwrap();
        let double = Double::new(60);
        tracing::subscriber::with_default(subscriber, || {
            let epic = epic(dir.path(), double.clone());
            let rt = runtime();
            assert_eq!(rt.block_on(epic.sign_in_pasted(CODE)), ActionResult::Ok);
            assert_eq!(
                rt.block_on(epic.sign_in_pasted("used")),
                ActionResult::err(ErrorCode::EpicRejected)
            );
            drop(rt.block_on(epic.fresh_session()).unwrap());
            assert_eq!(rt.block_on(epic.sign_out()), ActionResult::Ok);
            tracing::warn!(target: "launcher::check", "the capture works");
        });
        let captured = capture.0.lock().unwrap().clone();
        let file = std::fs::read_to_string(logs.path().join("launcher.log")).unwrap_or_default();
        log::stop_file_for_tests();
        assert_eq!(double.seen(), ["code", "code", "refresh", "end"]);
        assert!(captured.contains("the capture works"), "{captured}");
        assert!(
            file.contains("signed in to Epic") && file.contains("signed out of Epic"),
            "{file}"
        );
        for secret in [CODE, "access-token", "refresh-token", "egs_api"] {
            assert!(!captured.contains(secret), "{secret} in tracing: {captured}");
            assert!(!file.contains(secret), "{secret} in the log file: {file}");
        }
    }
}
