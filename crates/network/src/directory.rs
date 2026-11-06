//! The directory (DD17, DN4): which node is each account's home, and which account holds each username,
//! unique network-wide.
//!
//! One node is the leader and writes; every change is an entry of an append-only log it numbers from 1.
//! Every other node is a replica: it pulls the entries after the last one it holds, so a replica cut off
//! by a partition catches up by pulling again. A home registers and renames its own accounts through the
//! leader, so two regions can't claim one name. A server outside a network is its own leader with no
//! peers (section 6): the same tables and code, nothing sent.
//!
//! Both the log and what it adds up to are tables in `store` ([`SCHEMA`]), written through
//! [`Database::write`] so the change log (DD23) holds every directory write too.
//!
//! **Failover** ([`DirectoryConfig::failover`], `docs/distributed-mode.md`): the leader is whoever
//! [`crate::election`] elected, followed through [`Directory::follow`]. Every entry carries the term it was
//! written in, a new leader first writes a [`DirectoryChange::Lead`] entry, and a write is acknowledged only
//! once a majority of the nodes holds it (Raft's commit rule), so no acknowledged change is lost when the leader
//! goes down. A node that comes back holding entries the new leader doesn't have drops them and pulls again:
//! they were never acknowledged.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use diesel::prelude::*;
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use module_api::BoxFuture;
use module_api::network::{GlobalId, IdKind, NodeId};
use store::{Actor, AnyConnection, Database, DatabaseSchema, Tx};

use crate::election::{Leadership, LogPosition, LogSource};
use crate::link::{LinkError, NodeService, PeerLink, Service};
use crate::messages::{
    ClashedAccount, DIRECTORY_VERSION, DecodeError, DirectoryCall, DirectoryChange, DirectoryEntry, DirectoryReply,
    DirectoryRequest, DirectoryResponse, JoinOutcome, JoiningAccount, MAX_PULL, Refusal, decode_request,
    decode_response, encode,
};
use crate::username::Username;

diesel::table! {
    /// The leader's append-only log of directory changes.
    network_directory_log (seq) {
        /// Entry number, from 1, assigned by the leader.
        seq -> BigInt,
        /// Unix milliseconds.
        at -> BigInt,
        /// `register`, `rename` or their `_clash` forms.
        op -> Text,
        /// The account's global id.
        account -> Text,
        /// The username the entry gives it.
        username -> Text,
        /// The account's home node, for a registration; the leader, for a `lead` entry.
        home -> Nullable<Text>,
        /// The election term the entry was written in; 0 outside failover.
        term -> BigInt,
    }
}

diesel::table! {
    /// What the log adds up to: one row per account.
    network_directory_accounts (account) {
        /// The account's global id.
        account -> Text,
        /// Its home node.
        home -> Text,
        /// Its username as written.
        username -> Text,
        /// The username's case-folded key, unique.
        username_key -> Text,
        /// The log entry that last changed the row.
        seq -> BigInt,
    }
}

use network_directory_accounts as accounts;
use network_directory_log as log;

diesel::table! {
    /// The game's `accounts` table, read when a server joins a network (DN13).
    #[sql_name = "accounts"]
    host_accounts (id) {
        /// The game's account id.
        id -> Text,
        /// The account's user id.
        user_id -> Text,
        /// The in-game name.
        name -> Text,
        /// The game's notes counter.
        notes -> BigInt,
        /// Unix milliseconds; set when deleted.
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    /// The launcher's `launcher_accounts` table, read when a server joins a network (DN13).
    #[sql_name = "launcher_accounts"]
    host_launcher_accounts (account_id) {
        /// The global account id.
        account_id -> Text,
        /// The display name, normalised for comparison.
        display_name_normalized -> Text,
        /// The display name.
        display_name -> Text,
        /// When the account was created, as the launcher wrote it.
        created_at -> Text,
        /// 0 while the account still has its placeholder name.
        username_set -> Nullable<BigInt>,
        /// Unix milliseconds; set when deleted.
        deleted_at -> Nullable<BigInt>,
    }
}

/// The directory's SQLite migrations.
pub const SQLITE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
/// The directory's Postgres migrations.
pub const POSTGRES_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// The directory's tables on a database of their own. A node that keeps them in its game database
/// (DD15) runs these migrations with its own instead.
pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE_MIGRATIONS,
    postgres: || POSTGRES_MIGRATIONS,
    soft_delete: &[],
};

/// Who applies the leader's entries on a replica, in the change log.
const REPLICATION: &str = "directory-replication";

/// Where this node stands in the directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// Not in a network: the directory is this server's alone
    Standalone,
    /// Holds the log and accepts writes.
    Leader,
    /// Pulls the leader's log.
    Replica {
        /// The node it pulls from.
        leader: NodeId,
    },
    /// Under failover, while no leader is known: reads go on, writes wait for the election
    Electing,
}

/// How a directory under failover reaches agreement (`docs/distributed-mode.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failover {
    /// Every other node of the network: the voters a write must reach a majority of.
    pub peers: Vec<NodeId>,
    /// How long a write waits for a majority before it answers [`Refusal::NoQuorum`].
    pub commit_timeout: Duration,
}

impl Failover {
    /// Nodes that must hold an entry before it is acknowledged, this one included.
    pub fn quorum(&self) -> usize {
        self.peers.len().div_ceil(2) + 1
    }
}

/// How a [`Directory`] is set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryConfig {
    /// This node.
    pub node: NodeId,
    /// The network's leader; none outside a network
    pub leader: Option<NodeId>,
    /// Entries a replica asks for per pull
    pub page: u32,
    /// The leader is elected rather than configured; none for a fixed leader
    pub failover: Option<Failover>,
}

impl DirectoryConfig {
    /// Outside a network: this node is its own leader.
    pub fn standalone(node: NodeId) -> Self {
        Self {
            node,
            leader: None,
            page: MAX_PULL,
            failover: None,
        }
    }

    /// In a network led by `leader`; the leader's own node is passed as both.
    pub fn in_network(node: NodeId, leader: NodeId) -> Self {
        Self {
            node,
            leader: Some(leader),
            page: MAX_PULL,
            failover: None,
        }
    }

    /// In a network whose leader is elected among `node` and `peers`; [`Directory::follow`] says who it is.
    /// A write waits up to 5 s for a majority.
    pub fn failover(node: NodeId, peers: Vec<NodeId>) -> Self {
        Self {
            node,
            leader: None,
            page: MAX_PULL,
            failover: Some(Failover {
                peers,
                commit_timeout: Duration::from_secs(5),
            }),
        }
    }

    /// Entries per pull, clamped to 1..=[`MAX_PULL`].
    pub fn with_page(mut self, page: u32) -> Self {
        self.page = page.clamp(1, MAX_PULL);
        self
    }

    fn role(&self) -> Role {
        if self.failover.is_some() {
            return Role::Electing;
        }
        match &self.leader {
            None => Role::Standalone,
            Some(leader) if *leader == self.node => Role::Leader,
            Some(leader) => Role::Replica { leader: leader.clone() },
        }
    }
}

/// What the directory knows of one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    /// The account's global id.
    pub account: GlobalId,
    /// Its username.
    pub username: Username,
    /// Its home node.
    pub home: NodeId,
    /// The entry that last changed it
    pub seq: u64,
}

