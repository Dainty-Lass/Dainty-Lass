// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The answers the module gets while the player-hosted module is off, and the holder that picks them or the real
//! component crates' answers.

use std::sync::Arc;

use crate::error::{Refusal, SeamError};
use crate::hooks::{
    AuthOutcome, Candidate, CandidateLeave, GuardHooks, HostRouteSample, HostWrite, HostWriteKind, HostWriteOptions,
    HubKind, HuntRouteContext, HuntRouteOutcome, InventoryDecision, InventoryTransaction, Lifecycle,
    OperationsOverview, RealmEntry, RealmsEntry, RealmsHooks, ReleaseGate, Settings,
};
use crate::http::{BoxFuture, Json, Reply, RequestInfo, Route, RouteHandler, RuntimeAuth};
use crate::id::{AccountId, PartyId, RealmId};
use crate::store::{DeletionStep, TableSpec};

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

/// The setting that switches the player-hosted module (umbrella spec D3: the Rustic name).
const P2P_MODULE_SETTING: &str = "RUSTIC_RIFTS_P2P_MODULE";

/// On unless the setting says one of the four "no" words, so a missing or mistyped value leaves it on.
pub fn p2p_module_enabled(settings: &dyn Settings) -> bool {
    let value = settings.get(P2P_MODULE_SETTING).unwrap_or("").trim();
    !["0", "false", "off", "no"]
        .iter()
        .any(|off| value.eq_ignore_ascii_case(off))
}

fn ready<'a, T: Send + 'a>(answer: Result<T, SeamError>) -> Fut<'a, T> {
    Box::pin(std::future::ready(answer))
}

fn realms_disabled() -> SeamError {
    SeamError::Refused(Refusal {
        code: "REALMS_DISABLED".to_owned(),
        message: "Realms are not available on this server.".to_owned(),
    })
}

fn forbidden(message: &str) -> Reply {
    Reply {
        status: 403,
        body: Json(format!(
            r#"{{"code":"forbidden","message":"{message}","payload":null}}"#
        )),
    }
}

/// Dedicated servers only: rifts declines everything and a hosted runtime may change nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct DedicatedOnly;

impl Lifecycle for DedicatedOnly {
    fn runtime_safety_issues(&self, _settings: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(Vec::new())
    }

    fn startup_summary(&self, _role: &str) -> Result<Option<Json>, SeamError> {
        Ok(None)
    }

    fn start(&self, _role: &str) -> Result<(), SeamError> {
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(Ok(()))
    }

    fn tables(&self) -> &'static [TableSpec] {
        &[]
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        Vec::new()
    }
}

impl RealmsHooks for DedicatedOnly {
    fn metric_class(&self, _path: &str) -> Option<&'static str> {
        None
    }

    fn request_log_tags(&self, _request: &RequestInfo) -> String {
        String::new()
    }

    fn authenticate(&self, _request: RequestInfo) -> Fut<'_, AuthOutcome> {
        ready(Ok(AuthOutcome::NotMine))
    }

    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        Ok(false)
    }

    fn filters_offline_party_members(&self, _game_mode: &str, _hunt_id: Option<&str>) -> Result<bool, SeamError> {
        Ok(false)
    }

    fn restore_candidate(&self, _player: AccountId) -> Fut<'_, Option<Candidate>> {
        ready(Ok(None))
    }

    fn candidate_status(&self, _player: AccountId, _current: Candidate) -> Fut<'_, Option<Candidate>> {
        ready(Ok(None))
    }

    fn party_instance(&self, _party: PartyId) -> Fut<'_, Option<Candidate>> {
        ready(Ok(None))
    }

    fn route_hunt(&self, _context: HuntRouteContext) -> Fut<'_, HuntRouteOutcome> {
        ready(Ok(HuntRouteOutcome::NotHandled))
    }

    fn cancel_candidate(&self, _player: AccountId, _current: Candidate) -> Fut<'_, CandidateLeave> {
        ready(Ok(CandidateLeave::NotHandled))
    }

    fn party_member_left(&self, _account: AccountId) -> Fut<'_, ()> {
        ready(Ok(()))
    }

    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        Ok(Vec::new())
    }

    fn operations_policy_fields_valid(&self, _record: &Json) -> Result<bool, SeamError> {
        Ok(true)
    }

    fn operations_overview(&self) -> Fut<'_, OperationsOverview> {
        ready(Ok(OperationsOverview::default()))
    }

    fn release_gates(&self) -> Fut<'_, Vec<ReleaseGate>> {
        ready(Ok(Vec::new()))
    }
}

