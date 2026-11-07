// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What the area asks of the rest of isles (spec §11.2). Each default changes nothing: the crate runs alone.

use std::any::Any;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::HeaderMap;
use dainty_module_api::BoxFuture;
use dainty_module_api::NodeIdentity;
use dainty_module_api::network::{HomeApi, StandaloneHome};
use serde_json::Value;

use crate::discord::{DiscordApi, HttpsDiscord};
use crate::errors::LauncherError;
use crate::launcher_build::{LauncherReport, LauncherVerdict};
use crate::mail::{MailTransport, SmtpTransport};
use crate::store::{GameAccount, LauncherAccount};

/// Activity monitor and matchmaking presence.
pub trait Activity: Send + Sync {
    /// Records that the user started a game session at `at_ms`.
    fn touch_game_session<'a>(&'a self, _user_id: &'a str, _at_ms: i64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    /// From the pinned launcher hash and the served version (`launcher_build::launcher_verdict`)
    fn classify_launcher_build<'a>(&'a self, _report: Option<&'a LauncherReport>) -> BoxFuture<'a, LauncherVerdict> {
        Box::pin(async { LauncherVerdict::Unknown })
    }
    /// Records the launcher build the user reported and its verdict.
    fn record_launcher_build<'a>(
        &'a self,
        _user_id: &'a str,
        _report: Option<&'a LauncherReport>,
        _verdict: LauncherVerdict,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    /// Marks the user present for matchmaking at `at_ms`.
    fn touch_matchmaking_presence<'a>(&'a self, _user_id: &'a str, _at_ms: i64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// The operations settings the launcher policy reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationsPolicy {
    /// The guard's mode, such as `ENFORCE`.
    pub guard_enforcement: String,
    /// `PRODUCTION`, or `DEVELOPMENT` for admins' diagnostics.
    pub diagnostics_profile: String,
    /// Bumped on every change; part of the policy version.
    pub version: u64,
}

impl Default for OperationsPolicy {
    fn default() -> Self {
        Self {
            guard_enforcement: "ENFORCE".into(),
            diagnostics_profile: "PRODUCTION".into(),
            version: 0,
        }
    }
}

/// The operations settings, from the Reforged API.
pub trait Operations: Send + Sync {
    /// The current settings; the default is enforcing and production.
    fn policy(&self) -> BoxFuture<'_, OperationsPolicy> {
        Box::pin(async { OperationsPolicy::default() })
    }
}

/// A player host's runtime, authenticated by the realms seam.
#[derive(Clone)]
pub struct RuntimeCaller {
    roster: Vec<String>,
    /// The realms seam's own record of the runtime, opaque here; an area downcasts it to hand back to rifts' guard
    handle: Option<Arc<dyn Any + Send + Sync>>,
}

impl RuntimeCaller {
    /// Empty ids dropped.
    pub fn new(roster: impl IntoIterator<Item = String>) -> Self {
        Self {
            roster: roster.into_iter().filter(|id| !id.is_empty()).collect(),
            handle: None,
        }
    }

    /// The same caller carrying the realms seam's handle (what it sets when it authenticates a runtime).
    #[must_use]
    pub fn with_handle(mut self, handle: Arc<dyn Any + Send + Sync>) -> Self {
        self.handle = Some(handle);
        self
    }

    /// The account texts it may act for.
    pub fn roster(&self) -> &[String] {
        &self.roster
    }

    /// The realms seam's opaque record of the runtime.
    pub fn handle(&self) -> Option<&Arc<dyn Any + Send + Sync>> {
        self.handle.as_ref()
    }

    /// The handle as `T`, when it is one.
    pub fn handle_as<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.handle.as_deref()?.downcast_ref::<T>()
    }
}

// The handle may hold a capability: only whether there is one is shown
impl fmt::Debug for RuntimeCaller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeCaller")
            .field("roster", &self.roster)
            .field("handle", &self.handle.is_some())
            .finish()
    }
}

/// Equal rosters and the same handle, by identity.
impl PartialEq for RuntimeCaller {
    fn eq(&self, other: &Self) -> bool {
        let same = match (&self.handle, &other.handle) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        self.roster == other.roster && same
    }
}

impl Eq for RuntimeCaller {}

/// What the realms seam made of a request that may come from a player host's runtime.
pub enum RuntimeAuth {
    /// A runtime the seam recognised.
    Authenticated(RuntimeCaller),
    /// Answered with its status and JSON body
    Refused {
        /// The HTTP status to answer.
        status: u16,
        /// The JSON body to answer.
        body: Value,
    },
    /// Not a runtime call; the game auth goes on.
    NotMine,
}

/// What a realm refusal is.
#[derive(Debug, Clone, PartialEq)]
pub enum RealmError {
    /// A rustic-rifts code and message, answered with the table's status
    Refused {
        /// The rustic-rifts error code.
        code: String,
        /// The English text the launcher shows.
        message: String,
    },
    /// Anything else (`NOT_IMPLEMENTED`)
    Failed(String),
}

/// The parts of a game request the realms seam reads.
pub struct RuntimeRequest<'a> {
    /// The request path.
    pub path: &'a str,
    /// The request headers.
    pub headers: &'a HeaderMap,
    /// The connecting peer, when known.
    pub peer: Option<SocketAddr>,
    /// The gateway's `x-rustic-isles-request-id`.
    pub request_id: Option<&'a str>,
}

