// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The answers the module gets while the player-hosted module is off, and the holder that picks them or the real
//! component crates' answers.

use std::sync::Arc;

use crate::{
    error::{Refusal, SeamError},
    hooks::*,
    http::{BoxFuture, Json, Reply, RequestInfo, Route, RouteHandler, RuntimeAuth},
    id::{AccountId, PartyId, RealmId},
    store::{DeletionStep, TableSpec},
};

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

/// The setting that switches the player-hosted module (umbrella spec D3: the Rustic name).
const P2P_MODULE_SETTING: &str = "RUSTIC_RIFTS_P2P_MODULE";
const OFF_WORDS: [&str; 4] = ["0", "false", "off", "no"];

/// On unless the setting says one of the four "no" words, so a missing or mistyped value leaves it on.
pub fn p2p_module_enabled(settings: &dyn Settings) -> bool {
    let Some(value) = settings.get(P2P_MODULE_SETTING).map(str::trim) else {
        return true;
    };
    OFF_WORDS.iter().all(|word| !word.eq_ignore_ascii_case(value))
}

/// An answer that is already there.
fn now<'a, T: Send + 'a>(value: T) -> Fut<'a, T> {
    Box::pin(std::future::ready(Ok(value)))
}

fn realms_disabled() -> SeamError {
    let refusal = Refusal {
        code: "REALMS_DISABLED".into(),
        message: "Realms are not available on this server.".into(),
    };
    refusal.into()
}

/// The module's `forbidden` envelope; `message` is a fixed text with nothing to escape.
fn forbidden(message: &str) -> Reply {
    let body = [
        "{\"code\":\"forbidden\",\"message\":\"",
        message,
        "\",\"payload\":null}",
    ]
    .concat();
    Reply {
        status: 403,
        body: Json(body),
    }
}

/// Dedicated servers only: rifts declines everything and a hosted runtime may change nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct DedicatedOnly;

impl Lifecycle for DedicatedOnly {
    fn start(&self, _: &str) -> Result<(), SeamError> {
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        now(())
    }

    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        Ok(None)
    }

    fn runtime_safety_issues(&self, _: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(vec![])
    }

    fn tables(&self) -> &'static [TableSpec] {
        &[]
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        vec![]
    }
}

impl RealmsHooks for DedicatedOnly {
    // Every request is the module's own.
    fn authenticate(&self, _: RequestInfo) -> Fut<'_, AuthOutcome> {
        now(AuthOutcome::NotMine)
    }

    fn metric_class(&self, _: &str) -> Option<&'static str> {
        None
    }

    fn request_log_tags(&self, _: &RequestInfo) -> String {
        String::new()
    }

    // Matchmaking stays on dedicated servers.
    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        Ok(false)
    }

    fn filters_offline_party_members(&self, _: &str, _: Option<&str>) -> Result<bool, SeamError> {
        Ok(false)
    }

    fn route_hunt(&self, _: HuntRouteContext) -> Fut<'_, HuntRouteOutcome> {
        now(HuntRouteOutcome::NotHandled)
    }

    fn restore_candidate(&self, _: AccountId) -> Fut<'_, Option<Candidate>> {
        now(None)
    }

    fn candidate_status(&self, _: AccountId, _: Candidate) -> Fut<'_, Option<Candidate>> {
        now(None)
    }

    fn cancel_candidate(&self, _: AccountId, _: Candidate) -> Fut<'_, CandidateLeave> {
        now(CandidateLeave::NotHandled)
    }

    fn party_instance(&self, _: PartyId) -> Fut<'_, Option<Candidate>> {
        now(None)
    }

    fn party_member_left(&self, _: AccountId) -> Fut<'_, ()> {
        now(())
    }

    // Nothing to add for operators or the launcher.
    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        Ok(vec![])
    }

    fn operations_policy_fields_valid(&self, _: &Json) -> Result<bool, SeamError> {
        Ok(true)
    }

    fn operations_overview(&self) -> Fut<'_, OperationsOverview> {
        now(OperationsOverview::default())
    }

    fn release_gates(&self) -> Fut<'_, Vec<ReleaseGate>> {
        now(vec![])
    }
}

