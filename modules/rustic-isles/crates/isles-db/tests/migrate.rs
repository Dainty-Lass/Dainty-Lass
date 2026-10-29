#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use isles_core::repo::auth::EmailCodePurpose;
use isles_core::repo::inventory::LedgerKey;
use isles_core::repo::{Repositories, UserId};
use isles_db::migrate::{DbError, MigrateArea, MigrateOptions, open_target_database, run_migrate};
use isles_db::mongo::{BsonBuf, MockMongoServer, MongoUri, decode_bson_doc};
use isles_db::transforms::deterministic_account_id;
use isles_db::verify::run_verify;
use isles_db::{Command, ExportOptions, parse_args, run_export_mongo};
use module_api::NodeId;
use serde_json::{Map, Value, json};

fn obj(val: Value) -> Map<String, Value> {
    match val {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn sample_mongo_collections() -> BTreeMap<String, Vec<Map<String, Value>>> {
    let mut cols: BTreeMap<String, Vec<Map<String, Value>>> = BTreeMap::new();

    // 1. Auth collections
    cols.insert(
        "accounts".into(),
        vec![
            obj(json!({
                "_id": "11111111-1111-4111-8111-111111111111",
                "userId": "11111111-1111-4111-8111-111111111111",
                "name": "SlayerOne",
                "notes": 5000,
                "email": "slayer1@example.com",
                "displayName": "SlayerOne",
                "displayNameNormalized": "slayerone",
                "passwordHash": "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA",
                "status": "active",
                // Intentionally omit approvalStatus to test RB11 migrate_launcher_approval
                "roles": ["user", "admin"],
                "createdAt": "2026-03-01T10:00:00.000Z",
                "lastLoginAt": "2026-03-10T12:00:00.000Z",
                "usernameSet": 1,
                "consent": {
                    "termsVersion": "1.0",
                    "privacyVersion": "1.0",
                    "acceptedAt": "2026-03-01T10:00:00.000Z"
                },
                "emailVerifiedAt": "2026-03-01T10:05:00.000Z"
            })),
            obj(json!({
                "_id": "22222222-2222-4222-8222-222222222222",
                "userId": "22222222-2222-4222-8222-222222222222",
                "name": "SlayerTwo",
                "notes": 1200,
                "displayName": "SlayerTwo",
                "displayNameNormalized": "slayertwo",
                "status": "active",
                "approvalStatus": "pending",
                "approvalUpdatedAt": "2026-03-02T08:00:00.000Z",
                "approvalUpdatedBy": "system:signup",
                "roles": ["user"],
                "createdAt": "2026-03-02T08:00:00.000Z"
            })),
        ],
    );

    cols.insert(
        "authIdentities".into(),
        vec![obj(json!({
            "_id": "discord:999888777",
            "userId": "11111111-1111-4111-8111-111111111111",
            "provider": "discord",
            "providerSubject": "999888777",
            "providerUsername": "slayer_one_discord",
            "providerAvatarUrl": "https://cdn.example.com/avatar.png",
            "linkedAt": "2026-03-01T10:01:00.000Z"
        }))],
    );

    cols.insert(
        "refreshSessions".into(),
        vec![obj(json!({
            "_id": "sess-001",
            "sessionId": "sess-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "tokenHash": "th_sess_001",
            "familyId": "fam-001",
            "deviceId": "dev-001",
            "deviceName": "Desktop-PC",
            "createdAt": "2026-03-10T12:00:00.000Z",
            "expiresAt": "2026-04-10T12:00:00.000Z",
            "revokedAt": null
        }))],
    );

    cols.insert(
        "gameExchangeCodes".into(),
        vec![obj(json!({
            "_id": "gec_hash_001",
            "codeHash": "gec_hash_001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "launcherSessionId": "sess-001",
            "buildChangelist": 647472,
            "executableSha256": "exe_sha_123",
            "runtimeChannel": "stable",
            "runtimeSha256": "rt_sha_123",
            "runtimeManifestVersion": "1.14.7-r1",
            "runtimeArtifactSetSha256": "art_sha_123",
            "realmId": "realm-alpha",
            "createdAt": "2026-03-10T12:01:00.000Z",
            "expiresAt": "2026-03-10T12:06:00.000Z",
            "consumedAt": "2026-03-10T12:01:15.000Z"
        }))],
    );

    cols.insert(
        "discordOAuthTransactions".into(),
        vec![obj(json!({
            "_id": "oauth_state_001",
            "state": "oauth_state_001",
            "codeVerifier": "pkce_verifier_secret_001",
            "createdAt": "2026-03-01T09:59:00.000Z",
            "expiresAt": "2026-03-01T10:09:00.000Z",
            "consumedAt": "2026-03-01T10:00:30.000Z",
            "userId": "11111111-1111-4111-8111-111111111111"
        }))],
    );

    cols.insert(
        "emailCodes".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111:verify_email",
            "userId": "11111111-1111-4111-8111-111111111111",
            "purpose": "verify_email",
            "codeHash": "email_code_hmac_001",
            "attempts": 1,
            "createdAt": "2026-03-01T10:00:00.000Z",
            "expiresAt": "2026-03-01T10:15:00.000Z"
        }))],
    );

    cols.insert(
        "gameServerApiKeys".into(),
        vec![obj(json!({ "_id": "gs_key_hash_001", "keyHash": "gs_key_hash_001" }))],
    );

    cols.insert(
        "userApiKeys".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111",
            "userId": "11111111-1111-4111-8111-111111111111",
            "keyHash": "user_key_hash_001"
        }))],
    );

    // 2. Characters & Loadouts (with RB11 starter instance repair + loadout slot reconciliation)
    cols.insert(
        "characters".into(),
        vec![obj(json!({
            "_id": "char-001",
            "characterId": "char-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "createdDate": "2026-03-01T10:10:00.000Z",
            "lastModifiedDate": "2026-03-10T11:00:00.000Z",
            "name": "SlayerOne",
            "updateVersion": 3,
            "data": { "primary_dye": "DYE_01" }
        }))],
    );

    cols.insert(
        "loadouts".into(),
        vec![obj(json!({
            "_id": "char-001",
            "characterId": "char-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "loadouts": [
                {
                    "slot_index": 0,
                    "custom_name": "Main Sword",
                    "update_version": 2,
                    "weapon": { "instance_id": "bad-id", "item_id": "WP_EB_BEGINNER" },
                    "helmet": { "instance_id": "bad-helm", "item_id": "AR_UNEQUIPPED_HELM" },
                    "chest": { "instance_id": "bad-chest", "item_id": "AR_BEGINNER_CHEST" },
                    "arms": { "instance_id": "bad-arms", "item_id": "AR_BEGINNER_ARMS" },
                    "legs": { "instance_id": "bad-legs", "item_id": "AR_BEGINNER_LEGS" },
                    "lantern": { "instance_id": "bad-lantern", "item_id": "LT_BASIC" },
                    "flask": { "instance_id": "bad-flask", "item_id": "FL_HEALING_DEFAULT" }
                }
            ],
            "persistent": {
                "banner": { "fabric_id": "", "pole_id": "" },
                "glider": { "instance_id": "" }
            },
            "activeIndex": 99,
            "unlockedTotalSlots": 1,
            "revision": 4,
            "bootstrapVersion": "starter-1.14-v1"
        }))],
    );

    // 3. Inventory & Inventory Transactions (with RB11 storage policy migration)
    cols.insert(
        "inventories".into(),
        vec![obj(json!({
            "_id": "char-001",
            "characterId": "char-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "instancedItems": [
                { "instanceId": "invalid-short", "catalogId": "WP_EB_BEGINNER", "durability": 100 },
                { "instanceId": "0123456789ABCDEFGHIJKLMNOP", "catalogId": "STACKABLE_CELL_01", "durability": 100 }
            ],
            "stackedItems": [
                { "catalogId": "QI_BASIC_FLARE_DURABLE", "quantity": 1 }
            ],
            "revision": 7,
            "bootstrapVersion": "starter-1.14-v1"
        }))],
    );

    cols.insert(
        "inventoryTransactions".into(),
        vec![obj(json!({
            "_id": "tx_hash_001",
            "transactionId": "tx-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "characterId": "char-001",
            "requestHash": "req_hash_001",
            "status": "completed",
            "result": { "ok": true },
            "createdAt": "2026-03-05T12:00:00.000Z",
            "completedAt": "2026-03-05T12:00:01.000Z",
            "economySource": "HUNT",
            "economySessionId": "hunt-sess-01"
        }))],
    );

    // 4. Progression (with 2 unlocked LoadoutSlot nodes in playerJourney -> total 3 loadout slots)
    cols.insert(
        "playerJourney".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111",
            "userId": "11111111-1111-4111-8111-111111111111",
            "nodes": {
                "LoadoutSlot_01": { "node_status": 2 },
                "LoadoutSlot_02": { "node_status": 2 },
                "LoadoutSlot_03": { "node_status": 1 }
            },
            "updateVersion": 5,
            "revision": 2
        }))],
    );

    cols.insert(
        "progressionTracks".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111:weapon_sword",
            "userId": "11111111-1111-4111-8111-111111111111",
            "progressionId": "weapon_sword",
            "progress": 3500,
            "confirmedFremiumRank": 5,
            "confirmedPremiumRank": 5,
            "updateVersion": 4,
            "createdAt": "2026-03-01T11:00:00.000Z",
            "updatedAt": "2026-03-09T11:00:00.000Z",
            "reforge": { "count": 1 },
            "prestigeClaimed": 0
        }))],
    );

    cols.insert(
        "progressionObjectives".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111:obj_slay_behemoths",
            "userId": "11111111-1111-4111-8111-111111111111",
            "objectiveId": "obj_slay_behemoths",
            "value": 12,
            "completedCount": 1,
            "createdAt": "2026-03-01T11:00:00.000Z",
            "updatedAt": "2026-03-09T11:00:00.000Z"
        }))],
    );

    cols.insert(
        "progressionObjectiveEvents".into(),
        vec![obj(json!({
            "_id": "evt-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "rawBody": { "events": ["behemoth_slain"] },
            "receivedAt": "2026-03-09T11:00:00.000Z"
        }))],
    );

    cols.insert(
        "progressionTransactions".into(),
        vec![obj(json!({
            "_id": "grant-001",
            "grantId": "grant-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "progressionId": "weapon_sword",
            "amount": 500,
            "requestHash": "grant_hash_001",
            "status": "applied",
            "result": {
                "userId": "11111111-1111-4111-8111-111111111111",
                "progressionId": "weapon_sword",
                "progress": 3500,
                "confirmedFremiumRank": 5,
                "confirmedPremiumRank": 5,
                "updateVersion": 4,
                "createdAt": "2026-03-01T11:00:00.000Z",
                "updatedAt": "2026-03-09T11:00:00.000Z"
            },
            "createdAt": "2026-03-09T11:00:00.000Z"
        }))],
    );

    cols.insert(
        "escalationProgress".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111:esc_shock",
            "userId": "11111111-1111-4111-8111-111111111111",
            "seasonId": "esc_shock",
            "escalationLevel": 15,
            "nextLevelXp": 800,
            "talentsProgress": [{ "id": "talent_1", "rank": 2 }],
            "unlockProgress": [{ "id": "unlock_1" }],
            "updateVersion": 6,
            "createdAt": "2026-03-02T11:00:00.000Z",
            "updatedAt": "2026-03-09T11:00:00.000Z"
        }))],
    );

    cols.insert(
        "breadcrumbs".into(),
        vec![obj(json!({
            "_id": "char-001",
            "characterId": "char-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "breadcrumbs": ["ITEM_NEW_01", "QUEST_02"],
            "updateVersion": 2
        }))],
    );

    cols.insert(
        "encounteredContent".into(),
        vec![obj(json!({
            "_id": "char-001",
            "characterId": "char-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "encounteredcontent": ["BEHEMOTH_GNASHER", "BEHEMOTH_SHRIKE"]
        }))],
    );

    // 5. Bounties (`bountyStates`, `cooldownStates`, `entitlements`, `wallets`, `economyDaily`)
    cols.insert(
        "bountyStates".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111",
            "userId": "11111111-1111-4111-8111-111111111111",
            "bounties": [{ "group": "base", "bounty": { "id": "BNT_01" } }],
            "draftData": { "base": { "choices": ["BNT_02", "BNT_03"] } },
            "dailyRewardCreditedOn": "2026-03-10",
            "dailyPremiumDraftGrantedOn": "2026-03-10",
            "updateVersion": 3,
            "createdAt": "2026-03-01T12:00:00.000Z",
            "updatedAt": "2026-03-10T12:00:00.000Z"
        }))],
    );

    cols.insert(
        "cooldownStates".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111",
            "userId": "11111111-1111-4111-8111-111111111111",
            "entries": [{
                "id": "daily_fountain",
                "startedDate": "2026-03-10T00:00:00.000Z",
                "updatedAt": "2026-03-10T00:00:00.000Z"
            }],
            "updateVersion": 1,
            "createdAt": "2026-03-01T12:00:00.000Z",
            "updatedAt": "2026-03-10T12:00:00.000Z"
        }))],
    );

    cols.insert(
        "entitlements".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111",
            "userId": "11111111-1111-4111-8111-111111111111",
            "entitlements": [
                {
                    "name": "slayers_club_30d",
                    "duration": 2592000,
                    "activatedDate": "2026-03-01T12:00:00.000Z",
                    "grantedAt": "2026-03-01T12:00:00.000Z",
                    "sourceSkuId": "SKU_CLUB_30"
                }
            ]
        }))],
    );

    cols.insert(
        "wallets".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111",
            "userId": "11111111-1111-4111-8111-111111111111",
            "balances": {
                "CURRENCY_NOTES": 5500,
                "CURRENCY_PLATINUM": 1200
            },
            "bootstrapVersion": "starter-1.14-v1"
        }))],
    );

    cols.insert(
        "economyDaily".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111:2026-03-10:CURRENCY_NOTES",
            "userId": "11111111-1111-4111-8111-111111111111",
            "day": "2026-03-10",
            "currency": "CURRENCY_NOTES",
            "credited": 500,
            "bySource": {
                "HUNT": { "credited": 500 },
                "STORE": { "debited": 100 }
            },
            "ttlAt": "2027-04-14T00:00:00.000Z"
        }))],
    );

    // 6. Parties & Friendships
    cols.insert(
        "parties".into(),
        vec![obj(json!({
            "_id": "party_hex_647472",
            "partyId": "party_hex_647472",
            "revision": 2,
            "leaderPlayerId": "11111111-1111-4111-8111-111111111111",
            "members": [
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222"
            ],
            "buildId": "647472",
            "activity": "IDLE",
            "activeHuntSessionId": null,
            "createdAt": "2026-03-10T12:10:00.000Z",
            "updatedAt": "2026-03-10T12:11:00.000Z"
        }))],
    );

    cols.insert(
        "partyMemberships".into(),
        vec![
            obj(json!({
                "_id": "11111111-1111-4111-8111-111111111111",
                "partyId": "party_hex_647472",
                "joinedAt": "2026-03-10T12:10:00.000Z"
            })),
            obj(json!({
                "_id": "22222222-2222-4222-8222-222222222222",
                "partyId": "party_hex_647472",
                "joinedAt": "2026-03-10T12:11:00.000Z"
            })),
        ],
    );

    cols.insert(
        "partyInvites".into(),
        vec![obj(json!({
            "_id": "22222222-2222-4222-8222-222222222222\u{0000}11111111-1111-4111-8111-111111111111",
            "partyId": "party_hex_647472",
            "partyRevision": 1,
            "sendingPlayerId": "11111111-1111-4111-8111-111111111111",
            "recipientPlayerId": "22222222-2222-4222-8222-222222222222",
            "sendingDisplayName": "SlayerOne",
            "sendingPlatform": "PC",
            "status": "ACCEPTED",
            "createdAt": "2026-03-10T12:10:30.000Z",
            "expiresAt": "2026-03-10T12:15:30.000Z",
            "consumedAt": "2026-03-10T12:11:00.000Z"
        }))],
    );

    cols.insert(
        "partyOutbox".into(),
        vec![obj(json!({
            "_id": "outbox-evt-001",
            "eventId": "outbox-evt-001",
            "partyId": "party_hex_647472",
            "partyRevision": 2,
            "eventType": "PARTY_MEMBERSHIP_CHANGED",
            "recipientAccountIds": [
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222"
            ],
            "payload": { "action": "JOIN" },
            "state": "PUBLISHED",
            "createdAt": "2026-03-10T12:11:00.000Z",
            "claimedAt": "2026-03-10T12:11:01.000Z",
            "claimOwner": "worker-1",
            "publishedAt": "2026-03-10T12:11:02.000Z"
        }))],
    );

    cols.insert(
        "friendships".into(),
        vec![obj(json!({
            "_id": "11111111-1111-4111-8111-111111111111:22222222-2222-4222-8222-222222222222",
            "ownerId": "11111111-1111-4111-8111-111111111111",
            "otherId": "22222222-2222-4222-8222-222222222222",
            "status": "ACCEPTED",
            "direction": null,
            "created": "2026-03-05T10:00:00.000Z",
            "favorite": true
        }))],
    );

    // 7. Reforged API (`reforgedInvites`, `adminSessions`, `adminAudit`, `launcherNotices`, `operationsPolicy`, `launcherGuardSessions`, `guardFindings`)
    cols.insert(
        "reforgedInvites".into(),
        vec![obj(json!({
            "_id": "INVITE-ALPHA-2026",
            "invitecode": "INVITE-ALPHA-2026",
            "usesRemaining": 5,
            "infiniteUses": false,
            "note": "Alpha wave",
            "createdAt": "2026-03-01T00:00:00.000Z"
        }))],
    );

    cols.insert(
        "adminSessions".into(),
        vec![obj(json!({
            "_id": "admin-sess-001",
            "userId": "11111111-1111-4111-8111-111111111111",
            "tokenHash": "admin_token_hash_001",
            "csrfToken": "csrf_001",
            "createdAt": "2026-03-10T12:00:00.000Z",
            "expiresAt": "2026-03-11T12:00:00.000Z",
            "ip": "127.0.0.1",
            "userAgent": "Mozilla/5.0",
            "revokedAt": null,
            "ttlAt": "2026-03-11T12:00:00.000Z"
        }))],
    );

    cols.insert(
        "adminAudit".into(),
        vec![obj(json!({
            "_id": "audit-001",
            "actorUserId": "11111111-1111-4111-8111-111111111111",
            "targetUserId": "22222222-2222-4222-8222-222222222222",
            "action": "ACCOUNT_APPROVAL_UPDATED",
            "oldState": { "approvalStatus": "pending" },
            "newState": { "approvalStatus": "approved" },
            "reason": "Verified player",
            "ip": "127.0.0.1",
            "requestId": "req-audit-001",
            "createdAt": "2026-03-10T12:05:00.000Z"
        }))],
    );

    cols.insert(
        "launcherNotices".into(),
        vec![obj(json!({
            "_id": "current",
            "version": 3,
            "updatedAt": "2026-03-10T00:00:00.000Z",
            "updatedBy": "11111111-1111-4111-8111-111111111111",
            "announcements": [{ "id": "ann-1", "title": "Welcome to Rustic Isles" }],
            "tags": ["patch-1.14.7"]
        }))],
    );

    cols.insert(
        "operationsPolicy".into(),
        vec![obj(json!({
            "_id": "current",
            "guardEnforcement": "OBSERVE",
            "diagnosticsProfile": "ELEVATED",
            "version": 2,
            "updatedAt": "2026-03-10T00:00:00.000Z",
            "updatedBy": "11111111-1111-4111-8111-111111111111",
            "diagnosticsExpiresAt": "2026-03-17T00:00:00.000Z"
        }))],
    );

    cols.insert(
        "launcherGuardSessions".into(),
        vec![obj(json!({
            "_id": "guard-sess-001",
            "accountId": "11111111-1111-4111-8111-111111111111",
            "role": "client",
            "publicKeyBase64": "cHVia2V5",
            "manifestId": "man-001",
            "manifestSequence": 1,
            "channel": "stable",
            "launcherVersion": "0.2.0",
            "challenge": "chal-001",
            "lastSequence": 2,
            "status": "HEALTHY",
            "riskScore": 0.5,
            "createdAt": "2026-03-10T12:00:00.000Z",
            "expiresAt": "2026-03-10T13:00:00.000Z",
            "lastHeartbeatAt": "2026-03-10T12:02:00.000Z",
            "lastSignals": { "debugger": false },
            "reasons": [],
            "history": [],
            "ttlAt": "2026-06-08T12:02:00.000Z"
        }))],
    );

    cols.insert(
        "guardFindings".into(),
        vec![obj(json!({
            "_id": "finding-001",
            "fileName": "overlay.dll",
            "sha256": "deadbeef0123456789abcdef0123456789abcdef0123456789abcdef01234567",
            "signer": "Unsigned",
            "size": 40960,
            "firstSeenAt": "2026-03-10T12:01:00.000Z",
            "lastSeenAt": "2026-03-10T12:02:00.000Z",
            "sessionCount": 1,
            "accountCount": 1,
            "accountIds": ["11111111-1111-4111-8111-111111111111"],
            "ttlAt": "2026-06-08T12:02:00.000Z"
        }))],
    );

    cols
}

