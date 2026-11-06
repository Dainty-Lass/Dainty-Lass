// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The three `/character` rows and the entry, black-box through core's `app()` (spec §13.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{Harness, Options, Who, inner, parse};
use rustic_isles::{Group, Handlers, NoPlayerPolicy, Service, routes};
use rustic_isles_auth::AuthData;
use rustic_isles_characters::routes as area_routes;
use rustic_isles_store::repo::bounties::WalletRecord;
use serde_json::{Value, json};

const CONFLICT: &str = "<html>\n<head>\n<meta http-equiv=\"Content-Type\" content=\"text/html;charset=ISO-8859-1\"/>\n\
<title>Error 409 </title>\n</head>\n<body>\n<h2>HTTP ERROR: 409</h2>\n<p>Problem accessing /character. Reason:\n\
<pre>    Conflict</pre></p>\n<hr />\n</body>\n</html>\n";

#[test]
fn routes_are_the_rows() {
    let rows: Vec<_> = routes::all()
        .iter()
        .filter(|r| r.group == Group::CharactersLoadouts)
        .map(|r| (r.method, r.path))
        .collect();
    assert_eq!(area_routes(), rows.as_slice());
    let mut handlers = Handlers::new();
    rustic_isles_characters::register(&mut handlers);
    assert_eq!(handlers.check(), Ok(()));
    for (method, path) in area_routes() {
        assert!(handlers.contains(Service::Backend, *method, path), "{path}");
    }
}

#[tokio::test]
async fn entry() {
    let h = Harness::bare().await;
    let answer = h.get("/character", &Who::Nobody).await;
    assert_eq!(answer.status, 501);
    let id = answer.request_id.clone().unwrap();
    let want = json!({"error": {"code": "NOT_IMPLEMENTED", "message": "isles: GET /character is not implemented yet", "requestId": id}});
    assert_eq!(answer.json(), want);

    h.get("/character", &Who::BadBearer).await.assert_empty(401);

    let answer = h
        .send_with(
            "PUT",
            "/character",
            &Who::Nobody,
            Some("nope"),
            &[("content-type", "application/json")],
        )
        .await;
    assert_eq!(answer.status, 400);
    assert_eq!(answer.content_type.as_deref(), Some("text/html; charset=utf-8"));

    let bare = Harness::new(Options {
        store: false,
        ..Options::default()
    })
    .await;
    assert_eq!(bare.get("/character", &Who::Player("p1")).await.status, 501);
    let no_area = Harness::new(Options {
        area: false,
        ..Options::default()
    })
    .await;
    assert_eq!(no_area.get("/character", &Who::Player("p1")).await.status, 501);
}

#[tokio::test]
async fn get_creates_the_first_character() {
    let h = Harness::bare().await;
    let account = h.make_account("p1").await;
    h.launcher_account(&account, "p1", "Ash").await;
    let answer = h.get("/character", &Who::Player("p1")).await;
    assert_eq!(answer.content_type.as_deref(), Some("application/json; charset=utf-8"));
    let list = answer.ok();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 1);
    let character = list[0].as_object().unwrap();
    let keys: Vec<&str> = character.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "accountId",
            "catalogDaoId",
            "createdDate",
            "data",
            "id",
            "lastModifiedDate",
            "name",
            "updateVersion"
        ]
    );
    assert_eq!(character["name"], "Ash");
    assert_eq!(character["accountId"], "p1");
    assert_eq!(character["catalogDaoId"], Value::Null);
    assert_eq!(character["updateVersion"], 0);
    assert_eq!(character["createdDate"], "Oct 14, 2026");
    assert_eq!(character["lastModifiedDate"], "Oct 14, 2026");
    let data = inner(&list[0], "data");
    assert_eq!(data["PlayerAccountProgressStep"], "EnteredRamsgate");
    assert_eq!(data["LoginTime"], "2026.10.14-17.46.40");

    let again = h.get("/character", &Who::Player("p1")).await.ok();
    assert_eq!(again.as_array().unwrap().len(), 1);
    assert_eq!(again[0]["id"], list[0]["id"]);

    let p2 = h.get("/character", &Who::Player("p2")).await.ok();
    assert_eq!(p2[0]["name"], "p2");
}

