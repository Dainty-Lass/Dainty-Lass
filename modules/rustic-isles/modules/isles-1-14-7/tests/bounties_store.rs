//! RU5 bounties-store on 1.14.7: the 17 routes over the real pipeline and a real `store` database (SQLite always,
//! Postgres when `REFORGED_TEST_POSTGRES_URL` names a server), with the seams of other areas faked. Only tests back
//! it: nothing here was played against the 1.14.7 client.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use isles_core::areas::bounties_store::harness::{
    DatabaseConfig, Reply, Server, make_account, postgres_target, start, write_game_data,
};
use isles_core::areas::bounties_store::wallet::add_currency;
use module_api::{Actor, ActorContext};
use serde_json::{Value, json};

const GS: Option<&str> = Some("gs");

fn player(id: &str) -> String {
    format!("player:{id}")
}

async fn boot(target: &DatabaseConfig, dir: &Path, settings: &[(&str, &str)]) -> Server {
    write_game_data(dir);
    start(isles_1_14_7::version(), target, Some(dir), settings).await
}

/// Credits `amount` of a currency straight through the wallet, as an earlier grant would have.
async fn credit(server: &Server, user: &str, catalog_id: &'static str, amount: f64) {
    let account = make_account_if_missing(server, user).await;
    server
        .store
        .account_write(&account.clone(), &ActorContext::new(Actor::System), move |tx, repos| {
            Ok(add_currency(repos, tx, &account, catalog_id, amount).map_err(|e| e.to_string()))
        })
        .await
        .unwrap()
        .unwrap();
}

async fn make_account_if_missing(server: &Server, user: &str) -> isles_core::AccountId {
    let user_id = isles_core::UserId(user.to_owned());
    let found = server
        .store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
        .await
        .unwrap();
    match found {
        Some(account) => account.id,
        None => make_account(&server.store, user).await,
    }
}

async fn on_each_engine<F, Fut>(run: F)
where
    F: Fn(DatabaseConfig, std::path::PathBuf) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let dir = tempfile::tempdir().unwrap();
    run(
        DatabaseConfig::Sqlite {
            file: dir.path().join("isles.db"),
        },
        dir.path().to_path_buf(),
    )
    .await;
    if let Some(pg) = postgres_target() {
        let data = tempfile::tempdir().unwrap();
        run(pg.config.clone(), data.path().to_path_buf()).await;
    }
}

fn season(id: &str, progress: i64, claimed: bool) -> Value {
    json!({ "bounty_id": id, "premium_bounty": false, "slot_index": 0,
        "objectives": [{ "objective_id": "Obj", "progress": progress }],
        "drafted_timestamp": "2026-07-30T17:00:00.000Z", "update_version": 0, "claimed": claimed })
}

const EMPTY_DRAFT: &str =
    r#"{"current_draft_choices":[],"previous_draft_selections":[],"bronze_count":0,"silver_count":0,"gold_count":0}"#;

fn empty_state() -> String {
    format!(
        r#"{{"code":null,"message":"OK","payload":{{"season_start_date":"2020-08-23T00:00:00.000Z","season_end_date":"2099-01-01T00:00:00.000Z","bounties":[],"draft_data":{EMPTY_DRAFT},"draft_data_daily":{EMPTY_DRAFT},"draft_data_weekly":{EMPTY_DRAFT}}}}}"#
    )
}

fn assert_status(reply: &Reply, status: u16) {
    assert_eq!(reply.status, status, "{} {}", reply.status, reply.body);
}

