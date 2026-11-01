//! The route skeleton: every route of RU3a's inventory, in Express's match order, with what decides whether it is
//! mounted (DD8; RU3c's "what RU4's seams must expose", 1 and 7). [`RouteSet`] is one version's share of it.
//!
//! Paths match as Express 5 matches them (`check-route-inventory.mjs` models the same): case-insensitively, one
//! optional trailing slash, `:name` one non-empty segment, empty segments literal (`/loadout/:userId//all`), and a
//! regex route (`/^\/inventory\/([^/]*)\/(.+)$/`) run as written against the path.

mod table;

use percent_encoding::percent_decode_str;
use regex::Regex;

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

    /// Whether a request with `method` reaches this route: Express answers HEAD with the GET route.
    pub fn accepts(self, method: &axum::http::Method) -> bool {
        method.as_str() == self.as_str() || (self == Self::Get && method == axum::http::Method::HEAD)
    }
}

/// RU5's area groups for isles rows and the source area for rifts rows, as RU3a names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// What mounts a route, as Node's `app.ts` decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    Always,
    /// `MYSTICPARADOX_ADMIN_API` on (`AdminApi.enabled`)
    Admin,
    /// Answers only for an `/__origin/` host containing `gauntlet`; Node's handler calls `next()` for any other
    Gauntlet,
    /// Answered by rustic-rifts when its crates are loaded (RF2); absent otherwise, as with Node's p2p module off.
    /// Launcher Guard and the Guard manifests are isles' (RF1, decision 1), so they are `Always` or `Admin`.
    Rifts,
}

/// Whose route it is (RU3c, seam 7): the game client's, answered per version, or the launcher's, operators' and
/// probes', which are no game version's and are served from the shared code with the module's changelist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Version,
    Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteDef {
    pub service: Service,
    pub method: Method,
    /// As RU3a writes it: an Express path, or a regex literal with its flags
    pub path: &'static str,
    pub group: Group,
    pub side: Side,
    pub when: When,
}

const fn route(service: Service, method: Method, path: &'static str, group: Group, side: Side, when: When) -> RouteDef {
    RouteDef {
        service,
        method,
        path,
        group,
        side,
        when,
    }
}

const NODE_PREFIXES: [&str; 5] = [
    "/launcher/",
    "/admin/",
    "/health/",
    "/internal/v1/metrics",
    "/reforged/api/",
];

impl RouteDef {
    pub fn scope(&self) -> Scope {
        if NODE_PREFIXES.iter().any(|prefix| self.path.starts_with(prefix)) {
            Scope::Node
        } else {
            Scope::Version
        }
    }

    pub fn is_regex(&self) -> bool {
        self.path.starts_with("/^")
    }

    /// `GET /character`, as logs and the 501 message name it.
    pub fn name(&self) -> String {
        format!("{} {}", self.method.as_str(), self.path)
    }
}

/// Every route of both services, in match order.
pub fn all() -> &'static [RouteDef] {
    table::ROUTES
}

/// The module's routes, kept as the seam a game module names (1.14.7's alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSet {
    V1147,
}

impl RouteSet {
    pub fn routes(self, service: Service) -> impl Iterator<Item = &'static RouteDef> {
        all().iter().filter(move |route| route.service == service)
    }
}

/// Path parameters, decoded, in path order (regex captures are named `0`, `1`... as Express names them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params(pub Vec<(String, String)>);

impl Params {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A parameter that isn't valid percent-encoded UTF-8: Express answers 400.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadParam(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Param(String),
}

#[derive(Debug, Clone)]
enum Matcher {
    Segments(Vec<Segment>),
    Regex(Regex),
}

/// A compiled route path.
#[derive(Debug, Clone)]
pub struct Pattern(Matcher);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("route path {path:?}: {reason}")]
pub struct PatternError {
    pub path: String,
    pub reason: String,
}

