// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The gateway as the core plugin that serves module routes (DD3, DF4a): a request goes to the game module
//! its game version names (an invite's `version`, matched by [`ModuleManifest::answers_invite`]), then to
//! the loopback port of the longest route that module registered. The policy in [`crate::policy`] still
//! judges every request first, so a module's routes never widen what reaches loopback.
//!
//! Content and chat are core plugins, not module routes: [`Upstream::Content`] and [`Upstream::Ws`] keep
//! the gateway's own upstreams. A gateway started without a controller attaches a stand-in for its
//! `GATEWAY_GAME` ([`ModuleRoutes::standalone`]), so a server runs exactly as before.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use http::HeaderMap;
use module_api::manifest::{PortDefault, Transport};
use module_api::{
    API_VERSION, CoreCapability, CorePlugin, ErrorKind, ModuleError, ModuleId, ModuleKind, ModuleManifest, PortRange,
    Registrar, Registration,
};
use protocol::GameVersion;
use protocol::invite::version_alias;

use crate::config::{Config, Endpoint, is_loopback_host};
use crate::policy::{
    Decision, HOST_ONLY_PREFIXES, LimitClass, Upstream, canonical_path, check_path, classify_for, split_target, under,
    unwrap_origin,
};

/// A route group the gateway serves itself rather than a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteGroup {
    pub prefix: &'static str,
    pub host_only: bool,
}

/// Links between the nodes of a network (DD16): host-only. A peer reaches them on a connection that offered the
/// node ALPN name and presented a cluster certificate (DN3, `nodelink.rs`), which the gateway serves before any
/// routing. Every request that comes any other way, wrapped in `/__origin` or not, is refused here.
pub const NODE_LINKS: RouteGroup = RouteGroup {
    prefix: "/__node",
    host_only: true,
};

/// The gateway's own route groups.
pub const GROUPS: [RouteGroup; 1] = [NODE_LINKS];

/// Prefixes no module may register under: the content plugin's, the runtime's wrapper (paths are judged
/// unwrapped, so a route there never matches) and the gateway's own groups.
pub const RESERVED_PREFIXES: [&str; 3] = ["/content", "/__origin", NODE_LINKS.prefix];

/// What the gateway needs from the controller's registry (DF3 implements it): where each module listens.
/// Which modules exist comes through [`CorePlugin::attach`] and [`CorePlugin::detach`].
pub trait RegistryView: Send + Sync {
    /// The loopback endpoint of `module`'s TCP port `port`, as the controller assigned it (it moves a clash,
    /// DD4), or `None` while the module is not running. A non-loopback answer is refused per request.
    fn endpoint(&self, module: &ModuleManifest, port: &str) -> Option<Endpoint>;
}

/// The manifest's default ports on 127.0.0.1, for a registry that has moved nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct ManifestDefaults;

impl RegistryView for ManifestDefaults {
    fn endpoint(&self, module: &ModuleManifest, port: &str) -> Option<Endpoint> {
        module
            .port(port)
            .filter(|port| port.transport == Transport::Tcp)
            .map(|port| Endpoint {
                host: "127.0.0.1".into(),
                port: port.range.first,
            })
    }
}

/// The stand-in's one port, at the address `GATEWAY_BACKEND_URL` or `GATEWAY_METAGAME_URL` gives.
#[derive(Debug, Clone)]
struct ConfigPorts {
    port: &'static str,
    endpoint: Endpoint,
}

impl RegistryView for ConfigPorts {
    fn endpoint(&self, _module: &ModuleManifest, port: &str) -> Option<Endpoint> {
        (port == self.port).then(|| self.endpoint.clone())
    }
}

/// The module a gateway without a controller fronts, as the game's module will register itself (RU4,
/// RV1): one TCP port and every path to it.
pub fn stand_in(version: &GameVersion) -> Result<(ModuleManifest, Registration), String> {
    let (id, display_name, port) = ("isles-1-14-7", "Rustic Isles 1.14.7", "backend");
    let manifest = ModuleManifest {
        id: id.parse().map_err(|e| format!("{e}"))?,
        kind: ModuleKind::Game,
        display_name: display_name.into(),
        game_version: version.clone(),
        invite_aliases: version_alias(version).map(str::to_owned).into_iter().collect(),
        api: API_VERSION,
        ports: vec![PortDefault {
            name: port.into(),
            transport: Transport::Tcp,
            range: PortRange::single(61000),
        }],
        capabilities: Default::default(),
        unverified: false,
        client: None,
    };
    let mut registrar = Registrar::new(manifest.clone());
    registrar.route("/", port);
    let registration = registrar.finish().map_err(|e| e.to_string())?;
    Ok((manifest, registration))
}

