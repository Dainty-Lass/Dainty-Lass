//! Realms (RF5): the realm directory, player-hosted hunts and realm hubs, host capabilities, leases, host selection,
//! hosting tiers and the launcher's control channel, on DaintyLass's `store` (SQLite or Postgres, DD15).
//!
//! Ported from `realms/` and most of `p2p/` (RF1). Each Node repository is a module of [`repo`] over [`db::RealmsDb`];
//! every write goes through `store`'s write path with the actor its change log records (DD23). The pure rules are
//! [`realm`], [`hub`], [`lifecycle`], [`tiers`] and [`host_policy`]; the services are [`directory`], [`capabilities`],
//! [`control`], [`leases`] and [`selection`]. The routes and the `P2PExtension` hooks still answer through [`Stub`].

pub mod capabilities;
pub mod control;
pub mod db;
pub mod directory;
pub mod host_policy;
mod hosting;
pub mod hub;
pub mod hunt;
pub mod ids;
pub mod input;
pub mod leases;
pub mod lifecycle;
pub mod ports;
pub mod realm;
pub mod repo;
mod routes;
pub mod selection;
mod stub;
pub mod tiers;
pub mod time;

pub use hosting::{HostBootstrap, HuntLifecycle, HuntStatus, Observation, StartHub};
pub use routes::{ROUTES, stub_routes};
pub use stub::Stub;

use rifts_seam::{Component, Refusal, TableSpec, Unimplemented};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RealmsError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    /// A `REALM_*` or `P2P_*` launcher error code.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// The database failed (isles answers 500, as Node's unhandled errors).
    #[error("{0}")]
    Database(String),
}

const fn table(name: &'static str, node_collection: &'static str, keys: &'static str) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        owner: Component::Realms,
        keys,
        expires_by: None,
    }
}

const fn expiring(
    name: &'static str,
    node_collection: &'static str,
    keys: &'static str,
    by: &'static str,
) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        owner: Component::Realms,
        keys,
        expires_by: Some(by),
    }
}

/// Realms' tables, as `migrations/` creates them on both engines. `realmHubGates` has none: it serialised hub entries
/// on MongoDB, and store's writes are serial already.
pub const TABLES: &[TableSpec] = &[
    table("rifts_realms", "realms", "realm; name unique; host unique"),
    table("rifts_realm_members", "realms.members", "realm + account"),
    table("rifts_realm_bans", "realms.bans", "realm + account"),
    table("rifts_realm_join_requests", "realms.joinRequests", "realm + account"),
    table("rifts_realm_player_invites", "realms.playerInvites", "realm + account"),
    table("rifts_realm_hub_rooms", "realms.hubRoomIds", "room unique"),
    expiring(
        "rifts_realm_invites",
        "realmInvites",
        "code; realm + created_at",
        "purge_at",
    ),
    expiring(
        "rifts_realm_presence",
        "realmPresence",
        "account; realm + expires_at",
        "expires_at",
    ),
    table(
        "rifts_hunt_sessions",
        "p2pHuntSessions",
        "session; global id unique; active party key unique; state",
    ),
    table("rifts_hunt_roster", "p2pHuntSessions.roster", "session + account"),
    table(
        "rifts_hunt_transitions",
        "p2pHuntTransitions",
        "transition; session + at + state_version",
    ),
    table(
        "rifts_host_capabilities",
        "p2pHostCapabilities",
        "capability; secret digest unique; session + generation",
    ),
    expiring("rifts_host_presences", "p2pHostPresences", "account", "expires_at"),
    table(
        "rifts_host_offers",
        "p2pHostOffers",
        "offer; session + generation + rank; one accepted",
    ),
    table("rifts_host_reliability", "p2pHostReliability", "account"),
    table(
        "rifts_participant_observations",
        "p2pParticipantObservations",
        "observation; session + generation + at",
    ),
    table("rifts_control_sequences", "p2pControlSequences", "account"),
    expiring(
        "rifts_control_messages",
        "p2pControlMessages",
        "message; account + sequence unique",
        "expires_at",
    ),
    expiring(
        "rifts_hub_presence",
        "realmHubPresence",
        "session + account; account / realm + state",
        "purge_at",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_realm_collection_has_a_table() {
        for collection in [
            "realms",
            "realmInvites",
            "realmPresence",
            "p2pHuntSessions",
            "p2pHostCapabilities",
            "p2pControlSequences",
            "p2pControlMessages",
            "p2pHostPresences",
            "p2pHostOffers",
            "p2pHostReliability",
            "p2pParticipantObservations",
            "p2pHuntTransitions",
            "realmHubPresence",
        ] {
            assert!(
                TABLES.iter().any(|table| table.node_collection == collection),
                "{collection}"
            );
        }
    }

    #[test]
    fn the_migrations_create_exactly_the_listed_tables() {
        for sql in [
            include_str!("../migrations/sqlite/2026-10-26-000001_rifts_realms/up.sql"),
            include_str!("../migrations/postgres/2026-10-26-000001_rifts_realms/up.sql"),
        ] {
            let created: Vec<&str> = sql
                .lines()
                .filter_map(|line| line.strip_prefix("CREATE TABLE "))
                .filter_map(|rest| rest.split_whitespace().next())
                .collect();
            let listed: Vec<&str> = TABLES.iter().map(|table| table.name).collect();
            assert_eq!(created, listed);
        }
    }
}
