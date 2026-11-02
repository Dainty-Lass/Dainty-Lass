// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What realms take from the server around them. Isles supplies the real ones; the plain ones here serve tests and
//! a server with nothing better.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rifts_seam::{AccountId, BoxFuture, GuardMode, IslesCore};
use serde_json::Value;

use crate::lifecycle::GuardRole;
use crate::realm::{RealmEvent, generate_invite_code};
use crate::time::UnixMillis;

pub trait Clock: Send + Sync {
    fn now(&self) -> UnixMillis;
}

#[derive(Debug, Clone, Copy, Default)]
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
        Self {
            now: AtomicI64::new(now.0),
        }
    }

    pub fn set(&self, now: UnixMillis) {
        self.now.store(now.0, Ordering::SeqCst);
    }

    pub fn advance(&self, millis: i64) {
        self.now.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UnixMillis {
        UnixMillis(self.now.load(Ordering::SeqCst))
    }
}

/// Isles' per-key budget.
pub trait RateLimiter: Send + Sync {
    fn is_limited(&self, key: &str, max: u32, window_ms: i64) -> bool;
}

pub trait IdSource: Send + Sync {
    fn realm_id(&self) -> String;
    fn invite_code(&self) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RandomIds;

impl IdSource for RandomIds {
    fn realm_id(&self) -> String {
        crate::ids::random_uuid()
    }

