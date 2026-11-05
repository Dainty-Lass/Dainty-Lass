//! [`RealmsService`]: the live [`rifts_seam::RealmsHooks`], [`rifts_seam::RealmsEntry`] and [`rifts_seam::Lifecycle`]
//! implementation over [`RealmDirectory`] and [`HostCapabilities`].

use std::sync::Arc;

use module_api::NodeId;
use rifts_seam::{
    AccountId, AuthOutcome, BoxFuture, Candidate, CandidateLeave, Component, DedicatedOnly, DeletionAction,
    DeletionStep, HOST_CAPABILITY_HEADER, HubKind, HuntRouteContext, HuntRouteOutcome, IslesCore, Json, Lifecycle,
    OperationsOverview, PartyId, RealmEntry, RealmId, RealmsEntry, RealmsHooks, Refusal, ReleaseGate, Reply,
    RequestInfo, SeamError, Settings, TableSpec,
};
use serde_json::Value;

use crate::capabilities::{HostCapabilities, SignedUser, runtime_identity};
use crate::directory::RealmDirectory;
use crate::ids::{seam_realm_id, wire_realm_id};
use crate::leases::Hosting;
use crate::ports::Clock;
use crate::realm::{hub_room_id, realms_module_enabled};
use crate::tiers::HostingPolicy;
use crate::{RealmsError, TABLES};

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
    directory: Option<Arc<RealmDirectory>>,
    capabilities: Option<Arc<HostCapabilities>>,
    hosting: Option<Arc<Hosting>>,
    clock: Arc<dyn Clock>,
    core: Arc<dyn IslesCore>,
    policy: HostingPolicy,
    realms_enabled: bool,
    node: NodeId,
}

impl RealmsService {
    pub fn new(parts: RealmsServiceParts) -> Self {
        Self {
            directory: parts.directory,
            capabilities: parts.capabilities,
            hosting: parts.hosting,
            clock: parts.clock,
            core: parts.core,
            policy: parts.policy,
            realms_enabled: parts.realms_enabled,
            node: parts.node,
        }
    }
}

pub(crate) fn to_seam(error: RealmsError) -> SeamError {
    match error {
        RealmsError::Unimplemented(unimplemented) => SeamError::Unimplemented(unimplemented),
        RealmsError::Refused(refusal) => SeamError::Refused(refusal),
        RealmsError::Database(_) => SeamError::Refused(Refusal {
            code: "INTERNAL".to_owned(),
            message: "The realms request failed.".to_owned(),
        }),
    }
}

fn capability_refused() -> Reply {
    Reply {
        status: 401,
        body: Json(r#"{"code":"unauthorized","message":"Invalid host capability.","payload":null}"#.to_owned()),
    }
}

impl Lifecycle for RealmsService {
    fn runtime_safety_issues(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError> {
        let mut issues = Vec::new();
        if let Some(warning) = crate::tiers::relay_only_warning(settings) {
            issues.push(warning.to_owned());
        }
        Ok(issues)
    }

    fn startup_summary(&self, _: &str) -> Result<Option<Json>, SeamError> {
        Ok(Some(Json(format!(
            r#"{{"p2pHuntsEnabled":{},"realmsEnabled":{},"trustTier":{}}}"#,
            self.policy.enabled, self.realms_enabled, self.policy.trust_tier
        ))))
    }

    fn start(&self, _: &str) -> Result<(), SeamError> {
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async { Ok(()) })
    }

    fn tables(&self) -> &'static [TableSpec] {
        TABLES
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        vec![
            DeletionStep {
                table: "rifts_realms",
                account_column: "host_account_id",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_realm_members",
                account_column: "account_id",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_realm_bans",
                account_column: "account_id",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_realm_join_requests",
                account_column: "account_id",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_realm_player_invites",
                account_column: "account_id",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_realm_invites",
                account_column: "created_by",
                action: DeletionAction::Delete,
            },
            DeletionStep {
                table: "rifts_realm_presence",
                account_column: "account_id",
                action: DeletionAction::Delete,
            },
        ]
    }
}

