//! The 1.14.7 module without realms: Node's `DEDICATED_ONLY` (`extensions/p2p.ts`) and the realms module turned off.

use std::sync::Arc;

use crate::error::{Refusal, SeamError};
use crate::hooks::{
    AuthOutcome, Candidate, CandidateLeave, GuardHooks, HostWrite, HostWriteKind, HostWriteOptions, HubKind,
    HuntRouteContext, HuntRouteOutcome, InventoryDecision, InventoryTransaction, Lifecycle, OperationsOverview,
    RealmEntry, RealmsEntry, RealmsHooks, ReleaseGate, Settings,
};
use crate::http::{BoxFuture, Json, Reply, RequestInfo, Route, RouteHandler, RuntimeAuth};
use crate::id::{AccountId, PartyId, RealmId};
use crate::store::{DeletionStep, TableSpec};

/// `MYSTICPARADOX_P2P_MODULE`: on unless `0`, `false`, `off` or `no` (trimmed, any case).
pub fn p2p_module_enabled(settings: &dyn Settings) -> bool {
    let value = settings.get("MYSTICPARADOX_P2P_MODULE").unwrap_or_default().trim();
    !["0", "false", "off", "no"]
        .iter()
        .any(|off| value.eq_ignore_ascii_case(off))
}

fn ready<T: Send + 'static>(value: T) -> BoxFuture<'static, Result<T, SeamError>> {
    Box::pin(async move { Ok(value) })
}

fn forbidden(message: &str) -> Reply {
    Reply {
        status: 403,
        body: Json(format!(
            r#"{{"code":"forbidden","message":"{message}","payload":null}}"#
        )),
    }
}

fn realms_disabled() -> SeamError {
    SeamError::Refused(Refusal {
        code: "REALMS_DISABLED".to_owned(),
        message: "Realms are not available on this server.".to_owned(),
    })
}

/// Hunts go to the Director, no host-runtime credential, no routes, tables or policy fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct DedicatedOnly;

impl Lifecycle for DedicatedOnly {
    fn runtime_safety_issues(&self, _: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(Vec::new())
    }
    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        Ok(None)
    }
    fn start(&self, _: &str) -> Result<(), SeamError> {
        Ok(())
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(())
    }
    fn tables(&self) -> &'static [TableSpec] {
        &[]
    }
    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        Vec::new()
    }
}

impl RealmsHooks for DedicatedOnly {
    fn metric_class(&self, _: &str) -> Option<&'static str> {
        None
    }
    fn request_log_tags(&self, _: &RequestInfo) -> String {
        String::new()
    }
    fn authenticate(&self, _: RequestInfo) -> BoxFuture<'_, Result<AuthOutcome, SeamError>> {
        ready(AuthOutcome::NotMine)
    }
    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        Ok(false)
    }
    fn filters_offline_party_members(&self, _: &str, _: Option<&str>) -> Result<bool, SeamError> {
        Ok(false)
    }
    fn restore_candidate(&self, _: AccountId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        ready(None)
    }
    fn candidate_status(&self, _: AccountId, _: Candidate) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        ready(None)
    }
    fn party_instance(&self, _: PartyId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        ready(None)
    }
    fn route_hunt(&self, _: HuntRouteContext) -> BoxFuture<'_, Result<HuntRouteOutcome, SeamError>> {
        ready(HuntRouteOutcome::NotHandled)
    }
    fn cancel_candidate(&self, _: AccountId, _: Candidate) -> BoxFuture<'_, Result<CandidateLeave, SeamError>> {
        ready(CandidateLeave::NotHandled)
    }
    fn party_member_left(&self, _: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(())
    }
    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        Ok(Vec::new())
    }
    fn operations_policy_fields_valid(&self, _: &Json) -> Result<bool, SeamError> {
        Ok(true)
    }
    fn operations_overview(&self) -> BoxFuture<'_, Result<OperationsOverview, SeamError>> {
        ready(OperationsOverview::default())
    }
    fn release_gates(&self) -> BoxFuture<'_, Result<Vec<ReleaseGate>, SeamError>> {
        ready(Vec::new())
    }
}

impl GuardHooks for DedicatedOnly {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn census(&self, _: &RequestInfo) -> Result<(), SeamError> {
        Ok(())
    }
    fn rate_limit_bucket(&self, _: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        ready(None)
    }
    fn authorize_inventory(
        &self,
        _: RuntimeAuth,
        _: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        ready(InventoryDecision::Refused(forbidden(
            "This runtime cannot mutate inventory.",
        )))
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        _: HostWriteKind,
        _: AccountId,
        _: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        ready(Some(forbidden("This runtime cannot write player state.")))
    }
    fn record_host_write(&self, _: RuntimeAuth, _: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(())
    }
    // Without the module no host writes exist; strict answers keep it that way
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(true)
    }
    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), SeamError> {
        Ok(())
    }
}

/// `MYSTICPARADOX_REALMS_MODULE` off (`realms/entry.ts`).
impl RealmsEntry for DedicatedOnly {
    fn authorize_game_session_realm(
        &self,
        _: AccountId,
        realm: Option<Json>,
    ) -> BoxFuture<'_, Result<Option<RealmEntry>, SeamError>> {
        Box::pin(async move {
            if realm.is_none() {
                Ok(None)
            } else {
                Err(realms_disabled())
            }
        })
    }
    fn bind_realm_presence_at_login(&self, _: AccountId, _: Option<RealmId>) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(())
    }
    fn touch_presence(&self, _: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(())
    }
    fn find_by_hub_room_id(&self, _: String) -> BoxFuture<'_, Result<Option<RealmId>, SeamError>> {
        ready(None)
    }
    fn is_member(&self, _: RealmId, _: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        ready(false)
    }
    fn is_banned(&self, _: RealmId, _: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        ready(false)
    }
    fn hub_room_id(&self, _: &RealmId, _: HubKind) -> Result<String, SeamError> {
        Err(realms_disabled())
    }
}

