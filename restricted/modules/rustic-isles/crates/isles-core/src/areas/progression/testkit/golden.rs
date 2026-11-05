//! Replays a recording of the Node services (`tests/golden/progression/capture.ts`) against the Rust handlers and
//! lists where the answers differ. Bodies are compared as text, so field order and number formatting count; dates and
//! minted grant ids are normalised on both sides.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use module_api::BoxFuture;
use regex::Regex;
use rifts_seam::{HostWriteKind, Json as RiftsJson, Reply};
use serde_json::Value;

use crate::AccountId;
use crate::http::Shared;
use crate::repo::progression::{WeaponReforge, now_iso};
use crate::version::IslesVersion;

use super::{Harness, Options, RecordingGuard, TestRequest};
use crate::areas::progression::{Characters, Ports};

/// Node's `HostWriteKind` names.
pub fn kind_name(kind: HostWriteKind) -> &'static str {
    match kind {
        HostWriteKind::Xp => "XP",
        HostWriteKind::Mastery => "MASTERY",
        HostWriteKind::Objectives => "OBJECTIVES",
        HostWriteKind::Escalation => "ESCALATION",
        HostWriteKind::Cooldown => "COOLDOWN",
        HostWriteKind::Bounty => "BOUNTY",
        HostWriteKind::SlayersPath => "SLAYERS_PATH",
        HostWriteKind::Reforge => "REFORGE",
        HostWriteKind::RankConfirm => "RANK_CONFIRM",
        HostWriteKind::LoadoutUnlock => "LOADOUT_UNLOCK",
        HostWriteKind::HuntPassXp => "HUNT_PASS_XP",
        HostWriteKind::ItemSave => "ITEM_SAVE",
    }
}

fn kind_of(name: &str) -> Option<HostWriteKind> {
    [
        HostWriteKind::Xp,
        HostWriteKind::Mastery,
        HostWriteKind::Objectives,
        HostWriteKind::Escalation,
        HostWriteKind::Cooldown,
        HostWriteKind::Bounty,
        HostWriteKind::SlayersPath,
        HostWriteKind::Reforge,
        HostWriteKind::RankConfirm,
        HostWriteKind::LoadoutUnlock,
        HostWriteKind::HuntPassXp,
        HostWriteKind::ItemSave,
    ]
    .into_iter()
    .find(|kind| kind_name(*kind) == name)
}

/// The characters of the recording: which account each belongs to.
#[derive(Default)]
struct FakeCharacters(Mutex<HashMap<String, AccountId>>);

impl Characters for FakeCharacters {
    fn owns<'a>(&'a self, _: &'a Shared, account: &'a AccountId, character_id: &'a str) -> BoxFuture<'a, bool> {
        let owned = self.0.lock().is_ok_and(|held| held.get(character_id) == Some(account));
        Box::pin(async move { owned })
    }
}

fn normalise(text: &str) -> String {
    let date = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|\+00:00)");
    let grant = Regex::new(r"gs-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}");
    let text = match date {
        Ok(date) => date.replace_all(text, "<date>").into_owned(),
        Err(_) => text.to_owned(),
    };
    match grant {
        Ok(grant) => grant.replace_all(&text, "gs-<uuid>").into_owned(),
        Err(_) => text,
    }
}

