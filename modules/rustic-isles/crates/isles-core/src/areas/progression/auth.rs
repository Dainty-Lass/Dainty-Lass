//! `req.AuthData` as the progression handlers read it, and the host-write hooks (`extensions/p2p.ts`'s
//! `hostWrites`) they call.
//!
//! The auth middleware (`HasParadoxBackendAuth`) is RU5 auth's. This is the seam: it puts one [`AuthData`] in the
//! request's extensions, as Node's middleware sets `req.AuthData`, and a request without one is answered 401 here
//! as the middleware would. When auth lands its own type, this file's [`AuthData::of`] is the one place to change.

use axum::extract::Request;
use rifts_seam::{
    AccountId as RiftsAccount, HostWrite, HostWriteKind, HostWriteOptions, Json as RiftsJson, RuntimeAuth,
};
use serde_json::Value;

use crate::http::Shared;

use super::util::{empty, from_hook};

/// `req.AuthData`.
#[derive(Debug, Clone, Default)]
pub struct AuthData {
    /// The game server's API key checked out (`IsGameserver`)
    pub is_gameserver: bool,
    /// A player-hosted runtime's capability checked out (`IsPlayerHostRuntime`)
    pub is_player_host_runtime: bool,
    /// `userId` of the token, or the configured no-player account of a bearer-less game server call
    pub user_id: Option<String>,
    /// `RuntimeRosterAccountIds`
    pub roster: Vec<String>,
    /// The capability behind a host runtime, for the hooks
    pub runtime: Option<RuntimeAuth>,
}

impl AuthData {
    pub fn of(request: &Request) -> Option<Self> {
        request.extensions().get::<Self>().cloned()
    }

    /// `IsHostRuntimeAuth`.
    pub fn is_host(&self) -> bool {
        self.is_player_host_runtime
    }

    /// `IsAuthorizedProgressionReader`.
    pub fn may_read(&self, target: &str) -> bool {
        self.is_gameserver || self.user_id.as_deref() == Some(target)
    }

    /// `IsAuthorizedProgressionReporter`: a host never reports through its forwarded bearer.
    pub fn may_report(&self, target: &str) -> bool {
        if self.is_gameserver {
            return true;
        }
        !self.is_player_host_runtime && self.user_id.as_deref() == Some(target)
    }

    /// `IsAuthorizedEscalationReader`: a host reads its own hunt's roster.
    pub fn may_read_escalation(&self, target: &str) -> bool {
        if self.is_gameserver {
            return true;
        }
        if self.is_player_host_runtime {
            return self.roster.iter().any(|id| id == target);
        }
        self.user_id.as_deref() == Some(target)
    }

    /// `IsAuthorizedEscalationWriter`.
    pub fn may_write_escalation(&self, target: &str) -> bool {
        self.may_report(target)
    }

    /// `IsHostEscalationWriter`.
    pub fn is_host_escalation_writer(&self) -> bool {
        self.is_player_host_runtime && !self.is_gameserver
    }

    /// `MonitorSourceOf`.
    pub fn source(&self) -> &'static str {
        if self.is_gameserver {
            "gs"
        } else if self.is_player_host_runtime {
            "host"
        } else {
            "player"
        }
    }
}

/// `hostWrites.authorize`: `Ok(None)` lets the write through, `Ok(Some(_))` is the reply that refuses it. `Err` is
/// a hook that failed, which Node answers 500.
pub async fn refuse_host_write(
    shared: &Shared,
    auth: &AuthData,
    kind: HostWriteKind,
    target: &str,
    options: HostWriteOptions,
) -> Result<Option<axum::response::Response>, axum::response::Response> {
    let Some(runtime) = auth.runtime.clone() else {
        // A host flag without a capability behind it can write nothing
        return Ok(Some(from_hook(rifts_seam::Reply {
            status: 403,
            body: RiftsJson(
                r#"{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}"#.into(),
            ),
        })));
    };
    match shared
        .rifts
        .guard()
        .authorize_host_write(runtime, kind, RiftsAccount(target.to_owned()), options)
        .await
    {
        Ok(denial) => Ok(denial.map(from_hook)),
        Err(error) => {
            tracing::error!("[HostWrite] authorize {kind:?} for {target} failed: {error}");
            Err(empty(500))
        }
    }
}

/// `RefuseUnauthorizedHostWrite` without options.
pub async fn refuse_plain(
    shared: &Shared,
    auth: &AuthData,
    kind: HostWriteKind,
    target: &str,
) -> Result<Option<axum::response::Response>, axum::response::Response> {
    refuse_host_write(shared, auth, kind, target, HostWriteOptions::default()).await
}

/// `IsAuthoritativeServerFor`: the game server key, or a host whose session holds the player (parity check).
pub async fn is_authoritative_server_for(
    shared: &Shared,
    auth: &AuthData,
    target: &str,
    kind: HostWriteKind,
) -> Result<bool, axum::response::Response> {
    if auth.is_host() {
        let options = HostWriteOptions {
            parity: true,
            ..HostWriteOptions::default()
        };
        return Ok(refuse_host_write(shared, auth, kind, target, options).await?.is_none());
    }
    Ok(auth.is_gameserver)
}

/// `TagHostWrite`: never fails, the write is already stored.
pub async fn tag_host_write(shared: &Shared, auth: &AuthData, kind: HostWriteKind, target: &str, summary: Value) {
    let Some(runtime) = auth.runtime.clone() else { return };
    let write = HostWrite {
        kind,
        target: RiftsAccount(target.to_owned()),
        summary: RiftsJson(crate::areas::progression::util::stringify(&summary)),
    };
    if let Err(error) = shared.rifts.guard().record_host_write(runtime, write).await {
        tracing::error!("[HostWrite] tag failed kind={kind:?} target={target}: {error}");
    }
}

/// `HostLimitOnlyLogs`: the two limits a dedicated server never had only log in the host policy's observe mode.
pub fn limit_only_logs(shared: &Shared, reason: &str) -> bool {
    matches!(reason, "amount_over_ceiling" | "track_full") && !host_policy_refuses(shared)
}

/// `hostPolicy.refuses()`; a hook that fails refuses.
pub fn host_policy_refuses(shared: &Shared) -> bool {
    shared.rifts.guard().host_policy_refuses().unwrap_or(true)
}

pub fn note_would_refuse(shared: &Shared, what: &str, detail: &str) {
    if let Err(error) = shared.rifts.guard().note_would_refuse(what, detail) {
        tracing::error!("[HostPolicy] noteWouldRefuse failed: {error}");
    }
}
