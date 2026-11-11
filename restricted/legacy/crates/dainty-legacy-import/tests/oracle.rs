//! The oracle for the MIT `dainty-legacy` (spec-legacy-import.md section 13): the clean-room rewrite and this importer
//! run on every history 1.4.4 left, the 31 Drizzle prefixes and the Diesel baseline, and must agree on the report and
//! on every row they write. Its data file must also hold the real migrations' hashes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use dainty_legacy_import::revived::drsave::{PortableIdentity, write_save};
use dainty_legacy_import::revived::schema::drizzle_migrations;
use dainty_legacy_import::revived::{self, Options};
use dainty_legacy_import::target::Target;
use dainty_legacy_import::testing::{KEY, diesel_database, drizzle_database, isles_database};
use dainty_store::{Connection, DatabaseConfig};
use diesel::Connection as _;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::Text;
use serde_json::Value;
use sha2::{Digest, Sha256};

const FROZEN: usize = 31;
const DAY_MS: i64 = 86_400_000;

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_data_file_holds_the_real_history() {
    let data = dainty_legacy::data::revived().unwrap();
    let frozen = drizzle_migrations();
    assert_eq!(data.drizzle.len(), frozen.len());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("sql/drizzle");
    for (fact, real) in data.drizzle.iter().zip(&frozen) {
        assert_eq!((fact.tag.as_str(), fact.when), (real.tag.as_str(), real.created_at));
        let lf = std::fs::read_to_string(dir.join(format!("{}.sql", fact.tag)))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(fact.sha256_lf, sha256(lf.as_bytes()), "{}", fact.tag);
        assert_eq!(
            fact.sha256_crlf,
            sha256(lf.replace('\n', "\r\n").as_bytes()),
            "{}",
            fact.tag
        );
        assert_eq!(fact.sha256_lf, dainty_store::drizzle::hash(&real.sql), "{}", fact.tag);
    }
    let baseline: Vec<String> = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/revived"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy()[..4].to_owned())
        .collect();
    assert_eq!(data.diesel.versions, baseline);
}

fn has(conn: &mut Connection, table: &str, column: &str) -> bool {
    #[derive(QueryableByName)]
    struct Name {
        #[diesel(sql_type = Text)]
        name: String,
    }
    diesel::sql_query(format!("SELECT name FROM pragma_table_info('{table}')"))
        .load::<Name>(conn)
        .unwrap()
        .iter()
        .any(|n| n.name == column)
}

/// Players, bans, links, keys and invites, each where the history so far has the table and column for it. No account
/// without a userId: this importer carries one and the MIT one skips it, a departure on purpose.
fn seed(conn: &mut Connection, now_ms: i64) {
    let admin = has(conn, "users", "isAdmin");
    let players = [
        ("UID-sam", "Sam", true),
        ("UID-ban", "Banned", false),
        ("UID-temp", "Timeout", true),
        ("UID-old", "Forgiven", false),
        ("UID-off", "Pardoned", false),
        ("UID-twin", "sam", false),
    ];
    for (id, name, is_admin) in players {
        let sql = if admin {
            format!("INSERT INTO users (userId, name, notes, isAdmin) VALUES ('{id}', '{name}', 3, {is_admin})")
        } else {
            format!("INSERT INTO users (userId, name, notes) VALUES ('{id}', '{name}', 3)")
        };
        conn.batch_execute(&sql).unwrap();
    }
    for (n, id) in ["UID-sam", "UID-sam", "UID-off"].iter().enumerate() {
        conn.batch_execute(&format!(
            "INSERT INTO characters (characterId, userId, createdDate, lastModifiedDate, name, updateVersion, data) \
             VALUES ('{id}-c{n}', '{id}', 'x', 'x', 'Slayer', 1, '{{}}')"
        ))
        .unwrap();
    }
    if has(conn, "invitecodes", "invitecode") {
        conn.batch_execute(
            "INSERT INTO invitecodes (invitecode, usesRemaining, infiniteUses) VALUES \
             ('WELCOME', 5, false), ('FOREVER', 0, true), ('USED-UP', 0, false)",
        )
        .unwrap();
    }
    if has(conn, "userapikeys", "keyHash") {
        conn.batch_execute(&format!(
            "INSERT INTO userapikeys (userId, keyHash) VALUES ('UID-sam', '{}')",
            sha256(b"RRK_sam")
        ))
        .unwrap();
    }
    if has(conn, "discordlinks", "discordId") {
        conn.batch_execute(
            "INSERT INTO discordlinks (discordId, userId, linkedAt) VALUES \
             ('100000000000000001', 'UID-sam', '2026-10-01T12:00:00.000Z')",
        )
        .unwrap();
    }
    if has(conn, "account_bans", "account_id") {
        let expires = has(conn, "account_bans", "expires_at");
        for (id, active, until) in [
            ("UID-ban", 1, None),
            ("UID-temp", 1, Some(now_ms + 30 * DAY_MS)),
            ("UID-old", 1, Some(now_ms - DAY_MS)),
            ("UID-off", 0, None),
        ] {
            let sql = match (expires, until) {
                (true, Some(at)) => format!(
                    "INSERT INTO account_bans (account_id, reason, active, updated_at, actor, expires_at) \
                     VALUES ('{id}', 'griefing', {active}, 1, 'admin', {at})"
                ),
                _ => format!(
                    "INSERT INTO account_bans (account_id, reason, active, updated_at, actor) \
                     VALUES ('{id}', 'griefing', {active}, 1, 'admin')"
                ),
            };
            conn.batch_execute(&sql).unwrap();
        }
    }
}

