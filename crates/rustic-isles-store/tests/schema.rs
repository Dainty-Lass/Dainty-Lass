// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The schema facts an existing database already has (spec §4): versions, the column order and the seed row.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use diesel::QueryableByName;
use diesel::RunQueryDsl;
use diesel::sql_types::Text;

#[derive(QueryableByName)]
struct Name {
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(QueryableByName)]
struct Version {
    #[diesel(sql_type = Text)]
    version: String,
}

#[tokio::test]
async fn versions_columns_and_seed() {
    let t = common::targets().remove(0);
    let (versions, columns, indexes) = t
        .store
        .read(|conn, _| {
            let versions = diesel::sql_query("SELECT version FROM __diesel_schema_migrations ORDER BY version")
                .load::<Version>(conn)?;
            let columns = diesel::sql_query("SELECT name FROM pragma_table_info('launcher_accounts') ORDER BY cid")
                .load::<Name>(conn)?;
            let indexes = diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .load::<Name>(conn)?;
            Ok((versions, columns, indexes))
        })
        .await
        .unwrap();
    let versions: Vec<String> = versions.into_iter().map(|v| v.version).collect();
    assert_eq!(
        versions,
        [
            "0001", "0020", "0030", "0050", "0055", "0070", "0200", "0201", "0202", "0203", "0204", "0205", "0500",
            "0600", "0610", "0620", "0640"
        ]
    );
    let columns: Vec<String> = columns.into_iter().map(|c| c.name).collect();
    assert_eq!(columns.len(), 27);
    assert_eq!(
        &columns[22..],
        [
            "email_verification_pending_since",
            "deleted_at",
            "timeout_until",
            "timeout_since",
            "timeout_reason"
        ]
    );
    let indexes: Vec<String> = indexes.into_iter().map(|i| i.name).collect();
    for wanted in [
        "launcher_accounts_email",
        "launcher_accounts_display_name",
        "auth_identities_account",
        "refresh_sessions_token_hash",
        "refresh_sessions_family",
        "refresh_sessions_account",
        "game_exchange_codes_account",
        "discord_oauth_completion",
        "user_api_keys_hash",
        "characters_account",
        "loadouts_account",
        "inventories_account",
        "inventory_transactions_account",
        "friend_edges_other",
        "admin_audit_target",
        "launcher_guard_sessions_account",
        "launcher_guard_sessions_created",
        "player_activity_daily_day",
        "activity_hunts_account_started",
        "slayers_path_unlocks_account_unlocked",
        "activity_flags_created",
        "activity_flags_account_created",
        "progression_grants_account",
        "party_members_party",
        "party_outbox_state",
    ] {
        assert!(indexes.iter().any(|i| i == wanted), "{wanted}");
    }
    let policy = t
        .store
        .read(|conn, r| r.admin.read_operations_policy(conn))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            policy.guard_enforcement.as_str(),
            policy.diagnostics_profile.as_str(),
            policy.version
        ),
        ("ENFORCE", "PRODUCTION", 0)
    );
}

#[test]
fn both_engines_have_the_same_files_but_identity_keys() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let list = |engine: &str| {
        let mut names: Vec<String> = std::fs::read_dir(root.join(engine))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(list("sqlite"), list("postgres"));
    for folder in list("sqlite") {
        for file in ["up.sql", "down.sql"] {
            let sqlite = std::fs::read_to_string(root.join("sqlite").join(&folder).join(file)).unwrap();
            let postgres = std::fs::read_to_string(root.join("postgres").join(&folder).join(file)).unwrap();
            let unified = postgres.replace(
                "BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY",
                "INTEGER PRIMARY KEY AUTOINCREMENT",
            );
            assert_eq!(sqlite, unified, "{folder}/{file}");
        }
    }
}
