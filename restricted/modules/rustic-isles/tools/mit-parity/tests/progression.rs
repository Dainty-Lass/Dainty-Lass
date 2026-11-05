//! `isles-core/tests/golden/progression/scenarios-1.14.7.json` (the Node progression services, `capture.ts`) against
//! the MIT module over the golden's game data: status and body text after restricted's normalisation (dates, minted
//! grant ids), Express's own 404 page answered by the unstubbed 404. Accounts and their characters are written
//! through the store; `reforge-open` opens the reforge row as restricted's replay does.
//!
//! Not replayed: the `host` role (a player-host runtime with a roster) and the `guard` steps that script rifts'
//! guard. Restricted fakes both with a test guard and a header-made runtime; the composed module runs dedicated-only,
//! and a real runtime needs a live hosted session. Their host-write hooks (asked, tagged, notes) are part of those
//! steps only.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::path::Path;

use common::{Caller, Mit, Tally, credentials, seed_account, seed_character, send, system};
use regex::Regex;
use serde_json::{Value, json};

fn normalise(text: &str) -> String {
    let date = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|\+00:00)").unwrap();
    let grant = Regex::new(r"gs-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let text = date.replace_all(text, "<date>").into_owned();
    grant.replace_all(&text, "gs-<uuid>").into_owned()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[tokio::test(flavor = "multi_thread")]
async fn progression() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/isles-core/tests/golden/progression");
    let recording: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("scenarios-1.14.7.json")).unwrap()).unwrap();
    let source = dir.join("game-data");
    let mit = Mit::start(&[], |target| {
        for entry in std::fs::read_dir(&source).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    })
    .await;
    let store = mit.store();
    let mut accounts = HashMap::new();
    for user in recording["users"].as_array().unwrap().iter().filter_map(Value::as_str) {
        accounts.insert(user.to_owned(), seed_account(&store, user, user).await);
    }
    for pair in recording["characters"].as_array().unwrap() {
        let (user, character) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
        seed_character(&store, &accounts[user], character, None).await;
    }
    let mut tally = Tally::new("progression");
    for (n, step) in recording["steps"].as_array().unwrap().iter().enumerate() {
        let name = step["name"].as_str().unwrap_or_default();
        match step.get("op").and_then(Value::as_str) {
            Some("guard") => {
                tally.skip(
                    n + 1,
                    name,
                    "-",
                    "-",
                    "scripts rifts' guard (restricted's test guard)",
                    Value::Null,
                );
                continue;
            }
            Some("reforge-open") => {
                let account = accounts[step["user"].as_str().unwrap()].clone();
                let track = step["track"].as_str().unwrap().to_owned();
                let reforge = rustic_isles_store::repo::progression::WeaponReforge {
                    reforge_id: format!("rf-{track}"),
                    open: true,
                    prior_progress: step["progress"].as_i64().unwrap_or(0),
                    cap: step["cap"].as_i64().unwrap_or(0),
                    banked: step["banked"].as_i64().unwrap_or(0),
                    banked_restored: 0,
                    prestige_granted: false,
                    opened_at: rustic_isles_auth::iso(now_ms() - step["ageMs"].as_i64().unwrap_or(0)),
                    closed_at: None,
                };
                let owner = account.clone();
                let opened = store
                    .account_write(&account, &system(), move |tx, repos| {
                        let row = match repos.progression_tracks.get(tx.conn(), &owner, &track)? {
                            Some(row) => row,
                            None => repos.progression_tracks.increment(tx, &owner, &track, 0)?,
                        };
                        repos.progression_tracks.begin_reforge(
                            tx,
                            &owner,
                            &track,
                            row.progress,
                            row.update_version,
                            &reforge,
                        )
                    })
                    .await
                    .unwrap();
                if opened.is_none() {
                    tally.note(format!("{name}: the reforge did not open"));
                }
                continue;
            }
            _ => {}
        }
        let request = &step["request"];
        let method = request["method"].as_str().unwrap_or("GET");
        let path = request["path"].as_str().unwrap_or("/");
        let user = request.get("user").and_then(Value::as_str);
        let mut headers = match request.get("role").and_then(Value::as_str) {
            Some("host") => {
                tally.skip(
                    n + 1,
                    name,
                    method,
                    path,
                    "a player-host runtime (needs realms and a live session)",
                    Value::Null,
                );
                continue;
            }
            Some("gs") => credentials(Caller::Gameserver(user)),
            Some("player") => credentials(user.map_or(Caller::Nobody, Caller::Player)),
            _ => credentials(Caller::Nobody),
        };
        for (k, v) in request["headers"].as_object().into_iter().flatten() {
            headers.push((k.clone(), v.as_str().unwrap_or_default().to_owned()));
        }
        let body = request.get("body").filter(|b| !b.is_null()).map(Value::to_string);
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let reply = send(mit.backend, method, path, &headers, body.as_deref().map(str::as_bytes)).await;
        let want = &step["response"];
        let want_body = want["body"].as_str().unwrap_or_default();
        // Express's own page for a path no route takes: the app's unstubbed 404 answers it empty
        let page = want["status"] == 404 && want_body.starts_with("<!DOCTYPE");
        let expected = json!({ "status": want["status"], "body": if page { json!("<express 404 page>") } else { json!(normalise(want_body)) } });
        let actual = json!({ "status": reply.status, "body": if page && reply.status == 404 { json!("<express 404 page>") } else { json!(normalise(&reply.body)) } });
        tally.step(n + 1, name, method, path, expected, actual);
    }
    tally.finish();
}
