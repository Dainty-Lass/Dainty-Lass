// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The rows of the route table and what each says about itself (core spec §4.2).

mod table;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Backend,
    Director,
}

impl Service {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Director => "director",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    /// Express routes `HEAD` to `GET` rows.
    pub fn accepts(self, method: &http::Method) -> bool {
        match self {
            Self::Get => method == http::Method::GET || method == http::Method::HEAD,
            Self::Post => method == http::Method::POST,
            Self::Put => method == http::Method::PUT,
            Self::Patch => method == http::Method::PATCH,
            Self::Delete => method == http::Method::DELETE,
        }
    }

    pub fn http(self) -> http::Method {
        match self {
            Self::Get => http::Method::GET,
            Self::Post => http::Method::POST,
            Self::Put => http::Method::PUT,
            Self::Patch => http::Method::PATCH,
            Self::Delete => http::Method::DELETE,
        }
    }
}

/// RU5's area groups for isles rows, the rifts source area for rifts rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Auth,
    ReforgedApi,
    CharactersLoadouts,
    InventoryReforge,
    Progression,
    BountiesStore,
    MatchmakingParties,
    Chat,
    GameData,
    Director,
    Guard,
    P2p,
    Realms,
}

impl Group {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::ReforgedApi => "reforged-api",
            Self::CharactersLoadouts => "characters-loadouts",
            Self::InventoryReforge => "inventory-reforge",
            Self::Progression => "progression",
            Self::BountiesStore => "bounties-store",
            Self::MatchmakingParties => "matchmaking-parties",
            Self::Chat => "chat",
            Self::GameData => "game-data",
            Self::Director => "director",
            Self::Guard => "guard",
            Self::P2p => "p2p",
            Self::Realms => "realms",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Isles,
    Rifts,
}

impl Side {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Isles => "isles",
            Self::Rifts => "rifts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    Always,
    /// Mounted only with the admin API on
    Admin,
    /// Mounted, but answers only for a Gauntlet origin host
    Gauntlet,
    /// Mounted only when rustic-rifts is loaded
    Rifts,
}

impl When {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Admin => "admin",
            Self::Gauntlet => "gauntlet",
            Self::Rifts => "rifts",
        }
    }
}

/// Launcher, operator and probe routes belong to the node, not to a game version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Version,
    Node,
}

impl Scope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Node => "node",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouteDef {
    pub service: Service,
    pub method: Method,
    /// An Express path or a regex literal, exactly as registered
    pub path: &'static str,
    pub group: Group,
    pub side: Side,
    pub when: When,
}

const NODE_PREFIXES: [&str; 4] = ["/launcher/", "/admin/", "/health/", "/reforged/api/"];

impl RouteDef {
    pub fn scope(&self) -> Scope {
        let node =
            NODE_PREFIXES.iter().any(|p| self.path.starts_with(p)) || self.path.starts_with("/internal/v1/metrics");
        if node { Scope::Node } else { Scope::Version }
    }

    pub fn is_regex(&self) -> bool {
        self.path.starts_with("/^")
    }

    /// `{METHOD} {path}`
    pub fn name(&self) -> String {
        format!("{} {}", self.method.as_str(), self.path)
    }

    /// This row's position in [`all`].
    pub fn index(&self) -> usize {
        all().iter().position(|row| row == self).unwrap_or(usize::MAX)
    }
}

pub fn all() -> &'static [RouteDef] {
    &table::TABLE
}

/// A game version's share of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteSet {
    /// 1.14.7: every row
    V1147,
}

impl RouteSet {
    pub fn routes(self, service: Service) -> Vec<&'static RouteDef> {
        match self {
            Self::V1147 => all().iter().filter(|row| row.service == service).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_scopes() {
        let first = &all()[0];
        assert_eq!(first.name(), "GET /");
        assert_eq!(first.scope(), Scope::Version);
        let metrics = all().iter().find(|r| r.path == "/internal/v1/metrics").unwrap();
        assert_eq!(metrics.scope(), Scope::Node);
        assert_eq!(metrics.index(), 4);
        assert!(all()[47].is_regex());
        assert_eq!(all()[47].group.as_str(), "game-data");
        assert_eq!(Method::Get.as_str(), "GET");
        assert!(Method::Get.accepts(&http::Method::HEAD));
        assert!(!Method::Post.accepts(&http::Method::HEAD));
    }
}
