//! The guarded actions on both engines (DD15, DH6). `REFORGED_TEST_POSTGRES_URL` names a server the test may create a
//! database on (`postgres://user:pw@127.0.0.1:5432/postgres`); without it only SQLite runs.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use admin::guard::{GuardError, GuardSettings, Guards, Held, Kind, New, Rule, State, Who};
use admin::keys::{Keys, Role};
use diesel::Connection;
use diesel::connection::SimpleConnection;
use serde_json::json;
use store::PgConnection;
use store::{DatabaseConfig, PostgresConfig, portable};

struct PgDatabase {
    server: PostgresConfig,
    config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut conn) = PgConnection::establish(&self.server.conninfo()) {
            let _ = conn.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                portable::quote_ident(&self.config.database)
            ));
        }
    }
}

fn new_postgres() -> Option<PgDatabase> {
    let url = url::Url::parse(&std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?).expect("a postgres:// URL");
    let server = PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    };
    let name = format!("admin_guard_test_{}", std::process::id());
    PgConnection::establish(&server.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    Some(PgDatabase { server, config })
}

const OWNER: &str = "owner-key-0123456789abcdef0123456789";
const MIN: i64 = 60_000;

async fn open(config: &DatabaseConfig, rule: Rule) -> Guards {
    let config = config.clone();
    let keys = tokio::task::spawn_blocking(move || Keys::open(&config, OWNER))
        .await
        .unwrap()
        .unwrap();
    let settings = GuardSettings {
        ban_guard: rule,
        revert_guard: rule,
        ..GuardSettings::default()
    };
    Guards::new(keys.database().clone(), settings)
}

fn ban(subject: &str, by: &str, role: Role, rule: Rule) -> New {
    New {
        kind: Kind::Ban,
        subject: subject.to_owned(),
        summary: format!("ban {subject}"),
        params: json!({"name": subject, "operator": "42"}),
        rule,
        initiated_by: by.to_owned(),
        initiator_role: role,
    }
}

