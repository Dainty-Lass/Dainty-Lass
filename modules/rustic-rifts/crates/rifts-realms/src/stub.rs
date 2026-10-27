//! Every operation answers [`rifts_seam::Unimplemented`] until RF5 (RD18).

use rifts_seam::{
    AccountId, AuthOutcome, BoxFuture, Candidate, CandidateLeave, Component, DeletionStep, HubKind, HuntRouteContext,
    HuntRouteOutcome, Json, Lifecycle, OperationsOverview, PartyId, RealmEntry, RealmId, RealmsEntry, RealmsHooks,
    ReleaseGate, RequestInfo, RuntimeAuth, SeamError, SessionId, Settings, TableSpec, stub,
};

use crate::RealmsError;
use crate::TABLES;
use crate::hosting::{HostBootstrap, HuntLifecycle, HuntStatus, Observation, StartHub};

#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Realms;

fn refuse<T>(operation: &'static str) -> Result<T, SeamError> {
    Err(SeamError::unimplemented(COMPONENT, operation))
}

impl Lifecycle for Stub {
    fn runtime_safety_issues(&self, _: &dyn Settings) -> Result<Vec<String>, SeamError> {
        refuse("runtime_safety_issues")
    }
    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        refuse("startup_summary")
    }
    fn start(&self, _: &str) -> Result<(), SeamError> {
        refuse("start")
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        stub(COMPONENT, "stop")
    }
    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }
    // A stub stores nothing, so it has nothing to delete
    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        Vec::new()
    }
}

impl RealmsHooks for Stub {
    fn metric_class(&self, _: &str) -> Option<&'static str> {
        None
    }
    fn request_log_tags(&self, _: &RequestInfo) -> String {
        String::new()
    }
    fn authenticate(&self, _: RequestInfo) -> BoxFuture<'_, Result<AuthOutcome, SeamError>> {
        stub(COMPONENT, "authenticate")
    }
    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        refuse("dedicated_hunts_disabled")
    }
    fn filters_offline_party_members(&self, _: &str, _: Option<&str>) -> Result<bool, SeamError> {
        refuse("filters_offline_party_members")
    }
    fn restore_candidate(&self, _: AccountId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        stub(COMPONENT, "restore_candidate")
    }
    fn candidate_status(&self, _: AccountId, _: Candidate) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        stub(COMPONENT, "candidate_status")
    }
    fn party_instance(&self, _: PartyId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        stub(COMPONENT, "party_instance")
    }
    fn route_hunt(&self, _: HuntRouteContext) -> BoxFuture<'_, Result<HuntRouteOutcome, SeamError>> {
        stub(COMPONENT, "route_hunt")
    }
    fn cancel_candidate(&self, _: AccountId, _: Candidate) -> BoxFuture<'_, Result<CandidateLeave, SeamError>> {
        stub(COMPONENT, "cancel_candidate")
    }
    fn party_member_left(&self, _: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        stub(COMPONENT, "party_member_left")
    }
    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        refuse("launcher_policy_fields")
    }
    fn operations_policy_fields_valid(&self, _: &Json) -> Result<bool, SeamError> {
        refuse("operations_policy_fields_valid")
    }
    fn operations_overview(&self) -> BoxFuture<'_, Result<OperationsOverview, SeamError>> {
        stub(COMPONENT, "operations_overview")
    }
    fn release_gates(&self) -> BoxFuture<'_, Result<Vec<ReleaseGate>, SeamError>> {
        stub(COMPONENT, "release_gates")
    }
}

