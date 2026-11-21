// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Auth's realms seam over rifts (spec §5.6.2).

use std::sync::Arc;

use dainty_module_api::BoxFuture;
use rustic_isles_auth::{
    LauncherAccount, LauncherError, RealmError, Realms, RuntimeAuth, RuntimeCaller, RuntimeRequest,
};
use rustic_rifts_seam::{AuthOutcome, RealmId, RequestInfo, Rifts, SeamError};
use serde_json::Value;

/// Auth's [`Realms`] answered by rifts: launcher accounts and runtime callers go through the seam.
#[derive(Clone)]
pub struct RealmsAdapter {
    rifts: Rifts,
}

impl std::fmt::Debug for RealmsAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealmsAdapter")
            .field("rifts_loaded", &self.rifts.loaded())
            .finish()
    }
}

impl RealmsAdapter {
    /// Wraps the rifts seam.
    pub fn new(rifts: Rifts) -> Self {
        Self { rifts }
    }
}

fn request_info(request: &RuntimeRequest<'_>) -> RequestInfo {
    let headers = request
        .headers
        .iter()
        .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
        .collect();
    RequestInfo {
        path: request.path.to_owned(),
        headers,
        remote: request.peer.map(|peer| peer.ip()),
        request_id: request.request_id.unwrap_or("").to_owned(),
    }
}

fn seam_text(error: &SeamError) -> String {
    match error {
        SeamError::Refused(refusal) => refusal.message.clone(),
        other => other.to_string(),
    }
}

impl Realms for RealmsAdapter {
    fn authenticate_runtime<'a>(&'a self, request: RuntimeRequest<'a>) -> BoxFuture<'a, Result<RuntimeAuth, String>> {
        let info = request_info(&request);
        Box::pin(async move {
            match self.rifts.realms().authenticate(info).await {
                Ok(AuthOutcome::Authenticated(runtime)) => {
                    let roster = runtime.roster.iter().map(|account| account.0.clone());
                    let caller = RuntimeCaller::new(roster.collect::<Vec<_>>()).with_handle(Arc::new(runtime));
                    Ok(RuntimeAuth::Authenticated(caller))
                }
                Ok(AuthOutcome::Refused(reply)) => Ok(RuntimeAuth::Refused {
                    status: reply.status,
                    body: serde_json::from_str(&reply.body.0).unwrap_or(Value::String(reply.body.0)),
                }),
                Ok(AuthOutcome::NotMine) => Ok(RuntimeAuth::NotMine),
                Err(error) => Err(error.to_string()),
            }
        })
    }

    fn launcher_policy_fields<'a>(
        &'a self,
        _account: &'a LauncherAccount,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, LauncherError>> {
        let answer = match self.rifts.realms().launcher_policy_fields() {
            Ok(fields) => Ok(fields.into_iter().map(|(name, json)| (name, json.0)).collect()),
            Err(SeamError::Refused(refusal)) => Err(LauncherError::new(&refusal.code, refusal.message)),
            Err(_) => Err(LauncherError::internal()),
        };
        Box::pin(std::future::ready(answer))
    }

    fn authorize_game_session<'a>(
        &'a self,
        account: &'a LauncherAccount,
        realm: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<String>, RealmError>> {
        Box::pin(async move {
            let user = rustic_rifts_seam::AccountId(account.user_id.clone());
            let realm = realm.map(|text| rustic_rifts_seam::Json(text.to_owned()));
            match self.rifts.entry().authorize_game_session_realm(user, realm).await {
                Ok(entry) => Ok(entry.map(|e| e.realm.to_string())),
                Err(SeamError::Refused(refusal)) => Err(RealmError::Refused {
                    code: refusal.code,
                    message: refusal.message,
                }),
                Err(other) => Err(RealmError::Failed(other.to_string())),
            }
        })
    }

    fn bind_realm_presence<'a>(
        &'a self,
        account: &'a LauncherAccount,
        realm_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let realm = match realm_id {
                Some(text) => Some(text.parse::<RealmId>().map_err(|e| e.to_string())?),
                None => None,
            };
            let user = rustic_rifts_seam::AccountId(account.user_id.clone());
            self.rifts
                .entry()
                .bind_realm_presence_at_login(user, realm)
                .await
                .map_err(|e| seam_text(&e))
        })
    }
}