#[tokio::test(flavor = "multi_thread")]
async fn bounties() {
    on_each_engine(|target, dir| async move {
        let server = boot(&target, &dir, &[]).await;
        make_account(&server.store, "p1").await;
        make_account(&server.store, "p2").await;

        // the fixed game data, registered before /bounty/:userId
        let data = server.get("/bounty/game-data", GS).await;
        assert_status(&data, 200);
        assert_eq!(data.json()["payload"]["bounty_token_id"], "TOKEN_BOUNTY_DRAFT");
        assert_eq!(data.json()["payload"]["item_grant_data"], json!([]));
        assert_status(&server.get("/bounty/game-data", None).await, 401);
        assert_eq!(server.get("/bounty/game-data", None).await.body, "");

        // a player reads their own state, never another's; a game server reads anyone's
        let own = server.get("/bounty/p1", Some(&player("p1"))).await;
        assert_eq!(own.body, empty_state());
        assert_eq!(
            own.headers.iter().find(|(n, _)| n == "content-type").map(|(_, v)| v.as_str()),
            Some("application/json; charset=utf-8")
        );
        let other = server.get("/bounty/p2", Some(&player("p1"))).await;
        assert_status(&other, 403);
        assert_eq!(other.body, "");
        assert_eq!(server.get("/bounty/p2", GS).await.body, empty_state());

        // only a game server writes
        let save = json!({ "bounties": [season("43_1_1_Challenge_Season_A-season43-0", 3, false)], "draft_data_weekly": {
            "current_draft_choices": [], "previous_draft_selections": [], "bronze_count": 0, "silver_count": 0, "gold_count": 1 } });
        let refused = server.send_json("POST", "/bounty/p1", Some(&player("p1")), &save).await;
        assert_status(&refused, 403);
        assert_eq!(refused.body, "");
        let saved = server.send_json("POST", "/bounty/p1", GS, &save).await;
        assert_status(&saved, 200);
        let payload = &saved.json()["payload"];
        assert_eq!(payload["bounties"][0]["objectives"][0]["progress"], 3);
        assert_eq!(payload["draft_data_weekly"]["gold_count"], 1);
        // wire key order is Node's
        assert!(saved.body.starts_with(r#"{"code":null,"message":"OK","payload":{"season_start_date":"#), "{}", saved.body);
        let loaded = server.get("/bounty/p1", Some(&player("p1"))).await;
        assert_eq!(loaded.body, saved.body, "a save answers the full merged state, as the GET does");

        // a re-seed at progress 0 can't erase progress
        let reseed = json!({ "bounties": [season("43_1_1_Challenge_Season_A-season43-0", 0, false)] });
        let again = server.send_json("POST", "/bounty/p1", GS, &reseed).await;
        assert_eq!(again.json()["payload"]["bounties"][0]["objectives"][0]["progress"], 3);

        // refused bodies
        let bad = server.send_json("POST", "/bounty/p1", GS, &json!({ "bounties": {} })).await;
        assert_status(&bad, 400);
        assert_eq!(bad.body, r#"{"code":null,"message":"Bad Request","payload":{}}"#);
        let malformed = server.call("POST", "/bounty/p1", GS, Some("{nope")).await;
        assert_status(&malformed, 400);
        assert!(malformed.body.contains("<pre>Bad Request</pre>"), "{}", malformed.body);
        let no_body = server.call("POST", "/bounty/p1", GS, None).await;
        assert_status(&no_body, 400);

        // the no-player sentinel and an unknown account store nothing
        let sentinel = server.send_json("POST", "/bounty/INVALID", GS, &save).await;
        assert_eq!(sentinel.body, empty_state());
        assert_eq!(server.get("/bounty/INVALID", GS).await.body, empty_state());
        let unknown = server.send_json("POST", "/bounty/nobody", GS, &save).await;
        assert_eq!(unknown.body, empty_state());

        // a player-host runtime is a host writer: rifts, dedicated-only here, refuses it
        let host = server.send_json("POST", "/bounty/p1", Some("host:player-1"), &save).await;
        assert_status(&host, 403);

        // a claim deletes the bounty first; season challenges never go
        let base = json!({ "bounty_id": "Bounty_Bronze_X", "drafted_timestamp": "2026-10-10T00:00:00.000Z", "claimed": false,
            "objectives": [{ "objective_id": "A", "progress": 1 }] });
        server.send_json("POST", "/bounty/p1", GS, &json!({ "bounties": [base], "draft_data": json!({}) })).await;
        let deleted = server
            .send_json(
                "DELETE",
                "/bounty/delete/p1",
                GS,
                &json!({ "bounty_ids": ["Bounty_Bronze_X", "43_1_1_Challenge_Season_A-season43-0"] }),
            )
            .await;
        assert_status(&deleted, 200);
        let deleted_json = deleted.json();
        let left: Vec<&str> = deleted_json["payload"]["bounties"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["bounty_id"].as_str().unwrap())
            .collect();
        assert_eq!(left, ["43_1_1_Challenge_Season_A-season43-0"]);
        // the game's reset: no body, nothing changes
        let reset = server.call("POST", "/bounty/delete/p1", GS, None).await;
        assert_eq!(reset.body, r#"{"code":null,"message":"OK","payload":{}}"#);
        let nothing = server.send_json("POST", "/bounty/delete/p1", GS, &json!({ "bounty_ids": [] })).await;
        assert_status(&nothing, 400);
        assert_status(&server.send_json("POST", "/bounty/delete/p1", Some(&player("p1")), &json!({ "bounty_ids": ["x"] })).await, 403);
        assert_eq!(
            server.send_json("POST", "/bounty/delete/INVALID", GS, &json!({ "bounty_ids": ["x"] })).await.body,
            empty_state()
        );

        // the stored row and its change log
        let account = make_account_if_missing(&server, "p1").await;
        let record = server
            .store
            .read(move |conn, repos| repos.bounty_states.get(conn, &account))
            .await
            .unwrap()
            .unwrap();
        assert!(record.update_version >= 3);
        let log = server.store.database().changes(0, 1000).await.unwrap();
        assert!(log.iter().any(|e| e.table_name == "bounty_states" && e.op == "insert"));
        assert!(log.iter().any(|e| e.table_name == "bounty_states" && e.op == "update"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cooldowns() {
    on_each_engine(|target, dir| async move {
        let server = boot(&target, &dir, &[]).await;
        make_account(&server.store, "p1").await;
        let sprout = "58E91700491B00FA7DB373809E250E45";

        assert_eq!(server.get("/cooldown/p1", Some(&player("p1"))).await.body, r#"{"code":null,"message":"OK","payload":{}}"#);
        assert_status(&server.get("/cooldown/p1", None).await, 401);

        let batch = json!({ "cooldowns": [
            { "cooldown_id": "bounty_daily", "cooldown_started_date": "2026-10-09T17:00:00.000Z" },
            { "cooldown_id": "season43", "cooldown_started_date": "2026-10-22T17:00:00Z" } ] });
        assert_status(&server.send_json("PUT", "/cooldown/batch/p1", Some(&player("p1")), &batch).await, 403);
        let put = server.send_json("PUT", "/cooldown/batch/p1", GS, &batch).await;
        assert_eq!(
            put.body,
            r#"{"code":null,"message":"OK","payload":{"bounty_daily":"2026-10-09T17:00:00.000Z","season43":"2026-10-22T17:00:00Z"}}"#
        );
        assert_eq!(server.get("/cooldown/p1", Some(&player("p1"))).await.body, put.body);
        assert_status(&server.get("/cooldown/p1", Some(&player("p2"))).await, 403);
        assert_eq!(server.get("/cooldown/p1", GS).await.body, put.body);

        let bad = server.send_json("PUT", "/cooldown/batch/p1", GS, &json!({ "cooldowns": [{ "cooldown_id": "x" }] })).await;
        assert_status(&bad, 400);
        assert_eq!(bad.body, r#"{"code":null,"message":"Bad Request","payload":{}}"#);
        assert_eq!(
            server.send_json("PUT", "/cooldown/batch/INVALID", GS, &batch).await.body,
            r#"{"code":null,"message":"OK","payload":{}}"#
        );
        assert_status(&server.send_json("PUT", "/cooldown/batch/p1", Some("host:player-1"), &batch).await, 403);

        // StartCooldownEndpoint: the backend's own now, any body ignored
        let started = server.call("PUT", &format!("/cooldown/p1/{sprout}"), GS, None).await;
        assert_status(&started, 200);
        let map = started.json()["payload"].clone();
        assert!(map[sprout].as_str().unwrap().ends_with('Z') && map["bounty_daily"].is_string(), "{map}");
        assert_status(&server.send_json("POST", &format!("/cooldown/p1/{sprout}"), GS, &json!({ "x": 1 })).await, 200);
        assert_status(&server.call("PUT", "/cooldown/p1/bad%20id", GS, None).await, 400);
        assert_eq!(
            server.call("PUT", "/cooldown/INVALID/abc", GS, None).await.body,
            r#"{"code":null,"message":"Bad Request","payload":{}}"#
        );
        let denied = server.call("PUT", "/cooldown/p1/abc", Some(&player("p1")), None).await;
        assert_status(&denied, 403);
        assert_eq!(denied.body, "");
        assert_status(&server.call("PUT", "/cooldown/p1/abc", None, None).await, 401);
        assert_status(&server.call("PUT", "/cooldown/nobody/abc", GS, None).await, 404);
        // "batch" in the user slot is left to the batch route; POST has none
        let batch_post = server.call("POST", "/cooldown/batch/p1", GS, None).await;
        assert_status(&batch_post, 404);
        assert_eq!(batch_post.body, "");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn persistence_off_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let target = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let server = boot(
        &target,
        dir.path(),
        &[
            ("MYSTICPARADOX_BOUNTY_PERSISTENCE", "off"),
            ("MYSTICPARADOX_COOLDOWN_PERSISTENCE", "OFF"),
        ],
    )
    .await;
    make_account(&server.store, "p1").await;
    let save = json!({ "bounties": [season("43_1_1_Challenge_Season_A-season43-0", 3, true)] });
    assert_eq!(
        server.send_json("POST", "/bounty/p1", GS, &save).await.body,
        empty_state()
    );
    assert_eq!(server.get("/bounty/p1", GS).await.body, empty_state());
    let batch = json!({ "cooldowns": [{ "cooldown_id": "a", "cooldown_started_date": "2026-10-09T17:00:00.000Z" }] });
    let empty = r#"{"code":null,"message":"OK","payload":{}}"#;
    assert_eq!(
        server.send_json("PUT", "/cooldown/batch/p1", GS, &batch).await.body,
        empty
    );
    assert_eq!(server.get("/cooldown/p1", GS).await.body, empty);
    // the start never answers 2xx without storing
    let start = server.call("PUT", "/cooldown/p1/abc", GS, None).await;
    assert_status(&start, 503);
    assert_eq!(
        start.body,
        r#"{"code":null,"message":"Service Unavailable","payload":{}}"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wallet_balance_and_reconcile() {
    on_each_engine(|target, dir| async move {
        let server = boot(&target, &dir, &[]).await;
        make_account(&server.store, "p1").await;

        // the first access creates the wallet with the starter grant
        let balance = server.get("/balance", Some(&player("p1"))).await;
        assert_status(&balance, 200);
        let json = balance.json();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 52 + 5 + 5, "{keys:?}");
        assert_eq!(keys[0], "id_currency_s20_coin");
        assert_eq!(keys[9], "CURRENCY_NOTES");
        assert_eq!(json["CURRENCY_NOTES"], 5000);
        assert_eq!(json["id_currency_notes"], 5000);
        assert_eq!(json["id_currency_weapon_token"], 25);
        assert_eq!(json["CURRENCY_PJM_WEAPON"], 50);
        assert_eq!(json["id_currency_pjm_fishing"], 50);
        assert_eq!(json["CURRENCY_PLATINUM"], 0);
        // the starter currencies not in the captured list come after it, in the wallet's order
        assert_eq!(keys[52], "CURRENCY_PJM_WEAPON");
        assert!(
            balance
                .body
                .contains(r#""CURRENCY_NOTES":5000,"id_currency_event_frostfall":0"#)
        );

        // platinum buckets collapse into the one key the client reads
        credit(&server, "p1", "CURRENCY_PLATINUM_UNIV", 50.0).await;
        credit(&server, "p1", "CURRENCY_PLATINUM_EPIC", 20.0).await;
        let after = server.get("/balance", Some(&player("p1"))).await.json();
        assert_eq!(
            (
                after["CURRENCY_PLATINUM"].clone(),
                after["id_currency_platinum"].clone()
            ),
            (json!(70), json!(70))
        );
        assert_eq!(after["CURRENCY_PLATINUM_UNIV"], 50);

        // reconcile: the wallet alone, both forms, in the wallet's order
        let reconcile = server.call("POST", "/reconcile", Some(&player("p1")), None).await;
        assert_status(&reconcile, 200);
        let r = reconcile.json();
        assert_eq!(r["refreshInventory"], true);
        let first = r["balances"].as_object().unwrap().keys().next().unwrap().clone();
        assert_eq!(first, "CURRENCY_NOTES");
        assert_eq!(r["balances"]["CURRENCY_PLATINUM"], 70);
        assert!(
            reconcile
                .body
                .starts_with(r#"{"balances":{"CURRENCY_NOTES":5000,"id_currency_notes":5000,"#),
            "{}",
            reconcile.body
        );
        assert!(reconcile.body.ends_with(r#","refreshInventory":true}"#));

        // no account (or the sentinel) is an empty wallet: the captured list with Rams at 0
        let gs = server.get("/balance", GS).await.json();
        assert_eq!(gs["CURRENCY_NOTES"], 0);
        assert_eq!(gs.as_object().unwrap().len(), 52);
        assert_status(&server.get("/balance", None).await, 401);

        let creator = server.get("/creator", Some(&player("p1"))).await;
        assert_eq!(
            creator.body,
            r#"{"expirationDate":"2099-01-01T01:00:00.041Z","slug":"MROWMROW","success":true}"#
        );
    })
    .await;
}

fn skus_ids(reply: &Reply) -> Vec<String> {
    reply
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn buy(server: &Server, user: &str, currency: &str, sku: &str) -> Reply {
    let token = server
        .get(&format!("/token/{currency}/{sku}"), Some(&player(user)))
        .await;
    assert_status(&token, 200);
    let token = token.json()["purchaseToken"].as_str().unwrap().to_owned();
    server
        .call(
            "POST",
            &format!("/notification/{currency}?token={token}"),
            Some(&player(user)),
            None,
        )
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_store() {
    on_each_engine(|target, dir| async move {
        let server = boot(&target, &dir, &[]).await;
        make_account(&server.store, "p1").await;
        make_account(&server.store, "p2").await;

        // listings
        let missing = server.get("/product/skus/public", Some(&player("p1"))).await;
        assert_status(&missing, 400);
        assert_eq!(
            missing.body,
            r#"{"code":"400","message":"missing requiredTags query parameter"}"#
        );
        assert_status(
            &server
                .get("/product/skus/public?requiredTags=", Some(&player("p1")))
                .await,
            400,
        );
        assert_status(
            &server
                .get(
                    "/product/skus/public?requiredTags=a&requiredTags=b",
                    Some(&player("p1")),
                )
                .await,
            400,
        );
        assert_status(
            &server
                .get("/product/skus/public?requiredTags=ladyluckstore", None)
                .await,
            401,
        );
        assert_eq!(
            server
                .get("/product/skus/public?requiredTags=mailbox", Some(&player("p1")))
                .await
                .body,
            "[]"
        );
        let ladyluck = server
            .get("/product/skus/public?requiredTags=ladyluckstore", Some(&player("p1")))
            .await;
        assert_eq!(skus_ids(&ladyluck), ["ll_sword", "ll_tonic"]);
        let first = &ladyluck.json()[0];
        assert_eq!(first["remaining"], 1);
        assert_eq!(ladyluck.json()[1]["remaining"], 999);
        assert_eq!(
            first["prices"],
            json!([{ "currencyId": "id_currency_marks_steel", "price": 50, "salesPrice": null, "multiPrice": null }])
        );
        let keys: Vec<&String> = first.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "id",
                "displayName",
                "displayDescription",
                "displayPriority",
                "prices",
                "maxAllowed",
                "remaining",
                "duplicateInstancedItems",
                "images",
                "tags",
                "scheduledTags",
                "items",
                "entitlements",
                "skuProgression",
                "loadoutSlots",
                "availableFrom",
                "availableTo",
                "timeAvailabilityReason",
                "platformOfferId",
                "missingEntitlementNames"
            ]
        );
        // Platinum is free unless the host says otherwise
        let web = server
            .get("/product/skus/public?requiredTags=webstore", Some(&player("p1")))
            .await;
        assert_eq!(skus_ids(&web), ["dye_red", "plat_cape"]);
        assert_eq!(web.json()[0]["prices"][0]["price"], 0);
        assert_eq!(
            skus_ids(
                &server
                    .get("/product/skus/public?requiredTags=dyes", Some(&player("p1")))
                    .await
            ),
            ["dye_red"]
        );
        assert_eq!(
            skus_ids(
                &server
                    .get(
                        "/product/skus/public?requiredTags=exchange_vendor_slot_2",
                        Some(&player("p1"))
                    )
                    .await
            ),
            ["single_exchange_slot_2"]
        );
        assert_eq!(
            skus_ids(
                &server
                    .get("/product/skus/public?requiredTags=season_store", Some(&player("p1")))
                    .await
            ),
            ["rc_hat"]
        );

        // the purchase flow: token, then redeem
        let token = server.get("/token/markssteel/ll_sword", Some(&player("p1"))).await;
        assert_status(&token, 200);
        let unknown = server.get("/token/markssteel/nope", Some(&player("p1"))).await;
        assert_status(&unknown, 404);
        assert_eq!(unknown.body, r#"{"code":"404","message":"unknown_sku"}"#);
        // a bare item catalog id finds the one SKU it belongs to
        assert_status(
            &server.get("/token/markssteel/WP_SWORD", Some(&player("p1"))).await,
            200,
        );
        assert_status(&server.get("/token/markssteel/ll_sword", None).await, 401);

        let denied = buy(&server, "p1", "markssteel", "ll_sword").await;
        assert_status(&denied, 402);
        assert_eq!(denied.body, r#"{"code":"402","message":"insufficient_balance"}"#);
        credit(&server, "p1", "CURRENCY_MARKS_STEEL", 120.0).await;
        let bought = buy(&server, "p1", "markssteel", "ll_sword").await;
        assert_status(&bought, 204);
        assert_eq!(bought.body, "");
        // a one-time SKU replays its ledger row: success again, charged once
        assert_status(&buy(&server, "p1", "markssteel", "ll_sword").await, 204);
        assert_eq!(
            server.get("/balance", Some(&player("p1"))).await.json()["CURRENCY_MARKS_STEEL"],
            70
        );
        // a repeatable one is charged each time
        assert_status(&buy(&server, "p1", "markssteel", "ll_tonic").await, 204);
        assert_status(&buy(&server, "p1", "markssteel", "ll_tonic").await, 204);
        assert_eq!(
            server.get("/balance", Some(&player("p1"))).await.json()["CURRENCY_MARKS_STEEL"],
            60
        );
        let call = server.inventory.calls.lock().unwrap()[1].clone();
        assert_eq!(
            call.stacked_to_remove,
            [json!({ "catalogId": "CURRENCY_MARKS_STEEL", "quantity": 50 })]
        );
        assert_eq!(call.instanced_to_add[0]["catalogId"], "WP_SWORD");
        assert_eq!(call.instanced_to_add[0]["instanceId"].as_str().unwrap().len(), 26);

        // redeem refusals
        let token = token.json()["purchaseToken"].as_str().unwrap().to_owned();
        let none = server
            .call("POST", "/notification/markssteel", Some(&player("p1")), None)
            .await;
        assert_eq!(
            (none.status, none.body.as_str()),
            (400, r#"{"code":"400","message":"missing token"}"#)
        );
        let bad = server
            .call(
                "POST",
                "/notification/markssteel?token=garbage",
                Some(&player("p1")),
                None,
            )
            .await;
        assert_eq!(
            (bad.status, bad.body.as_str()),
            (401, r#"{"code":"401","message":"invalid_token"}"#)
        );
        let stolen = server
            .call(
                "POST",
                &format!("/notification/markssteel?token={token}"),
                Some(&player("p2")),
                None,
            )
            .await;
        assert_eq!(
            (stolen.status, stolen.body.as_str()),
            (403, r#"{"code":"403","message":"token_user_mismatch"}"#)
        );
        assert_status(
            &server
                .call("POST", &format!("/notification/markssteel?token={token}"), None, None)
                .await,
            401,
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn platinum_prices_entitlements_and_the_fountain() {
    on_each_engine(|target, dir| async move {
        let server = boot(&target, &dir, &[("PLATINUM_STORE_FREE", "0")]).await;
        make_account(&server.store, "p1").await;
        make_account(&server.store, "p2").await;

        assert_eq!(
            server.get("/entitlementsv2", Some(&player("p1"))).await.body,
            r#"{"entitlements":[]}"#
        );
        assert_status(&server.get("/entitlementsv2", None).await, 401);
        let web = server
            .get("/product/skus/public?requiredTags=webstore", Some(&player("p1")))
            .await;
        assert_eq!(web.json()[0]["prices"][0]["price"], 100);

        // an entitlement SKU: the platinum debit splits across buckets and the entitlement commits with it
        let poor = buy(&server, "p1", "platinum", "single_exchange_slot_2").await;
        assert_eq!(
            (poor.status, poor.body.as_str()),
            (402, r#"{"code":"402","message":"insufficient_balance"}"#)
        );
        credit(&server, "p1", "CURRENCY_PLATINUM_UNIV", 400.0).await;
        credit(&server, "p1", "CURRENCY_PLATINUM_EPIC", 700.0).await;
        assert_status(&buy(&server, "p1", "platinum", "single_exchange_slot_2").await, 204);
        let balance = server.get("/balance", Some(&player("p1"))).await.json();
        assert_eq!(balance["CURRENCY_PLATINUM"], 100);
        assert_eq!(balance["CURRENCY_PLATINUM_UNIV"], 0);
        assert_eq!(balance["CURRENCY_PLATINUM_EPIC"], 100);
        assert_status(&buy(&server, "p1", "platinum", "single_exchange_slot_2").await, 204);
        assert_eq!(
            server.get("/balance", Some(&player("p1"))).await.json()["CURRENCY_PLATINUM"],
            100,
            "charged once"
        );
        let entitlements = server.get("/entitlementsv2", Some(&player("p1"))).await;
        assert_eq!(
            entitlements.body,
            r#"{"entitlements":[{"activatedDate":null,"duration":0,"name":"exchange_slot_2"}]}"#
        );
        assert_eq!(
            server.get("/entitlementsv2", Some(&player("p2"))).await.body,
            r#"{"entitlements":[]}"#
        );
        assert_eq!(
            server
                .send_json("POST", "/entitlementv2/p1", Some(&player("p1")), &json!({}))
                .await
                .body,
            r#"{"code":null,"message":"OK","payload":[]}"#
        );

        // the fountain: free, once per 17:00 UTC window, absent once claimed, per player
        let listing = |user: &'static str| {
            let server = &server;
            async move {
                skus_ids(
                    &server
                        .get(
                            "/product/skus/public?requiredTags=fountain_daily_free_bundle",
                            Some(&player(user)),
                        )
                        .await,
                )
            }
        };
        assert_eq!(listing("p1").await, ["fountain_bundle"]);
        assert_status(&buy(&server, "p1", "platinum", "fountain_bundle").await, 204);
        assert!(listing("p1").await.is_empty(), "claimed: absent until the reset");
        assert_eq!(listing("p2").await, ["fountain_bundle"], "per player");
        // a second claim in the window replays and grants nothing
        assert_status(&buy(&server, "p1", "platinum", "fountain_bundle").await, 204);
        assert_eq!(
            server.get("/balance", Some(&player("p1"))).await.json()["CURRENCY_NOTES"],
            5500
        );
        // a zero price never reaches the wallet
        let first = server
            .inventory
            .calls
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.sku_id == "fountain_bundle")
            .cloned()
            .unwrap();
        assert!(first.stacked_to_remove.is_empty());
        // the listing fails closed when the ledger can't be read, and without a player
        *server.inventory.broken_ledger.lock().unwrap() = Some(());
        assert!(listing("p2").await.is_empty());
        *server.inventory.broken_ledger.lock().unwrap() = None;
        assert_eq!(
            server
                .get(
                    "/product/skus/public?requiredTags=fountain_daily_free_bundle",
                    Some("gs")
                )
                .await
                .body,
            "[]"
        );
    })
    .await;
}
