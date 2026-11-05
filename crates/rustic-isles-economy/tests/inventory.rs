// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The inventory and reforge routes (inventory spec §13.2, §13.3), black-box through core's `app()`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Harness, NOW_MS, Options, Who, host_as, settings};
use rifts_seam::{HostWriteKind, Json, Reply};
use serde_json::{Value, json};

const GS: Who = Who::Gameserver;

fn tx(user: &str, character: &str, id: &str, extra: Value) -> Value {
    let mut body = json!({"accountId": user, "characterId": character, "transactionId": id});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

fn stack(id: &str, q: i64) -> Value {
    json!({"catalogId": id, "quantity": q})
}

#[tokio::test]
async fn migration_stub() {
    let h = Harness::bare().await;
    let want = r#"{"code":"NONE","message":""}"#;
    h.send("POST", "/inventory//dauntlessrel-1.14.7:647472", &GS, None)
        .await
        .assert(200, want);
    h.send("POST", "/inventory/c1/changes", &GS, None)
        .await
        .assert(200, want);
    h.send("POST", "/inventory/c1/changes", &Who::Nobody, None)
        .await
        .assert_empty(401);
}

#[tokio::test]
async fn reads() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let other = h.character("p2").await;
    let weapon = json!({"catalogId": "WP_TEST_STARTER", "instanceId": "w1", "itemData": "{\"Name\":\"x\",\"CurrentLevel\":5}", "updateVersion": 1});
    h.set_inventory(&c, &json!([weapon]), &json!([stack("MAT_TEST", 3)]))
        .await;
    let read = h.get(&format!("/inventory/p1/{c}"), &GS).await.ok();
    assert_eq!(read["characterId"], c.as_str());
    let item = read["instancedItems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["instanceId"] == "w1")
        .unwrap()
        .clone();
    assert_eq!(item["itemData"], "{\"Name\":\"x\",\"CurrentLevel\":0}");
    // The stored level is untouched; the wallet is merged over the stacks
    assert!(h.instanced(&c).await.to_string().contains("CurrentLevel\\\":5"));
    let stacked = read["stackedItems"].as_array().unwrap();
    assert!(
        stacked
            .iter()
            .any(|s| s == &json!({"catalogId": "CURRENCY_NOTES", "quantity": 5000}))
    );
    // A player's path user is ignored
    let own = h.get(&format!("/inventory/someone/{c}"), &Who::Player("p1")).await.ok();
    assert_eq!(own["characterId"], c.as_str());
    h.get(&format!("/inventory/p1/{other}"), &GS).await.assert_empty(400);
    h.get("/inventory/p1/nope", &GS).await.assert_empty(400);
    h.get("/inventory/INVALID/c9", &GS)
        .await
        .assert(200, r#"{"characterId":"c9","instancedItems":[],"stackedItems":[]}"#);
    let first = h.get("/inventory/p1/", &GS).await.ok();
    assert_eq!(first["characterId"], c.as_str());
    h.get("/inventory/p1/", &Who::Player("p1")).await.assert_empty(400);
    h.get("/inventory/INVALID/", &GS).await.assert(
        200,
        r#"{"characterId":"INVALID","instancedItems":[],"stackedItems":[]}"#,
    );
}

#[tokio::test]
async fn weapons_keep_levels_when_asked() {
    let h = Harness::new(Options {
        settings: settings(&[("KEEP_WEAPON_ITEM_LEVELS", " true ")]),
        ..Options::default()
    })
    .await;
    let c = h.character("p1").await;
    let weapon = json!({"catalogId": "WP_TEST_STARTER", "instanceId": "w1", "itemData": "{\"CurrentLevel\":5}", "updateVersion": 1});
    h.set_inventory(&c, &json!([weapon]), &json!([])).await;
    let read = h.get(&format!("/inventory/p1/{c}"), &GS).await.ok();
    assert_eq!(read["instancedItems"][0]["itemData"], "{\"CurrentLevel\":5}");
}

#[tokio::test]
async fn grants_replays_and_refusals() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    h.set_inventory(&c, &json!([]), &json!([stack("MAT_TEST", 4)])).await;
    let body = tx(
        "p1",
        &c,
        "t1",
        json!({"addStackedItems": [stack("MAT_TEST", 3), stack("CURRENCY_NOTES", 50)]}),
    );
    let first = h.post("/inventory", &GS, &body).await.ok();
    assert_eq!(
        first,
        json!({"createdInstancedItems": [], "updatedInstancedItems": [],
            "updatedStackedItems": [stack("MAT_TEST", 7), stack("CURRENCY_NOTES", 5050)], "removedInstancedItems": []})
    );
    assert_eq!(h.stacked(&c).await, json!([stack("MAT_TEST", 7)]));
    // The same id and body (keys in another order) replays
    let reordered = json!({"addStackedItems": [{"quantity": 3, "catalogId": "MAT_TEST"}, stack("CURRENCY_NOTES", 50)],
        "transactionId": "t1", "characterId": c, "accountId": "p1"});
    assert_eq!(h.post("/inventory", &GS, &reordered).await.ok(), first);
    assert_eq!(h.balance("p1", "CURRENCY_NOTES").await, Some(5050.0));
    // Another body under the id
    let other = tx("p1", &c, "t1", json!({"addStackedItems": [stack("MAT_TEST", 4)]}));
    h.post("/inventory", &GS, &other).await.assert(
        409,
        r#"{"code":null,"message":"Transaction id reused with a different request body","payload":{}}"#,
    );
    // Removing more than owned leaves no ledger row; a fixed body then applies
    let over = tx("p1", &c, "t2", json!({"removeStackedItems": [stack("MAT_TEST", 8)]}));
    h.post("/inventory", &GS, &over).await.assert(
        409,
        r#"{"code":null,"message":"Insufficient stacked item","payload":{}}"#,
    );
    assert!(!h.ledger_rows("p1").await.iter().any(|r| r["transactionId"] == "t2"));
    let fixed = tx("p1", &c, "t2", json!({"removeStackedItems": [stack("MAT_TEST", 7)]}));
    let answer = h.post("/inventory", &GS, &fixed).await.ok();
    assert_eq!(answer["updatedStackedItems"], json!([stack("MAT_TEST", 0)]));
    // Overspending a currency
    let spend = tx(
        "p1",
        &c,
        "t3",
        json!({"removeStackedItems": [stack("CURRENCY_NOTES", 999_999)]}),
    );
    h.post("/inventory", &GS, &spend)
        .await
        .assert(409, r#"{"code":null,"message":"Insufficient balance","payload":{}}"#);
    assert_eq!(h.balance("p1", "CURRENCY_NOTES").await, Some(5050.0));
}

#[tokio::test]
async fn instanced_rules() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let body = tx(
        "p1",
        &c,
        "m1",
        json!({"addInstancedItems": [{"catalogId": "LT_NEW", "instanceId": "LT_NEW"}]}),
    );
    let created = h.post("/inventory", &GS, &body).await.ok()["createdInstancedItems"][0].clone();
    assert_eq!(created["instanceId"].as_str().unwrap().len(), 26);
    assert_eq!(
        (created["itemData"].clone(), created["updateVersion"].clone()),
        (Value::Null, json!(0))
    );
    // Policy corrections: a stackable sent instanced, a non-stackable sent as quantity 2
    let mixed = tx(
        "p1",
        &c,
        "n1",
        json!({
            "addInstancedItems": [{"catalogId": "MAT_TEST", "instanceId": "m"}],
            "addStackedItems": [stack("WP_TEST_SWORD", 2)],
        }),
    );
    let out = h.post("/inventory", &GS, &mixed).await.ok();
    let ids: Vec<String> = out["createdInstancedItems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["instanceId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(
        ids.iter()
            .all(|id| id.len() == 26 && id.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b)))
    );
    assert_eq!(out["updatedStackedItems"], json!([stack("MAT_TEST", 1)]));
    let mut again = mixed.clone();
    again["transactionId"] = json!("n2");
    let ids2: Vec<Value> = h.post("/inventory", &GS, &again).await.ok()["createdInstancedItems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["instanceId"].clone())
        .collect();
    assert_ne!(Value::from(ids2), json!(ids));
    // Quantities a non-stackable cannot become
    for (q, status) in [
        (json!(0), 500),
        (json!(1001), 500),
        (json!(1.5), 400),
        (json!("x"), 400),
    ] {
        let bad = tx(
            "p1",
            &c,
            "q",
            json!({"addStackedItems": [{"catalogId": "WP_TEST_SWORD", "quantity": q}]}),
        );
        let answer = h.post("/inventory", &GS, &bad).await;
        assert_eq!(answer.status, status, "{q}");
        if status == 400 {
            assert_eq!(
                answer.json()["message"],
                "Invalid inventory request: addStackedItems entry has an invalid quantity"
            );
        }
    }
    // Starter dyes are never stored
    let dyes = tx(
        "p1",
        &c,
        "d1",
        json!({
            "addInstancedItems": [{"catalogId": "DYE_BANNER_SIGIL_DEFAULT", "instanceId": "x"}],
            "addStackedItems": [stack("DYE_BANNER_BACKGROUND_DEFAULT", 1)],
        }),
    );
    let out = h.post("/inventory", &GS, &dyes).await.ok();
    assert_eq!(
        (out["createdInstancedItems"].clone(), out["updatedStackedItems"].clone()),
        (json!([]), json!([]))
    );
}

#[tokio::test]
async fn saves() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let stored = json!({"catalogId": "WP", "instanceId": "i", "itemData": "old", "updateVersion": 3});
    h.set_inventory(&c, &json!([stored]), &json!([])).await;
    let save = |id: &str, item: Value| tx("p1", &c, id, json!({"saveInstancedItems": [item]}));
    let newer = h
        .post(
            "/inventory",
            &Who::Player("p1"),
            &save(
                "s1",
                json!({"catalogId": "WP", "instanceId": "i", "itemData": "new", "updateVersion": 4}),
            ),
        )
        .await
        .ok();
    assert_eq!(newer["updatedInstancedItems"][0]["updateVersion"], 4);
    let stale = h
        .post(
            "/inventory",
            &GS,
            &save(
                "s2",
                json!({"catalogId": "WP", "instanceId": "i", "itemData": "older", "updateVersion": 2}),
            ),
        )
        .await
        .ok();
    assert_eq!(stale["updatedInstancedItems"], json!([]));
    let transmute = h
        .post(
            "/inventory",
            &GS,
            &save(
                "s3",
                json!({"catalogId": "BETTER", "instanceId": "i", "updateVersion": 9}),
            ),
        )
        .await
        .ok();
    assert_eq!(transmute["updatedInstancedItems"], json!([]));
    assert_eq!(h.instanced(&c).await[0]["itemData"], "new");
    // A player may save but not grant
    let grant = tx("p1", &c, "g", json!({"addStackedItems": [stack("MAT_TEST", 1)]}));
    h.post("/inventory", &Who::Player("p1"), &grant).await.assert(
        403,
        r#"{"code":null,"message":"Authoritative inventory mutations require gameserver authority","payload":{}}"#,
    );
}