impl RealmsHooks for RealmsService {
    fn metric_class(&self, path: &str) -> Option<&'static str> {
        if path.starts_with("/launcher/v1/realms") || path.starts_with("/launcher/v1/realm-hosting") {
            Some("realms")
        } else if path.starts_with("/launcher/v1/p2p") || path.starts_with("/mystic/v1/hunts") {
            Some("p2p")
        } else {
            None
        }
    }

    fn request_log_tags(&self, _: &RequestInfo) -> String {
        String::new()
    }

    fn authenticate(&self, request: RequestInfo) -> BoxFuture<'_, Result<AuthOutcome, SeamError>> {
        Box::pin(async move {
            let Some(raw) = request.header(HOST_CAPABILITY_HEADER) else {
                return Ok(AuthOutcome::NotMine);
            };
            let Some(capabilities) = &self.capabilities else {
                return Ok(AuthOutcome::Refused(capability_refused()));
            };
            let context = capabilities
                .resolve_runtime(Some(raw), self.clock.now())
                .await
                .map_err(to_seam)?;
            let Some(context) = context else {
                return Ok(AuthOutcome::Refused(capability_refused()));
            };
            let Some(identity) = runtime_identity(&context, SignedUser::Absent, None) else {
                return Ok(AuthOutcome::Refused(capability_refused()));
            };
            let Some(auth) = capabilities.runtime_auth(&context, &identity) else {
                return Ok(AuthOutcome::Refused(capability_refused()));
            };
            Ok(AuthOutcome::Authenticated(auth))
        })
    }

    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        let _ = &self.core;
        Ok(self.policy.dedicated_hunts_disabled)
    }

    fn filters_offline_party_members(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError> {
        DedicatedOnly.filters_offline_party_members(game_mode, hunt_id)
    }

    fn restore_candidate(&self, player: AccountId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        Box::pin(async move {
            let Some(hosting) = &self.hosting else {
                return DedicatedOnly.restore_candidate(player).await;
            };
            hosting.restore_matchmaking_candidate(&player.0).await.map_err(to_seam)
        })
    }

    fn candidate_status(
        &self,
        player: AccountId,
        current: Candidate,
    ) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        Box::pin(async move {
            let Some(hosting) = &self.hosting else {
                return DedicatedOnly.candidate_status(player, current).await;
            };
            hosting
                .poll_candidate_status(&player.0, &current)
                .await
                .map_err(to_seam)
        })
    }

    fn party_instance(&self, party: PartyId) -> BoxFuture<'_, Result<Option<Candidate>, SeamError>> {
        DedicatedOnly.party_instance(party)
    }

    fn route_hunt(&self, context: HuntRouteContext) -> BoxFuture<'_, Result<HuntRouteOutcome, SeamError>> {
        Box::pin(async move {
            let Some(hosting) = &self.hosting else {
                return DedicatedOnly.route_hunt(context).await;
            };
            let realm_id = if let Some(directory) = &self.directory {
                directory
                    .presence_of(&context.player.0)
                    .await
                    .ok()
                    .flatten()
                    .map(|p| p.realm_id)
            } else {
                None
            };
            hosting
                .route_matchmaking_hunt(&context, realm_id)
                .await
                .map_err(to_seam)
        })
    }

    fn cancel_candidate(
        &self,
        player: AccountId,
        current: Candidate,
    ) -> BoxFuture<'_, Result<CandidateLeave, SeamError>> {
        DedicatedOnly.cancel_candidate(player, current)
    }

    fn party_member_left(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        DedicatedOnly.party_member_left(account)
    }

    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        Ok(vec![
            ("p2pEmergencyStop".to_owned(), Json("false".to_owned())),
            (
                "coopHunts".to_owned(),
                Json(format!(
                    r#"{{"enabled":{},"maxPartySize":{},"forceSdr":{},"trustTier":{}}}"#,
                    self.policy.enabled, self.policy.max_party_size, self.policy.force_sdr, self.policy.trust_tier
                )),
            ),
        ])
    }

    fn operations_policy_fields_valid(&self, record: &Json) -> Result<bool, SeamError> {
        DedicatedOnly.operations_policy_fields_valid(record)
    }

    fn operations_overview(&self) -> BoxFuture<'_, Result<OperationsOverview, SeamError>> {
        DedicatedOnly.operations_overview()
    }

    fn release_gates(&self) -> BoxFuture<'_, Result<Vec<ReleaseGate>, SeamError>> {
        DedicatedOnly.release_gates()
    }
}