/// Why a directory call failed.
#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    /// The leader refused the change, such as a name already taken.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// Registrations and renames wait for the leader rather than risk a name claimed twice
    #[error("the directory's leader can't be reached: {0}")]
    LeaderUnreachable(LinkError),
    /// The leader answered something that does not fit the call.
    #[error("the leader's answer makes no sense: {0}")]
    Protocol(String),
    /// This node's log isn't a prefix of the leader's (a leader restored from an older backup, say):
    /// replication stops until an operator resets one of them
    #[error("this node's directory has diverged from the leader's: {0}")]
    Diverged(String),
    /// Under failover, no node leads just now: an election is running
    #[error("an election is running: no node leads the directory yet")]
    NoLeader,
    /// The database failed.
    #[error(transparent)]
    Store(#[from] store::Error),
}

impl From<diesel::result::Error> for DirectoryError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Store(store::Error::Query(error))
    }
}

fn corrupt(what: impl std::fmt::Display) -> DirectoryError {
    DirectoryError::Store(store::Error::Schema(format!("the directory's tables hold {what}")))
}

fn to_seq(value: i64) -> Result<u64, DirectoryError> {
    u64::try_from(value).map_err(|_| corrupt(format!("the entry number {value}")))
}

fn from_seq(value: u64) -> Result<i64, DirectoryError> {
    i64::try_from(value).map_err(|_| DirectoryError::Protocol(format!("the entry number {value}")))
}

// ------------------------------------------------------------------ the tables

type LogRow = (i64, i64, String, String, String, Option<String>, i64);
type AccountRow = (String, String, String, i64);

const REGISTER: &str = "register";
const REGISTER_CLASH: &str = "register_clash";
const RENAME: &str = "rename";
const RENAME_CLASH: &str = "rename_clash";
const LEAD: &str = "lead";

fn entry_from_row((seq, at, op, account, username, home, term): LogRow) -> Result<DirectoryEntry, DirectoryError> {
    let term = to_seq(term)?;
    if op == LEAD {
        let node = home.ok_or_else(|| corrupt("a lead entry without its leader"))?;
        return Ok(DirectoryEntry {
            seq: to_seq(seq)?,
            at,
            term,
            change: DirectoryChange::Lead {
                node: NodeId::new(node).map_err(corrupt)?,
            },
        });
    }
    let account: GlobalId = account
        .parse()
        .map_err(|_| corrupt(format!("the account id {account:?}")))?;
    let username = Username::new(username).map_err(corrupt)?;
    let change = match (op.as_str(), home) {
        (REGISTER, Some(home)) => DirectoryChange::Register {
            account,
            username,
            home: NodeId::new(home).map_err(corrupt)?,
        },
        (REGISTER_CLASH, Some(home)) => DirectoryChange::RegisterClash {
            account,
            username,
            home: NodeId::new(home).map_err(corrupt)?,
        },
        (RENAME, None) => DirectoryChange::Rename { account, username },
        (RENAME_CLASH, None) => DirectoryChange::RenameClash { account, username },
        _ => return Err(corrupt(format!("an entry of kind {op:?}"))),
    };
    Ok(DirectoryEntry {
        seq: to_seq(seq)?,
        at,
        term,
        change,
    })
}

fn record_from_row((account, home, username, seq): AccountRow) -> Result<AccountRecord, DirectoryError> {
    Ok(AccountRecord {
        account: account
            .parse()
            .map_err(|_| corrupt(format!("the account id {account:?}")))?,
        username: Username::new(username).map_err(corrupt)?,
        home: NodeId::new(home).map_err(corrupt)?,
        seq: to_seq(seq)?,
    })
}

const LOG_COLUMNS: (
    log::seq,
    log::at,
    log::op,
    log::account,
    log::username,
    log::home,
    log::term,
) = (
    log::seq,
    log::at,
    log::op,
    log::account,
    log::username,
    log::home,
    log::term,
);
const ACCOUNT_COLUMNS: (accounts::account, accounts::home, accounts::username, accounts::seq) =
    (accounts::account, accounts::home, accounts::username, accounts::seq);

fn head(conn: &mut AnyConnection) -> Result<u64, DirectoryError> {
    let newest: Option<i64> = log::table
        .select(log::seq)
        .order(log::seq.desc())
        .first(conn)
        .optional()?;
    to_seq(newest.unwrap_or(0))
}

fn entry_at(conn: &mut AnyConnection, seq: u64) -> Result<Option<DirectoryEntry>, DirectoryError> {
    let row: Option<LogRow> = log::table
        .select(LOG_COLUMNS)
        .filter(log::seq.eq(from_seq(seq)?))
        .first(conn)
        .optional()?;
    row.map(entry_from_row).transpose()
}

fn entries_after(conn: &mut AnyConnection, after: u64, limit: u32) -> Result<Vec<DirectoryEntry>, DirectoryError> {
    let rows: Vec<LogRow> = log::table
        .select(LOG_COLUMNS)
        .filter(log::seq.gt(from_seq(after)?))
        .order(log::seq.asc())
        .limit(i64::from(limit))
        .load(conn)?;
    rows.into_iter().map(entry_from_row).collect()
}

fn account_record(conn: &mut AnyConnection, account: &GlobalId) -> Result<Option<AccountRecord>, DirectoryError> {
    let row: Option<AccountRow> = accounts::table
        .select(ACCOUNT_COLUMNS)
        .filter(accounts::account.eq(account.to_string()))
        .first(conn)
        .optional()?;
    row.map(record_from_row).transpose()
}

fn holder(conn: &mut AnyConnection, username: &Username) -> Result<Option<AccountRecord>, DirectoryError> {
    let row: Option<AccountRow> = accounts::table
        .select(ACCOUNT_COLUMNS)
        .filter(accounts::username_key.eq(username.key()))
        .first(conn)
        .optional()?;
    row.map(record_from_row).transpose()
}

/// Whether `change` can follow what the tables hold: the leader's rules, which a replica checks again on
/// every entry it applies.
fn check_change(conn: &mut AnyConnection, change: &DirectoryChange) -> Result<(), DirectoryError> {
    let (Some(account), Some(username)) = (change.account(), change.username()) else {
        return Ok(());
    };
    if account.kind() != IdKind::Account {
        return Err(Refusal::NotAnAccount.into());
    }
    let existing = account_record(conn, account)?;
    match change {
        DirectoryChange::Register { home, .. } => {
            if home != account.owner() && account.owner().as_str() != "local" {
                return Err(Refusal::NotHome.into());
            }
            if existing.is_some() {
                return Err(Refusal::AccountExists.into());
            }
        }
        DirectoryChange::RegisterClash { home, .. } => {
            if home != account.owner() && account.owner().as_str() != "local" {
                return Err(Refusal::NotHome.into());
            }
            if existing.is_some() {
                return Err(Refusal::AccountExists.into());
            }
        }
        DirectoryChange::Rename { .. } | DirectoryChange::RenameClash { .. } => {
            if existing.is_none() {
                return Err(Refusal::UnknownAccount.into());
            }
        }
        DirectoryChange::Lead { .. } => {}
    }
    match holder(conn, username)? {
        Some(other) if other.account != *account => Err(Refusal::UsernameTaken.into()),
        _ => Ok(()),
    }
}

