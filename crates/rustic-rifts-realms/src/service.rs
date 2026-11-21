// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `RealmsService`: what realms answer to rifts-seam's lifecycle, hook and entry calls.

use std::future::Future;
use std::sync::Arc;

use dainty_module_api::NodeId;
use rustic_rifts_seam::{
    AccountId, BoxFuture, Component, HubKind, IslesCore, Json, PartyId, RealmId, Refusal, SeamError,
};
use rustic_rifts_seam::{
    AuthOutcome, Candidate, CandidateLeave, HuntRouteContext, HuntRouteOutcome, OperationsOverview,
};
use rustic_rifts_seam::{DedicatedOnly, DeletionAction, DeletionStep, Lifecycle, RealmsEntry, RealmsHooks, TableSpec};
use rustic_rifts_seam::{HOST_CAPABILITY_HEADER, RealmEntry, ReleaseGate, Reply, RequestInfo, Settings};
use serde_json::{Value, json};

use crate::{
    RealmsError,
    capabilities::{HostCapabilities, SignedUser, runtime_identity},
    directory::RealmDirectory,
    ids::{seam_realm_id, wire_realm_id},
    leases::Hosting,
    ports::Clock,
    realm::{REALMS_MODULE_SETTING, hub_room_id, realms_module_enabled},
    tiers::{HostingPolicy, relay_only_warning},
};

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

/// What the realms side of the seam answers from; an absent part answers as a dedicated-only server.
pub struct RealmsServiceParts {
    /// The realm directory, when realms run.
    pub directory: Option<Arc<RealmDirectory>>,
    /// Host capabilities, when hosted runtimes are checked.
    pub capabilities: Option<Arc<HostCapabilities>>,
    /// Player hosting, when it runs.
    pub hosting: Option<Arc<Hosting>>,
    /// The time now.
    pub clock: Arc<dyn Clock>,
    /// The module's accounts, parties and matchmaking.
    pub core: Arc<dyn IslesCore>,
    /// The player-hosted policy.
    pub policy: HostingPolicy,
    /// Whether the realms module is on.
    pub realms_enabled: bool,
    /// This server's node, for global ids.
    pub node: NodeId,
}

/// The module's realms hooks and game-session entry, over these parts.
pub struct RealmsService {
    parts: RealmsServiceParts,
}

/// On unless the setting is given: then the realms module switch decides.
pub fn realms_enabled_or_default(settings: &dyn Settings) -> bool {
    let given = settings
        .get(REALMS_MODULE_SETTING)
        .is_some_and(|value| !value.trim().is_empty());
    !given || realms_module_enabled(settings)
}

/// The database's text never reaches a client.
fn seam_error(error: RealmsError) -> SeamError {
    let refused = |refusal| SeamError::Refused(refusal);
    match error {
        RealmsError::Database(_) => refused(Refusal {
            code: "INTERNAL".into(),
            message: "The realms request failed.".into(),
        }),
        RealmsError::Refused(refusal) => refused(refusal),
        RealmsError::Unimplemented(missing) => SeamError::Unimplemented(missing),
    }
}

fn done<'a, T: Send + 'a>(answer: Result<T, SeamError>) -> Fut<'a, T> {
    Box::pin(std::future::ready(answer))
}

fn invalid_capability() -> AuthOutcome {
    let body = r#"{"code":"unauthorized","message":"Invalid host capability.","payload":null}"#;
    AuthOutcome::Refused(Reply {
        status: 401,
        body: Json(body.into()),
    })
}

/// Hooks realms leaves to the dedicated-only answers, declared by signature alone.
macro_rules! dedicated_only {
    ($( fn $hook:ident(&self $(, $arg:ident: $ty:ty)*) -> $answer:ty; )*) => {
        $( fn $hook(&self $(, $arg: $ty)*) -> $answer { DedicatedOnly.$hook($($arg),*) } )*
    };
}

/// (table, account column), deleted in this order.
const DELETION_STEPS: [(&str, &str); 7] = [
    ("rifts_realms", "host_account_id"),
    ("rifts_realm_members", "account_id"),
    ("rifts_realm_bans", "account_id"),
    ("rifts_realm_join_requests", "account_id"),
    ("rifts_realm_player_invites", "account_id"),
    ("rifts_realm_invites", "created_by"),
    ("rifts_realm_presence", "account_id"),
];

/// The metric class of each route family, by path prefix.
const METRIC_CLASSES: [(&str, &str); 4] = [
    ("/launcher/v1/realms", "realms"),
    ("/launcher/v1/realm-hosting", "realms"),
    ("/launcher/v1/p2p", "p2p"),
    ("/rustic/v1/hunts", "p2p"),
];

impl RealmsService {
    /// The service over these parts.
    pub fn new(parts: RealmsServiceParts) -> Self {
        Self { parts }
    }