#[test]
fn cli_arg_parsing_covers_all_flags() {
    let cmd = parse_args([
        "migrate",
        "--from",
        "mongodb://127.0.0.1:27017/dauntless",
        "--to",
        "/tmp/isles.db",
        "--node",
        "eu-west",
        "--area",
        "auth,characters",
        "--area",
        "bounties",
        "--dry-run",
        "--skip-invalid",
        "--allow-remote-postgres",
        "--json",
    ])
    .unwrap();

    match cmd {
        Command::Migrate { opts, json } => {
            assert!(json);
            assert!(opts.dry_run);
            assert!(opts.skip_invalid);
            assert!(opts.allow_remote_postgres);
            assert_eq!(opts.node.as_str(), "eu-west");
            assert_eq!(
                opts.areas,
                vec![MigrateArea::Auth, MigrateArea::Characters, MigrateArea::Bounties]
            );
        }
        other => panic!("unexpected command: {other:?}"),
    }

    assert_eq!(parse_args(["--help"]).unwrap(), Command::Help);
    assert!(parse_args(["unknown"]).is_err());
}

#[test]
fn mongo_uri_and_bson_wire_roundtrip() {
    let uri =
        MongoUri::parse("mongodb://myuser:p%40ss@127.0.0.1:27017,127.0.0.2:27018/dauntless?authSource=admin").unwrap();
    assert_eq!(
        uri.hosts,
        vec![("127.0.0.1".into(), 27017), ("127.0.0.2".into(), 27018)]
    );
    assert_eq!(uri.database, "dauntless");
    assert_eq!(uri.username.as_deref(), Some("myuser"));
    assert_eq!(uri.password.as_deref(), Some("p@ss"));
    assert_eq!(uri.auth_source, "admin");

    let mut buf = BsonBuf::new();
    buf.put_str("s", "hello")
        .put_i32("i32", 42)
        .put_i64("i64", 9_000_000_000)
        .put_f64("f64", 3.5)
        .put_bool("b", true)
        .put_null("n")
        .put_binary("bin", b"xyz");
    let decoded = decode_bson_doc(&buf.finish()).unwrap();
    assert_eq!(decoded["s"], "hello");
    assert_eq!(decoded["i32"], 42);
    assert_eq!(decoded["i64"], 9_000_000_000i64);
    assert_eq!(decoded["f64"], 3.5);
    assert_eq!(decoded["b"], true);
    assert_eq!(decoded["n"], Value::Null);
}

