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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Method {
    Get,
    Post,
    Patch,
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
    Early,
    Normal,
    Admin,
    Director,
}

/// A route as data: the module registers it and calls the handler that declared it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Route {
    pub method: Method,
    /// An Express pattern (`/a/:id`).
    pub path: &'static str,
    pub mount: Mount,
    /// Also the operation name an unwritten handler reports.
    pub handler: &'static str,
}

/// JSON text, spliced into replies as is.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Json(pub String);

impl Json {
    pub fn empty_object() -> Self {
        Self(String::from("{}"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: Json,
}

impl Reply {
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
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
        let (_, value) = self.headers.iter().find(|header| header.0 == name)?;
        Some(value)
    }
}

/// What a hosted runtime's capability lets it call.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// Who a route call comes from, as the module authenticated it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Caller {
    Anonymous,
    Launcher(AccountId),
    Admin { key_id: String },
    HostRuntime(RuntimeAuth),
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
#[derive(Clone, Copy, Debug)]
pub struct StubRoutes {
    pub component: Component,
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
