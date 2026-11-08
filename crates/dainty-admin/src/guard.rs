//! Guarded actions (DD23 layer 4, DH6): a destructive or bulk admin action does not run when it is asked.
//! It is written here and waits, for a delay to end or for a second operator's approval, and runs once.
//! Anyone who started it, or any owner, can cancel it while it waits. This is the store: the rules and the
//! state machine. `guarded.rs` decides what is guarded and runs the held action as the key that asked.
//!
//! Every write is `Database::write` under the key that made it, so the change log names who held, approved,
//! canceled and ran each one (`admin:<key id>`, or `host:admin guarded <command>` from the host kit). Each
//! transition is one conditional `UPDATE ... WHERE state = ...`: whoever changes the row first wins, so an
//! action is claimed, and runs, at most once. One the server stopped in the middle of is marked failed at the
//! next start and never run again: whether it took effect is for an operator to check.

use async_graphql::Enum;
use dainty_store::{Actor, AnyConnection, Database};
use diesel::prelude::*;
use rand::RngExt;
use serde_json::Value;

use crate::keys::Role;

diesel::table! {
    // 16 columns, Diesel's default width
    /// Held actions, one row per action.
    guarded_actions (id) {
        /// 16 lower-case hex digits.
        id -> Text,
        /// The kind, as [`crate::guard::Kind::as_str`] names it.
        kind -> Text,
        /// What it acts on: one action per kind and subject waits at a time.
        subject -> Text,
        /// One line for people.
        summary -> Text,
        /// What running it needs, as JSON. Never a key.
        params -> Text,
        /// `delay` or `approval`.
        mode -> Text,
        /// The state, as [`crate::guard::State::as_str`] names it.
        state -> Text,
        /// The role an approver needs.
        needs_role -> Text,
        /// The key that asked.
        initiated_by -> Text,
        /// Unix milliseconds.
        created_at -> BigInt,
        /// Unix milliseconds: when a delayed action runs.
        release_at -> Nullable<BigInt>,
        /// Unix milliseconds: when a waiting action expires.
        expires_at -> BigInt,
        /// The key that approved it.
        approved_by -> Nullable<Text>,
        /// The key, or `host`, that canceled it.
        canceled_by -> Nullable<Text>,
        /// Unix milliseconds: when it finished.
        ended_at -> Nullable<BigInt>,
        /// What running it did, or why it failed.
        result -> Nullable<Text>,
    }
}

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
/// The most characters of a summary or a result kept.
const MAX_TEXT: usize = 400;

// ------------------------------------------------------------------ settings

/// What a rule does with an action that crosses its threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Never held: the action runs at once, as before DH6.
    Off,
    /// Held for `delay_minutes`, then run. Cancelable meanwhile.
    Delay,
    /// Held until a second operator approves, up to `expire_hours`.
    Approval,
}

impl Rule {
    /// The rule a host.toml value names: `off`, `delay` or `approval`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "off" => Some(Self::Off),
            "delay" => Some(Self::Delay),
            "approval" => Some(Self::Approval),
            _ => None,
        }
    }

    /// The host.toml value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Delay => "delay",
            Self::Approval => "approval",
        }
    }

    fn mode(self) -> Option<Mode> {
        match self {
            Self::Off => None,
            Self::Delay => Some(Mode::Delay),
            Self::Approval => Some(Mode::Approval),
        }
    }
}

/// host.toml `[guard]`, as `admin.env` carries it. The defaults leave everyday moderation immediate: ten bans
/// or timeouts an hour from one key, a revert of up to a hundred changes, and the revoking of a viewer or
/// moderator key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardSettings {
    /// Bans and timeouts one key may make in `ban_window_minutes` before the next one is held.
    pub ban_limit: u32,
    /// The window `ban_limit` counts in, in minutes.
    pub ban_window_minutes: u32,
    /// What happens to the ban or timeout over the limit.
    pub ban_guard: Rule,
    /// A revert of more compensating changes than this is held.
    pub revert_limit: u32,
    /// What happens to a revert over the limit.
    pub revert_guard: Rule,
    /// Revoking an admin or owner key.
    pub revoke_guard: Rule,
    /// How long a held action waits when its rule is `delay`.
    pub delay_minutes: u32,
    /// How long one nobody approved (or that the server was down for) stays before it expires.
    pub expire_hours: u32,
}

