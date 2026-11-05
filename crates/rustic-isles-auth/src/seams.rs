// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What the area asks of the rest of isles (spec §11.2). Each default changes nothing: the crate runs alone.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::HeaderMap;
use module_api::BoxFuture;
use module_api::NodeIdentity;
use module_api::network::{HomeApi, StandaloneHome};
use serde_json::Value;

use crate::discord::{DiscordApi, HttpsDiscord};
use crate::errors::LauncherError;
use crate::launcher_build::{LauncherReport, LauncherVerdict};
use crate::mail::{MailTransport, SmtpTransport};
use crate::store::{GameAccount, LauncherAccount};

/// Activity monitor and matchmaking presence.
pub trait Activity: Send + Sync {
    fn touch_game_session<'a>(&'a self, _user_id: &'a str, _at_ms: i64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    /// From the pinned launcher hash and the served version (`launcher_build::launcher_verdict`)
    fn classify_launcher_build<'a>(&'a self, _report: Option<&'a LauncherReport>) -> BoxFuture<'a, LauncherVerdict> {
        Box::pin(async { LauncherVerdict::Unknown })
    }
    fn record_launcher_build<'a>(
        &'a self,
        _user_id: &'a str,
        _report: Option<&'a LauncherReport>,
        _verdict: LauncherVerdict,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn touch_matchmaking_presence<'a>(&'a self, _user_id: &'a str, _at_ms: i64) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationsPolicy {
    pub guard_enforcement: String,
    pub diagnostics_profile: String,
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

pub trait Operations: Send + Sync {
    fn policy(&self) -> BoxFuture<'_, OperationsPolicy> {
        Box::pin(async { OperationsPolicy::default() })
    }
}

/// A player host's runtime, authenticated by the realms seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCaller {
    roster: Vec<String>,
}

impl RuntimeCaller {
    /// Empty ids dropped.
    pub fn new(roster: impl IntoIterator<Item = String>) -> Self {
        Self {
            roster: roster.into_iter().filter(|id| !id.is_empty()).collect(),
        }
    }

    /// The account texts it may act for.
    pub fn roster(&self) -> &[String] {
        &self.roster
    }
}

pub enum RuntimeAuth {
    Authenticated(RuntimeCaller),
    /// Answered with its status and JSON body
    Refused {
        status: u16,
        body: Value,
    },
    NotMine,
}

/// What a realm refusal is.
#[derive(Debug, Clone, PartialEq)]
pub enum RealmError {
    /// A rustic-rifts code and message, answered with the table's status
    Refused { code: String, message: String },
    /// Anything else (`NOT_IMPLEMENTED`)
    Failed(String),
}

pub struct RuntimeRequest<'a> {
    pub path: &'a str,
    pub headers: &'a HeaderMap,
    pub peer: Option<SocketAddr>,
    pub request_id: Option<&'a str>,
}

pub trait Realms: Send + Sync {
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

pub trait Notices: Send + Sync {
    /// The Reforged API's public view
    fn public_view(&self, now_ms: i64) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async move { Ok(empty_notices(now_ms)) })
    }
}

pub fn empty_notices(now_ms: i64) -> Value {
    crate::util::object(vec![
        ("announcements", Value::Array(Vec::new())),
        ("tags", Value::Array(Vec::new())),
        ("serverTime", Value::String(crate::util::iso(now_ms))),
    ])
}

pub trait GameLogin: Send + Sync {
    /// The session supplies refill at `POST /login`
    fn session_supplies<'a>(&'a self, _account: &'a GameAccount) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

pub trait Erasure: Send + Sync {
    /// Live activity that keeps an account for the next run
    fn blockers<'a>(&'a self, _account: &'a LauncherAccount) -> BoxFuture<'a, Result<Vec<String>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
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

pub struct Seams {
    pub activity: Arc<dyn Activity>,
    pub operations: Arc<dyn Operations>,
    pub realms: Arc<dyn Realms>,
    pub notices: Arc<dyn Notices>,
    pub game_login: Arc<dyn GameLogin>,
    pub erasure: Arc<dyn Erasure>,
    pub home: Arc<dyn HomeApi>,
    pub mail: Arc<dyn MailTransport>,
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
