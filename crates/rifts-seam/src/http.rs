// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::{fmt::Write as _, future::Future, net::IpAddr, pin::Pin};

use crate::{
    error::{Component, SeamError, Unimplemented},
    id::{AccountId, PeerId, SessionId},
};

/// Boxed so the hook traits stay object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Carries the hosted runtime's capability token (umbrella spec D3: the Rustic name).
pub const HOST_CAPABILITY_HEADER: &str = "x-rustic-host-capability";

/// The HTTP verb a route answers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `PATCH`.
    Patch,
    /// Every verb, as Express's `app.all`.
    All,
}

impl Method {
    /// Express's name for the verb; `ALL` is `app.all`.
    pub const fn as_str(self) -> &'static str {
        const NAMES: [&str; 4] = ["GET", "POST", "PATCH", "ALL"];
        NAMES[self as usize]
    }
}

/// Where the module mounts a route.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Mount {
    /// Before the module's own middleware.
    Early,
    /// With the module's own routes.
    Normal,
    /// On the admin API, behind the admin key.
    Admin,
    /// On the director.
    Director,
}

/// A route as data: the module registers it and calls the handler that declared it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Route {
    /// The verb it answers.
    pub method: Method,
    /// An Express pattern (`/a/:id`).
    pub path: &'static str,
    /// Where the module mounts it.
    pub mount: Mount,
    /// Also the operation name an unwritten handler reports.
    pub handler: &'static str,
}

/// JSON text, spliced into replies as is.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Json(pub String);

impl Json {
    /// `{}`.
    pub fn empty_object() -> Self {
        Self(String::from("{}"))
    }
}

/// A status and JSON body for the module to send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    /// The HTTP status code.
    pub status: u16,
    /// The response body.
    pub body: Json,
}

impl Reply {
    /// The 501 `NOT_IMPLEMENTED` envelope for an unwritten operation, carrying the request id.
    pub fn not_implemented(unimplemented: &Unimplemented, request_id: &str) -> Self {
        let message = json_string(&unimplemented.to_string());
        let request = json_string(request_id);
        let body = format!(r#"{{"error":{{"code":"NOT_IMPLEMENTED","message":{message},"requestId":{request}}}}}"#);
        Self {
            body: Json(body),
            status: 501,
        }
    }
}

// Unlike rifts-relay's JSON.stringify escaper, backspace and form feed go out as \u escapes here; both are contracts.
fn json_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        let short = match c {
            '"' => Some('"'),
            '\\' => Some('\\'),
            '\n' => Some('n'),
            '\r' => Some('r'),
            '\t' => Some('t'),
            _ => None,
        };
        match short {
            Some(letter) => {
                quoted.push('\\');
                quoted.push(letter);
            }
            None if c.is_ascii_control() && c != '\u{7f}' => {
                let _ = write!(quoted, "\\u{:04x}", u32::from(c));
            }
            None => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// What a hook sees of an incoming request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequestInfo {
    /// The request path, without the query.
    pub path: String,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    /// The client's address, when known.
    pub remote: Option<IpAddr>,
    /// The module's id for the request, echoed in error envelopes.
    pub request_id: String,
}

impl RequestInfo {
    /// The first header named exactly `name`; names are stored lower case, so there is no folding here.
    pub fn header(&self, name: &str) -> Option<&str> {
        let (_, value) = self.headers.iter().find(|header| header.0 == name)?;
        Some(value)
    }
}

/// What a hosted runtime's capability lets it call.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CapabilityScope {
    /// Report that the host is ready.
    HostReady,
    /// Keep the hosting lease alive.
    LeaseHeartbeat,
    /// Report a peer the host admitted.
    PeerAdmitted,
    /// Call the game-server runtime routes, player-state writes included.
    GameServerRuntime,
    /// Submit hunt evidence events.
    EventsWrite,
    /// Ask for the hunt to be completed.
    CompleteRequest,
    /// Abort the hosted session.
    Abort,
}