/// Every row of every table, as JSON, with values the clock made masked: the two imports run moments apart.
fn dump(file: &Path, now_ms: i64) -> Value {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        j: String,
    }
    #[derive(QueryableByName)]
    struct Name {
        #[diesel(sql_type = Text)]
        name: String,
    }
    let mut conn = Connection::establish(&file.to_string_lossy()).unwrap();
    let tables = diesel::sql_query("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .load::<Name>(&mut conn)
        .unwrap();
    let mut out = serde_json::Map::new();
    // The log's ids and the sequence behind them count writes, not rows: see below
    for table in tables.iter().map(|t| &t.name).filter(|t| *t != "sqlite_sequence") {
        let columns = diesel::sql_query(format!("SELECT name FROM pragma_table_info('{table}')"))
            .load::<Name>(&mut conn)
            .unwrap();
        let pairs: Vec<String> = columns.iter().map(|c| format!("'{0}', \"{0}\"", c.name)).collect();
        let order = if table == "__store_change_log" {
            " ORDER BY id"
        } else {
            ""
        };
        let rows = diesel::sql_query(format!(
            "SELECT json_object({}) AS j FROM \"{table}\"{order}",
            pairs.join(", ")
        ))
        .load::<Row>(&mut conn)
        .unwrap()
        .into_iter()
        .map(|r| mask(serde_json::from_str(&r.j).unwrap(), now_ms));
        let value = if table == "__store_change_log" {
            change_log(rows, now_ms)
        } else {
            let mut rows: Vec<Value> = rows.collect();
            rows.sort_by_key(Value::to_string);
            Value::Array(rows)
        };
        out.insert(table.clone(), value);
    }
    Value::Object(out)
}

/// The change log as each logged row's last image, by actor. This importer inserts a timed-out account and then
/// updates it, the MIT one inserts it timed out: two entries against one, for the same row.
fn change_log(rows: impl Iterator<Item = Value>, now_ms: i64) -> Value {
    let mut last = serde_json::Map::new();
    for row in rows {
        let field = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let image = match row.get("after") {
            Some(Value::String(text)) => mask(serde_json::from_str(text).unwrap(), now_ms),
            _ => Value::Null,
        };
        let key = format!("{} {} {}", field("actor"), field("table_name"), field("row_key"));
        last.insert(key, image);
    }
    Value::Object(last)
}

fn mask(value: Value, now_ms: i64) -> Value {
    let near = |ms: i64| (ms - now_ms).abs() < 600_000;
    match value {
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, mask(v, now_ms))).collect()),
        Value::Number(n) if n.as_i64().is_some_and(near) => Value::from("<now>"),
        Value::String(s)
            if chrono::DateTime::parse_from_rfc3339(&s).is_ok_and(|t| near(t.timestamp_millis()))
                || chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                    .is_ok_and(|t| near(t.and_utc().timestamp_millis())) =>
        {
            Value::from("<now>")
        }
        v => v,
    }
}