impl RealmsEntry for Stub {
    fn authorize_game_session_realm(
        &self,
        _: AccountId,
        _: Option<Json>,
    ) -> BoxFuture<'_, Result<Option<RealmEntry>, SeamError>> {
        stub(COMPONENT, "authorize_game_session_realm")
    }
    fn bind_realm_presence_at_login(&self, _: AccountId, _: Option<RealmId>) -> BoxFuture<'_, Result<(), SeamError>> {
        stub(COMPONENT, "bind_realm_presence_at_login")
    }
    fn touch_presence(&self, _: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        stub(COMPONENT, "touch_presence")
    }
    fn find_by_hub_room_id(&self, _: String) -> BoxFuture<'_, Result<Option<RealmId>, SeamError>> {
        stub(COMPONENT, "find_by_hub_room_id")
    }
    fn is_member(&self, _: RealmId, _: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        stub(COMPONENT, "is_member")
    }
    fn is_banned(&self, _: RealmId, _: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        stub(COMPONENT, "is_banned")
    }
    fn hub_room_id(&self, _: &RealmId, _: HubKind) -> Result<String, SeamError> {
        refuse("hub_room_id")
    }
}

type Answer<'a, T> = BoxFuture<'a, Result<T, RealmsError>>;

impl HuntLifecycle for Stub {
    fn host_bootstrap(&self, _: AccountId, _: SessionId) -> Answer<'_, HostBootstrap> {
        stub(COMPONENT, "host_bootstrap")
    }
    fn route_ready(&self, _: AccountId, _: SessionId, _: u16) -> Answer<'_, ()> {
        stub(COMPONENT, "route_ready")
    }
    fn host_ready(&self, _: RuntimeAuth) -> Answer<'_, ()> {
        stub(COMPONENT, "host_ready")
    }
    fn abort(&self, _: RuntimeAuth, _: String) -> Answer<'_, ()> {
        stub(COMPONENT, "abort")
    }
    fn end(&self, _: RuntimeAuth, _: String) -> Answer<'_, ()> {
        stub(COMPONENT, "end")
    }
    fn status(&self, _: AccountId, _: SessionId) -> Answer<'_, HuntStatus> {
        stub(COMPONENT, "status")
    }
    fn observe(&self, _: AccountId, _: SessionId, _: Observation) -> Answer<'_, ()> {
        stub(COMPONENT, "observe")
    }
    fn start_hub(&self, _: AccountId, _: StartHub) -> Answer<'_, HostBootstrap> {
        stub(COMPONENT, "start_hub")
    }
    fn stop_hub(&self, _: AccountId, _: HubKind) -> Answer<'_, ()> {
        stub(COMPONENT, "stop_hub")
    }
    fn hosting_status(&self, _: AccountId) -> Answer<'_, Json> {
        stub(COMPONENT, "hosting_status")
    }
    fn set_emergency_stop(&self, _: String, _: Json) -> Answer<'_, Json> {
        stub(COMPONENT, "set_emergency_stop")
    }
    fn resolve_launch_manifest(&self, _: Json) -> Answer<'_, Json> {
        stub(COMPONENT, "resolve_launch_manifest")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::{Unimplemented, poll_once};

    fn account() -> AccountId {
        AccountId("a".to_owned())
    }

    fn session() -> SessionId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap()
    }

    fn realms(operation: &'static str) -> RealmsError {
        Unimplemented::new(COMPONENT, operation).into()
    }

    #[test]
    fn hooks_answer_unimplemented() {
        assert_eq!(Stub.dedicated_hunts_disabled(), refuse("dedicated_hunts_disabled"));
        assert_eq!(
            poll_once(Stub.authenticate(RequestInfo::default())).unwrap(),
            refuse::<AuthOutcome>("authenticate")
        );
        assert_eq!(
            poll_once(Stub.authorize_game_session_realm(account(), None)).unwrap(),
            refuse::<Option<RealmEntry>>("authorize_game_session_realm")
        );
        assert_eq!(Lifecycle::tables(&Stub), TABLES);
    }

    #[test]
    fn surfaces_answer_unimplemented() {
        assert_eq!(
            poll_once(Stub.host_bootstrap(account(), session())).unwrap(),
            Err(realms("host_bootstrap"))
        );
    }
}
