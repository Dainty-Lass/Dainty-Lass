// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The five `/loadout` rows and the starter repair, black-box through core's `app()` (spec §13.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{EMOJIS, FLASK, Harness, NO_EMOJI_TABLES, Options, PARTS, TABLES, WEAPON, Who, parse};
use dainty_service_kit::jsjson;
use rustic_isles_auth::AuthData;
use rustic_isles_characters::starter::is_instance_id;
use rustic_isles_characters::{
    Characters, CharactersError, empty_loadout_payload, is_fresh_onboarding, loadout_payload,
};
use rustic_rifts_seam::{HostWriteKind, Json, Reply};
use serde_json::{Value, json};

const EMPTY: &str = "{\"code\":null,\"message\":\"OK\",\"payload\":{\"loadouts\":[],\"persistent\":null,\
\"num_account_slots\":0,\"max_account_slots\":0,\"num_character_slots\":1,\"max_character_slots\":6,\
\"active_index\":0,\"needs_migration\":false}}";

const FORBIDDEN: &str =
    "{\"code\":\"forbidden\",\"message\":\"Loadout slot unlocks require gameserver authority.\",\"payload\":null}";
const BAD_COUNT: &str = "{\"code\":\"invalid_loadout_slot_count\",\"message\":\"Loadout slot unlock delta must be \
between 1 and 6.\",\"payload\":null}";
const BAD_ACTIVE: &str = "{\"code\":\"invalid_active_loadout\",\"message\":\"Active loadout index is invalid.\",\
\"payload\":null}";
const LOCKED: &str = "{\"code\":\"loadout_slot_not_unlocked\",\"message\":\"The requested loadout slot is not \
unlocked.\",\"payload\":null}";

fn all(user: &str, character: &str) -> String {
    format!("/loadout/{user}/{character}/all")
}

fn unlock(user: &str, character: &str, n: &str) -> String {
    format!("/loadout/{user}/{character}/unlock/{n}")
}

fn active(user: &str, character: &str, n: &str) -> String {
    format!("/loadout/{user}/{character}/active/{n}")
}

