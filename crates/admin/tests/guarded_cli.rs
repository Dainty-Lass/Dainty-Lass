//! `dainty-host admin guarded list|approve|cancel` against the admin.env setup writes (DH6): the host kit
//! records a decision, as the host, and the server carries it out.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::Path;

use admin::cli::main_from;
use admin::config::Config;
use admin::guard::{Guards, Kind, New, Rule, State};
use admin::keys::{Keys, Role};
use serde_json::json;
use service_kit::env::Env;

const SECRET: &str = "0123456789abcdef0123456789abcdef";

fn write_env(dir: &Path, extra: &str) -> String {
    let file = dir.join("admin.env");
    let text = format!(
        "REFORGED_ADMIN_API_KEY={SECRET}\nREFORGED_ADMIN_KEY={SECRET}\nRUSTIC_ISLES_MANAGER_OPERATOR_SECRET={SECRET}\n\
         REFORGED_ADMIN_API_TIMEOUTS_FILE={}\n{extra}",
        dir.join("admin-timeouts.json").display().to_string().replace("\\", "/")
    );
    std::fs::write(&file, text).unwrap();
    file.display().to_string()
}

fn run(args: &[String]) -> bool {
    let argv = std::iter::once("admin".to_owned()).chain(args.iter().cloned());
    format!("{:?}", main_from(argv)) == format!("{:?}", std::process::ExitCode::SUCCESS)
}

fn held(subject: &str, rule: Rule) -> New {
    New {
        kind: Kind::Ban,
        subject: subject.to_owned(),
        summary: format!("ban {subject}"),
        params: json!({"name": subject}),
        rule,
        initiated_by: "aaaa0001".to_owned(),
        initiator_role: Role::Moderator,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_lists_approves_and_cancels_under_its_own_actor() {
    let dir = tempfile::tempdir().unwrap();
    let env = write_env(dir.path(), "REFORGED_GUARD_BAN=approval\n");
    let go = |rest: &[&str]| {
        let args: Vec<String> = rest
            .iter()
            .map(|s| (*s).to_owned())
            .chain(["--env-file".to_owned(), env.clone()])
            .collect();
        tokio::task::block_in_place(|| run(&args))
    };

    let config = Config::from_env(&Env::load(Some(Path::new(&env))).unwrap()).unwrap();
    assert_eq!(config.guard.ban_guard, Rule::Approval);
    let keys = Keys::open(&Keys::database_for(&config.database), &config.api_key).unwrap();
    let guards = Guards::new(keys.database().clone(), config.guard.clone());
    let approval = guards
        .hold(held("ada", Rule::Approval), admin::now_ms())
        .await
        .unwrap()
        .record()
        .clone();
    let other = guards
        .hold(held("bo", Rule::Approval), admin::now_ms())
        .await
        .unwrap()
        .record()
        .clone();
    let delay = guards
        .hold(held("cy", Rule::Delay), admin::now_ms())
        .await
        .unwrap()
        .record()
        .clone();

    assert!(go(&["guarded", "list"]));
    assert!(go(&["guarded", "list", "--all"]));
    assert!(!go(&["guarded", "approve", "nope"]), "an id is 16 hex digits");
    assert!(!go(&["guarded", "approve", "0123456789abcdef"]), "no such action");
    assert!(!go(&["guarded", "approve", &delay.id]), "a delay is not approved early");

    // The host approves: recorded for the server to carry out, under the host actor
    assert!(go(&["guarded", "approve", &approval.id]));
    let after = guards.get(&approval.id).await.unwrap();
    assert_eq!(
        (after.state, after.approved_by.as_deref()),
        (State::Approved, Some("host"))
    );
    assert!(!go(&["guarded", "approve", &approval.id]), "not twice");

    assert!(go(&["guarded", "cancel", &other.id]));
    let after = guards.get(&other.id).await.unwrap();
    assert_eq!(
        (after.state, after.canceled_by.as_deref()),
        (State::Canceled, Some("host"))
    );
    assert!(!go(&["guarded", "cancel", &other.id]));

    let log = keys.database().changes(0, 1000).await.unwrap();
    let actors: Vec<&str> = log
        .iter()
        .filter(|e| e.table_name == "guarded_actions" && e.op == "update")
        .map(|e| e.actor.as_str())
        .collect();
    assert_eq!(actors, ["host:admin guarded approve", "host:admin guarded cancel"]);
}
