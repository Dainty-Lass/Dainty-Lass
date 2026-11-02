// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The relay's routes as data, and their dispatch onto an [`IceService`].

use std::sync::Arc;

use rifts_seam::{
    AccountId, BoxFuture, Caller, Component, GlobalId, Method, Mount, NodeId, Refusal, Reply, Route, RouteCall,
    RouteHandler, SeamError, SessionId, StubRoutes, Ulid, Unimplemented,
};
use serde_json::{Map, Value};

use crate::wire::MAX_SAFE_INTEGER;
use crate::{IceService, RelayError};

// Umbrella spec D3: neutral handler ids. They name the operation in a 501 answer.
const UPDATE_COOP_TRANSPORT: &str = "relay.update_coop_transport";
const CREATE_ICE_TRANSPORT_CONFIG: &str = "relay.create_ice_transport_config";
const SUBMIT_ICE_SIGNAL: &str = "relay.submit_ice_signal";

pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/coop-transport",
        mount: Mount::Admin,
        handler: UPDATE_COOP_TRANSPORT,
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/transport-config",
        mount: Mount::Normal,
        handler: CREATE_ICE_TRANSPORT_CONFIG,
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/signal",
        mount: Mount::Normal,
        handler: SUBMIT_ICE_SIGNAL,
    },
];

pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Relay,
        routes: ROUTES,
    }
}

#[derive(Clone)]
pub struct RelayRoutes {
    service: Arc<dyn IceService>,
    node: NodeId,
}

impl RelayRoutes {
    pub fn new(service: Arc<dyn IceService>, node: NodeId) -> Self {
        Self { service, node }
    }

    /// The session from the `sessionId` param: a global id, or a lower-case UUID read as a ULID on this node. Any
    /// other text becomes ULID 0, which no session has.
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

    async fn dispatch(&self, call: RouteCall) -> Result<Reply, SeamError> {
        match call.route.handler {
            UPDATE_COOP_TRANSPORT => {
                let key_id = match &call.caller {
                    Caller::Admin { key_id } => key_id.clone(),
                    _ => "admin".to_owned(),
                };
                let body = serde_json::from_slice::<Value>(&call.body).unwrap_or_else(|_| Value::Object(Map::new()));
                self.service.set_coop_transport(key_id, body).await.map_err(seam_error)
            }
            CREATE_ICE_TRANSPORT_CONFIG => {
                let account = launcher(&call)?;
                let session = self.session(&call);
                let body = object_body(&call)?;
                let generation = body.get("generation").and_then(generation_of);
                let json = self
                    .service
                    .transport_config(account, session, generation)
                    .await
                    .map_err(seam_error)?;
                Ok(Reply {
                    status: 200,
                    body: json,
                })
            }
            SUBMIT_ICE_SIGNAL => {
                let account = launcher(&call)?;
                let session = self.session(&call);
                let body = object_body(&call)?;
                let json = self
                    .service
                    .submit_signal(account, session, body)
                    .await
                    .map_err(seam_error)?;
                Ok(Reply {
                    status: 200,
                    body: json,
                })
            }
            handler => Err(Unimplemented::new(Component::Relay, handler).into()),
        }
    }
}

impl RouteHandler for RelayRoutes {
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

fn uuid_value(text: &str) -> Option<u128> {
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let mut value = 0_u128;
    for (at, byte) in bytes.iter().enumerate() {
        if matches!(at, 8 | 13 | 18 | 23) {
            if *byte != b'-' {
                return None;
            }
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => return None,
        };
        value = (value << 4) | u128::from(digit);
    }
    Some(value)
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

/// Any JSON integer that fits i64; a float only when it has no fraction and is a safe integer.
fn generation_of(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    if number.is_u64() {
        return None;
    }
    let float = number.as_f64()?;
    // Bounded by MAX_SAFE_INTEGER, so the cast is exact.
    #[allow(clippy::cast_possible_truncation)]
    (float.is_finite() && float.fract() == 0.0 && float.abs() <= MAX_SAFE_INTEGER).then_some(float as i64)
}

fn seam_error(error: RelayError) -> SeamError {
    match error {
        RelayError::Unimplemented(unimplemented) => SeamError::Unimplemented(unimplemented),
        RelayError::Refused(refusal) => SeamError::Refused(refusal),
        other => refused("INTERNAL", &other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rifts_seam::{Json, RequestInfo, poll_once};

    use super::*;

    /// Records what reached the service.
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<String>>,
    }

    impl IceService for Recorder {
        fn transport_config(
            &self,
            account: AccountId,
            session: SessionId,
            generation: Option<i64>,
        ) -> BoxFuture<'_, Result<Json, RelayError>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("config {} {session} {generation:?}", account.0));
            Box::pin(std::future::ready(Ok(Json("{\"ok\":1}".to_owned()))))
        }

        fn submit_signal(
            &self,
            account: AccountId,
            session: SessionId,
            body: Value,
        ) -> BoxFuture<'_, Result<Json, RelayError>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("signal {} {session} {body}", account.0));
            Box::pin(std::future::ready(Err(RelayError::Random)))
        }

        fn set_coop_transport(&self, admin_key_id: String, body: Value) -> BoxFuture<'_, Result<Reply, RelayError>> {
            self.calls.lock().unwrap().push(format!("coop {admin_key_id} {body}"));
            Box::pin(std::future::ready(Ok(Reply {
                status: 409,
                body: Json("{}".to_owned()),
            })))
        }
    }