impl Pattern {
    pub fn parse(path: &str) -> Result<Self, PatternError> {
        let error = |reason: &str| PatternError {
            path: path.to_owned(),
            reason: reason.to_owned(),
        };
        if let Some(literal) = path.strip_prefix('/').filter(|rest| rest.starts_with('^')) {
            let (source, flags) = literal
                .rsplit_once('/')
                .ok_or_else(|| error("a regex literal ends with /"))?;
            if !flags.chars().all(|flag| flag == 'i') {
                return Err(error("only the i flag"));
            }
            let source = if flags.is_empty() {
                source.to_owned()
            } else {
                format!("(?i){source}")
            };
            return Regex::new(&source)
                .map(|regex| Self(Matcher::Regex(regex)))
                .map_err(|e| error(&e.to_string()));
        }
        if !path.starts_with('/') {
            return Err(error("a path starts with /"));
        }
        let trimmed = match path.strip_suffix('/') {
            Some(rest) if !rest.is_empty() => rest,
            _ => path,
        };
        let segments = split(trimmed)
            .map(|segment| match segment.strip_prefix(':') {
                Some("") => Err(error("an unnamed parameter")),
                Some(name) => Ok(Segment::Param(name.to_owned())),
                None if segment.contains(['*', '{', '}', '?', '(', ')']) => Err(error("only literals and :params")),
                None => Ok(Segment::Literal(segment.to_owned())),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self(Matcher::Segments(segments)))
    }

    /// `None` when `path` doesn't match; its parameters, or the one that doesn't decode, when it does.
    pub fn matches(&self, path: &str) -> Option<Result<Params, BadParam>> {
        match &self.0 {
            Matcher::Regex(regex) => {
                let captures = regex.captures(path)?;
                let params = captures
                    .iter()
                    .skip(1)
                    .enumerate()
                    .map(|(i, capture)| decode(i.to_string(), capture.map_or("", |c| c.as_str())))
                    .collect::<Result<Vec<_>, _>>();
                Some(params.map(Params))
            }
            Matcher::Segments(segments) => {
                if !path.starts_with('/') {
                    return None;
                }
                let path = match path.strip_suffix('/') {
                    Some(rest) if !rest.is_empty() => rest,
                    _ => path,
                };
                let parts: Vec<&str> = split(path).collect();
                if parts.len() != segments.len() {
                    return None;
                }
                let mut raw = Vec::new();
                for (segment, part) in segments.iter().zip(&parts) {
                    match segment {
                        Segment::Literal(literal) if literal.eq_ignore_ascii_case(part) => {}
                        Segment::Literal(_) => return None,
                        Segment::Param(_) if part.is_empty() => return None,
                        Segment::Param(name) => raw.push((name.clone(), *part)),
                    }
                }
                Some(
                    raw.into_iter()
                        .map(|(name, value)| decode(name, value))
                        .collect::<Result<Vec<_>, _>>()
                        .map(Params),
                )
            }
        }
    }
}

fn decode(name: String, raw: &str) -> Result<(String, String), BadParam> {
    match percent_decode_str(raw).decode_utf8() {
        Ok(value) => Ok((name, value.into_owned())),
        Err(_) => Err(BadParam(raw.to_owned())),
    }
}

/// The segments after the leading slash, empty ones kept; the root path has none.
fn split(path: &str) -> impl Iterator<Item = &str> {
    let rest = &path[1..];
    rest.split('/').filter(move |_| !rest.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
        Pattern::parse(pattern).unwrap().matches(path).map(|r| r.unwrap().0)
    }

    #[test]
    fn express_paths() {
        assert_eq!(
            params("/account/api/oauth/token", "/Account/API/oauth/TOKEN/"),
            Some(vec![])
        );
        assert_eq!(params("/account/api/oauth/token", "/account/api/oauth/token//"), None);
        assert_eq!(params("/motd/", "/motd"), Some(vec![]));
        assert_eq!(params("/", "/"), Some(vec![]));
        assert_eq!(params("/", "/x"), None);
        assert_eq!(params("/friends/:id", "/friends/"), None);
        assert_eq!(
            params("/loadout/:userId//all", "/loadout/A%20B//ALL"),
            Some(vec![("userId".into(), "A B".into())])
        );
        assert_eq!(params("/loadout/:userId//all", "/loadout/a/all"), None);
        assert_eq!(params("/loadout/:userId/:characterId/all", "/loadout/a//all"), None);
    }

    #[test]
    fn regex_routes_run_as_written() {
        let inventory = r"/^\/inventory\/([^/]*)\/(.+)$/";
        assert_eq!(
            params(inventory, "/inventory//dauntlessrel-1.14.7:647472"),
            Some(vec![
                ("0".into(), "".into()),
                ("1".into(), "dauntlessrel-1.14.7:647472".into())
            ])
        );
        assert_eq!(params(inventory, "/INVENTORY/a/b"), None, "no i flag");
        let leaderboard = r"/^\/leaderboard\/get_leaderboard\//i";
        assert!(params(leaderboard, "/Leaderboard/get_leaderboard/x/y").is_some());
        assert!(params(leaderboard, "/leaderboard/get_leaderboard").is_none());
    }

    #[test]
    fn every_route_compiles() {
        for route in all() {
            Pattern::parse(route.path).unwrap();
        }
    }

    #[test]
    fn a_badly_encoded_parameter_is_refused() {
        let pattern = Pattern::parse("/x/:id").unwrap();
        assert_eq!(pattern.matches("/x/%E2%28"), Some(Err(BadParam("%E2%28".into()))));
    }
}