#[tokio::test]
async fn reads() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let answer = h.get(&all("p1", &id), &Who::Player("p1")).await;
    let value = answer.ok();
    let keys: Vec<&str> = value["payload"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "loadouts",
            "persistent",
            "num_account_slots",
            "max_account_slots",
            "num_character_slots",
            "max_character_slots",
            "active_index",
            "needs_migration"
        ]
    );
    let payload = &value["payload"];
    assert_eq!(payload["loadouts"].as_array().unwrap().len(), 1);
    assert_eq!(
        (
            &payload["num_character_slots"],
            &payload["max_character_slots"],
            &payload["active_index"]
        ),
        (&json!(1), &json!(6), &json!(0))
    );
    assert_eq!(
        (&payload["num_account_slots"], &payload["max_account_slots"]),
        (&json!(0), &json!(0))
    );
    let slot = &payload["loadouts"][0];
    assert_eq!(slot["weapon"]["item_id"], WEAPON);
    assert_eq!(slot["weapon"]["instance_data"], "{\"Level\":1}");
    assert_eq!(slot["helmet"]["instance_data"], Value::Null);
    assert_eq!(payload["persistent"]["emojis"], json!(EMOJIS));

    let other = h.first_character("p2").await;
    h.get(&all("p1", &other), &Who::Player("p1")).await.assert_empty(400);

    let phantom = h.get("/loadout/INVALID//all", &Who::Gameserver).await;
    assert_eq!((phantom.status, phantom.body.as_str()), (200, EMPTY));
    h.get("/loadout/p1//all", &Who::Player("p1")).await.assert_empty(400);
    h.get("/loadout/INVALID//all", &Who::Player("p1"))
        .await
        .assert_empty(400);
    let sentinel = h.get(&all("INVALID", &id), &Who::Gameserver).await;
    assert_eq!((sentinel.status, sentinel.body.as_str()), (200, EMPTY));
    // A game server reads any account's
    let theirs = h.get(&all("p2", &other), &Who::Gameserver).await.payload();
    assert_eq!(theirs["loadouts"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unlock_authority() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    for who in [Who::Player("p1"), Who::Data(AuthData::default())] {
        let answer = h.send("POST", &unlock("p1", &id, "2"), &who, None).await;
        assert_eq!((answer.status, answer.body.as_str()), (403, FORBIDDEN));
    }
    assert!(h.guard.asked.lock().unwrap().is_empty());
    for n in ["0", "7", "1.5", "abc"] {
        let answer = h.send("POST", &unlock("p1", &id, n), &Who::Gameserver, None).await;
        assert_eq!((answer.status, answer.body.as_str()), (400, BAD_COUNT), "{n}");
    }
    let answer = h
        .send("POST", &unlock("INVALID", &id, "1"), &Who::Gameserver, None)
        .await;
    assert_eq!((answer.status, answer.body.as_str()), (400, BAD_COUNT));
    let other = h.first_character("p2").await;
    h.send("POST", &unlock("p1", &other, "1"), &Who::Gameserver, None)
        .await
        .assert_empty(400);
}

#[tokio::test]
async fn unlock_follows_the_journey() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    h.journey(
        "p1",
        json!({"LoadoutSlot_01": {"node_status": 2}, "LoadoutSlot_02": {"node_status": 2}, "LoadoutSlot_03": {"node_status": 1}}),
    )
    .await;
    let payload = h
        .send("POST", &unlock("p1", &id, "1"), &Who::Gameserver, None)
        .await
        .payload();
    let slots = payload["loadouts"].as_array().unwrap();
    assert_eq!(slots.len(), 3);
    assert_eq!(payload["num_character_slots"], 3);
    for (i, slot) in slots.iter().enumerate().skip(1) {
        assert_eq!(slot["slot_index"], i);
        assert_eq!(slot["update_version"], 0);
        assert_eq!(slot["custom_name"], "");
        assert_eq!(slot["weapon"], slots[0]["weapon"]);
    }
    let row = h.loadout("p1", &id).await.unwrap();
    assert_eq!(row.unlocked_total_slots, Some(3));

    // Never relocked
    h.journey("p1", json!({})).await;
    for n in ["0x2", "%203"] {
        let payload = h
            .send("POST", &unlock("p1", &id, n), &Who::Gameserver, None)
            .await
            .payload();
        assert_eq!(payload["loadouts"].as_array().unwrap().len(), 3, "{n}");
    }
}

#[tokio::test]
async fn unlock_by_a_host_runtime() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let account = h.account("p1").await.unwrap().id.to_string();
    h.journey("p1", json!({"LoadoutSlot_01": {"node_status": 2}})).await;
    let payload = h
        .send("POST", &unlock("p1", &id, "1"), &Who::Host("cap-ok"), None)
        .await
        .payload();
    assert_eq!(payload["loadouts"].as_array().unwrap().len(), 2);
    assert_eq!(
        *h.guard.asked.lock().unwrap(),
        [(HostWriteKind::LoadoutUnlock, account.clone())]
    );
    let recorded = h.guard.recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].kind, HostWriteKind::LoadoutUnlock);
    assert_eq!(recorded[0].target.0, account);
    assert_eq!(
        recorded[0].summary.0,
        "{\"gameserverDelta\":1.0,\"authoritativeTotalSlots\":2}"
    );
    assert!(!recorded[0].summary.0.contains(&id));

    let body = "{\"code\":\"P2P_HOST_DENIED\",\"message\":\"no\",\"payload\":null}";
    *h.guard.refusal.lock().unwrap() = Some(Reply {
        status: 403,
        body: Json(body.into()),
    });
    let answer = h
        .send("POST", &unlock("p1", &id, "1"), &Who::Host("cap-ok"), None)
        .await;
    assert_eq!((answer.status, answer.body.as_str()), (403, body));
    assert_eq!(answer.content_type.as_deref(), Some("application/json; charset=utf-8"));
    *h.guard.refusal.lock().unwrap() = Some(Reply {
        status: 42,
        body: Json("{}".into()),
    });
    assert_eq!(
        h.send("POST", &unlock("p1", &id, "1"), &Who::Host("cap-ok"), None)
            .await
            .status,
        403
    );

    let answer = h
        .send("POST", &unlock("nobody", &id, "1"), &Who::Host("cap-ok"), None)
        .await;
    let denied = "{\"code\":\"P2P_HOST_WRITE_DENIED\",\"message\":\"This host may not write this player's state.\",\
\"payload\":null}";
    assert_eq!((answer.status, answer.body.as_str()), (403, denied));

    // No handle from the realms seam: refused as the dedicated-only guard refuses
    let answer = h
        .send("POST", &unlock("p1", &id, "1"), &Who::Host("cap-bare"), None)
        .await;
    let bare = "{\"code\":\"forbidden\",\"message\":\"This runtime cannot write player state.\",\"payload\":null}";
    assert_eq!((answer.status, answer.body.as_str()), (403, bare));
}

