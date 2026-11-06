// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The relay's routes as data, and their dispatch onto an [`IceService`].

use std::sync::Arc;

use rifts_seam::{
    AccountId, BoxFuture, Caller, Component, GlobalId, Json, Method, Mount, NodeId, Refusal, Reply, Route, RouteCall,
    RouteHandler, SeamError, SessionId, StubRoutes, Ulid, Unimplemented,
};
use serde_json::{Map, Value};

use crate::{IceService, RelayError, wire::is_safe_integer};

// Umbrella spec D3: neutral handler ids. They name the operation in a 501 answer.
const UPDATE_COOP_TRANSPORT: &str = "relay.update_coop_transport";
const CREATE_ICE_TRANSPORT_CONFIG: &str = "relay.create_ice_transport_config";
const SUBMIT_ICE_SIGNAL: &str = "relay.submit_ice_signal";

const fn route(method: Method, mount: Mount, path: &'static str, handler: &'static str) -> Route {
    Route {
        method,
        path,
        mount,
        handler,
    }
}

/// The relay's routes: the admin `coop-transport` switch, then the launcher's transport config and ICE signal.
pub const ROUTES: &[Route] = &[
    route(
        Method::Patch,
        Mount::Admin,
        "/admin/v1/operations/coop-transport",
        UPDATE_COOP_TRANSPORT,
    ),
    route(
        Method::Post,
        Mount::Normal,
        "/launcher/v1/p2p/hunts/:sessionId/transport-config",
        CREATE_ICE_TRANSPORT_CONFIG,
    ),
    route(
        Method::Post,
        Mount::Normal,
        "/launcher/v1/p2p/hunts/:sessionId/signal",
        SUBMIT_ICE_SIGNAL,
    ),
];

/// The relay's routes, each answering `Unimplemented`.
pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        routes: ROUTES,
        component: Component::Relay,
    }
}

/// Answers the relay's routes through an [`IceService`].
#[derive(Clone)]
pub struct RelayRoutes {
    service: Arc<dyn IceService>,
    node: NodeId,
}

fn refused(code: &str, message: &str) -> SeamError {
    Refusal {
        code: code.into(),
        message: message.into(),
    }
    .into()
}

/// What a relay error looks like to the router: refusals and 501s pass through, anything else is `INTERNAL`.
fn seam_error(error: RelayError) -> SeamError {
    match error {
        RelayError::Refused(refusal) => refusal.into(),
        RelayError::Unimplemented(missing) => missing.into(),
        other => refused("INTERNAL", &other.to_string()),
    }
}

fn ok(body: Json) -> Reply {
    Reply { status: 200, body }
}

impl RelayRoutes {
    /// Routes for `service`; `node` reads plain UUID session ids as this node's.
    pub fn new(service: Arc<dyn IceService>, node: NodeId) -> Self {
        Self { node, service }
    }

    /// The session from the `sessionId` param: a global id, or a lower-case UUID read as a ULID on this node. Any
    /// other text becomes ULID 0, which no session has.
    fn session(&self, call: &RouteCall) -> SessionId {
        let raw = call
            .params
            .iter()
            .find_map(|(name, value)| (name == "sessionId").then_some(value.as_str()))
            .unwrap_or_default();
        let text = match raw.split_once(':') {
            Some((_, after)) => after,
            None => raw,
        };
        text.parse().unwrap_or_else(|_| {
            SessionId(GlobalId {
                node: self.node.clone(),
                ulid: Ulid(lower_uuid(text).unwrap_or_default()),
            })
        })
    }

    async fn dispatch(&self, call: RouteCall) -> Result<Reply, SeamError> {
        let service = &*self.service;
        match call.route.handler {
            UPDATE_COOP_TRANSPORT => {
                let admin = match &call.caller {
                    Caller::Admin { key_id } => key_id.clone(),
                    _ => String::from("admin"),
                };
                // Not an object is left for the service's validation to refuse.
                let body = serde_json::from_slice(&call.body).unwrap_or_else(|_| Value::Object(Map::new()));
                service.set_coop_transport(admin, body).await.map_err(seam_error)
            }
            CREATE_ICE_TRANSPORT_CONFIG => {
                let (who, hunt, body) = self.launcher_call(&call)?;
                let generation = body.get("generation").and_then(generation_of);
                service
                    .transport_config(who, hunt, generation)
                    .await
                    .map(ok)
                    .map_err(seam_error)
            }
            SUBMIT_ICE_SIGNAL => {
                let (who, hunt, body) = self.launcher_call(&call)?;
                service.submit_signal(who, hunt, body).await.map(ok).map_err(seam_error)
            }
            other => Err(Unimplemented::new(Component::Relay, other).into()),
        }
    }

    /// A signed-in launcher's account, the session and the body, which must be a JSON object (or empty).
    fn launcher_call(&self, call: &RouteCall) -> Result<(AccountId, SessionId, Value), SeamError> {
        let Caller::Launcher(who) = &call.caller else {
            return Err(refused("AUTH_UNAUTHORIZED", "Sign in required."));
        };
        let hunt = self.session(call);
        let body = match call.body.as_slice() {
            [] => Value::Object(Map::new()),
            bytes => match serde_json::from_slice(bytes) {
                Ok(object @ Value::Object(_)) => object,
                _ => return Err(refused("P2P_TICKET_INVALID", "The request body must be a JSON object.")),
            },
        };
        Ok((who.clone(), hunt, body))
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

/// A UUID in lower-case hex with its four dashes, as one number.
fn lower_uuid(text: &str) -> Option<u128> {
    let groups: Vec<&str> = text.split('-').collect();
    if !groups.iter().map(|group| group.len()).eq([8, 4, 4, 4, 12]) {
        return None;
    }
    let digits = groups.concat();
    let lower_hex = digits.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    lower_hex.then(|| u128::from_str_radix(&digits, 16).ok()).flatten()
}

/// Any JSON integer that fits i64; a float only when it has no fraction and is a safe integer.
fn generation_of(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    match (number.as_i64(), number.is_u64()) {
        (Some(integer), _) => Some(integer),
        (None, true) => None,
        // A safe integer, so the cast is exact.
        (None, false) => number
            .as_f64()
            .filter(|&float| is_safe_integer(float))
            .map(|float| float as i64),
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
