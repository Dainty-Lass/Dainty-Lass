// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A Ramsgate Reborn claim code, redeemed for the signed-in account before its first play (docs/legacy-ramsgate.md).

use super::Controller;
use crate::bridge::types::ServerGame;
use crate::net::hostapi::redeem_legacy_claim;

/// What the page shows when the server gave no answer it knows: no server, no key, or no reply.
pub const UNAVAILABLE: &str = "unavailable";
/// A code's most characters, dashes and spaces included.
pub const CODE_MAX: usize = 64;

impl Controller {
    /// `redeem_legacy_claim`: the server's answer code, or [`UNAVAILABLE`]; the page keys its message by it.
    pub async fn redeem_legacy_claim(&self, code: &str) -> &'static str {
        let code = code.trim();
        if code.is_empty() || code.len() > CODE_MAX {
            return "invalid_code";
        }
        let Some(sv) = self.server().filter(|sv| sv.game != ServerGame::Revived) else {
            return UNAVAILABLE;
        };
        let Some(key) = self.load_key().await else {
            return UNAVAILABLE;
        };
        redeem_legacy_claim(&sv.endpoint(), &key, code)
            .await
            .unwrap_or(UNAVAILABLE)
    }
}