#[tokio::test]
async fn get_for_a_gameserver_without_a_player() {
    let h = Harness::new(Options {
        settings: vec![("GAMESERVER_NO_PLAYER_USER_ID", "hub")],
        ..Options::default()
    })
    .await;
    let list = h.get("/character", &Who::Gameserver).await.ok();
    assert_eq!(
        (list[0]["accountId"].clone(), list[0]["name"].clone()),
        (json!("hub"), json!("hub"))
    );
    assert!(h.account("hub").await.is_some());

    let off = Harness::new(Options {
        policy: NoPlayerPolicy::Off,
        settings: vec![("GAMESERVER_NO_PLAYER_USER_ID", "hub")],
        ..Options::default()
    })
    .await;
    assert_eq!(off.get("/character", &Who::Gameserver).await.ok(), json!([]));
    assert_eq!(h.get("/character", &Who::Player("INVALID")).await.ok(), json!([]));
    assert!(h.account("INVALID").await.is_none());
}

#[tokio::test]
async fn get_adds_the_dojo_series_when_the_journey_unlocked_it() {
    let cases = [
        (json!({"Dojo": {"node_status": 2}}), true),
        (json!({"X": {"node_id": "Dojo", "node_status": 1}}), true),
        (json!({"Dojo": {"node_status": 0}}), false),
        (json!({"Dojo": {"node_status": "2"}}), false),
    ];
    for (nodes, unlocked) in cases {
        let h = Harness::bare().await;
        h.journey("p1", nodes.clone()).await;
        let list = h.get("/character", &Who::Player("p1")).await.ok();
        let data = inner(&list[0], "data");
        let series = &data["SERIE_dojo"];
        if unlocked {
            assert_eq!(series, &json!("{\"Series\":\"DOJO_TEST\",\"Steps\":[1,2]}"), "{nodes}");
        } else {
            assert_eq!(series, &Value::Null, "{nodes}");
        }
        let id = list[0]["id"].as_str().unwrap();
        assert!(!h.stored_data("p1", id).await.contains("SERIE_dojo"));
    }
}

#[tokio::test]
async fn put_creates_and_seeds() {
    let h = Harness::bare().await;
    let character = h.post_put("p1", json!({"name": "Bo"})).await;
    assert_eq!(character["name"], "Bo");
    assert_eq!(character["accountId"], "p1");
    let id = character["id"].as_str().unwrap();
    let inventory = h.inventory(id).await.unwrap();
    let items = parse(&inventory.instanced_items);
    let catalog: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["catalogId"].as_str().unwrap())
        .collect();
    assert_eq!(catalog, common::kit().instanced);
    assert_eq!(
        parse(&inventory.stacked_items),
        json!([{"catalogId": "STACK_A", "quantity": 1}, {"catalogId": "STACK_B", "quantity": 1}])
    );
    assert_eq!(inventory.bootstrap_version.as_deref(), Some("test-1"));
    let loadout = h.loadout("p1", id).await.unwrap();
    assert_eq!(parse(&loadout.loadouts).as_array().unwrap().len(), 1);
    let account = h.account("p1").await.unwrap().id;
    let wallet = h
        .store
        .read(move |c, r| r.wallets.find(c, &account))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wallet.balances, common::kit().wallet);

    let second = h.post_put("p1", json!({"name": "Cy"})).await;
    let list = h.get("/character", &Who::Player("p1")).await.ok();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Bo", "Cy"]);
    assert_eq!(list[1]["id"], second["id"]);
}

#[tokio::test]
async fn put_names_and_refusals() {
    let h = Harness::bare().await;
    assert_eq!(h.post_put("p1", json!({})).await["name"], "undefined");
    assert_eq!(h.post_put("p1", json!({"name": 5})).await["name"], "5");
    assert_eq!(
        h.post_put("p1", json!({"name": " spaced\n"})).await["name"],
        " spaced\n"
    );

    let answer = h.send("PUT", "/character", &Who::Player("p1"), None).await;
    assert_eq!(answer.status, 500);
    assert_eq!(answer.content_type.as_deref(), Some("text/html; charset=utf-8"));

    let off = Harness::new(Options {
        policy: NoPlayerPolicy::Off,
        ..Options::default()
    })
    .await;
    off.send("PUT", "/character", &Who::Gameserver, Some("{\"name\":\"x\"}"))
        .await
        .assert_empty(400);
    h.send("PUT", "/character", &Who::Player("INVALID"), Some("{}"))
        .await
        .assert_empty(400);
}

