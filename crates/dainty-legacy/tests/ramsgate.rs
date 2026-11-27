// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The Ramsgate Reborn importer end to end: an old MongoDB (the mock) and a 1.14.7 store, matched by Discord or by a
//! redeemed claim, once per old account.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use dainty_legacy::ramsgate::{Outcome, RamsgateImporter};
use dainty_module_api::{Actor, ActorContext};
use diesel::QueryableByName;
use diesel::sql_types::BigInt;
use rustic_isles_db::LegacyStarter;
use rustic_isles_db::mongo::MockMongoServer;
use rustic_isles_store::repo::auth::{AccountRecord, AuthIdentityRecord};
use rustic_isles_store::repo::legacy::{self, RAMSGATE_REBORN};
use rustic_isles_store::test_support::TempStore;
use rustic_isles_store::{AccountId, IslesStore, UserId};
use serde_json::{Value, json};

const OLD: &str = "3f2b1c4d-aaaa-bbbb-cccc-000000000001";
const CHAR: &str = "9d0e8f7a-1111-2222-3333-444444444444";
const SNOWFLAKE: &str = "123456789012345678";

fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

fn starter() -> LegacyStarter {
    LegacyStarter {
        instanced: vec!["WP_EB_BEGINNER".into()],
        equipment: vec![
            ("weapon".into(), "WP_EB_BEGINNER".into()),
            ("flask".into(), "FL_BASIC".into()),
        ],
        banner: "BN_STARTER".into(),
        glider: "GD_STARTER".into(),
        default_dye: "DYE_DEFAULT".into(),
        default_sigil: "SIGIL_DEFAULT".into(),
    }
}

fn old_server(status: &str) -> MockMongoServer {
    let server = MockMongoServer::start(2).unwrap();
    let s = |c: &str, docs: Vec<Value>| server.seed(c, docs);
    s(
        "accounts",
        vec![json!({"_id": OLD, "userId": OLD, "name": "Ash", "displayName": "Ash",
        "displayNameNormalized": "ash", "status": status})],
    );
    s(
        "authIdentities",
        vec![json!({"provider": "discord", "providerSubject": SNOWFLAKE, "userId": OLD, "providerUsername": "ash"})],
    );
    s(
        "characters",
        vec![json!({"_id": CHAR, "characterId": CHAR, "userId": OLD, "name": "Ash",
        "createdDate": "Nov 19, 2026", "lastModifiedDate": "Nov 19, 2026", "updateVersion": 7,
        "data": "{\"LastChangelist\":392819}"})],
    );
    s(
        "inventories",
        vec![json!({"_id": CHAR, "characterId": CHAR, "userId": OLD,
        "instancedItems": "[{\"catalogId\":\"WP_EB_BEGINNER\",\"instanceId\":\"ABCDEFGHIJKLMNOPQRSTUVWXYZ\",\"itemData\":null,\"updateVersion\":1}]",
        "stackedItems": "[{\"catalogId\":\"DYE_RED\",\"quantity\":3}]"})],
    );
    s(
        "loadouts",
        vec![json!({"_id": CHAR, "characterId": CHAR, "userId": OLD,
        "loadouts": "[{\"slot_index\":0,\"weapon\":{\"item_id\":\"WP_EB_BEGINNER\",\"instance_id\":\"ABCDEFGHIJKLMNOPQRSTUVWXYZ\",\"instance_data\":null}}]",
        "persistent": "{}", "unlockedTotalSlots": 1})],
    );
    s(
        "wallets",
        vec![json!({"_id": OLD, "userId": OLD,
        "balances": {"CURRENCY_NOTES": 12345, "CURRENCY_PLATINUM": 999}})],
    );
    s(
        "progressionTracks",
        vec![
            json!({"_id": format!("{OLD}::ExperienceTrack_PlayerLevel"), "userId": OLD,
        "progressionId": "ExperienceTrack_PlayerLevel", "progress": 500, "confirmedFremiumRank": 3,
        "confirmedPremiumRank": 0, "updateVersion": 2}),
        ],
    );
    s(
        "bounties",
        vec![
            json!({"_id": format!("{OLD}::b1"), "userId": OLD, "bountyId": "b1", "slotIndex": 0,
        "data": {"bounty_id": "b1", "drafted_timestamp": "t"}}),
        ],
    );
    // Another player's rows, which must never be read as Ash's
    s(
        "characters",
        vec![
            json!({"_id": "other-char-0000", "userId": "other-user-0000", "name": "X",
        "data": "{}"}),
        ],
    );
    server
}