#[tokio::test]
async fn full_migration_and_verification_across_all_areas_and_rb11_transforms() {
    let collections = sample_mongo_collections();
    // Use max_batch = 1 so cursor pagination (`find` + `getMore`) is exercised on multi-doc collections!
    let mock_mongo = MockMongoServer::start(collections, 1).unwrap();
    let mongo_uri = mock_mongo.uri("dauntless");

    let temp_dir = tempfile::tempdir().unwrap();
    let sqlite_path = temp_dir.path().join("migrated.db");
    let policy_path = temp_dir.path().join("inventory_storage_policy.json");
    std::fs::write(
        &policy_path,
        serde_json::to_string(&json!({
            "entries": {
                "WP_EB_BEGINNER": { "storage": "instanced", "maxQuantity": 1, "tags": [] },
                "STACKABLE_CELL_01": { "storage": "stacked", "maxQuantity": 99, "tags": ["cell"] },
                "QI_BASIC_FLARE_DURABLE": { "storage": "stacked", "maxQuantity": 1, "tags": ["owned"] }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let node: NodeId = "local".parse().unwrap();

    // 1. Dry-run does not create or write to the target SQLite file
    let dry_opts = MigrateOptions {
        from_uri: mongo_uri.clone(),
        to_target: sqlite_path.display().to_string(),
        node: node.clone(),
        dry_run: true,
        areas: Vec::new(),
        skip_invalid: false,
        game_data_dir: Some(temp_dir.path().to_path_buf()),
        allow_remote_postgres: false,
    };
    let dry_report = run_migrate(&dry_opts).await.unwrap();
    assert!(dry_report.dry_run);
    assert_eq!(dry_report.rb11_launcher_approvals_backfilled, 1);
    assert!(!sqlite_path.exists(), "dry-run must not create target SQLite file");

    // 2. Real migration into SQLite
    let mut opts = dry_opts.clone();
    opts.dry_run = false;
    let report = run_migrate(&opts).await.unwrap();
    assert!(!report.dry_run);
    assert_eq!(report.counts.get("accounts"), Some(&2));
    assert_eq!(report.counts.get("launcher_accounts"), Some(&2));
    assert_eq!(report.counts.get("characters"), Some(&1));
    assert_eq!(report.counts.get("loadouts"), Some(&1));
    assert_eq!(report.counts.get("inventories"), Some(&1));
    assert_eq!(report.counts.get("parties"), Some(&1));
    assert_eq!(report.counts.get("guard_findings"), Some(&1));

    // 3. Verify passes across all 31 collections/tables
    let verify_report = run_verify(&opts).await.unwrap();
    assert!(verify_report.ok);
    assert_eq!(verify_report.collections.len(), 37);

    // 4. Re-running migration is idempotent and `verify` still passes
    let report_again = run_migrate(&opts).await.unwrap();
    assert_eq!(report_again.counts, report.counts);
    let verify_again = run_verify(&opts).await.unwrap();
    assert!(verify_again.ok);

    // 5. Verify hash chain (`store::history::verify`) and `system:migrate` actor attribution,
    //    and read back migrated records through `isles-core` repositories!
    let db = open_target_database(&opts.to_target, false).unwrap();
    let acc1 = deterministic_account_id(&node, "11111111-1111-4111-8111-111111111111").unwrap();
    let acc2 = deterministic_account_id(&node, "22222222-2222-4222-8222-222222222222").unwrap();

    db.read(move |conn| -> Result<(), DbError> {
        store::history::verify(conn, None).map_err(DbError::Store)?;
        let entries = store::history::changes(conn, 0, 10_000).map_err(DbError::Store)?;
        assert!(!entries.is_empty(), "change log must contain entries");
        assert!(
            entries.iter().any(|e| e.actor == "system:migrate"),
            "change log must record entries under system:migrate"
        );

        let repos = Repositories::on_store();

        // Auth readbacks + RB11 `migrate_launcher_approval` check
        let u1 = repos
            .accounts
            .find_by_user_id(conn, &UserId("11111111-1111-4111-8111-111111111111".into()))
            .unwrap()
            .unwrap();
        assert_eq!(u1.id, acc1);
        assert_eq!(u1.notes, 5000);

        let la1 = repos.launcher_accounts.find_by_account(conn, &acc1).unwrap().unwrap();
        assert_eq!(la1.approval_status.unwrap().as_str(), "approved");
        assert_eq!(la1.approval_updated_by.as_deref(), Some("migration:0.1.73"));

        let la2 = repos.launcher_accounts.find_by_account(conn, &acc2).unwrap().unwrap();
        assert_eq!(la2.approval_status.unwrap().as_str(), "pending");

        let ident = repos
            .auth_identities
            .find_by_provider_subject(conn, "999888777")
            .unwrap()
            .unwrap();
        assert_eq!(ident.account, acc1);

        let rs = repos
            .refresh_sessions
            .find_by_token_hash(conn, "th_sess_001")
            .unwrap()
            .unwrap();
        assert_eq!(rs.id, "sess-001");
        assert_eq!(rs.account, acc1);

        let ec = repos
            .email_codes
            .peek_live(conn, &acc1, EmailCodePurpose::Verify, "2026-03-01T10:05:00.000Z", 5)
            .unwrap()
            .unwrap();
        assert_eq!(ec.attempts, 1);

        let gs_keys = repos.api_keys.find_all_game_server_key_hashes(conn).unwrap();
        assert!(gs_keys.iter().any(|k| k.key_hash.as_deref() == Some("gs_key_hash_001")));
        let user_keys = repos.api_keys.find_all_user_key_hashes(conn).unwrap();
        assert!(
            user_keys
                .iter()
                .any(|k| k.account == acc1 && k.key_hash == "user_key_hash_001")
        );

        // Characters & Loadouts + RB11 starter instance repair + RB11 loadout slot reconciliation
        let chars = repos.characters.find_many_by_account(conn, &acc1).unwrap();
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].name, "SlayerOne");

        let ld = repos.loadouts.find(conn, "char-001", &acc1).unwrap().unwrap();
        // `LoadoutSlot_01` and `LoadoutSlot_02` were unlocked (status 2) in playerJourney -> 1 + 2 = 3 slots
        assert_eq!(ld.unlocked_total_slots, Some(3));
        let ld_slots: Vec<Value> = serde_json::from_str(&ld.loadouts).unwrap();
        assert_eq!(ld_slots.len(), 3);
        // Active index 99 was clamped to 2 (len - 1)
        assert_eq!(ld.active_index, Some(2));
        // Starter weapon instance_id "bad-id" was repaired to a valid 26-char uppercase alphanumeric ID
        let repaired_weapon_id = ld_slots[0]["weapon"]["instance_id"].as_str().unwrap();
        assert_eq!(repaired_weapon_id.len(), 26);

        // Inventory + RB11 storage policy relocation (`STACKABLE_CELL_01` moved from instanced to stacked)
        let inv = repos
            .inventories
            .find_by_character_id(conn, "char-001")
            .unwrap()
            .unwrap();
        let instanced: Vec<Value> = serde_json::from_str(&inv.instanced_items).unwrap();
        let stacked: Vec<Value> = serde_json::from_str(&inv.stacked_items).unwrap();
        assert!(
            instanced
                .iter()
                .any(|i| i["catalogId"] == "WP_EB_BEGINNER" && i["instanceId"] == repaired_weapon_id),
            "inventory must contain the repaired starter weapon instance referenced by loadout"
        );
        assert!(
            instanced.iter().any(|i| i["catalogId"] == "BN_BEGINNER_00"),
            "RB11 banner instance must be seeded"
        );
        assert!(
            stacked
                .iter()
                .any(|s| s["catalogId"] == "STACKABLE_CELL_01" && s["quantity"] == 1),
            "RB11 storage policy must move stackable item from instanced to stacked"
        );

        let itx = repos
            .inventory_transactions
            .find_record(
                conn,
                LedgerKey {
                    account: &acc1,
                    character_id: "char-001",
                    transaction_id: "tx-001",
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(itx.status.as_str(), "completed");

        // Bounties, Cooldowns, Entitlements, Wallets
        let bs = repos.bounty_states.get(conn, &acc1).unwrap().unwrap();
        assert_eq!(bs.update_version, 3);
        let cs = repos.cooldown_states.get(conn, &acc1).unwrap().unwrap();
        assert_eq!(cs.update_version, 1);
        let ent = repos.entitlements.find(conn, &acc1).unwrap().unwrap();
        assert_eq!(ent.entitlements.len(), 1);
        assert_eq!(ent.entitlements[0].name, "slayers_club_30d");
        let wallet = repos.wallets.find(conn, &acc1).unwrap().unwrap();
        assert_eq!(wallet.balance("CURRENCY_NOTES"), Some(5500.0));
        assert_eq!(wallet.balance("CURRENCY_PLATINUM"), Some(1200.0));

        // Progression
        let tracks = repos.progression_tracks.get_all_for_account(conn, &acc1).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].progress, 3500);
        let objs = repos
            .progression_tracks
            .get_objectives(conn, &acc1, &["obj_slay_behemoths".into()])
            .unwrap();
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].value, 12);
        let grant = repos
            .progression_grants
            .find_by_grant_id(conn, "grant-001")
            .unwrap()
            .unwrap();
        assert_eq!(grant.amount, 500);
        let esc = repos
            .escalation_progress
            .get(conn, &acc1, "esc_shock")
            .unwrap()
            .unwrap();
        assert_eq!(esc.escalation_level, 15);
        let pj = repos.player_journey.find(conn, &acc1).unwrap().unwrap();
        assert_eq!(pj.update_version, 5);
        let bc = repos.breadcrumbs.find(conn, "char-001", &acc1).unwrap().unwrap();
        let bc_items: Vec<String> = serde_json::from_str(&bc.breadcrumbs).unwrap();
        assert_eq!(bc_items, vec!["ITEM_NEW_01", "QUEST_02"]);
        let ec_content = repos
            .encountered_content
            .find(conn, "char-001", &acc1)
            .unwrap()
            .unwrap();
        let ec_items: Vec<String> = serde_json::from_str(&ec_content.encountered_content).unwrap();
        assert_eq!(ec_items, vec!["BEHEMOTH_GNASHER", "BEHEMOTH_SHRIKE"]);

        // Parties & Friendships
        let party = repos.parties.find_by_member(conn, &acc1).unwrap().unwrap();
        assert_eq!(party.party_id, "party_hex_647472");
        assert_eq!(party.members.len(), 2);
        let friends = repos.friendships.list_for_owner(conn, &acc1).unwrap();
        assert_eq!(friends.len(), 1);
        assert!(friends[0].favorite);

        // Reforged API
        let invites = repos.invites.list(conn, 10).unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].code, "INVITE-ALPHA-2026");
        let notices = repos.launcher_notices.get(conn).unwrap().unwrap();
        assert_eq!(notices.version, 3);
        let policy = repos.admin.read_operations_policy(conn).unwrap().unwrap();
        assert_eq!(policy.guard_enforcement.as_str(), "OBSERVE");
        let gs = repos.launcher_guard.find(conn, "guard-sess-001").unwrap().unwrap();
        assert_eq!(gs.account, acc1);
        let (findings, f_total) = repos.launcher_guard.list_findings(conn, None, 0, 10).unwrap();
        assert_eq!(f_total, 1);
        assert_eq!(findings[0].file_name, "overlay.dll");

        Ok(())
    })
    .await
    .unwrap();

    // 6. Tamper with a row in the target database and verify `run_verify` detects the mismatch
    db.write(store::Actor::System("test".into()), move |tx| -> Result<(), DbError> {
        use diesel::prelude::*;
        diesel::sql_query("UPDATE accounts SET notes = 999999 WHERE user_id = '11111111-1111-4111-8111-111111111111'")
            .execute(tx.conn())
            .map_err(DbError::Diesel)?;
        Ok(())
    })
    .await
    .unwrap();

    let tampered = run_verify(&opts).await;
    assert!(matches!(tampered, Err(DbError::VerificationFailed(1))));
}