impl GuardHooks for DedicatedOnly {
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }

    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }

    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(true)
    }

    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), SeamError> {
        Ok(())
    }

    fn census(&self, _: &HostRouteSample) -> Result<(), SeamError> {
        Ok(())
    }

    fn rate_limit_bucket(&self, _: RequestInfo) -> Fut<'_, Option<String>> {
        now(None)
    }

    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        _: HostWriteKind,
        _: AccountId,
        _: HostWriteOptions,
    ) -> Fut<'_, Option<Reply>> {
        now(Some(forbidden("This runtime cannot write player state.")))
    }

    fn record_host_write(&self, _: RuntimeAuth, _: HostWrite) -> Fut<'_, ()> {
        now(())
    }

    fn authorize_inventory(&self, _: RuntimeAuth, _: InventoryTransaction) -> Fut<'_, InventoryDecision> {
        let refusal = forbidden("This runtime cannot mutate inventory.");
        now(InventoryDecision::Refused(refusal))
    }
}

impl RealmsEntry for DedicatedOnly {
    fn authorize_game_session_realm(&self, _: AccountId, realm: Option<Json>) -> Fut<'_, Option<RealmEntry>> {
        // Asking for no realm is the ordinary login; only an actual realm request is refused.
        if realm.is_some() {
            return Box::pin(std::future::ready(Err(realms_disabled())));
        }
        now(None)
    }

    fn bind_realm_presence_at_login(&self, _: AccountId, _: Option<RealmId>) -> Fut<'_, ()> {
        now(())
    }

    fn touch_presence(&self, _: AccountId) -> Fut<'_, ()> {
        now(())
    }

    fn is_member(&self, _: RealmId, _: AccountId) -> Fut<'_, bool> {
        now(false)
    }

    fn is_banned(&self, _: RealmId, _: AccountId) -> Fut<'_, bool> {
        now(false)
    }

    fn hub_room_id(&self, _: &RealmId, _: HubKind) -> Result<String, SeamError> {
        Err(realms_disabled())
    }

    fn find_by_hub_room_id(&self, _: String) -> Fut<'_, Option<RealmId>> {
        now(None)
    }
}

/// The component crates' answers, handed to [`Rifts::new`].
pub struct RiftsParts {
    /// The realms hooks.
    pub realms: Arc<dyn RealmsHooks>,
    /// The game-session entry hooks.
    pub entry: Arc<dyn RealmsEntry>,
    /// Guard's hooks.
    pub guard: Arc<dyn GuardHooks>,
    /// The route handlers, registered in this order.
    pub routes: Vec<Arc<dyn RouteHandler>>,
}

impl RiftsParts {
    fn dedicated() -> Self {
        let answers = Arc::new(DedicatedOnly);
        Self {
            realms: answers.clone(),
            entry: answers.clone(),
            guard: answers,
            routes: vec![],
        }
    }
}

/// What the module holds: the component crates' answers when the module is on, `DedicatedOnly` otherwise.
#[derive(Clone)]
pub struct Rifts {
    parts: Arc<RiftsParts>,
    loaded: bool,
}

impl Rifts {
    /// Holds only the dedicated-server answers, as a server with the module off.
    pub fn dedicated_only() -> Self {
        Self {
            parts: Arc::new(RiftsParts::dedicated()),
            loaded: false,
        }
    }

    /// Holds the component crates' answers.
    pub fn new(parts: RiftsParts) -> Self {
        Self {
            parts: Arc::new(parts),
            loaded: true,
        }
    }

    /// `parts` runs only when the module is on, so a server with it off never builds the component crates' state.
    pub fn load(settings: &dyn Settings, parts: impl FnOnce() -> RiftsParts) -> Self {
        match p2p_module_enabled(settings) {
            true => Self::new(parts()),
            false => Self::dedicated_only(),
        }
    }

    /// Whether the component crates are loaded, rather than the dedicated-server answers.
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// The realms hooks.
    pub fn realms(&self) -> &dyn RealmsHooks {
        &*self.parts.realms
    }

    /// The game-session entry hooks.
    pub fn entry(&self) -> &dyn RealmsEntry {
        &*self.parts.entry
    }

