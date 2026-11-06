// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The bounties-store routes (bounties spec §12.2), black-box through core's `app()`.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{Harness, Options, Who, settings};
use rifts_seam::HostWriteKind;
use serde_json::{Value, json};

const GS: Who = Who::Gameserver;
const WEEKLY: &str = "43_1_1_Challenge_Season_Kill-season43-0";

async fn store_harness(extra: &[(&'static str, &str)]) -> Harness {
    Harness::new(Options {
        files: rustic_isles_economy::test_support::store_files(),
        settings: settings(extra),
        ..Options::default()
    })
    .await
}

#[tokio::test]
async fn bounty_routes() {
    let h = Harness::bare().await;
    h.character("player-1").await;
    h.character("player-2").await;
    let game_data = h.get("/bounty/game-data", &Who::Player("player-1")).await.ok();
    assert_eq!(game_data["payload"]["max_slots"], 4);
    assert_eq!(game_data["payload"].as_object().unwrap().len(), 19);
    h.get("/bounty/player-1", &Who::Nobody).await.assert_empty(401);
    let empty = h.get("/bounty/player-1", &Who::Player("player-1")).await.ok();
    assert_eq!(empty["payload"]["bounties"], json!([]));
    h.get("/bounty/player-2", &Who::Player("player-1"))
        .await
        .assert_empty(403);
    h.get("/bounty/player-2", &GS).await.ok();
    h.post("/bounty/player-1", &Who::Player("player-1"), &json!({}))
        .await
        .assert_empty(403);
    let seed = json!({"bounties": [{"bounty_id": WEEKLY, "drafted_timestamp": "2026-10-01T00:00:00.000Z",
        "objectives": [{"objective_id": "o", "progress": 5}]}], "draft_data_weekly": {"current_draft_choices": []}});
    let saved = h.post("/bounty/player-1", &GS, &seed).await.ok();
    assert_eq!(saved["payload"]["bounties"][0]["bounty_id"], WEEKLY);
    assert_eq!(h.get("/bounty/player-1", &Who::Player("player-1")).await.ok(), saved);
    let lower = json!({"bounties": [{"bounty_id": WEEKLY, "drafted_timestamp": "2026-10-01T00:00:00.000Z",
        "objectives": [{"objective_id": "o", "progress": 2}], "claimed": true}]});
    let claimed = h.post("/bounty/player-1", &GS, &lower).await.ok();
    assert_eq!(claimed["payload"]["bounties"][0]["objectives"][0]["progress"], 5);
    assert_eq!(claimed["payload"]["bounties"][0]["claimed"], true);
    let bad = r#"{"code":null,"message":"Bad Request","payload":{}}"#;
    h.post("/bounty/player-1", &GS, &json!({"bounties": 5}))
        .await
        .assert(400, bad);
    h.post("/bounty/player-1", &GS, &json!([])).await.assert(400, bad);
    assert_eq!(h.send("POST", "/bounty/player-1", &GS, Some("{nope")).await.status, 400);
    h.send("POST", "/bounty/player-1", &GS, None).await.assert(400, bad);
    assert_eq!(
        h.post("/bounty/INVALID", &GS, &seed).await.ok()["payload"]["bounties"],
        json!([])
    );
    // Deletes: a named base bounty goes, a season challenge stays
    let base = json!({"bounties": [{"bounty_id": "Base_1", "claimed": true}]});
    h.post("/bounty/player-1", &GS, &base).await.ok();
    let deleted = h
        .post(
            "/bounty/delete/player-1",
            &GS,
            &json!({"bounty_ids": ["Base_1", WEEKLY]}),
        )
        .await
        .ok();
    let ids: Vec<_> = deleted["payload"]["bounties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["bounty_id"].clone())
        .collect();
    assert_eq!(ids, [json!(WEEKLY)]);
    h.send("POST", "/bounty/delete/player-1", &GS, None)
        .await
        .assert(200, r#"{"code":null,"message":"OK","payload":{}}"#);
    h.post("/bounty/delete/player-1", &GS, &json!({}))
        .await
        .assert(200, r#"{"code":null,"message":"OK","payload":{}}"#);
    h.post("/bounty/delete/player-1", &Who::Player("player-1"), &json!({}))
        .await
        .assert_empty(403);
}

#[tokio::test]
async fn host_bounties() {
    let h = Harness::bare().await;
    h.character("player-1").await;
    let host = Who::Host("cap-ok");
    let read = h.get("/bounty/player-1", &host).await;
    assert_eq!(read.status, 200);
    let invented =
        json!({"bounties": [{"bounty_id": "43_1_1_Challenge_Season_Kill-season43-5", "drafted_timestamp": "x"}]});
    let saved = h.post("/bounty/player-1", &host, &invented).await.ok();
    assert_eq!(saved["payload"]["bounties"], json!([]));
    let tags = h.guard.tags();
    let (kind, _, summary) = tags.last().unwrap();
    assert_eq!(*kind, HostWriteKind::Bounty);
    assert_eq!(summary["ignoredWeekly"], 1);
    h.get("/bounty/player-1", &Who::Host("cap-bare")).await.ok();
    h.post("/bounty/player-1", &Who::Host("cap-bare"), &invented)
        .await
        .assert_empty(403);
}

#[tokio::test]
async fn cooldown_routes() {
    let h = Harness::bare().await;
    h.character("player-1").await;
    h.character("player-2").await;
    let batch = json!({"cooldowns": [
        {"cooldown_id": "Daily", "cooldown_started_date": "2026-10-01T17:00:00.000Z"},
        {"cooldown_id": "7", "cooldown_started_date": "2026-10-01T17:00:00.000Z"},
    ]});
    h.send(
        "PUT",
        "/cooldown/batch/player-1",
        &Who::Player("player-1"),
        Some(&batch.to_string()),
    )
    .await
    .assert_empty(403);
    let saved = h
        .send("PUT", "/cooldown/batch/player-1", &GS, Some(&batch.to_string()))
        .await;
    assert_eq!(
        saved.body,
        r#"{"code":null,"message":"OK","payload":{"7":"2026-10-01T17:00:00.000Z","Daily":"2026-10-01T17:00:00.000Z"}}"#
    );
    let mine = h.get("/cooldown/player-1", &Who::Player("player-1")).await.ok();
    assert_eq!(mine["payload"]["Daily"], "2026-10-01T17:00:00.000Z");
    h.get("/cooldown/player-1", &Who::Player("player-2"))
        .await
        .assert_empty(403);
    let bad = r#"{"code":null,"message":"Bad Request","payload":{}}"#;
    h.send(
        "PUT",
        "/cooldown/batch/player-1",
        &GS,
        Some(r#"{"cooldowns":[{"cooldown_id":""}]}"#),
    )
    .await
    .assert(400, bad);
    h.send("PUT", "/cooldown/batch/INVALID", &GS, Some(&batch.to_string()))
        .await
        .assert(200, r#"{"code":null,"message":"OK","payload":{}}"#);
    let started = h.send("PUT", "/cooldown/player-1/Sprout_1", &GS, None).await.ok();
    assert_eq!(
        started["payload"]["Sprout_1"],
        rustic_isles::season::to_iso_string(common::NOW_MS)
    );
    let backdated = h
        .post(
            "/cooldown/player-1/Sprout_2",
            &GS,
            &json!({"cooldown_started_date": "2020-01-01T00:00:00.000Z"}),
        )
        .await
        .ok();
    assert_eq!(
        backdated["payload"]["Sprout_2"],
        rustic_isles::season::to_iso_string(common::NOW_MS)
    );
    h.send("PUT", "/cooldown/player-1/bad%20id", &GS, None)
        .await
        .assert(400, bad);
    h.send("PUT", "/cooldown/INVALID/x", &GS, None).await.assert(400, bad);
    h.send("PUT", "/cooldown/player-1/x", &Who::Player("player-1"), None)
        .await
        .assert_empty(403);
    h.send("POST", "/cooldown/batch/player-1", &Who::Nobody, None)
        .await
        .assert_empty(404);
    let off = Harness::new(Options {
        settings: settings(&[("RUSTIC_ISLES_COOLDOWN_PERSISTENCE", "OFF")]),
        ..Options::default()
    })
    .await;
    off.send("PUT", "/cooldown/player-1/x", &GS, None)
        .await
        .assert(503, r#"{"code":null,"message":"Service Unavailable","payload":{}}"#);
}

#[tokio::test]
async fn wallet_routes() {
    let h = Harness::bare().await;
    h.character("p1").await;
    h.get("/entitlementsv2", &Who::Player("p1"))
        .await
        .assert(200, r#"{"entitlements":[]}"#);
    h.post("/entitlementv2/x", &Who::Player("p1"), &json!({}))
        .await
        .assert(200, r#"{"code":null,"message":"OK","payload":[]}"#);
    let balance = h.get("/balance", &Who::Player("p1")).await.ok();
    let map = balance.as_object().unwrap();
    assert_eq!(map["CURRENCY_NOTES"], 5000);
    assert_eq!(map["id_currency_notes"], 5000);
    assert_eq!(map["CURRENCY_WEAPON_TOKEN"], 25);
    assert_eq!(map["id_currency_pjm_weapon"], 50);
    assert_eq!(map.keys().take(52).count(), 52);
    let gs = h.get("/balance", &GS).await.ok();
    assert_eq!(gs["CURRENCY_NOTES"], 0);
    let reconcile = h.post("/reconcile", &Who::Player("p1"), &json!({})).await.ok();
    assert_eq!(reconcile["refreshInventory"], true);
    assert_eq!(reconcile["balances"]["CURRENCY_NOTES"], 5000);
    assert!(reconcile["balances"].get("CURRENCY_WEAPON_TOKEN").is_none());
    h.get("/creator", &Who::Player("p1")).await.assert(
        200,
        r#"{"expirationDate":"2099-01-01T01:00:00.041Z","slug":"MROWMROW","success":true}"#,
    );
}

/// The system clock: a token's expiry is checked on it while its issue time is the replay clock's (spec §14 item 5)
fn live() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}

async fn at_live(h: &Harness, method: &str, uri: &str, user: &'static str) -> common::Answer {
    let now = live();
    h.send_with(
        method,
        uri,
        &Who::Player(user),
        None,
        &[("x-isles-replay-now-ms", &now)],
    )
    .await
}

async fn token(h: &Harness, user: &'static str, sku: &str) -> String {
    at_live(h, "GET", &format!("/token/x/{sku}"), user).await.ok()["purchaseToken"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn buy(h: &Harness, user: &'static str, sku: &str) -> common::Answer {
    let token = token(h, user, sku).await;
    at_live(h, "POST", &format!("/notification/x?token={token}"), user).await
}

#[tokio::test]
async fn store_routes() {
    let h = store_harness(&[]).await;
    h.character("p1").await;
    h.character("p2").await;
    let missing = r#"{"code":"400","message":"missing requiredTags query parameter"}"#;
    for q in ["", "?requiredTags=", "?requiredTags=a&requiredTags=b"] {
        h.get(&format!("/product/skus/public{q}"), &Who::Player("p1"))
            .await
            .assert(400, missing);
    }
    h.get("/product/skus/public?requiredTags=nothing", &Who::Player("p1"))
        .await
        .assert(200, "[]");
    let ladyluck = h
        .get("/product/skus/public?requiredTags=ladyluckstore", &Who::Player("p1"))
        .await
        .ok();
    assert_eq!(ladyluck.as_array().unwrap().len(), 2);
    assert_eq!(ladyluck[0]["remaining"], 1);
    let webstore = h
        .get("/product/skus/public?requiredTags=webstore", &Who::Player("p1"))
        .await
        .ok();
    let dyes = h
        .get("/product/skus/public?requiredTags=dyes", &Who::Player("p1"))
        .await
        .ok();
    assert_eq!(webstore[0]["id"], "plat_dye");
    assert_eq!(dyes[0]["id"], "plat_dye");
    assert_eq!(webstore[0]["prices"][0]["price"], 0);
    let slot = h
        .get(
            "/product/skus/public?requiredTags=exchange_vendor_slot_2",
            &Who::Player("p1"),
        )
        .await
        .ok();
    assert_eq!(slot[0]["id"], "single_exchange_slot_2");
    let fountain = h
        .get(
            "/product/skus/public?requiredTags=fountain_daily_free_bundle",
            &Who::Player("p1"),
        )
        .await
        .ok();
    assert_eq!(fountain[0]["id"], "fountain_1");
    // Tokens
    let claims = h
        .shared
        .auth()
        .validate_store_token(&token(&h, "p1", "ll_one").await)
        .unwrap();
    assert_eq!(
        (claims.sku_id.as_str(), claims.store_tag.as_deref()),
        ("ll_one", Some("ladyluckstore"))
    );
    let claims = h
        .shared
        .auth()
        .validate_store_token(&token(&h, "p1", "DYE_TEST").await)
        .unwrap();
    assert_eq!(claims.sku_id, "plat_dye");
    h.get("/token/x/unknown", &Who::Player("p1"))
        .await
        .assert(404, r#"{"code":"404","message":"unknown_sku"}"#);
    let claims = h
        .shared
        .auth()
        .validate_store_token(&token(&h, "p1", "fountain_1").await)
        .unwrap();
    assert_eq!(claims.store_tag.as_deref(), Some("fountain_daily_free_bundle"));
    // Redeeming
    h.send("POST", "/notification/x", &Who::Player("p1"), None)
        .await
        .assert(400, r#"{"code":"400","message":"missing token"}"#);
    h.send("POST", "/notification/x?token=bad", &Who::Player("p1"), None)
        .await
        .assert(401, r#"{"code":"401","message":"invalid_token"}"#);
    let theirs = token(&h, "p2", "ll_one").await;
    at_live(&h, "POST", &format!("/notification/x?token={theirs}"), "p1")
        .await
        .assert(403, r#"{"code":"403","message":"token_user_mismatch"}"#);
    // A one-time SKU: one debit, then a replay
    h.credit("p1", "CURRENCY_MARKS_STEEL", 100.0).await;
    buy(&h, "p1", "ll_one").await.assert_empty(204);
    assert_eq!(h.balance("p1", "CURRENCY_MARKS_STEEL").await, Some(90.0));
    buy(&h, "p1", "ll_one").await.assert_empty(204);
    assert_eq!(h.balance("p1", "CURRENCY_MARKS_STEEL").await, Some(90.0));
    // A repeatable SKU twice
    buy(&h, "p1", "ll_repeat").await.assert_empty(204);
    buy(&h, "p1", "ll_repeat").await.assert_empty(204);
    assert_eq!(h.balance("p1", "CURRENCY_MARKS_STEEL").await, Some(80.0));
    // Above the balance
    h.credit("p2", "CURRENCY_MARKS_STEEL", 1.0).await;
    buy(&h, "p2", "ll_one")
        .await
        .assert(402, r#"{"code":"402","message":"insufficient_balance"}"#);
    // The fountain: claimed once a window, still offered to another player, never to a game server
    buy(&h, "p1", "fountain_1").await.assert_empty(204);
    let fountain = "/product/skus/public?requiredTags=fountain_daily_free_bundle";
    at_live(&h, "GET", fountain, "p1").await.assert(200, "[]");
    assert_eq!(
        at_live(&h, "GET", fountain, "p2").await.ok().as_array().unwrap().len(),
        1
    );
    h.get("/product/skus/public?requiredTags=fountain_daily_free_bundle", &GS)
        .await
        .assert(200, "[]");
    // An exchange slot with free Platinum
    buy(&h, "p1", "single_exchange_slot_2").await.assert_empty(204);
    let held = h.get("/entitlementsv2", &Who::Player("p1")).await.ok();
    assert_eq!(held["entitlements"][0]["name"], "exchange_slot_2");
    h.get("/no/such/route", &Who::Player("p1")).await.assert_empty(404);
}

#[tokio::test]
async fn platinum_buckets() {
    let h = store_harness(&[("PLATINUM_STORE_FREE", "0")]).await;
    h.character("p1").await;
    h.credit("p1", "CURRENCY_PLATINUM_CS", 25.0).await;
    h.credit("p1", "CURRENCY_PLATINUM_UNIV", 50.0).await;
    h.credit("p1", "CURRENCY_PLATINUM_PLATFORM", 1000.0).await;
    buy(&h, "p1", "single_exchange_slot_2").await.assert_empty(204);
    assert_eq!(h.balance("p1", "CURRENCY_PLATINUM_CS").await, Some(0.0));
    assert_eq!(h.balance("p1", "CURRENCY_PLATINUM_UNIV").await, Some(0.0));
    assert_eq!(h.balance("p1", "CURRENCY_PLATINUM_PLATFORM").await, Some(75.0));
    let balance = h.get("/balance", &Who::Player("p1")).await.ok();
    assert_eq!(balance["CURRENCY_PLATINUM"], 75);
    // A rank skip raises the Hunt Pass by one rank
    h.credit("p1", "CURRENCY_PLATINUM_PLATFORM", 1000.0).await;
    buy(&h, "p1", "plat_skip").await.assert_empty(204);
    assert_eq!(h.track("p1", common::PASS).await.unwrap().progress, 100);
    let _ = Value::Null;
}