#[tokio::test]
async fn export_mongo_roundtrip_passes_verify_across_all_collections() {
    let parsed = parse_args([
        "export-mongo",
        "--from",
        "sqlite.db",
        "--to",
        "mongodb://127.0.0.1:27017/dauntless",
        "--node",
        "local",
        "--area",
        "auth,characters",
        "--dry-run",
        "--allow-remote-postgres",
        "--json",
    ])
    .unwrap();
    match parsed {
        Command::ExportMongo { opts, json } => {
            assert!(json);
            assert!(opts.dry_run);
            assert!(opts.allow_remote_postgres);
            assert_eq!(opts.from_source, "sqlite.db");
            assert_eq!(opts.to_uri, "mongodb://127.0.0.1:27017/dauntless");
            assert_eq!(opts.areas, vec![MigrateArea::Auth, MigrateArea::Characters]);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let source_mongo = MockMongoServer::start(sample_mongo_collections(), 2).unwrap();
    let source_uri = source_mongo.uri("dauntless");

    let temp_dir = tempfile::tempdir().unwrap();
    let sqlite_path = temp_dir.path().join("roundtrip.db");
    let policy_path = temp_dir.path().join("inventory_storage_policy.json");
    std::fs::write(
        &policy_path,
        serde_json::to_string(&json!({
            "entries": {
                "WP_EB_BEGINNER": { "storage": "instanced", "maxQuantity": 1, "tags": [] },
                "STACKABLE_CELL_01": { "storage": "stacked", "maxQuantity": 99, "tags": ["cell"] },
                "QI_BASIC_FLARE_DURABLE": { "storage": "stacked", "maxQuantity": 1, "tags": ["owned"] }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let node: NodeId = "local".parse().unwrap();
    let migrate_opts = MigrateOptions {
        from_uri: source_uri,
        to_target: sqlite_path.display().to_string(),
        node: node.clone(),
        dry_run: false,
        areas: Vec::new(),
        skip_invalid: false,
        game_data_dir: Some(temp_dir.path().to_path_buf()),
        allow_remote_postgres: false,
    };
    run_migrate(&migrate_opts).await.unwrap();

    let dest_mongo = MockMongoServer::start(BTreeMap::new(), 2).unwrap();
    let dest_uri = dest_mongo.uri("dauntless");

    let dry_export = run_export_mongo(&ExportOptions {
        from_source: sqlite_path.display().to_string(),
        to_uri: dest_uri.clone(),
        areas: Vec::new(),
        dry_run: true,
        allow_remote_postgres: false,
    })
    .await
    .unwrap();
    assert!(dry_export.dry_run);
    assert!(
        dest_mongo.snapshot().is_empty(),
        "dry-run export must not write to Mongo"
    );

    let export_report = run_export_mongo(&ExportOptions {
        from_source: sqlite_path.display().to_string(),
        to_uri: dest_uri.clone(),
        areas: Vec::new(),
        dry_run: false,
        allow_remote_postgres: false,
    })
    .await
    .unwrap();
    assert!(!export_report.dry_run);
    assert_eq!(export_report.counts.get("accounts"), Some(&2));
    assert_eq!(export_report.counts.get("characters"), Some(&1));

    let verify_exported = run_verify(&MigrateOptions {
        from_uri: dest_uri,
        to_target: sqlite_path.display().to_string(),
        node,
        dry_run: false,
        areas: Vec::new(),
        skip_invalid: false,
        game_data_dir: Some(temp_dir.path().to_path_buf()),
        allow_remote_postgres: false,
    })
    .await
    .unwrap();
    assert!(verify_exported.ok);
    assert_eq!(verify_exported.collections.len(), 37);
}

#[tokio::test]
async fn external_mongo_and_postgres_rehearsal_if_configured() {
    let temp_dir = tempfile::tempdir().unwrap();
    let policy_dir = temp_dir.path().join("game-data");
    std::fs::create_dir_all(&policy_dir).unwrap();
    std::fs::write(
        policy_dir.join("catalog-storage-policy.json"),
        serde_json::to_string(&json!({
            "schemaVersion": 1,
            "generatedAt": "2026-10-27T00:00:00.000Z",
            "policy": {
                "WP_EB_BEGINNER": "instanced",
                "BN_BEGINNER_00": "instanced",
                "STACKABLE_CELL_01": "stacked"
            }
        }))
        .unwrap(),
    )
    .unwrap();

    // If `ISLES_TEST_POSTGRES_URL` is set without `ISLES_TEST_MONGO_URI`, rehearse Postgres target
    // against our embedded wire-protocol MongoDB server with all 31 collections.
    let (_mock_guard, mongo_uri) = match std::env::var("ISLES_TEST_MONGO_URI") {
        Ok(uri) => {
            if std::env::var("ISLES_TEST_SEED_MONGO").as_deref() == Ok("1") {
                seed_external_mongo(&uri, &sample_mongo_collections());
            }
            (None, uri)
        }
        Err(_) => {
            if std::env::var("ISLES_TEST_POSTGRES_URL").is_err() {
                return;
            }
            let mock = MockMongoServer::start(sample_mongo_collections(), 10).unwrap();
            let uri = mock.uri("dauntless");
            (Some(mock), uri)
        }
    };

    let node: NodeId = "rehearsal".parse().unwrap();
    let sqlite_target = temp_dir.path().join("rehearsal.db");

    let sqlite_opts = MigrateOptions {
        from_uri: mongo_uri.clone(),
        to_target: sqlite_target.display().to_string(),
        node: node.clone(),
        dry_run: false,
        areas: Vec::new(),
        skip_invalid: false,
        game_data_dir: Some(policy_dir.clone()),
        allow_remote_postgres: false,
    };
    let mig_sqlite = run_migrate(&sqlite_opts).await.unwrap();
    assert!(*mig_sqlite.counts.get("accounts").unwrap_or(&0) > 0);
    let ver_sqlite = run_verify(&sqlite_opts).await.unwrap();
    assert!(ver_sqlite.ok);

    if let Ok(pg_url) = std::env::var("ISLES_TEST_POSTGRES_URL") {
        let pg_opts = MigrateOptions {
            from_uri: mongo_uri,
            to_target: pg_url,
            node,
            dry_run: false,
            areas: Vec::new(),
            skip_invalid: false,
            game_data_dir: Some(policy_dir),
            allow_remote_postgres: false,
        };
        let mig_pg = run_migrate(&pg_opts).await.unwrap();
        assert_eq!(mig_pg.counts, mig_sqlite.counts);
        let ver_pg = run_verify(&pg_opts).await.unwrap();
        assert!(ver_pg.ok);
    }
}

fn seed_external_mongo(raw_uri: &str, collections: &BTreeMap<String, Vec<Map<String, Value>>>) {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let uri = MongoUri::parse(raw_uri).unwrap();
    let (host, port) = &uri.hosts[0];
    let mut stream = TcpStream::connect(format!("{host}:{port}")).unwrap();
    let mut req_id = 1i32;

    let mut send_cmd = |doc: &BsonBuf| {
        let body = doc.finish();
        let len = (16 + 4 + 1 + body.len()) as i32;
        let mut frame = Vec::with_capacity(len as usize);
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(&req_id.to_le_bytes());
        req_id += 1;
        frame.extend_from_slice(&0i32.to_le_bytes());
        frame.extend_from_slice(&2013i32.to_le_bytes());
        frame.extend_from_slice(&0u32.to_le_bytes());
        frame.push(0u8);
        frame.extend_from_slice(&body);
        stream.write_all(&frame).unwrap();
        stream.flush().unwrap();

        let mut hdr = [0u8; 16];
        stream.read_exact(&mut hdr).unwrap();
        let resp_len = i32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let mut payload = vec![0u8; resp_len - 16];
        stream.read_exact(&mut payload).unwrap();
        let reply = decode_bson_doc(&payload[5..]).unwrap();
        assert_eq!(
            reply.get("ok").and_then(Value::as_f64),
            Some(1.0),
            "mongo seed cmd failed: {reply:?}"
        );
    };

    let mut drop_db = BsonBuf::new();
    drop_db.put_i32("dropDatabase", 1).put_str("$db", &uri.database);
    send_cmd(&drop_db);

    for (col_name, docs) in collections {
        if docs.is_empty() {
            continue;
        }
        let bson_docs: Vec<BsonBuf> = docs.iter().map(BsonBuf::from_json_map).collect();
        let mut ins = BsonBuf::new();
        ins.put_str("insert", col_name)
            .put_array_docs("documents", &bson_docs)
            .put_str("$db", &uri.database);
        send_cmd(&ins);
    }
}
