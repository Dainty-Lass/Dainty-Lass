//! launcher-kit's Epic sign-in, session file and library check (DD22, DE2, DE3), as the shell's
//! `platform/{epic_session,epic_library,keystore/epic}.rs` tested them.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use epic::auth::{Accounts, AuthError, Grant, Pending, Session, Token};
use epic::library::{APP_NAME, Library, LibraryError, OWNED_FOR, Ownership, Record};
use launcher_kit::bridge::types::{ActionResult, EpicLibraryCheck, EpicView, ErrorCode, LauncherError};
use launcher_kit::platform::epic_library::{check, require};
use launcher_kit::platform::epic_session::Epic;
use launcher_kit::platform::keystore::KeyStoreError;
use launcher_kit::platform::keystore::epic::{EpicSessionStore, FILE_NAME};
use launcher_kit::platform::keystore::os::{Encryptor, SealError, Unavailable, open_rr1, seal_rr1};
use launcher_kit::platform::log;
use serde_json::json;

/// The keys' test sealer (rr1 under a fixed key): the file is never plain text.
struct Fake;

impl Fake {
    fn encrypt(plain: &str) -> Vec<u8> {
        seal_rr1(&[3; 32], plain.as_bytes()).unwrap()
    }
}

impl Encryptor for Fake {
    fn available(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        seal_rr1(&[3; 32], plain)
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        open_rr1(&[3; 32], sealed)
    }
}

fn now() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    i64::try_from(secs).unwrap()
}

fn session(access: &str, refresh: &str) -> Session {
    Session::new(
        "8645b4947bbc4c0092a8b7236df169d1".into(),
        "Slayer".into(),
        Token::new(access.into()),
        1_800_000_000,
        Token::new(refresh.into()),
        1_900_000_000,
    )
}

// ---------------------------------------------------------------- keystore/epic.rs

#[test]
fn a_session_round_trips_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let store = EpicSessionStore::new(&dir.path().join("keys"), Arc::new(Fake));
    assert!(store.load(0).unwrap().is_none());
    store.save(&session("eg1~access-a1b2c3", "eg1~refresh-d4e5f6")).unwrap();
    assert_eq!(store.file(), dir.path().join("keys").join("epic.session"));
    let raw = std::fs::read(store.file()).unwrap();
    assert!(
        raw.starts_with(b"rr1") && !String::from_utf8_lossy(&raw).contains("refresh-d4e5f6"),
        "sealed"
    );
    let loaded = store.load(1_850_000_000).unwrap().unwrap();
    assert_eq!(loaded.account_id(), "8645b4947bbc4c0092a8b7236df169d1");
    assert_eq!(loaded.display_name(), "Slayer");
    assert_eq!(loaded.access_token().expose(), "eg1~access-a1b2c3");
    assert_eq!(loaded.access_expires_at(), 1_800_000_000);
    assert_eq!(loaded.refresh_token().expose(), "eg1~refresh-d4e5f6");
    assert_eq!(loaded.refresh_expires_at(), 1_900_000_000);
    assert!(!log::redact("eg1~refresh-d4e5f6").contains("d4e5f6"));
    assert!(
        store.load(1_900_000_000).unwrap().is_none(),
        "a refresh token past its expiry"
    );
    store.clear().unwrap();
    assert!(!store.file().exists());
    store.clear().unwrap();
    assert!(store.load(0).unwrap().is_none());
}

#[test]
fn a_damaged_or_foreign_file_is_no_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = EpicSessionStore::new(dir.path(), Arc::new(Fake));
    for plain in [
        "",
        "{}",
        r#"{"v":2,"accountId":"a","displayName":"","accessToken":"x","accessExpiresAt":1,"refreshToken":"y","refreshExpiresAt":2}"#,
        r#"{"v":1,"accountId":"a","displayName":"","accessToken":"","accessExpiresAt":1,"refreshToken":"y","refreshExpiresAt":2}"#,
        r#"{"v":1,"accountId":"a","displayName":"","accessToken":"x","accessExpiresAt":"1","refreshToken":"y","refreshExpiresAt":2}"#,
    ] {
        std::fs::write(store.file(), Fake::encrypt(plain)).unwrap();
        assert!(store.load(0).unwrap().is_none(), "{plain}");
    }
    std::fs::write(store.file(), b"another user's DPAPI blob").unwrap();
    assert!(store.load(0).unwrap().is_none());
}

#[test]
fn an_unavailable_store_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = EpicSessionStore::new(dir.path(), Arc::new(Unavailable));
    assert!(matches!(
        store.save(&session("a1", "r1")),
        Err(KeyStoreError::Unavailable)
    ));
    assert!(!store.file().exists());
    std::fs::write(store.file(), b"x").unwrap();
    assert!(matches!(store.load(0), Err(KeyStoreError::Unavailable)));
}

// ---------------------------------------------------------------- epic_session.rs

const CODE: &str = "c0dec0dec0dec0dec0dec0dec0dec0de";

/// Epic's account service as a double. Like egs-api, it also logs what it shouldn't, under `egs_api`, through both
/// `tracing` and `log`: the launcher's log setup must drop those lines.
struct Double {
    seen: Mutex<Vec<String>>,
    issued: Mutex<u32>,
    access_expires_in: i64,
    refuse_refresh: bool,
}

