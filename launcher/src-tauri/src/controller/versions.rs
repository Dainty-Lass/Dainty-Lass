//! The version list (DE4, on `net::versions`): the compiled-in profiles and the joined servers' ones. Every
//! server this PC joined is asked, the current one first, without the account key.

use std::sync::LazyLock;

use super::Controller;
use crate::bridge::types::GameVersionList;
use crate::net::http::Endpoint;
use crate::net::versions::{HttpSources, ManifestCache, builtin, list};

/// Sizes by manifest pin, for the whole session
static MANIFESTS: LazyLock<ManifestCache> = LazyLock::new(ManifestCache::default);

impl Controller {
    pub async fn game_version_list(&self) -> GameVersionList {
        let servers = self.joined_endpoints();
        GameVersionList {
            versions: list(&HttpSources::default(), builtin::profiles(), &servers, &MANIFESTS).await,
        }
    }

    fn joined_endpoints(&self) -> Vec<Endpoint> {
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
            .map(|server| server.endpoint())
            .collect()
    }
}
