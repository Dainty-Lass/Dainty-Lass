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
mod wire;

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

#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
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

// Both read as the repository error they become, so the text is the same either way in.
impl From<store::Error> for RealmsError {
    fn from(error: store::Error) -> Self {
        db::RepoError::from(error).into()
    }
}

impl From<diesel::result::Error> for RealmsError {
    fn from(error: diesel::result::Error) -> Self {
        db::RepoError::from(error).into()
    }
}

/// `table => collection, keys [, expires column];`, every one owned by realms.
macro_rules! realm_tables {
    ($($name:literal => $collection:literal, $keys:literal $(, $expires:literal)?;)+) => {
        &[$(TableSpec {
            owner: Component::Realms,
            name: $name,
            node_collection: $collection,
            keys: $keys,
            expires_by: realm_tables!(@expires $($expires)?),
        }),+]
    };
    (@expires) => { None };
    (@expires $column:literal) => { Some($column) };
}

/// In the migrations' `CREATE TABLE` order. Node's `realmHubGates` has no table: `store` serialises writes.
pub const TABLES: &[TableSpec] = realm_tables! {
    "rifts_realms" => "realms", "realm; name unique; host unique";
    "rifts_realm_members" => "realms.members", "realm + account";
    "rifts_realm_bans" => "realms.bans", "realm + account";
    "rifts_realm_join_requests" => "realms.joinRequests", "realm + account";
    "rifts_realm_player_invites" => "realms.playerInvites", "realm + account";
    "rifts_realm_hub_rooms" => "realms.hubRoomIds", "room unique";
    "rifts_realm_invites" => "realmInvites", "code; realm + created_at", "purge_at";
    "rifts_realm_presence" => "realmPresence", "account; realm + expires_at", "expires_at";
    "rifts_hunt_sessions" => "p2pHuntSessions", "session; global id unique; active party key unique; state";
    "rifts_hunt_roster" => "p2pHuntSessions.roster", "session + account";
    "rifts_hunt_transitions" => "p2pHuntTransitions", "transition; session + at + state_version";
    "rifts_host_capabilities" => "p2pHostCapabilities", "capability; secret digest unique; session + generation";
    "rifts_host_presences" => "p2pHostPresences", "account", "expires_at";
    "rifts_host_offers" => "p2pHostOffers", "offer; session + generation + rank; one accepted";
    "rifts_host_reliability" => "p2pHostReliability", "account";
    "rifts_participant_observations" => "p2pParticipantObservations", "observation; session + generation + at";
    "rifts_control_sequences" => "p2pControlSequences", "account";
    "rifts_control_messages" => "p2pControlMessages", "message; account + sequence unique", "expires_at";
    "rifts_hub_presence" => "realmHubPresence", "session + account; account / realm + state", "purge_at";
};

#[cfg(test)]
mod tests {
    use super::*;

    const SQLITE_UP: &str = include_str!("../migrations/sqlite/2026-10-26-000001_rifts_realms/up.sql");
    const POSTGRES_UP: &str = include_str!("../migrations/postgres/2026-10-26-000001_rifts_realms/up.sql");
    const SQLITE_DOWN: &str = include_str!("../migrations/sqlite/2026-10-26-000001_rifts_realms/down.sql");
    const POSTGRES_DOWN: &str = include_str!("../migrations/postgres/2026-10-26-000001_rifts_realms/down.sql");

    /// The first word after `prefix` on each line that starts with it.
    fn named_after<'a>(sql: &'a str, prefix: &str) -> Vec<&'a str> {
        let rest = sql.lines().filter_map(|line| line.strip_prefix(prefix));
        rest.filter_map(|tail| tail.split([' ', ';']).next()).collect()
    }

    #[test]
    fn every_node_collection_has_a_table() {
        let collections = "realms realmInvites realmPresence p2pHuntSessions p2pHostCapabilities p2pControlSequences \
                           p2pControlMessages p2pHostPresences p2pHostOffers p2pHostReliability \
                           p2pParticipantObservations p2pHuntTransitions realmHubPresence";
        for collection in collections.split_whitespace() {
            let found = TABLES.iter().find(|table| table.node_collection == collection);
            assert!(found.is_some(), "{collection}");
        }
        assert!(TABLES.iter().all(|table| table.owner == Component::Realms));
    }

    #[test]
    fn both_migration_sets_create_the_tables_in_order() {
        let names: Vec<&str> = TABLES.iter().map(|table| table.name).collect();
        assert_eq!(named_after(SQLITE_UP, "CREATE TABLE "), names);
        assert_eq!(named_after(POSTGRES_UP, "CREATE TABLE "), names);
        let mut reversed = names.clone();
        reversed.reverse();
        assert_eq!(named_after(SQLITE_DOWN, "DROP TABLE "), reversed);
        assert_eq!(SQLITE_DOWN, POSTGRES_DOWN);
    }

    #[test]
    fn the_engines_differ_only_in_the_name_collation() {
        let code = |sql: &'static str| sql.lines().filter(|line| !line.starts_with("--")).collect::<Vec<_>>();
        let (sqlite, postgres) = (code(SQLITE_UP), code(POSTGRES_UP));
        assert_eq!(sqlite.len(), postgres.len());
        let differing: Vec<_> = sqlite.iter().zip(&postgres).filter(|(a, b)| a != b).collect();
        let collated = (
            &"    name TEXT NOT NULL UNIQUE,",
            &"    name TEXT COLLATE \"C\" NOT NULL UNIQUE,",
        );
        assert_eq!(differing, [collated]);
    }

    #[test]
    fn database_conversions() {
        let cases = [
            (
                RealmsError::from(diesel::result::Error::NotFound),
                "database: Record not found",
            ),
            (
                RealmsError::from(db::RepoError::Duplicate("secret_digest")),
                "duplicate key: secret_digest",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error, RealmsError::Database(text.into()));
        }
    }
}