impl GuardHooks for DedicatedOnly {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }

    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }

    fn census(&self, _sample: &HostRouteSample) -> Result<(), SeamError> {
        Ok(())
    }

    fn rate_limit_bucket(&self, _request: RequestInfo) -> Fut<'_, Option<String>> {
        ready(Ok(None))
    }

    fn authorize_inventory(
        &self,
        _runtime: RuntimeAuth,
        _transaction: InventoryTransaction,
    ) -> Fut<'_, InventoryDecision> {
        ready(Ok(InventoryDecision::Refused(forbidden(
            "This runtime cannot mutate inventory.",
        ))))
    }

    fn authorize_host_write(
        &self,
        _runtime: RuntimeAuth,
        _kind: HostWriteKind,
        _target: AccountId,
        _options: HostWriteOptions,
    ) -> Fut<'_, Option<Reply>> {
        ready(Ok(Some(forbidden("This runtime cannot write player state."))))
    }

    fn record_host_write(&self, _runtime: RuntimeAuth, _write: HostWrite) -> Fut<'_, ()> {
        ready(Ok(()))
    }

    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(true)
    }

    fn note_would_refuse(&self, _what: &str, _detail: &str) -> Result<(), SeamError> {
        Ok(())
    }
}

impl RealmsEntry for DedicatedOnly {
    fn authorize_game_session_realm(&self, _account: AccountId, realm: Option<Json>) -> Fut<'_, Option<RealmEntry>> {
        // Asking for no realm is the ordinary login; only an actual realm request is refused.
        ready(match realm {
            None => Ok(None),
            Some(_) => Err(realms_disabled()),
        })
    }

    fn bind_realm_presence_at_login(&self, _account: AccountId, _realm: Option<RealmId>) -> Fut<'_, ()> {
        ready(Ok(()))
    }

    fn touch_presence(&self, _account: AccountId) -> Fut<'_, ()> {
        ready(Ok(()))
    }

    fn find_by_hub_room_id(&self, _room: String) -> Fut<'_, Option<RealmId>> {
        ready(Ok(None))
    }

    fn is_member(&self, _realm: RealmId, _account: AccountId) -> Fut<'_, bool> {
        ready(Ok(false))
    }

    fn is_banned(&self, _realm: RealmId, _account: AccountId) -> Fut<'_, bool> {
        ready(Ok(false))
    }

    fn hub_room_id(&self, _realm: &RealmId, _kind: HubKind) -> Result<String, SeamError> {
        Err(realms_disabled())
    }
}

/// The component crates' answers, handed to [`Rifts::new`].
pub struct RiftsParts {
    pub realms: Arc<dyn RealmsHooks>,
    pub entry: Arc<dyn RealmsEntry>,
    pub guard: Arc<dyn GuardHooks>,
    pub routes: Vec<Arc<dyn RouteHandler>>,
}

/// What the module holds: the component crates' answers when the module is on, `DedicatedOnly` otherwise.
#[derive(Clone)]
pub struct Rifts {
    realms: Arc<dyn RealmsHooks>,
    entry: Arc<dyn RealmsEntry>,
    guard: Arc<dyn GuardHooks>,
    routes: Vec<Arc<dyn RouteHandler>>,
    loaded: bool,
}

impl Rifts {
    pub fn dedicated_only() -> Self {
        Self {
            realms: Arc::new(DedicatedOnly),
            entry: Arc::new(DedicatedOnly),
            guard: Arc::new(DedicatedOnly),
            routes: Vec::new(),
            loaded: false,
        }
    }

    pub fn new(parts: RiftsParts) -> Self {
        let RiftsParts {
            realms,
            entry,
            guard,
            routes,
        } = parts;
        Self {
            realms,
            entry,
            guard,
            routes,
            loaded: true,
        }
    }