    fn invite_code(&self) -> String {
        generate_invite_code()
    }
}

pub trait Accounts: Send + Sync {
    /// Best effort: unknown accounts are absent.
    fn display_names(&self, account_ids: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>>;
    /// The account whose launcher username this is, trimmed and case-insensitive.
    fn resolve_username(&self, username: String) -> BoxFuture<'_, Result<Option<String>, String>>;
}

/// Fixed answers; `usernames` keys are lower case.
#[derive(Debug, Clone, Default)]
pub struct FixedAccounts {
    pub names: BTreeMap<String, String>,
    pub usernames: BTreeMap<String, String>,
}

impl Accounts for FixedAccounts {
    fn display_names(&self, account_ids: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>> {
        let known = account_ids
            .into_iter()
            .filter_map(|id| self.names.get(&id).map(|name| (id, name.clone())))
            .collect();
        Box::pin(std::future::ready(Ok(known)))
    }

    fn resolve_username(&self, username: String) -> BoxFuture<'_, Result<Option<String>, String>> {
        let found = self.usernames.get(&username.to_lowercase()).cloned();
        Box::pin(std::future::ready(Ok(found)))
    }
}

/// A failure is the caller's to log, never the request's.
pub trait RealmEventSink: Send + Sync {
    fn emit(&self, event: RealmEvent) -> BoxFuture<'_, Result<(), String>>;
}

#[derive(Debug, Default)]
pub struct RecordedEvents {
    events: Mutex<Vec<RealmEvent>>,
}

impl RecordedEvents {
    pub fn take(&self) -> Vec<RealmEvent> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn all(&self) -> Vec<RealmEvent> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl RealmEventSink for RecordedEvents {
    fn emit(&self, event: RealmEvent) -> BoxFuture<'_, Result<(), String>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).push(event);
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Nothing logged carries a secret or an address.
pub trait RealmsLog: Send + Sync {
    fn info(&self, line: &str);
    fn warn(&self, line: &str);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StderrLog;

impl RealmsLog for StderrLog {
    fn info(&self, line: &str) {
        eprintln!("info {line}");
    }

    fn warn(&self, line: &str) {
        eprintln!("warn {line}");
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct QuietLog;

impl RealmsLog for QuietLog {
    fn info(&self, _line: &str) {}

    fn warn(&self, _line: &str) {}
}

/// A healthy Launcher Guard session (isles keeps Launcher Guard).
#[derive(Debug, Clone, PartialEq)]
pub struct HealthyGuard {
    pub role: GuardRole,
    pub guard_session_id: String,
    pub manifest_sequence: i64,
    pub last_sequence: i64,
    pub risk_score: f64,
}

pub trait GuardSessions: Send + Sync {
    fn find_healthy(
        &self,
        account: String,
        guard_session_id: String,
        role: GuardRole,
    ) -> BoxFuture<'_, Result<Option<HealthyGuard>, String>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoGuardSessions;

impl GuardSessions for NoGuardSessions {
    fn find_healthy(
        &self,
        _account: String,
        _guard_session_id: String,
        _role: GuardRole,
    ) -> BoxFuture<'_, Result<Option<HealthyGuard>, String>> {
        Box::pin(std::future::ready(Ok(None)))
    }
}

pub trait OperationsView: Send + Sync {
    fn emergency_stop(&self) -> BoxFuture<'_, Result<bool, String>>;
    /// The `ICE` co-op transport, the default.
    fn ice_transport(&self) -> bool;
    /// Enforce mode with the Guard rule on.
    fn guard_refuses(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct DefaultOperations {
    pub stopped: AtomicBool,
}

impl OperationsView for DefaultOperations {
    fn emergency_stop(&self) -> BoxFuture<'_, Result<bool, String>> {
        Box::pin(std::future::ready(Ok(self.stopped.load(Ordering::SeqCst))))
    }

    fn ice_transport(&self) -> bool {
        true
    }

    fn guard_refuses(&self) -> bool {
        false
    }
}

/// Display names and usernames from the core's accounts.
#[derive(Clone)]
pub struct CoreAccounts(pub Arc<dyn IslesCore>);

impl Accounts for CoreAccounts {
    fn display_names(&self, account_ids: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>> {
        Box::pin(async move {
            let mut names = BTreeMap::new();
            for id in account_ids {
                if let Ok(Some(name)) = self.0.display_name(AccountId(id.clone())).await {
                    names.insert(id, name);
                }
            }
            Ok(names)
        })
    }

    fn resolve_username(&self, username: String) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            self.0
                .find_by_display_name(username)
                .await
                .map(|found| found.map(|account| account.0))
                .map_err(|e| e.to_string())
        })
    }
}

fn integer(value: Option<&Value>) -> i64 {
    value.and_then(Value::as_i64).unwrap_or(0)
}

/// The core's Launcher Guard sessions, read from the JSON it keeps.
#[derive(Clone)]
pub struct CoreGuardSessions(pub Arc<dyn IslesCore>);

impl GuardSessions for CoreGuardSessions {
    fn find_healthy(
        &self,
        account: String,
        guard_session_id: String,
        role: GuardRole,
    ) -> BoxFuture<'_, Result<Option<HealthyGuard>, String>> {
        Box::pin(async move {
            let Ok(Some(json)) = self.0.healthy_guard_session(AccountId(account)).await else {
                return Ok(None);
            };
            let Ok(record) = serde_json::from_str::<Value>(&json.0) else {
                return Ok(None);
            };
            let stored = record.get("guardSessionId").and_then(Value::as_str).unwrap_or("");
            if !stored.is_empty() && stored != guard_session_id {
                return Ok(None);
            }
            let last = record.get("lastSequence").or_else(|| record.get("sequence"));
            Ok(Some(HealthyGuard {
                role,
                guard_session_id: if stored.is_empty() {
                    guard_session_id
                } else {
                    stored.to_owned()
                },
                manifest_sequence: integer(record.get("manifestSequence")),
                last_sequence: integer(last),
                risk_score: record.get("riskScore").and_then(Value::as_f64).unwrap_or(0.0),
            }))
        })
    }
}

/// The core's operations policy and Guard mode.
#[derive(Clone)]
pub struct CoreOperationsView(pub Arc<dyn IslesCore>);

impl OperationsView for CoreOperationsView {
    fn emergency_stop(&self) -> BoxFuture<'_, Result<bool, String>> {
        Box::pin(async move {
            let stopped = self
                .0
                .operations_policy()
                .await
                .ok()
                .and_then(|json| serde_json::from_str::<Value>(&json.0).ok())
                .and_then(|policy| policy.get("p2pEmergencyStop").and_then(Value::as_bool))
                .unwrap_or(false);
            Ok(stopped)
        })
    }

    fn ice_transport(&self) -> bool {
        true
    }

    fn guard_refuses(&self) -> bool {
        self.0.guard_mode() == GuardMode::Enforce
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::{Json, NoCore, poll_once};

    #[test]
    fn manual_clock_moves() {
        let clock = ManualClock::at(UnixMillis(10));
        clock.advance(5);
        assert_eq!(clock.now(), UnixMillis(15));
        clock.set(UnixMillis(3));
        assert_eq!(clock.now(), UnixMillis(3));
        assert!(SystemClock.now().0 > 0);
    }

    #[test]
    fn fixed_accounts_and_events() {
        let accounts = FixedAccounts {
            names: [("a".to_owned(), "Ann".to_owned())].into(),
            usernames: [("pal".to_owned(), "friend-2".to_owned())].into(),
        };
        let names = poll_once(accounts.display_names(vec!["a".into(), "b".into()]))
            .unwrap()
            .unwrap();
        assert_eq!(names, BTreeMap::from([("a".to_owned(), "Ann".to_owned())]));
        assert_eq!(
            poll_once(accounts.resolve_username("PAL".into())).unwrap(),
            Ok(Some("friend-2".to_owned()))
        );
        let events = RecordedEvents::default();
        let event = RealmEvent::HostOffline {
            realm_id: "r".into(),
            host_account_id: "h".into(),
        };
        poll_once(events.emit(event.clone())).unwrap().unwrap();
        assert_eq!(events.all(), std::slice::from_ref(&event));
        assert_eq!(events.take(), [event]);
        assert!(events.all().is_empty());
        assert_eq!(RandomIds.invite_code().len(), 10);
        assert_eq!(RandomIds.realm_id().len(), 36);
    }

    struct Core {
        guard: &'static str,
        policy: &'static str,
    }

    impl IslesCore for Core {
        fn display_name(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, rifts_seam::SeamError>> {
            NoCore.display_name(account)
        }
        fn find_by_display_name(
            &self,
            name: String,
        ) -> BoxFuture<'_, Result<Option<AccountId>, rifts_seam::SeamError>> {
            Box::pin(std::future::ready(Ok(
                (name == "Pal").then(|| AccountId("friend-2".into()))
            )))
        }
        fn launcher_account_eligible(&self, a: AccountId) -> BoxFuture<'_, Result<bool, rifts_seam::SeamError>> {
            NoCore.launcher_account_eligible(a)
        }
        fn party(
            &self,
            p: rifts_seam::PartyId,
        ) -> BoxFuture<'_, Result<Option<rifts_seam::Party>, rifts_seam::SeamError>> {
            NoCore.party(p)
        }
        fn get_or_create_party(&self, a: AccountId) -> BoxFuture<'_, Result<rifts_seam::Party, rifts_seam::SeamError>> {
            NoCore.get_or_create_party(a)
        }
        fn set_candidate(&self, a: AccountId, c: rifts_seam::Candidate) -> Result<(), rifts_seam::SeamError> {
            NoCore.set_candidate(a, c)
        }
        fn cancel_travel(&self, h: &str, a: &[AccountId], r: &str) -> Result<bool, rifts_seam::SeamError> {
            NoCore.cancel_travel(h, a, r)
        }
        fn dedicated_servers_configured(&self) -> bool {
            false
        }
        fn touch_game_session(&self, a: AccountId) -> BoxFuture<'_, Result<(), rifts_seam::SeamError>> {
            NoCore.touch_game_session(a)
        }
        fn record_hunt_activity(
            &self,
            s: rifts_seam::SessionId,
            j: Json,
        ) -> BoxFuture<'_, Result<(), rifts_seam::SeamError>> {
            NoCore.record_hunt_activity(s, j)
        }
        fn healthy_guard_session(&self, _a: AccountId) -> BoxFuture<'_, Result<Option<Json>, rifts_seam::SeamError>> {
            Box::pin(std::future::ready(Ok(Some(Json(self.guard.into())))))
        }
        fn published_guard_artifacts(&self) -> BoxFuture<'_, Result<Json, rifts_seam::SeamError>> {
            NoCore.published_guard_artifacts()
        }
        fn guard_mode(&self) -> GuardMode {
            GuardMode::Enforce
        }
        fn operations_policy(&self) -> BoxFuture<'_, Result<Json, rifts_seam::SeamError>> {
            Box::pin(std::future::ready(Ok(Json(self.policy.into()))))
        }
        fn verify_operations_step_up(
            &self,
            k: String,
            s: String,
        ) -> BoxFuture<'_, Result<bool, rifts_seam::SeamError>> {
            NoCore.verify_operations_step_up(k, s)
        }
    }

    #[test]
    fn core_adapters() {
        let core = Arc::new(Core {
            guard: r#"{"guardSessionId":"","manifestSequence":4,"sequence":9,"riskScore":0.25}"#,
            policy: r#"{"p2pEmergencyStop":true}"#,
        });
        let guard = poll_once(CoreGuardSessions(core.clone()).find_healthy("a".into(), "g-1".into(), GuardRole::Host))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            guard,
            HealthyGuard {
                role: GuardRole::Host,
                guard_session_id: "g-1".into(),
                manifest_sequence: 4,
                last_sequence: 9,
                risk_score: 0.25,
            }
        );
        let other = Arc::new(Core {
            guard: r#"{"guardSessionId":"g-2"}"#,
            policy: "not json",
        });
        assert_eq!(
            poll_once(CoreGuardSessions(other.clone()).find_healthy("a".into(), "g-1".into(), GuardRole::Client)),
            Some(Ok(None))
        );
        let view = CoreOperationsView(core.clone());
        assert_eq!(poll_once(view.emergency_stop()), Some(Ok(true)));
        assert!(view.guard_refuses() && view.ice_transport());
        assert_eq!(poll_once(CoreOperationsView(other).emergency_stop()), Some(Ok(false)));
        let accounts = CoreAccounts(core);
        assert_eq!(
            poll_once(accounts.resolve_username("Pal".into())),
            Some(Ok(Some("friend-2".to_owned())))
        );
        assert_eq!(
            poll_once(accounts.display_names(vec!["a".into()])),
            Some(Ok(BTreeMap::new()))
        );
    }
}
