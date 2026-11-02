// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `RealmsService`: what realms answer to rifts-seam's lifecycle, hook and entry calls.

use std::sync::Arc;

use module_api::NodeId;
use rifts_seam::{
    AccountId, AuthOutcome, BoxFuture, Candidate, CandidateLeave, Component, DedicatedOnly, DeletionAction,
    DeletionStep, HOST_CAPABILITY_HEADER, HubKind, HuntRouteContext, HuntRouteOutcome, IslesCore, Json, Lifecycle,
    OperationsOverview, PartyId, RealmEntry, RealmId, RealmsEntry, RealmsHooks, Refusal, ReleaseGate, Reply,
    RequestInfo, SeamError, Settings, TableSpec,
};
use serde_json::{Value, json};

use crate::RealmsError;
use crate::capabilities::{HostCapabilities, SignedUser, runtime_identity};
use crate::directory::RealmDirectory;
use crate::ids::{seam_realm_id, wire_realm_id};
use crate::leases::Hosting;
use crate::ports::Clock;
use crate::realm::{REALMS_MODULE_SETTING, hub_room_id, realms_module_enabled};
use crate::tiers::{HostingPolicy, relay_only_warning};

/// The database's text never reaches a client.
fn seam_error(error: RealmsError) -> SeamError {
    match error {
        RealmsError::Unimplemented(missing) => SeamError::Unimplemented(missing),
        RealmsError::Refused(refusal) => SeamError::Refused(refusal),
        RealmsError::Database(_) => SeamError::Refused(Refusal {
            code: "INTERNAL".to_owned(),
            message: "The realms request failed.".to_owned(),
        }),
    }
}

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

pub struct RealmsServiceParts {
    pub directory: Option<Arc<RealmDirectory>>,
    pub capabilities: Option<Arc<HostCapabilities>>,
    pub hosting: Option<Arc<Hosting>>,
    pub clock: Arc<dyn Clock>,
    pub core: Arc<dyn IslesCore>,
    pub policy: HostingPolicy,
    pub realms_enabled: bool,
    pub node: NodeId,
}

pub struct RealmsService {
    parts: RealmsServiceParts,
}

/// On unless the setting is given: then the realms module switch decides.
pub fn realms_enabled_or_default(settings: &dyn Settings) -> bool {
    match settings.get(REALMS_MODULE_SETTING) {
        Some(value) if !value.trim().is_empty() => realms_module_enabled(settings),
        _ => true,
    }
}

fn ready<'a, T: Send + 'a>(answer: Result<T, SeamError>) -> Fut<'a, T> {
    Box::pin(std::future::ready(answer))
}

fn invalid_capability() -> AuthOutcome {
    AuthOutcome::Refused(Reply {
        status: 401,
        body: Json(r#"{"code":"unauthorized","message":"Invalid host capability.","payload":null}"#.to_owned()),
    })
}

const DELETION_STEPS: [(&str, &str); 7] = [
    ("rifts_realms", "host_account_id"),
    ("rifts_realm_members", "account_id"),
    ("rifts_realm_bans", "account_id"),
    ("rifts_realm_join_requests", "account_id"),
    ("rifts_realm_player_invites", "account_id"),
    ("rifts_realm_invites", "created_by"),
    ("rifts_realm_presence", "account_id"),
];

impl RealmsService {
    pub fn new(parts: RealmsServiceParts) -> Self {
        Self { parts }
    }

    /// The seam id of a stored realm id on this node, else the text read as one.
    fn realm_id(&self, wire: &str) -> Option<RealmId> {
        seam_realm_id(wire, &self.parts.node).or_else(|| wire.parse().ok())
    }
}

impl Lifecycle for RealmsService {
    fn runtime_safety_issues(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(relay_only_warning(settings).into_iter().collect())
    }

    fn startup_summary(&self, _role: &str) -> Result<Option<Json>, SeamError> {
        let summary = json!({
            "p2pHuntsEnabled": self.parts.policy.enabled,
            "realmsEnabled": self.parts.realms_enabled,
            "trustTier": self.parts.policy.trust_tier,
        });
        Ok(Some(Json(summary.to_string())))
    }

    /// Starts nothing: no purge sweeper runs (spec-rd27-rifts §7 risk 3).
    fn start(&self, _role: &str) -> Result<(), SeamError> {
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        ready(Ok(()))
    }

    fn tables(&self) -> &'static [TableSpec] {
        crate::TABLES
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        DELETION_STEPS
            .iter()
            .map(|(table, account_column)| DeletionStep {
                table,
                account_column,
                action: DeletionAction::Delete,
            })
            .collect()
    }
}

