// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The signed-in account's online playtime, for the account page's hours and badges.

use super::Controller;
use crate::bridge::types::ServerGame;
use crate::net::hostapi::fetch_playtime;

impl Controller {
    /// `get_playtime`: whole seconds, none without a 1.14.7 server, a key or an answer.
    pub async fn playtime(&self) -> Option<u64> {
        let sv = self.server()?;
        if sv.game == ServerGame::Revived {
            return None;
        }
        let key = self.load_key().await?;
        fetch_playtime(&sv.endpoint(), &key).await
    }
}
