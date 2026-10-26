//! Guard (RF6): observe mode on stubs, since Mystic's anticheat module is private (DD14).
//!
//! Ported from `p2p/antiCheat.ts` (the stub side), `p2p/security/hostWrites.ts`, `hubAuthorization.ts`,
//! `p2pInventoryAuthorization.ts`, `hostPolicyMode.ts`, `guardTraffic*.ts`, the host route census and the operations
//! alerts (RF1). This crate answers [`rifts_seam::GuardHooks`].
//!
//! Launcher Guard (`/launcher/v1/guard/*`, `/admin/v1/guard/*`) and the Guard manifests stay isles' (RF1, decision 1):
//! every official Play uses them, so a 1.14.7 module built without rifts must keep them. Realms read them through
//! [`rifts_seam::IslesCore`], and `MYSTICPARADOX_GUARD_MODE` goes with them.

mod routes;
mod store;

pub use routes::{ROUTES, stub_routes};
pub use store::{
    AlertRecord, GuardStore, HostWriteRecord, StubGuardStore, TABLES, TrafficWindowRecord, XpSummaryRecord,
};

use rifts_seam::{
    AccountId, BoxFuture, Component, DeletionStep, GuardHooks, HostWrite, HostWriteKind, HostWriteOptions,
    InventoryDecision, InventoryTransaction, Json, Lifecycle, Reply, RequestInfo, RuntimeAuth, SeamError, Secret,
    SessionId, Settings, TableSpec, Unimplemented, stub,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GuardError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
}

/// `MYSTICPARADOX_HOST_POLICY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostPolicyMode {
    #[default]
    Observe,
    Enforce,
}

/// `P2P_TRUST_TIER` (RF1, decision 4 puts it here): 0, or 1 for evidence and Tier 1 rewards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrustTier {
    #[default]
    Tier0,
    Tier1,
}

/// `HuntVerdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HuntVerdict {
    pub progression_applied: bool,
    pub rewards_applied: bool,
}