/// The component crates' answers, as `isles-1-14-7` assembles them with its realms feature.
pub struct RiftsParts {
    pub realms: Arc<dyn RealmsHooks>,
    pub entry: Arc<dyn RealmsEntry>,
    pub guard: Arc<dyn GuardHooks>,
    pub routes: Vec<Arc<dyn RouteHandler>>,
}

/// What isles holds: `GetP2PExtension()` and the realms exports in one value.
#[derive(Clone)]
pub struct Rifts {
    loaded: bool,
    realms: Arc<dyn RealmsHooks>,
    entry: Arc<dyn RealmsEntry>,
    guard: Arc<dyn GuardHooks>,
    routes: Vec<Arc<dyn RouteHandler>>,
}

impl Rifts {
    pub fn dedicated_only() -> Self {
        Self {
            loaded: false,
            realms: Arc::new(DedicatedOnly),
            entry: Arc::new(DedicatedOnly),
            guard: Arc::new(DedicatedOnly),
            routes: Vec::new(),
        }
    }

    pub fn new(parts: RiftsParts) -> Self {
        Self {
            loaded: true,
            realms: parts.realms,
            entry: parts.entry,
            guard: parts.guard,
            routes: parts.routes,
        }
    }

    /// `LoadP2PExtension`: the parts only when `MYSTICPARADOX_P2P_MODULE` leaves the module on. Node mounts `realms/`
    /// on its own switch even with p2p off; whether that pairing survives (hubs need p2p) is RF5's to settle.
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

    /// Every route with the handler that answers it, in mount order per handler.
    pub fn routes(&self) -> impl Iterator<Item = (&'static Route, &dyn RouteHandler)> {
        self.routes
            .iter()
            .flat_map(|handler| handler.routes().iter().map(move |route| (route, handler.as_ref())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SessionId;
    use crate::poll_once;
    use std::collections::BTreeMap;

    fn settings(module: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("MYSTICPARADOX_P2P_MODULE".to_owned(), module.to_owned())])
    }

    fn runtime() -> RuntimeAuth {
        RuntimeAuth {
            capability_id: "cap".to_owned(),
            session: "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse::<SessionId>().unwrap(),
            generation: 1,
            host_account: AccountId("host".to_owned()),
            scopes: Vec::new(),
            roster: Vec::new(),
        }
    }

    #[test]
    fn module_switch_reads_like_node() {
        assert!(p2p_module_enabled(&BTreeMap::new()));
        assert!(p2p_module_enabled(&settings("on")));
        assert!(p2p_module_enabled(&settings("")));
        for off in ["0", "false", " OFF ", "No"] {
            assert!(!p2p_module_enabled(&settings(off)), "{off}");
        }
    }

    #[test]
    fn switched_off_never_builds_the_parts() {
        let rifts = Rifts::load(&settings("off"), || unreachable!("parts built with the module off"));
        assert!(!rifts.loaded());
        assert_eq!(rifts.routes().count(), 0);
    }

    #[test]
    fn dedicated_only_routes_to_the_director() {
        let rifts = Rifts::dedicated_only();
        let context = HuntRouteContext {
            game_mode: "HUNT".to_owned(),
            hunt_id: "h".to_owned(),
            player: AccountId("p".to_owned()),
            party: None,
            party_members: Vec::new(),
            party_revision: None,
            build_id: None,
            is_hub: false,
        };
        assert_eq!(
            poll_once(rifts.realms().route_hunt(context)).unwrap(),
            Ok(HuntRouteOutcome::NotHandled)
        );
        assert_eq!(rifts.realms().dedicated_hunts_disabled(), Ok(false));
        assert_eq!(
            poll_once(rifts.realms().authenticate(RequestInfo::default())).unwrap(),
            Ok(AuthOutcome::NotMine)
        );
    }

    #[test]
    fn dedicated_only_refuses_host_writes_as_node() {
        let rifts = Rifts::dedicated_only();
        let refusal = poll_once(rifts.guard().authorize_host_write(
            runtime(),
            HostWriteKind::Xp,
            AccountId("p".to_owned()),
            HostWriteOptions::default(),
        ))
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(refusal.status, 403);
        assert_eq!(
            refusal.body.0,
            r#"{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}"#
        );
        let decision = poll_once(rifts.guard().authorize_inventory(
            runtime(),
            InventoryTransaction {
                user: AccountId("p".to_owned()),
                character_id: "c".to_owned(),
                transaction_id: "t".to_owned(),
            },
        ))
        .unwrap()
        .unwrap();
        assert!(matches!(decision, InventoryDecision::Refused(reply) if reply.status == 403));
        assert_eq!(rifts.guard().host_policy_refuses(), Ok(true));
    }

    #[test]
    fn realms_off_refuses_a_realm_play_only() {
        let entry = Rifts::dedicated_only();
        let account = AccountId("p".to_owned());
        assert_eq!(
            poll_once(entry.entry().authorize_game_session_realm(account.clone(), None)).unwrap(),
            Ok(None)
        );
        let refused = poll_once(
            entry
                .entry()
                .authorize_game_session_realm(account, Some(Json::empty_object())),
        )
        .unwrap()
        .unwrap_err();
        assert!(matches!(refused, SeamError::Refused(Refusal { code, .. }) if code == "REALMS_DISABLED"));
    }
}
