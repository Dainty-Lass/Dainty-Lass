//! Routes as data plus one handler per component. Mounting them is isles' (through DF2's `Registrar`): this crate
//! names no web framework, so the gateway rules (Express matching, `JSON.stringify` output) stay in one place.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

use crate::error::{Component, SeamError, Unimplemented};
use crate::id::{AccountId, PeerId, SessionId};

/// A boxed `Send` future, so the traits stay object-safe and isles can swap stubs, the default and the real thing.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the hosted game server and the transport send on every backend call (DD1 keeps the name).
pub const HOST_CAPABILITY_HEADER: &str = "x-mystic-host-capability";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Patch,
    /// Express `router.all`: the realms catch-all.
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

/// Where Node mounts the router (RF1, section 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mount {
    /// `mountEarlyRoutes`, before the shared JSON parser: `p2pHunts.ts` parses its own (72 KiB, strict).
    Early,
    /// `mountRoutes`, or a core router the route lives in today.
    Normal,
    /// After the `/admin` gate, when the admin API is on.
    Admin,
    /// The Director's `/api/matchmaker` router (`ParadoxDirector/src/extensions/p2p.ts`).
    Director,
}

/// One route of RF1's inventory. `handler` is the Node function it is ported from, kept for the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub method: Method,
    pub path: &'static str,
    pub mount: Mount,
    pub handler: &'static str,
}

/// JSON text written as `JSON.stringify` writes it; isles splices it into replies unchanged, so field order holds.
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
    /// 501 in the launcher envelope (`p2pErrors.ts`), what RU4 answers on every route a stub holds.
    pub fn not_implemented(unimplemented: &Unimplemented, request_id: &str) -> Self {
        let body = format!(
            r#"{{"error":{{"code":"NOT_IMPLEMENTED","message":{},"requestId":{}}}}}"#,
            json_string(&unimplemented.to_string()),
            json_string(request_id)
        );
        Self {
            status: 501,
            body: Json(body),
        }
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The request as the hooks see it; isles fills it from its framework's request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestInfo {
    pub path: String,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub remote: Option<IpAddr>,
    pub request_id: String,
}

impl RequestInfo {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// `p2pHostCapabilities.scopes` (`P2PHuntRepository.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityScope {
    HostReady,
    LeaseHeartbeat,
    PeerAdmitted,
    /// The hosted game server's own per-player requests for admitted members; never global game-server authority.
    GameServerRuntime,
    EventsWrite,
    CompleteRequest,
    Abort,
}

/// A host capability that checked out (`hostCapabilities.ts`, `hostRuntimeAuth.ts`): Node's `AuthData` with
/// `IsPlayerHostRuntime` and `RuntimeRosterAccountIds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAuth {
    pub capability_id: String,
    pub session: SessionId,
    pub generation: u64,
    pub host_account: AccountId,
    pub scopes: Vec<CapabilityScope>,
    /// The accounts the runtime may act for.
    pub roster: Vec<AccountId>,
    /// The host's transport peer id (`hostSteamId`), when known on the capability record.
    pub host_peer: Option<PeerId>,
    /// The host's bridge instance id (`bridgeInstanceId`), when known on the capability record.
    pub bridge_instance: Option<String>,
    /// The session id in wire spelling (`sessionId`), when known on the capability record.
    pub wire_session: Option<String>,
}

/// Who is calling, after isles' middleware (`HasLauncherAuth`, `HasAdminAuth`, `RequireHostCapability`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Anonymous,
    Launcher(AccountId),
    /// DH2 gives every admin key an id, recorded as the actor.
    Admin {
        key_id: String,
    },
    HostRuntime(RuntimeAuth),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCall {
    pub route: Route,
    /// Express params in path order (`sessionId`, `admissionId`, `name`...).
    pub params: Vec<(String, String)>,
    pub query: String,
    pub request: RequestInfo,
    pub caller: Caller,
    pub body: Vec<u8>,
}

/// One component's routes. The real handlers parse and answer exactly as Node; stubs refuse with [`Unimplemented`].
pub trait RouteHandler: Send + Sync {
    fn routes(&self) -> &'static [Route];
    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>>;
}

/// A component's routes before its port: each answers [`Unimplemented`] named after its Node handler.
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

    #[test]
    fn not_implemented_uses_the_launcher_envelope() {
        let reply = Reply::not_implemented(&Unimplemented::new(Component::Realms, "realms.join"), "req \"1\"");
        assert_eq!(reply.status, 501);
        assert_eq!(
            reply.body.0,
            concat!(
                r#"{"error":{"code":"NOT_IMPLEMENTED","#,
                r#""message":"rifts-realms: realms.join is not implemented yet","requestId":"req \"1\""}}"#
            )
        );
    }

    #[test]
    fn headers_are_found_by_lower_case_name() {
        let request = RequestInfo {
            headers: vec![(HOST_CAPABILITY_HEADER.to_owned(), "secret".to_owned())],
            ..RequestInfo::default()
        };
        assert_eq!(request.header(HOST_CAPABILITY_HEADER), Some("secret"));
        assert_eq!(request.header("x-other"), None);
    }
}
