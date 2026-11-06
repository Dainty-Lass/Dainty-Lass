//! History and revert for the host kit, the console, the launcher and the admin API (DD23, DH3b): the change
//! logs of this server's databases read and undone together.
//!
//! A server keeps several databases (the game's, the admin API's, chat's: [`sources`]). Each is opened with
//! `Database::attach`, as it is, so the tools need none of the owner's migrations; `store` does the work
//! (`revert.rs`, DH3a). What is added here is what a person needs on top of one database: one page of history
//! across them (newest first, one cursor), one plan and one id for a revert that spans them, and the words for
//! both (`history_text`).
//!
//! A revert is applied one database at a time, each atomically. If a later one fails, the earlier ones stay
//! reverted and the error names them ([`HistoryError::Partial`]); run the dry run again for the rest.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
pub use store::Actor;
use store::history::Verification;
pub use store::revert::RowConflict;
use store::revert::{Cursor, RevertOutcome, RevertPlan};
use store::{AccountColumn, Database, DatabaseConfig, DatabaseOptions, Filter};

use crate::hmac::hex;

/// The game's database: `dainty-lass.db`, or the Postgres database every service shares.
pub const NODE: &str = "node";
/// The admin API's keys and the record of what each key did (DH2).
pub const ADMIN: &str = "admin";
/// Chat's messages and mutes.
pub const CHAT: &str = "chat";

/// The most a page holds (`store::revert::MAX_PAGE`).
pub const MAX_PAGE: i64 = store::revert::MAX_PAGE;
pub const DEFAULT_PAGE: i64 = 50;

/// The change log's actor for a host kit command, the console or the launcher's server panel: `host:<name>`.
pub fn host_actor(name: &str) -> Actor {
    Actor::Host(name.to_owned())
}

/// The longest account, actor or time the tools take, and the longest cursor.
pub const MAX_WHO: usize = 64;
pub const MAX_TIME: usize = 40;
pub const MAX_CURSOR: usize = 128;

/// Which rows belong to an account, in the game's database. Isles' accounts table is the only player-facing
/// table so far (`modules/rustic-isles` migration 0001); the directory's tables are here for a node that keeps
/// them in its game database (DD15). A module adds its own with `store::declare_accounts("node", ..)` where it
/// opens the database.
pub const NODE_ACCOUNTS: &[AccountColumn] = &[
    AccountColumn {
        table: "accounts",
        column: "id",
    },
    AccountColumn {
        table: "network_directory_accounts",
        column: "account",
    },
    AccountColumn {
        table: "network_directory_log",
        column: "account",
    },
];

/// Chat's tables (`crates/chat/migrations`): a sender or an account is the global account id.
pub const CHAT_ACCOUNTS: &[AccountColumn] = &[
    AccountColumn {
        table: "chat_accounts",
        column: "account",
    },
    AccountColumn {
        table: "chat_mutes",
        column: "account",
    },
    AccountColumn {
        table: "chat_room_messages",
        column: "sender",
    },
    AccountColumn {
        table: "chat_direct_messages",
        column: "sender",
    },
    AccountColumn {
        table: "chat_direct_messages",
        column: "recipient",
    },
];

// ------------------------------------------------------------------ errors

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// The caller's input: said to them as it is.
    #[error("{0}")]
    Bad(String),
    /// No database to read: the server has not run yet.
    #[error("there is no database to read yet: start the server once, then try again")]
    NoDatabase,
    #[error("the history changed since the dry run: run it again")]
    PlanChanged,
    /// A revert that spans databases failed part of the way.
    #[error("{} reverted, then {failed} failed: {why}; run the dry run again for the rest", .done.join(" and "))]
    Partial {
        done: Vec<String>,
        failed: String,
        why: String,
    },
    #[error("{0}")]
    Store(store::Error),
}

impl From<store::Error> for HistoryError {
    fn from(error: store::Error) -> Self {
        match error {
            store::Error::PlanChanged => Self::PlanChanged,
            store::Error::Revert(why) => Self::Bad(why),
            other => Self::Store(other),
        }
    }
}

fn bad(text: impl Into<String>) -> HistoryError {
    HistoryError::Bad(text.into())
}