#[tokio::test]
async fn users_and_bodies() {
    let h = Harness::new(Options {
        settings: settings(&[("GAMESERVER_NO_PLAYER_USER_ID", "p1")]),
        ..Options::default()
    })
    .await;
    let c = h.character("p1").await;
    // An empty character resolves to the first; a bearer-less game server is the configured account
    let body =
        json!({"accountId": "", "characterId": "", "transactionId": "u1", "addStackedItems": [stack("MAT_TEST", 2)]});
    h.post("/inventory", &GS, &body).await.ok();
    assert!(h.stacked(&c).await.as_array().unwrap().contains(&stack("MAT_TEST", 2)));
    let named = json!({"accountId": "p1", "transactionId": "u2", "addStackedItems": [stack("MAT_TEST", 1)]});
    h.post("/inventory", &GS, &named).await.ok();
    // The sentinel echoes whatever was sent (with no configured account to stand in for it)
    let odd = json!({"accountId": "INVALID", "removeInstancedItems": "x", "addStackedItems": null});
    Harness::bare().await.post("/inventory", &GS, &odd).await.assert(
        200,
        r#"{"createdInstancedItems":[],"updatedInstancedItems":[],"updatedStackedItems":[],"removedInstancedItems":"x"}"#,
    );
    h.send("POST", "/inventory", &GS, Some("[1]")).await.assert(
        400,
        r#"{"code":null,"message":"Invalid inventory request: body must be an object","payload":{}}"#,
    );
    assert_eq!(h.send("POST", "/inventory", &GS, Some("{bad")).await.status, 400);
    let no_tx = json!({"accountId": "p1", "characterId": c});
    assert_eq!(
        h.post("/inventory", &GS, &no_tx).await.json()["message"],
        "Invalid inventory request: transactionId must be a non-empty string"
    );
    let no_id = tx("p1", &c, "v", json!({"saveInstancedItems": [{"catalogId": "WP"}]}));
    assert_eq!(
        h.post("/inventory", &GS, &no_id).await.json()["message"],
        "Invalid inventory request: saveInstancedItems entry has an invalid instanceId"
    );
    let form = h
        .send_with(
            "POST",
            "/inventory",
            &GS,
            Some("accountId=p1&characterId=x"),
            &[("content-type", "application/x-www-form-urlencoded")],
        )
        .await;
    assert_eq!(
        form.json()["message"],
        "Invalid inventory request: transactionId must be a non-empty string"
    );
}