/// The anticheat module's `hunts.*`: the evidence chain intake RF6 builds, in observe mode.
pub trait EvidenceIntake: Send + Sync {
    /// `POST .../event-batches`; the stub module answers `HUNT_EVIDENCE_NOT_COLLECTED`.
    fn submit_evidence(&self, runtime: RuntimeAuth, batch: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    /// `POST .../completion`.
    fn complete(&self, runtime: RuntimeAuth, request: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    /// `hunts.result`, shown by the hunt status route.
    fn verdict(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>>;
    /// `hunts.rewardReceipt`, for `GET .../reward/self`.
    fn reward_receipt(
        &self,
        session: SessionId,
        generation: u64,
        account: AccountId,
    ) -> BoxFuture<'_, Result<Option<Json>, GuardError>>;
    /// `POST /internal/v1/hunt-guard/shadow-comparisons`; the key is `x-hunt-guard-shadow-key`.
    fn record_shadow_comparison(
        &self,
        key: Option<Secret>,
        comparison: Json,
    ) -> BoxFuture<'_, Result<Json, GuardError>>;
}

/// What a request or its bytes are charged to (`guardTraffic.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrafficScope {
    Host(AccountId),
    Session(SessionId, u64),
    RequestBudget(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficDecision {
    Accepted,
    Refused { retry_after_ms: u64 },
}

/// The Guard traffic limits, local or distributed (`P2P_DISTRIBUTED_TRAFFIC_LIMITER`).
pub trait TrafficLimits: Send + Sync {
    fn charge(&self, scope: TrafficScope, bytes: u64) -> BoxFuture<'_, Result<TrafficDecision, GuardError>>;
}

/// Guard's admin operations routes.
pub trait GuardOperations: Send + Sync {
    /// `UpdateRewardRollout` (Tier 1 grants).
    fn set_reward_rollout(&self, admin_key: String, rollout: Json) -> BoxFuture<'_, Result<Json, GuardError>>;
    /// `AcknowledgeOperationsAlert`.
    fn acknowledge_alert(&self, admin_key: String, alert_id: String) -> BoxFuture<'_, Result<(), GuardError>>;
}

/// Every operation answers [`Unimplemented`] until RF6 (RD18).
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Guard;

/// `launcher.guardManifestRoles`: the artifacts realms add to the signed manifest.
pub const GUARD_MANIFEST_ROLES: &[&str] = &["TRANSPORT_MODULE", "TRANSPORT_RUNTIME", "ADMISSION_KEY"];

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

impl GuardHooks for Stub {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        GUARD_MANIFEST_ROLES
    }
    fn census(&self, _: &RequestInfo) -> Result<(), SeamError> {
        refuse("census")
    }
    fn rate_limit_bucket(&self, _: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        stub(COMPONENT, "rate_limit_bucket")
    }
    fn authorize_inventory(
        &self,
        _: RuntimeAuth,
        _: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        stub(COMPONENT, "authorize_inventory")
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        _: HostWriteKind,
        _: AccountId,
        _: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        stub(COMPONENT, "authorize_host_write")
    }
    fn record_host_write(&self, _: RuntimeAuth, _: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        stub(COMPONENT, "record_host_write")
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        refuse("host_policy_refuses")
    }
    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), SeamError> {
        refuse("note_would_refuse")
    }
}

impl EvidenceIntake for Stub {
    fn submit_evidence(&self, _: RuntimeAuth, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        stub(COMPONENT, "submit_evidence")
    }
    fn complete(&self, _: RuntimeAuth, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        stub(COMPONENT, "complete")
    }
    fn verdict(&self, _: SessionId, _: u64) -> BoxFuture<'_, Result<Option<HuntVerdict>, GuardError>> {
        stub(COMPONENT, "verdict")
    }
    fn reward_receipt(&self, _: SessionId, _: u64, _: AccountId) -> BoxFuture<'_, Result<Option<Json>, GuardError>> {
        stub(COMPONENT, "reward_receipt")
    }
    fn record_shadow_comparison(&self, _: Option<Secret>, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        stub(COMPONENT, "record_shadow_comparison")
    }
}

impl TrafficLimits for Stub {
    fn charge(&self, _: TrafficScope, _: u64) -> BoxFuture<'_, Result<TrafficDecision, GuardError>> {
        stub(COMPONENT, "charge")
    }
}

impl GuardOperations for Stub {
    fn set_reward_rollout(&self, _: String, _: Json) -> BoxFuture<'_, Result<Json, GuardError>> {
        stub(COMPONENT, "set_reward_rollout")
    }
    fn acknowledge_alert(&self, _: String, _: String) -> BoxFuture<'_, Result<(), GuardError>> {
        stub(COMPONENT, "acknowledge_alert")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::poll_once;

    fn guard(operation: &'static str) -> GuardError {
        Unimplemented::new(COMPONENT, operation).into()
    }

    #[test]
    fn hooks_answer_unimplemented() {
        assert_eq!(Stub.host_policy_refuses(), refuse("host_policy_refuses"));
        assert_eq!(Stub.census(&RequestInfo::default()), refuse("census"));
        assert_eq!(
            poll_once(Stub.rate_limit_bucket(RequestInfo::default())).unwrap(),
            refuse::<Option<String>>("rate_limit_bucket")
        );
        assert_eq!(Stub.guard_manifest_roles(), GUARD_MANIFEST_ROLES);
    }

    #[test]
    fn surfaces_answer_unimplemented() {
        let session: SessionId = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap();
        assert_eq!(
            poll_once(Stub.verdict(session.clone(), 1)).unwrap(),
            Err(guard("verdict"))
        );
        assert_eq!(
            poll_once(Stub.charge(TrafficScope::Session(session, 1), 10)).unwrap(),
            Err(guard("charge"))
        );
        assert_eq!(
            poll_once(Stub.acknowledge_alert("key".to_owned(), "alert".to_owned())).unwrap(),
            Err(guard("acknowledge_alert"))
        );
    }
}
