//! `isles-core/tests/golden/inventory/` (Rustic Isles' Node inventory and reforge routes, `record.cjs`) against the
//! MIT module: the seed written through the store (accounts named `Hunter <userId>`, characters with their
//! inventories at revision 0, wallets with exactly the seeded balances, progression tracks), the golden game data,
//! `GAMESERVER_NO_PLAYER_USER_ID=u1`, and status, content type and body compared after the recorder's `normalize`.
//!
//! The recording's `x-test-auth` AuthData becomes the real credential: `IsGameserver` with a `userId` is the game
//! server key with that player's token, `IsGameserver` alone the key alone (the middleware then names `u1`, auth
//! §4.1), a `userId` alone that player's token.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;

use common::{Caller, Mit, Tally, credentials, seed_account, seed_character, seed_track, seed_wallet, send};
use isles_core::areas::inventory::testkit::normalize;
use serde_json::{Value, json};

/// The recording stubbed the characters area's starter bootstrap (an existing inventory served as it is); the composed
/// module runs it, and it repairs a seeded inventory that has no loadout with the starter kit (characters §7.2). So an
/// item the answer has and the recording lacks, whose catalog id is a starter one, is left out. The count comes back.
fn without_starter_items(actual: &str, recorded: &str) -> (String, usize) {
    let (Ok(mut value), Ok(want)) = (
        serde_json::from_str::<Value>(actual),
        serde_json::from_str::<Value>(recorded),
    ) else {
        return (actual.to_owned(), 0);
    };
    let kit = common::version_data::document();
    let mut starter: Vec<String> = kit["starterKit"]["instanced"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    starter.extend(
        rustic_isles_1_14_7::TABLES
            .starter_manifest
            .stacked_item_ids
            .iter()
            .map(|s| (*s).to_owned()),
    );
    let mut stripped = 0;
    for list in ["instancedItems", "stackedItems"] {
        let recorded_ids: Vec<&str> = want[list]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["catalogId"].as_str())
            .collect();
        if let Some(items) = value.get_mut(list).and_then(Value::as_array_mut) {
            let before = items.len();
            items.retain(|item| {
                let id = item["catalogId"].as_str().unwrap_or_default();
                !(starter.iter().any(|s| s == id) && !recorded_ids.contains(&id))
            });
            stripped += before - items.len();
        }
    }
    if stripped == 0 {
        return (actual.to_owned(), 0);
    }
    (value.to_string(), stripped)
}

#[tokio::test(flavor = "multi_thread")]
async fn inventory() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../isles-core/tests/golden/inventory");
    let scenarios: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("scenarios.json")).unwrap()).unwrap();
    let expected: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    let source = dir.join("game-data");
    let mit = Mit::start(&[("GAMESERVER_NO_PLAYER_USER_ID", "u1")], |target| {
        for entry in std::fs::read_dir(&source).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    })
    .await;
    let store = mit.store();
    for user in scenarios["seed"]["users"].as_array().unwrap() {
        let user_id = user["userId"].as_str().unwrap();
        let account = seed_account(&store, user_id, &format!("Hunter {user_id}")).await;
        let balances = user["wallet"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(id, n)| (id.clone(), n.as_f64().unwrap_or(0.0)))
            .collect();
        seed_wallet(&store, &account, balances).await;
        for character in user["characters"].as_array().into_iter().flatten() {
            let (instanced, stacked) = (character["instanced"].to_string(), character["stacked"].to_string());
            seed_character(
                &store,
                &account,
                character["characterId"].as_str().unwrap(),
                Some((&instanced, &stacked)),
            )
            .await;
        }
        for (track, progress) in user["tracks"].as_object().into_iter().flatten() {
            seed_track(&store, &account, track, progress.as_i64().unwrap_or(0)).await;
        }
    }
    let mut tally = Tally::new("inventory");
    for (n, (scenario, want)) in scenarios["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&expected)
        .enumerate()
    {
        let name = scenario["name"].as_str().unwrap_or_default();
        assert_eq!(want["name"].as_str(), Some(name));
        let mut headers = match scenario.get("auth") {
            Some(auth) if auth.is_object() => {
                let user = auth.get("userId").and_then(Value::as_str);
                if auth.get("IsGameserver").and_then(Value::as_bool) == Some(true) {
                    credentials(Caller::Gameserver(user))
                } else {
                    credentials(user.map_or(Caller::Nobody, Caller::Player))
                }
            }
            _ => credentials(Caller::Nobody),
        };
        let body = match (scenario.get("raw"), scenario.get("body")) {
            (Some(raw), _) => Some(raw.as_str().unwrap_or_default().to_owned()),
            (None, Some(body)) => Some(body.to_string()),
            _ => None,
        };
        if body.is_some() {
            let content_type = scenario
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap_or("application/json");
            headers.push(("content-type".into(), content_type.to_owned()));
        }
        let method = scenario["method"].as_str().unwrap_or("GET");
        let path = scenario["path"].as_str().unwrap_or("/");
        let reply = send(mit.backend, method, path, &headers, body.as_deref().map(str::as_bytes)).await;
        let expected = json!({ "status": want["status"], "content-type": want["contentType"], "body": want["body"] });
        let (body, stripped) = without_starter_items(&normalize(&reply.body), want["body"].as_str().unwrap_or(""));
        if stripped > 0 {
            tally.note(format!(
                "{name}: {stripped} starter item(s) the bootstrap added left out of the comparison"
            ));
        }
        let actual = json!({ "status": reply.status, "content-type": reply.header("content-type"), "body": body });
        tally.step(n + 1, name, method, path, expected, actual);
    }
    tally.finish();
}