#[tokio::test]
async fn no_policy_file() {
    let h = Harness::new(Options {
        no_policy: true,
        ..Options::default()
    })
    .await;
    let c = h.character("p1").await;
    let answer = h.post("/inventory", &GS, &tx("p1", &c, "x", json!({}))).await;
    assert_eq!(answer.status, 500);
}

#[tokio::test]
async fn single_item_saves() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let other = h.character("p2").await;
    let stored = json!({"catalogId": "WP", "instanceId": "i", "itemData": "old", "updateVersion": 3});
    h.set_inventory(&c, &json!([stored]), &json!([])).await;
    let save = |extra: Value| {
        let mut body = json!({"characterId": c, "instanceId": "i", "catalogId": "WP"});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        body
    };
    let me = Who::Player("p1");
    let newer = h
        .post(
            "/inventory/instanceditem",
            &me,
            &save(json!({"itemData": "new", "updateVersion": 4})),
        )
        .await
        .ok();
    assert_eq!(newer["updateVersion"], 4);
    let stale = h
        .post(
            "/inventory/instanceditem",
            &me,
            &save(json!({"itemData": "x", "updateVersion": 1})),
        )
        .await
        .ok();
    assert_eq!(stale["itemData"], "new");
    let same = h
        .post("/inventory/instanceditem", &me, &save(json!({"itemData": "new"})))
        .await
        .ok();
    assert_eq!(same["itemData"], "new");
    let unknown = h
        .post(
            "/inventory/instanceditem",
            &me,
            &save(json!({"instanceId": "nope", "updateVersion": 9})),
        )
        .await;
    assert_eq!(
        (unknown.status, unknown.body.as_str(), unknown.content_type.as_deref()),
        (200, "", Some("application/json; charset=utf-8"))
    );
    let transmute = h
        .post(
            "/inventory/instanceditem",
            &me,
            &save(json!({"catalogId": "BETTER", "updateVersion": 9})),
        )
        .await;
    assert_eq!((transmute.status, transmute.body.as_str()), (200, ""));
    let foreign = h
        .post(
            "/inventory/instanceditem",
            &me,
            &save(json!({"characterId": other, "updateVersion": 9})),
        )
        .await;
    assert_eq!((foreign.status, foreign.body.as_str()), (200, ""));
    h.post(
        "/inventory/instanceditem",
        &GS,
        &json!({"accountId": "INVALID", "instanceId": "i", "itemData": null}),
    )
    .await
    .assert(200, r#"{"instanceId":"i","itemData":null}"#);
    h.send("POST", "/inventory/instanceditem", &me, Some("[]"))
        .await
        .assert(
            400,
            r#"{"code":null,"message":"Invalid item update: body must be an object","payload":{}}"#,
        );
    let missing = h
        .post(
            "/inventory/instanceditem",
            &me,
            &json!({"characterId": c, "catalogId": "WP"}),
        )
        .await;
    assert_eq!(
        missing.json()["message"],
        "Invalid item update: instanceId must be a non-empty string"
    );
    let big = h
        .post("/inventory/instanceditem", &me, &save(json!({"updateVersion": 1e10})))
        .await;
    assert_eq!(
        big.json()["message"],
        "Invalid item update: updateVersion must be a bounded non-negative integer"
    );
}

