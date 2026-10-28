//! The directory's test kit (`realms/realmsTestKit.ts`): a `RealmDirectory` on a real database with a clock the test
//! moves, the recorded events, and the repositories to look underneath.
#![allow(dead_code, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use rifts_realms::RealmsError;
use rifts_realms::db::RealmsDb;
use rifts_realms::directory::{DirectoryParts, HostedReply, RealmDirectory, RealmHostView};
use rifts_realms::input::CreateRealmInput;
use rifts_realms::ports::{
    Accounts, FixedAccounts, IdSource, ManualClock, QuietLog, RateLimiter, RealmEventSink, RecordedEvents,
};
use rifts_realms::realm::{DEFAULT_REALM_LIMITS, HostingPatch, HubState, JoinPolicy, Patch, RealmEvent, RealmLimits};
use rifts_realms::repo::realms::{RealmPresenceRepository, RealmsRepository};
use rifts_seam::{BoxFuture, UnixMillis};

#[derive(Debug, Clone, Copy, Default)]
pub struct NoRateLimit;

impl RateLimiter for NoRateLimit {
    fn is_limited(&self, _: &str, _: u32, _: i64) -> bool {
        false
    }
}

pub const T0: UnixMillis = UnixMillis(1_791_892_800_000); // 2026-10-13T12:00:00.000Z
pub const DAY: i64 = 24 * 3_600_000;
pub const KEY_A: &str = "KEY-A-0123456789abcdef";
pub const KEY_B: &str = "KEY-B-0123456789abcdef";

pub fn at(offset: i64) -> UnixMillis {
    UnixMillis(T0.0 + offset)
}

/// `realm-1`, `realm-2`...; codes `ABCDEFGHJK`, `ABCDEFGHJL`... (valid, and different per call).
#[derive(Default)]
pub struct CountingIds {
    realms: Mutex<u32>,
    codes: Mutex<u32>,
}

impl IdSource for CountingIds {
    fn realm_id(&self) -> String {
        let mut n = self.realms.lock().unwrap_or_else(PoisonError::into_inner);
        *n += 1;
        format!("realm-{n}")
    }

    fn invite_code(&self) -> String {
        let mut n = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        let c = *n as usize;
        *n += 1;
        format!(
            "ABCDEFGH{}{}",
            &"JKLMNPQRST"[c / 10 % 10..][..1],
            &"KLMNPQRSTU"[c % 10..][..1]
        )
    }
}

/// Codes from a list, then a fixed one.
pub struct ListedCodes(pub Mutex<Vec<&'static str>>);

impl IdSource for ListedCodes {
    fn realm_id(&self) -> String {
        "realm-1".to_owned()
    }

    fn invite_code(&self) -> String {
        let mut codes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if codes.is_empty() {
            "ZZZZZZZZZZ".to_owned()
        } else {
            codes.remove(0).to_owned()
        }
    }
}

pub struct Limiter {
    pub keys: Mutex<Vec<String>>,
    pub limited: fn(&str) -> bool,
}

impl RateLimiter for Limiter {
    fn is_limited(&self, key: &str, _: u32, _: i64) -> bool {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        (self.limited)(key)
    }
}

pub struct FailingNames;

impl Accounts for FailingNames {
    fn display_names(&self, _: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>> {
        Box::pin(async { Err("accounts unavailable".to_owned()) })
    }

    fn resolve_username(&self, _: String) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async { Ok(None) })
    }
}

pub struct FailingSink;

impl RealmEventSink for FailingSink {
    fn emit(&self, _: RealmEvent) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Err("p2p module down".to_owned()) })
    }
}

pub struct Options {
    pub limits: RealmLimits,
    pub limiter: Arc<dyn RateLimiter>,
    pub accounts: Arc<dyn Accounts>,
    pub ids: Arc<dyn IdSource>,
    /// `None`: no sink at all.
    pub sink: Option<Arc<dyn RealmEventSink>>,
}

pub fn names() -> FixedAccounts {
    FixedAccounts {
        names: [
            ("host-1", "Hosty"),
            ("host-2", "Hosty Two"),
            ("friend-1", "Friendly"),
            ("friend-2", "Pal"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect(),
        usernames: BTreeMap::new(),
    }
}

pub fn with_usernames() -> FixedAccounts {
    FixedAccounts {
        usernames: [("pal", "friend-2"), ("friendly", "friend-1"), ("hosty", "host-1")]
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ..names()
    }
}

pub struct Kit {
    pub dir: RealmDirectory,
    pub clock: Arc<ManualClock>,
    pub events: Arc<RecordedEvents>,
    pub repo: RealmsRepository,
    pub presence: RealmPresenceRepository,
}

pub fn kit(db: RealmsDb) -> Kit {
    kit_with(db, |_| {})
}

pub fn kit_with(db: RealmsDb, change: impl FnOnce(&mut Options)) -> Kit {
    let events = Arc::new(RecordedEvents::default());
    let mut options = Options {
        limits: DEFAULT_REALM_LIMITS,
        limiter: Arc::new(NoRateLimit),
        accounts: Arc::new(names()),
        ids: Arc::new(CountingIds::default()),
        sink: Some(events.clone()),
    };
    change(&mut options);
    let clock = Arc::new(ManualClock::at(T0));
    let dir = RealmDirectory::new(DirectoryParts {
        db: db.clone(),
        accounts: options.accounts,
        events: options.sink,
        clock: clock.clone(),
        limiter: options.limiter,
        ids: options.ids,
        limits: options.limits,
        log: Arc::new(QuietLog),
    });
    Kit {
        dir,
        clock,
        events,
        repo: RealmsRepository::new(db.clone()),
        presence: RealmPresenceRepository::new(db),
    }
}

pub fn input(name: &str) -> CreateRealmInput {
    CreateRealmInput {
        name: Some(name.to_owned()),
        host_public_key: KEY_A.to_owned(),
        join_policy: JoinPolicy::InviteOnly,
        listed: false,
        version: None,
    }
}

impl Kit {
    pub fn set_now(&self, offset: i64) {
        self.clock.set(at(offset));
    }

    pub async fn create(&self, account: &str, name: &str) -> RealmHostView {
        self.create_with(account, name, |_| {}).await
    }

    pub async fn create_with(
        &self,
        account: &str,
        name: &str,
        change: impl FnOnce(&mut CreateRealmInput),
    ) -> RealmHostView {
        self.try_create(account, name, change).await.unwrap().realm
    }

    pub async fn try_create(
        &self,
        account: &str,
        name: &str,
        change: impl FnOnce(&mut CreateRealmInput),
    ) -> Result<HostedReply, RealmsError> {
        let mut request = input(name);
        change(&mut request);
        self.dir.create_realm(account, &request).await
    }

    /// The host is online and its hub is running: the realm can be entered.
    pub async fn serve(&self, host: &str, realm_id: &str) {
        self.dir.heartbeat(host, &Default::default()).await.unwrap();
        let patch = HostingPatch {
            hub_state: Some(HubState::Running),
            hub_session_id: Patch::Set(format!("hub-{realm_id}")),
            ..HostingPatch::default()
        };
        self.dir.set_hosting(realm_id, &patch).await.unwrap();
    }
}

/// The launcher error code of a refusal, or `ok`.
pub fn code<T>(result: &Result<T, RealmsError>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(RealmsError::Refused(refusal)) => refusal.code.clone(),
        Err(other) => format!("{other}"),
    }
}

#[track_caller]
pub fn rejects<T>(result: Result<T, RealmsError>, expected: &str) {
    assert_eq!(code(&result), expected);
}
