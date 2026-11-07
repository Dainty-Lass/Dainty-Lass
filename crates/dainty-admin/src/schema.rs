//! The GraphQL schema (written out in `schema.graphql`, checked by a test) and what each field does
//! against the backend and the Director.

use std::collections::HashMap;
use std::sync::Arc;

use async_graphql::{Context, EmptySubscription, Enum, ErrorExtensions, Object, Schema, SimpleObject};
use dainty_module_api::module::PlayerInfo;
use dainty_module_api::{AdminRequest, AdminResponse, ErrorKind, ModuleError};
use serde_json::json;
use tokio::task::JoinSet;

use crate::config;
use crate::guard::Guards;
use crate::guarded::{APPROVED_BY, Gate, GuardedAction};
use crate::history;
use crate::history_text;
use crate::host_settings::{self, LogSettings, LogSettingsInput};
use crate::input::{self, InstanceKind};
use crate::keys::{Action, Caller, KeyError, KeyInfo, Keys, Role};
use crate::log;
use crate::module_control::{self, Module, Modules};
use crate::modules::{AdminModule, AdminPlugin, NODE_GAME};
use crate::timeouts::{Timeout, Timeouts};
use crate::upstream::{Account, Clock, Operator, Upstream, UpstreamError};

/// The admin API's GraphQL schema.
pub type AdminSchema = Schema<Query, Mutation, EmptySubscription>;

/// Deep enough for every field here, small enough that no query fans out into thousands of calls.
const MAX_DEPTH: usize = 6;
const MAX_COMPLEXITY: usize = 200;

/// The schema over `admin`, with no module registry behind `modules`.
pub fn build(admin: Arc<Admin>) -> AdminSchema {
    build_with_control(admin, Modules::default())
}

/// [`build`] with the registry's list, load and unload behind `modules` (DF3).
pub fn build_with_control(admin: Arc<Admin>, modules: Modules) -> AdminSchema {
    Schema::build(Query, Mutation, EmptySubscription)
        .data(admin)
        .data(modules)
        .limit_depth(MAX_DEPTH)
        .limit_complexity(MAX_COMPLEXITY)
        .finish()
}

/// The schema in GraphQL's own language, as `schema.graphql` holds it.
pub fn sdl() -> String {
    Schema::build(Query, Mutation, EmptySubscription).finish().sdl()
}

// ------------------------------------------------------------------ errors

/// The `code` in an error's `extensions`, which the bot switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// The input was refused: a malformed name, reason, time or id.
    BadInput,
    /// No such player, key or action.
    NotFound,
    /// The player holds the backend's admin role.
    Protected,
    /// The player is already banned.
    AlreadyBanned,
    /// The player is not banned.
    NotBanned,
    /// The player is not timed out.
    NotTimedOut,
    /// The backend or Director did not answer.
    BackendUnavailable,
    /// The backend or a module refused.
    BackendRefused,
    /// The change was made, but its record (the timeouts file or ticket state) was not saved.
    StateNotSaved,
    /// The name matches players in more than one game: name the game.
    AmbiguousPlayer,
    /// This server runs no such game.
    UnknownGame,
    /// The player's game does not do this through the admin API.
    NotSupported,
    /// The key's role does not allow it, or the guard refused the caller.
    Forbidden,
    /// The action was not done but held (DH6): it waits for its delay or a second operator.
    Guarded,
    /// A guarded action that is not waiting, or that waits out a delay and cannot be approved.
    NotWaiting,
    /// The rows a revert plan covers changed since it was made.
    PlanChanged,
    /// The revert plan is no longer kept.
    PlanExpired,
    /// A change log could not be read.
    HistoryUnavailable,
    /// A revert changed some databases but not all.
    PartialRevert,
}

impl Code {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BadInput => "BAD_INPUT",
            Self::NotFound => "NOT_FOUND",
            Self::Protected => "PROTECTED_ACCOUNT",
            Self::AlreadyBanned => "ALREADY_BANNED",
            Self::NotBanned => "NOT_BANNED",
            Self::NotTimedOut => "NOT_TIMED_OUT",
            Self::BackendUnavailable => "BACKEND_UNAVAILABLE",
            Self::BackendRefused => "BACKEND_REFUSED",
            Self::StateNotSaved => "STATE_NOT_SAVED",
            Self::AmbiguousPlayer => "AMBIGUOUS_PLAYER",
            Self::UnknownGame => "UNKNOWN_GAME",
            Self::NotSupported => "NOT_SUPPORTED",
            Self::Forbidden => "FORBIDDEN",
            Self::Guarded => "GUARDED",
            Self::NotWaiting => "NOT_WAITING",
            Self::PlanChanged => "PLAN_CHANGED",
            Self::PlanExpired => "PLAN_EXPIRED",
            Self::HistoryUnavailable => "HISTORY_UNAVAILABLE",
            Self::PartialRevert => "PARTIAL_REVERT",
        }
    }
}

pub(crate) fn fail(code: Code, message: impl Into<String>) -> async_graphql::Error {
    async_graphql::Error::new(message).extend_with(|_, e| e.set("code", code.as_str()))
}

fn no_such_player() -> async_graphql::Error {
    fail(Code::NotFound, "no such player")
}

/// A module's failure as the bot sees it, its own words capped to one line (`input::module_message`).
fn module_failure(module: &AdminModule, error: ModuleError) -> async_graphql::Error {
    let message = input::module_message(&error.message);
    log::warn(
        "module admin call failed",
        json!({"module": module.id.to_string(), "kind": error.kind.to_string(), "error": message}),
    );
    let name = &module.display_name;
    match error.kind {
        ErrorKind::NotFound => no_such_player(),
        ErrorKind::Unsupported => fail(
            Code::NotSupported,
            format!("{name} does not do this through the admin API"),
        ),
        ErrorKind::Invalid => fail(Code::BadInput, format!("{name} refused: {message}")),
        ErrorKind::Failed => fail(Code::BackendRefused, format!("{name} refused: {message}")),
        ErrorKind::Protocol | ErrorKind::Unavailable | ErrorKind::Timeout => fail(
            Code::BackendUnavailable,
            format!("{name} did not answer; is it running?"),
        ),
    }
}

fn unexpected(answer: &AdminResponse) -> ModuleError {
    ModuleError::new(ErrorKind::Protocol, format!("answered {answer:?}"))
}

/// An upstream failure as the bot sees it: what to do about it, without the backend's addresses.
fn upstream(service: &str, error: UpstreamError) -> async_graphql::Error {
    log::warn(&format!("{service} call failed"), json!({"error": error.to_string()}));
    match error {
        UpstreamError::Unavailable(_) => fail(
            Code::BackendUnavailable,
            format!("the {service} did not answer; is the server running?"),
        ),
        UpstreamError::Refused { status: 404, .. } => fail(Code::NotFound, "no such player"),
        UpstreamError::Refused { status, code } => {
            let code = code.unwrap_or_default();
            let hint = if code.starts_with("ADMIN_OPERATOR") || matches!(status, 401 | 403 | 503) {
                ": the admin API's credentials do not match the backend's; run dainty-host setup and restart the server"
            } else {
                ""
            };
            fail(
                Code::BackendRefused,
                format!("the {service} refused ({status} {code}){hint}"),
            )
        }
    }
}

// ------------------------------------------------------------------ types

/// An account's state in its game.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AccountStatus {
    /// May play.
    Active,
    /// Banned or timed out.
    Banned,
    /// Disabled by the backend's own admin tools.
    Disabled,
    /// A status this API does not know, or a player in a game whose module doesn't report one.
    Unknown,
}

/// What keeps a player out.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RestrictionKind {
    /// Banned until someone lifts it.
    Ban,
    /// Banned until `until`, then lifted by the admin API.
    Timeout,
    /// Disabled by the backend's own admin tools.
    Disabled,
}

/// Why a player cannot play right now.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Restriction {
    /// Ban, timeout or disabled.
    pub kind: RestrictionKind,
    /// The text the player sees in the launcher.
    pub reason: Option<String>,
    /// When it was set (RFC 3339).
    pub since: Option<String>,
    /// Who set it: `manager:<operator id>` for the admin API, an account id for the backend's dashboard.
    pub by: Option<String>,
    /// When a timeout ends (RFC 3339); null for the other kinds.
    pub until: Option<String>,
}

/// A timeout the admin API will lift: what `timeouts.json` holds, not the backend's view of the account.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct ActiveTimeout {
    /// The player's name.
    pub username: String,
    /// When it ends (RFC 3339).
    pub until: String,
    /// The text the player sees in the launcher.
    pub reason: String,
    /// `manager:<operator id>`.
    pub by: String,
    /// When it was set (RFC 3339).
    pub at: String,
}

/// The kind of game server a player is on.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LocationKind {
    /// Ramsgate
    Hub,
    /// A hunt.
    Hunt,
    /// The Training Grounds
    Training,
    /// The tutorial.
    Tutorial,
}

/// The game server a player is on, from the Director.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Location {
    /// The kind of game server.
    pub kind: LocationKind,
    /// The Director's instance id.
    pub instance_id: String,
    /// "Ramsgate", "Training Grounds", or the hunt's behemoth or island.
    pub title: String,
}

/// A player as the admin API shows them.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Player {
    /// The account id.
    pub user_id: String,
    /// The display name.
    pub username: String,
    /// The account's state.
    pub status: AccountStatus,
    /// Null when the player may play.
    pub restriction: Option<Restriction>,
    /// A chat connection, or a game that reported in within the last minute.
    pub online: bool,
    /// Null when the player is in no game server the Director knows of (the menus, or offline).
    pub location: Option<Location>,
    /// Holds the backend's admin role: kicks, timeouts and bans refuse such accounts. False in a game
    /// whose module doesn't report it; that module refuses to act on its own admins.
    pub admin: bool,
    /// When the account was made (RFC 3339), if the game reports it.
    pub created_at: Option<String>,
    /// The last login (RFC 3339), if the game reports it.
    pub last_login_at: Option<String>,
    /// The game version the player is in, such as 1.14.7.
    pub game: String,
}

/// How players reach the server.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum ServerMode {
    /// Reached over Tailscale.
    Private,
    /// Reached over the internet through the gateway's TLS port.
    Public,
}

/// What the server is and how it is doing.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct ServerStatus {
    /// The server's name.
    pub name: String,
    /// How players reach it.
    pub mode: ServerMode,
    /// INVITECODE, OPEN or NONE.
    pub registration: String,
    /// The server's version.
    pub version: String,
    /// The server's source commit; null when unknown.
    pub commit: Option<String>,
    /// Seconds since the backend started.
    pub uptime_seconds: u32,
    /// Players online now.
    pub online_count: u32,
    /// Whether chat is running.
    pub chat: bool,
    /// real, synthetic or missing.
    pub game_data: String,
}

/// A player in a hunt.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct HuntPlayer {
    /// The account id.
    pub user_id: String,
    /// Null when the backend has no account with that id.
    pub username: Option<String>,
}

/// A running hunt.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct Hunt {
    /// The Director's instance id.
    pub id: String,
    /// The title the Director gives it.
    pub title: String,
    /// The map's name.
    pub map: String,
    /// The behemoth, if the hunt has one.
    pub behemoth: Option<String>,
    /// Players in it now.
    pub player_count: u32,
    /// The most it takes.
    pub max_players: u32,
    /// Who is in it.
    pub players: Vec<HuntPlayer>,
    /// When it started, as the Director gives it.
    pub started_at: Option<String>,
}

/// The hunt servers and what they hold.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct HuntStatus {
    /// `max_hunt_servers` in host.toml.
    pub max_hunt_servers: u32,
    /// Hunt servers running.
    pub running: u32,
    /// Hunt servers that can still start.
    pub free: u32,
    /// The running hunts.
    pub hunts: Vec<Hunt>,
}

/// What a kick did.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KickResult {
    /// The account id.
    pub user_id: String,
    /// The display name.
    pub username: String,
    /// Chat connections closed; 0 when the player held none, or in a game whose module doesn't count them.
    pub connections_closed: u32,
    /// The game version the player is in, such as 1.14.7.
    pub game: String,
}

