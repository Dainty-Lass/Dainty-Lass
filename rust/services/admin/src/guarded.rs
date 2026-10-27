//! What is guarded and how a held action runs (DD23 layer 4, DH6). `guard.rs` is the store; this is the
//! admin API side of it: the one hook each guarded action calls (`Admin::gate`), the fields that list,
//! approve and cancel, and the running of an action as the key that asked, once.
//!
//! | Action | Held when | Rule |
//! | --- | --- | --- |
//! | `ban`, `timeout` | one key made more than `ban_limit` of them in `ban_window_minutes` | `ban_guard` |
//! | `revokeKey` | the key revoked is an admin or an owner key, and not the caller own | `revoke_guard` |
//! | history `revert` | the plan has more than `revert_limit` changes (DH3b calls `Admin::gate`) | `revert_guard` |
//!
//! Not guarded, on purpose: `kick` (nothing is lost), `unban` and `liftTimeout` (they give back), `loadModule`,
//! `unloadModule` and `restartModule` (a stopped game is started again with one call, and nothing is erased),
//! `issueKey` (it adds access, and only the owner issues the high roles). The owner is held too: its key is a
//! key like any other here, and the approver is a second owner key or the host kit.

use async_graphql::{ErrorExtensions, SimpleObject};
use serde_json::{Value, json};

use crate::guard::{GuardError, Held, Kind, Mode, New, Record, Rule, State, Who, is_id};
use crate::input;
use crate::keys::{Caller, Role};
use crate::log;
use crate::schema::{Admin, Code, allow, fail};
use crate::upstream::Operator;

tokio::task_local! {
    /// The second operator key while a held action runs, so the audit row of what it does names them.
    pub static APPROVED_BY: Option<String>;
}

/// The most records a `guarded` query returns.
const MAX_LIST: i64 = 100;

/// What a guarded action asks of [`Admin::gate`]: enough to decide, to describe it and to run it later.
pub enum Gate<'a> {
    Ban {
        name: &'a str,
        reason: &'a str,
        game: Option<&'a str>,
    },
    Timeout {
        name: &'a str,
        minutes: Option<i64>,
        until: Option<&'a str>,
        reason: &'a str,
        game: Option<&'a str>,
    },
    RevokeKey {
        id: &'a str,
    },
    /// DH3b: a revert whose dry run planned `actions` changes. `selection` is the filter as DH3b writes it
    /// back to run the plan again; `plan_id` is `RevertPlan::id`, which stays the same while nothing else
    /// changed, so the same plan asked twice waits once and a changed database refuses a stale plan.
    Revert {
        selection: Value,
        plan_id: &'a str,
        actions: usize,
        summary: &'a str,
    },
}

/// A guarded action as the API shows it. Never the parameters it runs with.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct GuardedAction {
    /// 16 hex digits: what `approve` and `cancel` take.
    pub id: String,
    /// `ban`, `timeout`, `revokeKey` or `revert`.
    pub kind: String,
    /// One line of what it will do.
    pub summary: String,
    pub state: State,
    pub mode: Mode,
    /// The key that asked: `owner` or a key id.
    pub initiated_by: String,
    /// The lowest role that may approve it.
    pub needs_role: Role,
    pub created_at: String,
    /// When a delay ends and the action runs by itself; null when it waits for an approval.
    pub release_at: Option<String>,
    /// When it is dropped if it has not run.
    pub expires_at: String,
    /// The second operator key (`host` for the host kit). The action ran as the key that asked.
    pub approved_by: Option<String>,
    pub canceled_by: Option<String>,
    pub ended_at: Option<String>,
    /// What it did, or why it did not: the action own refusal, or that the key that asked was revoked.
    pub result: Option<String>,
}

impl From<Record> for GuardedAction {
    fn from(r: Record) -> Self {
        Self {
            id: r.id,
            kind: r.kind.as_str().to_owned(),
            summary: r.summary,
            state: r.state,
            mode: r.mode,
            initiated_by: r.initiated_by,
            needs_role: r.needs_role,
            created_at: input::iso(r.created_at),
            release_at: r.release_at.map(input::iso),
            expires_at: input::iso(r.expires_at),
            approved_by: r.approved_by,
            canceled_by: r.canceled_by,
            ended_at: r.ended_at.map(input::iso),
            result: r.result,
        }
    }
}

fn store_failure(error: &GuardError) -> async_graphql::Error {
    log::error("the guarded actions failed", json!({"error": error.to_string()}));
    fail(Code::BackendUnavailable, "the admin API database did not answer")
}