const AXE: &str = "ExperienceTrack_Weapon_Axe";

#[tokio::test]
async fn reforge() {
    let h = Harness::bare().await;
    h.character("p1").await;
    let conflict = r#"{"code":null,"message":"Conflict","payload":{}}"#;
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &Who::Player("p1"), None)
        .await
        .assert(403, r#"{"code":null,"message":"Forbidden","payload":{}}"#);
    h.send("DELETE", "/progression/p1/ExperienceTrack_PlayerLevel", &GS, None)
        .await
        .assert(409, conflict);
    h.send("DELETE", &format!("/progression/INVALID/{AXE}"), &GS, None)
        .await
        .assert(409, conflict);
    h.send("DELETE", &format!("/progression/nobody/{AXE}"), &GS, None)
        .await
        .assert(409, conflict);
    h.set_progress("p1", AXE, 2000).await;
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &GS, None)
        .await
        .assert(409, conflict);
    h.set_progress("p1", AXE, 4500).await;
    let reset = h
        .send("DELETE", &format!("/progression/p1/{AXE}"), &GS, None)
        .await
        .ok();
    assert_eq!(reset["payload"]["progress"], 0);
    assert_eq!(
        reset["payload"]["confirmed_date"],
        rustic_isles::season::to_iso_string(NOW_MS)
    );
    let stored = h.track("p1", AXE).await.unwrap().reforge.unwrap();
    assert_eq!((stored.banked, stored.cap, stored.prior_progress), (3500, 3000, 6500));
    // A duplicate within two minutes replays
    let again = h
        .send("DELETE", &format!("/progression/p1/{AXE}"), &GS, None)
        .await
        .ok();
    assert_eq!(again, reset);
    assert_eq!(
        h.track("p1", AXE).await.unwrap().reforge.unwrap().reforge_id,
        stored.reforge_id
    );
    // The banked restore
    let bad = r#"{"code":null,"message":"Bad Request","payload":{}}"#;
    for amount in ["0", "1.5", "100001"] {
        h.send("POST", &format!("/progression/purchase/p1/{AXE}/{amount}"), &GS, None)
            .await
            .assert(400, bad);
    }
    h.send("POST", &format!("/progression/purchase/INVALID/{AXE}/5"), &GS, None)
        .await
        .assert(400, bad);
    h.send(
        "POST",
        &format!("/progression/purchase/p1/{AXE}/5"),
        &Who::Player("p1"),
        None,
    )
    .await
    .assert(403, r#"{"code":null,"message":"Forbidden","payload":{}}"#);
    h.send("POST", &format!("/progression/purchase/p1/{AXE}/5"), &GS, None)
        .await
        .assert(409, conflict);
    h.send(
        "POST",
        "/progression/purchase/p1/ExperienceTrack_PlayerLevel/3500",
        &GS,
        None,
    )
    .await
    .assert(409, conflict);
    let restored = h
        .send("POST", &format!("/progression/purchase/p1/{AXE}/3500"), &GS, None)
        .await
        .ok();
    let grant = format!("reforge-{}-banked", stored.reforge_id);
    assert_eq!(restored["payload"]["progress"], 3500);
    assert_eq!(restored["payload"]["grant_id"], grant.as_str());
    assert_eq!(restored["payload"]["replayed"], false);
    let replay = h
        .send("POST", &format!("/progression/purchase/p1/{AXE}/3500"), &GS, None)
        .await
        .ok();
    assert_eq!(replay["payload"]["replayed"], true);
    assert_eq!(h.track("p1", AXE).await.unwrap().progress, 3500);
}

