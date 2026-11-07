// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The version list (DE4, on `net::versions`): the compiled-in profiles and the joined servers' ones. Every
//! server this PC joined is asked, the current one first. Each server's own account key goes to that server's
//! Epic manifest route and nowhere else (AGENTS.md).

use std::sync::LazyLock;

use super::Controller;
use crate::bridge::types::{GameVersionList, ServerGame};
use crate::net::versions::{HttpSources, JoinedServer, ManifestCache, builtin, list};

/// Sizes by manifest pin, for the whole session
static MANIFESTS: LazyLock<ManifestCache> = LazyLock::new(ManifestCache::default);

impl Controller {
    /// `game_version_list`.
    pub async fn game_version_list(&self) -> GameVersionList {
        let servers = self.joined_servers().await;
        GameVersionList {
            versions: list(&HttpSources::default(), builtin::profiles(), &servers, &MANIFESTS).await,
        }
    }

    pub(crate) async fn joined_servers(&self) -> Vec<JoinedServer> {
        let stored = self.stored();
        let mut ids = Vec::new();
        let servers: Vec<_> = stored
            .server()
            .into_iter()
            .chain(stored.saved_servers())
            .filter(|server| {
                let id = server.id();
                let first = !ids.contains(&id);
                ids.push(id);
                first
            })
            .collect();
        let mut joined = Vec::with_capacity(servers.len());
        for server in servers {
            // A 1.4.4 server's key is another flavour's and its content has no Epic manifest route
            let key = if server.game == ServerGame::Revived {
                None
            } else {
                let (slot, pin) = (server.slot(), server.pin().map(str::to_owned));
                self.with_keys(move |keys| keys.load(&slot, pin.as_deref()))
                    .await
                    .ok()
                    .flatten()
            };
            joined.push(JoinedServer {
                endpoint: server.endpoint(),
                key,
            });
        }
        joined
    }
}