/// A guard failure as the caller sees it.
fn guard_failure(error: GuardError) -> async_graphql::Error {
    match error {
        GuardError::NotFound => fail(Code::NotFound, error.to_string()),
        GuardError::NotWaiting(_) | GuardError::DelayOnly | GuardError::Expired => {
            fail(Code::NotWaiting, error.to_string())
        }
        GuardError::SameKey | GuardError::NotYours => fail(Code::Forbidden, error.to_string()),
        GuardError::RoleTooLow(role) => async_graphql::Error::new(error.to_string()).extend_with(|_, e| {
            e.set("code", Code::Forbidden.as_str());
            e.set("requiredRole", role.as_str());
        }),
        GuardError::Store(_) => store_failure(&error),
    }
}

/// The answer to an action that was held: not a failure, but not done. `extensions` carry what a bot needs.
fn held_error(kind: Kind, held: &Held) -> async_graphql::Error {
    let r = held.record();
    let already = matches!(held, Held::Already(_));
    let first = if already {
        format!("{} is already held as {}", kind.as_str(), r.id)
    } else {
        format!("{} is held as {}", kind.as_str(), r.id)
    };
    let rest = match (r.mode, r.release_at) {
        (Mode::Delay, Some(at)) => format!(
            ": it runs by itself at {}, unless it is canceled (cancel(id: \"{}\"))",
            input::iso(at),
            r.id
        ),
        _ => format!(
            ": it needs the approval of a second operator, another key with the {} role or higher \
             (approve(id: \"{}\")), or cancel(id: \"{}\")",
            r.needs_role, r.id, r.id
        ),
    };
    let (id, mode, release, expires, needs) = (
        r.id.clone(),
        if r.mode == Mode::Delay { "DELAY" } else { "APPROVAL" },
        r.release_at.map(input::iso),
        input::iso(r.expires_at),
        r.needs_role.as_str(),
    );
    async_graphql::Error::new(format!("{first}{rest}")).extend_with(move |_, e| {
        e.set("code", Code::Guarded.as_str());
        e.set("guardedId", id.clone());
        e.set("mode", mode);
        e.set("releaseAt", release.clone());
        e.set("expiresAt", expires.clone());
        e.set("needsRole", needs);
        e.set("already", already);
    })
}

fn checked_id(given: &str) -> async_graphql::Result<&str> {
    let id = given.trim();
    if is_id(id) {
        Ok(id)
    } else {
        Err(fail(
            Code::BadInput,
            "an id is 16 lower-case hex digits, as guarded lists it",
        ))
    }
}

fn database_down(error: &dyn std::fmt::Display) -> async_graphql::Error {
    log::error("the admin API database failed", json!({"error": error.to_string()}));
    fail(Code::BackendUnavailable, "the admin API database did not answer")
}

