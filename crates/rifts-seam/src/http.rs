// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fmt::Write as _;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

use crate::error::{Component, SeamError, Unimplemented};
use crate::id::{AccountId, PeerId, SessionId};

/// Boxed so the hook traits stay object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Carries the hosted runtime's capability token (umbrella spec D3: the Rustic name).
pub const HOST_CAPABILITY_HEADER: &str = "x-rustic-host-capability";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Patch,
    All,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Patch => "PATCH",
            Self::All => "ALL",
        }
    }
}

/// Where the module mounts a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mount {
    Early,
    Normal,
    Admin,
    Director,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub method: Method,
    /// An Express pattern (`/a/:id`).
    pub path: &'static str,
    pub mount: Mount,
    /// Also the operation name an unwritten handler reports.
    pub handler: &'static str,
}

/// JSON text, spliced into replies as is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Json(pub String);

impl Json {
    pub fn empty_object() -> Self {
        Self("{}".to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Json,
}

impl Reply {
    pub fn not_implemented(unimplemented: &Unimplemented, request_id: &str) -> Self {
        let mut body = String::from(r#"{"error":{"code":"NOT_IMPLEMENTED","message":"#);
        push_json_string(&mut body, &unimplemented.to_string());
        body.push_str(r#","requestId":"#);
        push_json_string(&mut body, request_id);
        body.push_str("}}");
        Self {
            status: 501,
            body: Json(body),
        }
    }
}

// Unlike rifts-relay's JSON.stringify escaper, backspace and form feed go out as \u escapes here; both are contracts.
fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestInfo {
    pub path: String,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    pub remote: Option<IpAddr>,
    pub request_id: String,
}

impl RequestInfo {
    /// The first header named exactly `name`; names are stored lower case, so there is no folding here.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityScope {
    HostReady,
    LeaseHeartbeat,
    PeerAdmitted,
    GameServerRuntime,
    EventsWrite,
    CompleteRequest,
    Abort,
}

/// A hosted runtime authenticated by its capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAuth {
    pub capability_id: String,
    pub session: SessionId,
    pub generation: u64,
    pub host_account: AccountId,
    pub scopes: Vec<CapabilityScope>,
    pub roster: Vec<AccountId>,
    pub host_peer: Option<PeerId>,
    pub bridge_instance: Option<String>,
    pub wire_session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Anonymous,
    Launcher(AccountId),
    Admin { key_id: String },
    HostRuntime(RuntimeAuth),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCall {
    pub route: Route,
    /// Express params in the order they appear in the path.
    pub params: Vec<(String, String)>,
    pub query: String,
    pub request: RequestInfo,
    pub caller: Caller,
    pub body: Vec<u8>,
}

pub trait RouteHandler: Send + Sync {
    fn routes(&self) -> &'static [Route];
    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>>;
}

/// Declares routes whose handlers are not written yet: each call answers `Unimplemented` named after its handler.
#[derive(Debug, Clone, Copy)]
pub struct StubRoutes {
    pub component: Component,
    pub routes: &'static [Route],
}

impl RouteHandler for StubRoutes {
    fn routes(&self) -> &'static [Route] {
        self.routes
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        crate::stub(self.component, call.route.handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poll_once;

    #[test]
    fn not_implemented_envelope() {
        let reply = Reply::not_implemented(&Unimplemented::new(Component::Realms, "realms.join"), "req \"1\"");
        assert_eq!(reply.status, 501);
        assert_eq!(
            reply.body.0,
            r#"{"error":{"code":"NOT_IMPLEMENTED","message":"rifts-realms: realms.join is not implemented yet","requestId":"req \"1\""}}"#
        );
    }

    #[test]
    fn escaper_writes_control_characters_as_unicode_escapes() {
        let mut out = String::new();
        push_json_string(&mut out, "a\\b/é\u{1}\u{8}\u{c}\n\r\t\u{1f}");
        assert_eq!(out, r#""a\\b/é\u0001\u0008\u000c\n\r\t\u001f""#);
    }

    #[test]
    fn header_lookup_is_exact() {
        let request = RequestInfo {
            headers: vec![
                ("x-other".to_owned(), "no".to_owned()),
                (HOST_CAPABILITY_HEADER.to_owned(), "cap-1".to_owned()),
                (HOST_CAPABILITY_HEADER.to_owned(), "cap-2".to_owned()),
            ],
            ..RequestInfo::default()
        };
        assert_eq!(request.header("x-rustic-host-capability"), Some("cap-1"));
        assert_eq!(request.header("X-Rustic-Host-Capability"), None);
        assert_eq!(request.header("x-missing"), None);
        let other = RequestInfo {
            headers: vec![("x-other".to_owned(), "no".to_owned())],
            ..RequestInfo::default()
        };
        assert_eq!(other.header(HOST_CAPABILITY_HEADER), None);
    }

    #[test]
    fn methods_and_json() {
        let names: Vec<&str> = [Method::Get, Method::Post, Method::Patch, Method::All]
            .map(Method::as_str)
            .to_vec();
        assert_eq!(names, ["GET", "POST", "PATCH", "ALL"]);
        assert_eq!(Json::empty_object().0, "{}");
        assert_eq!(Json::default().0, "");
    }

    #[test]
    fn stub_routes_answer_unimplemented_by_handler() {
        static ROUTES: [Route; 1] = [Route {
            method: Method::Post,
            path: "/x/:id",
            mount: Mount::Normal,
            handler: "x.post",
        }];
        let stub = StubRoutes {
            component: Component::Relay,
            routes: &ROUTES,
        };
        assert_eq!(stub.routes(), &ROUTES);
        let call = RouteCall {
            route: ROUTES[0],
            params: vec![("id".to_owned(), "1".to_owned())],
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Anonymous,
            body: Vec::new(),
        };
        assert_eq!(
            poll_once(stub.handle(call)),
            Some(Err(SeamError::unimplemented(Component::Relay, "x.post")))
        );
    }
}