/// Writes `entry` to the log and to what it adds up to. Checked already.
fn apply(tx: &mut Tx<'_>, entry: &DirectoryEntry) -> Result<(), DirectoryError> {
    let (op, account, username, home) = match &entry.change {
        DirectoryChange::Register {
            account,
            username,
            home,
        } => (REGISTER, account.to_string(), username.as_str(), Some(home)),
        DirectoryChange::RegisterClash {
            account,
            username,
            home,
        } => (REGISTER_CLASH, account.to_string(), username.as_str(), Some(home)),
        DirectoryChange::Rename { account, username } => (RENAME, account.to_string(), username.as_str(), None),
        DirectoryChange::RenameClash { account, username } => {
            (RENAME_CLASH, account.to_string(), username.as_str(), None)
        }
        DirectoryChange::Lead { node } => (LEAD, String::new(), "", Some(node)),
    };
    let term = i64::try_from(entry.term).map_err(|_| DirectoryError::Protocol(format!("the term {}", entry.term)))?;
    diesel::insert_into(log::table)
        .values((
            log::seq.eq(from_seq(entry.seq)?),
            log::at.eq(entry.at),
            log::op.eq(op),
            log::account.eq(account),
            log::username.eq(username),
            log::home.eq(home.map(NodeId::as_str)),
            log::term.eq(term),
        ))
        .execute(tx.conn())?;
    apply_account(tx.conn(), entry)
}

/// What `entry` does to the accounts table.
fn apply_account(conn: &mut AnyConnection, entry: &DirectoryEntry) -> Result<(), DirectoryError> {
    let seq = from_seq(entry.seq)?;
    let (account, username, home) = match &entry.change {
        DirectoryChange::Register {
            account,
            username,
            home,
        }
        | DirectoryChange::RegisterClash {
            account,
            username,
            home,
        } => (account, username, Some(home)),
        DirectoryChange::Rename { account, username } | DirectoryChange::RenameClash { account, username } => {
            (account, username, None)
        }
        DirectoryChange::Lead { .. } => return Ok(()),
    };
    match home {
        Some(home) => {
            diesel::insert_into(accounts::table)
                .values((
                    accounts::account.eq(account.to_string()),
                    accounts::home.eq(home.as_str()),
                    accounts::username.eq(username.as_str()),
                    accounts::username_key.eq(username.key()),
                    accounts::seq.eq(seq),
                ))
                .execute(conn)?;
        }
        None => {
            let changed = diesel::update(accounts::table.find(account.to_string()))
                .set((
                    accounts::username.eq(username.as_str()),
                    accounts::username_key.eq(username.key()),
                    accounts::seq.eq(seq),
                ))
                .execute(conn)?;
            if changed != 1 {
                return Err(Refusal::UnknownAccount.into());
            }
        }
    }
    Ok(())
}

/// Drops every entry after `keep` and rebuilds the accounts table from what is left (failover: entries a new
/// leader doesn't hold were never acknowledged). Returns how many entries went.
fn truncate_after(tx: &mut Tx<'_>, keep: u64) -> Result<usize, DirectoryError> {
    let dropped = diesel::delete(log::table.filter(log::seq.gt(from_seq(keep)?))).execute(tx.conn())?;
    diesel::delete(accounts::table).execute(tx.conn())?;
    let mut after = 0;
    loop {
        let page = entries_after(tx.conn(), after, MAX_PULL)?;
        let Some(last) = page.last() else {
            break;
        };
        after = last.seq;
        for entry in &page {
            apply_account(tx.conn(), entry)?;
        }
    }
    Ok(dropped)
}

/// The newest entry's term and number: what an election compares.
fn position(conn: &mut AnyConnection) -> Result<LogPosition, DirectoryError> {
    let index = head(conn)?;
    let term = if index == 0 {
        0
    } else {
        entry_at(conn, index)?.map_or(0, |entry| entry.term)
    };
    Ok(LogPosition { term, index })
}

/// When `account` was first registered in the log, or its ULID timestamp if no register row is present.
fn registered_at(conn: &mut AnyConnection, account: &GlobalId) -> Result<i64, DirectoryError> {
    let found: Option<i64> = log::table
        .select(log::at)
        .filter(log::account.eq(account.to_string()))
        .filter(log::op.eq(REGISTER).or(log::op.eq(REGISTER_CLASH)))
        .order(log::seq.asc())
        .first(conn)
        .optional()?;
    Ok(found.unwrap_or_else(|| i64::try_from(account.ulid().timestamp_ms()).unwrap_or(0)))
}

/// The first free suffixed username (`Fox_2`, `Fox_3`, ..., shortened to 16 characters) for `base` (DN13).
fn next_free_suffix(conn: &mut AnyConnection, base: &Username) -> Result<Username, DirectoryError> {
    for suffix in 2..=999_999 {
        let candidate = base.with_suffix(suffix);
        if holder(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(Refusal::UsernameTaken.into())
}

/// Joins a batch of existing accounts for `from` into the directory (DN13).
///
/// Accounts are applied in registration-time order. On a username clash, the account registered first keeps
/// the name; every other account with that normalized name gets a temporary suffixed name (`Fox_2`, shortened
/// to fit 16 characters) and is marked as clashed (`RegisterClash` / `RenameClash`) so its next login prompts
/// for a free rename.
fn join_batch(
    tx: &mut Tx<'_>,
    from: &NodeId,
    items: Vec<JoiningAccount>,
    term: u64,
) -> Result<JoinOutcome, DirectoryError> {
    let mut validated = Vec::with_capacity(items.len());
    for item in items {
        if item.account.kind() != IdKind::Account {
            return Err(Refusal::NotAnAccount.into());
        }
        if item.account.owner() != from && item.account.owner().as_str() != "local" {
            return Err(Refusal::NotHome.into());
        }
        let username = Username::new(&item.username).map_err(|e| Refusal::InvalidUsername { check: e.0 })?;
        validated.push((item.account, username, item.registered_at));
    }
    validated.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.to_string().cmp(&b.0.to_string())));

    let mut outcome = JoinOutcome::default();
    for (account, desired, reg_at) in validated {
        if let Some(existing) = account_record(tx.conn(), &account)? {
            if existing.home == *from {
                continue;
            }
            return Err(Refusal::AccountExists.into());
        }
        match holder(tx.conn(), &desired)? {
            None => {
                let entry = DirectoryEntry {
                    seq: head(tx.conn())? + 1,
                    at: reg_at,
                    term,
                    change: DirectoryChange::Register {
                        account,
                        username: desired,
                        home: from.clone(),
                    },
                };
                apply(tx, &entry)?;
                outcome.entries.push(entry);
            }
            Some(other) => {
                let other_at = registered_at(tx.conn(), &other.account)?;
                let joining_is_earlier = (reg_at, account.to_string()) < (other_at, other.account.to_string());
                if joining_is_earlier {
                    // The joining account registered first: displace the later holder to a suffixed name first
                    let fallback = next_free_suffix(tx.conn(), &other.username)?;
                    let rename_entry = DirectoryEntry {
                        seq: head(tx.conn())? + 1,
                        at: tx.at(),
                        term,
                        change: DirectoryChange::RenameClash {
                            account: other.account.clone(),
                            username: fallback.clone(),
                        },
                    };
                    apply(tx, &rename_entry)?;
                    outcome.entries.push(rename_entry);
                    outcome.clashed.push(ClashedAccount {
                        account: other.account,
                        home: other.home,
                        old_username: other.username,
                        new_username: fallback,
                    });

                    let register_entry = DirectoryEntry {
                        seq: head(tx.conn())? + 1,
                        at: reg_at,
                        term,
                        change: DirectoryChange::Register {
                            account,
                            username: desired,
                            home: from.clone(),
                        },
                    };
                    apply(tx, &register_entry)?;
                    outcome.entries.push(register_entry);
                } else {
                    // The existing holder registered first: it keeps the name, and the joining account gets a suffix
                    let fallback = next_free_suffix(tx.conn(), &desired)?;
                    let register_entry = DirectoryEntry {
                        seq: head(tx.conn())? + 1,
                        at: reg_at,
                        term,
                        change: DirectoryChange::RegisterClash {
                            account: account.clone(),
                            username: fallback.clone(),
                            home: from.clone(),
                        },
                    };
                    apply(tx, &register_entry)?;
                    outcome.entries.push(register_entry);
                    outcome.clashed.push(ClashedAccount {
                        account,
                        home: from.clone(),
                        old_username: desired,
                        new_username: fallback,
                    });
                }
            }
        }
    }
    Ok(outcome)
}