// ------------------------------------------------------------------ what a person types

/// A filter as people write it (`--actor`, `--since`): parsed by [`Selection::to_filter`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub account: Option<String>,
    pub actor: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

fn clean(name: &str, text: &str, max: usize) -> Result<String, HistoryError> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > max || text.chars().any(char::is_control) {
        return Err(bad(format!("{name} is 1 to {max} characters of text")));
    }
    Ok(text.to_owned())
}

/// An actor as the change log names it (`admin:ab12cd34`, `host:console`), or a short form: an admin key's id
/// (or `owner`) is `admin:<id>`, `console` is `host:console`.
pub fn parse_actor(text: &str) -> Result<Actor, HistoryError> {
    let text = clean("an actor", text, MAX_WHO)?;
    if let Some(actor) = Actor::parse(&text) {
        return Ok(actor);
    }
    let hex_digit = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    if (text.len() == 8 && text.bytes().all(hex_digit)) || text == "owner" {
        return Ok(Actor::Admin(text));
    }
    if text == "console" {
        return Ok(Actor::Host(text));
    }
    Err(bad(
        "name the actor as player:<account>, admin:<key id>, host:<command> or system:<task> \
         (admin key ids are in `admin key list`)",
    ))
}

const TIME_HELP: &str = "a time is RFC 3339 (2026-10-27T10:30:00Z), a date (2026-10-27), a date and time in UTC \
                         (2026-10-27T10:30), or how long ago: 90m, 6h, 2d, 1w";

/// How long ago `90m`, `6h`, `2d` or `1w` is, in milliseconds.
fn ago(text: &str) -> Option<i64> {
    let (digits, unit) = text.split_at_checked(text.len().checked_sub(1)?)?;
    let per: i64 = match unit {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return None,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()?.checked_mul(per)
}

/// A time as people write it, in unix milliseconds: RFC 3339 (`2026-10-27T10:30:00Z`), a date or a date and
/// time in UTC (`2026-10-27`, `2026-10-27T10:30`, `2026-10-27 10:30:15`), or how long ago (`90m`, `6h`, `2d`,
/// `1w`).
pub fn parse_time(text: &str, now_ms: i64) -> Result<i64, HistoryError> {
    let text = clean("a time", text, MAX_TIME)?;
    if let Some(before) = ago(&text) {
        return now_ms
            .checked_sub(before)
            .filter(|at| *at >= 0)
            .ok_or_else(|| bad(TIME_HELP));
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&text) {
        return Ok(t.timestamp_millis());
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(t) = chrono::NaiveDateTime::parse_from_str(&text, format) {
            return Ok(t.and_utc().timestamp_millis());
        }
    }
    if let Ok(day) = chrono::NaiveDate::parse_from_str(&text, "%Y-%m-%d")
        && let Some(t) = day.and_hms_opt(0, 0, 0)
    {
        return Ok(t.and_utc().timestamp_millis());
    }
    Err(bad(TIME_HELP))
}

impl Selection {
    pub fn to_filter(&self, now_ms: i64) -> Result<Filter, HistoryError> {
        let filter = Filter {
            account: self
                .account
                .as_deref()
                .map(|a| clean("an account", a, MAX_WHO))
                .transpose()?,
            actor: self.actor.as_deref().map(parse_actor).transpose()?,
            since: self.since.as_deref().map(|t| parse_time(t, now_ms)).transpose()?,
            until: self.until.as_deref().map(|t| parse_time(t, now_ms)).transpose()?,
        };
        if let (Some(since), Some(until)) = (filter.since, filter.until)
            && since >= until
        {
            return Err(bad("--since is not before --until"));
        }
        Ok(filter)
    }
}

// ------------------------------------------------------------------ the databases

/// One database of the server: its name, where it is, and the account columns its tables are declared with.
#[derive(Debug, Clone)]
pub struct Source {
    pub name: &'static str,
    pub config: DatabaseConfig,
    pub accounts: Vec<AccountColumn>,
}

impl Source {
    /// Whether there is anything to open: a SQLite file that is not there yet is a service that has not run.
    pub fn exists(&self) -> bool {
        match &self.config {
            DatabaseConfig::Sqlite { file } => file.is_file(),
            DatabaseConfig::Postgres(_) => true,
        }
    }
}

fn declared(name: &str, builtin: &[AccountColumn]) -> Vec<AccountColumn> {
    let mut all = builtin.to_vec();
    for column in store::declared_accounts(name) {
        if !all.contains(&column) {
            all.push(column);
        }
    }
    all
}

/// The databases of the server whose game database is `node`: on SQLite a file each, beside it
/// (`Keys::database_for`, chat's `History::beside`); on Postgres they are one database, read once.
pub fn sources(node: &DatabaseConfig) -> Vec<Source> {
    match node {
        DatabaseConfig::Sqlite { file } => {
            let beside = |name: &str| DatabaseConfig::Sqlite {
                file: file.with_file_name(name),
            };
            vec![
                Source {
                    name: NODE,
                    config: node.clone(),
                    accounts: declared(NODE, NODE_ACCOUNTS),
                },
                Source {
                    name: ADMIN,
                    config: beside(crate::keys::FILE_NAME),
                    accounts: declared(ADMIN, &[]),
                },
                Source {
                    name: CHAT,
                    config: beside("chat.db"),
                    accounts: declared(CHAT, CHAT_ACCOUNTS),
                },
            ]
        }
        DatabaseConfig::Postgres(_) => {
            let mut accounts = declared(NODE, NODE_ACCOUNTS);
            for column in declared(CHAT, CHAT_ACCOUNTS).into_iter().chain(declared(ADMIN, &[])) {
                if !accounts.contains(&column) {
                    accounts.push(column);
                }
            }
            vec![Source {
                name: NODE,
                config: node.clone(),
                accounts,
            }]
        }
    }
}

/// The game's database settings from `admin.env`, which `dainty-host setup` writes: the other databases are
/// beside it.
pub fn node_database(env_file: Option<&std::path::Path>) -> Result<DatabaseConfig, String> {
    let env = service_kit::env::Env::load(env_file).map_err(|e| e.to_string())?;
    Ok(crate::config::Config::from_env(&env)?.database)
}

// ------------------------------------------------------------------ one page across them

/// One entry of a log and the database it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub database: String,
    pub entry: store::ChangeEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Newest first.
    pub entries: Vec<Entry>,
    /// Pass it back to read the next, older page.
    pub next: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    At(Cursor),
    End,
}