impl RealmsHooks for RealmsService {
    fn metric_class(&self, path: &str) -> Option<&'static str> {
        if path.starts_with("/launcher/v1/realms") || path.starts_with("/launcher/v1/realm-hosting") {
            Some("realms")
        } else if path.starts_with("/launcher/v1/p2p") || path.starts_with("/rustic/v1/hunts") {
            Some("p2p")
        } else {
            None
        }
    }

    fn request_log_tags(&self, _request: &RequestInfo) -> String {
        String::new()
    }

    fn authenticate(&self, request: RequestInfo) -> Fut<'_, AuthOutcome> {
        if request.header(HOST_CAPABILITY_HEADER).is_none() {
            return ready(Ok(AuthOutcome::NotMine));
        }
        let Some(capabilities) = self.parts.capabilities.clone() else {
            return ready(Ok(invalid_capability()));
        };
        Box::pin(async move {
            let header = request.header(HOST_CAPABILITY_HEADER);
            let Some(context) = capabilities
                .resolve_runtime(header, self.parts.clock.now())
                .await
                .map_err(seam_error)?
            else {
                return Ok(invalid_capability());
            };
            let Some(identity) = runtime_identity(&context, SignedUser::Absent, None) else {
                return Ok(invalid_capability());
            };
            Ok(capabilities
                .runtime_auth(&context, &identity)
                .map_or_else(invalid_capability, AuthOutcome::Authenticated))
        })
    }

    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        Ok(self.parts.policy.dedicated_hunts_disabled)
    }

    fn filters_offline_party_members(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError> {
        DedicatedOnly.filters_offline_party_members(game_mode, hunt_id)
    }

    fn restore_candidate(&self, player: AccountId) -> Fut<'_, Option<Candidate>> {
        match self.parts.hosting.clone() {
            None => DedicatedOnly.restore_candidate(player),
            Some(hosting) => {
                Box::pin(async move { hosting.restore_matchmaking_candidate(&player).await.map_err(seam_error) })
            }
        }
    }

    fn candidate_status(&self, player: AccountId, current: Candidate) -> Fut<'_, Option<Candidate>> {
        match self.parts.hosting.clone() {
            None => DedicatedOnly.candidate_status(player, current),
            Some(hosting) => Box::pin(async move {
                hosting
                    .poll_candidate_status(&player, &current)
                    .await
                    .map_err(seam_error)
            }),
        }
    }

    fn party_instance(&self, party: PartyId) -> Fut<'_, Option<Candidate>> {
        DedicatedOnly.party_instance(party)
    }

    fn route_hunt(&self, context: HuntRouteContext) -> Fut<'_, HuntRouteOutcome> {
        match self.parts.hosting.clone() {
            None => DedicatedOnly.route_hunt(context),
            Some(hosting) => Box::pin(async move {
                // Any error or absence reads as no realm.
                let realm = match &self.parts.directory {
                    Some(directory) => directory
                        .presence_of(&context.player.0)
                        .await
                        .ok()
                        .flatten()
                        .map(|row| row.realm_id),
                    None => None,
                };
                hosting
                    .route_matchmaking_hunt(&context, realm)
                    .await
                    .map_err(seam_error)
            }),
        }
    }

    fn cancel_candidate(&self, player: AccountId, current: Candidate) -> Fut<'_, CandidateLeave> {
        DedicatedOnly.cancel_candidate(player, current)
    }

    fn party_member_left(&self, account: AccountId) -> Fut<'_, ()> {
        DedicatedOnly.party_member_left(account)
    }

    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        let policy = &self.parts.policy;
        let coop = json!({
            "enabled": policy.enabled,
            "maxPartySize": policy.max_party_size,
            "forceSdr": policy.force_sdr,
            "trustTier": policy.trust_tier,
        });
        Ok(vec![
            ("p2pEmergencyStop".to_owned(), Json("false".to_owned())),
            ("coopHunts".to_owned(), Json(coop.to_string())),
        ])
    }

    fn operations_policy_fields_valid(&self, record: &Json) -> Result<bool, SeamError> {
        DedicatedOnly.operations_policy_fields_valid(record)
    }

    fn operations_overview(&self) -> Fut<'_, OperationsOverview> {
        DedicatedOnly.operations_overview()
    }

    fn release_gates(&self) -> Fut<'_, Vec<ReleaseGate>> {
        DedicatedOnly.release_gates()
    }
}