#[tokio::test]
async fn active_index() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let answer = h.send("POST", &active("p1", &id, "1"), &me, None).await;
    assert_eq!((answer.status, answer.body.as_str()), (400, LOCKED));
    for n in ["6", "-1", "abc", "1.5"] {
        let answer = h.send("POST", &active("p1", &id, n), &me, None).await;
        assert_eq!((answer.status, answer.body.as_str()), (400, BAD_ACTIVE), "{n}");
    }
    let answer = h
        .send("POST", &active("p1", &id, "0"), &Who::Player("INVALID"), None)
        .await;
    assert_eq!((answer.status, answer.body.as_str()), (400, BAD_ACTIVE));

    h.journey(
        "p1",
        json!({"LoadoutSlot_01": {"node_status": 2}, "LoadoutSlot_02": {"node_status": 2}}),
    )
    .await;
    h.send("POST", &unlock("p1", &id, "1"), &Who::Gameserver, None)
        .await
        .payload();
    let payload = h.send("POST", &active("p1", &id, "0x2"), &me, None).await.payload();
    assert_eq!(payload["active_index"], 2);
    let payload = h.send("POST", &active("p1", &id, "%20"), &me, None).await.payload();
    assert_eq!(payload["active_index"], 0);
    let other = h.first_character("p2").await;
    h.send("POST", &active("p1", &other, "0"), &me, None)
        .await
        .assert_empty(400);
}

#[tokio::test]
async fn slot_and_persistent_writes() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let slot = json!({"data": "{\"weapon\":null,\"slot_index\":0}"});
    let payload = h.post(&format!("/loadout/p1/{id}/0"), &me, &slot).await.payload();
    assert_eq!(payload["loadouts"][0], json!({"weapon": null, "slot_index": 0}));

    let wheel = json!({"quick_chats": ["", ""], "emojis": ["x"], "mine": 1}).to_string();
    let payload = h
        .post(&format!("/loadout/p1/{id}/persistent"), &me, &json!({"data": wheel}))
        .await
        .payload();
    assert_eq!(payload["persistent"]["mine"], 1);
    assert_eq!(payload["persistent"]["quick_chats"][0], "chat_1");
    assert_eq!(payload["persistent"]["emojis"], json!(["x"]));
    let stored = parse(&h.loadout("p1", &id).await.unwrap().persistent);
    assert_eq!(stored["quick_chats"], json!(["", ""]));

    for index in ["6", "01", "-0", "x"] {
        h.post(&format!("/loadout/p1/{id}/{index}"), &me, &slot)
            .await
            .assert_empty(400);
    }
    for body in [
        json!({}),
        json!({"data": ""}),
        json!({"data": "[1]"}),
        json!({"data": "nope"}),
        json!({"data": 5}),
    ] {
        h.post(&format!("/loadout/p1/{id}/0"), &me, &body)
            .await
            .assert_empty(400);
    }
    h.send("POST", &format!("/loadout/p1/{id}/0"), &me, Some("[]"))
        .await
        .assert_empty(400);
    let other = h.first_character("p2").await;
    h.post(&format!("/loadout/p1/{other}/0"), &me, &slot)
        .await
        .assert_empty(400);
    // Beyond the visible slots: refused after the attempts
    h.post(&format!("/loadout/p1/{id}/2"), &me, &slot)
        .await
        .assert_empty(400);
    h.post(&format!("/loadout/INVALID/{id}/0"), &Who::Gameserver, &slot)
        .await
        .assert_empty(400);
}