impl Default for GuardSettings {
    fn default() -> Self {
        Self {
            ban_limit: 10,
            ban_window_minutes: 60,
            ban_guard: Rule::Delay,
            revert_limit: 100,
            revert_guard: Rule::Delay,
            revoke_guard: Rule::Delay,
            delay_minutes: 10,
            expire_hours: 48,
        }
    }
}

impl GuardSettings {
    /// The highest `ban_limit` or `revert_limit` accepted.
    pub const MAX_LIMIT: u32 = 1_000_000;
    /// The longest `ban_window_minutes` accepted.
    pub const MAX_WINDOW_MINUTES: u32 = 24 * 60;
    /// The longest `delay_minutes` accepted.
    pub const MAX_DELAY_MINUTES: u32 = 24 * 60;
    /// The longest `expire_hours` accepted.
    pub const MAX_EXPIRE_HOURS: u32 = 30 * 24;

    /// `delay_minutes` in milliseconds.
    pub fn delay_ms(&self) -> i64 {
        i64::from(self.delay_minutes) * MINUTE_MS
    }

    /// `ban_window_minutes` in milliseconds.
    pub fn window_ms(&self) -> i64 {
        i64::from(self.ban_window_minutes) * MINUTE_MS
    }

    fn expire_ms(&self) -> i64 {
        i64::from(self.expire_hours) * HOUR_MS
    }
}

// ------------------------------------------------------------------ what is held

/// The guarded actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A ban past the limit.
    Ban,
    /// A timeout past the limit.
    Timeout,
    /// Revoking an admin or owner key.
    RevokeKey,
    /// A revert past the limit.
    Revert,
}

impl Kind {
    /// The name the database and the API use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ban => "ban",
            Self::Timeout => "timeout",
            Self::RevokeKey => "revokeKey",
            Self::Revert => "revert",
        }
    }

    /// The kind [`Kind::as_str`] names.
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Ban, Self::Timeout, Self::RevokeKey, Self::Revert]
            .into_iter()
            .find(|k| k.as_str() == text)
    }

    /// The role the action needs on its own: the approver needs it too, and the initiator must still have it
    /// when the action runs.
    pub fn base_role(self) -> Role {
        match self {
            Self::Ban | Self::Timeout => Role::Moderator,
            Self::RevokeKey | Self::Revert => Role::Owner,
        }
    }
}

/// How a held action is released.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq)]
#[graphql(name = "GuardMode")]
pub enum Mode {
    /// Waits `delay_minutes`, then runs by itself.
    Delay,
    /// Waits for a second operator.
    Approval,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delay => "delay",
            Self::Approval => "approval",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "delay" => Some(Self::Delay),
            "approval" => Some(Self::Approval),
            _ => None,
        }
    }
}

/// Where a held action is.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq)]
#[graphql(name = "GuardedState")]
pub enum State {
    /// Waiting for its delay to end or for an approval.
    Pending,
    /// Approved; the server runs it within seconds.
    Approved,
    /// Being run now.
    Running,
    /// It ran and succeeded.
    Done,
    /// It ran and the action itself refused or failed, or the server stopped while it ran: see `result`.
    Failed,
    /// An operator canceled it.
    Canceled,
    /// Nobody approved it in time, or the server was down past its time.
    Expired,
}

impl State {
    /// The name the database uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Expired => "expired",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Approved,
            Self::Running,
            Self::Done,
            Self::Failed,
            Self::Canceled,
            Self::Expired,
        ]
        .into_iter()
        .find(|s| s.as_str() == text)
    }
}

/// One guarded action as held.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// 16 lower-case hex digits.
    pub id: String,
    /// What kind of action it is.
    pub kind: Kind,
    /// What it acts on: one action per kind and subject waits at a time.
    pub subject: String,
    /// One line for people: `ban Bo_Slayer (1.14.7): griefing hunts`.
    pub summary: String,
    /// What running it needs, and `operator`, the Discord user it is for. Never a key.
    pub params: Value,
    /// How it is released.
    pub mode: Mode,
    /// Where it is.
    pub state: State,
    /// The role an approver needs.
    pub needs_role: Role,
    /// The key that asked: `owner` or an issued key's id.
    pub initiated_by: String,
    /// Unix milliseconds.
    pub created_at: i64,
    /// Unix milliseconds: when a delayed action runs.
    pub release_at: Option<i64>,
    /// Unix milliseconds: when a waiting action expires.
    pub expires_at: i64,
    /// The key that approved it.
    pub approved_by: Option<String>,
    /// The key, or `host`, that canceled it.
    pub canceled_by: Option<String>,
    /// Unix milliseconds: when it finished.
    pub ended_at: Option<i64>,
    /// What running it did, or why it failed.
    pub result: Option<String>,
}

