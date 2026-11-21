// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What realms take from the server around them. Isles supplies the real ones; the plain ones here serve tests and
//! a server with nothing better.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering::SeqCst};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rifts_seam::{AccountId, BoxFuture, GuardMode, IslesCore};
use serde_json::Value;

use crate::{lifecycle::GuardRole, realm::RealmEvent, time::UnixMillis};

/// What every port future resolves to: the port's error is a message for the log.
type Port<'a, T> = BoxFuture<'a, Result<T, String>>;
/// Account id to display name.
type Names = BTreeMap<String, String>;

fn port<'a, T>(work: impl Future<Output = Result<T, String>> + Send + 'a) -> Port<'a, T> {
    Box::pin(work)
}

fn settled<T: Send + 'static>(value: T) -> Port<'static, T> {
    port(ready(Ok(value)))
}

pub trait Clock: Sync + Send {
    fn now(&self) -> UnixMillis;
}

/// Isles' per-key budget.
pub trait RateLimiter: Sync + Send {
    fn is_limited(&self, key: &str, max: u32, window_ms: i64) -> bool;
}

pub trait IdSource: Sync + Send {
    fn invite_code(&self) -> String;
    fn realm_id(&self) -> String;
}

pub trait Accounts: Sync + Send {
    /// The account whose launcher username this is, trimmed and case-insensitive.
    fn resolve_username(&self, username: String) -> Port<'_, Option<String>>;
    /// Best effort: unknown accounts are absent.
    fn display_names(&self, account_ids: Vec<String>) -> Port<'_, Names>;
}

/// A failure is the caller's to log, never the request's.
pub trait RealmEventSink: Sync + Send {
    fn emit(&self, event: RealmEvent) -> Port<'_, ()>;
}

/// Nothing logged carries a secret or an address.
pub trait RealmsLog: Sync + Send {
    fn warn(&self, line: &str);
    fn info(&self, line: &str);
}

pub trait GuardSessions: Sync + Send {
    fn find_healthy(&self, account: String, session_id: String, role: GuardRole) -> Port<'_, Option<HealthyGuard>>;
}

pub trait OperationsView: Sync + Send {
    /// Enforce mode with the Guard rule on.
    fn guard_refuses(&self) -> bool;
    /// The `ICE` co-op transport, the default.
    fn ice_transport(&self) -> bool;
    fn emergency_stop(&self) -> Port<'_, bool>;
}

/// A healthy Launcher Guard session (isles keeps Launcher Guard).
#[derive(Clone, Debug, PartialEq)]
pub struct HealthyGuard {
    pub role: GuardRole,
    pub guard_session_id: String,
    pub manifest_sequence: i64,
    pub last_sequence: i64,
    pub risk_score: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UnixMillis {
        crate::time::now()
    }
}

/// A clock a test sets and moves.
#[derive(Debug)]
pub struct ManualClock {
    now: AtomicI64,
}

impl ManualClock {
    pub fn at(now: UnixMillis) -> Self {
        let UnixMillis(start) = now;
        Self { now: start.into() }
    }

    pub fn advance(&self, millis: i64) {
        self.now.fetch_add(millis, SeqCst);
    }

    pub fn set(&self, now: UnixMillis) {
        self.now.store(now.0, SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UnixMillis {
        UnixMillis(self.now.load(SeqCst))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RandomIds;

impl IdSource for RandomIds {
    fn invite_code(&self) -> String {
        crate::realm::generate_invite_code()
    }
    fn realm_id(&self) -> String {
        crate::ids::random_uuid()
    }
}

/// Fixed answers; `usernames` keys are lower case.
#[derive(Clone, Debug, Default)]
pub struct FixedAccounts {
    pub names: Names,
    pub usernames: BTreeMap<String, String>,
}

impl Accounts for FixedAccounts {
    fn resolve_username(&self, username: String) -> Port<'_, Option<String>> {
        settled(
            self.usernames
                .get(username.to_lowercase().as_str())
                .map(String::to_owned),
        )
    }

    fn display_names(&self, account_ids: Vec<String>) -> Port<'_, Names> {
        let mut known = Names::new();
        for id in account_ids {
            if let Some(name) = self.names.get(&id) {
                known.insert(id, name.to_owned());
            }
        }
        settled(known)
    }
}

/// Display names and usernames from the core's accounts.
#[derive(Clone)]
pub struct CoreAccounts(pub Arc<dyn IslesCore>);

impl Accounts for CoreAccounts {
    fn resolve_username(&self, username: String) -> Port<'_, Option<String>> {
        let Self(core) = self;
        port(async move {
            match core.find_by_display_name(username).await {
                Ok(account) => Ok(account.map(|AccountId(id)| id)),
                Err(error) => Err(error.to_string()),
            }
        })
    }

    fn display_names(&self, account_ids: Vec<String>) -> Port<'_, Names> {
        let Self(core) = self;
        port(async move {
            let mut found = Names::new();
            for id in account_ids {
                // Best effort: an account the core cannot name is left out.
                if let Ok(Some(name)) = core.display_name(AccountId(id.clone())).await {
                    found.insert(id, name);
                }
            }
            Ok(found)
        })
    }
}

