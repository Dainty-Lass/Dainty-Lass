// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Realms with nothing behind them: every operation is `Unimplemented` under its own name, so the module answers 501.

use rifts_seam::{
    AccountId, AuthOutcome, BoxFuture, Candidate, CandidateLeave, Component, DeletionStep, HubKind, HuntRouteContext,
    HuntRouteOutcome, Json, Lifecycle, OperationsOverview, PartyId, RealmEntry, RealmId, RealmsEntry, RealmsHooks,
    ReleaseGate, RequestInfo, RuntimeAuth, SeamError, SessionId, Settings, TableSpec, Unimplemented,
};

use crate::RealmsError;
use crate::hosting::{HostBootstrap, HuntLifecycle, HuntStatus, Observation, StartHub};

/// Answers every realms hook and route as not written yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

fn missing(operation: &'static str) -> SeamError {
    SeamError::unimplemented(Component::Realms, operation)
}

fn seam<T: Send + 'static>(operation: &'static str) -> BoxFuture<'static, Result<T, SeamError>> {
    rifts_seam::stub(Component::Realms, operation)
}

fn realms<'a, T: Send + 'a>(operation: &'static str) -> BoxFuture<'a, Result<T, RealmsError>> {
    Box::pin(std::future::ready(Err(RealmsError::Unimplemented(Unimplemented::new(
        Component::Realms,
        operation,
    )))))
}

impl Lifecycle for Stub {
    fn runtime_safety_issues(&self, _settings: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Err(missing("runtime_safety_issues"))
    }

    fn startup_summary(&self, _role: &str) -> Result<Option<Json>, SeamError> {
        Err(missing("startup_summary"))
    }

    fn start(&self, _role: &str) -> Result<(), SeamError> {
        Err(missing("start"))
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        seam("stop")
    }

    fn tables(&self) -> &'static [TableSpec] {
        crate::TABLES
    }

    /// A stub stores nothing.
    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        Vec::new()
    }
}

impl RealmsHooks for Stub {
    fn metric_class(&self, _path: &str) -> Option<&'static str> {
        None
    }

    fn request_log_tags(&self, _request: &RequestInfo) -> String {
        String::new()
    }

    fn authenticate(&self, _request: RequestInfo) -> BoxFuture<'_, Result<AuthOutcome, SeamError>> {
        seam("authenticate")
    }

    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        Err(missing("dedicated_hunts_disabled"))
    }

    fn filters_offline_party_members(&self, _game_mode: &str, _hunt_id: Option<&str>) -> Result<bool, SeamError> {
        Err(missing("filters_offline_party_members"))
    }

    fn restore_candidate(&self, _player: AccountId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        seam("restore_candidate")
    }

    fn candidate_status(
        &self,
        _player: AccountId,
        _current: Candidate,
    ) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        seam("candidate_status")
    }

    fn party_instance(&self, _party: PartyId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        seam("party_instance")
    }

    fn route_hunt(&self, _context: HuntRouteContext) -> BoxFuture<'_, Result<HuntRouteOutcome, SeamError>> {
        seam("route_hunt")
    }

    fn cancel_candidate(
        &self,
        _player: AccountId,
        _current: Candidate,
    ) -> BoxFuture<'_, Result<CandidateLeave, SeamError>> {
        seam("cancel_candidate")
    }

    fn party_member_left(&self, _account: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        seam("party_member_left")
    }

    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        Err(missing("launcher_policy_fields"))
    }

    fn operations_policy_fields_valid(&self, _record: &Json) -> Result<bool, SeamError> {
        Err(missing("operations_policy_fields_valid"))
    }

    fn operations_overview(&self) -> BoxFuture<'_, Result<OperationsOverview, SeamError>> {
        seam("operations_overview")
    }

    fn release_gates(&self) -> BoxFuture<'_, Result<Vec<ReleaseGate>, SeamError>> {
        seam("release_gates")
    }
}

