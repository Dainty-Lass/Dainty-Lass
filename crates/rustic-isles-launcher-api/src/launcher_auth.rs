// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Launcher sign-in for this area's routes: the bearer half of auth's launcher auth, never the account key (public
//! spec §3.1).

use axum::response::Response;
use http::HeaderMap;
use rustic_isles::Shared;
use rustic_isles_auth::LauncherAccount;

use crate::util;

/// The signed-in launcher caller.
#[derive(Debug, Clone)]
pub struct LauncherCaller {
    /// The launcher account's id.
    pub account_id: String,
    /// The game account's userId.
    pub user_id: String,
    /// The refresh family
    pub sid: String,
    /// The launcher account.
    pub account: LauncherAccount,
}

impl LauncherCaller {
    /// Whether the account holds the role.
    pub fn has_role(&self, role: &str) -> bool {
        self.account.roles.iter().any(|r| r == role)
    }
}

/// A bearer token as auth checks it; a missing or non-bearer header is `AUTH_UNAUTHORIZED`.
pub async fn launcher_sign_in(shared: &Shared, headers: &HeaderMap) -> Result<LauncherCaller, Response> {
    let authorization = util::header_lossy(headers, "authorization");
    let bearer = authorization
        .as_deref()
        .filter(|a| ["bearer ", "Bearer ", "BEARER "].iter().any(|p| a.starts_with(p)));
    let Some(bearer) = bearer else {
        return Err(util::launcher_error("AUTH_UNAUTHORIZED", "Sign in required."));
    };
    match shared.auth().launcher_auth_bearer(Some(bearer)).await {
        Ok(auth) => Ok(LauncherCaller {
            account_id: auth.account.account_id.clone(),
            user_id: auth.user_id,
            sid: auth.sid,
            account: auth.account,
        }),
        Err(error) => Err(error.response(&util::random_uuid())),
    }
}