impl RealmsEntry for RealmsService {
    fn authorize_game_session_realm(
        &self,
        account: AccountId,
        realm: Option<Json>,
    ) -> BoxFuture<'_, Result<Option<RealmEntry>, SeamError>> {
        Box::pin(async move {
            let Some(directory) = &self.directory else {
                return DedicatedOnly.authorize_game_session_realm(account, realm).await;
            };
            let parsed = match &realm {
                Some(raw) => Some(serde_json::from_str::<Value>(&raw.0).map_err(|_| {
                    SeamError::Refused(Refusal {
                        code: "REALM_VALIDATION_FAILED".to_owned(),
                        message: "realm must be a JSON object.".to_owned(),
                    })
                })?),
                None => None,
            };
            let authorized = directory
                .authorize_game_session_realm(&account.0, parsed.as_ref(), self.realms_enabled)
                .await
                .map_err(to_seam)?;
            let Some((wire_id, is_host)) = authorized else {
                return Ok(None);
            };
            let seam_id = seam_realm_id(&wire_id, &self.node)
                .or_else(|| wire_id.parse().ok())
                .ok_or_else(|| SeamError::unimplemented(Component::Realms, "authorize_game_session_realm"))?;
            Ok(Some(RealmEntry {
                realm: seam_id,
                is_host,
            }))
        })
    }

    fn bind_realm_presence_at_login(
        &self,
        account: AccountId,
        realm: Option<RealmId>,
    ) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async move {
            let Some(directory) = &self.directory else {
                return Ok(());
            };
            let wire = realm.as_ref().map(wire_realm_id);
            directory
                .bind_presence_at_login(&account.0, wire.as_deref(), self.realms_enabled)
                .await
                .map_err(to_seam)
        })
    }

    fn touch_presence(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async move {
            let Some(directory) = &self.directory else {
                return Ok(());
            };
            directory.touch_presence(&account.0).await.map(|_| ()).map_err(to_seam)
        })
    }

    fn find_by_hub_room_id(&self, room: String) -> BoxFuture<'_, Result<Option<RealmId>, SeamError>> {
        Box::pin(async move {
            let Some(directory) = &self.directory else {
                return Ok(None);
            };
            let found = directory.find_by_hub_room_id(&room).await.map_err(to_seam)?;
            Ok(found.and_then(|record| {
                seam_realm_id(&record.realm_id, &self.node).or_else(|| record.realm_id.parse().ok())
            }))
        })
    }

    fn is_member(&self, realm: RealmId, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        Box::pin(async move {
            let Some(directory) = &self.directory else {
                return Ok(false);
            };
            let wire = wire_realm_id(&realm);
            directory.is_member(&wire, &account.0).await.map_err(to_seam)
        })
    }

    fn is_banned(&self, realm: RealmId, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        Box::pin(async move {
            let Some(directory) = &self.directory else {
                return Ok(false);
            };
            let wire = wire_realm_id(&realm);
            directory.is_banned(&wire, &account.0).await.map_err(to_seam)
        })
    }

    fn hub_room_id(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError> {
        if !self.realms_enabled && self.directory.is_none() {
            return DedicatedOnly.hub_room_id(realm, kind);
        }
        Ok(hub_room_id(&wire_realm_id(realm), kind))
    }
}

/// Helper to read `MYSTICPARADOX_REALMS_MODULE`, defaulting to true when `realms` feature is active unless explicitly
/// set to `0`, `false`, `off` or `no`.
pub fn realms_enabled_or_default(settings: &dyn Settings) -> bool {
    match settings.get("MYSTICPARADOX_REALMS_MODULE") {
        Some(raw) if !raw.trim().is_empty() => realms_module_enabled(settings),
        _ => true,
    }
}