    /// Guard's hooks.
    pub fn guard(&self) -> &dyn GuardHooks {
        &*self.parts.guard
    }

    /// Every handler's routes, handlers in the order given and each one's routes in its own order.
    pub fn routes(&self) -> impl Iterator<Item = (&'static Route, &dyn RouteHandler)> {
        let handlers = self.parts.routes.iter().map(|handler| &**handler);
        handlers.flat_map(|handler| handler.routes().iter().map(move |route| (route, handler)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;
    use crate::{
        error::Component,
        http::{CapabilityScope, Method, Mount, StubRoutes},
        id::SessionId,
        poll_once,
    };

    fn settings(value: Option<&str>) -> HashMap<String, String> {
        let entry = value.map(|v| (P2P_MODULE_SETTING.to_owned(), v.to_owned()));
        HashMap::from_iter(entry)
    }

    #[test]
    fn module_switch() {
        let on = [None, Some("on"), Some(""), Some("1"), Some("yes"), Some("nope")];
        let off = ["0", "false", " OFF ", "No", "FALSE", "\toff\n"].map(Some);
        for (values, enabled) in [(&on[..], true), (&off[..], false)] {
            for value in values {
                assert_eq!(p2p_module_enabled(&settings(*value)), enabled, "{value:?}");
            }
        }
        let tree = BTreeMap::from([(P2P_MODULE_SETTING.to_owned(), "off".to_owned())]);
        assert!(!p2p_module_enabled(&tree));
    }

    #[test]
    fn load_with_the_module_off_never_builds_the_parts() {
        let rifts = Rifts::load(&settings(Some("off")), || panic!("the parts closure ran"));
        assert!(!rifts.loaded());
        assert!(rifts.routes().next().is_none());
        let routed = poll_once(rifts.realms().route_hunt(context()));
        assert_eq!(routed, Some(Ok(HuntRouteOutcome::NotHandled)));
    }

    const fn route(method: Method, path: &'static str, mount: Mount, handler: &'static str) -> Route {
        Route {
            method,
            path,
            mount,
            handler,
        }
    }

    static FIRST: [Route; 2] = [
        route(Method::Get, "/a", Mount::Normal, "a.get"),
        route(Method::Post, "/a/:id", Mount::Early, "a.post"),
    ];
    static SECOND: [Route; 1] = [route(Method::All, "/b", Mount::Admin, "b.all")];

    #[test]
    fn load_with_the_module_on_keeps_handler_and_route_order() {
        let stub = |component: Component, routes: &'static [Route]| -> Arc<dyn RouteHandler> {
            Arc::new(StubRoutes { component, routes })
        };
        let rifts = Rifts::load(&settings(None), || RiftsParts {
            routes: vec![stub(Component::Realms, &FIRST), stub(Component::Guard, &SECOND)],
            ..RiftsParts::dedicated()
        });
        assert!(rifts.loaded());
        let copy = rifts.clone();
        let seen: Vec<(&str, &'static [Route])> = copy.routes().map(|(r, h)| (r.handler, h.routes())).collect();
        assert_eq!(
            seen,
            [("a.get", &FIRST[..]), ("a.post", &FIRST[..]), ("b.all", &SECOND[..])]
        );
        assert!(!Rifts::dedicated_only().loaded());
    }

    fn context() -> HuntRouteContext {
        HuntRouteContext {
            game_mode: "hunt".into(),
            hunt_id: "h1".into(),
            player: AccountId("p".into()),
            party: None,
            party_members: vec![],
            party_revision: None,
            build_id: None,
            is_hub: false,
        }
    }

    fn runtime() -> RuntimeAuth {
        RuntimeAuth {
            capability_id: "cap".into(),
            session: SessionId("01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap()),
            generation: 1,
            host_account: AccountId("host".into()),
            scopes: vec![CapabilityScope::GameServerRuntime],
            roster: vec![],
            host_peer: None,
            bridge_instance: None,
            wire_session: None,
        }
    }

    fn forbidden_body(message: &str) -> String {
        format!(r#"{{"code":"forbidden","message":"{message}","payload":null}}"#)
    }

    #[test]
    fn dedicated_only_answers() {
        let d = DedicatedOnly;
        let account = || AccountId("a".into());
        let realm = || RealmId("01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap());
        let candidate = || Candidate(Json::empty_object());

        // Lifecycle.
        assert_eq!(d.runtime_safety_issues(&settings(None)), Ok(vec![]));
        assert_eq!(d.startup_summary("backend"), Ok(None));
        assert_eq!(d.start("backend"), Ok(()));
        assert_eq!(poll_once(d.stop()), Some(Ok(())));
        assert!(d.tables().is_empty() && d.account_deletion_steps().is_empty());

        // Realms hooks.
        assert_eq!(d.metric_class("/x"), None);
        assert_eq!(d.request_log_tags(&RequestInfo::default()), "");
        assert_eq!(
            poll_once(d.authenticate(RequestInfo::default())),
            Some(Ok(AuthOutcome::NotMine))
        );
        assert_eq!(d.dedicated_hunts_disabled(), Ok(false));
        assert_eq!(d.filters_offline_party_members("hunt", Some("h")), Ok(false));
        assert_eq!(poll_once(d.restore_candidate(account())), Some(Ok(None)));
        assert_eq!(poll_once(d.candidate_status(account(), candidate())), Some(Ok(None)));
        assert_eq!(poll_once(d.party_instance(PartyId(realm().0))), Some(Ok(None)));
        assert_eq!(
            poll_once(d.route_hunt(context())),
            Some(Ok(HuntRouteOutcome::NotHandled))
        );
        let left = poll_once(d.cancel_candidate(account(), candidate()));
        assert_eq!(left, Some(Ok(CandidateLeave::NotHandled)));
        assert_eq!(poll_once(d.party_member_left(account())), Some(Ok(())));
        assert_eq!(d.launcher_policy_fields(), Ok(vec![]));
        assert_eq!(d.operations_policy_fields_valid(&Json::empty_object()), Ok(true));
        assert_eq!(
            poll_once(d.operations_overview()),
            Some(Ok(OperationsOverview::default()))
        );
        assert_eq!(poll_once(d.release_gates()), Some(Ok(vec![])));

        // Guard hooks.
        assert!(d.service_roles().is_empty() && d.guard_manifest_roles().is_empty());
        assert_eq!(d.census(&HostRouteSample::default()), Ok(()));
        assert_eq!(poll_once(d.rate_limit_bucket(RequestInfo::default())), Some(Ok(None)));
        let write = poll_once(d.authorize_host_write(runtime(), HostWriteKind::Xp, account(), Default::default()));
        let refused_write = Reply {
            status: 403,
            body: Json(forbidden_body("This runtime cannot write player state.")),
        };
        assert_eq!(write, Some(Ok(Some(refused_write))));
        let transaction = InventoryTransaction {
            user: account(),
            character_id: "c".into(),
            transaction_id: "t".into(),
            body: None,
        };
        let Some(Ok(InventoryDecision::Refused(reply))) = poll_once(d.authorize_inventory(runtime(), transaction))
        else {
            panic!("dedicated-only did not refuse the inventory change");
        };
        assert_eq!(reply.status, 403);
        assert_eq!(reply.body.0, forbidden_body("This runtime cannot mutate inventory."));
        let summary = HostWrite {
            kind: HostWriteKind::ItemSave,
            target: account(),
            summary: Json::empty_object(),
        };
        assert_eq!(poll_once(d.record_host_write(runtime(), summary)), Some(Ok(())));
        assert_eq!(d.host_policy_refuses(), Ok(true));
        assert_eq!(d.note_would_refuse("xp", "detail"), Ok(()));

        // Realms entry.
        assert_eq!(
            poll_once(d.authorize_game_session_realm(account(), None)),
            Some(Ok(None))
        );
        let asked = poll_once(d.authorize_game_session_realm(account(), Some(Json("\"r\"".into()))));
        assert_eq!(asked, Some(Err(realms_disabled())));
        let disabled = "REALMS_DISABLED: Realms are not available on this server.";
        assert_eq!(realms_disabled().to_string(), disabled);
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
