//! Every operation answers [`rifts_seam::Unimplemented`] until RF5 (RD18).

use rifts_seam::{
    AccountId, AuthOutcome, BoxFuture, Candidate, CandidateLeave, CapabilityScope, Component, DeletionStep, HubKind,
    HuntRouteContext, HuntRouteOutcome, Json, Lifecycle, OperationsOverview, PartyId, RealmEntry, RealmId, RealmsEntry,
    RealmsHooks, ReleaseGate, RequestInfo, RuntimeAuth, SeamError, Secret, SessionId, Settings, TableSpec, UnixMillis,
    stub,
};

use crate::RealmsError;
use crate::directory::{InviteOptions, JoinRealm, MemberAction, NewRealm, RealmDirectory, RealmSettings};
use crate::hosting::{
    ControlChannel, HostBootstrap, HostCapabilities, HostPresence, HostSelection, HuntLifecycle, HuntStatus, Leases,
    Observation, StartHub,
};
use crate::store::{
    ControlMessageRecord, HostCapabilityRecord, HostOfferRecord, RealmInviteRecord, RealmPresenceRecord, RealmRecord,
    TABLES,
};

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

impl RealmDirectory for Stub {
    fn create_realm(&self, _: AccountId, _: NewRealm) -> Answer<'_, RealmRecord> {
        stub(COMPONENT, "create_realm")
    }
    fn mine(&self, _: AccountId) -> Answer<'_, Option<RealmRecord>> {
        stub(COMPONENT, "mine")
    }
    fn listed(&self, _: AccountId, _: u32, _: u32) -> Answer<'_, Vec<RealmRecord>> {
        stub(COMPONENT, "listed")
    }
    fn by_name(&self, _: AccountId, _: String, _: Option<String>) -> Answer<'_, Option<RealmRecord>> {
        stub(COMPONENT, "by_name")
    }
    fn preview_invite(&self, _: AccountId, _: String) -> Answer<'_, RealmRecord> {
        stub(COMPONENT, "preview_invite")
    }
    fn join(&self, _: AccountId, _: JoinRealm) -> Answer<'_, RealmRecord> {
        stub(COMPONENT, "join")
    }
    fn leave(&self, _: AccountId, _: String) -> Answer<'_, ()> {
        stub(COMPONENT, "leave")
    }
    fn update_settings(&self, _: AccountId, _: RealmSettings) -> Answer<'_, RealmRecord> {
        stub(COMPONENT, "update_settings")
    }
    fn create_invite(&self, _: AccountId, _: InviteOptions) -> Answer<'_, RealmInviteRecord> {
        stub(COMPONENT, "create_invite")
    }
    fn invites(&self, _: AccountId) -> Answer<'_, Vec<RealmInviteRecord>> {
        stub(COMPONENT, "invites")
    }
    fn revoke_invite(&self, _: AccountId, _: String) -> Answer<'_, ()> {
        stub(COMPONENT, "revoke_invite")
    }
    fn moderate(&self, _: AccountId, _: AccountId, _: MemberAction) -> Answer<'_, ()> {
        stub(COMPONENT, "moderate")
    }
    fn host_heartbeat(&self, _: AccountId) -> Answer<'_, RealmRecord> {
        stub(COMPONENT, "host_heartbeat")
    }
    fn release(&self, _: AccountId) -> Answer<'_, ()> {
        stub(COMPONENT, "release")
    }
    fn join_requests(&self, _: AccountId) -> Answer<'_, Vec<AccountId>> {
        stub(COMPONENT, "join_requests")
    }
    fn decide_join_request(&self, _: AccountId, _: AccountId, _: bool) -> Answer<'_, ()> {
        stub(COMPONENT, "decide_join_request")
    }
    fn invite_player(&self, _: AccountId, _: String) -> Answer<'_, ()> {
        stub(COMPONENT, "invite_player")
    }
    fn cancel_player_invite(&self, _: AccountId, _: AccountId) -> Answer<'_, ()> {
        stub(COMPONENT, "cancel_player_invite")
    }
    fn player_invites(&self, _: AccountId) -> Answer<'_, Vec<RealmRecord>> {
        stub(COMPONENT, "player_invites")
    }
    fn answer_player_invite(&self, _: AccountId, _: String, _: bool) -> Answer<'_, ()> {
        stub(COMPONENT, "answer_player_invite")
    }
    fn presence_heartbeat(&self, _: AccountId) -> Answer<'_, RealmPresenceRecord> {
        stub(COMPONENT, "presence_heartbeat")
    }
    fn presence_leave(&self, _: AccountId) -> Answer<'_, ()> {
        stub(COMPONENT, "presence_leave")
    }
    fn presence(&self, _: AccountId) -> Answer<'_, Option<RealmPresenceRecord>> {
        stub(COMPONENT, "presence")
    }
}

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

impl HostCapabilities for Stub {
    fn mint(
        &self,
        _: SessionId,
        _: u64,
        _: AccountId,
        _: Vec<CapabilityScope>,
    ) -> Answer<'_, (Secret, HostCapabilityRecord)> {
        stub(COMPONENT, "capabilities.mint")
    }
    fn check(&self, _: Secret, _: SessionId, _: CapabilityScope) -> Answer<'_, RuntimeAuth> {
        stub(COMPONENT, "capabilities.check")
    }
    fn revoke(&self, _: SessionId, _: u64) -> Answer<'_, ()> {
        stub(COMPONENT, "capabilities.revoke")
    }
}

impl Leases for Stub {
    fn renew(&self, _: RuntimeAuth, _: u64) -> Answer<'_, UnixMillis> {
        stub(COMPONENT, "leases.renew")
    }
    fn sweep(&self, _: UnixMillis) -> Answer<'_, Vec<(SessionId, u64)>> {
        stub(COMPONENT, "leases.sweep")
    }
}

impl HostSelection for Stub {
    fn submit_presence(&self, _: AccountId, _: HostPresence) -> Answer<'_, ()> {
        stub(COMPONENT, "submit_presence")
    }
    fn respond_to_offer(&self, _: AccountId, _: String, _: bool, _: Option<String>) -> Answer<'_, ()> {
        stub(COMPONENT, "respond_to_offer")
    }
    fn select_host(&self, _: SessionId, _: u64) -> Answer<'_, Option<HostOfferRecord>> {
        stub(COMPONENT, "select_host")
    }
}

impl ControlChannel for Stub {
    fn poll(&self, _: AccountId, _: u64, _: u32) -> Answer<'_, Vec<ControlMessageRecord>> {
        stub(COMPONENT, "control.poll")
    }
    fn ack(&self, _: AccountId, _: String) -> Answer<'_, ()> {
        stub(COMPONENT, "control.ack")
    }
    fn send(&self, _: AccountId, _: String, _: Option<SessionId>, _: Json) -> Answer<'_, u64> {
        stub(COMPONENT, "control.send")
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
        assert_eq!(poll_once(Stub.mine(account())).unwrap(), Err(realms("mine")));
        assert_eq!(
            poll_once(Stub.host_bootstrap(account(), session())).unwrap(),
            Err(realms("host_bootstrap"))
        );
        assert_eq!(
            poll_once(Stub.sweep(UnixMillis(0))).unwrap(),
            Err(realms("leases.sweep"))
        );
        assert_eq!(
            poll_once(Stub.select_host(session(), 1)).unwrap(),
            Err(realms("select_host"))
        );
        assert_eq!(
            poll_once(Stub.poll(account(), 0, 0)).unwrap(),
            Err(realms("control.poll"))
        );
    }
}
