//! History across a server's databases (DD23, DH3b): one cursor over several logs, the words people type.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use dainty_admin::history::{self, HistorySet, Selection, parse_actor, parse_time};
use dainty_admin::keys::{Keys, Role};
use dainty_store::{Actor, Database, DatabaseConfig, DatabaseOptions, Filter};
use diesel::connection::SimpleConnection;
use diesel::{Connection, SqliteConnection};

const NOW: i64 = 1_800_000_000_000;

/// Isles' `accounts` table in `dainty-lass.db`, and the admin API's `admin.db` beside it.
async fn node(dir: &std::path::Path) -> (DatabaseConfig, Database, Keys) {
    let file = dir.join("dainty-lass.db");
    SqliteConnection::establish(&file.to_string_lossy())
        .unwrap()
        .batch_execute(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, user_id TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
             notes BIGINT NOT NULL DEFAULT 0, deleted_at BIGINT)",
        )
        .unwrap();
    let config = DatabaseConfig::Sqlite { file };
    let game = Database::attach(&config, &DatabaseOptions::default(), &[]).unwrap();
    let keys = Keys::open(&Keys::database_for(&config), "owner-key-0123456789abcdef0123456789").unwrap();
    (config, game, keys)
}

#[tokio::test(flavor = "multi_thread")]
async fn one_cursor_pages_every_log_newest_first_without_a_gap_or_a_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let (config, game, keys) = node(dir.path()).await;
    let host = Actor::Host("test".into());
    for round in 0..4 {
        game.write(Actor::System("isles".into()), move |tx| {
            for n in 0..3 {
                tx.conn().batch_execute(&format!(
                    "INSERT INTO accounts (id, user_id, name) VALUES ('a{round}{n}', 'u{round}{n}', 'P{round}{n}')"
                ))?;
            }
            Ok::<_, dainty_store::Error>(())
        })
        .await
        .unwrap();
        keys.issue(&format!("Person {round}"), Role::Viewer, host.clone(), NOW)
            .await
            .unwrap();
    }

    let set = HistorySet::open(&config).await.unwrap();
    assert_eq!(set.names(), ["node", "admin"], "chat has not run: its file is not made");
    let all = set.query(&Filter::default(), None, 1000).await.unwrap();
    assert!(all.next.is_none());
    assert!(all.entries.len() >= 12 + 4);
    let order: Vec<(i64, i64)> = all.entries.iter().map(|e| (e.entry.at, e.entry.id)).collect();
    assert!(order.windows(2).all(|w| w[0].0 >= w[1].0), "newest first");

    let mut paged = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let page = set.query(&Filter::default(), cursor.as_deref(), 5).await.unwrap();
        paged.extend(page.entries.iter().map(|e| (e.database.clone(), e.entry.id)));
        pages += 1;
        assert!(page.entries.len() <= 5);
        match page.next {
            Some(next) => {
                assert!(next.len() <= history::MAX_CURSOR);
                cursor = Some(next);
            }
            None => break,
        }
        assert!(pages < 50, "paging ends");
    }
    let wanted: Vec<(String, i64)> = all.entries.iter().map(|e| (e.database.clone(), e.entry.id)).collect();
    assert_eq!(paged, wanted);

    // A cursor is read strictly
    for bad in [
        "",
        "node",
        "node:0",
        "node:x",
        "NODE:5",
        "node:5,node:6",
        "node:5;admin:4",
        &"a:1,".repeat(40),
    ] {
        assert!(set.query(&Filter::default(), Some(bad), 5).await.is_err(), "{bad}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn by_account_only_the_databases_with_account_columns_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (config, game, _keys) = node(dir.path()).await;
    game.write(Actor::System("isles".into()), |tx| {
        tx.conn()
            .batch_execute("INSERT INTO accounts (id, user_id, name) VALUES ('a1', 'u1', 'Ada')")?;
        Ok::<_, dainty_store::Error>(())
    })
    .await
    .unwrap();
    let set = HistorySet::open(&config).await.unwrap();
    let ada = Filter {
        account: Some("a1".into()),
        ..Filter::default()
    };
    let page = set.query(&ada, None, 50).await.unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].database, "node");
    // The admin API's keys have no account: a revert by account reaches the game's database only
    let plan = set.dry_run(&ada).await.unwrap();
    assert_eq!(
        plan.parts.iter().map(|p| p.database.as_str()).collect::<Vec<_>>(),
        ["node"]
    );

    // A server whose files are not there yet has no history to read
    let empty = tempfile::tempdir().unwrap();
    let none = DatabaseConfig::Sqlite {
        file: empty.path().join("dainty-lass.db"),
    };
    assert!(matches!(
        HistorySet::open(&none).await,
        Err(history::HistoryError::NoDatabase)
    ));
}

#[test]
fn people_type_actors_and_times_in_the_forms_the_help_gives() {
    assert_eq!(parse_actor("admin:ab12cd34").unwrap(), Actor::Admin("ab12cd34".into()));
    assert_eq!(parse_actor("ab12cd34").unwrap(), Actor::Admin("ab12cd34".into()));
    assert_eq!(parse_actor("owner").unwrap(), Actor::Admin("owner".into()));
    assert_eq!(parse_actor("console").unwrap(), Actor::Host("console".into()));
    assert_eq!(
        parse_actor("host:admin key issue").unwrap(),
        Actor::Host("admin key issue".into())
    );
    for bad in ["", "partner", "AB12CD34", "kind:", "admin:\u{7}x", &"x".repeat(65)] {
        assert!(parse_actor(bad).is_err(), "{bad:?}");
    }

    let day = 86_400_000;
    assert_eq!(parse_time("2026-10-27T10:30:00Z", NOW).unwrap(), 1_793_097_000_000);
    assert_eq!(
        parse_time("2026-10-27T10:30:00+02:00", NOW).unwrap(),
        1_793_097_000_000 - 7_200_000
    );
    assert_eq!(parse_time("2026-10-27 10:30", NOW).unwrap(), 1_793_097_000_000);
    assert_eq!(
        parse_time("2026-10-27", NOW).unwrap(),
        1_793_097_000_000 - 10 * 3_600_000 - 1_800_000
    );
    assert_eq!(parse_time("90m", NOW).unwrap(), NOW - 90 * 60_000);
    assert_eq!(parse_time("2d", NOW).unwrap(), NOW - 2 * day);
    assert_eq!(parse_time("1w", NOW).unwrap(), NOW - 7 * day);
    for bad in [
        "",
        "d",
        "-5m",
        "5y",
        "99999999999999999m",
        "yesterday",
        "2026-13-40",
        "10:30",
    ] {
        assert!(parse_time(bad, NOW).is_err(), "{bad:?}");
    }

    let backwards = Selection {
        since: Some("2026-10-28".into()),
        until: Some("2026-10-27".into()),
        ..Selection::default()
    };
    assert!(backwards.to_filter(NOW).unwrap_err().to_string().contains("not before"));
}

#[test]
fn a_cursor_written_back_reads_the_same() {
    let normal = history::normalize_cursor("node:812,admin:end,chat:7").unwrap();
    assert_eq!(normal, "admin:end,chat:7,node:812");
    assert_eq!(history::normalize_cursor(&normal), Some(normal));
    assert_eq!(history::normalize_cursor("node:0"), None);
    assert_eq!(history::normalize_cursor("node:1,node:2"), None);
}
