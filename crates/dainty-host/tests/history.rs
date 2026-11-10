#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

//! `dainty-host history` and `revert` (DD23, DH3b): a partner's mass delete in the game's database is
//! reverted by one command, and a row changed since stays as it is and is reported.

use clap::Parser as _;
use dainty_host::cli::{Cli, Command};
use dainty_host::commands::history::{Done, Job, Select, Surface, parse_console, run};
use dainty_host::paths::Paths;
use dainty_store::{Actor, Database, DatabaseConfig, DatabaseOptions, KeyValue};
use diesel::connection::SimpleConnection;
use diesel::{Connection, SqliteConnection};

const KEY: &str = "0123456789abcdef0123456789abcdef";

/// A workspace whose admin.env points at a game database with isles' `accounts` table, written the way the
/// game module writes it (through `Database::write`) and holding five players.
struct Server {
    _dir: tempfile::TempDir,
    paths: Paths,
    db: Database,
}

fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    std::fs::create_dir_all(&paths.config).unwrap();
    let file = paths.config.join("dainty-lass.db");
    SqliteConnection::establish(&file.to_string_lossy())
        .unwrap()
        .batch_execute(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, user_id TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
             notes BIGINT NOT NULL DEFAULT 0, deleted_at BIGINT)",
        )
        .unwrap();
    let slashed = |p: &std::path::Path| format!("\"{}\"", p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
    let env = format!(
        "DAINTY_ADMIN_API_KEY={KEY}
DAINTY_ADMIN_KEY={KEY}
RUSTIC_ISLES_MANAGER_OPERATOR_SECRET={KEY}
         DAINTY_ADMIN_API_TIMEOUTS_FILE={}
DAINTY_DB_ENGINE=sqlite
DAINTY_DB_FILE={}
",
        slashed(&paths.config.join("timeouts.json")),
        slashed(&file)
    );
    std::fs::write(paths.config.join("admin.env"), env).unwrap();
    let db = Database::attach(&DatabaseConfig::Sqlite { file }, &DatabaseOptions::default(), &[]).unwrap();
    Server { _dir: dir, paths, db }
}

async fn seed(server: &Server) {
    server
        .db
        .write(Actor::System("isles".into()), |tx| {
            tx.conn().batch_execute(
                "INSERT INTO accounts (id, user_id, name) VALUES ('a1', 'u1', 'Ada'), ('a2', 'u2', 'Bo'),
                 ('a3', 'u3', 'Cy'), ('a4', 'u4', 'Di'), ('a5', 'u5', 'Ed')",
            )?;
            Ok::<_, dainty_store::Error>(())
        })
        .await
        .unwrap();
}

async fn live(server: &Server) -> Vec<String> {
    server
        .db
        .read(|conn| {
            use diesel::RunQueryDsl;
            use diesel::sql_types::Text;
            #[derive(diesel::QueryableByName)]
            struct Id {
                #[diesel(sql_type = Text)]
                id: String,
            }
            Ok::<_, dainty_store::Error>(
                diesel::sql_query("SELECT id FROM accounts WHERE deleted_at IS NULL ORDER BY id")
                    .load::<Id>(conn)?
                    .into_iter()
                    .map(|r| r.id)
                    .collect(),
            )
        })
        .await
        .unwrap()
}

fn selecting(actor: &str) -> Select {
    Select {
        actor: Some(actor.into()),
        ..Select::default()
    }
}

async fn command(server: &Server, job: Job) -> Result<Done, String> {
    run(&server.paths, &job, Surface::Cli).await
}

fn plan_id(done: &Done) -> String {
    done.lines
        .iter()
        .find_map(|l| l.strip_prefix("Revert plan "))
        .expect("the plan's id")
        .to_owned()
}

