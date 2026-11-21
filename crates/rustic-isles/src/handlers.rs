// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! How areas answer rows (core spec §3.6, §5.7): a registry keyed by service, method and the row's exact path text.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use axum::extract::Request;
use axum::response::Response;
use dainty_module_api::BoxFuture;

use crate::pattern::{Params, Pattern, PatternError};
use crate::routes::{self, Group, Method, RouteDef, Service};
use crate::shared::Shared;

/// One matched request, as a handler gets it.
pub struct Call {
    /// The module's shared state.
    pub shared: Arc<Shared>,
    /// The table row it matched.
    pub route: &'static RouteDef,
    /// Path parameters, decoded.
    pub params: Params,
    /// Its URI already stripped of `/__origin/<host>`, every header as received
    pub request: Request,
    /// The host the `/__origin/` prefix named, none when there was none.
    pub origin_host: Option<String>,
    /// The request's id, also on the answer as `x-rustic-isles-request-id`.
    pub request_id: String,
}

/// Answers one matched request.
pub type Handler = Arc<dyn Fn(Call) -> BoxFuture<'static, Response> + Send + Sync>;

/// Why a service's app could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MountError {
    /// A row's path does not compile.
    #[error(transparent)]
    Pattern(#[from] PatternError),
    /// A handler was registered for a row the table does not have.
    #[error("a handler for {service} {method} {path}, which the route table doesn't have")]
    UnknownRoute {
        /// The handler's service.
        service: &'static str,
        /// The handler's method.
        method: &'static str,
        /// The handler's path text.
        path: String,
    },
    /// A `rifts` row has neither an isles handler nor a rifts route.
    #[error("rustic-rifts is loaded but answers no {method} {path}")]
    NoRiftsHandler {
        /// The row's method.
        method: &'static str,
        /// The row's path text.
        path: &'static str,
    },
    /// The auth crate's routes and the table's auth rows disagree.
    #[error("rustic-isles-auth's routes differ from the table's auth rows: {0}")]
    AuthRoutes(String),
}

type Key = (Service, Method, String);

/// The handlers areas registered, by service, method and the row's exact path text.
#[derive(Clone, Default)]
pub struct Handlers(HashMap<Key, Handler>);

impl Handlers {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) the handler of one row.
    pub fn on<F, Fut>(&mut self, service: Service, method: Method, path: &str, handler: F) -> &mut Self
    where
        F: Fn(Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |call| Box::pin(handler(call)));
        self.0.insert((service, method, path.to_owned()), handler);
        self
    }

    /// The row's handler, if one is registered.
    pub fn get(&self, route: &RouteDef) -> Option<&Handler> {
        self.0.get(&(route.service, route.method, route.path.to_owned()))
    }

    /// Whether a handler is registered for the row.
    pub fn contains(&self, service: Service, method: Method, path: &str) -> bool {
        self.0.contains_key(&(service, method, path.to_owned()))
    }

    /// Every other set's handlers over these.
    pub fn extend(&mut self, other: Handlers) {
        self.0.extend(other.0);
    }

    /// Every key names a row, and every row compiles.
    pub fn check(&self) -> Result<(), MountError> {
        for row in routes::all() {
            Pattern::parse(row.path)?;
        }
        let mut keys: Vec<&Key> = self.0.keys().collect();
        keys.sort_by(|a, b| a.2.cmp(&b.2));
        for (service, method, path) in keys {
            let known = routes::all()
                .iter()
                .any(|row| row.service == *service && row.method == *method && row.path == path);
            if !known {
                return Err(MountError::UnknownRoute {
                    service: service.as_str(),
                    method: method.as_str(),
                    path: path.clone(),
                });
            }
        }
        Ok(())
    }
}

fn method_of(method: &http::Method) -> Option<Method> {
    [Method::Get, Method::Post, Method::Put, Method::Patch, Method::Delete]
        .into_iter()
        .find(|m| m.http() == method)
}

/// Each `rustic_isles_auth::routes()` entry as a backend handler calling `Auth::handle`; the pairs must be exactly
/// the table's auth rows.
pub fn auth_handlers() -> Result<Handlers, MountError> {
    let mut handlers = Handlers::new();
    for route in rustic_isles_auth::routes() {
        let method = method_of(&route.method).ok_or_else(|| MountError::AuthRoutes(route.method.to_string()))?;
        let id = route.id;
        handlers.on(Service::Backend, method, route.path, move |call: Call| async move {
            let Call {
                shared,
                params,
                request,
                ..
            } = call;
            shared.auth().handle(id, request, params.into_pairs()).await
        });
    }
    let rows: Vec<&RouteDef> = routes::all().iter().filter(|r| r.group == Group::Auth).collect();
    for row in &rows {
        if !handlers.contains(row.service, row.method, row.path) {
            return Err(MountError::AuthRoutes(format!("no auth route for {}", row.name())));
        }
    }
    if rows.len() != handlers.0.len() {
        return Err(MountError::AuthRoutes(format!(
            "{} auth routes for {} rows",
            handlers.0.len(),
            rows.len()
        )));
    }
    Ok(handlers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[test]
    fn unknown_rows_fail_the_check() {
        let mut handlers = Handlers::new();
        handlers.on(Service::Backend, Method::Get, "/", |_| async { "ok".into_response() });
        assert_eq!(handlers.check(), Ok(()));
        handlers.on(Service::Backend, Method::Get, "/charactr", |_| async {
            "ok".into_response()
        });
        let error = handlers.check().unwrap_err();
        assert_eq!(
            error.to_string(),
            "a handler for backend GET /charactr, which the route table doesn't have"
        );
    }

    #[test]
    fn auth_routes_are_the_auth_rows() {
        let handlers = auth_handlers().unwrap();
        assert_eq!(handlers.0.len(), 49);
        assert_eq!(handlers.check(), Ok(()));
    }
}