#[derive(Debug, Default)]
pub struct RecordedEvents {
    events: Mutex<Vec<RealmEvent>>,
}

impl RecordedEvents {
    fn held(&self) -> MutexGuard<'_, Vec<RealmEvent>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn all(&self) -> Vec<RealmEvent> {
        self.held().to_vec()
    }

    pub fn take(&self) -> Vec<RealmEvent> {
        self.held().drain(..).collect()
    }
}

impl RealmEventSink for RecordedEvents {
    fn emit(&self, event: RealmEvent) -> Port<'_, ()> {
        self.held().push(event);
        settled(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StderrLog;

#[derive(Clone, Copy, Debug, Default)]
pub struct QuietLog;

impl RealmsLog for StderrLog {
    fn warn(&self, line: &str) {
        eprintln!("warn {line}");
    }
    fn info(&self, line: &str) {
        eprintln!("info {line}");
    }
}

impl RealmsLog for QuietLog {
    fn warn(&self, _: &str) {}
    fn info(&self, _: &str) {}
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoGuardSessions;

impl GuardSessions for NoGuardSessions {
    fn find_healthy(&self, _: String, _: String, _: GuardRole) -> Port<'_, Option<HealthyGuard>> {
        settled(None)
    }
}

/// The core's Launcher Guard sessions, read from the JSON it keeps.
#[derive(Clone)]
pub struct CoreGuardSessions(pub Arc<dyn IslesCore>);

/// The stored session, if it is the one asked for; a record without an id matches any.
fn healthy_from_record(record: &Value, asked_id: String, role: GuardRole) -> Option<HealthyGuard> {
    let whole = |key: &str| record.get(key).and_then(Value::as_i64).unwrap_or(0);
    let guard_session_id = match record.get("guardSessionId").and_then(Value::as_str) {
        None | Some("") => asked_id,
        Some(stored) if stored == asked_id => asked_id,
        Some(_) => return None,
    };
    let last_key = if record.get("lastSequence").is_some() {
        "lastSequence"
    } else {
        "sequence"
    };
    Some(HealthyGuard {
        manifest_sequence: whole("manifestSequence"),
        last_sequence: whole(last_key),
        risk_score: record.get("riskScore").and_then(Value::as_f64).unwrap_or(0.0),
        role,
        guard_session_id,
    })
}

impl GuardSessions for CoreGuardSessions {
    fn find_healthy(&self, account: String, session_id: String, role: GuardRole) -> Port<'_, Option<HealthyGuard>> {
        let Self(core) = self;
        port(async move {
            let record = match core.healthy_guard_session(AccountId(account)).await {
                Ok(Some(json)) => serde_json::from_str::<Value>(&json.0).ok(),
                _ => None,
            };
            Ok(record.and_then(|record| healthy_from_record(&record, session_id, role)))
        })
    }
}

#[derive(Debug, Default)]
pub struct DefaultOperations {
    pub stopped: AtomicBool,
}

impl OperationsView for DefaultOperations {
    fn guard_refuses(&self) -> bool {
        false
    }
    fn ice_transport(&self) -> bool {
        true
    }
    fn emergency_stop(&self) -> Port<'_, bool> {
        settled(self.stopped.load(SeqCst))
    }
}

/// The core's operations policy and Guard mode.
#[derive(Clone)]
pub struct CoreOperationsView(pub Arc<dyn IslesCore>);

impl OperationsView for CoreOperationsView {
    fn guard_refuses(&self) -> bool {
        matches!(self.0.guard_mode(), GuardMode::Enforce)
    }
    fn ice_transport(&self) -> bool {
        DefaultOperations::default().ice_transport()
    }
    fn emergency_stop(&self) -> Port<'_, bool> {
        let Self(core) = self;
        port(async move {
            // An unreadable policy is no stop.
            let policy = match core.operations_policy().await {
                Ok(json) => serde_json::from_str::<Value>(&json.0).unwrap_or_default(),
                Err(_) => Value::Null,
            };
            Ok(policy["p2pEmergencyStop"].as_bool() == Some(true))
        })
    }
}

#[cfg(test)]
mod tests {
    use rifts_seam::{Candidate, Json, NoCore, Party, PartyId, SeamError, SessionId, poll_once};

    use super::*;

    type Seam<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