#[derive(Queryable)]
struct Row {
    id: String,
    kind: String,
    subject: String,
    summary: String,
    params: String,
    mode: String,
    state: String,
    needs_role: String,
    initiated_by: String,
    created_at: i64,
    release_at: Option<i64>,
    expires_at: i64,
    approved_by: Option<String>,
    canceled_by: Option<String>,
    ended_at: Option<i64>,
    result: Option<String>,
}

impl Row {
    fn record(self) -> Option<Record> {
        Some(Record {
            kind: Kind::parse(&self.kind)?,
            mode: Mode::parse(&self.mode)?,
            state: State::parse(&self.state)?,
            needs_role: Role::parse(&self.needs_role)?,
            params: serde_json::from_str(&self.params).ok()?,
            id: self.id,
            subject: self.subject,
            summary: self.summary,
            initiated_by: self.initiated_by,
            created_at: self.created_at,
            release_at: self.release_at,
            expires_at: self.expires_at,
            approved_by: self.approved_by,
            canceled_by: self.canceled_by,
            ended_at: self.ended_at,
            result: self.result,
        })
    }
}

/// An action to hold.
#[derive(Debug, Clone)]
pub struct New {
    /// What kind of action it is.
    pub kind: Kind,
    /// What it acts on: an action already waiting for the same kind and subject is answered instead.
    pub subject: String,
    /// One line for people, clipped to 400 characters.
    pub summary: String,
    /// What running it needs. Never a key.
    pub params: Value,
    /// Delay or approval; `off` is refused.
    pub rule: Rule,
    /// The key that asked.
    pub initiated_by: String,
    /// That key's role: an approver needs at least this and the action's own.
    pub initiator_role: Role,
}

/// Whether [`Guards::hold`] wrote a record, or the same action was already waiting.
#[derive(Debug, Clone, PartialEq)]
pub enum Held {
    /// A new record was written.
    New(Record),
    /// The same action was already waiting.
    Already(Record),
}

impl Held {
    /// The record, new or not.
    pub fn record(&self) -> &Record {
        match self {
            Self::New(r) | Self::Already(r) => r,
        }
    }
}

/// Who approves or cancels: a key, or the host kit (`key` is `host`, an owner's right on this machine).
#[derive(Debug, Clone)]
pub struct Who {
    /// The key's id, or `host`.
    pub key: String,
    /// Its role.
    pub role: Role,
    /// The actor the change log records.
    pub actor: Actor,
}

impl Who {
    /// The key the host kit acts as.
    pub const HOST: &'static str = "host";

    /// An admin key.
    pub fn key(key: &str, role: Role) -> Self {
        Self {
            key: key.to_owned(),
            role,
            actor: Actor::Admin(key.to_owned()),
        }
    }

    /// The host kit's command: it has the machine, so it is an owner no key can be the same as.
    pub fn host(command: &str) -> Self {
        Self {
            key: Self::HOST.to_owned(),
            role: Role::Owner,
            actor: Actor::Host(format!("admin guarded {command}")),
        }
    }
}

/// Why a guarded action could not be held, approved or canceled.
#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    /// No action has that id.
    #[error("no such guarded action")]
    NotFound,
    /// It is no longer waiting; holds the state it is in.
    #[error("this action is {0}, not waiting")]
    NotWaiting(&'static str),
    /// It waits out a delay and cannot be approved.
    #[error("this action waits out its delay: it runs by itself, or cancel it")]
    DelayOnly,
    /// The key that started it cannot approve it.
    #[error("a second operator must approve this action: it was started by this key")]
    SameKey,
    /// Approving it needs a higher role; holds the one needed.
    #[error("approving this action needs the {0} role")]
    RoleTooLow(Role),
    /// It waited past `expires_at`.
    #[error("this action has expired")]
    Expired,
    /// Only its initiator or an owner may cancel it.
    #[error("only the key that started this action, or an owner, may cancel it")]
    NotYours,
    /// The database failed.
    #[error("{0}")]
    Store(#[from] dainty_store::Error),
}

impl From<diesel::result::Error> for GuardError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Store(error.into())
    }
}