/// A hosted runtime authenticated by its capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeAuth {
    /// The capability's id, safe to log; never the token.
    pub capability_id: String,
    /// The hosted session the capability belongs to.
    pub session: SessionId,
    /// The session's generation the capability was issued for.
    pub generation: u64,
    /// The player hosting the session.
    pub host_account: AccountId,
    /// What the runtime may call.
    pub scopes: Vec<CapabilityScope>,
    /// The accounts admitted to the session.
    pub roster: Vec<AccountId>,
    /// The host's peer id, when known.
    pub host_peer: Option<PeerId>,
    /// The bridge instance the capability was issued to.
    pub bridge_instance: Option<String>,
    /// The session id as the wire and the runtime spell it.
    pub wire_session: Option<String>,
}

/// Who a route call comes from, as the module authenticated it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Caller {
    /// No credential.
    Anonymous,
    /// A player through the launcher.
    Launcher(AccountId),
    /// An operator with an admin key.
    Admin {
        /// The admin key's id, never the key.
        key_id: String,
    },
    /// A hosted runtime with a capability.
    HostRuntime(RuntimeAuth),
}

/// One call to a declared route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteCall {
    /// The route that matched.
    pub route: Route,
    /// Express params in the order they appear in the path.
    pub params: Vec<(String, String)>,
    /// The raw query string, without the `?`.
    pub query: String,
    /// The request it came in on.
    pub request: RequestInfo,
    /// Who is calling.
    pub caller: Caller,
    /// The raw request body.
    pub body: Vec<u8>,
}

/// A component that declares routes and answers them.
pub trait RouteHandler: Send + Sync {
    /// The routes it declares, in registration order.
    fn routes(&self) -> &'static [Route];
    /// Answers a call to one of its routes.
    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>>;
}

/// Declares routes whose handlers are not written yet: each call answers `Unimplemented` named after its handler.
#[derive(Clone, Copy, Debug)]
pub struct StubRoutes {
    /// The component the routes belong to.
    pub component: Component,
    /// The routes declared.
    pub routes: &'static [Route],
}

impl RouteHandler for StubRoutes {
    fn routes(&self) -> &'static [Route] {
        self.routes
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        let RouteCall { route, .. } = call;
        crate::stub(self.component, route.handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poll_once;

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    #[test]
    fn not_implemented_envelope() {
        let missing = Unimplemented::new(Component::Realms, "realms.join");
        let Reply { status, body } = Reply::not_implemented(&missing, "req \"1\"");
        assert_eq!(status, 501);
        let want = concat!(
            r#"{"error":{"code":"NOT_IMPLEMENTED","#,
            r#""message":"rifts-realms: realms.join is not implemented yet","#,
            r#""requestId":"req \"1\""}}"#
        );
        assert_eq!(body.0, want);
    }

    #[test]
    fn escaper_writes_control_characters_as_unicode_escapes() {
        let escaped = json_string("a\\b/é\u{1}\u{8}\u{c}\n\r\t\u{1f}");
        assert_eq!(escaped, r#""a\\b/é\u0001\u0008\u000c\n\r\t\u001f""#);
        assert_eq!(json_string("\u{7f}"), "\"\u{7f}\"");
    }

    #[test]
    fn header_lookup_is_exact() {
        let with = |headers| RequestInfo {
            headers,
            ..RequestInfo::default()
        };
        let request = with(vec![
            pair("x-other", "no"),
            pair(HOST_CAPABILITY_HEADER, "cap-1"),
            pair(HOST_CAPABILITY_HEADER, "cap-2"),
        ]);
        let lookups = [
            ("x-rustic-host-capability", Some("cap-1")),
            ("X-Rustic-Host-Capability", None),
            ("x-missing", None),
        ];
        for (name, found) in lookups {
            assert_eq!(request.header(name), found, "{name}");
        }
        assert_eq!(with(vec![pair("x-other", "no")]).header(HOST_CAPABILITY_HEADER), None);
    }

    #[test]
    fn methods_and_json() {
        let verbs = [
            (Method::Get, "GET"),
            (Method::Post, "POST"),
            (Method::Patch, "PATCH"),
            (Method::All, "ALL"),
        ];
        for (method, name) in verbs {
            assert_eq!(method.as_str(), name);
        }
        assert_eq!(
            (Json::empty_object().0.as_str(), Json::default().0.as_str()),
            ("{}", "")
        );
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
            params: vec![pair("id", "1")],
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Anonymous,
            body: vec![],
        };
        let answer = poll_once(stub.handle(call));
        assert_eq!(answer, Some(Err(SeamError::unimplemented(Component::Relay, "x.post"))));
    }
}