/// `node:812,admin:40,chat:end`: where each database's part of the next page starts.
fn parse_cursor(text: &str) -> Result<BTreeMap<String, Position>, HistoryError> {
    let wrong = || bad("not a history cursor");
    if text.len() > MAX_CURSOR {
        return Err(wrong());
    }
    let mut out = BTreeMap::new();
    for part in text.split(',') {
        let (name, at) = part.split_once(':').ok_or_else(wrong)?;
        let position = if at == "end" {
            Position::End
        } else {
            Position::At(at.parse::<Cursor>().map_err(|_| wrong())?)
        };
        let named = !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase());
        if !named || out.insert(name.to_owned(), position).is_some() {
            return Err(wrong());
        }
    }
    Ok(out)
}

/// A cursor as [`HistorySet::query`] would read it, written back in its one order; `None` for text that is
/// not a cursor. The fuzz target checks that writing it back changes nothing more.
pub fn normalize_cursor(text: &str) -> Option<String> {
    parse_cursor(text).ok().map(|positions| cursor_text(&positions))
}

fn cursor_text(positions: &BTreeMap<String, Position>) -> String {
    positions
        .iter()
        .map(|(name, position)| match position {
            Position::At(cursor) => format!("{name}:{cursor}"),
            Position::End => format!("{name}:end"),
        })
        .collect::<Vec<_>>()
        .join(",")
}

// ------------------------------------------------------------------ a revert across them

/// One database's part of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub database: String,
    pub plan: RevertPlan,
}

/// What a revert would do in every database the selection reaches, with one id: [`HistorySet::apply`] runs it
/// only while it is still this plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub selection: Filter,
    pub parts: Vec<Part>,
}