fn clip(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

fn new_id() -> String {
    let mut bytes = [0u8; 8];
    rand::rng().fill(&mut bytes[..]);
    hex::encode(bytes)
}

/// An id as `new_id` makes it: 16 lower-case hex digits.
pub fn is_id(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn load(conn: &mut AnyConnection, id: &str) -> Result<Record, GuardError> {
    let row: Option<Row> = guarded_actions::table.find(id).first(conn).optional()?;
    row.and_then(Row::record).ok_or(GuardError::NotFound)
}

const OPEN: [&str; 3] = ["pending", "approved", "running"];
const WAITING: [&str; 2] = ["pending", "approved"];

// ------------------------------------------------------------------ the store

/// The held actions, in the admin API's database.
#[derive(Clone)]
pub struct Guards {
    db: Database,
    settings: std::sync::Arc<GuardSettings>,
}

impl Guards {
    /// The held actions in `db`, under `settings`.
    pub fn new(db: Database, settings: GuardSettings) -> Self {
        Self {
            db,
            settings: settings.into(),
        }
    }

    /// The `[guard]` settings in force.
    pub fn settings(&self) -> &GuardSettings {
        &self.settings
    }

    /// Holds an action under the key that asked. The same action (same kind and subject) that is still waiting
    /// is not held twice: the first one is answered.
    pub async fn hold(&self, new: New, now: i64) -> Result<Held, GuardError> {
        let Some(mode) = new.rule.mode() else {
            return Err(GuardError::NotWaiting("not guarded"));
        };
        let delay = (mode == Mode::Delay).then(|| now + self.settings.delay_ms());
        let expires_at = delay.unwrap_or(now) + self.settings.expire_ms();
        let record = Record {
            id: new_id(),
            kind: new.kind,
            subject: new.subject,
            summary: clip(&new.summary),
            params: new.params,
            mode,
            state: State::Pending,
            needs_role: new.initiator_role.max(new.kind.base_role()),
            initiated_by: new.initiated_by.clone(),
            created_at: now,
            release_at: delay,
            expires_at,
            approved_by: None,
            canceled_by: None,
            ended_at: None,
            result: None,
        };
        let row = (
            guarded_actions::id.eq(record.id.clone()),
            guarded_actions::kind.eq(record.kind.as_str()),
            guarded_actions::subject.eq(record.subject.clone()),
            guarded_actions::summary.eq(record.summary.clone()),
            guarded_actions::params.eq(record.params.to_string()),
            guarded_actions::mode.eq(mode.as_str()),
            guarded_actions::state.eq(State::Pending.as_str()),
            guarded_actions::needs_role.eq(record.needs_role.as_str()),
            guarded_actions::initiated_by.eq(record.initiated_by.clone()),
            guarded_actions::created_at.eq(now),
            guarded_actions::release_at.eq(delay),
            guarded_actions::expires_at.eq(expires_at),
        );
        let (kind, subject) = (record.kind.as_str(), record.subject.clone());
        self.db
            .write(Actor::Admin(new.initiated_by), move |tx| {
                let open: Option<Row> = guarded_actions::table
                    .filter(guarded_actions::kind.eq(kind))
                    .filter(guarded_actions::subject.eq(&subject))
                    .filter(guarded_actions::state.eq_any(OPEN))
                    .first(tx.conn())
                    .optional()?;
                if let Some(existing) = open.and_then(Row::record) {
                    return Ok(Held::Already(existing));
                }
                diesel::insert_into(guarded_actions::table)
                    .values(row)
                    .execute(tx.conn())?;
                Ok::<_, GuardError>(Held::New(record))
            })
            .await
    }

    /// One action by id.
    pub async fn get(&self, id: &str) -> Result<Record, GuardError> {
        let id = id.to_owned();
        self.db.read(move |conn: &mut AnyConnection| load(conn, &id)).await
    }

    /// The waiting actions, oldest first; with `all`, the finished ones too, newest first, up to `limit`.
    pub async fn list(&self, all: bool, limit: i64) -> Result<Vec<Record>, GuardError> {
        self.db
            .read(move |conn: &mut AnyConnection| {
                let rows: Vec<Row> = if all {
                    guarded_actions::table
                        .order((guarded_actions::created_at.desc(), guarded_actions::id.desc()))
                        .limit(limit)
                        .load(conn)?
                } else {
                    guarded_actions::table
                        .filter(guarded_actions::state.eq_any(WAITING))
                        .order((guarded_actions::created_at.asc(), guarded_actions::id.asc()))
                        .limit(limit)
                        .load(conn)?
                };
                Ok::<_, GuardError>(rows.into_iter().filter_map(Row::record).collect())
            })
            .await
    }

    /// A second operator approves. The approver is another key than the one that started the action, and has
    /// at least its role (and the action's own); a delay is never approved early.
    pub async fn approve(&self, id: &str, by: Who, now: i64) -> Result<Record, GuardError> {
        let id = id.to_owned();
        self.db
            .write(by.actor.clone(), move |tx| {
                let record = load(tx.conn(), &id)?;
                if record.state != State::Pending {
                    return Err(GuardError::NotWaiting(record.state.as_str()));
                }
                if record.mode != Mode::Approval {
                    return Err(GuardError::DelayOnly);
                }
                if record.expires_at <= now {
                    return Err(GuardError::Expired);
                }
                if record.initiated_by == by.key {
                    return Err(GuardError::SameKey);
                }
                if by.role < record.needs_role {
                    return Err(GuardError::RoleTooLow(record.needs_role));
                }
                let changed = diesel::update(
                    guarded_actions::table
                        .filter(guarded_actions::id.eq(&id))
                        .filter(guarded_actions::state.eq(State::Pending.as_str())),
                )
                .set((
                    guarded_actions::state.eq(State::Approved.as_str()),
                    guarded_actions::approved_by.eq(Some(by.key.clone())),
                ))
                .execute(tx.conn())?;
                if changed == 0 {
                    return Err(GuardError::NotWaiting("no longer waiting"));
                }
                load(tx.conn(), &id)
            })
            .await
    }

    /// Cancels a waiting action: the key that started it, or an owner.
    pub async fn cancel(&self, id: &str, by: Who, now: i64) -> Result<Record, GuardError> {
        let id = id.to_owned();
        self.db
            .write(by.actor.clone(), move |tx| {
                let record = load(tx.conn(), &id)?;
                if !matches!(record.state, State::Pending | State::Approved) {
                    return Err(GuardError::NotWaiting(record.state.as_str()));
                }
                if record.initiated_by != by.key && by.role < Role::Owner {
                    return Err(GuardError::NotYours);
                }
                let changed = diesel::update(
                    guarded_actions::table
                        .filter(guarded_actions::id.eq(&id))
                        .filter(guarded_actions::state.eq_any(WAITING)),
                )
                .set((
                    guarded_actions::state.eq(State::Canceled.as_str()),
                    guarded_actions::canceled_by.eq(Some(by.key.clone())),
                    guarded_actions::ended_at.eq(Some(now)),
                ))
                .execute(tx.conn())?;
                if changed == 0 {
                    return Err(GuardError::NotWaiting("already running"));
                }
                load(tx.conn(), &id)
            })
            .await
    }

    /// The ids ready to run: approved, or a delay that has ended. Not yet claimed.
    pub async fn ready(&self, now: i64) -> Result<Vec<String>, GuardError> {
        self.db
            .read(move |conn: &mut AnyConnection| {
                Ok::<_, GuardError>(
                    guarded_actions::table
                        .filter(guarded_actions::expires_at.gt(now))
                        .filter(
                            guarded_actions::state
                                .eq(State::Approved.as_str())
                                .or(guarded_actions::state
                                    .eq(State::Pending.as_str())
                                    .and(guarded_actions::mode.eq(Mode::Delay.as_str()))
                                    .and(guarded_actions::release_at.le(now))),
                        )
                        .order((guarded_actions::created_at.asc(), guarded_actions::id.asc()))
                        .select(guarded_actions::id)
                        .load(conn)?,
                )
            })
            .await
    }

    /// Takes the action to run it, as the key that started it. Exactly one caller gets `Some`: the row moves to
    /// `running` in a conditional update, and nothing moves it back.
    pub async fn claim(&self, id: &str, now: i64) -> Result<Option<Record>, GuardError> {
        let record = match self.get(id).await {
            Ok(record) => record,
            Err(GuardError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        let id = id.to_owned();
        self.db
            .write(Actor::Admin(record.initiated_by), move |tx| {
                let ready = guarded_actions::table
                    .filter(guarded_actions::id.eq(&id))
                    .filter(guarded_actions::expires_at.gt(now))
                    .filter(
                        guarded_actions::state
                            .eq(State::Approved.as_str())
                            .or(guarded_actions::state
                                .eq(State::Pending.as_str())
                                .and(guarded_actions::mode.eq(Mode::Delay.as_str()))
                                .and(guarded_actions::release_at.le(now))),
                    );
                let changed = diesel::update(ready)
                    .set(guarded_actions::state.eq(State::Running.as_str()))
                    .execute(tx.conn())?;
                if changed == 0 {
                    return Ok::<_, GuardError>(None);
                }
                load(tx.conn(), &id).map(Some)
            })
            .await
    }

    /// Records how a claimed action ended.
    pub async fn finish(&self, record: &Record, outcome: Result<String, String>, now: i64) -> Result<(), GuardError> {
        let (state, text) = match outcome {
            Ok(text) => (State::Done, text),
            Err(text) => (State::Failed, text),
        };
        let id = record.id.clone();
        self.db
            .write(Actor::Admin(record.initiated_by.clone()), move |tx| {
                diesel::update(
                    guarded_actions::table
                        .filter(guarded_actions::id.eq(&id))
                        .filter(guarded_actions::state.eq(State::Running.as_str())),
                )
                .set((
                    guarded_actions::state.eq(state.as_str()),
                    guarded_actions::ended_at.eq(Some(now)),
                    guarded_actions::result.eq(Some(clip(&text))),
                ))
                .execute(tx.conn())?;
                Ok::<_, GuardError>(())
            })
            .await
    }

    /// Expires what nobody approved in time. Returns the records it expired.
    pub async fn expire(&self, now: i64) -> Result<Vec<Record>, GuardError> {
        self.db
            .write(Actor::System("guard expiry".into()), move |tx| {
                let rows: Vec<Row> = guarded_actions::table
                    .filter(guarded_actions::state.eq_any(WAITING))
                    .filter(guarded_actions::expires_at.le(now))
                    .load(tx.conn())?;
                let mut expired = Vec::new();
                for row in rows.into_iter().filter_map(Row::record) {
                    let changed = diesel::update(
                        guarded_actions::table
                            .filter(guarded_actions::id.eq(&row.id))
                            .filter(guarded_actions::state.eq(row.state.as_str())),
                    )
                    .set((
                        guarded_actions::state.eq(State::Expired.as_str()),
                        guarded_actions::ended_at.eq(Some(now)),
                        guarded_actions::result.eq(Some("not approved in time")),
                    ))
                    .execute(tx.conn())?;
                    if changed == 1 {
                        expired.push(row);
                    }
                }
                Ok::<_, GuardError>(expired)
            })
            .await
    }

    /// At start: an action left running by a server that stopped is marked failed, never run again.
    pub async fn recover(&self, now: i64) -> Result<usize, GuardError> {
        self.db
            .write(Actor::System("guard recovery".into()), move |tx| {
                Ok::<_, GuardError>(
                    diesel::update(guarded_actions::table.filter(guarded_actions::state.eq(State::Running.as_str())))
                        .set((
                            guarded_actions::state.eq(State::Failed.as_str()),
                            guarded_actions::ended_at.eq(Some(now)),
                            guarded_actions::result.eq(Some(INTERRUPTED)),
                        ))
                        .execute(tx.conn())?,
                )
            })
            .await
    }
}

const INTERRUPTED: &str = "the server stopped while this ran: check whether it took effect, then ask again";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_is_one_of_three_words() {
        for rule in [Rule::Off, Rule::Delay, Rule::Approval] {
            assert_eq!(Rule::parse(rule.as_str()), Some(rule));
        }
        for bad in ["", "Delay", "on", "wait"] {
            assert_eq!(Rule::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn ids_are_sixteen_hex_digits() {
        assert!(is_id(&new_id()));
        assert_ne!(new_id(), new_id());
        for bad in ["", "abc", "ABCDEF0123456789", "0123456789abcdeg", "0123456789abcdef0"] {
            assert!(!is_id(bad), "{bad}");
        }
    }

    #[test]
    fn the_defaults_leave_everyday_actions_alone() {
        let d = GuardSettings::default();
        assert_eq!((d.ban_limit, d.ban_window_minutes, d.revert_limit), (10, 60, 100));
        assert_eq!(d.delay_ms(), 600_000);
        assert!(
            [d.ban_guard, d.revert_guard, d.revoke_guard]
                .iter()
                .all(|r| *r == Rule::Delay)
        );
    }
}