#[tokio::test]
async fn stale_reforge_finishes() {
    let h = Harness::bare().await;
    h.character("p1").await;
    h.set_progress("p1", AXE, 3100).await;
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &GS, None)
        .await
        .ok();
    let later = (NOW_MS + 180_000).to_string();
    let stale = h
        .send_with(
            "DELETE",
            &format!("/progression/p1/{AXE}"),
            &GS,
            None,
            &[("x-isles-replay-now-ms", &later)],
        )
        .await;
    assert_eq!(stale.status, 409);
    assert_eq!(h.track("p1", AXE).await.unwrap().progress, 100);
    assert_eq!(h.track("p1", "PrestigeTrack_Weapon_Axe").await.unwrap().progress, 1);
    assert!(!h.track("p1", AXE).await.unwrap().reforge.unwrap().open);
}

#[tokio::test]
async fn reforge_modes() {
    for mode in ["off", "LOG"] {
        let h = Harness::new(Options {
            settings: settings(&[("RUSTIC_ISLES_REFORGE", mode)]),
            ..Options::default()
        })
        .await;
        h.character("p1").await;
        h.set_progress("p1", AXE, 4000).await;
        assert_eq!(
            h.send("DELETE", &format!("/progression/p1/{AXE}"), &GS, None)
                .await
                .status,
            409
        );
        assert_eq!(
            h.send("POST", &format!("/progression/purchase/p1/{AXE}/1000"), &GS, None)
                .await
                .status,
            409
        );
    }
}