impl Plan {
    fn new(selection: Filter, parts: Vec<Part>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"dainty-lass revert plan v1\n");
        for part in &parts {
            hasher.update(format!("{} {}\n", part.database, part.plan.id));
        }
        Self {
            id: hex(&hasher.finalize()),
            selection,
            parts,
        }
    }

    /// Rows that would change.
    pub fn actions(&self) -> usize {
        self.parts.iter().map(|p| p.plan.actions.len()).sum()
    }

    pub fn conflicts(&self) -> usize {
        self.parts.iter().map(|p| p.plan.conflicts.len()).sum()
    }

    pub fn unchanged(&self) -> usize {
        self.parts.iter().map(|p| p.plan.unchanged.len()).sum()
    }

    /// Log entries the selection holds.
    pub fn entries(&self) -> usize {
        self.parts.iter().map(|p| p.plan.entries).sum()
    }
}

/// What [`HistorySet::apply`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub plan: Plan,
    /// The databases that changed, with the revert's write time.
    pub done: Vec<(String, RevertOutcome)>,
}

/// One database's chain check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub database: String,
    pub verification: Verification,
}

// ------------------------------------------------------------------ the set

/// The server's databases, opened for history and revert. Cloning shares the pools.
#[derive(Clone)]
pub struct HistorySet {
    dbs: Arc<Vec<(String, Database)>>,
}