/// An admin key as listed: who it is for and what it may do, never the key itself.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct AdminKey {
    /// What the log and `admin:<id>` in the change log call it. `owner` is REFORGED_ADMIN_API_KEY.
    pub id: String,
    /// What the key is for, as its issuer named it.
    pub name: String,
    /// What it may do.
    pub role: Role,
    /// Null for the owner key, which setup made.
    pub created_at: Option<String>,
    /// The actor that issued it, as the change log writes it.
    pub created_by: String,
    /// Set once revoked: that key is refused.
    pub revoked_at: Option<String>,
    /// The actor that revoked it, as the change log writes it.
    pub revoked_by: Option<String>,
}

impl From<KeyInfo> for AdminKey {
    fn from(key: KeyInfo) -> Self {
        Self {
            id: key.id,
            name: key.name,
            role: key.role,
            created_at: Some(key.created_at).filter(|at| *at > 0).map(input::iso),
            created_by: key.created_by,
            revoked_at: key.revoked_at.map(input::iso),
            revoked_by: key.revoked_by,
        }
    }
}

/// A key just made. The key itself is shown here once and is not kept.
#[derive(SimpleObject, Clone)]
pub struct IssuedKey {
    /// `RRA_<id>_<secret>`: send it as `Authorization: Bearer <key>`. Keep it; it cannot be shown again.
    pub key: String,
    /// What the key may do, as listed.
    pub info: AdminKey,
}

// ------------------------------------------------------------------ history and revert (DH3b)

/// Which changes to look at: every field set must match. An actor is `admin:<key id>`, `host:console`,
/// `player:<account>` or `system:<task>` (a bare key id works too); times are RFC 3339, a date, or how long
/// ago (`6h`, `2d`).
#[derive(async_graphql::InputObject, Clone, Debug, Default)]
pub struct HistoryFilter {
    /// Rows that belong to this account id, before or after the change.
    pub account: Option<String>,
    /// Changes made by this actor.
    pub actor: Option<String>,
    /// From this time on, inclusive.
    pub since: Option<String>,
    /// Up to this time, exclusive.
    pub until: Option<String>,
}

impl HistoryFilter {
    fn selection(&self) -> history::Selection {
        history::Selection {
            account: self.account.clone(),
            actor: self.actor.clone(),
            since: self.since.clone(),
            until: self.until.clone(),
        }
    }
}

/// One change to one row, from a database's change log.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The log entry's id.
    pub id: String,
    /// `node` (the game), `admin` or `chat`.
    pub database: String,
    /// When the change was made (RFC 3339).
    pub at: String,
    /// Who made it: `player:<account>`, `admin:<key id>`, `host:<command>` or `system:<task>`.
    pub actor: String,
    /// The table the row is in.
    pub table: String,
    /// The row's primary key, as JSON.
    pub key: String,
    /// `insert`, `update`, `delete`, `soft_delete`, `restore` or `migrate`.
    pub op: String,
    /// The row before and after, as JSON; an admin key's hash is hidden.
    pub before: Option<String>,
    /// The row after the change, as JSON; an admin key's hash is hidden.
    pub after: Option<String>,
}

/// Changes newest first.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    /// The entries on this page.
    pub entries: Vec<HistoryEntry>,
    /// Pass it as `cursor` for the next, older page; null at the end.
    pub next: Option<String>,
}

/// What a revert does to one row.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum RevertKind {
    /// A deleted row, put back.
    Insert,
    /// A changed row, given its earlier values.
    Update,
    /// An added row, removed.
    Delete,
    /// An added row in a soft-delete table, soft-deleted.
    SoftDelete,
}

/// One row a revert would change.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RevertAction {
    /// `node`, `admin` or `chat`.
    pub database: String,
    /// The table the row is in.
    pub table: String,
    /// The row's primary key, as JSON.
    pub key: String,
    /// What the revert does to it.
    pub kind: RevertKind,
    /// The row now and as the revert leaves it, as JSON.
    pub current: Option<String>,
    /// The row as the revert leaves it, as JSON.
    pub target: Option<String>,
}

/// A row the revert leaves as it is.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RevertConflict {
    /// `node`, `admin` or `chat`.
    pub database: String,
    /// The table the row is in.
    pub table: String,
    /// The row's primary key, as JSON.
    pub key: String,
    /// `changed_since`, `not_as_logged`, `key_changed` or `schema_changed`.
    pub reason: String,
    /// `changed_since`: the first newer entry, who made it and when.
    pub entry: Option<String>,
    /// `changed_since`: who made the newer entry.
    pub by: Option<String>,
    /// `changed_since`: when (RFC 3339).
    pub at: Option<String>,
    /// More on the conflict, for the operator.
    pub detail: Option<String>,
}

/// A row the revert finds already as it would leave it.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RevertUnchanged {
    /// `node`, `admin` or `chat`.
    pub database: String,
    /// The table the row is in.
    pub table: String,
    /// The row's primary key, as JSON.
    pub key: String,
}

/// What a revert would do. `revertApply` with `id` does exactly this and nothing else.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RevertPlan {
    /// Pass it to `revertApply`.
    pub id: String,
    /// Log entries the selection matches.
    pub entries: i32,
    /// Rows the revert would change.
    pub action_count: i32,
    /// Rows it would leave because they changed since.
    pub conflict_count: i32,
    /// Rows already as it would leave them.
    pub unchanged_count: i32,
    /// The first rows of each list; `truncated` says there are more than listed.
    pub actions: Vec<RevertAction>,
    /// Rows it would leave, the first ones.
    pub conflicts: Vec<RevertConflict>,
    /// Rows already as it would leave them, the first ones.
    pub unchanged: Vec<RevertUnchanged>,
    /// Whether any list holds fewer rows than its count.
    pub truncated: bool,
    /// How long the server keeps this plan for `revertApply` without the selection.
    pub kept_for_seconds: i32,
}

/// Rows a revert changed in one database.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RevertedIn {
    /// `node`, `admin` or `chat`.
    pub database: String,
    /// Rows changed.
    pub rows: i32,
}

/// What a revert did.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct RevertDone {
    /// The plan carried out.
    pub plan_id: String,
    /// The revert's own entries carry this time and `admin:<key id>`.
    pub at: Option<String>,
    /// Rows changed in all.
    pub rows_changed: i32,
    /// Rows changed, by database.
    pub databases: Vec<RevertedIn>,
    /// Rows left as they are.
    pub conflicts: Vec<RevertConflict>,
    /// Rows left as they are, in all.
    pub conflict_count: i32,
    /// Whether `conflicts` holds fewer than `conflictCount`.
    pub truncated: bool,
}

/// One log's hash chain checked from its start (`history verify`).
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct HistoryCheck {
    /// `node`, `admin` or `chat`.
    pub database: String,
    /// Entries checked, as a decimal string.
    pub entries: String,
    /// Whether the whole chain holds.
    pub ok: bool,
    /// The newest entry's id.
    pub head_id: Option<String>,
    /// The newest entry's hash.
    pub head_hash: Option<String>,
    /// Where the chain breaks and what that means.
    pub broken: Option<String>,
}

const MAX_LISTED: usize = history::MAX_LISTED;

fn count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn conflict_of(database: &str, c: &history::RowConflict) -> RevertConflict {
    let f = history::conflict_fields(c);
    RevertConflict {
        database: database.to_owned(),
        table: c.table.clone(),
        key: c.key.to_string(),
        reason: f.reason.to_owned(),
        entry: f.entry,
        by: f.by,
        at: f.at,
        detail: f.detail,
    }
}

fn plan_of(plan: &history::Plan) -> RevertPlan {
    let mut out = RevertPlan {
        id: plan.id.clone(),
        entries: count(plan.entries()),
        action_count: count(plan.actions()),
        conflict_count: count(plan.conflicts()),
        unchanged_count: count(plan.unchanged()),
        actions: Vec::new(),
        conflicts: Vec::new(),
        unchanged: Vec::new(),
        truncated: plan.actions() > MAX_LISTED || plan.conflicts() > MAX_LISTED || plan.unchanged() > MAX_LISTED,
        kept_for_seconds: count(usize::try_from(history::PLAN_KEPT.as_secs()).unwrap_or(0)),
    };
    for part in &plan.parts {
        let room = |listed: usize| MAX_LISTED.saturating_sub(listed);
        for a in part.plan.actions.iter().take(room(out.actions.len())) {
            let kind = match a.kind {
                dainty_store::revert::RevertKind::Insert => RevertKind::Insert,
                dainty_store::revert::RevertKind::Update => RevertKind::Update,
                dainty_store::revert::RevertKind::Delete => RevertKind::Delete,
                dainty_store::revert::RevertKind::SoftDelete => RevertKind::SoftDelete,
            };
            out.actions.push(RevertAction {
                database: part.database.clone(),
                table: a.table.clone(),
                key: a.key.to_string(),
                kind,
                current: history::redacted_json(&a.table, a.current.as_ref()),
                target: history::redacted_json(&a.table, a.target.as_ref()),
            });
        }
        for c in part.plan.conflicts.iter().take(room(out.conflicts.len())) {
            out.conflicts.push(conflict_of(&part.database, c));
        }
        for u in part.plan.unchanged.iter().take(room(out.unchanged.len())) {
            out.unchanged.push(RevertUnchanged {
                database: part.database.clone(),
                table: u.table.clone(),
                key: u.key.to_string(),
            });
        }
    }
    out
}

/// A history failure as the caller sees it.
fn history_failure(error: history::HistoryError) -> async_graphql::Error {
    use history::HistoryError;
    match error {
        HistoryError::Bad(why) => fail(Code::BadInput, why),
        HistoryError::PlanChanged => fail(Code::PlanChanged, error.to_string()),
        HistoryError::NoDatabase => fail(Code::HistoryUnavailable, error.to_string()),
        HistoryError::Partial { .. } => fail(Code::PartialRevert, error.to_string()),
        HistoryError::Store(error) => {
            log::error("the history failed", json!({"error": error.to_string()}));
            fail(Code::HistoryUnavailable, "the history did not answer")
        }
    }
}

// ------------------------------------------------------------------ what the fields do

/// What the admin API's fields act on.
pub struct Admin {
    /// The backend and the Director over loopback.
    pub upstream: Upstream,
    /// The game modules that answer admin actions (DF4c); none until DF3's controller loads modules.
    pub modules: Arc<AdminPlugin>,
    /// The timeouts this API set and will lift.
    pub timeouts: Timeouts,
    /// The owner's key and the issued ones, and the record of what each did (DH2).
    pub keys: Keys,
    /// Destructive and bulk actions that wait for a delay or a second operator (DH6).
    pub guard: Guards,
    /// The change logs of this server's databases (DH3b).
    pub history: history::Service,
    /// Peer-to-peer admin relay, session ticket revocation and down-peer queue (DN11).
    pub relay: crate::relay::AdminRelay,
    /// `max_hunt_servers` in host.toml.
    pub max_hunt_servers: u32,
    /// How players reach the server.
    pub server_mode: config::ServerMode,
    /// Where every time comes from.
    pub clock: Clock,
}

const TIMEOUT_ENDED: &str = "Timeout ended";

fn status_of(account: &Account) -> AccountStatus {
    match account.status.as_str() {
        "active" => AccountStatus::Active,
        "banned" => AccountStatus::Banned,
        "disabled" => AccountStatus::Disabled,
        _ => AccountStatus::Unknown,
    }
}

fn location_of(kind: InstanceKind) -> LocationKind {
    match kind {
        InstanceKind::City => LocationKind::Hub,
        InstanceKind::Hunt => LocationKind::Hunt,
        InstanceKind::Dojo => LocationKind::Training,
        InstanceKind::Tutorial => LocationKind::Tutorial,
    }
}

/// The reason a timeout carries, so the player reads when it ends.
fn timeout_reason(until: i64, reason: &str) -> String {
    format!("Timed out until {}: {reason}", input::iso(until))
}

/// Where a player was found.
enum Found {
    /// In the Node backend's accounts: 1.14.7, until a module for it is loaded
    Node(Account),
    /// In a module's game
    Module(AdminModule, PlayerInfo),
}