#[tokio::test]
async fn put_keeps_an_existing_wallet() {
    let h = Harness::bare().await;
    let account = h.make_account("p1").await;
    let wallet = WalletRecord {
        account: account.clone(),
        balances: vec![("COIN_TEST".into(), 5000.0)],
        bootstrap_version: None,
    };
    h.store
        .write(&common::system(), move |tx, r| r.wallets.create(tx, &wallet))
        .await
        .unwrap();
    h.post_put("p1", json!({"name": "Bo"})).await;
    let found = h
        .store
        .read(move |c, r| r.wallets.find(c, &account))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.balances, vec![("COIN_TEST".into(), 5000.0)]);
}

impl Harness {
    async fn post_put(&self, user: &'static str, body: Value) -> Value {
        self.send("PUT", "/character", &Who::Player(user), Some(&body.to_string()))
            .await
            .ok()
    }

    async fn save(&self, who: &Who, body: Value) -> common::Answer {
        self.post("/character", who, &body).await
    }
}

#[tokio::test]
async fn post_versions() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let saved = h
        .save(
            &me,
            json!({"characterId": id, "updateVersion": 1, "data": "{\"Extra\":1}"}),
        )
        .await
        .ok();
    assert_eq!(saved["updateVersion"], 1);
    assert_eq!(inner(&saved, "data")["Extra"], 1);

    let answer = h
        .save(&me, json!({"characterId": id, "updateVersion": 1, "data": "{}"}))
        .await;
    assert_eq!(answer.status, 409);
    assert_eq!(answer.content_type.as_deref(), Some("text/html; charset=utf-8"));
    assert_eq!(answer.body, CONFLICT);
    assert_eq!(answer.body.len(), 245);

    let forced = h
        .save(
            &Who::GameserverAs("p1"),
            json!({"characterId": id, "updateVersion": 1, "data": "{\"Extra\":2}"}),
        )
        .await
        .ok();
    assert_eq!(forced["updateVersion"], 2);

    // Stale for a player: refused even when it is no whole number
    let answer = h.save(&me, json!({"characterId": id, "updateVersion": 1.5})).await;
    assert_eq!(answer.status, 409);

    // On a character still at version 0: accepted, nothing written
    let fresh = h.first_character("p4").await;
    for version in [json!("abc"), json!(1.5), json!(1e300)] {
        let before = h.stored_data("p4", &fresh).await;
        let answer = h
            .save(
                &Who::Player("p4"),
                json!({"characterId": fresh, "updateVersion": version, "data": "{\"Extra\":3}"}),
            )
            .await
            .ok();
        assert_eq!(answer["updateVersion"], 0, "{version}");
        assert_eq!(h.stored_data("p4", &fresh).await, before);
    }

    assert_eq!(
        h.save(&me, json!({"characterId": "nobody", "updateVersion": 9}))
            .await
            .status,
        409
    );
    let other = h.first_character("p2").await;
    assert_eq!(
        h.save(&me, json!({"characterId": other, "updateVersion": 9}))
            .await
            .status,
        409
    );
}

#[tokio::test]
async fn post_data() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    for data in [json!(5), json!({"a": 1})] {
        let answer = h
            .save(&me, json!({"characterId": id, "updateVersion": 5, "data": data}))
            .await;
        assert_eq!(answer.status, 500, "{data}");
        assert_eq!(answer.content_type.as_deref(), Some("text/html; charset=utf-8"));
    }
    let saved = h.save(&me, json!({"characterId": id, "updateVersion": 5})).await.ok();
    let data = inner(&saved, "data");
    assert_eq!(data["PlayerAccountProgressStep"], "EnteredRamsgate");
    assert_eq!(saved["updateVersion"], 5);

    let answer = h.send("POST", "/character", &me, None).await;
    assert_eq!(answer.status, 500);
    h.save(&Who::Player("INVALID"), json!({"characterId": id}))
        .await
        .assert_empty(400);
}