impl Admin {
    /// The hook every guarded action calls after its role check and its input checks, and before it does
    /// anything: `Ok` runs the action now; an error with code `GUARDED` means it was held instead, and the
    /// caller returns that error as it is.
    pub async fn gate(&self, caller: &Caller, what: Gate<'_>) -> async_graphql::Result<()> {
        let settings = self.guard.settings().clone();
        let operator = caller.operator.id();
        let (kind, rule, subject, summary, params) = match what {
            Gate::Ban { name, reason, game } => {
                if !self.over_ban_limit(caller).await? {
                    return Ok(());
                }
                let at = game.map_or(String::new(), |g| format!(" ({g})"));
                (
                    Kind::Ban,
                    settings.ban_guard,
                    format!("{}/{}", name.to_lowercase(), game.unwrap_or("")),
                    format!("ban {name}{at}: {reason}"),
                    json!({"name": name, "reason": reason, "game": game, "operator": operator}),
                )
            }
            Gate::Timeout {
                name,
                minutes,
                until,
                reason,
                game,
            } => {
                if !self.over_ban_limit(caller).await? {
                    return Ok(());
                }
                let at = game.map_or(String::new(), |g| format!(" ({g})"));
                let span = match (minutes, until) {
                    (Some(m), _) => format!("for {m} minutes"),
                    (_, Some(u)) => format!("until {u}"),
                    _ => String::new(),
                };
                (
                    Kind::Timeout,
                    settings.ban_guard,
                    format!("{}/{}", name.to_lowercase(), game.unwrap_or("")),
                    format!("time out {name}{at} {span}: {reason}"),
                    json!({"name": name, "minutes": minutes, "until": until, "reason": reason, "game": game,
                           "operator": operator}),
                )
            }
            Gate::RevokeKey { id } => {
                if id == caller.key || settings.revoke_guard == Rule::Off {
                    return Ok(());
                }
                let keys = self.keys.list().await.map_err(|e| database_down(&e))?;
                // Viewer and moderator keys go at once: a leaked one should not stay live for ten minutes
                let Some(target) = keys
                    .iter()
                    .find(|k| k.id == id && k.revoked_at.is_none() && k.role >= Role::Admin)
                else {
                    return Ok(());
                };
                (
                    Kind::RevokeKey,
                    settings.revoke_guard,
                    id.to_owned(),
                    format!("revoke the {} key {} ({})", target.role, target.id, target.name),
                    json!({"id": id, "operator": operator}),
                )
            }
            Gate::Revert {
                selection,
                plan_id,
                actions,
                summary,
            } => {
                if actions <= settings.revert_limit as usize {
                    return Ok(());
                }
                (
                    Kind::Revert,
                    settings.revert_guard,
                    plan_id.to_owned(),
                    summary.to_owned(),
                    json!({"selection": selection, "planId": plan_id, "actions": actions, "operator": operator}),
                )
            }
        };
        if rule == Rule::Off {
            return Ok(());
        }
        let new = New {
            kind,
            subject,
            summary,
            params,
            rule,
            initiated_by: caller.key.clone(),
            initiator_role: caller.role,
        };
        let held = self.guard.hold(new, self.now()).await.map_err(|e| store_failure(&e))?;
        if let Held::New(record) = &held {
            log::info(
                "admin action held",
                json!({"id": record.id, "kind": kind.as_str(), "delay": record.mode == Mode::Delay,
                       "key": caller.key, "operator": caller.operator.id()}),
            );
            self.audit(caller, "guardHold", Some(&record.id), None, Some(&record.summary))
                .await;
        }
        Err(held_error(kind, &held))
    }

    /// Whether the next ban or timeout from this key is one too many for the window.
    async fn over_ban_limit(&self, caller: &Caller) -> async_graphql::Result<bool> {
        let settings = self.guard.settings();
        if settings.ban_guard == Rule::Off {
            return Ok(false);
        }
        let since = self.now() - settings.window_ms();
        let made = self
            .keys
            .count_actions(&caller.key, &["ban", "timeout"], since)
            .await
            .map_err(|e| database_down(&e))?;
        Ok(made >= i64::from(settings.ban_limit))
    }

    /// The waiting actions (and with `all`, the latest finished ones). Moderator role or higher.
    pub async fn guarded(&self, caller: &Caller, all: bool) -> async_graphql::Result<Vec<GuardedAction>> {
        allow(caller, Role::Moderator, "guarded")?;
        let records = self.guard.list(all, MAX_LIST).await.map_err(guard_failure)?;
        Ok(records.into_iter().map(GuardedAction::from).collect())
    }

    /// The approval of a second operator: another key than the one that asked, with at least its role. The
    /// action runs now, as the key that asked, and the answer says how it went.
    pub async fn approve(&self, caller: &Caller, id: &str) -> async_graphql::Result<GuardedAction> {
        allow(caller, Role::Moderator, "approve")?;
        let id = checked_id(id)?;
        let record = self
            .guard
            .approve(id, Who::key(&caller.key, caller.role), self.now())
            .await
            .map_err(guard_failure)?;
        log::info(
            "admin action approved",
            json!({"id": record.id, "kind": record.kind.as_str(), "key": caller.key, "initiatedBy": record.initiated_by}),
        );
        self.audit(caller, "guardApprove", Some(&record.id), None, Some(&record.summary))
            .await;
        Ok(self.run_one(&record.id).await?.into())
    }

    /// Cancels a waiting action: the key that asked, or an owner.
    pub async fn cancel(&self, caller: &Caller, id: &str) -> async_graphql::Result<GuardedAction> {
        allow(caller, Role::Moderator, "cancel")?;
        let id = checked_id(id)?;
        let record = self
            .guard
            .cancel(id, Who::key(&caller.key, caller.role), self.now())
            .await
            .map_err(guard_failure)?;
        log::info(
            "admin action canceled",
            json!({"id": record.id, "kind": record.kind.as_str(), "key": caller.key, "initiatedBy": record.initiated_by}),
        );
        self.audit(caller, "guardCancel", Some(&record.id), None, Some(&record.summary))
            .await;
        Ok(record.into())
    }

