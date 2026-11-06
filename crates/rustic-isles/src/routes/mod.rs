// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The rows of the route table and what each says about itself (core spec §4.2).

mod table;

/// Which of the module's services a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    /// The backend, the game client's API.
    Backend,
    /// The Director, the game servers' manager.
    Director,
}

impl Service {
    /// The service's name in the table and the errors.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Director => "director",
        }
    }
}

/// A row's HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// GET.
    Get,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// PATCH.
    Patch,
    /// DELETE.
    Delete,
}

impl Method {
    /// The method's name, upper case.
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

    /// The `http` crate's method.
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
    /// Sign-in and sessions.
    Auth,
    /// The Reforged API.
    ReforgedApi,
    /// Characters and loadouts.
    CharactersLoadouts,
    /// Inventory and reforging.
    InventoryReforge,
    /// Progression.
    Progression,
    /// Bounties and the store.
    BountiesStore,
    /// Matchmaking and parties.
    MatchmakingParties,
    /// Chat.
    Chat,
    /// Game data.
    GameData,
    /// The Director.
    Director,
    /// Rifts' guard.
    Guard,
    /// Rifts' player hosting.
    P2p,
    /// Rifts' realms.
    Realms,
}

impl Group {
    /// The group's name in the table.
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

/// Which crate answers a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Rustic Isles' areas.
    Isles,
    /// Rustic Rifts.
    Rifts,
}

impl Side {
    /// The side's name in the table.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Isles => "isles",
            Self::Rifts => "rifts",
        }
    }
}

/// When a row is mounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    /// Always.
    Always,
    /// Mounted only with the admin API on
    Admin,
    /// Mounted, but answers only for a Gauntlet origin host
    Gauntlet,
    /// Mounted only when rustic-rifts is loaded
    Rifts,
}

impl When {
    /// The condition's name in the table.
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
    /// A game version's route.
    Version,
    /// The node's own route, whatever the version.
    Node,
}

impl Scope {
    /// The scope's name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Node => "node",
        }
    }
}

/// One row of the route table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouteDef {
    /// The service it is mounted on.
    pub service: Service,
    /// Its method.
    pub method: Method,
    /// An Express path or a regex literal, exactly as registered
    pub path: &'static str,
    /// The area group it belongs to.
    pub group: Group,
    /// Which crate answers it.
    pub side: Side,
    /// When it is mounted.
    pub when: When,
}

const NODE_PREFIXES: [&str; 4] = ["/launcher/", "/admin/", "/health/", "/reforged/api/"];

impl RouteDef {
    /// [`Scope::Node`] when the path is under a launcher, admin, health or Reforged API prefix, else
    /// [`Scope::Version`].
    pub fn scope(&self) -> Scope {
        let node =
            NODE_PREFIXES.iter().any(|p| self.path.starts_with(p)) || self.path.starts_with("/internal/v1/metrics");
        if node { Scope::Node } else { Scope::Version }
    }

    /// Whether the path is a regex literal rather than an Express path.
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

/// Every row in Express's match order.
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
    /// The set's rows of one service, in table order.
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