#[tokio::test]
async fn wheels_without_quick_emojis() {
    let h = Harness::new(Options {
        tables: &NO_EMOJI_TABLES,
        ..Options::default()
    })
    .await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let payload = h.get(&all("p1", &id), &me).await.payload();
    assert_eq!(
        payload["persistent"]["quick_chats"],
        json!(["", "", "", "", "", "", "", "", ""])
    );
    let wheel = json!({"quick_chats": [], "emojis": []}).to_string();
    let payload = h
        .post(&format!("/loadout/p1/{id}/persistent"), &me, &json!({"data": wheel}))
        .await
        .payload();
    assert_eq!(payload["persistent"], json!({"quick_chats": [], "emojis": []}));
}

fn item_of<'a>(items: &'a Value, catalog: &str) -> &'a Value {
    items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["catalogId"] == catalog)
        .unwrap()
}

#[tokio::test]
async fn a_missing_inventory_is_rebuilt_from_the_loadout() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let slot = parse(&h.loadout("p1", &id).await.unwrap().loadouts)[0].clone();
    let (weapon, flask) = (slot["weapon"]["instance_id"].clone(), slot["flask"].clone());
    h.remove_row("inventories", &id).await;
    h.get(&all("p1", &id), &me).await.payload();
    let inventory = h.inventory(&id).await.unwrap();
    let items = parse(&inventory.instanced_items);
    assert_eq!(item_of(&items, WEAPON)["instanceId"], weapon);
    assert_eq!(item_of(&items, FLASK)["instanceId"], flask);
    assert_eq!(items.as_array().unwrap().len(), common::kit().instanced.len());
    assert_eq!(parse(&inventory.stacked_items).as_array().unwrap().len(), 2);

    // A placeholder is never reused
    let mut placeholder = slot.clone();
    placeholder["weapon"]["instance_id"] = json!("placeholder");
    let body = json!({"data": placeholder.to_string()});
    h.post(&format!("/loadout/p1/{id}/0"), &me, &body).await.payload();
    h.remove_row("inventories", &id).await;
    h.get(&all("p1", &id), &me).await.payload();
    let items = parse(&h.inventory(&id).await.unwrap().instanced_items);
    let fresh = item_of(&items, WEAPON)["instanceId"].as_str().unwrap().to_owned();
    assert!(is_instance_id(&fresh) && fresh != "placeholder");
    assert_eq!(item_of(&items, FLASK)["instanceId"], flask);
}

#[tokio::test]
async fn a_missing_loadout_is_rebuilt_from_the_inventory() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let items = parse(&h.inventory(&id).await.unwrap().instanced_items);
    h.remove_row("loadouts", &id).await;
    let payload = h.get(&all("p1", &id), &me).await.payload();
    let slot = &payload["loadouts"][0];
    assert_eq!(slot["weapon"]["instance_id"], item_of(&items, WEAPON)["instanceId"]);
    assert_eq!(slot["flask"], item_of(&items, FLASK)["instanceId"]);
    assert_eq!(
        payload["persistent"]["banner"],
        item_of(&items, "BANNER_TEST")["instanceId"]
    );

    // Items healed on the way: a bad id, a bad version, a non-object entry
    let mut list = items.as_array().unwrap().clone();
    let weapon = list.iter_mut().find(|i| i["catalogId"] == WEAPON).unwrap();
    *weapon = json!({"catalogId": WEAPON, "instanceId": "", "updateVersion": "x"});
    list.push(json!(7));
    let stacked = parse(&h.inventory(&id).await.unwrap().stacked_items);
    h.set_inventory(&id, &Value::Array(list), &stacked).await;
    h.remove_row("loadouts", &id).await;
    let payload = h.get(&all("p1", &id), &me).await.payload();
    let items = parse(&h.inventory(&id).await.unwrap().instanced_items);
    let healed = item_of(&items, WEAPON);
    assert!(is_instance_id(healed["instanceId"].as_str().unwrap()));
    assert_eq!(
        (&healed["itemData"], &healed["updateVersion"]),
        (&Value::Null, &json!(0))
    );
    assert!(items.as_array().unwrap().iter().all(Value::is_object));
    assert_eq!(payload["loadouts"][0]["weapon"]["instance_id"], healed["instanceId"]);
}