    /// At start, before the API listens: an action the last run left half done is marked failed.
    pub async fn guard_start(&self) {
        match self.guard.recover(self.now()).await {
            Ok(0) => {}
            Ok(n) => log::warn(
                "guarded actions were running when the server stopped: marked failed, not run again",
                json!({"count": n}),
            ),
            Err(error) => log::error(
                "could not recover the guarded actions",
                json!({"error": error.to_string()}),
            ),
        }
    }

    /// Every sweep: drops what nobody approved in time, and runs what is ready (a delay that ended, an approval
    /// the host kit recorded while the server ran).
    pub async fn guard_sweep(&self) {
        let now = self.now();
        match self.guard.expire(now).await {
            Ok(expired) => {
                for record in expired {
                    log::info(
                        "guarded action expired",
                        json!({"id": record.id, "kind": record.kind.as_str(), "initiatedBy": record.initiated_by}),
                    );
                }
            }
            Err(error) => log::error("could not expire guarded actions", json!({"error": error.to_string()})),
        }
        let ready = match self.guard.ready(now).await {
            Ok(ready) => ready,
            Err(error) => {
                log::error(
                    "could not read the guarded actions",
                    json!({"error": error.to_string()}),
                );
                return;
            }
        };
        for id in ready {
            if let Err(error) = self.run_one(&id).await {
                log::error(
                    "could not run a guarded action",
                    json!({"id": id, "error": error.message}),
                );
            }
        }
    }

    /// Claims the action and runs it, once; whoever loses the claim gets the record as it is.
    async fn run_one(&self, id: &str) -> async_graphql::Result<Record> {
        let Some(record) = self.guard.claim(id, self.now()).await.map_err(|e| store_failure(&e))? else {
            return self.guard.get(id).await.map_err(guard_failure);
        };
        let outcome = self.execute(&record).await;
        log::info(
            "guarded action ran",
            json!({"id": record.id, "kind": record.kind.as_str(), "initiatedBy": record.initiated_by,
                   "approvedBy": record.approved_by, "ok": outcome.is_ok()}),
        );
        self.guard
            .finish(&record, outcome, self.now())
            .await
            .map_err(|e| store_failure(&e))?;
        self.guard.get(id).await.map_err(guard_failure)
    }

    /// The action as the key that asked, if that key is still active and still has the role.
    async fn execute(&self, record: &Record) -> Result<String, String> {
        let role = match self.keys.active_role(&record.initiated_by).await {
            Ok(Some(role)) => role,
            Ok(None) => return Err("the key that asked was revoked before this ran".to_owned()),
            Err(error) => return Err(format!("the keys could not be read: {error}")),
        };
        if role < record.kind.base_role() {
            return Err(format!(
                "the key that asked no longer has the {} role this needs",
                record.kind.base_role()
            ));
        }
        let operator = record
            .params
            .get("operator")
            .and_then(Value::as_str)
            .and_then(Operator::parse)
            .unwrap_or_else(Operator::host);
        let caller = Caller {
            key: record.initiated_by.clone(),
            role,
            operator,
        };
        let text = |name: &str| record.params.get(name).and_then(Value::as_str);
        let (name, reason, game) = (text("name").unwrap_or(""), text("reason").unwrap_or(""), text("game"));
        let run = async {
            match record.kind {
                Kind::Ban => self
                    .ban_now(&caller, name, reason, game)
                    .await
                    .map(|p| format!("banned {}", p.username)),
                Kind::Timeout => self
                    .timeout_now(
                        &caller,
                        name,
                        record.params.get("minutes").and_then(Value::as_i64),
                        text("until"),
                        reason,
                        game,
                    )
                    .await
                    .map(|p| format!("timed out {}", p.username)),
                Kind::RevokeKey => self
                    .revoke_key_now(&caller, text("id").unwrap_or(""))
                    .await
                    .map(|k| format!("revoked key {}", k.id)),
                Kind::Revert => self.run_revert(&caller, &record.params).await,
            }
        };
        APPROVED_BY
            .scope(record.approved_by.clone(), run)
            .await
            .map_err(|e| e.message)
    }

    /// DH3b: runs a held revert as `caller`. It plans `params["selection"]` again, refuses if the plan id is no
    /// longer `params["planId"]` (something changed since it was held), and applies it as the actor of
    /// `caller`. Not wired until DH3b gives the admin API the node database.
    #[allow(clippy::unused_async)]
    async fn run_revert(&self, _caller: &Caller, _params: &Value) -> async_graphql::Result<String> {
        Err(fail(
            Code::NotSupported,
            "history revert is not available through the admin API yet",
        ))
    }
}