#[tokio::test]
async fn post_triggers_and_the_dojo_series() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let trigger = json!({"SERIE_trigger": "{\"Node_T\":{\"Status\":3}}"}).to_string();
    let saved = h
        .save(&me, json!({"characterId": id, "updateVersion": 1, "data": trigger}))
        .await
        .ok();
    let series = "{\"Series\":\"DOJO_TEST\",\"Steps\":[1,2]}";
    assert_eq!(inner(&saved, "data")["SERIE_dojo"], series);
    // A client blob without it keeps it
    let saved = h
        .save(
            &me,
            json!({"characterId": id, "updateVersion": 2, "data": "{\"Other\":1}"}),
        )
        .await
        .ok();
    assert_eq!(inner(&saved, "data")["SERIE_dojo"], series);
    assert!(h.stored_data("p1", &id).await.contains("SERIE_dojo"));
    let bad = json!({"SERIE_trigger": "nope"}).to_string();
    let answer = h
        .save(&me, json!({"characterId": id, "updateVersion": 3, "data": bad}))
        .await;
    assert_eq!(answer.status, 500);
}

#[tokio::test]
async fn post_from_a_gameserver() {
    let h = Harness::bare().await;
    let other = h.first_character("p2").await;
    let saved = h
        .save(
            &Who::Gameserver,
            json!({"accountId": "p2", "characterId": other, "updateVersion": 1, "data": "{\"Hub\":1}"}),
        )
        .await
        .ok();
    assert_eq!(saved["accountId"], "p2");
    assert_eq!(inner(&saved, "data")["Hub"], 1);

    // The hub's phantom save names no character: the first one takes it
    let saved = h
        .save(
            &Who::Gameserver,
            json!({"accountId": "p3", "characterId": "", "updateVersion": 4, "data": "{\"Phantom\":1}"}),
        )
        .await
        .ok();
    assert_eq!(saved["accountId"], "p3");
    assert_eq!(saved["updateVersion"], 4);
    assert_eq!(inner(&saved, "data")["Phantom"], 1);

    let off = Harness::new(Options {
        policy: NoPlayerPolicy::Off,
        ..Options::default()
    })
    .await;
    off.first_character("p3").await;
    let answer = off
        .save(
            &Who::Gameserver,
            json!({"accountId": "p3", "updateVersion": 4, "data": "{}"}),
        )
        .await;
    assert_eq!(answer.status, 409);
}

#[tokio::test]
async fn an_authenticated_caller_is_kept() {
    let h = Harness::bare().await;
    let data = AuthData {
        user_id: Some("p9".into()),
        ..AuthData::default()
    };
    let list = h.get("/character", &Who::Data(data)).await.ok();
    assert_eq!(list[0]["accountId"], "p9");
}

#[tokio::test]
async fn actors() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let account = h.account("p1").await.unwrap().id;
    let before = h.log().await.len();
    h.save(&Who::Player("p1"), json!({"characterId": id, "updateVersion": 1}))
        .await
        .ok();
    let player = format!("player:{account}");
    let log = h.log().await;
    let written: Vec<_> = log[before..].iter().filter(|(_, t, _)| t == "characters").collect();
    assert!(!written.is_empty());
    assert!(written.iter().all(|(actor, _, _)| actor == &player), "{written:?}");

    let before = log.len();
    h.save(&Who::GameserverAs("p1"), json!({"characterId": id, "updateVersion": 2}))
        .await
        .ok();
    let log = h.log().await;
    assert!(log[before..].iter().all(|(actor, _, _)| actor == "system:isles"));
    // The first character of a new account is the system's
    let first = h.log().await.len();
    h.first_character("p5").await;
    assert!(
        h.log().await[first..]
            .iter()
            .all(|(actor, _, _)| actor == "system:isles")
    );
}

#[tokio::test]
async fn replay_seeds_fix_the_ids() {
    let headers = [("x-isles-replay-seed", "seed-1")];
    let mut seen = Vec::new();
    for _ in 0..2 {
        let h = Harness::bare().await;
        let answer = h
            .send_with(
                "PUT",
                "/character",
                &Who::Player("p1"),
                Some("{\"name\":\"Bo\"}"),
                &headers,
            )
            .await
            .ok();
        let id = answer["id"].as_str().unwrap().to_owned();
        let items = h.inventory(&id).await.unwrap().instanced_items;
        seen.push((id, items));
    }
    assert_eq!(seen[0], seen[1]);
    let scope = rustic_isles::replay::ReplayScope::new(None, Some("seed-1"));
    let first = rustic_isles::replay::with_replay_scope(scope, async { rustic_isles::replay::next_uuid() }).await;
    assert_eq!(seen[0].0, first);
}
