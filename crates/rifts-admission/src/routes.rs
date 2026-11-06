// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Admission's routes as data, and their dispatch onto an [`AdmissionService`].

use std::sync::Arc;

use rifts_relay::wire::MAX_SAFE_INTEGER;
use rifts_seam::{
    AccountId, BoxFuture, Caller, CapabilityScope, Component, GlobalId, HOST_CAPABILITY_HEADER, Json, Method, Mount,
    NodeId, Refusal, Reply, Route, RouteCall, RouteHandler, SeamError, SessionId, StubRoutes, Ulid, Unimplemented,
};
use serde_json::{Map, Value};

use crate::service::{ConsumeTicket, HostCapability};
use crate::{AdmissionError, AdmissionService};

// Umbrella spec D3: neutral handler ids. They name the operation in a 501 answer.
const ISSUE_SELF_TICKET: &str = "admission.issue_self_ticket";
const CONSUME_TICKET: &str = "admission.consume_ticket";
const RECORD_PEER_DISCONNECTED: &str = "admission.record_peer_disconnected";
const CREATE_JOIN_BOOTSTRAP: &str = "admission.create_join_bootstrap";
const LIST_HUB_PEERS: &str = "admission.list_hub_peers";

/// Admission's routes: the host runtime's ticket routes and the launcher's join bootstrap and hub peer list.
pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Post,
        path: "/rustic/v1/hunts/:sessionId/tickets/self",
        mount: Mount::Early,
        handler: ISSUE_SELF_TICKET,
    },
    Route {
        method: Method::Post,
        path: "/rustic/v1/hunts/:sessionId/tickets/consume",
        mount: Mount::Early,
        handler: CONSUME_TICKET,
    },
    Route {
        method: Method::Post,
        path: "/rustic/v1/hunts/:sessionId/peers/:admissionId/disconnect",
        mount: Mount::Early,
        handler: RECORD_PEER_DISCONNECTED,
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/join-bootstrap",
        mount: Mount::Normal,
        handler: CREATE_JOIN_BOOTSTRAP,
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/peers",
        mount: Mount::Normal,
        handler: LIST_HUB_PEERS,
    },
];

/// Every admission route under its handler id, for a build that answers them `Unimplemented`.
pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Admission,
        routes: ROUTES,
    }
}

/// Resolves a host capability from the header value and the raw `sessionId` param, for a caller the module did not
/// already authenticate as a host runtime.
pub type CapabilityResolver = Arc<
    dyn Fn(Option<&str>, &str, CapabilityScope) -> BoxFuture<'static, Result<HostCapability, AdmissionError>>
        + Send
        + Sync,
>;

/// Admission's HTTP routes over a service.
#[derive(Clone)]
pub struct AdmissionRoutes {
    service: Arc<dyn AdmissionService>,
    resolver: Option<CapabilityResolver>,
    node: NodeId,
}

impl AdmissionRoutes {
    /// Routes over `service`; host routes resolve their capability with `resolver` when the caller is not yet known.
    pub fn new(service: Arc<dyn AdmissionService>, resolver: Option<CapabilityResolver>, node: NodeId) -> Self {
        Self {
            service,
            resolver,
            node,
        }
    }

    /// A global id, or a lower-case UUID read as a ULID on this node. Any other text becomes ULID 0, which no
    /// session has.
    fn session(&self, call: &RouteCall) -> SessionId {
        let raw = param(call, "sessionId");
        let text = raw.split_once(':').map_or(raw, |(_, rest)| rest);
        if let Ok(session) = text.parse::<SessionId>() {
            return session;
        }
        SessionId(GlobalId {
            ulid: Ulid(uuid_value(text).unwrap_or(0)),
            node: self.node.clone(),
        })
    }

    async fn capability(&self, call: &RouteCall) -> Result<HostCapability, SeamError> {
        if let Caller::HostRuntime(runtime) = &call.caller
            && let Some(capability) = HostCapability::from_runtime(runtime)
        {
            return Ok(capability);
        }
        let Some(resolver) = &self.resolver else {
            return Err(refused("P2P_CAPABILITY_REQUIRED", "Missing host capability header."));
        };
        resolver(
            call.request.header(HOST_CAPABILITY_HEADER),
            param(call, "sessionId"),
            CapabilityScope::PeerAdmitted,
        )
        .await
        .map_err(seam_error)
    }

