// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Rifts' realms: player-owned realms and their directory, realm hubs, the player-hosted hunt lifecycle, the
//! launcher's control channel, and the store under all of them (one database with Guard's, on `store`).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod capabilities;
pub mod control;
pub mod db;
pub mod directory;
pub mod host_policy;
pub mod hub;
pub mod hunt;
pub mod ids;
pub mod input;
pub mod leases;
pub mod lifecycle;
pub mod ports;
pub mod realm;
pub mod repo;
pub mod selection;
pub mod service;
pub mod tiers;
pub mod time;

mod hosting;
mod routes;
mod stub;

use rifts_seam::{Component, Refusal, TableSpec, Unimplemented};

pub use crate::capabilities::RealmsHuntSessions;
pub use crate::hosting::{HostBootstrap, HuntLifecycle, HuntStatus, Observation, StartHub, resolve_manifest_for_hunt};
pub use crate::leases::HostingRuntime;
pub use crate::lifecycle::RealmsRecovery;
pub use crate::ports::{CoreAccounts, CoreGuardSessions, CoreOperationsView};
pub use crate::routes::{ROUTES, RealmsRoutes, RealmsRoutesParts, stub_routes};
pub use crate::service::{RealmsService, RealmsServiceParts, realms_enabled_or_default};
pub use crate::stub::Stub;
pub use rifts_guard::SqlGuardStore;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RealmsError {
    /// The service behind the route is absent: a 501.
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    /// A launcher error code (`REALM_*`, `P2P_*`, `AUTH_*`).
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// Never shown to a client: the routes answer `INTERNAL`.
    #[error("{0}")]
    Database(String),
}

impl From<db::RepoError> for RealmsError {
    fn from(error: db::RepoError) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<store::Error> for RealmsError {
    fn from(error: store::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<diesel::result::Error> for RealmsError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(format!("database: {error}"))
    }
}

const fn table(
    name: &'static str,
    node_collection: &'static str,
    keys: &'static str,
    expires_by: Option<&'static str>,
) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        owner: Component::Realms,
        keys,
        expires_by,
    }
}

/// In the migrations' `CREATE TABLE` order. Node's `realmHubGates` has no table: `store` serialises writes.
pub const TABLES: &[TableSpec] = &[
    table("rifts_realms", "realms", "realm; name unique; host unique", None),
    table("rifts_realm_members", "realms.members", "realm + account", None),
    table("rifts_realm_bans", "realms.bans", "realm + account", None),
    table(
        "rifts_realm_join_requests",
        "realms.joinRequests",
        "realm + account",
        None,
    ),
    table(
        "rifts_realm_player_invites",
        "realms.playerInvites",
        "realm + account",
        None,
    ),
    table("rifts_realm_hub_rooms", "realms.hubRoomIds", "room unique", None),
    table(
        "rifts_realm_invites",
        "realmInvites",
        "code; realm + created_at",
        Some("purge_at"),
    ),
    table(
        "rifts_realm_presence",
        "realmPresence",
        "account; realm + expires_at",
        Some("expires_at"),
    ),
    table(
        "rifts_hunt_sessions",
        "p2pHuntSessions",
        "session; global id unique; active party key unique; state",
        None,
    ),
    table("rifts_hunt_roster", "p2pHuntSessions.roster", "session + account", None),
    table(
        "rifts_hunt_transitions",
        "p2pHuntTransitions",
        "transition; session + at + state_version",
        None,
    ),
    table(
        "rifts_host_capabilities",
        "p2pHostCapabilities",
        "capability; secret digest unique; session + generation",
        None,
    ),
    table(
        "rifts_host_presences",
        "p2pHostPresences",
        "account",
        Some("expires_at"),
    ),
    table(
        "rifts_host_offers",
        "p2pHostOffers",
        "offer; session + generation + rank; one accepted",
        None,
    ),
    table("rifts_host_reliability", "p2pHostReliability", "account", None),
    table(
        "rifts_participant_observations",
        "p2pParticipantObservations",
        "observation; session + generation + at",
        None,
    ),
    table("rifts_control_sequences", "p2pControlSequences", "account", None),
    table(
        "rifts_control_messages",
        "p2pControlMessages",
        "message; account + sequence unique",
        Some("expires_at"),
    ),
    table(
        "rifts_hub_presence",
        "realmHubPresence",
        "session + account; account / realm + state",
        Some("purge_at"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_node_collection_has_a_table() {
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
            assert!(TABLES.iter().any(|t| t.node_collection == collection), "{collection}");
        }
        assert!(TABLES.iter().all(|t| t.owner == Component::Realms));
    }

    fn created_tables(sql: &str) -> Vec<&str> {
        sql.lines()
            .filter_map(|line| line.strip_prefix("CREATE TABLE "))
            .filter_map(|rest| rest.split(' ').next())
            .collect()
    }

    #[test]
    fn both_migration_sets_create_the_tables_in_order() {
        let names: Vec<&str> = TABLES.iter().map(|t| t.name).collect();
        for sql in [
            include_str!("../migrations/sqlite/2026-10-26-000001_rifts_realms/up.sql"),
            include_str!("../migrations/postgres/2026-10-26-000001_rifts_realms/up.sql"),
        ] {
            assert_eq!(created_tables(sql), names);
        }
        let dropped: Vec<&str> = include_str!("../migrations/sqlite/2026-10-26-000001_rifts_realms/down.sql")
            .lines()
            .filter_map(|line| line.strip_prefix("DROP TABLE "))
            .map(|rest| rest.trim_end_matches(';'))
            .collect();
        assert_eq!(dropped, names.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(
            include_str!("../migrations/sqlite/2026-10-26-000001_rifts_realms/down.sql"),
            include_str!("../migrations/postgres/2026-10-26-000001_rifts_realms/down.sql")
        );
    }

    #[test]
    fn the_engines_differ_only_in_the_name_collation() {
        let sqlite = include_str!("../migrations/sqlite/2026-10-26-000001_rifts_realms/up.sql");
        let postgres = include_str!("../migrations/postgres/2026-10-26-000001_rifts_realms/up.sql");
        let code = |sql: &'static str| -> Vec<&'static str> { sql.lines().filter(|l| !l.starts_with("--")).collect() };
        let (a, b) = (code(sqlite), code(postgres));
        assert_eq!(a.len(), b.len());
        let differing: Vec<(&str, &str)> = a.into_iter().zip(b).filter(|(x, y)| x != y).collect();
        assert_eq!(
            differing,
            [(
                "    name TEXT NOT NULL UNIQUE,",
                "    name TEXT COLLATE \"C\" NOT NULL UNIQUE,"
            )]
        );
    }

    #[test]
    fn database_conversions() {
        assert_eq!(
            RealmsError::from(diesel::result::Error::NotFound),
            RealmsError::Database("database: Record not found".into())
        );
        assert_eq!(
            RealmsError::from(db::RepoError::Duplicate("secret_digest")),
            RealmsError::Database("duplicate key: secret_digest".into())
        );
    }
}