    /// `parts` runs only when the module is on, so a server with it off never builds the component crates' state.
    pub fn load(settings: &dyn Settings, parts: impl FnOnce() -> RiftsParts) -> Self {
        if p2p_module_enabled(settings) {
            Self::new(parts())
        } else {
            Self::dedicated_only()
        }
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn realms(&self) -> &dyn RealmsHooks {
        self.realms.as_ref()
    }

    pub fn entry(&self) -> &dyn RealmsEntry {
        self.entry.as_ref()
    }

    pub fn guard(&self) -> &dyn GuardHooks {
        self.guard.as_ref()
    }

    /// Every handler's routes, handlers in the order given and each one's routes in its own order.
    pub fn routes(&self) -> impl Iterator<Item = (&'static Route, &dyn RouteHandler)> {
        self.routes
            .iter()
            .flat_map(|handler| handler.routes().iter().map(move |route| (route, handler.as_ref())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Component;
    use crate::http::{CapabilityScope, Method, Mount, StubRoutes};
    use crate::id::{GlobalId, SessionId};
    use crate::poll_once;
    use std::collections::{BTreeMap, HashMap};

    fn settings(value: Option<&str>) -> HashMap<String, String> {
        value
            .map(|v| (P2P_MODULE_SETTING.to_owned(), v.to_owned()))
            .into_iter()
            .collect()
    }

    #[test]
    fn module_switch() {
        for on in [None, Some("on"), Some(""), Some("1"), Some("yes"), Some("nope")] {
            assert!(p2p_module_enabled(&settings(on)), "{on:?}");
        }
        for off in ["0", "false", " OFF ", "No", "FALSE", "\toff\n"] {
            assert!(!p2p_module_enabled(&settings(Some(off))), "{off:?}");
        }
        let tree: BTreeMap<String, String> = [(P2P_MODULE_SETTING.to_owned(), "off".to_owned())].into();
        assert!(!p2p_module_enabled(&tree));
    }

    #[test]
    fn load_with_the_module_off_never_builds_the_parts() {
        let rifts = Rifts::load(&settings(Some("off")), || panic!("the parts closure ran"));
        assert!(!rifts.loaded());
        assert_eq!(rifts.routes().count(), 0);
        assert_eq!(
            poll_once(rifts.realms().route_hunt(context())),
            Some(Ok(HuntRouteOutcome::NotHandled))
        );
    }

    static FIRST: [Route; 2] = [
        Route {
            method: Method::Get,
            path: "/a",
            mount: Mount::Normal,
            handler: "a.get",
        },
        Route {
            method: Method::Post,
            path: "/a/:id",
            mount: Mount::Early,
            handler: "a.post",
        },
    ];
    static SECOND: [Route; 1] = [Route {
        method: Method::All,
        path: "/b",
        mount: Mount::Admin,
        handler: "b.all",
    }];

    #[test]
    fn load_with_the_module_on_keeps_handler_and_route_order() {
        let rifts = Rifts::load(&settings(None), || RiftsParts {
            realms: Arc::new(DedicatedOnly),
            entry: Arc::new(DedicatedOnly),
            guard: Arc::new(DedicatedOnly),
            routes: vec![
                Arc::new(StubRoutes {
                    component: Component::Realms,
                    routes: &FIRST,
                }),
                Arc::new(StubRoutes {
                    component: Component::Guard,
                    routes: &SECOND,
                }),
            ],
        });
        assert!(rifts.loaded());
        let copy = rifts.clone();
        let handlers: Vec<&str> = copy.routes().map(|(route, _)| route.handler).collect();
        assert_eq!(handlers, ["a.get", "a.post", "b.all"]);
        let owners: Vec<&'static [Route]> = copy.routes().map(|(_, handler)| handler.routes()).collect();
        assert_eq!(owners, [&FIRST[..], &FIRST[..], &SECOND[..]]);
        assert!(!Rifts::dedicated_only().loaded());
    }

    fn context() -> HuntRouteContext {
        HuntRouteContext {
            game_mode: "hunt".into(),
            hunt_id: "h1".into(),
            player: AccountId("p".into()),
            party: None,
            party_members: Vec::new(),
            party_revision: None,
            build_id: None,
            is_hub: false,
        }
    }

    fn runtime() -> RuntimeAuth {
        let id: GlobalId = "01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap();
        RuntimeAuth {
            capability_id: "cap".into(),
            session: SessionId(id),
            generation: 1,
            host_account: AccountId("host".into()),
            scopes: vec![CapabilityScope::GameServerRuntime],
            roster: Vec::new(),
            host_peer: None,
            bridge_instance: None,
            wire_session: None,
        }
    }

    #[test]
    fn dedicated_only_answers() {
        let d = DedicatedOnly;
        let account = || AccountId("a".into());
        let realm = || RealmId("01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap());

        assert_eq!(d.runtime_safety_issues(&settings(None)), Ok(Vec::new()));
        assert_eq!(d.startup_summary("backend"), Ok(None));
        assert_eq!(d.start("backend"), Ok(()));
        assert_eq!(poll_once(d.stop()), Some(Ok(())));
        assert!(d.tables().is_empty());
        assert!(d.account_deletion_steps().is_empty());

        assert_eq!(d.metric_class("/x"), None);
        assert_eq!(d.request_log_tags(&RequestInfo::default()), "");
        assert_eq!(
            poll_once(d.authenticate(RequestInfo::default())),
            Some(Ok(AuthOutcome::NotMine))
        );
        assert_eq!(d.dedicated_hunts_disabled(), Ok(false));
        assert_eq!(d.filters_offline_party_members("hunt", Some("h")), Ok(false));
        assert_eq!(poll_once(d.restore_candidate(account())), Some(Ok(None)));
        assert_eq!(
            poll_once(d.candidate_status(account(), Candidate(Json::empty_object()))),
            Some(Ok(None))
        );
        assert_eq!(poll_once(d.party_instance(PartyId(realm().0))), Some(Ok(None)));
        assert_eq!(
            poll_once(d.route_hunt(context())),
            Some(Ok(HuntRouteOutcome::NotHandled))
        );
        assert_eq!(
            poll_once(d.cancel_candidate(account(), Candidate(Json::empty_object()))),
            Some(Ok(CandidateLeave::NotHandled))
        );
        assert_eq!(poll_once(d.party_member_left(account())), Some(Ok(())));
        assert_eq!(d.launcher_policy_fields(), Ok(Vec::new()));
        assert_eq!(d.operations_policy_fields_valid(&Json::empty_object()), Ok(true));
        assert_eq!(
            poll_once(d.operations_overview()),
            Some(Ok(OperationsOverview::default()))
        );
        assert_eq!(poll_once(d.release_gates()), Some(Ok(Vec::new())));

        assert!(d.service_roles().is_empty());
        assert!(d.guard_manifest_roles().is_empty());
        assert_eq!(d.census(&HostRouteSample::default()), Ok(()));
        assert_eq!(poll_once(d.rate_limit_bucket(RequestInfo::default())), Some(Ok(None)));
        let write = poll_once(d.authorize_host_write(runtime(), HostWriteKind::Xp, account(), Default::default()));
        assert_eq!(
            write,
            Some(Ok(Some(Reply {
                status: 403,
                body: Json(
                    r#"{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}"#.into()
                ),
            })))
        );
        let transaction = InventoryTransaction {
            user: account(),
            character_id: "c".into(),
            transaction_id: "t".into(),
            body: None,
        };
        match poll_once(d.authorize_inventory(runtime(), transaction)) {
            Some(Ok(InventoryDecision::Refused(reply))) => {
                assert_eq!(reply.status, 403);
                assert_eq!(
                    reply.body.0,
                    r#"{"code":"forbidden","message":"This runtime cannot mutate inventory.","payload":null}"#
                );
            }
            Some(Ok(InventoryDecision::Authorized(_))) => panic!("dedicated-only authorized an inventory change"),
            other => panic!("unexpected answer: {:?}", other.map(|r| r.err())),
        }
        let summary = HostWrite {
            kind: HostWriteKind::ItemSave,
            target: account(),
            summary: Json::empty_object(),
        };
        assert_eq!(poll_once(d.record_host_write(runtime(), summary)), Some(Ok(())));
        assert_eq!(d.host_policy_refuses(), Ok(true));
        assert_eq!(d.note_would_refuse("xp", "detail"), Ok(()));

        assert_eq!(
            poll_once(d.authorize_game_session_realm(account(), None)),
            Some(Ok(None))
        );
        assert_eq!(
            poll_once(d.authorize_game_session_realm(account(), Some(Json("\"r\"".into())))),
            Some(Err(realms_disabled()))
        );
        assert_eq!(
            realms_disabled().to_string(),
            "REALMS_DISABLED: Realms are not available on this server."
        );
        assert_eq!(
            poll_once(d.bind_realm_presence_at_login(account(), Some(realm()))),
            Some(Ok(()))
        );
        assert_eq!(poll_once(d.touch_presence(account())), Some(Ok(())));
        assert_eq!(poll_once(d.find_by_hub_room_id("room".into())), Some(Ok(None)));
        assert_eq!(poll_once(d.is_member(realm(), account())), Some(Ok(false)));
        assert_eq!(poll_once(d.is_banned(realm(), account())), Some(Ok(false)));
        assert_eq!(d.hub_room_id(&realm(), HubKind::Ramsgate), Err(realms_disabled()));
    }
}
