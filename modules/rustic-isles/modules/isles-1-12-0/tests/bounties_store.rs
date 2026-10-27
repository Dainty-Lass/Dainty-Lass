//! RU5 bounties-store on 1.12: the area's routes answer 1.12.0's stubs, the 1.14.7-only routes leave the shared 404,
//! and the store serves Lady Luck only (RU3c's `entitlements_payload`, `STORE_SECTIONS` and the persistence switches).
//! Unverified, as DD9 says of every 1.12 answer: only tests back it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use isles_core::areas::bounties_store::harness::{DatabaseConfig, Reply, Server, make_account, start, write_game_data};
use isles_core::areas::bounties_store::wallet::add_currency;
use module_api::{Actor, ActorContext};
use serde_json::{Value, json};

const GS: Option<&str> = Some("gs");

fn player(id: &str) -> String {
    format!("player:{id}")
}

async fn boot(dir: &std::path::Path) -> Server {
    write_game_data(dir);
    let target = DatabaseConfig::Sqlite {
        file: dir.join("isles.db"),
    };
    start(isles_1_12_0::version(), &target, Some(dir), &[]).await
}

const EMPTY_DRAFT: &str =
    r#"{"current_draft_choices":[],"previous_draft_selections":[],"bronze_count":0,"silver_count":0,"gold_count":0}"#;

fn stub_state() -> String {
    format!(
        r#"{{"code":null,"message":"OK","payload":{{"season_start_date":"2020-08-23T00:00:00.000Z","season_end_date":"2099-01-01T00:00:00.000Z","bounties":[],"draft_data":{EMPTY_DRAFT},"draft_data_daily":{EMPTY_DRAFT},"draft_data_weekly":{EMPTY_DRAFT}}}}}"#
    )
}

fn ids(reply: &Reply) -> Vec<String> {
    reply
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn bounties_and_cooldowns_are_the_empty_stubs_that_store_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = boot(dir.path()).await;
    make_account(&server.store, "p1").await;
    let game_data = server.get("/bounty/game-data", GS).await;
    assert_eq!(game_data.status, 200);
    assert_eq!(game_data.json()["payload"]["max_slots"], 4);

    let save = json!({ "bounties": [{ "bounty_id": "Bounty_Bronze_X", "claimed": true, "objectives": [] }],
        "draft_data": { "gold_count": 9 } });
    assert_eq!(server.get("/bounty/p1", Some(&player("p1"))).await.body, stub_state());
    assert_eq!(
        server.send_json("POST", "/bounty/p1", GS, &save).await.body,
        stub_state()
    );
    assert_eq!(server.get("/bounty/p1", GS).await.body, stub_state());
    // the stub checks nothing: 1.12.0 had no authorisation on these routes
    assert_eq!(server.get("/bounty/p1", Some(&player("p2"))).await.body, stub_state());
    assert_eq!(
        server
            .send_json("POST", "/bounty/p1", Some(&player("p1")), &save)
            .await
            .body,
        stub_state()
    );
    assert_eq!(
        server
            .send_json("POST", "/bounty/p1", GS, &json!({ "bounties": {} }))
            .await
            .body,
        stub_state()
    );
    assert_eq!(server.get("/bounty/p1", None).await.status, 401);

    let empty = r#"{"code":null,"message":"OK","payload":{}}"#;
    let batch = json!({ "cooldowns": [{ "cooldown_id": "a", "cooldown_started_date": "2026-10-09T17:00:00.000Z" }] });
    assert_eq!(
        server.send_json("PUT", "/cooldown/batch/p1", GS, &batch).await.body,
        empty
    );
    assert_eq!(server.get("/cooldown/p1", GS).await.body, empty);

    // nothing was written
    let account = make_account_if_missing(&server, "p1").await;
    let a2 = account.clone();
    assert!(
        server
            .store
            .read(move |c, r| r.bounty_states.get(c, &a2))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        server
            .store
            .read(move |c, r| r.cooldown_states.get(c, &account))
            .await
            .unwrap()
            .is_none()
    );
}