async fn exercise(config: DatabaseConfig) {
    let guards = open(&config, Rule::Approval).await;
    let now = 1_000_000;

    // Held once: the same action asked again is the first one
    let Held::New(first) = guards
        .hold(ban("ada", "aaaa0001", Role::Moderator, Rule::Approval), now)
        .await
        .unwrap()
    else {
        panic!("a new action is held");
    };
    let Held::Already(again) = guards
        .hold(ban("ada", "aaaa0002", Role::Moderator, Rule::Approval), now + 1)
        .await
        .unwrap()
    else {
        panic!("the same action waits once");
    };
    assert_eq!(again.id, first.id);
    assert_eq!(
        (first.state, first.release_at, first.needs_role),
        (State::Pending, None, Role::Moderator)
    );

    // The key that asked cannot approve it, a lesser role cannot, another moderator can
    let me = Who::key("aaaa0001", Role::Moderator);
    assert!(matches!(
        guards.approve(&first.id, me, now).await,
        Err(GuardError::SameKey)
    ));
    let viewer = Who::key("bbbb0001", Role::Viewer);
    assert!(matches!(
        guards.approve(&first.id, viewer, now).await,
        Err(GuardError::RoleTooLow(Role::Moderator))
    ));
    assert!(
        guards.ready(now + MIN).await.unwrap().is_empty(),
        "not approved: not ready"
    );
    let approved = guards
        .approve(&first.id, Who::key("bbbb0002", Role::Moderator), now + 2)
        .await
        .unwrap();
    assert_eq!(
        (approved.state, approved.approved_by.as_deref()),
        (State::Approved, Some("bbbb0002"))
    );
    assert!(matches!(
        guards
            .approve(&first.id, Who::key("bbbb0003", Role::Owner), now + 3)
            .await,
        Err(GuardError::NotWaiting("approved"))
    ));

    // Claimed once, however many ask at the same moment
    assert_eq!(guards.ready(now + 4).await.unwrap(), std::slice::from_ref(&first.id));
    let mut claims = Vec::new();
    for _ in 0..8 {
        let (g, id) = (guards.clone(), first.id.clone());
        claims.push(tokio::spawn(
            async move { g.claim(&id, now + 4).await.unwrap().is_some() },
        ));
    }
    let mut won = 0;
    for claim in claims {
        won += usize::from(claim.await.unwrap());
    }
    assert_eq!(won, 1, "exactly one runs it");
    assert!(guards.ready(now + 5).await.unwrap().is_empty());
    assert!(matches!(
        guards.cancel(&first.id, Who::host("cancel"), now + 5).await,
        Err(GuardError::NotWaiting("running"))
    ));
    let running = guards.get(&first.id).await.unwrap();
    guards.finish(&running, Ok("banned ada".into()), now + 6).await.unwrap();
    let done = guards.get(&first.id).await.unwrap();
    assert_eq!(
        (done.state, done.result.as_deref(), done.ended_at),
        (State::Done, Some("banned ada"), Some(now + 6))
    );

    // A delay is never approved early; it is ready when it ends, and the host kit can cancel it
    let delayed = guards
        .hold(ban("bo", "aaaa0001", Role::Admin, Rule::Delay), now)
        .await
        .unwrap();
    let delayed = delayed.record().clone();
    assert_eq!(
        (delayed.release_at, delayed.needs_role),
        (Some(now + 10 * MIN), Role::Admin)
    );
    assert!(matches!(
        guards
            .approve(&delayed.id, Who::key("bbbb0002", Role::Owner), now)
            .await,
        Err(GuardError::DelayOnly)
    ));
    assert!(guards.ready(now + 10 * MIN - 1).await.unwrap().is_empty());
    assert_eq!(
        guards.ready(now + 10 * MIN).await.unwrap(),
        std::slice::from_ref(&delayed.id)
    );
    let host = guards.cancel(&delayed.id, Who::host("cancel"), now + 1).await.unwrap();
    assert_eq!(
        (host.state, host.canceled_by.as_deref()),
        (State::Canceled, Some("host"))
    );

    // Only the key that asked, or an owner, cancels; the rest are refused
    let third = guards
        .hold(ban("cy", "aaaa0001", Role::Moderator, Rule::Approval), now)
        .await
        .unwrap();
    let id = third.record().id.clone();
    assert!(matches!(
        guards.cancel(&id, Who::key("bbbb0002", Role::Admin), now).await,
        Err(GuardError::NotYours)
    ));
    guards
        .cancel(&id, Who::key("aaaa0001", Role::Moderator), now)
        .await
        .unwrap();

    // Unapproved ones expire; one left running by a stopped server is failed, never run again
    let waiting = guards
        .hold(ban("di", "aaaa0001", Role::Moderator, Rule::Approval), now)
        .await
        .unwrap();
    let wid = waiting.record().id.clone();
    assert!(guards.expire(now + 47 * 60 * MIN).await.unwrap().is_empty());
    let expired = guards.expire(now + 49 * 60 * MIN).await.unwrap();
    assert_eq!(
        expired.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        std::slice::from_ref(&wid)
    );
    assert_eq!(guards.get(&wid).await.unwrap().state, State::Expired);
    let stuck = guards
        .hold(ban("ed", "aaaa0001", Role::Moderator, Rule::Delay), now)
        .await
        .unwrap();
    let sid = stuck.record().id.clone();
    assert!(guards.claim(&sid, now + 10 * MIN).await.unwrap().is_some());
    assert_eq!(guards.recover(now + 11 * MIN).await.unwrap(), 1);
    assert_eq!(guards.get(&sid).await.unwrap().state, State::Failed);
    assert!(guards.claim(&sid, now + 12 * MIN).await.unwrap().is_none());

    // Waiting ones are listed oldest first, and everything newest first with `all`
    let w1 = guards
        .hold(ban("fay", "aaaa0001", Role::Moderator, Rule::Approval), now + 20)
        .await
        .unwrap();
    let w2 = guards
        .hold(ban("gus", "aaaa0001", Role::Moderator, Rule::Approval), now + 30)
        .await
        .unwrap();
    let open: Vec<String> = guards
        .list(false, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(open, [w1.record().id.clone(), w2.record().id.clone()]);
    let every = guards.list(true, 100).await.unwrap();
    assert_eq!(every.len(), 7);
    assert_eq!(every[0].id, w2.record().id);
}

#[tokio::test(flavor = "multi_thread")]
async fn guarded_actions_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    exercise(DatabaseConfig::Sqlite {
        file: dir.path().join("nested").join("admin.db"),
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn guarded_actions_on_postgres() {
    let Some(pg) = new_postgres() else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: skipping the Postgres half");
        return;
    };
    exercise(DatabaseConfig::Postgres(pg.config.clone())).await;
}

/// What waits is in the database, so a restart finds it as it was.
#[tokio::test(flavor = "multi_thread")]
async fn what_waits_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("admin.db"),
    };
    let id = {
        let guards = open(&config, Rule::Approval).await;
        let held = guards
            .hold(ban("ada", "aaaa0001", Role::Moderator, Rule::Approval), 5_000)
            .await
            .unwrap();
        held.record().id.clone()
    };
    let guards = open(&config, Rule::Approval).await;
    let waiting = guards.list(false, 100).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(
        (waiting[0].id.as_str(), waiting[0].state, waiting[0].created_at),
        (id.as_str(), State::Pending, 5_000)
    );
    assert_eq!(waiting[0].params["operator"], "42");
}