impl RealmsEntry for RealmsService {
    fn authorize_game_session_realm(&self, account: AccountId, realm: Option<Json>) -> Fut<'_, Option<RealmEntry>> {
        if self.parts.directory.is_none() {
            return DedicatedOnly.authorize_game_session_realm(account, realm);
        }
        let parsed = match &realm {
            None => None,
            Some(text) => match serde_json::from_str::<Value>(&text.0) {
                Ok(value) => Some(value),
                Err(_) => {
                    return ready(Err(SeamError::Refused(Refusal {
                        code: "REALM_VALIDATION_FAILED".to_owned(),
                        message: "realm must be a JSON object.".to_owned(),
                    })));
                }
            },
        };
        Box::pin(async move {
            let Some(directory) = &self.parts.directory else {
                return Ok(None);
            };
            let answer = directory
                .authorize_game_session_realm(&account.0, parsed.as_ref(), self.parts.realms_enabled)
                .await
                .map_err(seam_error)?;
            let Some((wire, is_host)) = answer else {
                return Ok(None);
            };
            let realm = self
                .realm_id(&wire)
                .ok_or_else(|| SeamError::unimplemented(Component::Realms, "authorize_game_session_realm"))?;
            Ok(Some(RealmEntry { realm, is_host }))
        })
    }

    fn bind_realm_presence_at_login(&self, account: AccountId, realm: Option<RealmId>) -> Fut<'_, ()> {
        match self.parts.directory.clone() {
            None => ready(Ok(())),
            Some(directory) => Box::pin(async move {
                let wire = realm.as_ref().map(wire_realm_id);
                directory
                    .bind_presence_at_login(&account.0, wire.as_deref(), self.parts.realms_enabled)
                    .await
                    .map_err(seam_error)
            }),
        }
    }

    fn touch_presence(&self, account: AccountId) -> Fut<'_, ()> {
        match self.parts.directory.clone() {
            None => ready(Ok(())),
            Some(directory) => Box::pin(async move {
                directory.touch_presence(&account.0).await.map_err(seam_error)?;
                Ok(())
            }),
        }
    }

    fn find_by_hub_room_id(&self, room: String) -> Fut<'_, Option<RealmId>> {
        match self.parts.directory.clone() {
            None => ready(Ok(None)),
            Some(directory) => Box::pin(async move {
                let found = directory.find_by_hub_room_id(&room).await.map_err(seam_error)?;
                Ok(found.and_then(|realm| self.realm_id(&realm.realm_id)))
            }),
        }
    }

    fn is_member(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool> {
        match self.parts.directory.clone() {
            None => ready(Ok(false)),
            Some(directory) => Box::pin(async move {
                directory
                    .is_member(&wire_realm_id(&realm), &account.0)
                    .await
                    .map_err(seam_error)
            }),
        }
    }

    fn is_banned(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool> {
        match self.parts.directory.clone() {
            None => ready(Ok(false)),
            Some(directory) => Box::pin(async move {
                directory
                    .is_banned(&wire_realm_id(&realm), &account.0)
                    .await
                    .map_err(seam_error)
            }),
        }
    }

    /// With realms off but a directory present it still answers (kept as found, store spec §17 item 12).
    fn hub_room_id(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError> {
        if !self.parts.realms_enabled && self.parts.directory.is_none() {
            return DedicatedOnly.hub_room_id(realm, kind);
        }
        Ok(hub_room_id(&wire_realm_id(realm), kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::SystemClock;
    use rifts_seam::{NoCore, poll_once};
    use std::collections::BTreeMap;

    fn service(policy: HostingPolicy, realms_enabled: bool) -> RealmsService {
        RealmsService::new(RealmsServiceParts {
            directory: None,
            capabilities: None,
            hosting: None,
            clock: Arc::new(SystemClock),
            core: Arc::new(NoCore),
            policy,
            realms_enabled,
            node: NodeId::new("local").unwrap(),
        })
    }

    #[test]
    fn summary_and_policy_fields_are_byte_exact() {
        let s = service(
            HostingPolicy {
                enabled: true,
                trust_tier: 1,
                max_party_size: 4,
                force_sdr: true,
                dedicated_hunts_disabled: true,
                ..HostingPolicy::default()
            },
            false,
        );
        assert_eq!(
            s.startup_summary("backend").unwrap().unwrap().0,
            r#"{"p2pHuntsEnabled":true,"realmsEnabled":false,"trustTier":1}"#
        );
        assert_eq!(
            s.launcher_policy_fields().unwrap(),
            vec![
                ("p2pEmergencyStop".to_owned(), Json("false".into())),
                (
                    "coopHunts".to_owned(),
                    Json(r#"{"enabled":true,"maxPartySize":4,"forceSdr":true,"trustTier":1}"#.into())
                ),
            ]
        );
        assert_eq!(s.dedicated_hunts_disabled(), Ok(true));
        assert_eq!(s.tables(), crate::TABLES);
        let steps: Vec<(&str, &str)> = s
            .account_deletion_steps()
            .iter()
            .map(|step| (step.table, step.account_column))
            .collect();
        assert_eq!(steps, DELETION_STEPS);
        assert_eq!(s.start("backend"), Ok(()));
        assert_eq!(poll_once(s.stop()), Some(Ok(())));
    }

    #[test]
    fn metric_classes() {
        let s = service(HostingPolicy::default(), true);
        for (path, class) in [
            ("/launcher/v1/realms/mine", Some("realms")),
            ("/launcher/v1/realm-hosting/status", Some("realms")),
            ("/launcher/v1/p2p/control", Some("p2p")),
            ("/rustic/v1/hunts/x/lease", Some("p2p")),
            ("/launcher/v1/play", None),
        ] {
            assert_eq!(s.metric_class(path), class, "{path}");
        }
        assert_eq!(s.request_log_tags(&RequestInfo::default()), "");
    }

    #[test]
    fn authenticate_without_capabilities() {
        let s = service(HostingPolicy::default(), true);
        assert_eq!(
            poll_once(s.authenticate(RequestInfo::default())),
            Some(Ok(AuthOutcome::NotMine))
        );
        let with_header = RequestInfo {
            headers: vec![(HOST_CAPABILITY_HEADER.to_owned(), "x".to_owned())],
            ..RequestInfo::default()
        };
        assert_eq!(poll_once(s.authenticate(with_header)), Some(Ok(invalid_capability())));
    }

    #[test]
    fn entry_without_a_directory() {
        let s = service(HostingPolicy::default(), false);
        let realm: RealmId = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap();
        let account = || AccountId("a".into());
        assert_eq!(poll_once(s.touch_presence(account())), Some(Ok(())));
        assert_eq!(poll_once(s.is_member(realm.clone(), account())), Some(Ok(false)));
        assert_eq!(poll_once(s.find_by_hub_room_id("r".into())), Some(Ok(None)));
        assert_eq!(
            s.hub_room_id(&realm, HubKind::Ramsgate),
            DedicatedOnly.hub_room_id(&realm, HubKind::Ramsgate)
        );
        let on = service(HostingPolicy::default(), true);
        assert_eq!(
            on.hub_room_id(&realm, HubKind::Training),
            Ok(hub_room_id(&wire_realm_id(&realm), HubKind::Training))
        );
        assert_eq!(
            poll_once(on.route_hunt(HuntRouteContext {
                game_mode: "ISLAND".into(),
                hunt_id: "h".into(),
                player: account(),
                party: None,
                party_members: Vec::new(),
                party_revision: None,
                build_id: None,
                is_hub: false,
            })),
            Some(Ok(HuntRouteOutcome::NotHandled))
        );
    }

    #[test]
    fn realms_switch_defaults_on() {
        let with =
            |value: &str| -> BTreeMap<String, String> { [(REALMS_MODULE_SETTING.to_owned(), value.to_owned())].into() };
        assert!(realms_enabled_or_default(&BTreeMap::new()));
        assert!(realms_enabled_or_default(&with("  ")));
        assert!(realms_enabled_or_default(&with("on")));
        assert!(!realms_enabled_or_default(&with("off")));
        assert!(!realms_enabled_or_default(&with("0")));
    }

    #[test]
    fn safety_issue_is_the_relay_warning() {
        let s = service(HostingPolicy::default(), true);
        let on: BTreeMap<String, String> = [("P2P_HUNTS_ENABLED".to_owned(), "true".to_owned())].into();
        assert_eq!(s.runtime_safety_issues(&on).unwrap().len(), 1);
        assert!(s.runtime_safety_issues(&BTreeMap::new()).unwrap().is_empty());
    }
}