#[tokio::test]
async fn no_store() {
    let h = Harness::new(Options {
        store: false,
        ..Options::default()
    })
    .await;
    let message = "isles: no database is configured (REFORGED_DB_FILE or REFORGED_DB_ENGINE)";
    for (method, uri, body) in [
        ("GET", "/inventory/p1/c1", None),
        (
            "POST",
            "/inventory",
            Some(r#"{"accountId":"p1","characterId":"c","transactionId":"t"}"#),
        ),
        (
            "POST",
            "/inventory/instanceditem",
            Some(r#"{"characterId":"c","instanceId":"i","catalogId":"x"}"#),
        ),
        ("DELETE", &format!("/progression/p1/{AXE}"), None),
        ("POST", &format!("/progression/purchase/p1/{AXE}/5"), None),
    ] {
        let answer = h.send(method, uri, &GS, body).await;
        assert_eq!(answer.status, 501, "{method} {uri}");
        assert_eq!(answer.json()["error"]["message"], message);
    }
}

#[tokio::test]
async fn dedicated_only_runtime() {
    let h = Harness::new(Options {
        dedicated_guard: true,
        ..Options::default()
    })
    .await;
    let c = h.character("p1").await;
    let host = Who::Host("cap-ok");
    let body = tx("p1", &c, "h1", json!({"addStackedItems": [stack("MAT_TEST", 1)]}));
    h.post("/inventory", &host, &body).await.assert(
        403,
        r#"{"code":"forbidden","message":"This runtime cannot mutate inventory.","payload":null}"#,
    );
    h.post("/inventory/instanceditem", &host, &json!({})).await.assert(
        403,
        r#"{"code":"forbidden","message":"Player-host runtime cannot mutate inventory.","payload":null}"#,
    );
    let write = r#"{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}"#;
    h.post("/inventory/instanceditem", &host_as(Some("p1"), true), &json!({}))
        .await
        .assert(403, write);
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &host, None)
        .await
        .assert(403, write);
    h.send("POST", &format!("/progression/purchase/p1/{AXE}/5"), &host, None)
        .await
        .assert(403, write);
}

#[tokio::test]
async fn host_fence() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let host = host_as(Some("p1"), true);
    let body =
        json!({"characterId": c, "transactionId": "h1", "source": "quest", "addStackedItems": [stack("MAT_TEST", 2)]});
    h.post("/inventory", &host, &body).await.ok();
    let journal = h.guard.journal.lock().unwrap().clone();
    assert_eq!(journal[0], format!("authorize_inventory p1 {c} h1"));
    assert_eq!(journal[1], "Applying");
    assert!(journal[2].starts_with("grant {") && journal[2].contains("\"source\":\"quest\""));
    assert_eq!(journal[3], "Accepted");
    // A refused grant rolls everything back
    *h.guard.grant_refusal.lock().unwrap() = Some(Reply {
        status: 403,
        body: Json(r#"{"code":"no"}"#.into()),
    });
    let refused = json!({"characterId": c, "transactionId": "h2", "addStackedItems": [stack("MAT_TEST", 5)]});
    let answer = h.post("/inventory", &host_as(Some("p1"), true), &refused).await;
    answer.assert(403, r#"{"code":"no"}"#);
    assert!(!h.ledger_rows("p1").await.iter().any(|r| r["transactionId"] == "h2"));
}

#[tokio::test]
async fn host_item_saves_and_reforge_tags() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    let stored = json!({"catalogId": "WP", "instanceId": "i", "itemData": "old", "updateVersion": 3});
    h.set_inventory(&c, &json!([stored]), &json!([])).await;
    let host = host_as(Some("p1"), true);
    let body = json!({"characterId": c, "instanceId": "i", "catalogId": "WP", "itemData": "newer", "updateVersion": 4});
    h.post("/inventory/instanceditem", &host, &body).await.ok();
    let tags = h.guard.tags();
    assert_eq!(
        tags[0],
        (
            HostWriteKind::ItemSave,
            "p1".into(),
            json!({"instanceId": "i", "catalogId": "WP", "updateVersion": 4, "itemDataBytes": 5})
        )
    );
    h.guard.refuse_all();
    h.post("/inventory/instanceditem", &host, &body).await.assert(
        403,
        r#"{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}"#,
    );
    *h.guard.refusal.lock().unwrap() = None;
    h.set_progress("p1", AXE, 4000).await;
    let runtime = Who::Host("cap-ok");
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &runtime, None)
        .await
        .ok();
    h.send("DELETE", &format!("/progression/p1/{AXE}"), &runtime, None)
        .await
        .ok();
    h.send("POST", &format!("/progression/purchase/p1/{AXE}/1000"), &runtime, None)
        .await
        .ok();
    let tags = h.guard.tags();
    let reforge: Vec<_> = tags.iter().filter(|t| t.0 == HostWriteKind::Reforge).collect();
    assert_eq!(reforge.len(), 2);
    assert_eq!(reforge[0].2["route"], "delete");
    assert_eq!(
        (
            reforge[0].2["priorProgress"].clone(),
            reforge[0].2["cap"].clone(),
            reforge[0].2["banked"].clone()
        ),
        (json!(4000), json!(3000), json!(1000))
    );
    assert_eq!(reforge[1].2["route"], "purchase");
    assert_eq!(reforge[1].2["amount"], 1000);
}