    fn node() -> NodeId {
        NodeId::new("node-a").unwrap()
    }

    fn call(route: usize, caller: Caller, session: &str, body: &str) -> RouteCall {
        RouteCall {
            route: ROUTES[route],
            params: vec![("sessionId".to_owned(), session.to_owned())],
            query: String::new(),
            request: RequestInfo::default(),
            caller,
            body: body.as_bytes().to_vec(),
        }
    }

    fn launcher_caller() -> Caller {
        Caller::Launcher(AccountId("account-0001".to_owned()))
    }

    #[test]
    fn route_table() {
        let expected = [
            (
                Method::Patch,
                "/admin/v1/operations/coop-transport",
                Mount::Admin,
                "relay.update_coop_transport",
            ),
            (
                Method::Post,
                "/launcher/v1/p2p/hunts/:sessionId/transport-config",
                Mount::Normal,
                "relay.create_ice_transport_config",
            ),
            (
                Method::Post,
                "/launcher/v1/p2p/hunts/:sessionId/signal",
                Mount::Normal,
                "relay.submit_ice_signal",
            ),
        ];
        let actual: Vec<_> = ROUTES.iter().map(|r| (r.method, r.path, r.mount, r.handler)).collect();
        assert_eq!(actual, expected);
        let stubs = stub_routes();
        assert_eq!(stubs.routes, ROUTES);
        for (index, route) in ROUTES.iter().enumerate() {
            let answer = poll_once(stubs.handle(call(index, launcher_caller(), "", ""))).unwrap();
            assert_eq!(answer, Err(SeamError::unimplemented(Component::Relay, route.handler)));
        }
        let routes = RelayRoutes::new(Arc::new(Recorder::default()), node());
        assert_eq!(routes.routes(), ROUTES);
    }

    #[test]
    fn dispatch() {
        let recorder = Arc::new(Recorder::default());
        let routes = RelayRoutes::new(Arc::clone(&recorder) as Arc<dyn IceService>, node());
        let run = |call| poll_once(routes.handle(call)).unwrap();

        let global = "01ARZ3NDEKTSV4RRFFQ69G5FAV@node-b";
        let reply = run(call(
            1,
            launcher_caller(),
            &format!("x:{global}"),
            r#"{"generation":3.0}"#,
        ))
        .unwrap();
        assert_eq!(
            reply,
            Reply {
                status: 200,
                body: Json("{\"ok\":1}".to_owned())
            }
        );
        run(call(1, launcher_caller(), "01234567-89ab-4def-8123-456789abcdef", "")).unwrap();
        run(call(
            1,
            launcher_caller(),
            "01234567-89AB-4DEF-8123-456789ABCDEF",
            r#"{"generation":"3"}"#,
        ))
        .unwrap();
        run(call(
            1,
            launcher_caller(),
            "nonsense",
            r#"{"generation":-9223372036854775808}"#,
        ))
        .unwrap();
        run(call(
            1,
            launcher_caller(),
            "nonsense",
            r#"{"generation":9007199254740992.0}"#,
        ))
        .unwrap();

        assert_eq!(
            run(call(1, Caller::Anonymous, global, "")),
            Err(refused("AUTH_UNAUTHORIZED", "Sign in required."))
        );
        assert_eq!(
            run(call(2, launcher_caller(), global, "[1]")),
            Err(refused("P2P_TICKET_INVALID", "The request body must be a JSON object."))
        );
        assert_eq!(
            run(call(2, launcher_caller(), global, r#"{"kind":"CANDIDATE"}"#)),
            Err(refused("INTERNAL", "the system random source failed"))
        );

        let admin = Caller::Admin {
            key_id: "key-7".to_owned(),
        };
        assert_eq!(run(call(0, admin, "", "not json")).unwrap().status, 409);
        run(call(0, launcher_caller(), "", r#"{"reason":"x"}"#)).unwrap();

        let uuid = Ulid(0x0123_4567_89ab_4def_8123_4567_89ab_cdef);
        assert_eq!(
            *recorder.calls.lock().unwrap(),
            [
                format!("config account-0001 {global} Some(3)"),
                format!("config account-0001 {uuid}@node-a None"),
                format!("config account-0001 {}@node-a None", Ulid(0)),
                format!("config account-0001 {}@node-a Some(-9223372036854775808)", Ulid(0)),
                format!("config account-0001 {}@node-a None", Ulid(0)),
                format!("signal account-0001 {global} {{\"kind\":\"CANDIDATE\"}}"),
                "coop key-7 {}".to_owned(),
                "coop admin {\"reason\":\"x\"}".to_owned(),
            ]
        );
    }

    #[test]
    fn unknown_handler_is_unimplemented() {
        let routes = RelayRoutes::new(Arc::new(Recorder::default()), node());
        let mut odd = call(1, launcher_caller(), "", "");
        odd.route.handler = "relay.other";
        assert_eq!(
            poll_once(routes.handle(odd)).unwrap(),
            Err(SeamError::unimplemented(Component::Relay, "relay.other"))
        );
    }
}