impl Found {
    fn game(&self) -> String {
        match self {
            Self::Node(_) => NODE_GAME.to_owned(),
            Self::Module(module, _) => module.game_version.to_string(),
        }
    }
}

/// The account, unless it is one moderation must not touch.
fn protect(account: Account) -> async_graphql::Result<Account> {
    if account.roles.iter().any(|r| r == "admin") {
        return Err(fail(Code::Protected, "this player is a server admin"));
    }
    Ok(account)
}

/// A module's player in today's shape. What `PlayerInfo` doesn't carry (DF2) is UNKNOWN or null; a module that
/// reports no restriction may still have one, so its status stays UNKNOWN rather than ACTIVE.
fn module_player(module: &AdminModule, info: PlayerInfo) -> Player {
    let restriction = info.restriction.as_ref().map(|r| Restriction {
        kind: match r.kind {
            dainty_module_api::RestrictionKind::Ban => RestrictionKind::Ban,
            dainty_module_api::RestrictionKind::Timeout => RestrictionKind::Timeout,
            dainty_module_api::RestrictionKind::Disabled => RestrictionKind::Disabled,
        },
        reason: r.reason.clone(),
        since: r.since.clone(),
        by: None,
        until: r.until.clone(),
    });
    let status = match info.restriction.as_ref().map(|r| r.kind) {
        Some(dainty_module_api::RestrictionKind::Ban | dainty_module_api::RestrictionKind::Timeout) => {
            AccountStatus::Banned
        }
        Some(dainty_module_api::RestrictionKind::Disabled) => AccountStatus::Disabled,
        None => AccountStatus::Unknown,
    };
    Player {
        user_id: user_id_of(&info),
        username: info.name,
        status,
        restriction,
        online: info.online,
        location: None,
        admin: false,
        created_at: None,
        last_login_at: None,
        game: module.game_version.to_string(),
    }
}

/// The account's network-wide id, or the name for a game that gives none.
fn user_id_of(info: &PlayerInfo) -> String {
    info.account
        .as_ref()
        .map_or_else(|| info.name.clone(), ToString::to_string)
}

fn not_supported(module: &AdminModule, what: &str) -> async_graphql::Error {
    fail(
        Code::NotSupported,
        format!("{} can't {what} through the admin API yet", module.display_name),
    )
}

/// Refuses a key whose role is below `needs`, and says which role it needs (DH2). The log gets the key's
/// id, never the key.
pub fn allow(caller: &Caller, needs: Role, action: &str) -> async_graphql::Result<()> {
    if caller.role >= needs {
        return Ok(());
    }
    log::warn(
        "admin action refused: the key's role is too low",
        json!({"key": caller.key, "role": caller.role.as_str(), "needs": needs.as_str(),
               "action": action, "operator": caller.operator.id()}),
    );
    Err(async_graphql::Error::new(format!(
        "{action} needs the {needs} role; this key's role is {}",
        caller.role
    ))
    .extend_with(|_, e| {
        e.set("code", Code::Forbidden.as_str());
        e.set("requiredRole", needs.as_str());
    }))
}

/// A key store failure as the caller sees it.
fn key_failure(error: KeyError) -> async_graphql::Error {
    match error {
        KeyError::Invalid(why) => fail(Code::BadInput, why),
        KeyError::NameTaken(_) | KeyError::Owner => fail(Code::BadInput, error.to_string()),
        KeyError::NotFound => fail(Code::NotFound, error.to_string()),
        KeyError::Store(error) => {
            log::error("the admin API database failed", json!({"error": error.to_string()}));
            fail(Code::BackendUnavailable, "the admin API database did not answer")
        }
    }
}