#[tokio::test]
async fn currency_rules() {
    let h = Harness::bare().await;
    let c = h.character("p1").await;
    // 400 Aetherspark buys two Aetherhearts; a third needs banked prestige
    h.credit("p1", "CURRENCY_PJM_PRESTIGE_EMPTY", 400.0).await;
    let two = tx(
        "p1",
        &c,
        "c1",
        json!({"removeStackedItems": [stack("CURRENCY_PJM_PRESTIGE_EMPTY", 400)], "addStackedItems": [stack("CURRENCY_PJM_PRESTIGE_FILLED", 2)]}),
    );
    h.post("/inventory", &GS, &two).await.ok();
    assert_eq!(h.balance("p1", "CURRENCY_PJM_PRESTIGE_FILLED").await, Some(2.0));
    let third = tx(
        "p1",
        &c,
        "c2",
        json!({"addStackedItems": [stack("CURRENCY_PJM_PRESTIGE_FILLED", 1)]}),
    );
    h.post("/inventory", &GS, &third).await.assert(
        409,
        r#"{"code":null,"message":"Insufficient banked prestige progress","payload":{}}"#,
    );
    // Banked levels past the cap (3000) of 1000 each pay one apiece
    h.set_progress("p1", AXE, 5000).await;
    h.post(
        "/inventory",
        &GS,
        &tx(
            "p1",
            &c,
            "c3",
            json!({"addStackedItems": [stack("CURRENCY_PJM_PRESTIGE_FILLED", 2)]}),
        ),
    )
    .await
    .ok();
    assert_eq!(h.track("p1", AXE).await.unwrap().prestige_claimed, Some(2));
    h.post(
        "/inventory",
        &GS,
        &tx(
            "p1",
            &c,
            "c4",
            json!({"addStackedItems": [stack("CURRENCY_PJM_PRESTIGE_FILLED", 1)]}),
        ),
    )
    .await
    .assert(
        409,
        r#"{"code":null,"message":"Insufficient banked prestige progress","payload":{}}"#,
    );
}
