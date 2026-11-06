// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The SASL `PLAIN` login check (spec §5.5 steps 1-5). Identity comes from the game token, never the JID; the
//! reasons are for the log only (the client sees the plugin's `not-authorized` or `temporary-auth-failure`).

use std::fmt;

use serde_json::Value;

use crate::{CLIENT_CREDENTIALS_USER_ID, ChatSource, Player};

/// Why a login was refused; logged, never shown to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The token did not verify, or there is no token check.
    JwtVerifyFailed,
    /// The token carries no string `userId`.
    NoUserId,
    /// The token is a client-credentials one, not a player's.
    ClientCredentials,
    /// The SASL authcid is not the token's `userId`.
    Mismatch,
    /// No account answers to that `userId`.
    NotEligible,
    /// The account lookup failed.
    LookupError,
}

impl Refusal {
    /// The reason as the log line writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JwtVerifyFailed => "jwt verify failed",
            Self::NoUserId => "no userId in token",
            Self::ClientCredentials => "client-credentials token not allowed",
            Self::Mismatch => "authcid/token mismatch",
            Self::NotEligible => "account not eligible",
            Self::LookupError => "account lookup error",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The player a SASL `PLAIN` login names, checked in the order of spec §5.5; the first failing step is the refusal.
pub async fn authenticate(authcid: &str, token: &str, source: &dyn ChatSource) -> Result<Player, Refusal> {
    let claims = source
        .token_check()
        .ok_or(Refusal::JwtVerifyFailed)?
        .validate(token)
        .map_err(|_| Refusal::JwtVerifyFailed)?;
    let user_id = match claims.get("userId") {
        Some(Value::String(id)) if !id.is_empty() => id.clone(),
        _ => return Err(Refusal::NoUserId),
    };
    if user_id == CLIENT_CREDENTIALS_USER_ID {
        return Err(Refusal::ClientCredentials);
    }
    if user_id != authcid {
        return Err(Refusal::Mismatch);
    }
    match source.player_by_user_id(&user_id).await {
        Ok(Some(player)) => Ok(player),
        Ok(None) => Err(Refusal::NotEligible),
        Err(_) => Err(Refusal::LookupError),
    }
}