fn report(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

/// Runs this importer on `isles` as `dainty-legacy import <kind>` does.
async fn agpl(revived: bool, source: &Path, isles: &Path, dry_run: bool) -> Result<Value, String> {
    let config = DatabaseConfig::Sqlite {
        file: isles.to_path_buf(),
    };
    let target = Target::open(&config, "local".parse().unwrap()).map_err(|e| e.to_string())?;
    let options = Options::now(dry_run);
    let result = if revived {
        revived::import_server(source, &target, options).await
    } else {
        revived::import_offline_save(source, &target, options).await
    };
    result.map(report).map_err(|e| e.to_string())
}

/// Runs the MIT importer the same way.
async fn mit(revived: bool, source: &Path, isles: &Path, dry_run: bool) -> Result<Value, String> {
    let args = dainty_legacy::ImportArgs {
        kind: if revived {
            dainty_legacy::Kind::Revived
        } else {
            dainty_legacy::Kind::OfflineSave
        },
        source: source.to_path_buf(),
        target: Some(dainty_legacy::Target::To(isles.to_path_buf())),
        node: None,
        dry_run,
        fresh_start: false,
        db_tool: None,
    };
    dainty_legacy::run(args, "local")
        .await
        .map(report)
        .map_err(|e| e.to_string())
}

/// Each importer on its own copy of `source` and of an empty 1.14.7 database: a dry run, an import, then the same
/// import again. The reports and the rows must match at each step, and the source must be left as it was.
async fn agree(dir: &Path, name: &str, revived: bool, source: &Path, now_ms: i64) {
    let before = std::fs::read(source).unwrap();
    let mut runs = Vec::new();
    for side in ["agpl", "mit"] {
        let src = dir.join(format!("{name}-{side}-src.{}", if revived { "db" } else { "drsave" }));
        std::fs::copy(source, &src).unwrap();
        let isles = dir.join(format!("{name}-{side}-isles.db"));
        isles_database(&isles);
        let mut steps = Vec::new();
        for dry_run in [true, false, false] {
            let out = if side == "agpl" {
                agpl(revived, &src, &isles, dry_run).await
            } else {
                mit(revived, &src, &isles, dry_run).await
            };
            steps.push((out, dump(&isles, now_ms)));
        }
        assert_eq!(
            std::fs::read(&src).unwrap(),
            before,
            "{name}: {side} changed its source"
        );
        runs.push(steps);
    }
    let (agpl, mit) = (&runs[0], &runs[1]);
    for (step, ((a_out, a_rows), (m_out, m_rows))) in agpl.iter().zip(mit).enumerate() {
        match (a_out, m_out) {
            (Ok(a), Ok(m)) => assert_eq!(a, m, "{name}, step {step}: the reports differ"),
            // This importer opens the target (and so puts store's change log on it) before it reads the source; the
            // MIT one reads first, so a refusal leaves the target as it was
            (Err(_), Err(_)) => continue,
            _ => panic!("{name}, step {step}: one importer refused\nagpl: {a_out:?}\nmit: {m_out:?}"),
        }
        let diff = differences(a_rows, m_rows, "");
        assert!(
            diff.is_empty(),
            "{name}, step {step}: the 1.14.7 rows differ:\n{}",
            diff.join("\n")
        );
    }
}

/// Where two dumps differ, by path, the AGPL value first.
fn differences(a: &Value, m: &Value, path: &str) -> Vec<String> {
    match (a, m) {
        (Value::Object(a), Value::Object(m)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(m.keys()).collect();
            keys.into_iter()
                .flat_map(|k| {
                    let (x, y) = (a.get(k).unwrap_or(&Value::Null), m.get(k).unwrap_or(&Value::Null));
                    differences(x, y, &format!("{path}.{k}"))
                })
                .collect()
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .flat_map(|(i, (x, y))| differences(x, y, &format!("{path}[{i}]")))
            .collect(),
        (Value::Array(x), Value::Array(y)) => {
            let only = |p: &[Value], q: &[Value], side: &str| -> Vec<String> {
                p.iter()
                    .filter(|v| !q.contains(v))
                    .map(|v| format!("{path}: only {side}: {v}"))
                    .collect()
            };
            [only(x, y, "agpl"), only(y, x, "mit")].concat()
        }
        _ if a == m => Vec::new(),
        _ => vec![format!("{path}: {a} | {m}")],
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[tokio::test(flavor = "multi_thread")]
async fn both_importers_agree_on_every_server_history() {
    let dir = tempfile::tempdir().unwrap();
    let now = now_ms();
    for prefix in 1..=FROZEN {
        let source = dir.path().join(format!("drizzle-{prefix}.db"));
        seed(&mut drizzle_database(&source, prefix), now);
        agree(dir.path(), &format!("drizzle-{prefix}"), true, &source, now).await;
    }
    let source = dir.path().join("diesel.db");
    seed(&mut diesel_database(&source), now);
    agree(dir.path(), "diesel", true, &source, now).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn both_importers_agree_on_offline_saves() {
    let dir = tempfile::tempdir().unwrap();
    let now = now_ms();
    let database = dir.path().join("offline.db");
    diesel_database(&database)
        .batch_execute(&format!(
            "INSERT INTO users (userId, name, notes, isAdmin) VALUES ('UID-sam', 'Sam', 3, true);
             INSERT INTO userapikeys (userId, keyHash) VALUES ('UID-sam', '{}');
             INSERT INTO discordlinks (discordId, userId, linkedAt)
                 VALUES ('100000000000000001', 'UID-sam', '2026-10-01T12:00:00.000Z');
             INSERT INTO characters (characterId, userId, createdDate, lastModifiedDate, name, updateVersion, data)
                 VALUES ('UID-sam-c0', 'UID-sam', 'x', 'x', 'Slayer', 1, '{{}}');",
            sha256(KEY.as_bytes())
        ))
        .unwrap();
    agree(dir.path(), "bare", false, &database, now).await;

    // A container each side wrote, read by both
    let when = "2026-10-21T22:00:00.000Z";
    let agpl_save = dir.path().join("agpl.drsave");
    let identity = PortableIdentity {
        user_id: "UID-sam".into(),
        username: "Sam".into(),
        key: KEY.into(),
    };
    write_save(&agpl_save, &database, &identity, when).unwrap();
    agree(dir.path(), "agpl-save", false, &agpl_save, now).await;
    let mit_save: PathBuf = dir.path().join("mit.drsave");
    let bytes = std::fs::read(&database).unwrap();
    std::fs::write(
        &mit_save,
        dainty_legacy::drsave::build(&bytes, "UID-sam", "Sam", KEY, when),
    )
    .unwrap();
    agree(dir.path(), "mit-save", false, &mit_save, now).await;

    // Someone else's key: both refuse
    let foreign = dir.path().join("foreign.drsave");
    let other = concat!("UUK_", "fedcba9876543210", "fedcba9876543210", "fedcba9876543210");
    std::fs::write(
        &foreign,
        dainty_legacy::drsave::build(&bytes, "UID-sam", "Sam", other, when),
    )
    .unwrap();
    agree(dir.path(), "foreign-save", false, &foreign, now).await;
}

#[test]
fn both_convert_host_json_alike() {
    let data = dainty_legacy::data::reforged().unwrap();
    let cases = [
        r#"{"version": 1, "mode": "private", "edition": "both", "host": "127.0.0.1", "gameAddress": "127.0.0.1",
            "name": "Old Kit", "legacyGameDir": "C:/Games/Dauntless-1.4.4",
            "mongo": {"uri": "mongodb://kit:hunter2@127.0.0.1:27017", "manage": true}}"#,
        r#"{"version": 1, "mode": "public", "edition": "legacy", "name": "Only 1.4.4", "legacyPublicPort": 4000}"#,
        r#"{"version": 1, "name": "Odd", "somethingNew": true, "modules": {"external": [
            {"id": "revived-1-4-4", "path": "x"}, {"id": "keep", "path": "C:/m/keep.exe", "sha256": "ab"}]}}"#,
        r#"{"version": 1, "name": "Quote \" and \\ and ünïcode", "network": {"nodes": [{"id": "eu-1"}]}}"#,
        r#"{"version": 1, "name": null}"#,
        r#"[1, 2]"#,
        r#"not json"#,
    ];
    for text in cases {
        let old = dainty_legacy_import::reforged::hostjson::convert(text);
        let new = dainty_legacy::reforged::hostjson::convert(text, data);
        match (old, new) {
            (Ok(mut o), Ok(mut n)) => {
                // The same header and values; a string may be quoted either way
                let comments = |t: &str| {
                    t.lines()
                        .filter(|l| l.starts_with('#'))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                assert_eq!(comments(&o.toml), comments(&n.toml), "{text}");
                let parse = |t: &str| t.parse::<toml::Table>().unwrap();
                assert_eq!(parse(&o.toml), parse(&n.toml), "{text}");
                // The spec doesn't order what was dropped, and the two list it differently
                o.dropped.sort();
                n.dropped.sort();
                assert_eq!(o.dropped, n.dropped, "{text}");
                assert_eq!(o.was_legacy_only, n.legacy_only, "{text}");
            }
            (Err(_), Err(_)) => {}
            (o, n) => panic!("{text}: one converter refused\nagpl: {o:?}\nmit: {n:?}"),
        }
    }
}