async fn make_account_if_missing(server: &Server, user: &str) -> isles_core::AccountId {
    let user_id = isles_core::UserId(user.to_owned());
    server
        .store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
        .await
        .unwrap()
        .unwrap()
        .id
}

#[tokio::test(flavor = "multi_thread")]
async fn the_1_14_7_only_routes_answer_the_shared_404() {
    let dir = tempfile::tempdir().unwrap();
    let server = boot(dir.path()).await;
    make_account(&server.store, "p1").await;
    for (method, path) in [
        ("POST", "/bounty/delete/p1"),
        ("DELETE", "/bounty/delete/p1"),
        ("PUT", "/cooldown/p1/abc"),
        ("POST", "/cooldown/p1/abc"),
    ] {
        let reply = server.call(method, path, GS, Some(r#"{"bounty_ids":["x"]}"#)).await;
        assert_eq!((reply.status, reply.body.as_str()), (404, ""), "{method} {path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn entitlements_are_1_12s_stub_whatever_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let server = boot(dir.path()).await;
    make_account(&server.store, "p1").await;
    let stub = r#"{"code":null,"message":"OK","payload":[]}"#;
    assert_eq!(server.get("/entitlementsv2", Some(&player("p1"))).await.body, stub);
    assert_eq!(
        server
            .send_json("POST", "/entitlementv2/p1", Some(&player("p1")), &json!({}))
            .await
            .body,
        stub
    );
    assert_eq!(server.get("/entitlementsv2", None).await.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_store_serves_lady_luck_only() {
    let dir = tempfile::tempdir().unwrap();
    let server = boot(dir.path()).await;
    let account = make_account(&server.store, "p1").await;
    let a = account.clone();
    server
        .store
        .account_write(&account, &ActorContext::new(Actor::System), move |tx, repos| {
            add_currency(repos, tx, &a, "CURRENCY_MARKS_STEEL", 60.0)
                .map(|_| ())
                .map_err(|e| isles_core::repo::RepoError::Data(e.to_string()))
        })
        .await
        .unwrap();
    let get = |tag: &'static str| {
        let server = &server;
        async move {
            server
                .get(&format!("/product/skus/public?requiredTags={tag}"), Some(&player("p1")))
                .await
        }
    };
    assert_eq!(ids(&get("ladyluckstore").await), ["ll_sword", "ll_tonic"]);
    for tag in [
        "webstore",
        "dyes",
        "season_store",
        "exchange_vendor_slot_2",
        "fountain_daily_free_bundle",
        "huntpass_store",
    ] {
        assert_eq!(get(tag).await.body, "[]", "{tag}");
    }
    // another section's SKU is unknown here, by id and by item
    for id in [
        "dye_red",
        "rc_hat",
        "single_exchange_slot_2",
        "fountain_bundle",
        "DYE_RED",
    ] {
        assert_eq!(
            server
                .get(&format!("/token/platinum/{id}"), Some(&player("p1")))
                .await
                .status,
            404,
            "{id}"
        );
    }
    // a Lady Luck purchase works as it does on 1.14.7
    let token = server.get("/token/markssteel/ll_sword", Some(&player("p1"))).await;
    assert_eq!(token.status, 200);
    let token = token.json()["purchaseToken"].as_str().unwrap().to_owned();
    let bought = server
        .call(
            "POST",
            &format!("/notification/markssteel?token={token}"),
            Some(&player("p1")),
            None,
        )
        .await;
    assert_eq!((bought.status, bought.body.as_str()), (204, ""));
    let balance: Value = server.get("/balance", Some(&player("p1"))).await.json();
    assert_eq!(balance["CURRENCY_MARKS_STEEL"], 10);
}

#[tokio::test(flavor = "multi_thread")]
async fn there_is_no_season_challenge_funding() {
    use isles_core::areas::bounties_store::challenge_rewards::is_challenge_reward_grant;
    let tables = isles_1_12_0::TABLES;
    assert!(!is_challenge_reward_grant(
        tables.seasonal_coin.as_ref(),
        "CURRENCY_SEASONAL_COIN",
        400.0
    ));
    assert!(!is_challenge_reward_grant(
        tables.seasonal_coin.as_ref(),
        "CURRENCY_S19_COIN",
        400.0
    ));
}