/// A register or rename the leader was asked for, by `from`.
#[derive(Debug, Clone)]
enum Proposal {
    Register { account: GlobalId, username: Username },
    Rename { account: GlobalId, username: Username },
}

/// The leader's side of a register or rename, in its write: answers a repeat with the entry that made
/// the state asked for, so a home that lost the answer can ask again.
fn propose(tx: &mut Tx<'_>, from: &NodeId, proposal: Proposal, term: u64) -> Result<DirectoryEntry, DirectoryError> {
    let change = match proposal {
        Proposal::Register { account, username } => DirectoryChange::Register {
            account,
            username,
            home: from.clone(),
        },
        Proposal::Rename { account, username } => DirectoryChange::Rename { account, username },
    };
    let (account, username) = match &change {
        DirectoryChange::Register { account, username, .. } | DirectoryChange::Rename { account, username } => {
            (account, username)
        }
        _ => return Err(Refusal::Malformed.into()),
    };
    if account.kind() != IdKind::Account {
        return Err(Refusal::NotAnAccount.into());
    }
    let existing = account_record(tx.conn(), account)?;
    let home = match &change {
        DirectoryChange::Register { .. } => account.owner(),
        _ => existing.as_ref().map_or(from, |record| &record.home),
    };
    if home != from {
        return Err(Refusal::NotHome.into());
    }
    if let Some(record) = existing
        && record.username == *username
        && record.home == *from
    {
        return entry_at(tx.conn(), record.seq)?.ok_or_else(|| corrupt("an account without its entry"));
    }
    check_change(tx.conn(), &change)?;
    let entry = DirectoryEntry {
        seq: head(tx.conn())? + 1,
        at: tx.at(),
        term,
        change,
    };
    apply(tx, &entry)?;
    Ok(entry)
}

// ------------------------------------------------------------------ the directory

/// One node's directory.
pub struct Directory {
    db: Database,
    node: NodeId,
    role: RwLock<Role>,
    page: u32,
    link: Arc<dyn PeerLink>,
    failover: Option<Failover>,
    /// The newest election term this node knows; 0 outside failover
    term: AtomicU64,
    /// This node's `lead` entry as leader: its term and number
    lead: Mutex<Option<(u64, u64)>>,
}

/// How many times one pull may drop a diverged tail before it gives up: a second one means the leader moved
/// mid-pull, a third something is wrong
const MAX_HEALS: usize = 3;

impl Directory {
    /// The directory on `db`, which must hold [`SCHEMA`]'s tables. `link` reaches the leader; a
    /// standalone server passes [`crate::link::NoPeers`].
    pub async fn new(db: Database, config: DirectoryConfig, link: Arc<dyn PeerLink>) -> Result<Self, DirectoryError> {
        db.read(|conn| head(conn).map(drop)).await?;
        Ok(Self {
            db,
            role: RwLock::new(config.role()),
            node: config.node,
            page: config.page.clamp(1, MAX_PULL),
            link,
            failover: config.failover,
            term: AtomicU64::new(0),
            lead: Mutex::new(None),
        })
    }

