// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! PLACEHOLDER for a Category A file (DF4a). The real `modules.rs` lives in dainty-restricted and moves here
//! unchanged at integration (docs/port/clean-room/gateway.md); the clean-room implementer never saw it. This
//! stand-in is written only from spec-rd27-gateway-front.md section 5 so the crate builds and its tests run.

use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

use http::HeaderMap;
use module_api::manifest::ModuleManifest;
use module_api::registrar::{Registration, RouteSpec};
use module_api::{ModuleError, ModuleId, PortRange};
use protocol::GameVersion;

use crate::config::Endpoint;
use crate::policy::{self, Decision, LimitClass, Upstream};

/// The gateway's own node-link routes, which no module may take.
pub const NODE_LINKS: &str = "/__node";

/// Prefixes the gateway keeps for itself.
pub const RESERVED_PREFIXES: &[&str] = &[NODE_LINKS, "/network"];

/// A group of routes the gateway answers or refuses itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteGroup {
    pub prefix: &'static str,
}

pub const GROUPS: &[RouteGroup] = &[RouteGroup { prefix: NODE_LINKS }];

/// The module the stand-in plays.
pub const STAND_IN: &str = "isles-1-14-7";

/// What the gateway knows of the registry's placements.
pub trait RegistryView: Send + Sync {
    fn placed(&self, module: &ModuleId) -> Option<BTreeMap<String, PortRange>>;
}

/// A manifest's defaults for a stand-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDefaults {
    pub id: &'static str,
    pub name: &'static str,
    pub backend_port: u16,
}

/// The stand-in for the configured game: one TCP port `backend` (61000) and one route `/`.
pub fn stand_in(version: &GameVersion) -> ManifestDefaults {
    let _ = protocol::invite::version_alias(version);
    ManifestDefaults {
        id: STAND_IN,
        name: "Rustic Isles 1.14.7",
        backend_port: 61000,
    }
}

/// A decision and, for a module-bound request, the module's loopback endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routed {
    pub decision: Decision,
    pub endpoint: Option<Endpoint>,
}

#[derive(Debug, Clone)]
struct Module {
    routes: Vec<RouteSpec>,
    ports: Option<BTreeMap<String, PortRange>>,
    running: bool,
    /// The stand-in points at GATEWAY_BACKEND_URL
    fixed: Option<Endpoint>,
}

/// The routes attached modules registered.
#[derive(Debug, Default)]
pub struct ModuleRoutes {
    modules: RwLock<BTreeMap<ModuleId, Module>>,
}

impl ModuleRoutes {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stand-in, as a gateway started without a controller runs it.
    pub fn with_stand_in(version: &GameVersion, backend: &Endpoint) -> Self {
        let routes = Self::new();
        routes.attach_stand_in(version, backend);
        routes
    }