impl Double {
    fn with(access_expires_in: i64, refuse_refresh: bool) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            issued: Mutex::new(0),
            access_expires_in,
            refuse_refresh,
        })
    }

    fn new(access_expires_in: i64) -> Arc<Self> {
        Self::with(access_expires_in, false)
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

/// The secrets and the log folder are the process's.
static GLOBAL: Mutex<()> = Mutex::new(());

#[test]
fn signed_in_after_a_restart_until_signed_out() {
    let _global = GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let _global = GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let _global = GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let double = Double::new(60);
    let epic_now = epic(dir.path(), double.clone());
    let rt = runtime();
    assert_eq!(rt.block_on(epic_now.sign_in_pasted(CODE)), ActionResult::Ok);
    {
        let fresh = rt.block_on(epic_now.fresh_session()).unwrap();
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

    let refusing = epic(dir.path(), Double::with(60, true));
    assert!(refusing.view().signed_in);
    assert_eq!(
        rt.block_on(refusing.fresh_session()).err(),
        Some(LauncherError::new(ErrorCode::EpicSignedOut))
    );
    assert!(!refusing.view().signed_in);
    assert!(!dir.path().join(FILE_NAME).exists(), "a refused session is forgotten");
}

/// Everything `tracing` (and `log`, forwarded) carries while signing in, refreshing and signing out, with the
/// launcher's log layers in the subscriber, plus the launcher's log file: no code and no token anywhere.
#[test]
fn no_code_or_token_reaches_any_log() {
    use tracing_subscriber::layer::SubscriberExt as _;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<String>>);

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

    let _global = GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let logs = tempfile::tempdir().unwrap();
    log::set_folder(logs.path());
    let _ = tracing_log::LogTracer::init();
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry()
        .with(log::LogFilter)
        .with(log::LogLayer)
        .with(capture.clone());
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
    let file = std::fs::read_to_string(logs.path().join(log::LOG_FILE)).unwrap_or_default();
    log::set_folder(&std::env::temp_dir().join(format!("launcher-tests-{}", std::process::id())));
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

// ---------------------------------------------------------------- epic_library.rs

const NOW: i64 = 1_800_000_000;

struct Shelf(Mutex<Result<Vec<Record>, LibraryError>>);

impl Library for Shelf {
    async fn records(&self, _: &Session) -> Result<Vec<Record>, LibraryError> {
        self.0.lock().unwrap().clone()
    }
}

fn library(app_names: &[&str]) -> Result<Vec<Record>, LibraryError> {
    Ok(app_names
        .iter()
        .map(|name| Record {
            app_name: (*name).to_owned(),
            ..Record::default()
        })
        .collect())
}

fn signed_in() -> Session {
    Session::new(
        "8645b4947bbc4c0092a8b7236df169d1".to_owned(),
        "Slayer".to_owned(),
        Token::new("eg1~access-secret".to_owned()),
        NOW + 30 * 86_400,
        Token::new("eg1~refresh-secret".to_owned()),
        NOW + 60 * 86_400,
    )
}

fn ownership(answer: Result<Vec<Record>, LibraryError>) -> (Ownership<Shelf>, Arc<AtomicI64>) {
    let clock = Arc::new(AtomicI64::new(NOW));
    let read = Arc::clone(&clock);
    let ownership = Ownership::new(Shelf(Mutex::new(answer)), None).with_clock(move || read.load(Ordering::SeqCst));
    (ownership, clock)
}

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn page(answer: &EpicLibraryCheck) -> serde_json::Value {
    serde_json::to_value(answer).unwrap()
}

#[test]
fn the_check_answers_as_the_page_reads_it() {
    let (owner, _) = ownership(library(&["Fortnite", APP_NAME]));
    assert_eq!(
        page(&run(check(&owner, Some(&signed_in())))),
        json!({ "ok": true, "owned": true, "checkedAt": "2027-01-15T08:00:00.000Z" })
    );
    let (not_owner, _) = ownership(library(&["Fortnite"]));
    assert_eq!(
        page(&run(check(&not_owner, Some(&signed_in())))),
        json!({ "ok": true, "owned": false, "checkedAt": "2027-01-15T08:00:00.000Z" })
    );
    assert_eq!(
        page(&run(check(&not_owner, None))),
        json!({ "ok": false, "error": { "code": "epic_signed_out" } })
    );
    let (offline, _) = ownership(Err(LibraryError::Unreachable));
    assert_eq!(
        page(&run(check(&offline, Some(&signed_in())))),
        json!({ "ok": false, "error": { "code": "epic_library_failed" } })
    );
}

#[test]
fn an_account_without_dauntless_cant_start_a_download() {
    let (owner, _) = ownership(library(&[APP_NAME]));
    assert_eq!(run(require(&owner, Some(&signed_in()))), Ok(()));
    let (not_owner, _) = ownership(library(&["Fortnite"]));
    assert_eq!(
        run(require(&not_owner, Some(&signed_in()))),
        Err(LauncherError::new(ErrorCode::EpicNotOwned))
    );
    assert_eq!(
        run(require(&owner, None)),
        Err(LauncherError::new(ErrorCode::EpicSignedOut))
    );
    let (offline, _) = ownership(Err(LibraryError::Unreachable));
    assert_eq!(
        run(require(&offline, Some(&signed_in()))),
        Err(LauncherError::new(ErrorCode::EpicLibraryFailed))
    );
}

#[test]
fn an_expired_session_signs_in_again() {
    let (owner, clock) = ownership(library(&[APP_NAME]));
    clock.store(NOW + OWNED_FOR + 30 * 86_400, Ordering::SeqCst);
    assert_eq!(
        run(require(&owner, Some(&signed_in()))),
        Err(LauncherError::new(ErrorCode::EpicSignedOut))
    );
}