    /// The seam id of a stored realm id on this node, else the text read as one.
    fn realm_id(&self, wire: &str) -> Option<RealmId> {
        seam_realm_id(wire, &self.parts.node).or_else(|| wire.parse().ok())
    }

    /// Runs `work` on the directory, or answers `absent` without one.
    fn on_directory<'a, T, W, F>(&'a self, absent: T, work: W) -> Fut<'a, T>
    where
        T: Send + 'a,
        W: FnOnce(Arc<RealmDirectory>) -> F,
        F: Future<Output = Result<T, RealmsError>> + Send + 'a,
    {
        let Some(directory) = self.parts.directory.clone() else {
            return done(Ok(absent));
        };
        let pending = work(directory);
        Box::pin(async move { pending.await.map_err(seam_error) })
    }

    /// Player hosting's work, its errors in the seam's terms.
    fn hosted<'a, T>(work: impl Future<Output = Result<T, RealmsError>> + Send + 'a) -> Fut<'a, T> {
        Box::pin(async move { work.await.map_err(seam_error) })
    }
}

impl Lifecycle for RealmsService {
    fn tables(&self) -> &'static [TableSpec] {
        crate::TABLES
    }

    fn account_deletion_steps(&self) -> Vec<DeletionStep> {
        let step = |&(table, account_column): &(&'static str, &'static str)| DeletionStep {
            action: DeletionAction::Delete,
            account_column,
            table,
        };
        DELETION_STEPS.iter().map(step).collect()
    }

    fn startup_summary(&self, _role: &str) -> Result<Option<Json>, SeamError> {
        let RealmsServiceParts {
            policy, realms_enabled, ..
        } = &self.parts;
        let summary = json!({ "p2pHuntsEnabled": policy.enabled, "realmsEnabled": realms_enabled, "trustTier": policy.trust_tier });
        Ok(Some(Json(summary.to_string())))
    }

    fn runtime_safety_issues(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError> {
        Ok(Vec::from_iter(relay_only_warning(settings)))
    }

    /// Starts nothing: no purge sweeper runs (spec-rd27-rifts §7 risk 3).
    fn start(&self, _role: &str) -> Result<(), SeamError> {
        Ok(())
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        done(Ok(()))
    }
}

impl RealmsHooks for RealmsService {
    fn authenticate(&self, request: RequestInfo) -> Fut<'_, AuthOutcome> {
        if request.header(HOST_CAPABILITY_HEADER).is_none() {
            return done(Ok(AuthOutcome::NotMine));
        }
        let Some(capabilities) = self.parts.capabilities.clone() else {
            return done(Ok(invalid_capability()));
        };
        Box::pin(async move {
            let now = self.parts.clock.now();
            let resolved = capabilities
                .resolve_runtime(request.header(HOST_CAPABILITY_HEADER), now)
                .await;
            let auth = resolved.map_err(seam_error)?.and_then(|context| {
                let identity = runtime_identity(&context, SignedUser::Absent, None)?;
                capabilities.runtime_auth(&context, &identity)
            });
            Ok(auth.map_or_else(invalid_capability, AuthOutcome::Authenticated))
        })
    }

    fn metric_class(&self, path: &str) -> Option<&'static str> {
        METRIC_CLASSES
            .iter()
            .find(|(prefix, _)| path.starts_with(prefix))
            .map(|&(_, class)| class)
    }

    fn request_log_tags(&self, _request: &RequestInfo) -> String {
        String::new()
    }

    fn launcher_policy_fields(&self) -> Result<Vec<(String, Json)>, SeamError> {
        let HostingPolicy {
            enabled,
            max_party_size,
            force_sdr,
            trust_tier,
            ..
        } = &self.parts.policy;
        let coop = json!({ "enabled": enabled, "maxPartySize": max_party_size, "forceSdr": force_sdr, "trustTier": trust_tier });
        let field = |name: &str, value: String| (name.to_owned(), Json(value));
        Ok(vec![
            field("p2pEmergencyStop", "false".into()),
            field("coopHunts", coop.to_string()),
        ])
    }

    fn dedicated_hunts_disabled(&self) -> Result<bool, SeamError> {
        Ok(self.parts.policy.dedicated_hunts_disabled)
    }

    fn route_hunt(&self, context: HuntRouteContext) -> Fut<'_, HuntRouteOutcome> {
        let Some(hosting) = self.parts.hosting.clone() else {
            return DedicatedOnly.route_hunt(context);
        };
        Self::hosted(async move {
            // Any error or absence reads as no realm.
            let mut realm = None;
            if let Some(directory) = &self.parts.directory
                && let Ok(Some(row)) = directory.presence_of(&context.player.0).await
            {
                realm = Some(row.realm_id);
            }
            hosting.route_matchmaking_hunt(&context, realm).await
        })
    }

    fn restore_candidate(&self, player: AccountId) -> Fut<'_, Option<Candidate>> {
        match self.parts.hosting.clone() {
            Some(hosting) => Self::hosted(async move { hosting.restore_matchmaking_candidate(&player).await }),
            None => DedicatedOnly.restore_candidate(player),
        }
    }

    fn candidate_status(&self, player: AccountId, current: Candidate) -> Fut<'_, Option<Candidate>> {
        match self.parts.hosting.clone() {
            Some(hosting) => Self::hosted(async move { hosting.poll_candidate_status(&player, &current).await }),
            None => DedicatedOnly.candidate_status(player, current),
        }
    }

    dedicated_only! {
        fn cancel_candidate(&self, player: AccountId, current: Candidate) -> Fut<'_, CandidateLeave>;
        fn party_member_left(&self, account: AccountId) -> Fut<'_, ()>;
        fn party_instance(&self, party: PartyId) -> Fut<'_, Option<Candidate>>;
        fn filters_offline_party_members(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError>;
        fn release_gates(&self) -> Fut<'_, Vec<ReleaseGate>>;
        fn operations_overview(&self) -> Fut<'_, OperationsOverview>;
        fn operations_policy_fields_valid(&self, record: &Json) -> Result<bool, SeamError>;
    }
}