    /// This node.
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    /// Whether this node leads, replicates or stands alone; under failover it changes with each election.
    pub fn role(&self) -> Role {
        self.role.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The election term this node knows; 0 outside failover.
    pub fn term(&self) -> u64 {
        self.term.load(Ordering::SeqCst)
    }

    /// Whether the leader is elected (`docs/distributed-mode.md`).
    pub fn has_failover(&self) -> bool {
        self.failover.is_some()
    }

    /// The newest entry this node holds.
    pub async fn head(&self) -> Result<u64, DirectoryError> {
        self.db.read(head).await
    }

    /// Entries after `after`, oldest first.
    pub async fn entries(&self, after: u64, limit: u32) -> Result<Vec<DirectoryEntry>, DirectoryError> {
        self.db.read(move |conn| entries_after(conn, after, limit)).await
    }

    /// The account as this node's copy has it (a replica's may be behind the leader's).
    pub async fn account(&self, account: &GlobalId) -> Result<Option<AccountRecord>, DirectoryError> {
        let account = account.clone();
        self.db.read(move |conn| account_record(conn, &account)).await
    }

    /// The account's home, as this node's copy has it.
    pub async fn home_of(&self, account: &GlobalId) -> Result<Option<NodeId>, DirectoryError> {
        Ok(self.account(account).await?.map(|record| record.home))
    }

    /// The account holding `username`, in any case. Nothing for a name the rule refuses.
    pub async fn find(&self, username: &str) -> Result<Option<AccountRecord>, DirectoryError> {
        let Ok(username) = Username::new(username) else {
            return Ok(None);
        };
        self.db.read(move |conn| holder(conn, &username)).await
    }

    /// Whether `account`'s current username is a temporary suffixed name from a network join clash (DN13),
    /// so its next login should prompt for a free rename. Cleared as soon as [`Directory::rename`] succeeds.
    pub async fn rename_required(&self, account: &GlobalId) -> Result<bool, DirectoryError> {
        let account = account.clone();
        self.db
            .read(move |conn| {
                let Some(record) = account_record(conn, &account)? else {
                    return Ok(false);
                };
                let Some(entry) = entry_at(conn, record.seq)? else {
                    return Ok(false);
                };
                Ok(entry.change.is_clash())
            })
            .await
    }

    /// Joins an existing server's accounts to the directory with this node as their home (DN13): through the
    /// leader in a network, here otherwise.
    ///
    /// On a username clash, the account registered first keeps the name; every other account with that
    /// normalized name gets a temporary suffixed name (`Fox_2`, shortened to fit 16 characters) and is
    /// returned in [`JoinOutcome::clashed`] (and marked in the directory log so [`Directory::rename_required`]
    /// is `true` until its free rename).
    pub async fn join(&self, actor: Actor, accounts: Vec<JoiningAccount>) -> Result<JoinOutcome, DirectoryError> {
        for item in &accounts {
            if item.account.kind() != IdKind::Account {
                return Err(Refusal::NotAnAccount.into());
            }
            if item.account.owner() != &self.node && item.account.owner().as_str() != "local" {
                return Err(Refusal::NotHome.into());
            }
            Self::username(&item.username)?;
        }
        let leader = match self.role() {
            Role::Replica { leader } => leader,
            Role::Electing => return Err(DirectoryError::NoLeader),
            Role::Leader | Role::Standalone => {
                let from = self.node.clone();
                return self.join_here(actor, from, accounts).await;
            }
        };
        let call = DirectoryCall::Join {
            accounts,
            actor: actor.as_text(),
        };
        let outcome = match self.call_leader(&leader, call).await? {
            DirectoryReply::Joined(outcome) => outcome,
            DirectoryReply::Refused(refusal) => return Err(refusal.into()),
            DirectoryReply::Applied(_) | DirectoryReply::Changes { .. } | DirectoryReply::Caught { .. } => {
                return Err(DirectoryError::Protocol("unexpected reply for a join".into()));
            }
        };
        if let Err(error) = self.sync().await {
            tracing::debug!(%error, "directory: no pull after a join");
        }
        Ok(outcome)
    }

    /// Joins a Reforged host's existing database (`accounts` and `launcher_accounts`) to this node's
    /// directory (DN13), and writes any temporary suffixed names and `username_set = false` prompts for
    /// accounts homed on this node back to `host_db`.
    pub async fn join_host_database(&self, actor: Actor, host_db: &Database) -> Result<JoinOutcome, DirectoryError> {
        let accounts = read_host_accounts(host_db).await?;
        let outcome = self.join(actor.clone(), accounts).await?;
        apply_host_clashes(host_db, actor, &self.node, &outcome.clashed).await?;
        Ok(outcome)
    }

    /// Registers an account homed on this node: through the leader in a network, here otherwise.
    /// `actor` is who asked, for the change log.
    pub async fn register(
        &self,
        actor: Actor,
        account: &GlobalId,
        username: &str,
    ) -> Result<DirectoryEntry, DirectoryError> {
        let name = Self::username(username)?;
        if account.owner() != &self.node {
            return Err(Refusal::NotHome.into());
        }
        let proposal = Proposal::Register {
            account: account.clone(),
            username: name,
        };
        let call = DirectoryCall::Register {
            account: account.clone(),
            username: username.to_owned(),
            actor: actor.as_text(),
        };
        self.change(actor, proposal, call).await
    }

    /// Gives an account homed on this node a new username; the old one is free once the leader has it.
    pub async fn rename(
        &self,
        actor: Actor,
        account: &GlobalId,
        username: &str,
    ) -> Result<DirectoryEntry, DirectoryError> {
        let name = Self::username(username)?;
        let proposal = Proposal::Rename {
            account: account.clone(),
            username: name,
        };
        let call = DirectoryCall::Rename {
            account: account.clone(),
            username: username.to_owned(),
            actor: actor.as_text(),
        };
        self.change(actor, proposal, call).await
    }

    fn username(text: &str) -> Result<Username, DirectoryError> {
        Username::new(text).map_err(|e| Refusal::InvalidUsername { check: e.0 }.into())
    }

    async fn change(
        &self,
        actor: Actor,
        proposal: Proposal,
        call: DirectoryCall,
    ) -> Result<DirectoryEntry, DirectoryError> {
        let mut leader = match self.role() {
            Role::Replica { leader } => leader,
            Role::Electing => return Err(DirectoryError::NoLeader),
            Role::Leader | Role::Standalone => return self.propose_here(actor, self.node.clone(), proposal).await,
        };
        let mut reply = self.call_leader(&leader, call.clone()).await?;
        // An election may have moved the leader since this node last heard: follow the hint once
        if self.failover.is_some()
            && let DirectoryReply::Refused(Refusal::NotLeader { leader: Some(hint) }) = &reply
            && *hint != leader
            && *hint != self.node
        {
            leader = hint.clone();
            reply = self.call_leader(&leader, call).await?;
        }
        let entry = match reply {
            DirectoryReply::Applied(entry) => entry,
            DirectoryReply::Refused(refusal) => return Err(refusal.into()),
            DirectoryReply::Changes { .. } | DirectoryReply::Joined(_) | DirectoryReply::Caught { .. } => {
                return Err(DirectoryError::Protocol("changes for a write".into()));
            }
        };
        let (account, username) = match &proposal {
            Proposal::Register { account, username } | Proposal::Rename { account, username } => (account, username),
        };
        let asked = entry.change.account() == Some(account)
            && entry.change.username() == Some(username)
            && match (&proposal, &entry.change) {
                (Proposal::Register { .. }, DirectoryChange::Register { home, .. }) => *home == self.node,
                (Proposal::Register { .. }, _) => false,
                // A rename to the name it has already answers with the entry that gave it
                (Proposal::Rename { .. }, _) => true,
            };
        if !asked {
            return Err(DirectoryError::Protocol(format!(
                "entry {} is not the change asked for",
                entry.seq
            )));
        }
        // Done at the leader; this copy catches up now or on the next pull
        if let Err(error) = self.sync_from(&leader).await {
            tracing::debug!(%error, "directory: no pull after a change");
        }
        Ok(entry)
    }

    async fn propose_here(
        &self,
        actor: Actor,
        from: NodeId,
        proposal: Proposal,
    ) -> Result<DirectoryEntry, DirectoryError> {
        self.write_here(actor, move |tx, term| {
            let entry = propose(tx, &from, proposal, term)?;
            let seq = entry.seq;
            Ok((entry, seq))
        })
        .await
    }

    async fn join_here(
        &self,
        actor: Actor,
        from: NodeId,
        accounts: Vec<JoiningAccount>,
    ) -> Result<JoinOutcome, DirectoryError> {
        self.write_here(actor, move |tx, term| {
            let outcome = join_batch(tx, &from, accounts, term)?;
            let newest = outcome.entries.iter().map(|entry| entry.seq).max().unwrap_or(0);
            Ok((outcome, newest))
        })
        .await
    }

    /// A write on the leader or a standalone server. `write` gets the term to stamp its entries with and
    /// returns the newest entry it needs on a majority; under failover the answer waits until it is there.
    async fn write_here<T, F>(&self, actor: Actor, write: F) -> Result<T, DirectoryError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Tx<'_>, u64) -> Result<(T, u64), DirectoryError> + Send + 'static,
    {
        if self.failover.is_none() {
            return self
                .db
                .write(actor, move |tx| write(tx, 0).map(|(value, _)| value))
                .await;
        }
        let term = self.term();
        match self.role() {
            Role::Leader => {}
            Role::Replica { leader } => return Err(Refusal::NotLeader { leader: Some(leader) }.into()),
            Role::Electing | Role::Standalone => return Err(DirectoryError::NoLeader),
        }
        let lead = *self.lead.lock().unwrap_or_else(PoisonError::into_inner);
        let wrote_lead = lead.is_none_or(|(lead_term, _)| lead_term != term);
        let node = self.node.clone();
        let (value, newest, lead_seq) = self
            .db
            .write(actor, move |tx| {
                let lead_seq = if wrote_lead {
                    let entry = DirectoryEntry {
                        seq: head(tx.conn())? + 1,
                        at: tx.at(),
                        term,
                        change: DirectoryChange::Lead { node },
                    };
                    apply(tx, &entry)?;
                    Some(entry.seq)
                } else {
                    None
                };
                let (value, newest) = write(tx, term)?;
                Ok::<_, DirectoryError>((value, newest, lead_seq))
            })
            .await?;
        let lead_seq = match lead_seq {
            Some(seq) => {
                *self.lead.lock().unwrap_or_else(PoisonError::into_inner) = Some((term, seq));
                seq
            }
            None => lead.map_or(0, |(_, seq)| seq),
        };
        // Raft's commit rule: nothing is acknowledged before an entry of this term is on a majority
        self.replicate(term, newest.max(lead_seq)).await?;
        Ok(value)
    }

    /// Asks every peer to pull from this leader, and returns once a majority, this node included, holds entry
    /// `upto`: [`Refusal::NoQuorum`] if that takes longer than the commit timeout.
    async fn replicate(&self, term: u64, upto: u64) -> Result<(), DirectoryError> {
        let Some(failover) = &self.failover else {
            return Ok(());
        };
        let quorum = failover.quorum();
        if quorum <= 1 {
            return Ok(());
        }
        let request = DirectoryRequest {
            version: DIRECTORY_VERSION,
            call: DirectoryCall::Notify { term },
        };
        let body = encode(&request).map_err(|e| DirectoryError::Protocol(e.to_string()))?;
        let mut calls = tokio::task::JoinSet::new();
        for peer in &failover.peers {
            let (link, peer, body) = (self.link.clone(), peer.clone(), body.clone());
            calls.spawn(async move {
                let answer = link.call(&peer, Service::Directory, body).await;
                (peer, answer)
            });
        }
        let mut holding = 1;
        let deadline = tokio::time::Instant::now() + failover.commit_timeout;
        while holding < quorum {
            let Ok(next) = tokio::time::timeout_at(deadline, calls.join_next()).await else {
                break;
            };
            let (peer, answer) = match next {
                None => break,
                Some(Err(_)) => continue,
                Some(Ok(done)) => done,
            };
            match answer.map(|body| decode_response(&body).map(|response| response.reply)) {
                Ok(Ok(DirectoryReply::Caught { head })) if head >= upto => holding += 1,
                Ok(Ok(DirectoryReply::Refused(Refusal::Superseded { term: newer }))) => {
                    tracing::info!(peer = %peer, term = newer, "directory: a newer leader is elected");
                }
                Ok(Ok(reply)) => tracing::debug!(peer = %peer, ?reply, "directory: a replica didn't catch up"),
                Ok(Err(error)) => tracing::debug!(peer = %peer, %error, "directory: a replica's answer is broken"),
                Err(error) => tracing::debug!(peer = %peer, %error, "directory: a replica didn't answer"),
            }
        }
        if holding >= quorum {
            Ok(())
        } else {
            tracing::warn!(
                term,
                upto,
                holding,
                quorum,
                "directory: no majority holds the change yet"
            );
            Err(Refusal::NoQuorum.into())
        }
    }

    async fn call_leader(&self, leader: &NodeId, call: DirectoryCall) -> Result<DirectoryReply, DirectoryError> {
        let request = DirectoryRequest {
            version: DIRECTORY_VERSION,
            call,
        };
        let body = encode(&request).map_err(|e| DirectoryError::Protocol(e.to_string()))?;
        let answer = self
            .link
            .call(leader, Service::Directory, body)
            .await
            .map_err(DirectoryError::LeaderUnreachable)?;
        let response = decode_response(&answer).map_err(|e| DirectoryError::Protocol(e.to_string()))?;
        Ok(response.reply)
    }

    /// A replica pulls the leader's entries after its own until it holds them all. Returns how many it
    /// applied; nothing to do on the leader or a standalone server.
    pub async fn sync(&self) -> Result<u64, DirectoryError> {
        let Role::Replica { leader } = self.role() else {
            return Ok(0);
        };
        self.sync_from(&leader).await
    }

    /// Pulls from `leader` until this node holds everything it does. Under failover a tail `leader` doesn't hold
    /// is dropped first; with a fixed leader replication stops there.
    async fn sync_from(&self, leader: &NodeId) -> Result<u64, DirectoryError> {
        let mut applied = 0;
        let mut heals = 0;
        loop {
            let (after, last) = self
                .db
                .read(|conn| {
                    let after = head(conn)?;
                    let last = if after == 0 { None } else { entry_at(conn, after)? };
                    Ok::<_, DirectoryError>((after, last))
                })
                .await?;
            let call = DirectoryCall::Pull {
                after,
                last,
                limit: self.page,
            };
            let (entries, leader_head) = match self.call_leader(leader, call).await? {
                DirectoryReply::Changes { entries, head } => (entries, head),
                DirectoryReply::Refused(Refusal::Diverged) if self.failover.is_some() && heals < MAX_HEALS => {
                    heals += 1;
                    self.heal(leader, after).await?;
                    continue;
                }
                DirectoryReply::Refused(Refusal::Diverged) => {
                    let error = DirectoryError::Diverged(format!("{leader} doesn't hold this node's entry {after}"));
                    tracing::warn!(%error, "directory: replication stopped");
                    return Err(error);
                }
                DirectoryReply::Refused(refusal) => return Err(refusal.into()),
                DirectoryReply::Applied(_) | DirectoryReply::Joined(_) | DirectoryReply::Caught { .. } => {
                    return Err(DirectoryError::Protocol("an entry for a pull".into()));
                }
            };
            if entries.len() > self.page as usize {
                return Err(DirectoryError::Protocol(format!(
                    "{} entries for a page",
                    entries.len()
                )));
            }
            if entries.is_empty() {
                if leader_head > after {
                    return Err(DirectoryError::Protocol(format!(
                        "no entries after {after}, with the newest at {leader_head}"
                    )));
                }
                if leader_head < after {
                    if self.failover.is_some() && heals < MAX_HEALS {
                        heals += 1;
                        self.heal(leader, after).await?;
                        continue;
                    }
                    return Err(DirectoryError::Diverged(format!(
                        "{leader}'s newest entry is {leader_head}, this node's {after}"
                    )));
                }
                return Ok(applied);
            }
            let (count, now) = self
                .db
                .write(Actor::System(REPLICATION.into()), move |tx| {
                    let held = head(tx.conn())?;
                    let mut next = held + 1;
                    let mut count = 0;
                    // A pull that ran beside this one may have applied some already
                    for entry in entries.into_iter().filter(|e| e.seq > held) {
                        if entry.seq != next {
                            return Err(DirectoryError::Protocol(format!(
                                "entry {} where {next} was due",
                                entry.seq
                            )));
                        }
                        check_change(tx.conn(), &entry.change).map_err(|error| match error {
                            DirectoryError::Refused(refusal) => {
                                DirectoryError::Diverged(format!("entry {} can't apply here: {refusal}", entry.seq))
                            }
                            other => other,
                        })?;
                        apply(tx, &entry)?;
                        next += 1;
                        count += 1;
                    }
                    Ok::<_, DirectoryError>((count, next - 1))
                })
                .await
                .inspect_err(|error| {
                    if matches!(error, DirectoryError::Diverged(_)) {
                        tracing::warn!(%error, "directory: replication stopped");
                    }
                })?;
            applied += count;
            if now <= after {
                return Err(DirectoryError::Protocol(format!(
                    "a page after {after} that adds nothing"
                )));
            }
            if now >= leader_head {
                return Ok(applied);
            }
        }
    }

    /// Whether `leader` holds this node's entry `seq` as it is here.
    async fn agrees(&self, leader: &NodeId, seq: u64) -> Result<bool, DirectoryError> {
        if seq == 0 {
            return Ok(true);
        }
        let last = self.db.read(move |conn| entry_at(conn, seq)).await?;
        let call = DirectoryCall::Pull {
            after: seq,
            last,
            limit: 1,
        };
        match self.call_leader(leader, call).await? {
            DirectoryReply::Changes { .. } => Ok(true),
            DirectoryReply::Refused(Refusal::Diverged) => Ok(false),
            DirectoryReply::Refused(refusal) => Err(refusal.into()),
            _ => Err(DirectoryError::Protocol("not a page for a pull".into())),
        }
    }

    /// Under failover: finds the newest entry this node shares with `leader` (below `after`, which it doesn't)
    /// and drops everything after it. Raft's log repair: what goes was never on a majority, so never acknowledged.
    async fn heal(&self, leader: &NodeId, after: u64) -> Result<(), DirectoryError> {
        // Gallop down from the tail, then halve: a diverged tail is a few unacknowledged writes, not the log
        let (mut good, mut bad) = (0, after);
        let mut step = 1;
        while step < after {
            let probe = after - step;
            if self.agrees(leader, probe).await? {
                good = probe;
                break;
            }
            bad = probe;
            step = step.saturating_mul(2);
        }
        while bad - good > 1 {
            let middle = good + (bad - good) / 2;
            if self.agrees(leader, middle).await? {
                good = middle;
            } else {
                bad = middle;
            }
        }
        let dropped = self
            .db
            .write(Actor::System(REPLICATION.into()), move |tx| truncate_after(tx, good))
            .await?;
        tracing::warn!(%leader, kept = good, dropped, "directory: dropped entries the elected leader doesn't hold");
        Ok(())
    }

    /// Pulls every `every` on a replica, for as long as the directory lives. Nothing on the leader or a
    /// standalone server; under failover it runs whatever the role, and pulls only while following.
    pub fn spawn_replication(self: &Arc<Self>, every: Duration) -> Option<tokio::task::JoinHandle<()>> {
        if self.failover.is_none() && !matches!(self.role(), Role::Replica { .. }) {
            return None;
        }
        let directory = Arc::downgrade(self);
        Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(directory) = directory.upgrade() else {
                    break;
                };
                match directory.sync().await {
                    Ok(_) => {}
                    // Expected in a partition, every tick until it heals
                    Err(error @ DirectoryError::LeaderUnreachable(_)) => {
                        tracing::debug!(%error, "directory: pull failed");
                    }
                    // The leader moved; the election says where soon
                    Err(error @ DirectoryError::Refused(Refusal::NotLeader { .. })) => {
                        tracing::debug!(%error, "directory: pull failed");
                    }
                    Err(error) => tracing::warn!(%error, "directory: pull failed"),
                }
            }
        }))
    }

    /// Under failover: takes `leadership` as who leads now, unless this node already knows a newer term.
    /// Returns whether this node has just become the leader.
    pub fn observe(&self, leadership: &Leadership) -> bool {
        if self.failover.is_none() {
            return false;
        }
        let mut role = self.role.write().unwrap_or_else(PoisonError::into_inner);
        if leadership.term < self.term.load(Ordering::SeqCst) {
            return false;
        }
        self.term.store(leadership.term, Ordering::SeqCst);
        let next = match &leadership.leader {
            Some(leader) if *leader == self.node => Role::Leader,
            Some(leader) => Role::Replica { leader: leader.clone() },
            None => Role::Electing,
        };
        let became = next == Role::Leader && *role != Role::Leader;
        *role = next;
        became
    }

    /// Follows an election's decisions for as long as the directory lives. On winning, this node writes its
    /// `lead` entry and pushes it to a majority, which settles whatever the old leader left unacknowledged.
    pub fn follow(
        self: &Arc<Self>,
        mut leadership: tokio::sync::watch::Receiver<Leadership>,
    ) -> tokio::task::JoinHandle<()> {
        let directory = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let now = leadership.borrow_and_update().clone();
                let Some(dir) = directory.upgrade() else {
                    break;
                };
                if dir.observe(&now) {
                    match dir.establish().await {
                        Ok(()) => tracing::info!(term = now.term, "directory: leading"),
                        Err(error) => {
                            tracing::warn!(term = now.term, %error, "directory: leading, but no majority yet")
                        }
                    }
                }
                drop(dir);
                if leadership.changed().await.is_err() {
                    break;
                }
            }
        })
    }

    /// The first write of a new leader: its `lead` entry, on a majority.
    pub async fn establish(&self) -> Result<(), DirectoryError> {
        self.write_here(Actor::System(REPLICATION.into()), |_, _| Ok(((), 0)))
            .await
    }

    /// Answers a peer's call. `from` is the caller as its link authenticated it.
    pub async fn handle(&self, from: &NodeId, call: DirectoryCall) -> DirectoryReply {
        if let DirectoryCall::Notify { term } = call {
            return self.serve_notify(from, term).await.unwrap_or_else(|error| match error {
                DirectoryError::Refused(refusal) => DirectoryReply::Refused(refusal),
                other => {
                    tracing::warn!(error = %other, %from, "directory: a leader's notify failed");
                    DirectoryReply::Refused(Refusal::Unavailable)
                }
            });
        }
        match self.role() {
            Role::Replica { leader } => {
                return DirectoryReply::Refused(Refusal::NotLeader { leader: Some(leader) });
            }
            Role::Electing => return DirectoryReply::Refused(Refusal::NotLeader { leader: None }),
            Role::Leader | Role::Standalone => {}
        }
        let result = match call {
            DirectoryCall::Register {
                account,
                username,
                actor,
            } => self.serve_proposal(from, &actor, account, &username, true).await,
            DirectoryCall::Rename {
                account,
                username,
                actor,
            } => self.serve_proposal(from, &actor, account, &username, false).await,
            DirectoryCall::Join { accounts, actor } => self.serve_join(from, &actor, accounts).await,
            DirectoryCall::Pull { after, last, limit } => self.serve_pull(after, last, limit).await,
            DirectoryCall::Notify { .. } => Err(Refusal::Malformed.into()),
        };
        result.unwrap_or_else(|error| match error {
            DirectoryError::Refused(refusal) => DirectoryReply::Refused(refusal),
            DirectoryError::NoLeader => DirectoryReply::Refused(Refusal::NotLeader { leader: None }),
            other => {
                tracing::error!(error = %other, %from, "directory: a peer's call failed");
                DirectoryReply::Refused(Refusal::Unavailable)
            }
        })
    }

    /// A leader of `term` asks this node to pull from it: the caller is the node its link authenticated, and a
    /// term older than this node's is refused, so a deposed leader can't count it towards a majority.
    async fn serve_notify(&self, from: &NodeId, term: u64) -> Result<DirectoryReply, DirectoryError> {
        let Some(failover) = &self.failover else {
            return Err(Refusal::Malformed.into());
        };
        if !failover.peers.contains(from) {
            return Err(Refusal::Malformed.into());
        }
        let known = self.term();
        if term < known {
            return Ok(DirectoryReply::Refused(Refusal::Superseded { term: known }));
        }
        // Only the elected leader of a term notifies for it, so the caller leads; the election hears soon too
        self.observe(&Leadership {
            term,
            leader: Some(from.clone()),
        });
        self.sync_from(from).await?;
        Ok(DirectoryReply::Caught {
            head: self.head().await?,
        })
    }

    async fn serve_proposal(
        &self,
        from: &NodeId,
        actor: &str,
        account: GlobalId,
        username: &str,
        register: bool,
    ) -> Result<DirectoryReply, DirectoryError> {
        let actor = Actor::parse(actor).ok_or(Refusal::BadActor)?;
        let username = Self::username(username)?;
        let proposal = if register {
            Proposal::Register { account, username }
        } else {
            Proposal::Rename { account, username }
        };
        let entry = self.propose_here(actor, from.clone(), proposal).await?;
        Ok(DirectoryReply::Applied(entry))
    }

    async fn serve_join(
        &self,
        from: &NodeId,
        actor: &str,
        accounts: Vec<JoiningAccount>,
    ) -> Result<DirectoryReply, DirectoryError> {
        let actor = Actor::parse(actor).ok_or(Refusal::BadActor)?;
        let outcome = self.join_here(actor, from.clone(), accounts).await?;
        Ok(DirectoryReply::Joined(outcome))
    }

    async fn serve_pull(
        &self,
        after: u64,
        last: Option<DirectoryEntry>,
        limit: u32,
    ) -> Result<DirectoryReply, DirectoryError> {
        if (after == 0) != last.is_none() {
            return Err(Refusal::Malformed.into());
        }
        let limit = limit.clamp(1, MAX_PULL);
        self.db
            .read(move |conn| {
                if let Some(last) = last
                    && entry_at(conn, after)?.as_ref() != Some(&last)
                {
                    return Err(Refusal::Diverged.into());
                }
                Ok(DirectoryReply::Changes {
                    entries: entries_after(conn, after, limit)?,
                    head: head(conn)?,
                })
            })
            .await
    }
}

