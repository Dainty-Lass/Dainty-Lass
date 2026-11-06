// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! An authored 1.14.7-shaped source (no recorded data) on the mock server, and the options around it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    dead_code,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rustic_isles_db::mongo::MockMongoServer;
use rustic_isles_db::{Area, ExportOptions, LegacyStarter, MigrateOptions};
use serde_json::{Value, json};
use store::{Database, DatabaseConfig, DatabaseOptions};

pub fn starter() -> LegacyStarter {
    LegacyStarter {
        instanced: vec!["WEAPON_STARTER".into(), "ARMOR_HEAD".into()],
        equipment: vec![
            ("weapon".into(), "WEAPON_STARTER".into()),
            ("head".into(), "ARMOR_HEAD".into()),
            ("flask".into(), "FLASK".into()),
        ],
        banner: "BANNER_STARTER".into(),
        glider: "GLIDER_STARTER".into(),
        default_dye: "DYE_DEFAULT".into(),
        default_sigil: "SIGIL_DEFAULT".into(),
    }
}

/// Two players, one character with a loadout and an inventory, one party, and a row of every other area.
pub fn seed(server: &MockMongoServer) {
    let s = |c: &str, docs: Vec<Value>| server.seed(c, docs);
    s(
        "accounts",
        vec![
            json!({"_id": "u1", "name": "Alice", "notes": 3, "displayName": "Alice", "passwordHash": "$argon2id$x",
            "roles": ["user", "admin"], "email": "alice@example.test", "createdAt": "2026-02-01T00:00:00Z"}),
            json!({"_id": "u2", "displayNameNormalized": "bob", "displayName": "Bob", "approvalStatus": "pending",
            "approvalUpdatedAt": 1_772_000_000, "createdAt": {"$date": "2026-02-02T00:00:00Z"}}),
        ],
    );
    s(
        "authIdentities",
        vec![json!({"userId": "u1", "provider": "discord", "providerSubject": "d-1",
        "username": "alice#1", "linkedAt": "2026-02-01T00:00:00Z"})],
    );
    s(
        "refreshSessions",
        vec![json!({"_id": "s1", "userId": "u1", "tokenHash": "token-hash-1",
        "expiresAt": "2026-12-01T00:00:00Z"})],
    );
    s(
        "gameExchangeCodes",
        vec![json!({"codeHash": "code-1", "userId": "u1", "buildChangelist": 647_472})],
    );
    s(
        "discordOAuthTransactions",
        vec![json!({"state": "st-1", "codeVerifier": "v", "userId": "u1"})],
    );
    s(
        "emailCodes",
        vec![
            json!({"userId": "u1", "purpose": "verify_email", "codeHash": "email-hash", "attempts": 2,
        "expiresAt": "2099-01-01T00:00:00Z"}),
        ],
    );
    s("gameServerApiKeys", vec![json!({"keyHash": "gs-key-1"})]);
    s("userApiKeys", vec![json!({"userId": "u1", "keyHash": "user-key-1"})]);
    s(
        "playerJourney",
        vec![json!({"userId": "u1", "nodes": {"LoadoutSlot_01": {"node_status": 2},
        "LoadoutSlot_02": {"node_status": 2}, "LoadoutSlot_03": {"node_status": 1}}, "updateVersion": 4})],
    );
    s(
        "progressionTracks",
        vec![json!({"userId": "u1", "progressionId": "TRACK_1", "progress": 120,
        "reforge": {"level": 2}, "prestigeClaimed": true})],
    );
    s(
        "progressionObjectives",
        vec![json!({"userId": "u1", "objectiveId": "OBJ_1", "value": 7})],
    );
    s(
        "progressionObjectiveEvents",
        vec![json!({"_id": "ev-1", "userId": "u1", "body": "{ \"a\": 1 }"})],
    );
    s(
        "progressionTransactions",
        vec![json!({"grantId": "g-1", "userId": "u1", "progressionId": "TRACK_1",
        "amount": 50, "result": {"userId": "u1", "ok": true}})],
    );
    s(
        "escalationProgress",
        vec![json!({"userId": "u1", "seasonId": "S1", "escalationLevel": 3,
        "talentsProgress": [1, 2]})],
    );
    s(
        "breadcrumbs",
        vec![json!({"characterId": "c1", "userId": "u1", "breadcrumbs": ["b"]})],
    );
    s(
        "encounteredContent",
        vec![json!({"characterId": "c1", "userId": "u1", "encounteredcontent": ["x"]})],
    );
    s(
        "characters",
        vec![
            json!({"_id": "c1", "userId": "u1", "name": "Hero", "data": {"appearance": {"a": 1}},
        "createdDate": "2026-02-03T00:00:00Z"}),
        ],
    );
    s(
        "inventories",
        vec![json!({"characterId": "c1", "userId": "u1", "instancedItems": [
        {"catalogId": "WEAPON_STARTER", "instanceId": "bad-id"},
        {"catalogId": "STACK_ME", "instanceId": "AAAAAAAAAAAAAAAAAAAAAAAAAA", "quantity": 1}
    ], "stackedItems": []})],
    );
    s(
        "loadouts",
        vec![json!({"characterId": "c1", "userId": "u1", "activeIndex": 99,
        "loadouts": [{"slot_index": 0, "custom_name": "Main", "weapon": {"instance_id": "stale"}, "flask": "FLASK"}],
        "persistent": {}})],
    );
    s(
        "inventoryTransactions",
        vec![json!({"userId": "u1", "characterId": "c1", "transactionId": "t-1",
        "requestHash": "rh", "__v": 0, "updatedAt": "2026-02-04T00:00:00Z"})],
    );
    s(
        "bountyStates",
        vec![json!({"userId": "u1", "bounties": [{"id": "B1"}], "updateVersion": 2})],
    );
    s("cooldownStates", vec![json!({"userId": "u1", "cooldowns": {"X": 1}})]);
    s(
        "entitlements",
        vec![
            json!({"userId": "u1", "entitlements": [{"name": "ENT_A", "duration": 30},
        "junk", {"name": "ENT_B", "duration": "inf"}]}),
        ],
    );
    s(
        "wallets",
        vec![json!({"userId": "u1", "balances": {"CUR_A": 100, "skip": "x", "CUR_B": 2.5}})],
    );
    s(
        "economyDaily",
        vec![json!({"userId": "u1", "day": "2026-03-01", "currency": "CUR_A",
        "earned": {"hunts": 3, "bounties": 2}, "label": "dropped", "ttlAt": 1_772_400_000_000_i64, "__v": 0})],
    );
    s(
        "partyMemberships",
        vec![json!({"userId": "u2", "partyId": "p1", "joinedAt": "2026-02-05T00:00:00Z"})],
    );
    s(
        "parties",
        vec![
            json!({"partyId": "p1", "leaderPlayerId": "u1", "members": ["u1", 7, "u2"],
        "createdAt": "2026-02-05T00:00:00Z"}),
        ],
    );
    s(
        "partyInvites",
        vec![json!({"recipientPlayerId": "u2", "sendingPlayerId": "u1", "partyId": "p1"})],
    );
    s(
        "partyOutbox",
        vec![json!({"eventId": "ev-out-1", "partyId": "p1", "recipients": ["u2"]})],
    );
    s(
        "friendships",
        vec![json!({"ownerId": "u1", "otherId": "u2", "favorite": "TRUE", "direction": "OUT"})],
    );
    s(
        "reforgedInvites",
        vec![json!({"invitecode": "INV-1", "usesRemaining": 3})],
    );
    s(
        "adminSessions",
        vec![json!({"_id": "as-1", "userId": "u1", "tokenHash": "admin-token",
        "expiresAt": "2026-03-01T10:00:00Z"})],
    );
    s(
        "adminAudit",
        vec![json!({"_id": "aa-1", "actorUserId": "u1", "action": "BAN", "oldState": {"x": 1}})],
    );
    s(
        "launcherNotices",
        vec![
            json!({"version": 4, "announcements": [{"text": "hi"}]}),
            json!({"version": 9}),
        ],
    );
    s(
        "operationsPolicy",
        vec![json!({"guardEnforcement": "OBSERVE", "version": 2})],
    );
    s(
        "launcherGuardSessions",
        vec![json!({"guardSessionId": "gs-1", "accountId": "u1", "role": "host",
        "riskScore": 0.25, "challenge": "guard-challenge", "reasons": ["r"]})],
    );
    s(
        "guardFindings",
        vec![json!({"fileName": "x.dll", "sha256": "abc", "accountIds": ["u1", "", 5]})],
    );
}

pub fn sqlite(dir: &Path) -> PathBuf {
    dir.join("isles.db")
}

pub fn migrate_options(server: &MockMongoServer, target: &str) -> MigrateOptions {
    MigrateOptions {
        from_uri: server.uri("dauntless"),
        to_target: target.into(),
        node: "local".parse().unwrap(),
        dry_run: false,
        areas: BTreeSet::new(),
        skip_invalid: false,
        game_data_dir: None,
        allow_remote_postgres: false,
        starter: starter(),
    }
}

pub fn export_options(server: &MockMongoServer, source: &str, areas: &[Area]) -> ExportOptions {
    ExportOptions {
        from_source: source.into(),
        to_uri: server.uri("dauntless"),
        areas: areas.iter().copied().collect(),
        dry_run: false,
        allow_remote_postgres: false,
    }
}

pub fn open(file: &Path) -> Database {
    Database::open(
        &DatabaseConfig::Sqlite {
            file: file.to_path_buf(),
        },
        &DatabaseOptions::default(),
        &rustic_isles_store::SCHEMA,
    )
    .unwrap()
}