/// `JSON.stringify` then parse, so a whole number is one number whichever way it was made.
fn canonical(value: &Value) -> Value {
    serde_json::from_str(&service_kit::jsjson::stringify(value)).unwrap_or(Value::Null)
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Replays `recording` on a fresh database under `dir`; the differences, empty when the Rust area answers as Node did.
pub async fn replay(
    database: store::DatabaseConfig,
    version: IslesVersion,
    game_data: &Path,
    recording: &Value,
) -> Vec<String> {
    let guard = RecordingGuard::new(true);
    let characters = Arc::new(FakeCharacters::default());
    let ports = Ports {
        characters: characters.clone(),
        inventory: Arc::new(crate::areas::progression::UnwiredInventory),
        starter: Arc::new(crate::areas::progression::NoStarterBootstrap),
        ..Ports::default()
    };
    let harness = match Harness::start(Options {
        version,
        game_data: Some(game_data.to_owned()),
        database,
        settings: Vec::new(),
        ports,
        guard: Some(guard.clone()),
    })
    .await
    {
        Ok(harness) => harness,
        Err(error) => return vec![format!("start: {error}")],
    };
    let mut accounts: HashMap<String, AccountId> = HashMap::new();
    for user in recording["users"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        match harness.account(user).await {
            Ok(account) => {
                accounts.insert(user.to_owned(), account);
            }
            Err(error) => return vec![format!("account {user}: {error}")],
        }
    }
    for pair in recording["characters"].as_array().into_iter().flatten() {
        let (Some(user), Some(character)) = (pair[0].as_str(), pair[1].as_str()) else {
            continue;
        };
        if let (Some(account), Ok(mut held)) = (accounts.get(user), characters.0.lock()) {
            held.insert(character.to_owned(), account.clone());
        }
    }

    let mut differences = Vec::new();
    for step in recording["steps"].as_array().into_iter().flatten() {
        let name = str_of(step, "name");
        match step.get("op").and_then(Value::as_str) {
            Some("guard") => {
                if let Ok(mut denial) = guard.denial.lock() {
                    *denial = step.get("denial").filter(|d| !d.is_null()).map(|d| Reply {
                        status: d["status"].as_u64().unwrap_or(403) as u16,
                        body: RiftsJson(d["body"].to_string()),
                    });
                }
                guard.policy_refuses.store(
                    step["policyRefuses"].as_bool().unwrap_or(true),
                    std::sync::atomic::Ordering::SeqCst,
                );
                if let Ok(mut probes) = guard.probe_refuses.lock() {
                    *probes = step["probeRefuses"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|k| kind_of(k.as_str()?))
                        .collect();
                }
            }
            Some("reforge-open") => {
                if let Err(error) = open_reforge(&harness, &accounts, step).await {
                    differences.push(format!("{name}: {error}"));
                }
            }
            _ => {
                if let Some(difference) = replay_request(&harness, &guard, step).await {
                    differences.push(format!("{name}: {difference}"));
                }
            }
        }
    }
    differences
}

async fn open_reforge(harness: &Harness, accounts: &HashMap<String, AccountId>, step: &Value) -> Result<(), String> {
    let account = accounts.get(str_of(step, "user")).cloned().ok_or("no such user")?;
    let track = str_of(step, "track").to_owned();
    let age = step["ageMs"].as_i64().unwrap_or(0);
    let opened = chrono::Utc::now() - chrono::Duration::milliseconds(age);
    let reforge = WeaponReforge {
        reforge_id: format!("rf-{track}"),
        open: true,
        prior_progress: step["progress"].as_i64().unwrap_or(0),
        cap: step["cap"].as_i64().unwrap_or(0),
        banked: step["banked"].as_i64().unwrap_or(0),
        banked_restored: 0,
        prestige_granted: false,
        opened_at: opened.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        closed_at: None,
    };
    let owner = account.clone();
    let opened = harness
        .store
        .account_write(
            &account,
            &module_api::ActorContext::new(module_api::Actor::System),
            move |tx, repos| {
                let row = match repos.progression_tracks.get(tx.conn(), &owner, &track)? {
                    Some(row) => row,
                    None => repos.progression_tracks.increment(tx, &owner, &track, 0)?,
                };
                repos
                    .progression_tracks
                    .begin_reforge(tx, &owner, &track, row.progress, row.update_version, &reforge)
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    let _ = now_iso;
    opened.map(|_| ()).ok_or_else(|| "the reforge did not open".to_owned())
}

async fn replay_request(harness: &Harness, guard: &Arc<RecordingGuard>, step: &Value) -> Option<String> {
    if let Ok(mut asked) = guard.authorized.lock() {
        asked.clear();
    }
    if let Ok(mut records) = guard.records.lock() {
        records.clear();
    }
    if let Ok(mut notes) = guard.notes.lock() {
        notes.clear();
    }
    let request = &step["request"];
    let mut sent = TestRequest::new(str_of(request, "method"), str_of(request, "path"));
    if let Some(role) = request.get("role").and_then(Value::as_str) {
        sent = sent.as_role(role, request.get("user").and_then(Value::as_str));
    }
    if let Some(roster) = request.get("roster").and_then(Value::as_str) {
        sent = sent.header("x-test-roster", roster);
    }
    for (name, value) in request["headers"].as_object().into_iter().flatten() {
        sent = sent.header(name, value.as_str().unwrap_or_default());
    }
    if let Some(body) = request.get("body").filter(|b| !b.is_null()) {
        sent = sent.json(body);
    }
    let answer = match harness.send(&sent).await {
        Ok(answer) => answer,
        Err(error) => return Some(error),
    };
    let want = &step["response"];
    let want_status = want["status"].as_u64().unwrap_or(0) as u16;
    let want_body = str_of(want, "body");
    let mut problems = Vec::new();
    if answer.status != want_status {
        problems.push(format!(
            "status {} (Node {want_status}): {}",
            answer.status, answer.body
        ));
    }
    // Express's own page for a path no route takes: the app's unstubbed 404 answers it empty
    let express_page = want_status == 404 && want_body.starts_with("<!DOCTYPE");
    if !express_page && normalise(&answer.body) != normalise(want_body) {
        problems.push(format!(
            "body\n  rust: {}\n  node: {}",
            normalise(&answer.body),
            normalise(want_body)
        ));
    }
    let want_asked: Vec<(String, String)> = want_hooks(step)["authorized"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|pair| (str_of_index(pair, 0), str_of_index(pair, 1)))
        .collect();
    let asked: Vec<(String, String)> = guard
        .asked()
        .into_iter()
        .map(|(k, t)| (kind_name(k).to_owned(), t))
        .collect();
    if asked != want_asked {
        problems.push(format!("host writes asked {asked:?} (Node {want_asked:?})"));
    }
    let want_records: Vec<(String, String, Value)> = want_hooks(step)["records"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            (
                str_of(r, "kind").to_owned(),
                str_of(r, "target").to_owned(),
                canonical(&r["summary"]),
            )
        })
        .collect();
    let records: Vec<(String, String, Value)> = guard
        .tagged()
        .into_iter()
        .map(|(k, t, s)| (kind_name(k).to_owned(), t, canonical(&s)))
        .collect();
    if records != want_records {
        problems.push(format!("host tags\n  rust: {records:?}\n  node: {want_records:?}"));
    }
    let want_notes = want_hooks(step)["notes"].as_u64().unwrap_or(0) as usize;
    let notes = guard.notes.lock().map_or(0, |n| n.len());
    if notes != want_notes {
        problems.push(format!("{notes} would-refuse notes (Node {want_notes})"));
    }
    (!problems.is_empty()).then(|| problems.join("\n"))
}

fn want_hooks(step: &Value) -> &Value {
    &step["hooks"]
}

fn str_of_index(value: &Value, index: usize) -> String {
    value.get(index).and_then(Value::as_str).unwrap_or_default().to_owned()
}