/// The realms area (rustic-rifts), as the auth area calls it.
pub trait Realms: Send + Sync {
    /// Recognises a player host's runtime call; the default recognises none.
    fn authenticate_runtime<'a>(&'a self, _request: RuntimeRequest<'a>) -> BoxFuture<'a, Result<RuntimeAuth, String>> {
        Box::pin(async { Ok(RuntimeAuth::NotMine) })
    }
    /// `(name, JSON text)` fields added to the launcher policy (`p2pEmergencyStop`, `coopHunts`)
    fn launcher_policy_fields<'a>(
        &'a self,
        _account: &'a LauncherAccount,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, LauncherError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    /// The realm a launch enters (`realm` as JSON text); the realm id to put on the exchange code
    fn authorize_game_session<'a>(
        &'a self,
        _account: &'a LauncherAccount,
        realm: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<String>, RealmError>> {
        Box::pin(async move {
            match realm {
                None => Ok(None),
                Some(_) => Err(RealmError::Refused {
                    code: "REALMS_DISABLED".into(),
                    message: "Realms are not available on this server.".into(),
                }),
            }
        })
    }
    /// Binds the account's realm presence to the code's realm or to none; also refuses a realm id that doesn't parse
    fn bind_realm_presence<'a>(
        &'a self,
        _account: &'a LauncherAccount,
        realm_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            match realm_id {
                None => Ok(()),
                Some(_) => Err("realms are off".to_owned()),
            }
        })
    }
}

/// The Reforged API's notices.
pub trait Notices: Send + Sync {
    /// The Reforged API's public view
    fn public_view(&self, now_ms: i64) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async move { Ok(empty_notices(now_ms)) })
    }
}

/// The notices view with no announcements and no tags.
pub fn empty_notices(now_ms: i64) -> Value {
    crate::util::object(vec![
        ("announcements", Value::Array(Vec::new())),
        ("tags", Value::Array(Vec::new())),
        ("serverTime", Value::String(crate::util::iso(now_ms))),
    ])
}

/// What the game's `POST /login` triggers outside auth.
pub trait GameLogin: Send + Sync {
    /// The session supplies refill at `POST /login`
    fn session_supplies<'a>(&'a self, _account: &'a GameAccount) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// Erasure's checks and clean-up outside the auth store.
pub trait Erasure: Send + Sync {
    /// Live activity that keeps an account for the next run
    fn blockers<'a>(&'a self, _account: &'a LauncherAccount) -> BoxFuture<'a, Result<Vec<String>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    /// Removes the logs the account uploaded; a failure is logged and the erasure stands.
    fn remove_uploaded_logs<'a>(&'a self, _account: &'a LauncherAccount) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// The trait defaults, as a value.
#[derive(Debug, Clone, Copy, Default)]
pub struct Defaults;

impl Activity for Defaults {}
impl Operations for Defaults {}
impl Realms for Defaults {}
impl Notices for Defaults {}
impl GameLogin for Defaults {}
impl Erasure for Defaults {}

/// Everything the auth area calls outside itself; the defaults do nothing.
pub struct Seams {
    /// Session and launcher activity.
    pub activity: Arc<dyn Activity>,
    /// The operations settings.
    pub operations: Arc<dyn Operations>,
    /// The realms area.
    pub realms: Arc<dyn Realms>,
    /// The notices.
    pub notices: Arc<dyn Notices>,
    /// The game login's side effects.
    pub game_login: Arc<dyn GameLogin>,
    /// The erasure checks.
    pub erasure: Arc<dyn Erasure>,
    /// The home node's API.
    pub home: Arc<dyn HomeApi>,
    /// How codes are mailed.
    pub mail: Arc<dyn MailTransport>,
    /// The Discord calls.
    pub discord: Arc<dyn DiscordApi>,
}

impl Default for Seams {
    fn default() -> Self {
        Self {
            activity: Arc::new(Defaults),
            operations: Arc::new(Defaults),
            realms: Arc::new(Defaults),
            notices: Arc::new(Defaults),
            game_login: Arc::new(Defaults),
            erasure: Arc::new(Defaults),
            home: Arc::new(StandaloneHome {
                identity: NodeIdentity::standalone(),
            }),
            mail: Arc::new(SmtpTransport),
            discord: Arc::new(HttpsDiscord::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Capability(&'static str);

    #[test]
    fn runtime_caller_handle() {
        let bare = RuntimeCaller::new(["a".to_owned(), String::new()]);
        assert_eq!(bare.roster(), ["a"]);
        assert!(bare.handle().is_none() && bare.handle_as::<Capability>().is_none());
        let handle: Arc<dyn Any + Send + Sync> = Arc::new(Capability("secret"));
        let caller = bare.clone().with_handle(Arc::clone(&handle));
        assert_eq!(caller.handle_as::<Capability>(), Some(&Capability("secret")));
        assert!(caller.handle_as::<String>().is_none());
        assert!(Arc::ptr_eq(caller.handle().unwrap(), &handle));
        assert_eq!(caller.clone(), caller);
        assert_ne!(caller, bare);
        assert_ne!(caller, bare.clone().with_handle(Arc::new(Capability("secret"))));
        let shown = format!("{caller:?}");
        assert!(shown.contains("handle: true") && !shown.contains("secret"), "{shown}");
    }
}