impl LogSource for Directory {
    fn position(&self) -> BoxFuture<'_, Result<LogPosition, String>> {
        Box::pin(async move { self.db.read(position).await.map_err(|e: DirectoryError| e.to_string()) })
    }
}

impl NodeService for Directory {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Directory {
                return Err(LinkError::NoService {
                    node: self.node.clone(),
                    service,
                });
            }
            let reply = match decode_request(&body) {
                Ok(request) => self.handle(from, request.call).await,
                Err(DecodeError::Version(_)) => DirectoryReply::Refused(Refusal::UnsupportedVersion {
                    supported: DIRECTORY_VERSION,
                }),
                Err(_) => DirectoryReply::Refused(Refusal::Malformed),
            };
            encode(&DirectoryResponse {
                version: DIRECTORY_VERSION,
                reply,
            })
            .map_err(|_| LinkError::TooLarge)
        })
    }
}

/// Reads existing accounts from a Reforged host's game database (`launcher_accounts` and `accounts`) for
/// [`Directory::join`] (DN13).
pub async fn read_host_accounts(host_db: &Database) -> Result<Vec<JoiningAccount>, DirectoryError> {
    host_db
        .read(|conn| {
            let mut seen = std::collections::BTreeSet::new();
            let mut out = Vec::new();
            let rows: Vec<(String, String, String, Option<i64>)> = host_launcher_accounts::table
                .filter(host_launcher_accounts::deleted_at.is_null())
                .select((
                    host_launcher_accounts::account_id,
                    host_launcher_accounts::display_name,
                    host_launcher_accounts::created_at,
                    host_launcher_accounts::username_set,
                ))
                .order((
                    host_launcher_accounts::created_at.asc(),
                    host_launcher_accounts::account_id.asc(),
                ))
                .load(conn)?;
            for (account_id, display_name, created_at, username_set) in rows {
                let account: GlobalId = account_id
                    .parse()
                    .map_err(|_| corrupt(format!("the account id {account_id:?}")))?;
                // An account registered without a display name holds its userId as a placeholder until its
                // first username choice; skip it if it isn't a valid username yet
                if username_set == Some(0) && Username::new(&display_name).is_err() {
                    seen.insert(account_id);
                    continue;
                }
                let registered_at =
                    time::OffsetDateTime::parse(&created_at, &time::format_description::well_known::Rfc3339)
                        .map(|dt| i64::try_from(dt.unix_timestamp_nanos() / 1_000_000).unwrap_or(0))
                        .unwrap_or_else(|_| i64::try_from(account.ulid().timestamp_ms()).unwrap_or(0));
                seen.insert(account_id);
                out.push(JoiningAccount {
                    account,
                    username: display_name,
                    registered_at,
                });
            }
            let game_rows: Vec<(String, String)> = host_accounts::table
                .filter(host_accounts::deleted_at.is_null())
                .select((host_accounts::id, host_accounts::name))
                .order(host_accounts::id.asc())
                .load(conn)?;
            for (id, name) in game_rows {
                if seen.contains(&id) {
                    continue;
                }
                let account: GlobalId = id.parse().map_err(|_| corrupt(format!("the account id {id:?}")))?;
                if Username::new(&name).is_err() {
                    continue;
                }
                let registered_at = i64::try_from(account.ulid().timestamp_ms()).unwrap_or(0);
                out.push(JoiningAccount {
                    account,
                    username: name,
                    registered_at,
                });
            }
            Ok(out)
        })
        .await
}