    #[test]
    fn manual_clock_moves() {
        let clock = ManualClock::at(UnixMillis(10));
        let read = || clock.now().0;
        clock.advance(5);
        assert_eq!(read(), 15);
        clock.set(UnixMillis(3));
        assert_eq!(read(), 3);
        assert!(SystemClock.now().0 > 0);
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_owned(), b.to_owned())
    }

    #[test]
    fn fixed_accounts_and_events() {
        let accounts = FixedAccounts {
            names: [pair("a", "Ann")].into(),
            usernames: [pair("pal", "friend-2")].into(),
        };
        let looked_up = poll_once(accounts.display_names(vec!["a".into(), "b".into()]));
        assert_eq!(looked_up, Some(Ok(Names::from([pair("a", "Ann")]))));
        let resolved = poll_once(accounts.resolve_username("PAL".into()));
        assert_eq!(resolved, Some(Ok(Some(String::from("friend-2")))));

        let sink = RecordedEvents::default();
        let offline = RealmEvent::HostOffline {
            realm_id: String::from("r"),
            host_account_id: String::from("h"),
        };
        assert_eq!(poll_once(sink.emit(offline.clone())), Some(Ok(())));
        assert_eq!(sink.all(), vec![offline.clone()]);
        assert_eq!(sink.take(), vec![offline]);
        assert_eq!(sink.all(), vec![]);

        assert_eq!((RandomIds.invite_code().len(), RandomIds.realm_id().len()), (10, 36));
    }

    /// The core answers only guard sessions, the operations policy and one username; the rest is `NoCore`'s.
    struct Canned {
        guard: &'static str,
        policy: &'static str,
    }

    fn canned(guard: &'static str, policy: &'static str) -> Arc<dyn IslesCore> {
        Arc::new(Canned { guard, policy })
    }

    impl IslesCore for Canned {
        fn healthy_guard_session(&self, _: AccountId) -> Seam<'_, Option<Json>> {
            Box::pin(ready(Ok(Some(Json(self.guard.to_owned())))))
        }
        fn operations_policy(&self) -> Seam<'_, Json> {
            Box::pin(ready(Ok(Json(self.policy.to_owned()))))
        }
        fn find_by_display_name(&self, name: String) -> Seam<'_, Option<AccountId>> {
            let found = if name == "Pal" {
                Some(AccountId("friend-2".to_owned()))
            } else {
                None
            };
            Box::pin(ready(Ok(found)))
        }
        fn guard_mode(&self) -> GuardMode {
            GuardMode::Enforce
        }
        fn dedicated_servers_configured(&self) -> bool {
            false
        }
        fn display_name(&self, account: AccountId) -> Seam<'_, Option<String>> {
            NoCore.display_name(account)
        }
        fn launcher_account_eligible(&self, account: AccountId) -> Seam<'_, bool> {
            NoCore.launcher_account_eligible(account)
        }
        fn party(&self, party: PartyId) -> Seam<'_, Option<Party>> {
            NoCore.party(party)
        }
        fn get_or_create_party(&self, account: AccountId) -> Seam<'_, Party> {
            NoCore.get_or_create_party(account)
        }
        fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError> {
            NoCore.set_candidate(account, candidate)
        }
        fn cancel_travel(&self, hunt: &str, accounts: &[AccountId], why: &str) -> Result<bool, SeamError> {
            NoCore.cancel_travel(hunt, accounts, why)
        }
        fn touch_game_session(&self, account: AccountId) -> Seam<'_, ()> {
            NoCore.touch_game_session(account)
        }
        fn record_hunt_activity(&self, session: SessionId, activity: Json) -> Seam<'_, ()> {
            NoCore.record_hunt_activity(session, activity)
        }
        fn published_guard_artifacts(&self) -> Seam<'_, Json> {
            NoCore.published_guard_artifacts()
        }
        fn verify_operations_step_up(&self, key: String, step_up: String) -> Seam<'_, bool> {
            NoCore.verify_operations_step_up(key, step_up)
        }
    }

    #[test]
    fn core_adapters() {
        let open = canned(
            r#"{"guardSessionId":"","manifestSequence":4,"sequence":9,"riskScore":0.25}"#,
            r#"{"p2pEmergencyStop":true}"#,
        );
        let mismatched = canned(r#"{"guardSessionId":"g-2"}"#, "not json");
        let lookup = |core: &Arc<dyn IslesCore>, role| {
            let sessions = CoreGuardSessions(Arc::clone(core));
            poll_once(sessions.find_healthy("a".into(), "g-1".into(), role))
        };

        let expected = HealthyGuard {
            role: GuardRole::Host,
            guard_session_id: "g-1".into(),
            manifest_sequence: 4,
            last_sequence: 9,
            risk_score: 0.25,
        };
        assert_eq!(lookup(&open, GuardRole::Host), Some(Ok(Some(expected))));
        assert_eq!(lookup(&mismatched, GuardRole::Client), Some(Ok(None)));

        let stopped = CoreOperationsView(Arc::clone(&open));
        assert_eq!(poll_once(stopped.emergency_stop()), Some(Ok(true)));
        assert!(stopped.guard_refuses());
        assert!(stopped.ice_transport());
        assert_eq!(
            poll_once(CoreOperationsView(mismatched).emergency_stop()),
            Some(Ok(false))
        );

        let names = CoreAccounts(open);
        assert_eq!(
            poll_once(names.resolve_username("Pal".into())),
            Some(Ok(Some(String::from("friend-2"))))
        );
        assert_eq!(poll_once(names.display_names(vec!["a".into()])), Some(Ok(Names::new())));
    }
}
