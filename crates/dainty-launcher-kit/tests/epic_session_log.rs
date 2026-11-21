// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! No Epic code or token reaches any log (DE2). Its own test binary, because it writes the process-wide log.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::{Arc, Mutex};

use dainty_epic::auth::{Accounts, AuthError, Grant, Pending, Session, Token};
use tracing_subscriber::layer::SubscriberExt as _;

use dainty_launcher_kit::bridge::types::{ActionResult, ErrorCode};
use dainty_launcher_kit::platform::epic_session::Epic;
use dainty_launcher_kit::platform::keystore::epic::EpicSessionStore;
use dainty_launcher_kit::platform::keystore::os::{Encryptor, SealError, open_rr1, seal_rr1};
use dainty_launcher_kit::platform::log;

const CODE: &str = "c0dec0dec0dec0dec0dec0dec0dec0de";

struct Fake;

impl Encryptor for Fake {
    fn available(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        seal_rr1(&[1; 32], plain)
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        open_rr1(&[1; 32], sealed)
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Epic's account service as a double. Like egs-api, it also logs what it shouldn't, under `egs_api`, through
/// both `tracing` and `log`: the launcher's log setup must drop those lines.
#[derive(Default)]
struct Double {
    seen: Mutex<Vec<String>>,
    issued: Mutex<u32>,
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
        let refused = kind == "code" && secret == "used";
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
                now() + 60,
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

/// Everything `tracing` (and `log`, forwarded) carries while signing in, refreshing and signing out, with the
/// launcher's log layers in the subscriber, plus the launcher's log file: no code and no token anywhere.
#[test]
fn no_code_or_token_reaches_any_log() {
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

    let logs = tempfile::tempdir().unwrap();
    log::set_folder(logs.path());
    let _ = tracing_log::LogTracer::init();
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry()
        .with(log::LogFilter)
        .with(log::LogLayer)
        .with(capture.clone());
    let dir = tempfile::tempdir().unwrap();
    let double = Arc::new(Double::default());
    tracing::subscriber::with_default(subscriber, || {
        let epic = Epic::new(EpicSessionStore::new(dir.path(), Arc::new(Fake)), double.clone());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
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
    assert_eq!(*double.seen.lock().unwrap(), ["code", "code", "refresh", "end"]);
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
