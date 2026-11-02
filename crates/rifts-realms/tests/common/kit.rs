// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The directory test kit: a directory over a test database with a clock the test moves, counting ids, recorded
//! events and quiet logs.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rifts_realms::RealmsError;
use rifts_realms::directory::{DirectoryParts, RealmDirectory};
use rifts_realms::input::HeartbeatInput;
use rifts_realms::ports::{
    Accounts, FixedAccounts, IdSource, ManualClock, QuietLog, RateLimiter, RealmEventSink, RecordedEvents,
};
use rifts_realms::realm::{
    DEFAULT_REALM_LIMITS, HostingPatch, HubState, INVITE_ALPHABET, Patch, RealmEvent, RealmLimits,
};
use rifts_seam::{BoxFuture, UnixMillis};

use super::Target;

/// 2026-10-13T12:00:00.000Z.
pub const T0: i64 = 1_791_892_800_000;

pub struct NoLimit;

impl RateLimiter for NoLimit {
    fn is_limited(&self, _key: &str, _max: u32, _window_ms: i64) -> bool {
        false
    }
}

/// Records every key and limits the ones the test names.
#[derive(Default)]
pub struct KeyLimiter {
    pub keys: Mutex<Vec<String>>,
    pub limited: Mutex<Vec<String>>,
}

impl RateLimiter for KeyLimiter {
    fn is_limited(&self, key: &str, _max: u32, _window_ms: i64) -> bool {
        self.keys.lock().unwrap().push(key.to_owned());
        self.limited.lock().unwrap().iter().any(|k| k == key)
    }
}

/// `realm-1`, `realm-2`...; invite codes `ABCDEFGHJK`, `ABCDEFGHJL`... (valid and distinct).
#[derive(Default)]
pub struct CountingIds {
    realms: AtomicU64,
    codes: AtomicU64,
}

pub fn code_number(n: u64) -> String {
    let symbol = |i: u64| char::from(INVITE_ALPHABET[usize::try_from(i % 32).unwrap()]);
    let start = 8 * 32 + 9; // "JK"
    let value = start + n;
    format!("ABCDEFGH{}{}", symbol(value / 32), symbol(value))
}

impl IdSource for CountingIds {
    fn realm_id(&self) -> String {
        format!("realm-{}", self.realms.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn invite_code(&self) -> String {
        code_number(self.codes.fetch_add(1, Ordering::SeqCst))
    }
}

/// The listed codes in turn, then `ZZZZZZZZZZ` for ever.
pub struct ListedCodes {
    pub codes: Mutex<VecDeque<String>>,
    pub realms: CountingIds,
}

impl IdSource for ListedCodes {
    fn realm_id(&self) -> String {
        self.realms.realm_id()
    }

    fn invite_code(&self) -> String {
        self.codes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "ZZZZZZZZZZ".to_owned())
    }
}

pub struct FailingNames;

impl Accounts for FailingNames {
    fn display_names(&self, _ids: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>> {
        Box::pin(std::future::ready(Err("names are down".to_owned())))
    }

    fn resolve_username(&self, _username: String) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(std::future::ready(Ok(None)))
    }
}

#[derive(Default)]
pub struct FailingSink {
    pub calls: AtomicBool,
}

impl RealmEventSink for FailingSink {
    fn emit(&self, _event: RealmEvent) -> BoxFuture<'_, Result<(), String>> {
        self.calls.store(true, Ordering::SeqCst);
        Box::pin(std::future::ready(Err("sink is down".to_owned())))
    }
}

pub fn accounts() -> FixedAccounts {
    let pairs = |list: &[(&str, &str)]| -> BTreeMap<String, String> {
        list.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    };
    FixedAccounts {
        names: pairs(&[
            ("host-1", "Hosty"),
            ("host-2", "Hosty Two"),
            ("friend-1", "Friendly"),
            ("friend-2", "Pal"),
        ]),
        usernames: pairs(&[("pal", "friend-2"), ("friendly", "friend-1"), ("hosty", "host-1")]),
    }
}

pub struct Options {
    pub accounts: Arc<dyn Accounts>,
    pub events: Option<Arc<dyn RealmEventSink>>,
    pub limiter: Arc<dyn RateLimiter>,
    pub ids: Arc<dyn IdSource>,
    pub limits: RealmLimits,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            accounts: Arc::new(accounts()),
            events: None,
            limiter: Arc::new(NoLimit),
            ids: Arc::new(CountingIds::default()),
            limits: DEFAULT_REALM_LIMITS,
        }
    }
}

pub struct Kit {
    pub target: Target,
    pub clock: Arc<ManualClock>,
    pub events: Arc<RecordedEvents>,
    pub directory: RealmDirectory,
}

pub fn kit(target: Target) -> Kit {
    kit_with(target, Options::default())
}

pub fn kit_with(target: Target, options: Options) -> Kit {
    let clock = Arc::new(ManualClock::at(UnixMillis(T0)));
    let events = Arc::new(RecordedEvents::default());
    let sink: Arc<dyn RealmEventSink> = options.events.unwrap_or_else(|| events.clone());
    let directory = RealmDirectory::new(DirectoryParts {
        db: target.db.clone(),
        accounts: options.accounts,
        events: Some(sink),
        clock: clock.clone(),
        limiter: options.limiter,
        ids: options.ids,
        limits: options.limits,
        log: Arc::new(QuietLog),
    });
    Kit {
        target,
        clock,
        events,
        directory,
    }
}

impl Kit {
    pub fn advance(&self, millis: i64) {
        self.clock.advance(millis);
    }

    /// The host heartbeats and its Ramsgate hub runs, as session `hub-<realm>`.
    pub async fn serve(&self, host: &str, realm_id: &str) {
        let beat = HeartbeatInput {
            online: Some(true),
            version: None,
            hosting: None,
        };
        self.directory.heartbeat(host, &beat).await.unwrap();
        let patch = HostingPatch {
            hub_state: Some(HubState::Running),
            hub_session_id: Patch::Set(format!("hub-{realm_id}")),
            ..HostingPatch::default()
        };
        assert!(self.directory.set_hosting(realm_id, &patch).await.unwrap());
    }
}

/// The refusal code, or `ok`.
pub fn code<T>(result: &Result<T, RealmsError>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(RealmsError::Refused(refusal)) => refusal.code.clone(),
        Err(other) => format!("{other:?}"),
    }
}