#[tokio::test]
async fn missing_defaults_are_reconciled() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let me = Who::Player("p1");
    let inventory = h.inventory(&id).await.unwrap();
    let items: Vec<Value> = parse(&inventory.instanced_items)
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["catalogId"] != PARTS[0])
        .cloned()
        .collect();
    let stacked = json!([{"catalogId": "STACK_B", "quantity": 9}]);
    h.set_inventory(&id, &Value::Array(items), &stacked).await;
    for _ in 0..2 {
        h.get(&all("p1", &id), &me).await.payload();
    }
    let inventory = h.inventory(&id).await.unwrap();
    let items = parse(&inventory.instanced_items);
    let parts: Vec<&Value> = items
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["catalogId"] == PARTS[0])
        .collect();
    assert_eq!(parts.len(), 1);
    assert!(is_instance_id(parts[0]["instanceId"].as_str().unwrap()));
    assert_eq!(
        parse(&inventory.stacked_items),
        json!([{"catalogId": "STACK_B", "quantity": 9}, {"catalogId": "STACK_A", "quantity": 1}])
    );
}

#[tokio::test]
async fn a_read_with_nothing_to_repair_writes_nothing() {
    let h = Harness::bare().await;
    let id = h.first_character("p1").await;
    let before = h.log().await.len();
    h.get(&all("p1", &id), &Who::Player("p1")).await.payload();
    assert_eq!(h.log().await.len(), before);
}

#[tokio::test]
async fn the_seam_other_units_call() {
    let h = Harness::bare().await;
    let area = h.shared.slot::<Characters>().unwrap();
    let tables = h.shared.version().tables;
    let system = common::system();
    let list = area.characters_for_user(&h.store, tables, &system, "p1").await.unwrap();
    let id = list[0]["id"].as_str().unwrap().to_owned();
    let one = area
        .character_for_user(&h.store, tables, "p1", &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(one["id"], list[0]["id"]);
    assert!(
        area.character_for_user(&h.store, tables, "p2", &id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        area.loadout_state(&h.store, tables, &system, "p2", &id).await,
        Err(CharactersError::NotOwned)
    ));
    let state = area.loadout_state(&h.store, tables, &system, "p1", &id).await.unwrap();
    assert_eq!(loadout_payload(&state)["payload"]["num_character_slots"], 1);
    assert_eq!(jsjson::stringify(&empty_loadout_payload()), EMPTY);
    assert_eq!(area.instanced_catalog_ids(), common::kit().instanced.as_slice());
    assert_eq!(area.starter_wallet(), common::kit().wallet.as_slice());

    let account = h.account("p1").await.unwrap().id;
    let other = h.make_account("p2").await;
    let (me, mine, theirs) = (area.clone(), account.clone(), other.clone());
    let (in_tx, refused, fresh) = h
        .store
        .write(&system, move |tx, repos| {
            let found = me.ensure_starter_in(tx, repos, &TABLES, &mine, "p1", &id);
            let refused = me.ensure_starter_in(tx, repos, &TABLES, &theirs, "p2", &id);
            let fresh = is_fresh_onboarding(tx.conn(), repos, &theirs)?;
            Ok((found.is_ok(), refused.map_err(|e| e.to_string()).err(), fresh))
        })
        .await
        .unwrap();
    assert!(in_tx);
    assert_eq!(
        refused.unwrap(),
        format!(
            "Character {} does not belong to user p2",
            list[0]["id"].as_str().unwrap()
        )
    );
    assert!(fresh);
    let not_fresh = h
        .store
        .read(move |c, r| is_fresh_onboarding(c, r, &account))
        .await
        .unwrap();
    assert!(!not_fresh);
}