impl RealmsEntry for Stub {
    fn authorize_game_session_realm(
        &self,
        _account: AccountId,
        _realm: Option<Json>,
    ) -> BoxFuture<'_, Result<Option<RealmEntry>, SeamError>> {
        seam("authorize_game_session_realm")
    }

    fn bind_realm_presence_at_login(
        &self,
        _account: AccountId,
        _realm: Option<RealmId>,
    ) -> BoxFuture<'_, Result<(), SeamError>> {
        seam("bind_realm_presence_at_login")
    }

    fn touch_presence(&self, _account: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        seam("touch_presence")
    }

    fn find_by_hub_room_id(&self, _room: String) -> BoxFuture<'_, Result<Option<RealmId>, SeamError>> {
        seam("find_by_hub_room_id")
    }

    fn is_member(&self, _realm: RealmId, _account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        seam("is_member")
    }

    fn is_banned(&self, _realm: RealmId, _account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        seam("is_banned")
    }

    fn hub_room_id(&self, _realm: &RealmId, _kind: HubKind) -> Result<String, SeamError> {
        Err(missing("hub_room_id"))
    }
}

impl HuntLifecycle for Stub {
    fn host_bootstrap(
        &self,
        _account: AccountId,
        _session: SessionId,
    ) -> BoxFuture<'_, Result<HostBootstrap, RealmsError>> {
        realms("host_bootstrap")
    }

    fn route_ready(
        &self,
        _account: AccountId,
        _session: SessionId,
        _port: u16,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        realms("route_ready")
    }

    fn host_ready(&self, _runtime: RuntimeAuth) -> BoxFuture<'_, Result<(), RealmsError>> {
        realms("host_ready")
    }

    fn abort(&self, _runtime: RuntimeAuth, _reason: String) -> BoxFuture<'_, Result<(), RealmsError>> {
        realms("abort")
    }

    fn end(&self, _runtime: RuntimeAuth, _reason: String) -> BoxFuture<'_, Result<(), RealmsError>> {
        realms("end")
    }

    fn status(&self, _account: AccountId, _session: SessionId) -> BoxFuture<'_, Result<HuntStatus, RealmsError>> {
        realms("status")
    }

    fn observe(
        &self,
        _account: AccountId,
        _session: SessionId,
        _observation: Observation,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        realms("observe")
    }

    fn start_hub(&self, _account: AccountId, _hub: StartHub) -> BoxFuture<'_, Result<HostBootstrap, RealmsError>> {
        realms("start_hub")
    }

    fn stop_hub(&self, _account: AccountId, _kind: HubKind) -> BoxFuture<'_, Result<(), RealmsError>> {
        realms("stop_hub")
    }

    fn hosting_status(&self, _account: AccountId) -> BoxFuture<'_, Result<Json, RealmsError>> {
        realms("hosting_status")
    }

    fn set_emergency_stop(&self, _admin_key: String, _policy: Json) -> BoxFuture<'_, Result<Json, RealmsError>> {
        realms("set_emergency_stop")
    }

    fn resolve_launch_manifest(&self, _request: Json) -> BoxFuture<'_, Result<Json, RealmsError>> {
        realms("resolve_launch_manifest")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::{GlobalId, poll_once};
    use std::collections::BTreeMap;

    fn name_of<T: std::fmt::Debug>(answer: Option<Result<T, SeamError>>) -> &'static str {
        match answer {
            Some(Err(SeamError::Unimplemented(u))) if u.component == Component::Realms => u.operation,
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    fn lifecycle_name_of<T: std::fmt::Debug>(answer: Option<Result<T, RealmsError>>) -> &'static str {
        match answer {
            Some(Err(RealmsError::Unimplemented(u))) if u.component == Component::Realms => u.operation,
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn every_hook_is_unimplemented_by_name() {
        let s = Stub;
        let account = || AccountId("a".into());
        let id = || -> GlobalId { "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap() };
        let candidate = || Candidate(Json::empty_object());
        let context = HuntRouteContext {
            game_mode: "ISLAND".into(),
            hunt_id: "h".into(),
            player: account(),
            party: None,
            party_members: Vec::new(),
            party_revision: None,
            build_id: None,
            is_hub: false,
        };
        let names = [
            name_of(Some(s.runtime_safety_issues(&BTreeMap::new()))),
            name_of(Some(s.startup_summary("backend"))),
            name_of(Some(s.start("backend"))),
            name_of(poll_once(s.stop())),
            name_of(poll_once(s.authenticate(RequestInfo::default()))),
            name_of(Some(s.dedicated_hunts_disabled())),
            name_of(Some(s.filters_offline_party_members("ISLAND", None))),
            name_of(poll_once(s.restore_candidate(account()))),
            name_of(poll_once(s.candidate_status(account(), candidate()))),
            name_of(poll_once(s.party_instance(PartyId(id())))),
            name_of(poll_once(s.route_hunt(context))),
            name_of(poll_once(s.cancel_candidate(account(), candidate()))),
            name_of(poll_once(s.party_member_left(account()))),
            name_of(Some(s.launcher_policy_fields())),
            name_of(Some(s.operations_policy_fields_valid(&Json::empty_object()))),
            name_of(poll_once(s.operations_overview())),
            name_of(poll_once(s.release_gates())),
            name_of(poll_once(s.authorize_game_session_realm(account(), None))),
            name_of(poll_once(s.bind_realm_presence_at_login(account(), None))),
            name_of(poll_once(s.touch_presence(account()))),
            name_of(poll_once(s.find_by_hub_room_id("r".into()))),
            name_of(poll_once(s.is_member(RealmId(id()), account()))),
            name_of(poll_once(s.is_banned(RealmId(id()), account()))),
            name_of(Some(s.hub_room_id(&RealmId(id()), HubKind::Ramsgate))),
        ];
        assert_eq!(
            names,
            [
                "runtime_safety_issues",
                "startup_summary",
                "start",
                "stop",
                "authenticate",
                "dedicated_hunts_disabled",
                "filters_offline_party_members",
                "restore_candidate",
                "candidate_status",
                "party_instance",
                "route_hunt",
                "cancel_candidate",
                "party_member_left",
                "launcher_policy_fields",
                "operations_policy_fields_valid",
                "operations_overview",
                "release_gates",
                "authorize_game_session_realm",
                "bind_realm_presence_at_login",
                "touch_presence",
                "find_by_hub_room_id",
                "is_member",
                "is_banned",
                "hub_room_id",
            ]
        );
        assert_eq!(s.tables(), crate::TABLES);
        assert!(s.account_deletion_steps().is_empty());
        assert_eq!(s.metric_class("/launcher/v1/realms"), None);
        assert_eq!(s.request_log_tags(&RequestInfo::default()), "");
    }

    #[test]
    fn the_lifecycle_facade_is_unimplemented_by_name() {
        let s = Stub;
        let account = || AccountId("a".into());
        let session = || SessionId("01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap());
        let runtime = || RuntimeAuth {
            capability_id: "c".into(),
            session: session(),
            generation: 1,
            host_account: account(),
            scopes: Vec::new(),
            roster: Vec::new(),
            host_peer: None,
            bridge_instance: None,
            wire_session: None,
        };
        let observation = Observation {
            host_reachable: true,
            local_bridge_alive: true,
            route_kind: None,
            ping_ms: None,
            jitter_ms: None,
            packet_loss_percent: None,
            last_packet_age_ms: None,
        };
        let hub = StartHub {
            kind: HubKind::Ramsgate,
            max_players: 16,
            afk_seconds: 0,
        };
        let names = [
            lifecycle_name_of(poll_once(s.host_bootstrap(account(), session()))),
            lifecycle_name_of(poll_once(s.route_ready(account(), session(), 1))),
            lifecycle_name_of(poll_once(s.host_ready(runtime()))),
            lifecycle_name_of(poll_once(s.abort(runtime(), "r".into()))),
            lifecycle_name_of(poll_once(s.end(runtime(), "r".into()))),
            lifecycle_name_of(poll_once(s.status(account(), session()))),
            lifecycle_name_of(poll_once(s.observe(account(), session(), observation))),
            lifecycle_name_of(poll_once(s.start_hub(account(), hub))),
            lifecycle_name_of(poll_once(s.stop_hub(account(), HubKind::Training))),
            lifecycle_name_of(poll_once(s.hosting_status(account()))),
            lifecycle_name_of(poll_once(s.set_emergency_stop("k".into(), Json::empty_object()))),
            lifecycle_name_of(poll_once(s.resolve_launch_manifest(Json::empty_object()))),
        ];
        assert_eq!(
            names,
            [
                "host_bootstrap",
                "route_ready",
                "host_ready",
                "abort",
                "end",
                "status",
                "observe",
                "start_hub",
                "stop_hub",
                "hosting_status",
                "set_emergency_stop",
                "resolve_launch_manifest",
            ]
        );
    }
}
