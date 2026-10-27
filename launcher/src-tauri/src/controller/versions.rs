//! The version list (DE4, on `net::versions`): the compiled-in profiles and the joined servers' ones. Every
//! server this PC joined is asked, the current one first. Each server's own account key goes to that server's
//! Epic manifest route and nowhere else (AGENTS.md).

use std::sync::LazyLock;

use super::Controller;
use crate::bridge::types::GameVersionList;
use crate::net::versions::{HttpSources, JoinedServer, ManifestCache, builtin, list};

/// Sizes by manifest pin, for the whole session
static MANIFESTS: LazyLock<ManifestCache> = LazyLock::new(ManifestCache::default);

impl Controller {
    pub async fn game_version_list(&self) -> GameVersionList {
        let servers = self.joined_servers();
        GameVersionList {
            versions: list(&HttpSources::default(), builtin::profiles(), &servers, &MANIFESTS).await,
        }
    }

    pub(super) fn joined_servers(&self) -> Vec<JoinedServer> {
        let s = self.s();
        let mut ids = Vec::new();
        s.server
            .iter()
            .chain(&s.saved_servers)
            .filter(|server| {
                let id = server.id();
                let first = !ids.contains(&id);
                ids.push(id);
                first
            })
            .map(|server| JoinedServer {
                endpoint: server.endpoint(),
                // A 1.4.4 server's key is another flavour's and its content has no Epic manifest route
                key: if server.revived() {
                    None
                } else {
                    self.keys.load(&server.slot()).ok().flatten()
                },
            })
            .collect()
    }
}