/// Applies the temporary suffixed names and `username_set = false` prompts from [`JoinOutcome::clashed`]
/// to the accounts homed on `node` in `host_db` (`launcher_accounts` and `accounts`, DN13).
pub async fn apply_host_clashes(
    host_db: &Database,
    actor: Actor,
    node: &NodeId,
    clashed: &[ClashedAccount],
) -> Result<usize, DirectoryError> {
    let local_clashes: Vec<ClashedAccount> = clashed.iter().filter(|c| &c.home == node).cloned().collect();
    if local_clashes.is_empty() {
        return Ok(0);
    }
    host_db
        .write(actor, move |tx| {
            // Park normalized names on unique placeholders first so a chain (Fox -> Fox_2) never collides
            for item in &local_clashes {
                let id = item.account.to_string();
                let placeholder = format!("__clash_{id}");
                diesel::update(host_launcher_accounts::table.filter(host_launcher_accounts::account_id.eq(&id)))
                    .set(host_launcher_accounts::display_name_normalized.eq(placeholder))
                    .execute(tx.conn())?;
            }
            let mut updated = 0;
            for item in &local_clashes {
                let id = item.account.to_string();
                let new_name = item.new_username.as_str();
                let new_key = item.new_username.key();
                let l_rows =
                    diesel::update(host_launcher_accounts::table.filter(host_launcher_accounts::account_id.eq(&id)))
                        .set((
                            host_launcher_accounts::display_name.eq(new_name),
                            host_launcher_accounts::display_name_normalized.eq(new_key),
                            host_launcher_accounts::username_set.eq(Some(0_i64)),
                        ))
                        .execute(tx.conn())?;
                let a_rows = diesel::update(host_accounts::table.filter(host_accounts::id.eq(&id)))
                    .set(host_accounts::name.eq(new_name))
                    .execute(tx.conn())?;
                if l_rows > 0 || a_rows > 0 {
                    updated += 1;
                }
            }
            Ok(updated)
        })
        .await
}