impl RealmsEntry for RealmsService {
    /// With realms off but a directory present it still answers (kept as found, store spec §17 item 12).
    fn hub_room_id(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError> {
        let answers = self.parts.realms_enabled || self.parts.directory.is_some();
        if answers {
            Ok(hub_room_id(&wire_realm_id(realm), kind))
        } else {
            DedicatedOnly.hub_room_id(realm, kind)
        }
    }

    fn is_banned(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool> {
        self.on_directory(false, move |directory| async move {
            directory.is_banned(&wire_realm_id(&realm), &account.0).await
        })
    }

    fn is_member(&self, realm: RealmId, account: AccountId) -> Fut<'_, bool> {
        self.on_directory(false, move |directory| async move {
            directory.is_member(&wire_realm_id(&realm), &account.0).await
        })
    }

    fn find_by_hub_room_id(&self, room: String) -> Fut<'_, Option<RealmId>> {
        self.on_directory(None, move |directory| async move {
            let found = directory.find_by_hub_room_id(&room).await?;
            Ok(found.and_then(|record| self.realm_id(&record.realm_id)))
        })
    }

    fn touch_presence(&self, account: AccountId) -> Fut<'_, ()> {
        self.on_directory((), move |directory| async move {
            directory.touch_presence(&account.0).await.map(drop)
        })
    }

    fn bind_realm_presence_at_login(&self, account: AccountId, realm: Option<RealmId>) -> Fut<'_, ()> {
        let enabled = self.parts.realms_enabled;
        self.on_directory((), move |directory| async move {
            let wire = realm.as_ref().map(wire_realm_id);
            directory
                .bind_presence_at_login(&account.0, wire.as_deref(), enabled)
                .await
        })
    }

    fn authorize_game_session_realm(&self, account: AccountId, realm: Option<Json>) -> Fut<'_, Option<RealmEntry>> {
        let Some(directory) = self.parts.directory.clone() else {
            return DedicatedOnly.authorize_game_session_realm(account, realm);
        };
        // Malformed JSON is refused before anything is read.
        let parsed = match realm.map(|text| serde_json::from_str::<Value>(&text.0)).transpose() {
            Ok(parsed) => parsed,
            Err(_) => {
                let message = "realm must be a JSON object.".to_owned();
                return done(Err(SeamError::Refused(Refusal {
                    code: "REALM_VALIDATION_FAILED".into(),
                    message,
                })));
            }
        };
        let enabled = self.parts.realms_enabled;
        Box::pin(async move {
            let answer = directory
                .authorize_game_session_realm(&account.0, parsed.as_ref(), enabled)
                .await;
            let Some((wire, is_host)) = answer.map_err(seam_error)? else {
                return Ok(None);
            };
            match self.realm_id(&wire) {
                Some(realm) => Ok(Some(RealmEntry { realm, is_host })),
                None => Err(SeamError::unimplemented(
                    Component::Realms,
                    "authorize_game_session_realm",
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rustic_rifts_seam::{NoCore, poll_once};

    use super::*;
    use crate::ports::SystemClock;

    fn service(policy: HostingPolicy, realms_enabled: bool) -> RealmsService {
        let node = NodeId::new("local").unwrap();
        let (clock, core): (Arc<dyn Clock>, Arc<dyn IslesCore>) = (Arc::new(SystemClock), Arc::new(NoCore));
        let parts = RealmsServiceParts {
            directory: None,
            capabilities: None,
            hosting: None,
            clock,
            core,
            policy,
            realms_enabled,
            node,
        };
        RealmsService::new(parts)
    }

    fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|&(k, v)| (k.to_owned(), v.to_owned())).collect()
    }

    #[test]
    fn summary_and_policy_fields_are_byte_exact() {
        let mut policy = HostingPolicy::default();
        (policy.enabled, policy.trust_tier, policy.max_party_size) = (true, 1, 4);
        (policy.force_sdr, policy.dedicated_hunts_disabled) = (true, true);
        let svc = service(policy, false);

        let summary = svc.startup_summary("backend").unwrap().map(|json| json.0);
        assert_eq!(
            summary.as_deref(),
            Some(r#"{"p2pHuntsEnabled":true,"realmsEnabled":false,"trustTier":1}"#)
        );
        let fields = svc.launcher_policy_fields().unwrap();
        let coop = r#"{"enabled":true,"maxPartySize":4,"forceSdr":true,"trustTier":1}"#;
        let expected = [("p2pEmergencyStop", "false"), ("coopHunts", coop)];
        assert_eq!(
            fields,
            expected.map(|(name, value)| (name.to_owned(), Json(value.to_owned())))
        );

        assert_eq!(svc.dedicated_hunts_disabled(), Ok(true));
        assert_eq!(svc.tables(), crate::TABLES);
        let steps = svc.account_deletion_steps();
        assert!(
            steps
                .iter()
                .map(|step| (step.table, step.account_column))
                .eq(DELETION_STEPS)
        );
        assert_eq!(svc.start("backend"), Ok(()));
        assert_eq!(poll_once(svc.stop()), Some(Ok(())));
    }

    #[test]
    fn metric_classes() {
        let svc = service(HostingPolicy::default(), true);
        let cases = [
            ("/launcher/v1/realms/mine", Some("realms")),
            ("/launcher/v1/realm-hosting/status", Some("realms")),
            ("/launcher/v1/p2p/control", Some("p2p")),
            ("/rustic/v1/hunts/x/lease", Some("p2p")),
            ("/launcher/v1/play", None),
        ];
        for (path, class) in cases {
            assert_eq!(svc.metric_class(path), class, "{path}");
        }
        assert!(svc.request_log_tags(&RequestInfo::default()).is_empty());
    }

    #[test]
    fn authenticate_without_capabilities() {
        let svc = service(HostingPolicy::default(), true);
        assert_eq!(
            poll_once(svc.authenticate(RequestInfo::default())),
            Some(Ok(AuthOutcome::NotMine))
        );
        let mut with_header = RequestInfo::default();
        with_header.headers.push((HOST_CAPABILITY_HEADER.into(), "x".into()));
        assert_eq!(poll_once(svc.authenticate(with_header)), Some(Ok(invalid_capability())));
    }

    #[test]
    fn entry_without_a_directory() {
        let off = service(HostingPolicy::default(), false);
        let realm = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse::<RealmId>().unwrap();
        let player = || AccountId(String::from("a"));
        assert_eq!(poll_once(off.touch_presence(player())), Some(Ok(())));
        assert_eq!(poll_once(off.is_member(realm.clone(), player())), Some(Ok(false)));
        assert_eq!(poll_once(off.find_by_hub_room_id(String::from("r"))), Some(Ok(None)));
        assert_eq!(
            off.hub_room_id(&realm, HubKind::Ramsgate),
            DedicatedOnly.hub_room_id(&realm, HubKind::Ramsgate)
        );

        let on = service(HostingPolicy::default(), true);
        let room = hub_room_id(&wire_realm_id(&realm), HubKind::Training);
        assert_eq!(on.hub_room_id(&realm, HubKind::Training), Ok(room));
        let island = HuntRouteContext {
            game_mode: "ISLAND".into(),
            hunt_id: "h".into(),
            player: player(),
            party: None,
            party_members: Vec::new(),
            party_revision: None,
            build_id: None,
            is_hub: false,
        };
        assert_eq!(poll_once(on.route_hunt(island)), Some(Ok(HuntRouteOutcome::NotHandled)));
    }

    #[test]
    fn realms_switch_defaults_on() {
        let switch = |value: &str| settings(&[(REALMS_MODULE_SETTING, value)]);
        assert!(realms_enabled_or_default(&settings(&[])));
        for (value, enabled) in [("  ", true), ("on", true), ("off", false), ("0", false)] {
            assert_eq!(realms_enabled_or_default(&switch(value)), enabled, "{value:?}");
        }
    }

    #[test]
    fn safety_issue_is_the_relay_warning() {
        let svc = service(HostingPolicy::default(), true);
        assert_eq!(
            svc.runtime_safety_issues(&settings(&[("P2P_HUNTS_ENABLED", "true")]))
                .map(|v| v.len()),
            Ok(1)
        );
        assert_eq!(svc.runtime_safety_issues(&settings(&[])), Ok(Vec::new()));
    }
}