// DH3's Verify: a partner's mass delete is reverted in one command; a row changed since stays as it is.
#[tokio::test(flavor = "multi_thread")]
async fn a_partners_mass_delete_is_reverted_in_one_command_and_a_row_changed_since_stays() {
    let server = server();
    seed(&server).await;
    let partner = Actor::Admin("partner".into());
    server
        .db
        .write(partner, |tx| {
            for id in ["a1", "a2", "a3"] {
                tx.soft_delete("accounts", &[("id", KeyValue::Text(id.into()))])?;
            }
            tx.conn().batch_execute("DELETE FROM accounts WHERE id = 'a4'")?;
            Ok::<_, dainty_store::Error>(())
        })
        .await
        .unwrap();
    // The owner has since renamed Bo's account: not the partner's to undo
    server
        .db
        .write(Actor::Admin("owner".into()), |tx| {
            tx.conn()
                .batch_execute("UPDATE accounts SET name = 'Bo, kept' WHERE id = 'a2'")?;
            Ok::<_, dainty_store::Error>(())
        })
        .await
        .unwrap();
    assert_eq!(live(&server).await, ["a5"]);

    let dry = command(
        &server,
        Job::Revert {
            select: selecting("admin:partner"),
            apply: None,
        },
    )
    .await
    .unwrap();
    let text = dry.lines.join("\n");
    assert!(text.contains("Would change 3 rows"), "{text}");
    assert!(text.contains("Stays as it is (1 rows)"), "{text}");
    assert!(text.contains("changed since by admin:owner"), "{text}");
    assert!(text.contains(r#"{"id":"a2"}"#), "{text}");
    assert_eq!(live(&server).await, ["a5"], "a dry run writes nothing");
    let id = plan_id(&dry);

    // Only what was shown: another selection's id is not this plan
    let wrong = Job::Revert {
        select: selecting("admin:owner"),
        apply: Some(id.clone()),
    };
    assert!(command(&server, wrong).await.unwrap_err().contains("run it again"));
    assert_eq!(live(&server).await, ["a5"]);

    let apply = Job::Revert {
        select: selecting("admin:partner"),
        apply: Some(id.clone()),
    };
    let done = command(&server, apply.clone()).await.unwrap();
    let text = done.lines.join("\n");
    assert!(text.contains("Reverted 3 rows as host:console"), "{text}");
    assert!(text.contains("1 rows stayed as they are"), "{text}");
    assert_eq!(
        live(&server).await,
        ["a1", "a3", "a4", "a5"],
        "Bo's account stays deleted"
    );

    // The plan is spent: the same id no longer matches what a dry run shows
    assert!(command(&server, apply).await.unwrap_err().contains("run it again"));

    // The revert is in the history under the console, and reverts in turn
    let page = Job::Page {
        select: selecting("host:console"),
        limit: "50".into(),
        cursor: None,
    };
    let shown = command(&server, page).await.unwrap().lines.join("\n");
    assert!(shown.contains("host:console") && shown.contains("accounts"), "{shown}");
    let undo = command(
        &server,
        Job::Revert {
            select: selecting("host:console"),
            apply: None,
        },
    )
    .await
    .unwrap();
    let id = plan_id(&undo);
    command(
        &server,
        Job::Revert {
            select: selecting("host:console"),
            apply: Some(id),
        },
    )
    .await
    .unwrap();
    assert_eq!(live(&server).await, ["a5"]);

    let chain = command(&server, Job::Verify).await.unwrap();
    assert!(chain.ok && chain.lines[0].contains("OK"), "{:?}", chain.lines);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revert_needs_a_selection_and_a_server_that_has_run() {
    let server = server();
    let nothing = Job::Revert {
        select: Select::default(),
        apply: None,
    };
    assert!(
        command(&server, nothing)
            .await
            .unwrap_err()
            .contains("say what to revert")
    );
    let bad_time = Select {
        since: Some("yesterday-ish".into()),
        ..Select::default()
    };
    let err = command(
        &server,
        Job::Revert {
            select: bad_time,
            apply: None,
        },
    )
    .await
    .unwrap_err();
    assert!(err.contains("a time is RFC 3339"), "{err}");
    let too_many = Job::Page {
        select: Select::default(),
        limit: "5000".into(),
        cursor: None,
    };
    assert!(command(&server, too_many).await.unwrap_err().contains("1 to 1000"));

    // A workspace setup has not been run in
    let empty = tempfile::tempdir().unwrap();
    let err = run(&Paths::new(empty.path()), &Job::Verify, Surface::Cli)
        .await
        .unwrap_err();
    assert!(err.contains("run dainty-host setup"), "{err}");
}

#[test]
fn the_command_line_and_the_console_say_the_same_thing() {
    let parse = |args: &[&str]| Cli::try_parse_from(std::iter::once("dainty-host").chain(args.iter().copied()));
    let Command::History(h) = parse(&["history", "--actor", "admin:ab12cd34", "--limit", "10"])
        .unwrap()
        .command
    else {
        panic!("history")
    };
    assert_eq!(h.filter.actor.as_deref(), Some("admin:ab12cd34"));
    assert_eq!((h.limit.as_str(), h.command.is_none()), ("10", true));
    let Command::History(h) = parse(&["history", "verify"]).unwrap().command else {
        panic!("history")
    };
    assert!(h.command.is_some());
    let Command::Revert(r) = parse(&["revert", "--actor", "owner", "--apply", "abc"])
        .unwrap()
        .command
    else {
        panic!("revert")
    };
    assert_eq!(r.apply.as_deref(), Some("abc"));
    assert!(parse(&["revert", "--actor", "x", "--dry-run", "--apply", "abc"]).is_err());
    assert!(parse(&["history", "--actor", "x", "verify"]).is_err());

    assert_eq!(
        parse_console("revert", &["--actor=admin:partner", "--apply", "abc"]).unwrap(),
        Job::Revert {
            select: selecting("admin:partner"),
            apply: Some("abc".into())
        }
    );
    assert_eq!(parse_console("history", &["verify"]).unwrap(), Job::Verify);
    for (word, args) in [
        ("history", &["--apply", "x"][..]),
        ("revert", &["--limit", "5"]),
        ("revert", &["--dry-run", "--apply", "x"]),
        ("history", &["--actor"]),
        ("history", &["now"]),
    ] {
        assert!(parse_console(word, args).is_err(), "{word} {args:?}");
    }
}
