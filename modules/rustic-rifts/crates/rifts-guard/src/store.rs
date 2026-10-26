//! Guard's tables (RF1, section 7), shaped for `store` (see `rifts_seam`'s store notes). The private module's
//! evidence, verdict and receipt collections are not in the tree; RF6 adds its own for the evidence chain.

use rifts_seam::{
    AccountId, Actor, Component, HostWriteKind, Json, RealmId, SessionId, StoreError, TableSpec, Unimplemented,
    UnixMillis,
};

pub const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "rifts_hunt_guard",
        node_collection: "p2pHuntSessions.guard*",
        owner: Component::Guard,
        keys: "session + generation",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_traffic_windows",
        node_collection: "guardTrafficWindows",
        owner: Component::Guard,
        keys: "hashed key",
        expires_by: Some("expires_at"),
    },
    TableSpec {
        name: "rifts_host_writes",
        node_collection: "p2pHostWrites",
        owner: Component::Guard,
        keys: "session + generation; target, host, realm by at",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_hunt_xp_summaries",
        node_collection: "p2pHuntXpSummaries",
        owner: Component::Guard,
        keys: "session + member",
        expires_by: Some("ttl_at"),
    },
    TableSpec {
        name: "rifts_operations_alerts",
        node_collection: "operationsAlerts",
        owner: Component::Guard,
        keys: "alert; status + created_at",
        expires_by: None,
    },
];

/// `p2pHostWrites`: one accepted host write, for monitoring and reversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWriteRecord {
    pub kind: HostWriteKind,
    pub target: AccountId,
    pub host: AccountId,
    pub session: SessionId,
    pub generation: u64,
    pub realm: Option<RealmId>,
    pub summary: Json,
    pub at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficWindowRecord {
    pub key_digest: String,
    pub window_start: UnixMillis,
    pub requests: u64,
    pub bytes: u64,
    pub expires_at: UnixMillis,
}

/// `p2pHuntXpSummaries`, about 400 days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XpSummaryRecord {
    pub session: SessionId,
    pub member: AccountId,
    pub host: AccountId,
    pub realm: Option<RealmId>,
    pub minutes: u32,
    pub grants: Json,
    pub flagged: bool,
    pub created_at: UnixMillis,
}

/// `operationsAlerts`; never purged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertRecord {
    pub alert_id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub details: Json,
    pub created_at: UnixMillis,
    pub acknowledged_at: Option<UnixMillis>,
    pub acknowledged_by: Option<String>,
}

pub trait GuardStore: Send + Sync {
    fn record_host_write(&self, actor: &Actor, write: &HostWriteRecord) -> Result<(), StoreError>;
    /// Adds to the window, creating it; the totals after.
    fn charge_window(
        &self,
        actor: &Actor,
        key_digest: &str,
        window_start: UnixMillis,
        bytes: u64,
        expires_at: UnixMillis,
    ) -> Result<TrafficWindowRecord, StoreError>;
    fn put_xp_summary(&self, actor: &Actor, summary: &XpSummaryRecord) -> Result<(), StoreError>;
    fn insert_alert(&self, actor: &Actor, alert: &AlertRecord) -> Result<(), StoreError>;
    fn acknowledge_alert(&self, actor: &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StubGuardStore;

fn unimplemented<T>(operation: &'static str) -> Result<T, StoreError> {
    Err(Unimplemented::new(Component::Guard, operation).into())
}

impl GuardStore for StubGuardStore {
    fn record_host_write(&self, _: &Actor, _: &HostWriteRecord) -> Result<(), StoreError> {
        unimplemented("store.record_host_write")
    }
    fn charge_window(
        &self,
        _: &Actor,
        _: &str,
        _: UnixMillis,
        _: u64,
        _: UnixMillis,
    ) -> Result<TrafficWindowRecord, StoreError> {
        unimplemented("store.charge_window")
    }
    fn put_xp_summary(&self, _: &Actor, _: &XpSummaryRecord) -> Result<(), StoreError> {
        unimplemented("store.put_xp_summary")
    }
    fn insert_alert(&self, _: &Actor, _: &AlertRecord) -> Result<(), StoreError> {
        unimplemented("store.insert_alert")
    }
    fn acknowledge_alert(&self, _: &Actor, _: &str, _: UnixMillis) -> Result<bool, StoreError> {
        unimplemented("store.acknowledge_alert")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_store_refuses() {
        let error = StubGuardStore
            .acknowledge_alert(&Actor::System("test"), "a", UnixMillis(0))
            .unwrap_err();
        assert_eq!(
            error,
            StoreError::Unimplemented(Unimplemented::new(Component::Guard, "store.acknowledge_alert"))
        );
    }
}
