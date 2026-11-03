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
    fn is_limited(&self, _: &str, _: u32, _: i64) -> bool {
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
    fn is_limited(&self, key: &str, _: u32, _: i64) -> bool {
        let mut seen = self.keys.lock().unwrap();
        seen.push(key.into());
        self.limited.lock().unwrap().contains(&key.to_owned())
    }
}

/// `realm-1`, `realm-2`...; invite codes `ABCDEFGHJK`, `ABCDEFGHJL`... (valid and distinct).
#[derive(Default)]
pub struct CountingIds {
    realms: AtomicU64,
    codes: AtomicU64,
}

pub fn code_number(n: u64) -> String {
    // Two base-32 digits counted from "JK" (8, 9).
    let value = n + 8 * 32 + 9;
    let digit = |place: u64| INVITE_ALPHABET[usize::try_from(place % 32).unwrap()] as char;
    let tail: String = [digit(value / 32), digit(value)].iter().collect();
    format!("ABCDEFGH{tail}")
}

impl IdSource for CountingIds {
    fn realm_id(&self) -> String {
        let issued = 1 + self.realms.fetch_add(1, Ordering::SeqCst);
        format!("realm-{issued}")
    }

    fn invite_code(&self) -> String {
        let issued = self.codes.fetch_add(1, Ordering::SeqCst);
        code_number(issued)
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
        let next = self.codes.lock().unwrap().pop_front();
        next.unwrap_or_else(|| "Z".repeat(10))
    }
}

pub struct FailingNames;

fn ready<T: Send + 'static>(value: T) -> BoxFuture<'static, T> {
    Box::pin(std::future::ready(value))
}

impl Accounts for FailingNames {
    fn display_names(&self, _: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>> {
        ready(Err(String::from("names are down")))
    }

    fn resolve_username(&self, _: String) -> BoxFuture<'_, Result<Option<String>, String>> {
        ready(Ok(None))
    }
}

#[derive(Default)]
pub struct FailingSink {
    pub calls: AtomicBool,
}

impl RealmEventSink for FailingSink {
    fn emit(&self, _: RealmEvent) -> BoxFuture<'_, Result<(), String>> {
        self.calls.store(true, Ordering::SeqCst);
        ready(Err(String::from("sink is down")))
    }
}

/// `a=b;c=d` as a map.
fn pairs(list: &str) -> BTreeMap<String, String> {
    let split = list.split(';').filter_map(|pair| pair.split_once('='));
    split.map(|(key, value)| (key.into(), value.into())).collect()
}

pub fn accounts() -> FixedAccounts {
    let names = pairs("host-1=Hosty;host-2=Hosty Two;friend-1=Friendly;friend-2=Pal");
    let usernames = pairs("pal=friend-2;friendly=friend-1;hosty=host-1");
    FixedAccounts { names, usernames }
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
            limits: DEFAULT_REALM_LIMITS,
            ids: Arc::<CountingIds>::default(),
            limiter: Arc::new(NoLimit),
            events: None,
            accounts: Arc::new(accounts()),
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
    let Options {
        accounts,
        events: sink,
        limiter,
        ids,
        limits,
    } = options;
    let clock = Arc::new(ManualClock::at(UnixMillis(T0)));
    let events = Arc::<RecordedEvents>::default();
    let recorded: Arc<dyn RealmEventSink> = events.clone();
    let parts = DirectoryParts {
        log: Arc::new(QuietLog),
        events: Some(sink.unwrap_or(recorded)),
        clock: clock.clone(),
        db: target.db.clone(),
        accounts,
        limiter,
        ids,
        limits,
    };
    let directory = RealmDirectory::new(parts);
    Kit {
        directory,
        events,
        clock,
        target,
    }
}

impl Kit {
    pub fn advance(&self, millis: i64) {
        self.clock.advance(millis);
    }

    /// The host heartbeats and its Ramsgate hub runs, as session `hub-<realm>`.
    pub async fn serve(&self, host: &str, realm_id: &str) {
        let online = HeartbeatInput {
            hosting: None,
            version: None,
            online: Some(true),
        };
        self.directory.heartbeat(host, &online).await.unwrap();
        let running = HostingPatch {
            hub_session_id: Patch::Set(format!("hub-{realm_id}")),
            hub_state: Some(HubState::Running),
            ..Default::default()
        };
        let applied = self.directory.set_hosting(realm_id, &running).await.unwrap();
        assert!(applied);
    }
}

/// The refusal code, or `ok`.
pub fn code<T>(result: &Result<T, RealmsError>) -> String {
    let Err(error) = result else {
        return String::from("ok");
    };
    if let RealmsError::Refused(refusal) = error {
        refusal.code.clone()
    } else {
        format!("{error:?}")
    }
}