    pub fn attach_stand_in(&self, version: &GameVersion, backend: &Endpoint) {
        let defaults = stand_in(version);
        let Ok(id) = defaults.id.parse::<ModuleId>() else {
            return;
        };
        self.write().insert(
            id,
            Module {
                routes: vec![RouteSpec {
                    prefix: "/".into(),
                    port: "backend".into(),
                    host_only: false,
                    websocket: false,
                }],
                ports: None,
                running: true,
                fixed: Some(backend.clone()),
            },
        );
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<ModuleId, Module>> {
        self.modules.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<ModuleId, Module>> {
        self.modules.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        for route in &registration.routes {
            if RESERVED_PREFIXES
                .iter()
                .any(|p| policy::under(&route.prefix.to_lowercase(), p))
            {
                return Err(ModuleError::new(
                    module_api::ErrorKind::Invalid,
                    format!("{} is the gateway's own", route.prefix),
                ));
            }
        }
        let mut modules = self.write();
        let entry = modules.entry(module.id.clone()).or_insert(Module {
            routes: Vec::new(),
            ports: None,
            running: false,
            fixed: None,
        });
        entry.routes = registration.routes.clone();
        entry.fixed = None;
        Ok(())
    }

    pub fn detach(&self, module: &ModuleId) {
        self.write().remove(module);
    }

    /// Where the controller placed `module`'s ports.
    pub fn placed(&self, module: &ModuleId, ports: BTreeMap<String, PortRange>, running: bool) {
        let mut modules = self.write();
        let entry = modules.entry(module.clone()).or_insert(Module {
            routes: Vec::new(),
            ports: None,
            running,
            fixed: None,
        });
        entry.ports = Some(ports);
        entry.running = running;
    }

    pub fn stopped(&self, module: &ModuleId) {
        if let Some(m) = self.write().get_mut(module) {
            m.running = false;
        }
    }

    fn endpoint(module: &Module, route: &RouteSpec) -> Result<Endpoint, Decision> {
        if let Some(fixed) = &module.fixed {
            return Ok(fixed.clone());
        }
        let Some(ports) = &module.ports else {
            return Err(Decision::reject(503, "unavailable", "no_module", LimitClass::General));
        };
        if !module.running {
            return Err(Decision::reject(
                503,
                "unavailable",
                "module_unavailable",
                LimitClass::General,
            ));
        }
        match ports.get(&route.port) {
            Some(range) => Ok(Endpoint::new("127.0.0.1", range.first)),
            None => Err(Decision::reject(
                502,
                "bad_gateway",
                "upstream_not_loopback",
                LimitClass::General,
            )),
        }
    }

    /// The longest route matching `canonical` among the routes `accept` keeps.
    fn find(&self, canonical: &str, accept: impl Fn(&RouteSpec) -> bool) -> Option<(Module, RouteSpec)> {
        let modules = self.read();
        let mut best: Option<(Module, RouteSpec)> = None;
        for module in modules.values() {
            for route in &module.routes {
                let prefix = route.prefix.to_lowercase();
                let matches = prefix == "/" || policy::under(canonical, prefix.trim_end_matches('/'));
                if matches && accept(route) && best.as_ref().is_none_or(|(_, b)| b.prefix.len() < route.prefix.len()) {
                    best = Some((module.clone(), route.clone()));
                }
            }
        }
        best
    }

    pub fn route(
        &self,
        _version: &GameVersion,
        game: &GameVersion,
        method: &str,
        target: &str,
        headers: &HeaderMap,
        is_upgrade: bool,
    ) -> Routed {
        let decision = policy::classify_for(game, method, target, headers, is_upgrade);
        let Decision::Proxy { upstream, class, .. } = &decision else {
            return Routed {
                decision,
                endpoint: None,
            };
        };
        let (upstream, class) = (*upstream, *class);
        let refuse = |status, error, reason| Routed {
            decision: Decision::reject(status, error, reason, class),
            endpoint: None,
        };
        let Some((path, _)) = policy::split_target(target) else {
            return refuse(400, "bad_request", "target_not_origin_form");
        };
        let Ok(inner) = policy::unwrap_origin(path) else {
            return refuse(400, "bad_path", "bad_origin");
        };
        let raw = policy::canonical_path(path).unwrap_or_default();
        let Some(canonical) = policy::canonical_path(&inner.path) else {
            return refuse(400, "bad_path", "bad_percent_encoding");
        };
        if policy::under(&raw, NODE_LINKS) || policy::under(&canonical, NODE_LINKS) {
            return refuse(403, "forbidden", "node_link");
        }
        let with_class = |d: Decision| match d {
            Decision::Reject {
                status, error, reason, ..
            } => Decision::reject(status, error, reason, class),
            other => other,
        };
        match upstream {
            Upstream::Content => Routed {
                decision,
                endpoint: None,
            },
            Upstream::Chat => {
                let endpoint = self
                    .find(&canonical, |r| r.websocket)
                    .and_then(|(m, r)| Self::endpoint(&m, &r).ok());
                Routed { decision, endpoint }
            }
            Upstream::Backend => {
                let Some((module, route)) = self.find(&canonical, |r| !r.websocket) else {
                    return refuse(404, "not_found", "no_route");
                };
                if route.host_only || policy::HOST_ONLY_PREFIXES.iter().any(|p| policy::under(&canonical, p)) {
                    return refuse(403, "forbidden", "host_only_route");
                }
                match Self::endpoint(&module, &route) {
                    Ok(endpoint) => Routed {
                        decision,
                        endpoint: Some(endpoint),
                    },
                    Err(d) => Routed {
                        decision: with_class(d),
                        endpoint: None,
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stand_in_routes_everything_to_the_backend_but_node_links() {
        let version = protocol::invite::reforged_version();
        let routes = ModuleRoutes::with_stand_in(&version, &Endpoint::new("127.0.0.1", 61000));
        let route = |t: &str| routes.route(&version, &version, "GET", t, &HeaderMap::new(), false);
        assert_eq!(route("/x").endpoint, Some(Endpoint::new("127.0.0.1", 61000)));
        assert!(matches!(
            route("/__node/ping").decision,
            Decision::Reject {
                reason: "node_link",
                ..
            }
        ));
        assert!(matches!(
            route("/__origin/h.example/__node/x").decision,
            Decision::Reject {
                reason: "node_link",
                ..
            }
        ));
        assert_eq!(route("/content/x").endpoint, None);
    }
}