impl HistorySet {
    /// Opens every database of the server that exists. Blocking work runs on the blocking pool.
    pub async fn open(node: &DatabaseConfig) -> Result<Self, HistoryError> {
        let wanted: Vec<Source> = sources(node).into_iter().filter(Source::exists).collect();
        if wanted.is_empty() {
            return Err(HistoryError::NoDatabase);
        }
        let dbs = tokio::task::spawn_blocking(move || {
            wanted
                .into_iter()
                .map(|source| {
                    Database::attach(&source.config, &DatabaseOptions::default(), &source.accounts)
                        .map(|db| (source.name.to_owned(), db))
                        .map_err(HistoryError::Store)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        .map_err(|_| HistoryError::Store(store::Error::Panicked))??;
        Ok(Self { dbs: Arc::new(dbs) })
    }

    /// Databases already open, such as a service's own.
    pub fn from_databases(dbs: Vec<(String, Database)>) -> Self {
        Self { dbs: Arc::new(dbs) }
    }

    pub fn names(&self) -> Vec<&str> {
        self.dbs.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// Whether this set holds every database that exists now: false once a service has made its file since.
    pub fn is_current(&self, node: &DatabaseConfig) -> bool {
        let existing: Vec<&str> = sources(node).iter().filter(|s| s.exists()).map(|s| s.name).collect();
        existing == self.names()
    }

    /// One page of history, newest first, across the databases. `cursor` is a previous page's `next`.
    pub async fn query(&self, filter: &Filter, cursor: Option<&str>, limit: i64) -> Result<Page, HistoryError> {
        let limit = limit.clamp(1, MAX_PAGE);
        let incoming = cursor.map(parse_cursor).transpose()?.unwrap_or_default();
        let mut found: Vec<Entry> = Vec::new();
        // Per database: how many entries it gave, and whether its page was full (there may be older ones)
        let mut fetched: Vec<(&str, usize, bool)> = Vec::new();
        let mut positions: BTreeMap<String, Position> = BTreeMap::new();
        for (name, db) in self.dbs.iter() {
            if filter.account.is_some() && db.account_columns().is_empty() {
                continue;
            }
            let from = match incoming.get(name) {
                Some(Position::End) => {
                    positions.insert(name.clone(), Position::End);
                    continue;
                }
                Some(Position::At(cursor)) => Some(*cursor),
                None => None,
            };
            let page = db.history(filter.clone(), from, limit).await?;
            fetched.push((name, page.entries.len(), page.next.is_some()));
            found.extend(page.entries.into_iter().map(|entry| Entry {
                database: name.clone(),
                entry,
            }));
        }
        found.sort_by(|a, b| (b.entry.at, b.entry.id, &b.database).cmp(&(a.entry.at, a.entry.id, &a.database)));
        found.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        for (name, count, full) in fetched {
            let oldest = found.iter().filter(|e| e.database == name).map(|e| e.entry.id).min();
            let taken = found.iter().filter(|e| e.database == name).count();
            let position = match oldest {
                _ if taken == count && !full => Some(Position::End),
                Some(id) => id.to_string().parse::<Cursor>().ok().map(Position::At),
                None => incoming.get(name).copied(),
            };
            if let Some(position) = position {
                positions.insert(name.to_owned(), position);
            }
        }
        let more = positions.values().any(|p| *p != Position::End);
        Ok(Page {
            entries: found,
            next: more.then(|| cursor_text(&positions)),
        })
    }

    /// What reverting `selection` would do, without writing.
    pub async fn dry_run(&self, selection: &Filter) -> Result<Plan, HistoryError> {
        if selection.actor.is_none() && selection.account.is_none() && selection.since.is_none() {
            return Err(bad("say what to revert: --actor, --account or --since"));
        }
        let mut parts = Vec::new();
        for (name, db) in self.dbs.iter() {
            if selection.account.is_some() && db.account_columns().is_empty() {
                continue;
            }
            let plan = db.revert_dry_run(selection.clone()).await?;
            parts.push(Part {
                database: name.clone(),
                plan,
            });
        }
        if parts.is_empty() {
            return Err(bad(
                "no table of this server is declared with accounts, so --account finds nothing",
            ));
        }
        Ok(Plan::new(selection.clone(), parts))
    }

    /// Reverts `selection` as `actor` (the revert is logged under it, and can be reverted in turn), only while
    /// the plan it makes now has the id the dry run showed: what is applied is what was shown.
    pub async fn apply(&self, actor: &Actor, selection: &Filter, plan_id: &str) -> Result<Applied, HistoryError> {
        let fresh = self.dry_run(selection).await?;
        if fresh.id != plan_id {
            return Err(HistoryError::PlanChanged);
        }
        let mut done: Vec<(String, RevertOutcome)> = Vec::new();
        for part in &fresh.parts {
            let Some((_, db)) = self.dbs.iter().find(|(name, _)| *name == part.database) else {
                continue;
            };
            if part.plan.actions.is_empty() {
                continue;
            }
            match db.revert_apply(actor.clone(), part.plan.clone()).await {
                Ok(outcome) => done.push((part.database.clone(), outcome)),
                Err(error) if done.is_empty() => return Err(error.into()),
                Err(error) => {
                    return Err(HistoryError::Partial {
                        done: done.into_iter().map(|(name, _)| name).collect(),
                        failed: part.database.clone(),
                        why: error.to_string(),
                    });
                }
            }
        }
        Ok(Applied { plan: fresh, done })
    }

    /// `history verify`: each log walked from its start (DH5).
    pub async fn verify(&self) -> Result<Vec<Verified>, HistoryError> {
        let mut out = Vec::new();
        for (name, db) in self.dbs.iter() {
            out.push(Verified {
                database: name.clone(),
                verification: db.verify(None).await?,
            });
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ between two calls

/// How long a dry run's plan is kept for `revertApply`, and how many.
pub const PLAN_KEPT: Duration = Duration::from_secs(10 * 60);
const PLAN_MAX: usize = 16;

/// The plans the admin API made, for the apply call that follows: short-lived, newest kept. A plan that is
/// gone is made again from the same selection, and applied only if its id is the same.
#[derive(Default)]
pub struct PlanCache {
    slots: Mutex<Vec<(Instant, Plan)>>,
}

impl PlanCache {
    pub fn put(&self, plan: Plan) {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.retain(|(at, p)| at.elapsed() < PLAN_KEPT && p.id != plan.id);
        slots.push((Instant::now(), plan));
        while slots.len() > PLAN_MAX {
            slots.remove(0);
        }
    }

    pub fn get(&self, id: &str) -> Option<Plan> {
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots
            .iter()
            .find(|(at, p)| p.id == id && at.elapsed() < PLAN_KEPT)
            .map(|(_, plan)| plan.clone())
    }

    pub fn forget(&self, id: &str) {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(_, p)| p.id != id);
    }
}

/// The admin API's view of the databases: opened on first use, and again when a service has made its file
/// since, with the plans kept between the two calls.
pub struct Service {
    node: DatabaseConfig,
    set: tokio::sync::Mutex<Option<HistorySet>>,
    pub plans: PlanCache,
}

impl Service {
    pub fn new(node: DatabaseConfig) -> Self {
        Self {
            node,
            set: tokio::sync::Mutex::new(None),
            plans: PlanCache::default(),
        }
    }

    pub async fn set(&self) -> Result<HistorySet, HistoryError> {
        let mut held = self.set.lock().await;
        if let Some(set) = held.as_ref().filter(|set| set.is_current(&self.node)) {
            return Ok(set.clone());
        }
        let set = HistorySet::open(&self.node).await?;
        *held = Some(set.clone());
        Ok(set)
    }
}

/// The most rows of each kind a plan or a result lists to a caller: a mass delete is thousands, and the
/// count says how many there were.
pub const MAX_LISTED: usize = 500;

/// A kind of compensating change by name: `insert`, `update`, `delete` or `soft_delete`.
pub fn kind_name(kind: store::revert::RevertKind) -> &'static str {
    use store::revert::RevertKind;
    match kind {
        RevertKind::Insert => "insert",
        RevertKind::Update => "update",
        RevertKind::Delete => "delete",
        RevertKind::SoftDelete => "soft_delete",
    }
}

/// Why a row stays as it is, as a caller is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFields {
    /// `changed_since`, `not_as_logged`, `key_changed` or `schema_changed`.
    pub reason: &'static str,
    /// `changed_since`: the first newer entry, who made it and when.
    pub entry: Option<String>,
    pub by: Option<String>,
    pub at: Option<String>,
    pub detail: Option<String>,
}

pub fn conflict_fields(conflict: &RowConflict) -> ConflictFields {
    use store::revert::Conflict;
    let none = |reason| ConflictFields {
        reason,
        entry: None,
        by: None,
        at: None,
        detail: None,
    };
    match &conflict.conflict {
        Conflict::ChangedSince { entry, actor, at } => ConflictFields {
            entry: Some(entry.to_string()),
            by: Some(actor.clone()),
            at: Some(crate::input::iso(*at)),
            ..none("changed_since")
        },
        Conflict::NotAsLogged => none("not_as_logged"),
        Conflict::KeyChanged => none("key_changed"),
        Conflict::SchemaChanged { detail } => ConflictFields {
            detail: Some(detail.clone()),
            ..none("schema_changed")
        },
    }
}

// ------------------------------------------------------------------ what a reader may see

/// Key hashes are never shown, though they can't be turned back into a key: a row of `admin_keys` goes out
/// without its `key_hash`.
pub fn redact(table: &str, row: &mut serde_json::Value) {
    if table == "admin_keys"
        && let Some(object) = row.as_object_mut()
        && object.contains_key("key_hash")
    {
        object.insert("key_hash".into(), "<hidden>".into());
    }
}

/// A log row's JSON text with [`redact`] applied.
pub fn redacted_text(table: &str, text: Option<&str>) -> Option<String> {
    let text = text?;
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(mut row) => {
            redact(table, &mut row);
            Some(row.to_string())
        }
        Err(_) => Some(String::from("<not JSON>")),
    }
}

/// A row of a plan as JSON text with [`redact`] applied.
pub fn redacted_json(table: &str, row: Option<&serde_json::Value>) -> Option<String> {
    row.map(|row| {
        let mut row = row.clone();
        redact(table, &mut row);
        row.to_string()
    })
}

/// A selection as a held revert keeps it (DH6): the filter's four fields, times in milliseconds.
pub fn selection_json(filter: &Filter) -> serde_json::Value {
    serde_json::json!({
        "account": filter.account,
        "actor": filter.actor.as_ref().map(Actor::as_text),
        "since": filter.since,
        "until": filter.until,
    })
}

/// [`selection_json`] read back; `None` for anything the log could not have written.
pub fn selection_from(value: &serde_json::Value) -> Option<Filter> {
    let text = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
    let time = |name: &str| value.get(name).and_then(serde_json::Value::as_i64);
    Some(Filter {
        account: text("account").map(str::to_owned),
        actor: text("actor").map(|a| Actor::parse(a).ok_or(())).transpose().ok()?,
        since: time("since"),
        until: time("until"),
    })
}