/// Where one request goes: the policy's decision, and for a request bound for a module, its endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routed {
    pub decision: Decision,
    /// `None` for the content server and the chat listener, which the gateway's config names
    pub endpoint: Option<Endpoint>,
}

#[derive(Debug, Clone)]
struct Route {
    /// Lower case, as `canonical_path` compares
    prefix: String,
    port: String,
}

#[derive(Debug, Clone)]
struct Attached {
    manifest: ModuleManifest,
    public: Vec<Route>,
    websocket: Vec<Route>,
    host_only: Vec<String>,
    stand_in: bool,
}

/// The names an invite's `version` may use for a module: its version and its aliases.
fn invite_names(manifest: &ModuleManifest) -> Vec<String> {
    std::iter::once(manifest.game_version.to_string())
        .chain(manifest.invite_aliases.iter().cloned())
        .collect()
}

/// `under`, with `/` holding every path.
fn covers(canonical: &str, prefix: &str) -> bool {
    prefix == "/" || under(canonical, prefix)
}

fn invalid(message: impl Into<String>) -> ModuleError {
    ModuleError::new(ErrorKind::Invalid, message)
}

fn reject(status: u16, error: &'static str, reason: &'static str, limit_class: LimitClass) -> Routed {
    Routed {
        decision: Decision::Reject {
            status,
            error,
            reason,
            limit_class,
        },
        endpoint: None,
    }
}

/// The modules' routes, keyed by module.
pub struct ModuleRoutes {
    view: Arc<dyn RegistryView>,
    modules: RwLock<BTreeMap<ModuleId, Attached>>,
}