    async fn dispatch(&self, call: RouteCall) -> Result<Reply, SeamError> {
        let body = match call.route.handler {
            ISSUE_SELF_TICKET => {
                let account = launcher(&call)?;
                let issued = self.service.issue_self_ticket(account, self.session(&call)).await;
                issued.map_err(seam_error)?.to_json()
            }
            CONSUME_TICKET => {
                let capability = self.capability(&call).await?;
                let body = object_body(&call)?;
                let request_id = match call.request.request_id.as_str() {
                    "" => "missing",
                    id => id,
                };
                let request = ConsumeTicket::from_body(&body, request_id).map_err(seam_error)?;
                let consumed = self.service.consume_ticket(capability, request).await;
                consumed.map_err(seam_error)?.to_json()
            }
            RECORD_PEER_DISCONNECTED => {
                let capability = self.capability(&call).await?;
                let admission_id = param(&call, "admissionId").to_owned();
                let disconnected = self
                    .service
                    .record_peer_disconnected(capability, admission_id, None)
                    .await
                    .map_err(seam_error)?;
                format!("{{\"disconnected\":{disconnected}}}")
            }
            CREATE_JOIN_BOOTSTRAP => {
                let account = launcher(&call)?;
                let bootstrap = self.service.create_join_bootstrap(account, self.session(&call)).await;
                bootstrap.map_err(seam_error)?.to_json()
            }
            LIST_HUB_PEERS => {
                let account = launcher(&call)?;
                let session = self.session(&call);
                let body = object_body(&call)?;
                let generation = body.get("generation").and_then(generation_of);
                let peers = self.service.list_hub_peers(account, session, generation).await;
                peers.map_err(seam_error)?.to_json()
            }
            handler => return Err(Unimplemented::new(Component::Admission, handler).into()),
        };
        Ok(Reply {
            status: 200,
            body: Json(body),
        })
    }
}

impl RouteHandler for AdmissionRoutes {
    fn routes(&self) -> &'static [Route] {
        ROUTES
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        Box::pin(self.dispatch(call))
    }
}

fn param<'a>(call: &'a RouteCall, name: &str) -> &'a str {
    call.params
        .iter()
        .find(|(key, _)| key == name)
        .map_or("", |(_, value)| value.as_str())
}

/// The 128-bit value of a dashed UUID in lower-case hex.
fn uuid_value(text: &str) -> Option<u128> {
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    bytes.iter().enumerate().try_fold(0_u128, |value, (at, &byte)| {
        if matches!(at, 8 | 13 | 18 | 23) {
            return (byte == b'-').then_some(value);
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => return None,
        };
        Some((value << 4) | u128::from(digit))
    })
}

fn refused(code: &str, message: &str) -> SeamError {
    SeamError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

fn launcher(call: &RouteCall) -> Result<AccountId, SeamError> {
    match &call.caller {
        Caller::Launcher(account) => Ok(account.clone()),
        _ => Err(refused("AUTH_UNAUTHORIZED", "Sign in required.")),
    }
}

fn object_body(call: &RouteCall) -> Result<Value, SeamError> {
    if call.body.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    match serde_json::from_slice::<Value>(&call.body) {
        Ok(body @ Value::Object(_)) => Ok(body),
        _ => Err(refused("P2P_TICKET_INVALID", "The request body must be a JSON object.")),
    }
}

/// Any JSON integer that fits i64; a float only when whole and within ±(2^53 - 1).
fn generation_of(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    if number.is_u64() {
        return None;
    }
    let float = number.as_f64()?;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "bounded by MAX_SAFE_INTEGER, so the cast is exact"
    )]
    (float.is_finite() && float.fract() == 0.0 && float.abs() <= MAX_SAFE_INTEGER).then_some(float as i64)
}

