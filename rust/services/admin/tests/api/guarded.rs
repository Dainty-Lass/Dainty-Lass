//! Guarded actions through the API (DD23 layer 4, DH6): a bulk ban waits out its delay or a second operator,
//! the owner too; it is canceled, approved or run once; and a restart keeps what waits.

use std::sync::atomic::Ordering;

use admin::guard::{GuardSettings, Rule};
use admin::keys::Role;

use super::*;

const MINUTE: i64 = 60_000;
const BAN: &str = "mutation($name: String!, $reason: String!) { ban(username: $name, reason: $reason) { status } }";
const TIMEOUT: &str = "mutation($name: String!, $minutes: Int!) {
  timeout(username: $name, minutes: $minutes, reason: \"cooling off\") { status } }";
const REVOKE: &str = "mutation($id: String!) { revokeKey(id: $id) { id } }";
const STATUS: &str = "{ serverStatus { name } }";
const FIELDS: &str = "id kind summary state mode initiatedBy needsRole releaseAt approvedBy canceledBy result";

fn approve_query() -> String {
    format!("mutation($id: String!) {{ approve(id: $id) {{ {FIELDS} }} }}")
}

fn cancel_query() -> String {
    format!("mutation($id: String!) {{ cancel(id: $id) {{ {FIELDS} }} }}")
}

fn list(all: bool) -> String {
    format!("{{ guarded(all: {all}) {{ {FIELDS} }} }}")
}

/// One ban or timeout is free, the next is held.
fn settings(rule: Rule) -> GuardSettings {
    GuardSettings {
        ban_limit: 1,
        ban_guard: rule,
        ..GuardSettings::default()
    }
}

async fn guarded_world(guard: GuardSettings) -> World {
    let options = Options {
        director_up: true,
        secret: SECRET,
        modules: Arc::new(AdminPlugin::none()),
    };
    world_with_guard(options, guard).await
}

/// The body of a request with `key`.
async fn call(w: &World, key: &str, query: &str, variables: Value) -> Value {
    w.gql_as(key, query, variables).await.1
}

fn held_id(body: &Value) -> String {
    assert_eq!(code(body), "GUARDED", "{body}");
    body["errors"][0]["extensions"]["guardedId"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// What the audit rows say: (key, action, approved_by) for the actions named.
async fn audited(w: &World, actions: &[&str]) -> Vec<(String, String, Option<String>)> {
    let log = w.admin.keys.database().changes(0, 10_000).await.unwrap();
    log.iter()
        .filter(|e| e.table_name == "admin_actions" && e.op == "insert")
        .map(|e| serde_json::from_str::<Value>(e.after.as_deref().unwrap()).unwrap())
        .filter(|row| actions.contains(&row["action"].as_str().unwrap()))
        .map(|row| {
            (
                row["key_id"].as_str().unwrap().to_owned(),
                row["action"].as_str().unwrap().to_owned(),
                row["approved_by"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn ban_args(name: &str) -> Value {
    json!({"name": name, "reason": "griefing hunts"})
}

fn owner() -> admin::keys::Caller {
    admin::keys::Caller {
        key: "owner".into(),
        role: Role::Owner,
        operator: admin::upstream::Operator::host(),
    }
}

#[tokio::test]
async fn everyday_moderation_is_immediate_and_old_installs_behave_as_before() {
    let w = guarded_world(GuardSettings::default()).await;
    let (_, maya) = w.issue(Role::Moderator, "Maya").await;
    for name in ["Bo_Slayer", "Ada"] {
        let body = call(&w, &maya, BAN, ban_args(name)).await;
        assert_eq!(body["data"]["ban"]["status"], "BANNED", "{body}");
    }
    assert_eq!(w.gql(&list(true), json!({})).await["data"]["guarded"], json!([]));
    // With the guard off, the limit does not matter
    let w = guarded_world(settings(Rule::Off)).await;
    for name in ["Bo_Slayer", "Ada"] {
        assert_eq!(w.gql(BAN, ban_args(name)).await["data"]["ban"]["status"], "BANNED");
    }
}

#[tokio::test]
async fn a_bulk_ban_waits_out_its_delay_and_then_runs_once() {
    let w = guarded_world(settings(Rule::Delay)).await;
    let (maya_id, maya) = w.issue(Role::Moderator, "Maya").await;
    let body = call(&w, &maya, BAN, ban_args("Bo_Slayer")).await;
    assert_eq!(body["data"]["ban"]["status"], "BANNED");

    let body = call(&w, &maya, BAN, ban_args("Ada")).await;
    let id = held_id(&body);
    assert_eq!(body["errors"][0]["extensions"]["mode"], "DELAY");
    assert_eq!(body["errors"][0]["extensions"]["already"], false);
    assert_eq!(w.status_of("u-ada"), "active", "held, not done");
    // Asking again waits once
    let again = call(&w, &maya, BAN, ban_args("ada")).await;
    assert_eq!(held_id(&again), id);
    assert_eq!(again["errors"][0]["extensions"]["already"], true);

    let waiting = w.gql(&list(false), json!({})).await["data"]["guarded"].clone();
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    assert_eq!(waiting[0]["id"], json!(id));
    assert_eq!(waiting[0]["state"], "PENDING");
    assert_eq!(waiting[0]["kind"], "ban");
    assert_eq!(waiting[0]["initiatedBy"], json!(maya_id));
    assert!(waiting[0]["summary"].as_str().unwrap().contains("Ada"));

    // A delay is not approved early, not even by the owner
    let body = w.gql(&approve_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "NOT_WAITING", "{body}");

    // Nine minutes in: still waiting. Ten: the sweep runs it, as Maya
    w.clock.fetch_add(9 * MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    assert_eq!(w.status_of("u-ada"), "active");
    w.clock.fetch_add(MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    assert_eq!(w.status_of("u-ada"), "banned");
    let done = w.gql(&list(true), json!({})).await["data"]["guarded"][0].clone();
    assert_eq!(done["state"], "DONE");
    assert_eq!(done["result"], "banned Ada");

    // Once: more sweeps and a late approval change nothing
    for _ in 0..3 {
        w.admin.guard_sweep().await;
    }
    let body = w.gql(&approve_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "NOT_WAITING");
    let maya_row = |action: &str| (maya_id.clone(), action.to_owned(), None);
    assert_eq!(
        audited(&w, &["ban", "guardHold"]).await,
        [maya_row("ban"), maya_row("guardHold"), maya_row("ban")]
    );
}

#[tokio::test]
async fn a_second_operator_cancels_a_bulk_action_while_it_waits() {
    let w = guarded_world(settings(Rule::Approval)).await;
    let (_, maya) = w.issue(Role::Moderator, "Maya").await;
    let (_, noor) = w.issue(Role::Moderator, "Noor").await;
    call(&w, &maya, BAN, ban_args("Bo_Slayer")).await;
    let body = call(&w, &maya, BAN, ban_args("Ada")).await;
    let id = held_id(&body);
    assert_eq!(body["errors"][0]["extensions"]["mode"], "APPROVAL");
    assert_eq!(body["errors"][0]["extensions"]["releaseAt"], Value::Null);

    // A moderator who did not ask may not cancel it; the owner may
    let body = call(&w, &noor, &cancel_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "FORBIDDEN", "{body}");
    let body = w.gql(&cancel_query(), json!({"id": id})).await;
    assert_eq!(body["data"]["cancel"]["state"], "CANCELED", "{body}");
    assert_eq!(body["data"]["cancel"]["canceledBy"], "owner");

    // The one who asked cancels their own
    let body = call(&w, &maya, BAN, ban_args("Ada")).await;
    let second = held_id(&body);
    assert_ne!(second, id);
    let body = call(&w, &maya, &cancel_query(), json!({"id": second})).await;
    assert_eq!(body["data"]["cancel"]["state"], "CANCELED");
    assert_eq!(w.status_of("u-ada"), "active");

    // Nothing runs afterwards, however long it waits, and it cannot be approved or canceled again
    w.clock.fetch_add(100 * 60 * MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    assert_eq!(w.status_of("u-ada"), "active");
    assert_eq!(
        code(&call(&w, &noor, &approve_query(), json!({"id": id})).await),
        "NOT_WAITING"
    );
    assert_eq!(code(&w.gql(&cancel_query(), json!({"id": id})).await), "NOT_WAITING");
}

#[tokio::test]
async fn approval_by_the_same_key_or_a_lesser_role_is_refused() {
    let w = guarded_world(settings(Rule::Approval)).await;
    let (_, maya) = w.issue(Role::Moderator, "Maya").await;
    let (ari_id, ari) = w.issue(Role::Admin, "Ari").await;
    let (ben_id, ben) = w.issue(Role::Admin, "Ben").await;
    call(&w, &ari, BAN, ban_args("Bo_Slayer")).await;
    let body = call(&w, &ari, BAN, ban_args("Ada")).await;
    let id = held_id(&body);
    assert_eq!(body["errors"][0]["extensions"]["needsRole"], "admin");

    // The same key: refused, and nothing changed
    let body = call(&w, &ari, &approve_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "FORBIDDEN", "{body}");
    assert!(
        body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("second operator")
    );
    // A moderator is below the admin who asked
    let body = call(&w, &maya, &approve_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "FORBIDDEN");
    assert_eq!(body["errors"][0]["extensions"]["requiredRole"], "admin");
    assert_eq!(w.status_of("u-ada"), "active");
    assert_eq!(
        w.gql(&list(false), json!({})).await["data"]["guarded"][0]["state"],
        "PENDING"
    );

    // Another admin approves: it runs now, as the key that asked, and the audit row names both
    let body = call(&w, &ben, &approve_query(), json!({"id": id})).await;
    assert_eq!(body["data"]["approve"]["state"], "DONE", "{body}");
    assert_eq!(body["data"]["approve"]["approvedBy"], json!(ben_id));
    assert_eq!(body["data"]["approve"]["initiatedBy"], json!(ari_id));
    assert_eq!(w.status_of("u-ada"), "banned");

    // Approving again runs nothing a second time
    let body = call(&w, &ben, &approve_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "NOT_WAITING");
    w.admin.guard_sweep().await;
    assert_eq!(
        audited(&w, &["ban"]).await,
        [
            (ari_id.clone(), "ban".to_owned(), None),
            (ari_id.clone(), "ban".to_owned(), Some(ben_id.clone()))
        ]
    );
    // The change log tells the same story: the approval was Ben, the running was Ari
    let log = w.admin.keys.database().changes(0, 10_000).await.unwrap();
    let steps: Vec<(&str, String)> = log
        .iter()
        .filter(|e| e.table_name == "guarded_actions")
        .map(|e| {
            let after: Value = serde_json::from_str(e.after.as_deref().unwrap()).unwrap();
            (e.actor.as_str(), after["state"].as_str().unwrap().to_owned())
        })
        .collect();
    let (ari_actor, ben_actor) = (format!("admin:{ari_id}"), format!("admin:{ben_id}"));
    assert_eq!(
        steps,
        [
            (ari_actor.as_str(), "pending".to_owned()),
            (ben_actor.as_str(), "approved".to_owned()),
            (ari_actor.as_str(), "running".to_owned()),
            (ari_actor.as_str(), "done".to_owned()),
        ]
    );
}

#[tokio::test]
async fn the_owner_waits_too_and_needs_a_second_owner() {
    let w = guarded_world(settings(Rule::Approval)).await;
    w.gql(BAN, ban_args("Bo_Slayer")).await;
    let body = w.gql(BAN, ban_args("Ada")).await;
    let id = held_id(&body);
    assert_eq!(body["errors"][0]["extensions"]["needsRole"], "owner");
    // The owner key cannot approve its own; an admin is below an owner
    assert_eq!(code(&w.gql(&approve_query(), json!({"id": id})).await), "FORBIDDEN");
    let (_, admin) = w.issue(Role::Admin, "Ari").await;
    let body = call(&w, &admin, &approve_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "FORBIDDEN");
    assert_eq!(w.status_of("u-ada"), "active");
    // A second owner key does
    let (second_id, second) = w.issue(Role::Owner, "Co-owner").await;
    let body = call(&w, &second, &approve_query(), json!({"id": id})).await;
    assert_eq!(body["data"]["approve"]["state"], "DONE", "{body}");
    assert_eq!(body["data"]["approve"]["approvedBy"], json!(second_id));
    assert_eq!(w.status_of("u-ada"), "banned");
    assert_eq!(
        audited(&w, &["ban"]).await[1],
        ("owner".to_owned(), "ban".to_owned(), Some(second_id))
    );
}

#[tokio::test]
async fn a_held_approval_expires_when_nobody_comes() {
    let w = guarded_world(settings(Rule::Approval)).await;
    w.gql(BAN, ban_args("Bo_Slayer")).await;
    let id = held_id(&w.gql(BAN, ban_args("Ada")).await);
    w.clock.fetch_add(47 * 60 * MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    assert_eq!(
        w.gql(&list(false), json!({})).await["data"]["guarded"][0]["state"],
        "PENDING"
    );
    w.clock.fetch_add(61 * MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    let (_, second) = w.issue(Role::Owner, "Co-owner").await;
    let body = call(&w, &second, &approve_query(), json!({"id": id})).await;
    assert_eq!(code(&body), "NOT_WAITING", "{body}");
    let all = w.gql(&list(true), json!({})).await["data"]["guarded"][0].clone();
    assert_eq!(all["state"], "EXPIRED");
    assert_eq!(w.status_of("u-ada"), "active");
}

#[tokio::test]
async fn revoking_an_admin_key_waits_but_a_moderator_key_goes_at_once() {
    let w = guarded_world(GuardSettings::default()).await;
    let (ari_id, ari) = w.issue(Role::Admin, "Ari").await;
    let (maya_id, maya) = w.issue(Role::Moderator, "Maya").await;
    let body = w.gql(REVOKE, json!({"id": maya_id})).await;
    assert_eq!(body["data"]["revokeKey"]["id"], json!(maya_id), "{body}");
    assert_eq!(w.gql_as(&maya, STATUS, json!({})).await.0, 401);

    let id = held_id(&w.gql(REVOKE, json!({"id": ari_id})).await);
    assert_eq!(
        w.gql_as(&ari, STATUS, json!({})).await.0,
        200,
        "still live while it waits"
    );
    w.clock.fetch_add(10 * MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    assert_eq!(w.gql_as(&ari, STATUS, json!({})).await.0, 401);
    let record = w.admin.guard.get(&id).await.unwrap();
    assert_eq!(record.result.as_deref(), Some(format!("revoked key {ari_id}").as_str()));
    // The key store logged the revoke under the owner, who asked
    let keys = w.gql("{ adminKeys { id revokedBy } }", json!({})).await;
    let rows = keys["data"]["adminKeys"].as_array().unwrap();
    let row = rows.iter().find(|k| k["id"] == json!(ari_id)).unwrap();
    assert_eq!(row["revokedBy"], "admin:owner");
}

#[tokio::test]
async fn a_held_action_does_not_run_for_a_key_that_was_revoked_meanwhile() {
    let w = guarded_world(settings(Rule::Delay)).await;
    let (maya_id, maya) = w.issue(Role::Moderator, "Maya").await;
    call(&w, &maya, BAN, ban_args("Bo_Slayer")).await;
    let id = held_id(&call(&w, &maya, BAN, ban_args("Ada")).await);
    w.gql(REVOKE, json!({"id": maya_id})).await;
    w.clock.fetch_add(10 * MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    assert_eq!(w.status_of("u-ada"), "active");
    let record = w.admin.guard.get(&id).await.unwrap();
    assert_eq!(record.state.as_str(), "failed");
    assert!(record.result.unwrap().contains("revoked"));
}

#[tokio::test]
async fn a_held_timeout_counts_its_minutes_from_when_it_runs() {
    let w = guarded_world(settings(Rule::Delay)).await;
    w.gql(BAN, ban_args("Bo_Slayer")).await;
    let id = held_id(&w.gql(TIMEOUT, json!({"name": "Ada", "minutes": 30})).await);
    // A span that is not valid is refused before anything is held
    assert_eq!(
        code(&w.gql(TIMEOUT, json!({"name": "Cyra", "minutes": -5})).await),
        "BAD_INPUT"
    );
    w.clock.fetch_add(10 * MINUTE, Ordering::SeqCst);
    w.admin.guard_sweep().await;
    let after = w.admin.guard.get(&id).await.unwrap();
    assert_eq!(after.state.as_str(), "done", "{after:?}");
    let until = w.admin.timeouts.get("u-ada").await.unwrap().until;
    assert_eq!(until, START + 40 * MINUTE);
}

#[tokio::test]
async fn a_restart_keeps_what_waits_and_runs_it_once() {
    let w = guarded_world(settings(Rule::Delay)).await;
    let (_, maya) = w.issue(Role::Moderator, "Maya").await;
    call(&w, &maya, BAN, ban_args("Bo_Slayer")).await;
    let id = held_id(&call(&w, &maya, BAN, ban_args("Ada")).await);

    // The server stops and starts again on the same database
    let reading = w.clock.clone();
    let (_, second) = admin::server::prepare_with_modules(
        &w.config,
        Arc::new(move || reading.load(Ordering::SeqCst)),
        Arc::new(AdminPlugin::none()),
    )
    .unwrap();
    second.guard_start().await;
    let waiting = second.guarded(&owner(), false).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id, id);
    w.clock.fetch_add(10 * MINUTE, Ordering::SeqCst);
    second.guard_sweep().await;
    second.guard_sweep().await;
    assert_eq!(w.status_of("u-ada"), "banned");
    // The first instance, still alive beside it, finds nothing left to do
    w.admin.guard_sweep().await;
    assert_eq!(audited(&w, &["ban"]).await.len(), 2);
}

#[tokio::test]
async fn an_action_the_server_stopped_in_the_middle_of_is_failed_and_never_run_again() {
    let w = guarded_world(settings(Rule::Delay)).await;
    w.gql(BAN, ban_args("Bo_Slayer")).await;
    let id = held_id(&w.gql(BAN, ban_args("Ada")).await);
    w.clock.fetch_add(10 * MINUTE, Ordering::SeqCst);
    // Claimed, as the sweep does, and then the process dies before it finishes
    let now = w.clock.load(Ordering::SeqCst);
    assert!(w.admin.guard.claim(&id, now).await.unwrap().is_some());
    assert!(w.admin.guard.claim(&id, now).await.unwrap().is_none(), "claimed once");
    w.admin.guard_start().await;
    w.admin.guard_sweep().await;
    let record = w.admin.guard.get(&id).await.unwrap();
    assert_eq!(record.state.as_str(), "failed");
    assert!(record.result.unwrap().contains("stopped"));
    assert_eq!(w.status_of("u-ada"), "active");
}

#[tokio::test]
async fn a_viewer_cannot_see_or_decide_and_a_bad_id_is_refused() {
    let w = guarded_world(settings(Rule::Delay)).await;
    let (_, viewer) = w.issue(Role::Viewer, "Vic").await;
    assert_eq!(code(&call(&w, &viewer, &list(false), json!({})).await), "FORBIDDEN");
    for query in [approve_query(), cancel_query()] {
        let known = json!({"id": "0123456789abcdef"});
        assert_eq!(code(&call(&w, &viewer, &query, known.clone()).await), "FORBIDDEN");
        assert_eq!(code(&w.gql(&query, json!({"id": "nope"})).await), "BAD_INPUT");
        assert_eq!(code(&w.gql(&query, known).await), "NOT_FOUND");
    }
}