impl std::fmt::Debug for ModuleRoutes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleRoutes")
            .field("modules", &self.read().keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ModuleRoutes {
    /// Routes with nothing attached yet; the controller attaches each module as it loads.
    pub fn new(view: Arc<dyn RegistryView>) -> Self {
        Self {
            view,
            modules: RwLock::default(),
        }
    }

    /// A gateway without a controller: the stand-in for `GATEWAY_GAME` at the configured backend.
    pub fn standalone(config: &Config) -> Result<Self, String> {
        let (manifest, registration) = stand_in(&config.version)?;
        let routes = Self::new(Arc::new(ConfigPorts {
            port: "backend",
            endpoint: config.backend.clone(),
        }));
        routes.attach(&manifest, &registration).map_err(|e| e.to_string())?;
        Ok(routes)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<ModuleId, Attached>> {
        self.modules.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The attached modules, for status and tests.
    pub fn modules(&self) -> Vec<ModuleId> {
        self.read().keys().cloned().collect()
    }

    /// Judges one request for the game `version` names: `game`'s policy first (which also picks the path
    /// rules: only 1.14.7 unwraps `/__origin`), then the gateway's groups, then the module's routes.
    pub fn route(
        &self,
        version: &str,
        game: &GameVersion,
        method: &str,
        raw_url: &str,
        headers: &HeaderMap,
        is_upgrade: bool,
    ) -> Routed {
        let decision = classify_for(game, method, raw_url, headers, is_upgrade);
        let Decision::Proxy {
            upstream, limit_class, ..
        } = decision
        else {
            return Routed {
                decision,
                endpoint: None,
            };
        };
        // A proxied target split, checked and decoded already; the raw path is judged as well as the
        // unwrapped one, so `/__origin/<host>` can't carry a group's path past it
        let Some((path, _)) = split_target(raw_url).filter(|(path, _)| check_path(path)) else {
            return reject(400, "bad_path", "bad_path", limit_class);
        };
        let inner = unwrap_origin(path).and_then(|unwrapped| canonical_path(&unwrapped.path));
        let (Some(raw), Some(inner)) = (canonical_path(path), inner) else {
            return reject(400, "bad_path", "bad_percent_encoding", limit_class);
        };
        for group in GROUPS {
            if group.host_only && (under(&raw, group.prefix) || under(&inner, group.prefix)) {
                return reject(403, "forbidden", "node_link", limit_class);
            }
        }
        if matches!(upstream, Upstream::Content) {
            return Routed {
                decision,
                endpoint: None,
            };
        }
        let modules = self.read();
        if matches!(upstream, Upstream::Ws) {
            let Some(module) = modules
                .values()
                .find(|m| m.manifest.answers_invite(version) && !m.websocket.is_empty())
            else {
                return Routed {
                    decision,
                    endpoint: None,
                };
            };
            let Some(route) = module
                .websocket
                .iter()
                .filter(|route| covers(&inner, &route.prefix))
                .max_by_key(|route| route.prefix.len())
            else {
                return reject(404, "not_found", "no_route", limit_class);
            };
            let Some(endpoint) = self.view.endpoint(&module.manifest, &route.port) else {
                return reject(503, "unavailable", "module_unavailable", limit_class);
            };
            if !is_loopback_host(&endpoint.host) {
                return reject(502, "bad_gateway", "upstream_not_loopback", limit_class);
            }
            return Routed {
                decision,
                endpoint: Some(endpoint),
            };
        }
        let Some(module) = modules.values().find(|m| m.manifest.answers_invite(version)) else {
            return reject(503, "unavailable", "no_module", limit_class);
        };
        // Host-only wins over a longer public route below it
        if module.host_only.iter().any(|prefix| covers(&inner, prefix)) {
            return reject(403, "forbidden", "host_only_route", limit_class);
        }
        let Some(route) = module
            .public
            .iter()
            .filter(|route| covers(&inner, &route.prefix))
            .max_by_key(|route| route.prefix.len())
        else {
            return reject(404, "not_found", "no_route", limit_class);
        };
        let Some(endpoint) = self.view.endpoint(&module.manifest, &route.port) else {
            return reject(503, "unavailable", "module_unavailable", limit_class);
        };
        // The secret header goes with every request: never off this machine
        if !is_loopback_host(&endpoint.host) {
            return reject(502, "bad_gateway", "upstream_not_loopback", limit_class);
        }
        Routed {
            decision,
            endpoint: Some(endpoint),
        }
    }

    fn check(&self, module: &ModuleManifest, registration: &Registration) -> Result<Attached, ModuleError> {
        registration
            .check(module)
            .map_err(|e| invalid(format!("{}: {e}", module.id)))?;
        if module.kind != ModuleKind::Game {
            return Err(ModuleError::unsupported(format!(
                "{}: routes from a plugin (they belong to game modules, DD3)",
                module.id
            )));
        }
        let mut public = Vec::new();
        let mut websocket = Vec::new();
        let mut host_only = Vec::new();
        for spec in &registration.routes {
            let prefix = spec.prefix.to_ascii_lowercase();
            if let Some(reserved) = RESERVED_PREFIXES.iter().find(|reserved| under(&prefix, reserved)) {
                return Err(invalid(format!(
                    "{}: route {} is under {reserved}",
                    module.id, spec.prefix
                )));
            }
            if spec.websocket {
                if websocket.iter().any(|route: &Route| route.prefix == prefix) {
                    return Err(invalid(format!("{}: route {} given twice", module.id, spec.prefix)));
                }
                if let Some(refused) = HOST_ONLY_PREFIXES.iter().find(|refused| under(&prefix, refused)) {
                    return Err(invalid(format!(
                        "{}: public route {} is under {refused}, which the gateway refuses from outside",
                        module.id, spec.prefix
                    )));
                }
                websocket.push(Route {
                    prefix,
                    port: spec.port.clone(),
                });
                continue;
            }
            if public.iter().any(|route: &Route| route.prefix == prefix) || host_only.contains(&prefix) {
                return Err(invalid(format!("{}: route {} given twice", module.id, spec.prefix)));
            }
            if spec.host_only {
                host_only.push(prefix);
                continue;
            }
            if let Some(refused) = HOST_ONLY_PREFIXES.iter().find(|refused| under(&prefix, refused)) {
                return Err(invalid(format!(
                    "{}: public route {} is under {refused}, which the gateway refuses from outside",
                    module.id, spec.prefix
                )));
            }
            public.push(Route {
                prefix,
                port: spec.port.clone(),
            });
        }
        Ok(Attached {
            manifest: module.clone(),
            public,
            websocket,
            host_only,
            stand_in: false,
        })
    }

    /// Attaches a stand-in module for the server's own configured backend until a loaded game module
    /// with the same id or invite version replaces it.
    pub fn attach_stand_in(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        if registration.routes.is_empty() {
            return Ok(());
        }
        let mut attached = self.check(module, registration)?;
        attached.stand_in = true;
        let mut modules = self.modules.write().unwrap_or_else(PoisonError::into_inner);
        modules.insert(module.id.clone(), attached);
        Ok(())
    }
}

impl CorePlugin for ModuleRoutes {
    fn name(&self) -> &'static str {
        "gateway"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::Routes]
    }

    /// Refuses a registration that fails its checks, a plugin's routes, a route under a reserved or
    /// host-only prefix, and a module an invite couldn't tell from one already attached (replacing any
    /// stand-in for that game version).
    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        if registration.routes.is_empty() {
            return Ok(());
        }
        let attached = self.check(module, registration)?;
        let mut modules = self.modules.write().unwrap_or_else(PoisonError::into_inner);
        if modules.get(&module.id).is_some_and(|existing| !existing.stand_in) {
            return Err(invalid(format!("{} is already attached", module.id)));
        }
        let names = invite_names(module);
        if let Some(other) = modules
            .values()
            .find(|other| !other.stand_in && names.iter().any(|name| other.manifest.answers_invite(name)))
        {
            return Err(invalid(format!(
                "{} answers the same invite version as {} (DD4)",
                module.id, other.manifest.id
            )));
        }
        modules.retain(|_, existing| {
            !(existing.stand_in && names.iter().any(|name| existing.manifest.answers_invite(name)))
        });
        modules.insert(module.id.clone(), attached);
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        self.modules
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(module);
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;
    use module_api::GameModule;
    use module_api::stub::StubModule;
    use protocol::invite::reforged_version;

    use super::*;

    fn none() -> HeaderMap {
        HeaderMap::new()
    }

    fn stub() -> (ModuleManifest, Registration) {
        let stub = StubModule::new();
        let mut registrar = Registrar::new(stub.manifest());
        stub.register(&mut registrar).unwrap();
        (stub.manifest(), registrar.finish().unwrap())
    }

    fn with_stub() -> ModuleRoutes {
        let routes = ModuleRoutes::new(Arc::new(ManifestDefaults));
        let (manifest, registration) = stub();
        routes.attach(&manifest, &registration).unwrap();
        routes
    }

    fn reason(routed: &Routed) -> Option<&'static str> {
        match routed.decision {
            Decision::Reject { reason, .. } => Some(reason),
            Decision::Proxy { .. } | Decision::Network { .. } => None,
        }
    }

    fn port(routed: &Routed) -> Option<u16> {
        routed.endpoint.as_ref().map(|endpoint| endpoint.port)
    }

    fn get(routes: &ModuleRoutes, version: &str, target: &str) -> Routed {
        routes.route(version, &reforged_version(), "GET", target, &none(), false)
    }

    fn standalone(game: &GameVersion) -> ModuleRoutes {
        let mut config = crate::config::Config::from_env(&service_kit::env::Env::from_pairs(vec![
            ("GATEWAY_TLS", "0"),
            ("GATEWAY_BIND", "127.0.0.1"),
            ("GATEWAY_SECRET", "0123456789abcdef0123456789abcdef"),
            ("GATEWAY_ALLOWLIST", "0"),
            ("GATEWAY_BACKEND_URL", "http://127.0.0.1:7001"),
        ]))
        .unwrap();
        config.version = game.clone();
        ModuleRoutes::standalone(&config).unwrap()
    }

    #[test]
    fn the_stand_ins_are_what_the_modules_will_register() {
        let game = reforged_version();
        let (manifest, registration) = stand_in(&game).unwrap();
        manifest.validate().unwrap();
        let alias = version_alias(&game).unwrap();
        assert!(manifest.answers_invite(&game.to_string()) && manifest.answers_invite(alias));
        assert_eq!(registration.routes.len(), 1);
    }

    #[test]
    fn a_standalone_gateway_sends_the_game_to_its_backend_as_before() {
        let routes = standalone(&reforged_version());
        for target in [
            "/",
            "/party/invites",
            "/__origin/gamedev-prod.steelyard.ca/party",
            "/reforged/api/ServerStatus",
        ] {
            let routed = get(&routes, "1.14.7", target);
            assert_eq!(port(&routed), Some(7001), "{target}");
            assert!(
                matches!(
                    routed.decision,
                    Decision::Proxy {
                        upstream: Upstream::Backend,
                        ..
                    }
                ),
                "{target}"
            );
        }
        // Content and chat stay the gateway's own
        assert_eq!(get(&routes, "1.14.7", "/content/v1/manifest").endpoint, None);
        let ws = {
            let mut headers = HeaderMap::new();
            headers.insert("upgrade", HeaderValue::from_static("websocket"));
            headers
        };
        let chat = routes.route("1.14.7", &reforged_version(), "GET", "//", &ws, true);
        assert_eq!((reason(&chat), chat.endpoint), (None, None));
        // The policy's refusals come first and are unchanged
        assert_eq!(
            reason(&get(&routes, "1.14.7", "/reforged/api/CreateInvite")),
            Some("admin_route")
        );
        assert_eq!(reason(&get(&routes, "1.14.7", "/admin/v1")), Some("host_only_route"));
        // The gateway answers the network's routes itself (DN7), with or without a module
        let nodes = get(&routes, "9.9", "/network/v1/nodes");
        assert!(matches!(nodes.decision, Decision::Network { .. }) && nodes.endpoint.is_none());
        // The alias names the same module; another version none
        assert_eq!(port(&get(&routes, "reforged", "/party")), Some(7001));
        assert_eq!(reason(&get(&routes, "1.4.4", "/party")), Some("no_module"));
    }

    #[test]
    fn a_module_gets_its_routes_and_never_its_host_only_ones() {
        let routes = with_stub();
        for version in ["0.0.1", "stub"] {
            assert_eq!(port(&get(&routes, version, "/stub/x")), Some(65000), "{version}");
        }
        for target in [
            "/stub/admin",
            "/STUB/Admin/x/",
            "//stub//admin",
            "/stub/%61dmin",
            "/__origin/gamedev-prod.steelyard.ca/stub/admin",
        ] {
            let routed = get(&routes, "0.0.1", target);
            assert_eq!(reason(&routed), Some("host_only_route"), "{target}");
            assert_eq!(routed.endpoint, None, "{target}");
        }
        assert_eq!(reason(&get(&routes, "0.0.1", "/other")), Some("no_route"));
        assert_eq!(reason(&get(&routes, "0.0.1", "/stubx")), Some("no_route"));
        assert_eq!(reason(&get(&routes, "1.14.7", "/stub")), Some("no_module"));
        routes.detach(&"stub".parse().unwrap());
        assert_eq!(reason(&get(&routes, "0.0.1", "/stub/x")), Some("no_module"));
    }

    #[test]
    fn host_only_wins_over_a_longer_public_route() {
        let routes = ModuleRoutes::new(Arc::new(ManifestDefaults));
        let (manifest, _) = stub();
        let mut registrar = Registrar::new(manifest.clone());
        registrar
            .host_only_route("/a", "http")
            .route("/a/b", "http")
            .route("/", "http");
        routes.attach(&manifest, &registrar.finish().unwrap()).unwrap();
        assert_eq!(reason(&get(&routes, "0.0.1", "/a/b/c")), Some("host_only_route"));
        assert_eq!(port(&get(&routes, "0.0.1", "/ab")), Some(65000));
    }

    #[test]
    fn the_node_link_group_refuses_everything() {
        let routes = with_stub();
        let ws = {
            let mut headers = HeaderMap::new();
            headers.insert("upgrade", HeaderValue::from_static("websocket"));
            headers
        };
        for target in [
            "/__node",
            "/__NODE/peer/hello",
            "//__node//x/",
            "/__origin/gamedev-prod.steelyard.ca/__node/x",
        ] {
            assert_eq!(reason(&get(&routes, "0.0.1", target)), Some("node_link"), "{target}");
            let upgrade = routes.route("0.0.1", &reforged_version(), "GET", target, &ws, true);
            assert_eq!(reason(&upgrade), Some("node_link"), "{target}");
        }
        assert_eq!(reason(&get(&routes, "0.0.1", "/__nodes")), Some("no_route"));
    }

    #[test]
    fn a_view_off_loopback_or_without_the_port_is_refused() {
        struct Off;
        impl RegistryView for Off {
            fn endpoint(&self, _: &ModuleManifest, port: &str) -> Option<Endpoint> {
                (port == "http").then(|| Endpoint {
                    host: "192.0.2.1".into(),
                    port: 80,
                })
            }
        }
        struct Stopped;
        impl RegistryView for Stopped {
            fn endpoint(&self, _: &ModuleManifest, _: &str) -> Option<Endpoint> {
                None
            }
        }
        let (manifest, registration) = stub();
        let off = ModuleRoutes::new(Arc::new(Off));
        off.attach(&manifest, &registration).unwrap();
        assert_eq!(reason(&get(&off, "0.0.1", "/stub")), Some("upstream_not_loopback"));
        let stopped = ModuleRoutes::new(Arc::new(Stopped));
        stopped.attach(&manifest, &registration).unwrap();
        assert_eq!(reason(&get(&stopped, "0.0.1", "/stub")), Some("module_unavailable"));
    }

    #[test]
    fn bad_attachments_are_refused() {
        let (manifest, registration) = stub();
        let routes = with_stub();
        // Twice, or another module an invite couldn't tell apart (DD4)
        assert!(routes.attach(&manifest, &registration).is_err());
        let mut twin = manifest.clone();
        twin.id = "twin".parse().unwrap();
        twin.game_version = "9.9".parse().unwrap();
        assert!(routes.attach(&twin, &registration).is_err(), "alias taken");
        twin.invite_aliases = vec!["0.0.1".into()];
        assert!(routes.attach(&twin, &registration).is_err(), "version as alias");
        twin.invite_aliases.clear();
        routes.attach(&twin, &registration).unwrap();

        let fresh = || ModuleRoutes::new(Arc::new(ManifestDefaults));
        let mut plugin = manifest.clone();
        plugin.kind = ModuleKind::Plugin;
        assert_eq!(
            fresh().attach(&plugin, &registration).unwrap_err().kind,
            ErrorKind::Unsupported
        );
        let mut forged = registration.clone();
        forged.routes[0].port = "game".into();
        assert!(fresh().attach(&manifest, &forged).is_err(), "unchecked registration");
        for (prefix, host_only) in [
            ("/content", false),
            ("/Content/x", true),
            ("/__node", true),
            ("/__origin/x", false),
            ("/admin/x", false),
            ("/Health", false),
        ] {
            let mut registrar = Registrar::new(manifest.clone());
            if host_only {
                registrar.host_only_route(prefix, "http");
            } else {
                registrar.route(prefix, "http");
            }
            let refused = registrar.finish().unwrap();
            assert!(fresh().attach(&manifest, &refused).is_err(), "{prefix}");
        }
        let mut registrar = Registrar::new(manifest.clone());
        registrar.route("/A", "http").host_only_route("/a", "http");
        assert!(fresh().attach(&manifest, &registrar.finish().unwrap()).is_err(), "case");
        // A host-only route under a host-only prefix only repeats the gateway's rule
        let mut registrar = Registrar::new(manifest.clone());
        registrar.host_only_route("/admin/stub", "http");
        assert!(fresh().attach(&manifest, &registrar.finish().unwrap()).is_ok());
        // Nothing to route, nothing kept
        let empty = fresh();
        empty.attach(&manifest, &Registration::default()).unwrap();
        assert!(empty.modules().is_empty());
    }

    #[test]
    fn websocket_routes_route_upgrades_to_a_modules_port() {
        let mut manifest = stub().0;
        manifest.ports.push(PortDefault {
            name: "chat".into(),
            transport: Transport::Tcp,
            range: PortRange::single(61099),
        });
        let mut registrar = Registrar::new(manifest.clone());
        registrar.route("/", "http").websocket_route("/ws", "chat");
        let routes = ModuleRoutes::new(Arc::new(ManifestDefaults));
        routes.attach(&manifest, &registrar.finish().unwrap()).unwrap();
        let ws = {
            let mut headers = HeaderMap::new();
            headers.insert("upgrade", HeaderValue::from_static("websocket"));
            headers
        };
        let upgraded = routes.route("0.0.1", &manifest.game_version, "GET", "/ws", &ws, true);
        assert_eq!(port(&upgraded), Some(61099));
        let missed = routes.route("0.0.1", &manifest.game_version, "GET", "/other", &ws, true);
        assert_eq!(reason(&missed), Some("no_route"));
    }
}
