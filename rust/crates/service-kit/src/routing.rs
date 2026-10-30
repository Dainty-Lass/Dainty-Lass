// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::handler::Handler;
use axum::http::request::Parts;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, Route, on};
use percent_encoding::percent_decode_str;
use tower::{Layer, Service};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Param(String),
}

#[derive(Debug, Clone)]
pub struct Pattern {
    raw: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadParam(pub String);

impl Pattern {
    pub fn parse(path: &str) -> Self {
        assert!(path.starts_with('/'), "route pattern must start with '/': {path}");
        assert!(
            !path.contains(['*', '{', '}', '?', '(', ')']),
            "route pattern contains unsupported wildcard/regex characters: {path}"
        );

        let mut segments = Vec::new();
        let rest = &path[1..];
        if !rest.is_empty() {
            for seg in rest.split('/') {
                assert!(!seg.is_empty(), "route pattern must not contain empty segments: {path}");
                if let Some(param_name) = seg.strip_prefix(':') {
                    assert!(
                        !param_name.is_empty(),
                        "route pattern parameter must have a name: {path}"
                    );
                    segments.push(Segment::Param(param_name.to_owned()));
                } else {
                    segments.push(Segment::Literal(seg.to_owned()));
                }
            }
        }

        Self {
            raw: path.to_owned(),
            segments,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn matches(&self, path: &str) -> Option<Result<Params, BadParam>> {
        if !path.starts_with('/') {
            return None;
        }
        let normalized = if path == "/" {
            "/"
        } else {
            path.strip_suffix('/').unwrap_or(path)
        };
        let rest = &normalized[1..];

        if self.segments.is_empty() {
            return if rest.is_empty() {
                Some(Ok(Params::default()))
            } else {
                None
            };
        }

        if rest.is_empty() {
            return None;
        }

        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != self.segments.len() {
            return None;
        }

        let mut params = Vec::new();
        for (seg, actual) in self.segments.iter().zip(parts) {
            match seg {
                Segment::Literal(expected) => {
                    if !expected.eq_ignore_ascii_case(actual) {
                        return None;
                    }
                }
                Segment::Param(name) => {
                    if actual.is_empty() {
                        return None;
                    }
                    match percent_decode_str(actual).decode_utf8() {
                        Ok(decoded) => params.push((name.clone(), decoded.into_owned())),
                        Err(_) => return Some(Err(BadParam(actual.to_owned()))),
                    }
                }
            }
        }

        Some(Ok(Params(params)))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params(Vec<(String, String)>);

impl Params {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find_map(|(k, v)| (k == name).then_some(v.as_str()))
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Params {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<Self>().cloned().unwrap_or_default())
    }
}

#[derive(Debug, Clone)]
pub struct MatchedPattern(pub Arc<str>);

struct RouteEntry<S> {
    method: Method,
    pattern: Pattern,
    name: Arc<str>,
    router: MethodRouter<S>,
}

pub struct Router<S = ()> {
    routes: Vec<RouteEntry<S>>,
}

impl<S: Clone + Send + Sync + 'static> Default for Router<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Clone + Send + Sync + 'static> Router<S> {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    pub fn route<H, T>(mut self, method: Method, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        let pattern = Pattern::parse(path);
        let name: Arc<str> = Arc::from(pattern.as_str());
        let filter =
            MethodFilter::try_from(method.clone()).unwrap_or_else(|_| panic!("unsupported HTTP method: {method}"));
        let router = on(filter, handler);
        self.routes.push(RouteEntry {
            method,
            pattern,
            name,
            router,
        });
        self
    }

    pub fn get<H: Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self {
        self.route(Method::GET, path, handler)
    }

    pub fn post<H: Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self {
        self.route(Method::POST, path, handler)
    }

    pub fn put<H: Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self {
        self.route(Method::PUT, path, handler)
    }

    pub fn patch<H: Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self {
        self.route(Method::PATCH, path, handler)
    }

    pub fn delete<H: Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self {
        self.route(Method::DELETE, path, handler)
    }

    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request<Body>> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request<Body>>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request<Body>>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request<Body>>>::Future: Send + 'static,
    {
        self.routes = self
            .routes
            .into_iter()
            .map(|entry| RouteEntry {
                method: entry.method,
                pattern: entry.pattern,
                name: entry.name,
                router: entry.router.layer(layer.clone()),
            })
            .collect();
        self
    }

    pub fn merge(mut self, other: Router<S>) -> Self {
        self.routes.extend(other.routes);
        self
    }

    pub fn table(&self) -> Vec<(Method, String)> {
        self.routes
            .iter()
            .map(|r| (r.method.clone(), r.pattern.as_str().to_owned()))
            .collect()
    }

    pub fn into_service(self, state: S, fallback: fn(&Request<Body>) -> Response) -> RouterService {
        let compiled = self
            .routes
            .into_iter()
            .map(|entry| CompiledRoute {
                method: entry.method,
                pattern: entry.pattern,
                name: entry.name,
                router: entry.router.with_state(state.clone()),
            })
            .collect();
        RouterService {
            routes: Arc::new(compiled),
            fallback,
        }
    }
}

struct CompiledRoute {
    method: Method,
    pattern: Pattern,
    name: Arc<str>,
    router: MethodRouter<()>,
}

#[derive(Clone)]
pub struct RouterService {
    routes: Arc<Vec<CompiledRoute>>,
    fallback: fn(&Request<Body>) -> Response,
}

impl Service<Request<Body>> for RouterService {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        let req_method = request.method();
        let req_path = request.uri().path();

        for route in self.routes.iter() {
            let method_ok = route.method == *req_method || (*req_method == Method::HEAD && route.method == Method::GET);
            if !method_ok {
                continue;
            }
            if let Some(matched) = route.pattern.matches(req_path) {
                return match matched {
                    Ok(params) => {
                        request.extensions_mut().insert(params);
                        request.extensions_mut().insert(MatchedPattern(Arc::clone(&route.name)));
                        let mut svc = route.router.clone();
                        Box::pin(async move { Service::call(&mut svc, request).await })
                    }
                    Err(_) => Box::pin(async move { Ok(StatusCode::BAD_REQUEST.into_response()) }),
                };
            }
        }

        let resp = (self.fallback)(&request);
        Box::pin(async move { Ok(resp) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitively_with_trailing_slash_and_params() {
        let pat = Pattern::parse("/Users/:id/Profile");
        let m1 = pat.matches("/users/42%20a/profile/").unwrap().unwrap();
        assert_eq!(m1.get("id"), Some("42 a"));
        assert!(pat.matches("/users/42/profile//").is_none());
        assert_eq!(pat.matches("/users/%FF/profile"), Some(Err(BadParam("%FF".into()))));
    }

    #[test]
    #[should_panic]
    fn rejects_empty_segments() {
        Pattern::parse("/a//b");
    }

    #[test]
    #[should_panic]
    fn rejects_wildcards() {
        Pattern::parse("/a/*");
    }
}