impl Admin {
    pub(crate) fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Writes what `caller` did to the admin API database, under `admin:<key id>` in the change log (DH2).
    /// The action is done by now: a write that fails is logged with everything, not hidden by an error.
    pub(crate) async fn audit(
        &self,
        caller: &Caller,
        action: &str,
        target: Option<&str>,
        game: Option<&str>,
        detail: Option<&str>,
    ) {
        // Set while a held action runs: the second operator who approved it (DH6)
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            action,
            target,
            game,
            detail,
            approved_by.as_deref(),
            None,
            None,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments, reason = "each argument is one column of the audit row")]
    pub(crate) async fn audit_full(
        &self,
        key: &str,
        operator: &str,
        action: &str,
        target: Option<&str>,
        game: Option<&str>,
        detail: Option<&str>,
        approved_by: Option<&str>,
        via_node: Option<&str>,
        request_key: Option<&str>,
    ) {
        let entry = Action {
            operator,
            action,
            target,
            game,
            detail,
            approved_by,
            via_node,
            request_key,
        };
        if let Err(error) = self.keys.record(key, entry, self.now()).await {
            log::error(
                "could not record an admin action",
                json!({"key": key, "operator": operator, "action": action,
                       "target": target, "game": game, "detail": detail,
                       "viaNode": via_node, "error": error.to_string()}),
            );
        }
    }

    /// Builds a [`network_plugin::NodeService`] for `Service::Admin` backed by this [`Admin`] (DN11).
    pub fn relay_service(self: &Arc<Self>) -> Arc<network_plugin::relay::AdminRelayService> {
        let authority: Arc<dyn network_plugin::relay::AdminAuthority> = self.clone();
        Arc::new(self.relay.service(authority))
    }

    /// Queued admin actions waiting for down peers (DN11). Moderator role or higher.
    pub async fn pending_relay(&self, caller: &Caller) -> async_graphql::Result<Vec<crate::relay::PendingRelay>> {
        allow(caller, Role::Moderator, "pendingRelay")?;
        let pending = self.relay.pending().await.map_err(|error| {
            log::error("the admin API database failed", json!({"error": error.to_string()}));
            fail(Code::BackendUnavailable, "the admin API database did not answer")
        })?;
        Ok(pending.into_iter().map(crate::relay::PendingRelay::from).collect())
    }

    /// Every key, the owner first. Never a key or a hash.
    pub async fn admin_keys(&self, caller: &Caller) -> async_graphql::Result<Vec<AdminKey>> {
        allow(caller, Role::Admin, "adminKeys")?;
        let keys = self.keys.list().await.map_err(key_failure)?;
        Ok(keys.into_iter().map(AdminKey::from).collect())
    }

    /// A new key. An admin issues viewers and moderators; the owner issues any role. The key is in the answer
    /// once and nowhere else.
    pub async fn issue_key(&self, caller: &Caller, name: &str, role: Role) -> async_graphql::Result<IssuedKey> {
        allow(caller, Role::Admin, "issueKey")?;
        if role >= Role::Admin {
            allow(caller, Role::Owner, &format!("issueKey for the {role} role"))?;
        }
        let by = dainty_store::Actor::Admin(caller.key.clone());
        let (info, key) = self.keys.issue(name, role, by, self.now()).await.map_err(key_failure)?;
        log::info(
            "admin key issued",
            json!({"id": info.id, "name": info.name, "role": role.as_str(), "by": caller.key}),
        );
        Ok(IssuedKey { key, info: info.into() })
    }

    /// Ends a key: its next request is refused. Only the owner does it.
    pub async fn revoke_key(&self, caller: &Caller, id: &str) -> async_graphql::Result<AdminKey> {
        allow(caller, Role::Owner, "revokeKey")?;
        self.gate(caller, Gate::RevokeKey { id }).await?;
        self.revoke_key_now(caller, id).await
    }

    /// [`Admin::revoke_key`] once it is allowed to run: a held revoke runs here when its time comes (DH6).
    pub(crate) async fn revoke_key_now(&self, caller: &Caller, id: &str) -> async_graphql::Result<AdminKey> {
        let now = self.now();
        let by = dainty_store::Actor::Admin(caller.key.clone());
        let info = self.keys.revoke(id, by, now).await.map_err(key_failure)?;
        log::info(
            "admin key revoked",
            json!({"id": info.id, "name": info.name, "by": caller.key}),
        );
        if self.relay.in_network() {
            let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
            if let Ok(spec) = crate::relay::OutboundAction::new(
                crate::relay::operations::REVOKE_KEY,
                None,
                false,
                &caller.key,
                caller.operator.id(),
                approved_by.clone(),
                Some(info.id.clone()),
                None,
                json!({ "id": info.id }),
            ) {
                let pending = self.relay.fan_out(&spec, None, now).await;
                if !pending.is_empty() {
                    let detail = crate::relay::detail_with_pending(None, &pending);
                    self.audit_full(
                        &caller.key,
                        caller.operator.id(),
                        "revokeKey",
                        Some(&info.id),
                        None,
                        detail.as_deref(),
                        approved_by.as_deref(),
                        None,
                        Some(spec.key.as_str()),
                    )
                    .await;
                }
            }
        }
        Ok(info.into())
    }

    /// One page of the change logs, newest first. Admin role or higher: a row's before and after are in it.
    pub async fn history(
        &self,
        caller: &Caller,
        filter: Option<HistoryFilter>,
        cursor: Option<&str>,
        limit: Option<i32>,
    ) -> async_graphql::Result<HistoryPage> {
        allow(caller, Role::Admin, "history")?;
        let filter = filter
            .unwrap_or_default()
            .selection()
            .to_filter(self.now())
            .map_err(history_failure)?;
        let set = self.history.set().await.map_err(history_failure)?;
        let limit = i64::from(limit.unwrap_or(history::DEFAULT_PAGE as i32));
        let page = set.query(&filter, cursor, limit).await.map_err(history_failure)?;
        Ok(HistoryPage {
            entries: page
                .entries
                .into_iter()
                .map(|e| HistoryEntry {
                    id: e.entry.id.to_string(),
                    database: e.database,
                    at: input::iso(e.entry.at),
                    actor: e.entry.actor,
                    key: e.entry.row_key,
                    op: e.entry.op,
                    before: history::redacted_text(&e.entry.table_name, e.entry.before.as_deref()),
                    after: history::redacted_text(&e.entry.table_name, e.entry.after.as_deref()),
                    table: e.entry.table_name,
                })
                .collect(),
            next: page.next,
        })
    }

    /// Each log's hash chain walked from its start: an entry edited or removed is named (DH5).
    pub async fn history_check(&self, caller: &Caller) -> async_graphql::Result<Vec<HistoryCheck>> {
        allow(caller, Role::Admin, "historyCheck")?;
        let set = self.history.set().await.map_err(history_failure)?;
        let all = set.verify().await.map_err(history_failure)?;
        Ok(all
            .into_iter()
            .map(|v| HistoryCheck {
                database: v.database,
                entries: v.verification.entries.to_string(),
                ok: v.verification.broken.is_none(),
                head_id: v.verification.head.as_ref().map(|h| h.id.to_string()),
                head_hash: v.verification.head.map(|h| h.hash),
                broken: v.verification.broken.map(|b| b.to_string()),
            })
            .collect())
    }

    /// What reverting a selection would do, without changing anything. The plan is kept for a few minutes for
    /// `revertApply`. Only the owner: the plan carries rows.
    pub async fn revert_plan(&self, caller: &Caller, selection: HistoryFilter) -> async_graphql::Result<RevertPlan> {
        allow(caller, Role::Owner, "revertPlan")?;
        let filter = selection.selection().to_filter(self.now()).map_err(history_failure)?;
        let set = self.history.set().await.map_err(history_failure)?;
        let plan = set.dry_run(&filter).await.map_err(history_failure)?;
        self.history.plans.put(plan.clone());
        log::info(
            "revert planned",
            json!({"plan": plan.id, "by": caller.key, "rows": plan.actions(), "conflicts": plan.conflicts()}),
        );
        Ok(plan_of(&plan))
    }

    /// Runs a revert as `caller`: plans `filter` again and applies it only if the plan still has `plan_id`,
    /// logged as `admin:<key id>`. What a held revert does when its delay ends or a second operator approves
    /// (DH6), and what `revertApply` does at once under the limit.
    pub(crate) async fn revert_now(
        &self,
        caller: &Caller,
        filter: &dainty_store::Filter,
        plan_id: &str,
    ) -> async_graphql::Result<history::Applied> {
        let set = self.history.set().await.map_err(history_failure)?;
        let actor = dainty_store::Actor::Admin(caller.key.clone());
        let applied = set.apply(&actor, filter, plan_id).await.map_err(history_failure)?;
        let rows: usize = applied.done.iter().map(|(_, o)| o.plan.actions.len()).sum();
        log::info(
            "revert applied",
            json!({"plan": plan_id, "by": caller.key, "rows": rows, "conflicts": applied.plan.conflicts()}),
        );
        let detail = format!("{rows} rows; {} left as they are", applied.plan.conflicts());
        self.audit(caller, "revertApply", Some(plan_id), None, Some(&detail))
            .await;
        Ok(applied)
    }

    /// Applies a plan: exactly what `revertPlan` showed, or `PLAN_CHANGED` and nothing written. The plan is the
    /// one the server kept; if it has gone, give the same `selection` and it is made again and applied only if
    /// its id is `planId`. Only the owner. The revert is logged as `admin:<key id>` and can be reverted.
    pub async fn revert_apply(
        &self,
        caller: &Caller,
        plan_id: &str,
        selection: Option<HistoryFilter>,
    ) -> async_graphql::Result<RevertDone> {
        allow(caller, Role::Owner, "revertApply")?;
        let filter = match (self.history.plans.get(plan_id), selection) {
            (Some(plan), _) => plan.selection,
            (None, Some(selection)) => selection.selection().to_filter(self.now()).map_err(history_failure)?,
            (None, None) => {
                return Err(fail(
                    Code::PlanExpired,
                    "that plan is no longer kept: run revertPlan again, or give the same selection",
                ));
            }
        };
        let set = self.history.set().await.map_err(history_failure)?;
        // The plan as it is now: the id the caller was shown, or the same selection planned again
        let plan = set.dry_run(&filter).await.map_err(history_failure)?;
        if plan.id != plan_id {
            return Err(history_failure(history::HistoryError::PlanChanged));
        }
        let summary = format!(
            "revert {} changes selected by {}",
            plan.actions(),
            history_text::flags(&filter)
        );
        // A big revert is held for its delay or a second operator (DH6): the GUARDED error is the receipt
        self.gate(
            caller,
            Gate::Revert {
                selection: history::selection_json(&filter),
                plan_id: &plan.id,
                actions: plan.actions(),
                summary: &summary,
            },
        )
        .await?;
        let applied = self.revert_now(caller, &filter, plan_id).await?;
        self.history.plans.forget(plan_id);
        let rows: usize = applied.done.iter().map(|(_, o)| o.plan.actions.len()).sum();
        let shown = plan_of(&applied.plan);
        Ok(RevertDone {
            plan_id: plan_id.to_owned(),
            at: applied.done.first().map(|(_, o)| input::iso(o.at)),
            rows_changed: count(rows),
            databases: applied
                .done
                .iter()
                .map(|(database, o)| RevertedIn {
                    database: database.clone(),
                    rows: count(o.plan.actions.len()),
                })
                .collect(),
            conflicts: shown.conflicts,
            conflict_count: shown.conflict_count,
            truncated: shown.truncated,
        })
    }

    fn checked_name(name: &str) -> async_graphql::Result<&str> {
        input::username(name).ok_or_else(|| {
            fail(
                Code::BadInput,
                "username must be 3 to 16 letters, digits or underscores",
            )
        })
    }

    /// Where the player named `name` is: the Node backend's accounts (1.14.7, until a module for it is
    /// loaded) or a module's game. Every game that could hold them is asked at once, since a registration
    /// names no accounts. Two answering is ambiguous until the caller names the game; with `act`, so is a
    /// game that didn't answer, so a kick or ban never lands on a namesake while the right game is down.
    async fn locate(
        &self,
        caller: &Caller,
        name: &str,
        game: Option<&str>,
        act: bool,
    ) -> async_graphql::Result<Option<Found>> {
        let name = Self::checked_name(name)?;
        let wanted = match game {
            Some(given) => Some(
                input::game(given)
                    .ok_or_else(|| fail(Code::BadInput, "game must be a game version, such as 1.14.7"))?,
            ),
            None => None,
        };
        let is_wanted = |version: &str| wanted.as_deref().is_none_or(|w| w == version);
        let node = !self.modules.has_game(NODE_GAME) && is_wanted(NODE_GAME);
        let modules: Vec<AdminModule> = self
            .modules
            .modules()
            .into_iter()
            .filter(|m| is_wanted(&m.game_version.to_string()))
            .collect();
        if modules.is_empty() {
            if !node {
                return Err(fail(Code::UnknownGame, "this server runs no such game"));
            }
            // Today's path, unchanged while the Node backend is the only game
            let account = self
                .upstream
                .account_by_name(&caller.operator, name)
                .await
                .map_err(|e| upstream("backend", e))?;
            return Ok(account.map(Found::Node));
        }

        let mut asked = JoinSet::new();
        for module in modules {
            let plugin = self.modules.clone();
            let actor = self.modules.actor(&caller.key, self.now());
            let request = AdminRequest::Lookup {
                player: name.to_owned(),
            };
            asked.spawn(async move {
                let answer = plugin.call(&module, actor, request).await;
                (module, answer)
            });
        }
        let from_node = async {
            if node {
                Some(self.upstream.account_by_name(&caller.operator, name).await)
            } else {
                None
            }
        };
        let from_modules = async {
            let mut answers = Vec::new();
            while let Some(joined) = asked.join_next().await {
                if let Ok(answer) = joined {
                    answers.push(answer);
                }
            }
            answers.sort_by(|a, b| a.0.id.cmp(&b.0.id));
            answers
        };
        let (from_node, from_modules) = tokio::join!(from_node, from_modules);

        let mut found = Vec::new();
        let mut silent = Vec::new();
        match from_node {
            Some(Ok(Some(account))) => found.push(Found::Node(account)),
            Some(Ok(None)) | None => {}
            Some(Err(error)) => silent.push((String::from("the backend"), upstream("backend", error))),
        }
        for (module, answer) in from_modules {
            let error = match answer {
                Ok(AdminResponse::Player(info)) => {
                    found.push(Found::Module(module, info));
                    continue;
                }
                Ok(AdminResponse::NotFound) => continue,
                Err(error) if error.kind == ErrorKind::NotFound => continue,
                Ok(other) => unexpected(&other),
                Err(error) => error,
            };
            silent.push((module.display_name.clone(), module_failure(&module, error)));
        }
        if found.len() > 1 {
            let games: Vec<String> = found.iter().map(Found::game).collect();
            return Err(fail(
                Code::AmbiguousPlayer,
                format!(
                    "more than one game has a player with this name ({}); name the game",
                    games.join(", ")
                ),
            ));
        }
        if act && !found.is_empty() && !silent.is_empty() {
            let names: Vec<&str> = silent.iter().map(|(name, _)| name.as_str()).collect();
            return Err(fail(
                Code::BackendUnavailable,
                format!(
                    "{} did not answer, so another game may have a player with this name; try again, or name the game",
                    names.join(", ")
                ),
            ));
        }
        match found.pop() {
            Some(found) => Ok(Some(found)),
            None => match silent.into_iter().next() {
                Some((_, error)) => Err(error),
                None => Ok(None),
            },
        }
    }

    /// One kick or ban sent to a module, logged with the key and the operator id beside the request id the
    /// module's change log records (DH1).
    async fn module_act(
        &self,
        caller: &Caller,
        module: &AdminModule,
        request: AdminRequest,
    ) -> async_graphql::Result<()> {
        let actor = self.modules.actor(&caller.key, self.now());
        let request_id = actor.request.clone().unwrap_or_default();
        let sent = serde_json::to_value(&request).unwrap_or_default();
        match self.modules.call(module, actor, request).await {
            Ok(AdminResponse::Done) => {
                log::info(
                    "admin action done",
                    json!({"module": module.id.to_string(), "game": module.game_version.to_string(),
                           "action": sent, "key": caller.key, "operator": caller.operator.id(), "request": request_id}),
                );
                Ok(())
            }
            Ok(AdminResponse::NotFound) => Err(no_such_player()),
            Ok(other) => Err(module_failure(module, unexpected(&other))),
            Err(error) => Err(module_failure(module, error)),
        }
    }

    /// The player as their module reports them now, or as `info` had them when it doesn't answer.
    async fn player_now(&self, caller: &Caller, module: &AdminModule, info: PlayerInfo) -> Player {
        let request = AdminRequest::Lookup {
            player: info.name.clone(),
        };
        let actor = self.modules.actor(&caller.key, self.now());
        let latest = match self.modules.call(module, actor, request).await {
            Ok(AdminResponse::Player(latest)) => latest,
            _ => info,
        };
        module_player(module, latest)
    }

    async fn restriction(&self, account: &Account) -> Option<Restriction> {
        let kind = match status_of(account) {
            AccountStatus::Banned => {
                let timeout = self
                    .timeouts
                    .get(&account.user_id)
                    .await
                    .filter(|t| account.approval_reason.as_deref() == Some(t.reason.as_str()));
                if let Some(t) = timeout {
                    return Some(Restriction {
                        kind: RestrictionKind::Timeout,
                        reason: account.approval_reason.clone(),
                        since: account.approval_updated_at.clone(),
                        by: account.approval_updated_by.clone(),
                        until: Some(input::iso(t.until)),
                    });
                }
                RestrictionKind::Ban
            }
            AccountStatus::Disabled => RestrictionKind::Disabled,
            AccountStatus::Active | AccountStatus::Unknown => return None,
        };
        Some(Restriction {
            kind,
            reason: account.approval_reason.clone(),
            since: account.approval_updated_at.clone(),
            by: account.approval_updated_by.clone(),
            until: None,
        })
    }

    async fn player(&self, operator: &Operator, account: Account) -> async_graphql::Result<Player> {
        let online = self
            .upstream
            .online(operator)
            .await
            .map_err(|e| upstream("backend", e))?
            .iter()
            .any(|p| p.user_id == account.user_id);
        // Where a player is, is a nice-to-have: a Director that is down leaves it null
        let location = match self.upstream.instances().await {
            Ok(instances) => instances
                .into_iter()
                .find(|i| i.player_ids.contains(&account.user_id))
                .map(|i| Location {
                    kind: location_of(i.kind),
                    instance_id: i.id,
                    title: i.title,
                }),
            Err(error) => {
                log::warn("Director call failed", json!({"error": error.to_string()}));
                None
            }
        };
        Ok(Player {
            restriction: self.restriction(&account).await,
            status: status_of(&account),
            admin: account.roles.iter().any(|r| r == "admin"),
            user_id: account.user_id,
            username: account.display_name,
            online,
            location,
            created_at: account.created_at,
            last_login_at: account.last_login_at,
            game: NODE_GAME.to_owned(),
        })
    }

    /// A player by name, from whichever game has them.
    pub async fn find_player(
        &self,
        caller: &Caller,
        name: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Option<Player>> {
        match self.locate(caller, name, game, false).await? {
            None => Ok(None),
            Some(Found::Node(account)) => self.player(&caller.operator, account).await.map(Some),
            Some(Found::Module(module, info)) => Ok(Some(module_player(&module, info))),
        }
    }

    async fn set_access(
        &self,
        caller: &Caller,
        account: &Account,
        status: &str,
        reason: &str,
    ) -> async_graphql::Result<Account> {
        let updated = self
            .upstream
            .set_access(&caller.operator, &account.user_id, status, reason)
            .await
            .map_err(|e| upstream("backend", e))?;
        log::info(
            &format!("player is now {status}"),
            json!({"userId": updated.user_id, "username": updated.display_name, "key": caller.key, "operator": caller.operator.id(), "reason": reason}),
        );
        Ok(updated)
    }

    async fn forget_timeout(&self, user_id: &str) -> async_graphql::Result<()> {
        self.timeouts.remove(user_id).await.map_err(|e| {
            log::error("could not save the timeouts", json!({"error": e}));
            fail(
                Code::StateNotSaved,
                "the change was made, but the timeouts file could not be saved",
            )
        })
    }

    /// Where the player named `name` is in the network directory, if a directory is attached.
    async fn directory_target(
        &self,
        name: &str,
    ) -> (
        Option<dainty_module_api::network::GlobalId>,
        Option<dainty_module_api::network::NodeId>,
    ) {
        let Some(record) = self.relay.find_in_directory(name).await else {
            return (None, None);
        };
        let remote_home = match self.relay.local_node() {
            Some(local) if local != record.home => Some(record.home),
            _ => None,
        };
        (Some(record.account), remote_home)
    }

    fn peer_error(error: crate::relay::PeerCallError) -> async_graphql::Error {
        match error {
            crate::relay::PeerCallError::Refused { code, message } => {
                let mapped = match code.as_deref() {
                    Some("BAD_INPUT") => Code::BadInput,
                    Some("NOT_FOUND") => Code::NotFound,
                    Some("PROTECTED_ACCOUNT") => Code::Protected,
                    Some("ALREADY_BANNED") => Code::AlreadyBanned,
                    Some("NOT_BANNED") => Code::NotBanned,
                    Some("NOT_TIMED_OUT") => Code::NotTimedOut,
                    Some("STATE_NOT_SAVED") => Code::StateNotSaved,
                    Some("AMBIGUOUS_PLAYER") => Code::AmbiguousPlayer,
                    Some("UNKNOWN_GAME") => Code::UnknownGame,
                    Some("NOT_SUPPORTED") => Code::NotSupported,
                    Some("FORBIDDEN") => Code::Forbidden,
                    _ => Code::BackendRefused,
                };
                fail(mapped, message)
            }
            crate::relay::PeerCallError::Unreachable(why) => fail(Code::BackendUnavailable, why),
        }
    }

    /// Calls the player's remote home `home` first when `remote_home` is `Some`; if the home is down,
    /// queues the action for `home` and returns `Ok(None)` with `home` in the pending peer list.
    async fn call_home_and_fan_out(
        &self,
        spec: &crate::relay::OutboundAction,
        remote_home: Option<&dainty_module_api::network::NodeId>,
        now_ms: i64,
    ) -> async_graphql::Result<(Option<serde_json::Value>, Vec<dainty_module_api::network::NodeId>)> {
        let mut pending = Vec::new();
        let mut home_reply = None;
        if let Some(home) = remote_home {
            let wire = spec.to_wire().map_err(|why| fail(Code::BadInput, why))?;
            match self.relay.call_peer(home, &wire).await {
                Ok(value) => home_reply = Some(value),
                Err(crate::relay::PeerCallError::Unreachable(why)) => {
                    let _ = self.relay.enqueue(home, spec, &why, now_ms).await;
                    pending.push(home.clone());
                }
                Err(refused) => return Err(Self::peer_error(refused)),
            }
        }
        let mut rest = self.relay.fan_out(spec, remote_home, now_ms).await;
        pending.append(&mut rest);
        Ok((home_reply, pending))
    }

    /// Disconnects the player (on every node in a network) and ends their session tickets.
    pub async fn kick(&self, caller: &Caller, name: &str, game: Option<&str>) -> async_graphql::Result<KickResult> {
        allow(caller, Role::Moderator, "kick")?;
        let name = Self::checked_name(name)?;
        if !self.relay.in_network() {
            return self.kick_local(caller, name, game, None, None).await;
        }
        let now = self.now();
        let (account_id, remote_home) = self.directory_target(name).await;
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        let spec = crate::relay::OutboundAction::new(
            crate::relay::operations::KICK,
            account_id.clone(),
            true,
            &caller.key,
            caller.operator.id(),
            approved_by.clone(),
            Some(name.to_owned()),
            game.map(str::to_owned),
            json!({ "username": name, "game": game }),
        )
        .map_err(|why| fail(Code::BadInput, why))?;
        let (home_reply, pending) = self.call_home_and_fan_out(&spec, remote_home.as_ref(), now).await?;
        let local = self.kick_local_quiet(caller, name, game).await;
        let result = match (local, home_reply) {
            (Ok(local_res), _) => local_res,
            (Err(_), Some(value)) => serde_json::from_value::<KickResult>(value).unwrap_or_else(|_| KickResult {
                user_id: account_id.as_ref().map_or_else(|| name.to_owned(), ToString::to_string),
                username: name.to_owned(),
                connections_closed: 0,
                game: game.unwrap_or(NODE_GAME).to_owned(),
            }),
            (Err(_), None) if remote_home.is_some() && !pending.is_empty() => KickResult {
                user_id: account_id.as_ref().map_or_else(|| name.to_owned(), ToString::to_string),
                username: name.to_owned(),
                connections_closed: 0,
                game: game.unwrap_or(NODE_GAME).to_owned(),
            },
            (Err(err), None) => return Err(err),
        };
        if let Some(account) = &account_id {
            self.relay
                .revoke_tickets(account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let detail = crate::relay::detail_with_pending(None, &pending);
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "kick",
            Some(&result.username),
            Some(&result.game),
            detail.as_deref(),
            approved_by.as_deref(),
            None,
            Some(spec.key.as_str()),
        )
        .await;
        Ok(result)
    }

    async fn kick_local_quiet(
        &self,
        caller: &Caller,
        name: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<KickResult> {
        let account = match self
            .locate(caller, name, game, true)
            .await?
            .ok_or_else(no_such_player)?
        {
            Found::Node(account) => protect(account)?,
            Found::Module(module, info) => {
                let request = AdminRequest::Kick {
                    player: info.name.clone(),
                    reason: None,
                };
                self.module_act(caller, &module, request).await?;
                return Ok(KickResult {
                    user_id: user_id_of(&info),
                    username: info.name,
                    connections_closed: 0,
                    game: module.game_version.to_string(),
                });
            }
        };
        let kicked = self
            .upstream
            .kick(&account.display_name)
            .await
            .map_err(|e| upstream("backend", e))?;
        log::info(
            "player kicked",
            json!({"userId": kicked.user_id, "username": kicked.username, "key": caller.key, "operator": caller.operator.id(), "closed": kicked.closed}),
        );
        Ok(KickResult {
            user_id: kicked.user_id,
            username: kicked.username,
            connections_closed: kicked.closed,
            game: NODE_GAME.to_owned(),
        })
    }

    async fn kick_local(
        &self,
        caller: &Caller,
        name: &str,
        game: Option<&str>,
        via_node: Option<&str>,
        request_key: Option<&str>,
    ) -> async_graphql::Result<KickResult> {
        let result = self.kick_local_quiet(caller, name, game).await?;
        if let (Some(account), _) = self.directory_target(name).await {
            self.relay
                .revoke_tickets(&account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "kick",
            Some(&result.username),
            Some(&result.game),
            None,
            approved_by.as_deref(),
            via_node,
            request_key,
        )
        .await;
        Ok(result)
    }

    /// Bans the player; past the ban limit it waits in the guard (DH6). Relayed to every node in a network.
    pub async fn ban(
        &self,
        caller: &Caller,
        name: &str,
        reason: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        allow(caller, Role::Moderator, "ban")?;
        let reason = input::reason(reason).ok_or_else(|| fail(Code::BadInput, "reason must be 3 to 400 characters"))?;
        let name = Self::checked_name(name)?;
        self.gate(
            caller,
            Gate::Ban {
                name,
                reason: &reason,
                game,
            },
        )
        .await?;
        self.ban_now(caller, name, &reason, game).await
    }

    /// [`Admin::ban`] once it is allowed to run, with the reason already checked (DH6).
    pub(crate) async fn ban_now(
        &self,
        caller: &Caller,
        name: &str,
        reason: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        if !self.relay.in_network() {
            return self.ban_local(caller, name, reason, game, None, None).await;
        }
        let now = self.now();
        let (account_id, remote_home) = self.directory_target(name).await;
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        let spec = crate::relay::OutboundAction::new(
            crate::relay::operations::BAN,
            account_id.clone(),
            true,
            &caller.key,
            caller.operator.id(),
            approved_by.clone(),
            Some(name.to_owned()),
            game.map(str::to_owned),
            json!({ "username": name, "reason": reason, "game": game }),
        )
        .map_err(|why| fail(Code::BadInput, why))?;
        let (home_reply, pending) = self.call_home_and_fan_out(&spec, remote_home.as_ref(), now).await?;
        let local = self.ban_local_quiet(caller, name, reason, game).await;
        let player = match (local, home_reply) {
            (Ok(local_player), _) => local_player,
            (Err(_), Some(value)) => serde_json::from_value::<Player>(value).unwrap_or_else(|_| {
                Self::fallback_player(
                    account_id.as_ref(),
                    name,
                    AccountStatus::Banned,
                    Some(Restriction {
                        kind: RestrictionKind::Ban,
                        reason: Some(reason.to_owned()),
                        since: Some(input::iso(now)),
                        by: Some(format!("manager:{}", caller.operator.id())),
                        until: None,
                    }),
                    game,
                )
            }),
            (Err(_), None) if remote_home.is_some() && !pending.is_empty() => Self::fallback_player(
                account_id.as_ref(),
                name,
                AccountStatus::Banned,
                Some(Restriction {
                    kind: RestrictionKind::Ban,
                    reason: Some(reason.to_owned()),
                    since: Some(input::iso(now)),
                    by: Some(format!("manager:{}", caller.operator.id())),
                    until: None,
                }),
                game,
            ),
            (Err(err), None) => return Err(err),
        };
        if let Some(account) = &account_id {
            self.relay
                .revoke_tickets(account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let detail = crate::relay::detail_with_pending(Some(reason), &pending);
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "ban",
            Some(&player.username),
            Some(&player.game),
            detail.as_deref(),
            approved_by.as_deref(),
            None,
            Some(spec.key.as_str()),
        )
        .await;
        Ok(player)
    }

    fn fallback_player(
        account_id: Option<&dainty_module_api::network::GlobalId>,
        name: &str,
        status: AccountStatus,
        restriction: Option<Restriction>,
        game: Option<&str>,
    ) -> Player {
        Player {
            user_id: account_id.map_or_else(|| name.to_owned(), ToString::to_string),
            username: name.to_owned(),
            status,
            restriction,
            online: false,
            location: None,
            admin: false,
            created_at: None,
            last_login_at: None,
            game: game.unwrap_or(NODE_GAME).to_owned(),
        }
    }

    async fn ban_local_quiet(
        &self,
        caller: &Caller,
        name: &str,
        reason: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let reason = reason.to_owned();
        let account = match self
            .locate(caller, name, game, true)
            .await?
            .ok_or_else(no_such_player)?
        {
            Found::Node(account) => protect(account)?,
            Found::Module(module, info) => {
                let request = AdminRequest::Ban {
                    player: info.name.clone(),
                    reason: Some(reason.clone()),
                    until: None,
                };
                self.module_act(caller, &module, request).await?;
                return Ok(self.player_now(caller, &module, info).await);
            }
        };
        let updated = self.set_access(caller, &account, "banned", &reason).await?;
        self.forget_timeout(&updated.user_id).await?;
        self.player(&caller.operator, updated).await
    }

    async fn ban_local(
        &self,
        caller: &Caller,
        name: &str,
        reason: &str,
        game: Option<&str>,
        via_node: Option<&str>,
        request_key: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let player = self.ban_local_quiet(caller, name, reason, game).await?;
        if let (Some(account), _) = self.directory_target(name).await {
            self.relay
                .revoke_tickets(&account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "ban",
            Some(&player.username),
            Some(&player.game),
            Some(reason),
            approved_by.as_deref(),
            via_node,
            request_key,
        )
        .await;
        Ok(player)
    }

    /// Bans the player until `minutes` from now or `until`, then lifts it. Relayed to every node in a network.
    pub async fn timeout(
        &self,
        caller: &Caller,
        name: &str,
        minutes: Option<i64>,
        until: Option<&str>,
        reason: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        allow(caller, Role::Moderator, "timeout")?;
        let reason = input::reason(reason).ok_or_else(|| fail(Code::BadInput, "reason must be 3 to 400 characters"))?;
        let name = Self::checked_name(name)?;
        input::timeout_end(minutes, until, self.now()).map_err(|why| fail(Code::BadInput, why))?;
        self.gate(
            caller,
            Gate::Timeout {
                name,
                minutes,
                until,
                reason: &reason,
                game,
            },
        )
        .await?;
        self.timeout_now(caller, name, minutes, until, &reason, game).await
    }

    /// [`Admin::timeout`] once it is allowed to run, with the reason already checked. The end is worked out
    /// now, so a held `minutes` counts from when it runs (DH6).
    pub(crate) async fn timeout_now(
        &self,
        caller: &Caller,
        name: &str,
        minutes: Option<i64>,
        until: Option<&str>,
        reason: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let now = self.now();
        let end = input::timeout_end(minutes, until, now).map_err(|why| fail(Code::BadInput, why))?;
        if !self.relay.in_network() {
            return self.timeout_local(caller, name, end, reason, game, None, None).await;
        }
        let end_iso = input::iso(end);
        let full_reason = timeout_reason(end, reason);
        let (account_id, remote_home) = self.directory_target(name).await;
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        let spec = crate::relay::OutboundAction::new(
            crate::relay::operations::TIMEOUT,
            account_id.clone(),
            true,
            &caller.key,
            caller.operator.id(),
            approved_by.clone(),
            Some(name.to_owned()),
            game.map(str::to_owned),
            json!({ "username": name, "until": end_iso, "reason": reason, "game": game }),
        )
        .map_err(|why| fail(Code::BadInput, why))?;
        let (home_reply, pending) = self.call_home_and_fan_out(&spec, remote_home.as_ref(), now).await?;
        let local = self.timeout_local_quiet(caller, name, end, reason, game).await;
        let player = match (local, home_reply) {
            (Ok(local_player), _) => local_player,
            (Err(_), Some(value)) => serde_json::from_value::<Player>(value).unwrap_or_else(|_| {
                Self::fallback_player(
                    account_id.as_ref(),
                    name,
                    AccountStatus::Banned,
                    Some(Restriction {
                        kind: RestrictionKind::Timeout,
                        reason: Some(full_reason.clone()),
                        since: Some(input::iso(now)),
                        by: Some(format!("manager:{}", caller.operator.id())),
                        until: Some(end_iso.clone()),
                    }),
                    game,
                )
            }),
            (Err(_), None) if remote_home.is_some() && !pending.is_empty() => Self::fallback_player(
                account_id.as_ref(),
                name,
                AccountStatus::Banned,
                Some(Restriction {
                    kind: RestrictionKind::Timeout,
                    reason: Some(full_reason.clone()),
                    since: Some(input::iso(now)),
                    by: Some(format!("manager:{}", caller.operator.id())),
                    until: Some(end_iso),
                }),
                game,
            ),
            (Err(err), None) => return Err(err),
        };
        if let Some(account) = &account_id {
            self.relay
                .revoke_tickets(account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let detail = crate::relay::detail_with_pending(Some(&full_reason), &pending);
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "timeout",
            Some(&player.username),
            Some(&player.game),
            detail.as_deref(),
            approved_by.as_deref(),
            None,
            Some(spec.key.as_str()),
        )
        .await;
        Ok(player)
    }

    async fn timeout_local_quiet(
        &self,
        caller: &Caller,
        name: &str,
        end: i64,
        reason: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let now = self.now();
        let account = match self
            .locate(caller, name, game, true)
            .await?
            .ok_or_else(no_such_player)?
        {
            Found::Node(account) => protect(account)?,
            // The module keeps the end and lifts the ban itself, so timeouts.json holds Node bans only
            Found::Module(module, info) => {
                let request = AdminRequest::Ban {
                    player: info.name.clone(),
                    reason: Some(timeout_reason(end, reason)),
                    until: Some(input::iso(end)),
                };
                self.module_act(caller, &module, request).await?;
                return Ok(self.player_now(caller, &module, info).await);
            }
        };
        let timed_out = self.timeouts.get(&account.user_id).await.is_some();
        match status_of(&account) {
            AccountStatus::Active => {}
            AccountStatus::Banned if timed_out => {}
            // Lifting the timeout would also lift the ban or the backend's own disabling
            _ => {
                return Err(fail(
                    Code::AlreadyBanned,
                    "this player is already banned or disabled; unban them first",
                ));
            }
        }
        let full_reason = timeout_reason(end, reason);
        let updated = self.set_access(caller, &account, "banned", &full_reason).await?;
        let saved = self
            .timeouts
            .put(Timeout {
                user_id: updated.user_id.clone(),
                username: updated.display_name.clone(),
                until: end,
                reason: full_reason.clone(),
                by: caller.operator.id().to_owned(),
                at: now,
            })
            .await;
        if let Err(error) = saved {
            // Not remembered means never lifted: undo the ban rather than leave it permanent
            log::error("could not save the timeouts", json!({"error": error}));
            self.set_access(caller, &updated, "active", "Timeout could not be recorded")
                .await?;
            return Err(fail(
                Code::StateNotSaved,
                "the timeouts file could not be saved, so the timeout was undone",
            ));
        }
        self.player(&caller.operator, updated).await
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the mutation's inputs plus where the request came from"
    )]
    async fn timeout_local(
        &self,
        caller: &Caller,
        name: &str,
        end: i64,
        reason: &str,
        game: Option<&str>,
        via_node: Option<&str>,
        request_key: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let player = self.timeout_local_quiet(caller, name, end, reason, game).await?;
        if let (Some(account), _) = self.directory_target(name).await {
            self.relay
                .revoke_tickets(&account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let full_reason = timeout_reason(end, reason);
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "timeout",
            Some(&player.username),
            Some(&player.game),
            Some(&full_reason),
            approved_by.as_deref(),
            via_node,
            request_key,
        )
        .await;
        Ok(player)
    }

    /// Lifts a ban. Admin role only. Relayed to every node in a network.
    pub async fn unban(
        &self,
        caller: &Caller,
        name: &str,
        reason: Option<&str>,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        allow(caller, Role::Admin, "unban")?;
        let reason = match reason {
            Some(given) => {
                input::reason(given).ok_or_else(|| fail(Code::BadInput, "reason must be 3 to 400 characters"))?
            }
            None => "Ban lifted".into(),
        };
        let name = Self::checked_name(name)?;
        if !self.relay.in_network() {
            return self.unban_local(caller, name, &reason, game, None, None).await;
        }
        let now = self.now();
        let (account_id, remote_home) = self.directory_target(name).await;
        if let Some(account) = &account_id {
            self.relay
                .allow_tickets(account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        let spec = crate::relay::OutboundAction::new(
            crate::relay::operations::UNBAN,
            account_id.clone(),
            false,
            &caller.key,
            caller.operator.id(),
            approved_by.clone(),
            Some(name.to_owned()),
            game.map(str::to_owned),
            json!({ "username": name, "reason": reason, "game": game }),
        )
        .map_err(|why| fail(Code::BadInput, why))?;
        let (home_reply, pending) = self.call_home_and_fan_out(&spec, remote_home.as_ref(), now).await?;
        let local = self.unban_local_quiet(caller, name, &reason, game).await;
        let player = match (local, home_reply) {
            (Ok(local_player), _) => local_player,
            (Err(_), Some(value)) => serde_json::from_value::<Player>(value).unwrap_or_else(|_| {
                Self::fallback_player(account_id.as_ref(), name, AccountStatus::Active, None, game)
            }),
            (Err(_), None) if remote_home.is_some() && !pending.is_empty() => {
                Self::fallback_player(account_id.as_ref(), name, AccountStatus::Active, None, game)
            }
            (Err(err), None) => return Err(err),
        };
        let detail = crate::relay::detail_with_pending(Some(&reason), &pending);
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "unban",
            Some(&player.username),
            Some(&player.game),
            detail.as_deref(),
            approved_by.as_deref(),
            None,
            Some(spec.key.as_str()),
        )
        .await;
        Ok(player)
    }

    async fn unban_local_quiet(
        &self,
        caller: &Caller,
        name: &str,
        reason: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let account = match self
            .locate(caller, name, game, true)
            .await?
            .ok_or_else(no_such_player)?
        {
            Found::Node(account) => account,
            Found::Module(module, info) => {
                if !module.answers(dainty_module_api::AdminAction::Unban) {
                    return Err(not_supported(&module, "lift a ban"));
                }
                let request = AdminRequest::Unban {
                    player: info.name.clone(),
                    reason: Some(reason.to_owned()),
                };
                self.module_act(caller, &module, request).await?;
                return Ok(self.player_now(caller, &module, info).await);
            }
        };
        if status_of(&account) != AccountStatus::Banned {
            return Err(fail(Code::NotBanned, "this player is not banned"));
        }
        let updated = self.set_access(caller, &account, "active", reason).await?;
        self.forget_timeout(&updated.user_id).await?;
        self.player(&caller.operator, updated).await
    }

    async fn unban_local(
        &self,
        caller: &Caller,
        name: &str,
        reason: &str,
        game: Option<&str>,
        via_node: Option<&str>,
        request_key: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let player = self.unban_local_quiet(caller, name, reason, game).await?;
        if let (Some(account), _) = self.directory_target(name).await {
            self.relay
                .allow_tickets(&account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "unban",
            Some(&player.username),
            Some(&player.game),
            Some(reason),
            approved_by.as_deref(),
            via_node,
            request_key,
        )
        .await;
        Ok(player)
    }

    /// Lifts a timeout early. Admin role only. Relayed to every node in a network.
    pub async fn lift_timeout(&self, caller: &Caller, name: &str, game: Option<&str>) -> async_graphql::Result<Player> {
        allow(caller, Role::Admin, "liftTimeout")?;
        let name = Self::checked_name(name)?;
        if !self.relay.in_network() {
            return self.lift_timeout_local(caller, name, game, None, None).await;
        }
        let now = self.now();
        let (account_id, remote_home) = self.directory_target(name).await;
        if let Some(account) = &account_id {
            self.relay
                .allow_tickets(account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        let spec = crate::relay::OutboundAction::new(
            crate::relay::operations::LIFT_TIMEOUT,
            account_id.clone(),
            false,
            &caller.key,
            caller.operator.id(),
            approved_by.clone(),
            Some(name.to_owned()),
            game.map(str::to_owned),
            json!({ "username": name, "game": game }),
        )
        .map_err(|why| fail(Code::BadInput, why))?;
        let (home_reply, pending) = self.call_home_and_fan_out(&spec, remote_home.as_ref(), now).await?;
        let local = self.lift_timeout_local_quiet(caller, name, game).await;
        let player = match (local, home_reply) {
            (Ok(local_player), _) => local_player,
            (Err(_), Some(value)) => serde_json::from_value::<Player>(value).unwrap_or_else(|_| {
                Self::fallback_player(account_id.as_ref(), name, AccountStatus::Active, None, game)
            }),
            (Err(_), None) if remote_home.is_some() && !pending.is_empty() => {
                Self::fallback_player(account_id.as_ref(), name, AccountStatus::Active, None, game)
            }
            (Err(err), None) => return Err(err),
        };
        let detail = crate::relay::detail_with_pending(None, &pending);
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "liftTimeout",
            Some(&player.username),
            Some(&player.game),
            detail.as_deref(),
            approved_by.as_deref(),
            None,
            Some(spec.key.as_str()),
        )
        .await;
        Ok(player)
    }

    async fn lift_timeout_local_quiet(
        &self,
        caller: &Caller,
        name: &str,
        game: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let account = match self
            .locate(caller, name, game, true)
            .await?
            .ok_or_else(no_such_player)?
        {
            Found::Node(account) => account,
            Found::Module(module, info) => {
                if !module.answers(dainty_module_api::AdminAction::LiftTimeout) {
                    return Err(not_supported(&module, "lift a timeout"));
                }
                let request = AdminRequest::LiftTimeout {
                    player: info.name.clone(),
                    reason: Some("Timeout lifted".to_owned()),
                };
                self.module_act(caller, &module, request).await?;
                return Ok(self.player_now(caller, &module, info).await);
            }
        };
        let player = self.restriction(&account).await;
        if player.map(|r| r.kind) != Some(RestrictionKind::Timeout) {
            return Err(fail(Code::NotTimedOut, "this player is not timed out"));
        }
        let updated = self.set_access(caller, &account, "active", "Timeout lifted").await?;
        self.forget_timeout(&updated.user_id).await?;
        self.player(&caller.operator, updated).await
    }

    async fn lift_timeout_local(
        &self,
        caller: &Caller,
        name: &str,
        game: Option<&str>,
        via_node: Option<&str>,
        request_key: Option<&str>,
    ) -> async_graphql::Result<Player> {
        let player = self.lift_timeout_local_quiet(caller, name, game).await?;
        if let (Some(account), _) = self.directory_target(name).await {
            self.relay
                .allow_tickets(&account)
                .await
                .map_err(|error| fail(Code::StateNotSaved, error))?;
        }
        let approved_by = APPROVED_BY.try_with(Clone::clone).ok().flatten();
        self.audit_full(
            &caller.key,
            caller.operator.id(),
            "liftTimeout",
            Some(&player.username),
            Some(&player.game),
            None,
            approved_by.as_deref(),
            via_node,
            request_key,
        )
        .await;
        Ok(player)
    }

    /// Lifts the timeouts that have run out, and retries queued admin actions for peers whose links
    /// have recovered (DN11). One whose ban was changed since (lifted, or replaced by another ban)
    /// is only forgotten. A backend that does not answer is tried again next time.
    pub async fn sweep(&self) {
        let _ = self.relay.sweep().await;
        let operator = Operator::host();
        for due in self.timeouts.due(self.now()).await {
            let accounts = match self
                .upstream
                .accounts_by_ids(&operator, std::slice::from_ref(&due.user_id))
                .await
            {
                Ok(accounts) => accounts,
                Err(error) => {
                    log::warn(
                        "timeout not lifted yet",
                        json!({"userId": due.user_id, "error": error.to_string()}),
                    );
                    continue;
                }
            };
            if let Some(account) = accounts
                .first()
                .filter(|a| a.status == "banned" && a.approval_reason.as_deref() == Some(due.reason.as_str()))
                && let Err(error) = self
                    .upstream
                    .set_access(&operator, &account.user_id, "active", TIMEOUT_ENDED)
                    .await
            {
                log::warn(
                    "timeout not lifted yet",
                    json!({"userId": due.user_id, "error": error.to_string()}),
                );
                continue;
            }
            log::info(
                "timeout ended",
                json!({"userId": due.user_id, "username": due.username}),
            );
            if let Err(error) = self.timeouts.remove(&due.user_id).await {
                log::error("could not save the timeouts", json!({"error": error}));
            }
        }
    }

    /// The server's name, mode, version and who is online.
    pub async fn server_status(&self, caller: &Caller) -> async_graphql::Result<ServerStatus> {
        let status = self.upstream.status().await.map_err(|e| upstream("backend", e))?;
        let online = self
            .upstream
            .online(&caller.operator)
            .await
            .map_err(|e| upstream("backend", e))?;
        Ok(ServerStatus {
            name: status.name,
            mode: match self.server_mode {
                config::ServerMode::Private => ServerMode::Private,
                config::ServerMode::Public => ServerMode::Public,
            },
            registration: status.registration,
            version: status.version,
            commit: Some(status.commit).filter(|c| !c.is_empty()),
            uptime_seconds: u32::try_from(status.uptime_seconds).unwrap_or(u32::MAX),
            online_count: u32::try_from(online.len()).unwrap_or(u32::MAX),
            chat: status.chat,
            game_data: status.game_data,
        })
    }

    /// The running hunts and how many more may start.
    pub async fn hunt_status(&self, caller: &Caller) -> async_graphql::Result<HuntStatus> {
        let hunts: Vec<_> = self
            .upstream
            .instances()
            .await
            .map_err(|e| upstream("Director", e))?
            .into_iter()
            .filter(|i| i.kind == InstanceKind::Hunt)
            .collect();
        let mut ids: Vec<String> = hunts.iter().flat_map(|h| h.player_ids.iter().cloned()).collect();
        ids.sort();
        ids.dedup();
        let names: HashMap<String, String> = self
            .upstream
            .accounts_by_ids(&caller.operator, &ids)
            .await
            .map_err(|e| upstream("backend", e))?
            .into_iter()
            .map(|a| (a.user_id, a.display_name))
            .collect();
        let running = u32::try_from(hunts.len()).unwrap_or(u32::MAX);
        Ok(HuntStatus {
            max_hunt_servers: self.max_hunt_servers,
            running,
            free: self.max_hunt_servers.saturating_sub(running),
            hunts: hunts
                .into_iter()
                .map(|h| Hunt {
                    players: h
                        .player_ids
                        .iter()
                        .map(|id| HuntPlayer {
                            user_id: id.clone(),
                            username: names.get(id).cloned(),
                        })
                        .collect(),
                    id: h.id,
                    title: h.title,
                    map: h.map,
                    behemoth: h.behemoth,
                    player_count: h.players,
                    max_players: h.max_players,
                    started_at: h.started_at,
                })
                .collect(),
        })
    }
}

// ------------------------------------------------------------------ the GraphQL roots

fn parts<'a>(ctx: &'a Context<'_>) -> async_graphql::Result<(&'a Arc<Admin>, &'a Caller)> {
    Ok((ctx.data::<Arc<Admin>>()?, ctx.data::<Caller>()?))
}

/// The GraphQL query root.
pub struct Query;

#[Object]
impl Query {
    /// A player by username (case-insensitive), in whichever game on this server has them; null when
    /// there is no such account. `game` (a game version, such as 1.4.4) picks one when several games
    /// have the name, here and in every mutation.
    async fn player(
        &self,
        ctx: &Context<'_>,
        username: String,
        game: Option<String>,
    ) -> async_graphql::Result<Option<Player>> {
        let (admin, caller) = parts(ctx)?;
        admin.find_player(caller, &username, game.as_deref()).await
    }

    async fn server_status(&self, ctx: &Context<'_>) -> async_graphql::Result<ServerStatus> {
        let (admin, caller) = parts(ctx)?;
        admin.server_status(caller).await
    }

    /// The running hunt servers, who is in each, and how many more can start.
    async fn hunt_status(&self, ctx: &Context<'_>) -> async_graphql::Result<HuntStatus> {
        let (admin, caller) = parts(ctx)?;
        admin.hunt_status(caller).await
    }

    /// The timeouts this API will lift when they end, soonest first. Read from its own file: the
    /// backend is not asked, so a ban someone replaced since still shows until the sweep sees it.
    async fn active_timeouts(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<ActiveTimeout>> {
        let (admin, _caller) = parts(ctx)?;
        Ok(admin
            .timeouts
            .all()
            .await
            .into_iter()
            .map(|t| ActiveTimeout {
                username: t.username,
                until: input::iso(t.until),
                reason: t.reason,
                by: t.by,
                at: input::iso(t.at),
            })
            .collect())
    }

    /// Every game module and plugin host.toml lists, loaded or not (DF3).
    async fn modules(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Module>> {
        module_control::list(ctx)
    }

    /// One listed module; null when host.toml lists none by that id.
    async fn module(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<Option<Module>> {
        module_control::get(ctx, &id)
    }

    /// host.toml's `[logs]`, and whether the file changed since this run started. Needs the admin role.
    async fn log_settings(&self, ctx: &Context<'_>) -> async_graphql::Result<LogSettings> {
        let (_, caller) = parts(ctx)?;
        allow(caller, Role::Admin, "logSettings")?;
        host_settings::logs(ctx)
    }

    /// Every admin key, the owner first, with its role and who issued it. Admin role or higher. Never the
    /// key itself.
    async fn admin_keys(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<AdminKey>> {
        let (admin, caller) = parts(ctx)?;
        admin.admin_keys(caller).await
    }

    /// The destructive and bulk actions that are held: waiting for their delay to end or for a second
    /// operator (`approve`), oldest first; with `all`, the latest finished ones too, newest first. A held
    /// action was answered with the code GUARDED and its id. Moderator role or higher.
    async fn guarded(&self, ctx: &Context<'_>, all: Option<bool>) -> async_graphql::Result<Vec<GuardedAction>> {
        let (admin, caller) = parts(ctx)?;
        admin.guarded(caller, all.unwrap_or(false)).await
    }

    /// Admin actions waiting in the retry queue for unreachable peers, oldest first (DN11). Each row's
    /// `status` is `"pending on node <peer>"`. Moderator role or higher.
    async fn pending_relay(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<crate::relay::PendingRelay>> {
        let (admin, caller) = parts(ctx)?;
        admin.pending_relay(caller).await
    }

    /// Changes from the change logs, newest first: who changed what, with the row before and after. Pass the
    /// previous page's `next` as `cursor`; `limit` is 1 to 1000 (50). Admin role or higher.
    async fn history(
        &self,
        ctx: &Context<'_>,
        filter: Option<HistoryFilter>,
        cursor: Option<String>,
        limit: Option<i32>,
    ) -> async_graphql::Result<HistoryPage> {
        let (admin, caller) = parts(ctx)?;
        admin.history(caller, filter, cursor.as_deref(), limit).await
    }

    /// Walks each log's hash chain from its start and names the first entry edited or removed. Admin role or
    /// higher.
    async fn history_check(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<HistoryCheck>> {
        let (admin, caller) = parts(ctx)?;
        admin.history_check(caller).await
    }

    /// What reverting a selection (an actor, an account, a time range) would do, and what stays as it is
    /// because it changed since. Changes nothing; `revertApply` with the plan's `id` does what it shows.
    /// Owner role.
    async fn revert_plan(&self, ctx: &Context<'_>, selection: HistoryFilter) -> async_graphql::Result<RevertPlan> {
        let (admin, caller) = parts(ctx)?;
        admin.revert_plan(caller, selection).await
    }
}

/// The GraphQL mutation root.
pub struct Mutation;

#[Object]
impl Mutation {
    /// Drops a connected player now: their chat connections close and their sign-in tokens are
    /// revoked. They are not banned, and may log in again.
    async fn kick(
        &self,
        ctx: &Context<'_>,
        username: String,
        game: Option<String>,
    ) -> async_graphql::Result<KickResult> {
        let (admin, caller) = parts(ctx)?;
        admin.kick(caller, &username, game.as_deref()).await
    }

    /// Bans a player until a given time, then lifts the ban. Give `minutes` or `until` (RFC 3339),
    /// within a year. Replaces an earlier timeout; refused for a player who is banned.
    async fn timeout(
        &self,
        ctx: &Context<'_>,
        username: String,
        minutes: Option<i64>,
        until: Option<String>,
        reason: String,
        game: Option<String>,
    ) -> async_graphql::Result<Player> {
        let (admin, caller) = parts(ctx)?;
        admin
            .timeout(caller, &username, minutes, until.as_deref(), &reason, game.as_deref())
            .await
    }

    /// Lifts a timeout before it runs out.
    async fn lift_timeout(
        &self,
        ctx: &Context<'_>,
        username: String,
        game: Option<String>,
    ) -> async_graphql::Result<Player> {
        let (admin, caller) = parts(ctx)?;
        admin.lift_timeout(caller, &username, game.as_deref()).await
    }

    /// Bans a player until someone unbans them. Replaces a timeout. The player reads `reason` in the
    /// launcher.
    async fn ban(
        &self,
        ctx: &Context<'_>,
        username: String,
        reason: String,
        game: Option<String>,
    ) -> async_graphql::Result<Player> {
        let (admin, caller) = parts(ctx)?;
        admin.ban(caller, &username, &reason, game.as_deref()).await
    }

    /// Lifts a ban or a timeout.
    async fn unban(
        &self,
        ctx: &Context<'_>,
        username: String,
        reason: Option<String>,
        game: Option<String>,
    ) -> async_graphql::Result<Player> {
        let (admin, caller) = parts(ctx)?;
        admin.unban(caller, &username, reason.as_deref(), game.as_deref()).await
    }

    /// Loads a listed module (a failed one is restarted). Refused with MODULE_REFUSED and the reason: a
    /// second module for a game version, a SHA-256 that doesn't match, a module that failed to start.
    /// Needs the admin role.
    async fn load_module(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<Module> {
        let (admin, caller) = parts(ctx)?;
        allow(caller, Role::Admin, "loadModule")?;
        let module = module_control::load(ctx, &id).await?;
        admin.audit(caller, "loadModule", Some(&id), None, None).await;
        Ok(module)
    }

    /// Stops a module, its plugins first, and kills what it left running. Needs the admin role.
    async fn unload_module(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<Module> {
        let (admin, caller) = parts(ctx)?;
        allow(caller, Role::Admin, "unloadModule")?;
        let module = module_control::unload(ctx, &id).await?;
        admin.audit(caller, "unloadModule", Some(&id), None, None).await;
        Ok(module)
    }

    /// Writes host.toml's `[logs]`; the server takes it at its next start. Only the owner: a collector receives
    /// every line the server logs.
    async fn set_log_settings(&self, ctx: &Context<'_>, input: LogSettingsInput) -> async_graphql::Result<LogSettings> {
        let (admin, caller) = parts(ctx)?;
        allow(caller, Role::Owner, "setLogSettings")?;
        let after = host_settings::set_logs(ctx, &input)?;
        let detail = format!(
            "format={} color={} gelf_url={}",
            after.format,
            after.color,
            after.gelf_url.as_deref().unwrap_or("-")
        );
        admin
            .audit(caller, "setLogSettings", Some("logs"), None, Some(&detail))
            .await;
        Ok(after)
    }

    /// Needs the admin role.
    async fn restart_module(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<Module> {
        let (admin, caller) = parts(ctx)?;
        allow(caller, Role::Admin, "restartModule")?;
        let module = module_control::restart(ctx, &id).await?;
        admin.audit(caller, "restartModule", Some(&id), None, None).await;
        Ok(module)
    }

    /// Makes an admin key for one person (`name`) with a role. Needs the admin role, and an admin issues only
    /// viewer and moderator keys; the owner issues any. The key is in the answer once.
    async fn issue_key(&self, ctx: &Context<'_>, name: String, role: Role) -> async_graphql::Result<IssuedKey> {
        let (admin, caller) = parts(ctx)?;
        admin.issue_key(caller, &name, role).await
    }

    /// Ends an issued key: its next request is refused. Only the owner does it; the owner key is changed in
    /// secrets.env.
    async fn revoke_key(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<AdminKey> {
        let (admin, caller) = parts(ctx)?;
        admin.revoke_key(caller, &id).await
    }

    /// Approves a held action as a second operator: another key than the one that asked, with at least its
    /// role. It runs now as the key that asked, and the answer says how it went. A delay is never approved
    /// early. Moderator role or higher.
    async fn approve(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<GuardedAction> {
        let (admin, caller) = parts(ctx)?;
        admin.approve(caller, &id).await
    }

    /// Cancels a held action that has not run: the key that asked, or an owner.
    async fn cancel(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<GuardedAction> {
        let (admin, caller) = parts(ctx)?;
        admin.cancel(caller, &id).await
    }

    /// Applies the plan `revertPlan` showed: its rows are put back by compensating changes, logged as this
    /// key, and a row changed since stays as it is. `PLAN_CHANGED` when the history moved since the plan, with
    /// nothing written. Pass the same `selection` if the server no longer keeps the plan (PLAN_EXPIRED).
    /// Owner role.
    async fn revert_apply(
        &self,
        ctx: &Context<'_>,
        plan_id: String,
        selection: Option<HistoryFilter>,
    ) -> async_graphql::Result<RevertDone> {
        let (admin, caller) = parts(ctx)?;
        admin.revert_apply(caller, &plan_id, selection).await
    }
}

// ------------------------------------------------------------------ inbound peer relay (DN11)

fn to_relay_error(error: async_graphql::Error) -> network_plugin::relay::AdminRelayError {
    let code = error.extensions.as_ref().and_then(|ext| match ext.get("code") {
        Some(async_graphql::Value::String(text)) => Some(text.clone()),
        _ => None,
    });
    network_plugin::relay::AdminRelayError::new(code, error.message)
}

impl network_plugin::relay::AdminAuthority for Admin {
    fn apply<'a>(
        &'a self,
        action: network_plugin::relay::RelayedAction,
    ) -> dainty_module_api::BoxFuture<'a, Result<serde_json::Value, network_plugin::relay::AdminRelayError>> {
        Box::pin(async move {
            let via_node = action.via_node.as_str().to_owned();
            let req_key = action.key.as_str().to_owned();
            if self.keys.has_relayed(&via_node, &req_key).await.unwrap_or(false) {
                return Ok(json!({ "applied": true, "duplicate": true }));
            }
            let operator = Operator::parse(&action.operator).unwrap_or_else(Operator::host);
            let caller = Caller {
                key: action.actor_key,
                role: Role::Owner,
                operator,
            };
            let approved_by = action.approved_by;
            let body = &action.body;
            let text = |field: &str| body.get(field).and_then(serde_json::Value::as_str);
            let op = action.operation.as_str();

            match op {
                crate::relay::operations::KICK => {
                    let name = text("username").ok_or_else(|| {
                        network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), "missing username")
                    })?;
                    let game = text("game");
                    match self.kick_local_quiet(&caller, name, game).await {
                        Ok(kicked) => {
                            self.audit_full(
                                &caller.key,
                                caller.operator.id(),
                                "kick",
                                Some(&kicked.username),
                                Some(&kicked.game),
                                None,
                                approved_by.as_deref(),
                                Some(&via_node),
                                Some(&req_key),
                            )
                            .await;
                            Ok(serde_json::to_value(kicked).unwrap_or_else(|_| json!({ "applied": true })))
                        }
                        Err(err) => {
                            if is_not_found(&err) {
                                self.audit_full(
                                    &caller.key,
                                    caller.operator.id(),
                                    "kick",
                                    Some(name),
                                    game,
                                    None,
                                    approved_by.as_deref(),
                                    Some(&via_node),
                                    Some(&req_key),
                                )
                                .await;
                                return Ok(json!({ "applied": true }));
                            }
                            Err(to_relay_error(err))
                        }
                    }
                }
                crate::relay::operations::BAN => {
                    let name = text("username").ok_or_else(|| {
                        network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), "missing username")
                    })?;
                    let reason = text("reason").unwrap_or("Banned");
                    let game = text("game");
                    match self.ban_local_quiet(&caller, name, reason, game).await {
                        Ok(player) => {
                            self.audit_full(
                                &caller.key,
                                caller.operator.id(),
                                "ban",
                                Some(&player.username),
                                Some(&player.game),
                                Some(reason),
                                approved_by.as_deref(),
                                Some(&via_node),
                                Some(&req_key),
                            )
                            .await;
                            Ok(serde_json::to_value(player).unwrap_or_else(|_| json!({ "applied": true })))
                        }
                        Err(err) => {
                            if is_not_found(&err) {
                                self.audit_full(
                                    &caller.key,
                                    caller.operator.id(),
                                    "ban",
                                    Some(name),
                                    game,
                                    Some(reason),
                                    approved_by.as_deref(),
                                    Some(&via_node),
                                    Some(&req_key),
                                )
                                .await;
                                return Ok(json!({ "applied": true }));
                            }
                            Err(to_relay_error(err))
                        }
                    }
                }
                crate::relay::operations::TIMEOUT => {
                    let name = text("username").ok_or_else(|| {
                        network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), "missing username")
                    })?;
                    let until_str = text("until").ok_or_else(|| {
                        network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), "missing until")
                    })?;
                    let end = input::timeout_end(None, Some(until_str), self.now())
                        .map_err(|why| network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), why))?;
                    let reason = text("reason").unwrap_or("Timed out");
                    let game = text("game");
                    let full_reason = timeout_reason(end, reason);
                    match self.timeout_local_quiet(&caller, name, end, reason, game).await {
                        Ok(player) => {
                            self.audit_full(
                                &caller.key,
                                caller.operator.id(),
                                "timeout",
                                Some(&player.username),
                                Some(&player.game),
                                Some(&full_reason),
                                approved_by.as_deref(),
                                Some(&via_node),
                                Some(&req_key),
                            )
                            .await;
                            Ok(serde_json::to_value(player).unwrap_or_else(|_| json!({ "applied": true })))
                        }
                        Err(err) => {
                            if is_not_found(&err) {
                                self.audit_full(
                                    &caller.key,
                                    caller.operator.id(),
                                    "timeout",
                                    Some(name),
                                    game,
                                    Some(&full_reason),
                                    approved_by.as_deref(),
                                    Some(&via_node),
                                    Some(&req_key),
                                )
                                .await;
                                return Ok(json!({ "applied": true }));
                            }
                            Err(to_relay_error(err))
                        }
                    }
                }
                crate::relay::operations::UNBAN => {
                    let name = text("username").ok_or_else(|| {
                        network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), "missing username")
                    })?;
                    let reason = text("reason").unwrap_or("Ban lifted");
                    let game = text("game");
                    match self.unban_local_quiet(&caller, name, reason, game).await {
                        Ok(player) => {
                            self.audit_full(
                                &caller.key,
                                caller.operator.id(),
                                "unban",
                                Some(&player.username),
                                Some(&player.game),
                                Some(reason),
                                approved_by.as_deref(),
                                Some(&via_node),
                                Some(&req_key),
                            )
                            .await;
                            Ok(serde_json::to_value(player).unwrap_or_else(|_| json!({ "applied": true })))
                        }
                        Err(err) => {
                            if is_not_found(&err) {
                                self.audit_full(
                                    &caller.key,
                                    caller.operator.id(),
                                    "unban",
                                    Some(name),
                                    game,
                                    Some(reason),
                                    approved_by.as_deref(),
                                    Some(&via_node),
                                    Some(&req_key),
                                )
                                .await;
                                return Ok(json!({ "applied": true }));
                            }
                            Err(to_relay_error(err))
                        }
                    }
                }
                crate::relay::operations::LIFT_TIMEOUT => {
                    let name = text("username").ok_or_else(|| {
                        network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), "missing username")
                    })?;
                    let game = text("game");
                    match self.lift_timeout_local_quiet(&caller, name, game).await {
                        Ok(player) => {
                            self.audit_full(
                                &caller.key,
                                caller.operator.id(),
                                "liftTimeout",
                                Some(&player.username),
                                Some(&player.game),
                                None,
                                approved_by.as_deref(),
                                Some(&via_node),
                                Some(&req_key),
                            )
                            .await;
                            Ok(serde_json::to_value(player).unwrap_or_else(|_| json!({ "applied": true })))
                        }
                        Err(err) => {
                            if is_not_found(&err) {
                                self.audit_full(
                                    &caller.key,
                                    caller.operator.id(),
                                    "liftTimeout",
                                    Some(name),
                                    game,
                                    None,
                                    approved_by.as_deref(),
                                    Some(&via_node),
                                    Some(&req_key),
                                )
                                .await;
                                return Ok(json!({ "applied": true }));
                            }
                            Err(to_relay_error(err))
                        }
                    }
                }
                crate::relay::operations::REVOKE_KEY => {
                    let id = text("id").ok_or_else(|| {
                        network_plugin::relay::AdminRelayError::new(Some("BAD_INPUT"), "missing key id")
                    })?;
                    let by = dainty_store::Actor::Admin(caller.key.clone());
                    let _ = self.keys.revoke(id, by, self.now()).await;
                    self.audit_full(
                        &caller.key,
                        caller.operator.id(),
                        "revokeKey",
                        Some(id),
                        None,
                        None,
                        approved_by.as_deref(),
                        Some(&via_node),
                        Some(&req_key),
                    )
                    .await;
                    Ok(json!({ "applied": true, "id": id }))
                }
                _ => Err(network_plugin::relay::AdminRelayError::new(
                    Some("NOT_SUPPORTED"),
                    format!("unknown operation {op}"),
                )),
            }
        })
    }
}

fn is_not_found(error: &async_graphql::Error) -> bool {
    error
        .extensions
        .as_ref()
        .is_some_and(|ext| matches!(ext.get("code"), Some(async_graphql::Value::String(s)) if s == "NOT_FOUND"))
}