async fn new_account(store: &IslesStore, user: &str, discord: Option<&str>) -> AccountRecord {
    let record = AccountRecord {
        id: AccountId::new(store.node()),
        user_id: UserId::new(user),
        name: user.to_owned(),
        notes: 0,
    };
    let (r, subject) = (record.clone(), discord.map(str::to_owned));
    store
        .write(&system(), move |tx, repos| {
            repos.accounts.create(tx, &r)?;
            if let Some(subject) = subject {
                let link = AuthIdentityRecord {
                    provider_subject: subject,
                    account: r.id.clone(),
                    provider_username: "ash".into(),
                    provider_avatar_url: None,
                    linked_at: "2026-11-20T00:00:00.000Z".into(),
                };
                repos.auth_identities.create_for_provider(tx, "discord", &link)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    record
}

#[derive(QueryableByName)]
struct N {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

async fn rows(store: &IslesStore, query: &str) -> i64 {
    let q = query.to_owned();
    store
        .read(move |conn, _| {
            use diesel::RunQueryDsl as _;
            diesel::sql_query(q)
                .get_result::<N>(conn)
                .map(|r| r.n)
                .map_err(|e| rustic_isles_store::RepoError::Data(e.to_string()))
        })
        .await
        .unwrap()
}

fn importer(server: &MockMongoServer) -> RamsgateImporter {
    RamsgateImporter::new(&server.uri("rr"), "local".parse().unwrap(), starter(), None).unwrap()
}

#[tokio::test]
async fn a_discord_link_brings_the_whole_save_once() {
    let server = old_server("active");
    let temp = TempStore::open().unwrap();
    let store = &temp.store;
    let account = new_account(store, "dl-ash", Some(SNOWFLAKE)).await;
    let importer = importer(&server);

    let Outcome::Imported(old, counts) = importer.run(store, &account).await.unwrap() else {
        panic!("not imported");
    };
    assert_eq!(old, OLD);
    assert_eq!(counts.get("characters"), Some(&1));
    let a = account.id.to_string();
    assert_eq!(
        rows(
            store,
            &format!("SELECT COUNT(*) AS n FROM characters WHERE account_id = '{a}'")
        )
        .await,
        1
    );
    assert_eq!(
        rows(store, "SELECT COUNT(*) AS n FROM characters WHERE name = 'X'").await,
        0
    );
    assert_eq!(
        rows(
            store,
            &format!("SELECT CAST(balance AS BIGINT) AS n FROM wallet_balances WHERE account_id = '{a}' AND catalog_id = 'CURRENCY_NOTES'")
        )
        .await,
        12345
    );
    assert_eq!(
        rows(
            store,
            "SELECT COUNT(*) AS n FROM wallet_balances WHERE catalog_id LIKE '%PLATINUM%'"
        )
        .await,
        0,
        "platinum never carries"
    );
    assert_eq!(
        rows(
            store,
            &format!("SELECT COUNT(*) AS n FROM bounty_states WHERE account_id = '{a}'")
        )
        .await,
        1
    );
    assert_eq!(
        rows(
            store,
            &format!("SELECT COUNT(*) AS n FROM progression_tracks WHERE account = '{a}'")
        )
        .await,
        1
    );
    assert_eq!(
        rows(store, "SELECT COUNT(*) AS n FROM accounts").await,
        1,
        "no account is made"
    );

    assert_eq!(
        importer.run(store, &account).await.unwrap(),
        Outcome::NoMatch,
        "already imported"
    );
    // The same Discord user on a second account gets nothing: the save is the first account's
    let second = new_account(store, "dl-ash-2", None).await;
    assert_eq!(importer.run(store, &second).await.unwrap(), Outcome::NoMatch);
}

#[tokio::test]
async fn a_claim_names_the_save_and_a_ban_bars_it() {
    let server = old_server("active");
    let temp = TempStore::open().unwrap();
    let store = &temp.store;
    let account = new_account(store, "dl-noddy", None).await;
    let claims = importer(&server);
    assert_eq!(claims.run(store, &account).await.unwrap(), Outcome::NoMatch);

    let id = account.id.clone();
    store
        .write(&system(), move |tx, _| {
            legacy::create_claim(tx, "h", RAMSGATE_REBORN, OLD, "staff", 1)?;
            legacy::redeem_claim(tx, "h", &id, 2).map(|_| ())
        })
        .await
        .unwrap();
    assert!(matches!(
        claims.run(store, &account).await.unwrap(),
        Outcome::Imported(..)
    ));

    let banned = old_server("banned");
    let temp = TempStore::open().unwrap();
    let store = &temp.store;
    let account = new_account(store, "dl-ash", Some(SNOWFLAKE)).await;
    assert_eq!(
        importer(&banned).run(store, &account).await.unwrap(),
        Outcome::Refused("banned on Ramsgate Reborn".into())
    );
}

#[tokio::test]
async fn an_account_that_already_played_keeps_its_save() {
    let server = old_server("active");
    let temp = TempStore::open().unwrap();
    let store = &temp.store;
    let account = new_account(store, "dl-ash", Some(SNOWFLAKE)).await;
    let id = account.id.clone();
    store
        .write(&system(), move |tx, repos| {
            repos.characters.create(
                tx,
                &rustic_isles_store::repo::characters::CharacterRecord {
                    character_id: "mine".into(),
                    account: id.clone(),
                    created_date: "d".into(),
                    last_modified_date: "d".into(),
                    name: "n".into(),
                    update_version: 0,
                    data: "{}".into(),
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        importer(&server).run(store, &account).await.unwrap(),
        Outcome::AlreadyHasOne
    );
    assert_eq!(rows(store, "SELECT COUNT(*) AS n FROM characters").await, 1);
}

#[tokio::test]
async fn an_unreachable_old_database_is_an_error_not_a_panic() {
    let temp = TempStore::open().unwrap();
    let store = &temp.store;
    let account = new_account(store, "dl-ash", Some(SNOWFLAKE)).await;
    let dead = RamsgateImporter::new("mongodb://127.0.0.1:1/rr", "local".parse().unwrap(), starter(), None).unwrap();
    assert!(
        dead.run(store, &account)
            .await
            .unwrap_err()
            .contains("the old database")
    );
}