/// A refusal keeps its code; anything else reaches the client as `INTERNAL` with its text.
fn seam_error(error: AdmissionError) -> SeamError {
    match error {
        AdmissionError::Unimplemented(unimplemented) => SeamError::Unimplemented(unimplemented),
        AdmissionError::Refused(refusal) => SeamError::Refused(refusal),
        other => refused("INTERNAL", &other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rifts_seam::{PeerId, RequestInfo, RuntimeAuth, Secret, UnixMillis, poll_once};

    use super::*;
    use crate::service::{ConsumedTicket, HubPeers, IssuedTicket, JoinBootstrap};

    const UUID: &str = "01234567-89ab-4def-8123-456789abcdef";

    /// Records what reached the service, and answers with fixed values.
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<String>>,
    }

    impl Recorder {
        fn push(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl AdmissionService for Recorder {
        fn issue_self_ticket(
            &self,
            account: AccountId,
            session: SessionId,
        ) -> BoxFuture<'_, Result<IssuedTicket, AdmissionError>> {
            self.push(format!("self {} {session}", account.0));
            Box::pin(std::future::ready(Err(crate::refused(
                "P2P_SESSION_NOT_FOUND",
                "Hunt session not found.",
            ))))
        }

        fn create_join_bootstrap(
            &self,
            account: AccountId,
            session: SessionId,
        ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>> {
            self.push(format!("bootstrap {} {session}", account.0));
            Box::pin(std::future::ready(Ok(JoinBootstrap {
                common: Json("{\"a\":1}".to_owned()),
                ticket: Secret::new("t".to_owned()),
                lobby_id: "L".to_owned(),
            })))
        }

        fn consume_ticket(
            &self,
            capability: HostCapability,
            request: ConsumeTicket,
        ) -> BoxFuture<'_, Result<ConsumedTicket, AdmissionError>> {
            self.push(format!(
                "consume {} {} {}",
                capability.wire_session, request.request_id, request.connection_handle
            ));
            Box::pin(std::future::ready(Ok(ConsumedTicket {
                idempotent: false,
                admission_id: "id".to_owned(),
                account: AccountId("joiner-1".to_owned()),
                peer: PeerId(7),
                wire_session: capability.wire_session,
                generation: 2,
            })))
        }

        fn record_peer_disconnected(
            &self,
            capability: HostCapability,
            admission_id: String,
            disconnected_at: Option<UnixMillis>,
        ) -> BoxFuture<'_, Result<bool, AdmissionError>> {
            self.push(format!(
                "disconnect {} {admission_id} {disconnected_at:?}",
                capability.generation
            ));
            Box::pin(std::future::ready(Ok(true)))
        }

        fn list_hub_peers(
            &self,
            account: AccountId,
            session: SessionId,
            generation: Option<i64>,
        ) -> BoxFuture<'_, Result<HubPeers, AdmissionError>> {
            self.push(format!("peers {} {session} {generation:?}", account.0));
            Box::pin(std::future::ready(Err(AdmissionError::Conflict)))
        }
    }

    fn node() -> NodeId {
        NodeId::new("home").unwrap()
    }

    fn route(handler: &str) -> Route {
        *ROUTES.iter().find(|route| route.handler == handler).unwrap()
    }

    fn call(handler: &str, caller: Caller, session: &str, body: &str) -> RouteCall {
        RouteCall {
            route: route(handler),
            params: vec![
                ("sessionId".to_owned(), session.to_owned()),
                ("admissionId".to_owned(), "abc".to_owned()),
            ],
            query: String::new(),
            request: RequestInfo {
                headers: vec![(HOST_CAPABILITY_HEADER.to_owned(), "cap-1".to_owned())],
                request_id: "req-9".to_owned(),
                ..RequestInfo::default()
            },
            caller,
            body: body.as_bytes().to_vec(),
        }
    }

    fn launcher_caller() -> Caller {
        Caller::Launcher(AccountId("account-1".to_owned()))
    }

    fn runtime(host_peer: Option<PeerId>) -> RuntimeAuth {
        RuntimeAuth {
            capability_id: "c".to_owned(),
            session: SessionId(GlobalId {
                ulid: Ulid(5),
                node: node(),
            }),
            generation: 2,
            host_account: AccountId("host-1".to_owned()),
            scopes: vec![CapabilityScope::PeerAdmitted],
            roster: Vec::new(),
            host_peer,
            bridge_instance: Some("b".repeat(16)),
            wire_session: None,
        }
    }

    fn handle(routes: &AdmissionRoutes, call: RouteCall) -> Result<Reply, SeamError> {
        poll_once(routes.handle(call)).unwrap()
    }

    fn refusal(code: &str, message: &str) -> Result<Reply, SeamError> {
        Err(refused(code, message))
    }

    #[test]
    fn route_table() {
        let table: Vec<(Method, &str, Mount, &str)> = ROUTES
            .iter()
            .map(|route| (route.method, route.path, route.mount, route.handler))
            .collect();
        assert_eq!(
            table,
            [
                (
                    Method::Post,
                    "/rustic/v1/hunts/:sessionId/tickets/self",
                    Mount::Early,
                    "admission.issue_self_ticket"
                ),
                (
                    Method::Post,
                    "/rustic/v1/hunts/:sessionId/tickets/consume",
                    Mount::Early,
                    "admission.consume_ticket"
                ),
                (
                    Method::Post,
                    "/rustic/v1/hunts/:sessionId/peers/:admissionId/disconnect",
                    Mount::Early,
                    "admission.record_peer_disconnected"
                ),
                (
                    Method::Post,
                    "/launcher/v1/p2p/hunts/:sessionId/join-bootstrap",
                    Mount::Normal,
                    "admission.create_join_bootstrap"
                ),
                (
                    Method::Post,
                    "/launcher/v1/p2p/hunts/:sessionId/peers",
                    Mount::Normal,
                    "admission.list_hub_peers"
                ),
            ]
        );
    }

    #[test]
    fn stub_routes_answer_unimplemented() {
        let stubs = stub_routes();
        assert_eq!(stubs.routes(), ROUTES);
        for route in ROUTES {
            let answer = poll_once(stubs.handle(call(route.handler, launcher_caller(), UUID, ""))).unwrap();
            assert_eq!(
                answer,
                Err(SeamError::unimplemented(Component::Admission, route.handler))
            );
        }
        let routes = AdmissionRoutes::new(Arc::new(Recorder::default()), None, node());
        let mut other = call(LIST_HUB_PEERS, launcher_caller(), UUID, "");
        other.route.handler = "admission.unknown";
        assert_eq!(
            handle(&routes, other),
            Err(SeamError::unimplemented(Component::Admission, "admission.unknown"))
        );
    }

    #[test]
    fn session_params() {
        let recorder = Arc::new(Recorder::default());
        let routes = AdmissionRoutes::new(recorder.clone(), None, node());
        let global = format!("{}@far", Ulid(9));
        for session in [
            UUID,
            &format!("hunt:{UUID}"),
            &global,
            "01234567-89AB-4DEF-8123-456789ABCDEF",
            "x",
        ] {
            let _ = handle(&routes, call(ISSUE_SELF_TICKET, launcher_caller(), session, ""));
        }
        let calls = recorder.calls.lock().unwrap().clone();
        let uuid = format!("{}@home", Ulid(0x0123_4567_89ab_4def_8123_4567_89ab_cdef));
        assert_eq!(
            calls,
            [
                format!("self account-1 {uuid}"),
                format!("self account-1 {uuid}"),
                format!("self account-1 {}@far", Ulid(9)),
                format!("self account-1 {}@home", Ulid(0)),
                format!("self account-1 {}@home", Ulid(0)),
            ]
        );
    }

    #[test]
    fn launcher_routes() {
        let recorder = Arc::new(Recorder::default());
        let routes = AdmissionRoutes::new(recorder.clone(), None, node());
        assert_eq!(
            handle(&routes, call(ISSUE_SELF_TICKET, Caller::Anonymous, UUID, "")),
            refusal("AUTH_UNAUTHORIZED", "Sign in required.")
        );
        assert_eq!(
            handle(&routes, call(ISSUE_SELF_TICKET, launcher_caller(), UUID, "")),
            refusal("P2P_SESSION_NOT_FOUND", "Hunt session not found.")
        );
        assert_eq!(
            handle(&routes, call(CREATE_JOIN_BOOTSTRAP, launcher_caller(), UUID, "")),
            Ok(Reply {
                status: 200,
                body: Json("{\"a\":1,\"ticket\":\"t\",\"lobbyId\":\"L\"}".to_owned()),
            })
        );
        assert_eq!(
            handle(&routes, call(LIST_HUB_PEERS, launcher_caller(), UUID, "[1]")),
            refusal("P2P_TICKET_INVALID", "The request body must be a JSON object.")
        );
        for (body, generation) in [
            ("", "None"),
            ("{\"generation\":3}", "Some(3)"),
            ("{\"generation\":-9223372036854775808}", "Some(-9223372036854775808)"),
            ("{\"generation\":4.0}", "Some(4)"),
            ("{\"generation\":4.5}", "None"),
            ("{\"generation\":9007199254740992.0}", "None"),
            ("{\"generation\":18446744073709551615}", "None"),
            ("{\"generation\":\"3\"}", "None"),
        ] {
            assert_eq!(
                handle(&routes, call(LIST_HUB_PEERS, launcher_caller(), UUID, body)),
                refusal("INTERNAL", "the record already exists")
            );
            let last = recorder.calls.lock().unwrap().last().cloned().unwrap();
            assert!(last.ends_with(generation), "{body}: {last}");
        }
    }

    #[test]
    fn host_routes() {
        let recorder = Arc::new(Recorder::default());
        let routes = AdmissionRoutes::new(recorder.clone(), None, node());
        let consume_body = format!(
            "{{\"ticket\":\"t\",\"authenticatedSteamId\":\"7\",\"bridgeInstanceId\":\"{}\",\"connectionHandle\":\"h1\"}}",
            "b".repeat(16)
        );
        // The capability comes first, even before the body.
        assert_eq!(
            handle(&routes, call(CONSUME_TICKET, launcher_caller(), UUID, "not json")),
            refusal("P2P_CAPABILITY_REQUIRED", "Missing host capability header.")
        );
        assert_eq!(
            handle(
                &routes,
                call(CONSUME_TICKET, Caller::HostRuntime(runtime(None)), UUID, &consume_body)
            ),
            refusal("P2P_CAPABILITY_REQUIRED", "Missing host capability header.")
        );
        let host = || Caller::HostRuntime(runtime(Some(PeerId(4))));
        assert_eq!(
            handle(&routes, call(CONSUME_TICKET, host(), UUID, "not json")),
            refusal("P2P_TICKET_INVALID", "The request body must be a JSON object.")
        );
        assert_eq!(
            handle(&routes, call(CONSUME_TICKET, host(), UUID, "{\"ticket\":1}")),
            refusal("P2P_TICKET_INVALID", "The admission request is malformed.")
        );
        let reply = handle(&routes, call(CONSUME_TICKET, host(), UUID, &consume_body)).unwrap();
        assert_eq!(reply.status, 200);
        let wire = Ulid(5).to_string();
        assert_eq!(
            reply.body.0,
            format!(
                "{{\"admitted\":true,\"idempotent\":false,\"admissionId\":\"id\",\"accountId\":\"joiner-1\",\
                 \"steamId\":\"7\",\"sessionId\":\"{wire}\",\"generation\":2}}"
            )
        );
        let mut missing_id = call(CONSUME_TICKET, host(), UUID, &consume_body);
        missing_id.request.request_id = String::new();
        handle(&routes, missing_id).unwrap();

        let reply = handle(&routes, call(RECORD_PEER_DISCONNECTED, host(), UUID, "ignored")).unwrap();
        assert_eq!(reply.body.0, "{\"disconnected\":true}");
        let calls = recorder.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                format!("consume {wire} req-9 h1"),
                format!("consume {wire} missing h1"),
                "disconnect 2 abc None".to_owned(),
            ]
        );
    }

    #[test]
    fn the_resolver_sees_the_header_and_raw_param() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let resolver: CapabilityResolver = Arc::new(move |header, session, scope| {
            log.lock().unwrap().push(format!("{header:?} {session} {scope:?}"));
            let capability = HostCapability {
                session: SessionId(GlobalId {
                    ulid: Ulid(1),
                    node: NodeId::new("home").unwrap(),
                }),
                wire_session: "wire".to_owned(),
                generation: 3,
                host_account: AccountId("host-1".to_owned()),
                host_peer: PeerId(4),
                bridge_instance_id: "b".repeat(16),
            };
            Box::pin(std::future::ready(Ok(capability)))
        });
        let recorder = Arc::new(Recorder::default());
        let routes = AdmissionRoutes::new(recorder.clone(), Some(resolver), node());
        handle(
            &routes,
            call(RECORD_PEER_DISCONNECTED, Caller::Anonymous, "hunt:raw", ""),
        )
        .unwrap();
        assert_eq!(seen.lock().unwrap().clone(), ["Some(\"cap-1\") hunt:raw PeerAdmitted"]);
        assert_eq!(recorder.calls.lock().unwrap().clone(), ["disconnect 3 abc None"]);

        let refusing: CapabilityResolver =
            Arc::new(|_, _, _| Box::pin(std::future::ready(Err(crate::refused("AUTH_UNAUTHORIZED", "No.")))));
        let routes = AdmissionRoutes::new(recorder, Some(refusing), node());
        assert_eq!(
            handle(&routes, call(RECORD_PEER_DISCONNECTED, Caller::Anonymous, UUID, "")),
            refusal("AUTH_UNAUTHORIZED", "No.")
        );
    }
}
