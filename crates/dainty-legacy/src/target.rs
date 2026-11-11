// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 1.14.7 database rustic-isles made (spec §4): checked, then written in one transaction as
//! `host:legacy-import`, so the change log can revert an import (DD23). A dry run rolls the same writes back.

use std::collections::BTreeMap;
use std::time::Duration;

use dainty_store::{Actor, AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, MultiBackend};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use diesel_migrations::EmbeddedMigrations;

use crate::error::ImportError;
use crate::report::Skipped;
use crate::source::{Account, Invite, Now, format_ms};

/// rustic-isles' soft-deleted tables, so the store puts back the triggers the backend runs with.
pub const SOFT_DELETE: &[&str] = &[
    "accounts",
    "launcher_accounts",
    "characters",
    "loadouts",
    "inventories",
    "progression_tracks",
    "progression_objectives",
    "escalation_progress",
    "player_journey",
    "breadcrumbs",
    "encountered_content",
    "bounty_states",
    "cooldown_states",
    "entitlements",
    "wallets",
    "wallet_balances",
];

/// The migrations that make the tables an import writes.
const REQUIRED: [&str; 3] = ["0001", "0020", "0200"];
/// The migration that gives launcher accounts their timeouts.
const TIMEOUTS: &str = "0610";

const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || EmbeddedMigrations::new(&[]),
    postgres: || EmbeddedMigrations::new(&[]),
    soft_delete: SOFT_DELETE,
};

#[derive(QueryableByName)]
struct VersionRow {
    #[diesel(sql_type = Text)]
    version: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

fn missing(versions: &[String]) -> Result<(), String> {
    let missing: Vec<&str> = REQUIRED
        .into_iter()
        .filter(|v| !versions.iter().any(|have| have == v))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "not a 1.14.7 database rustic-isles made, or one too old: migrations {} are missing",
            missing.join(", ")
        ))
    }
}

/// Checks and opens the target. A SQLite file is checked read-only before anything opens it to write.
pub(crate) async fn open(config: DatabaseConfig) -> Result<Database, ImportError> {
    tokio::task::spawn_blocking(move || {
        if let DatabaseConfig::Sqlite { file } = &config {
            if !file.is_file() {
                return Err(ImportError::Target(format!(
                    "{}: there is no database here; start the 1.14.7 server once so it makes one",
                    file.display()
                )));
            }
            let mut conn = dainty_store::open_read_only(file, Duration::from_secs(5))
                .map_err(|e| ImportError::Target(format!("not a database rustic-isles made ({e})")))?;
            let versions: Vec<VersionRow> = diesel::sql_query("SELECT version FROM __diesel_schema_migrations")
                .load(&mut conn)
                .map_err(|e| ImportError::Target(format!("not a database rustic-isles made ({e})")))?;
            let versions: Vec<String> = versions.into_iter().map(|v| v.version).collect();
            missing(&versions).map_err(ImportError::Target)?;
        }
        let options = DatabaseOptions {
            read_connections: 1,
            write_connections: 1,
            ..DatabaseOptions::default()
        };
        Database::open(&config, &options, &SCHEMA).map_err(|e| ImportError::Target(e.to_string()))
    })
    .await
    .map_err(|_| ImportError::Target("the database task stopped".into()))?
}

/// What to write.
pub(crate) struct Plan {
    pub accounts: Vec<Account>,
    pub invites: Vec<Invite>,
    pub node: String,
    /// `revived` or `offline-save`.
    pub source: &'static str,
    pub now: Now,
    pub dry_run: bool,
}

/// What the write did, or would have done.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub imported: BTreeMap<String, u64>,
    pub unchanged: BTreeMap<String, u64>,
    pub skipped: Vec<Skipped>,
}

impl Outcome {
    fn count(&mut self, what: &str) {
        *self.imported.entry(what.to_owned()).or_default() += 1;
    }

    fn skip(&mut self, kind: &str, id: &str, reason: impl Into<String>) {
        self.skipped.push(Skipped {
            kind: kind.to_owned(),
            id: id.to_owned(),
            reason: reason.into(),
        });
    }
}

enum Fail {
    Db(String),
    Check(String),
    DryRun(Box<Outcome>),
}

impl From<dainty_store::Error> for Fail {
    fn from(e: dainty_store::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<diesel::result::Error> for Fail {
    fn from(e: diesel::result::Error) -> Self {
        Self::Db(e.to_string())
    }
}

/// Writes `plan` in one transaction; a dry run rolls it back and reports the same counts.
pub(crate) async fn write(database: &Database, plan: Plan) -> Result<Outcome, ImportError> {
    let result = database
        .write(Actor::Host("legacy-import".into()), move |tx| {
            let outcome = apply(tx.conn(), &plan)?;
            if plan.dry_run {
                Err(Fail::DryRun(Box::new(outcome)))
            } else {
                Ok(outcome)
            }
        })
        .await;
    match result {
        Ok(outcome) => Ok(outcome),
        Err(Fail::DryRun(outcome)) => Ok(*outcome),
        Err(Fail::Db(text) | Fail::Check(text)) => Err(ImportError::Target(text)),
    }
}

fn count(conn: &mut AnyConnection, sql: &str, binds: &[&str]) -> Result<i64, Fail> {
    let mut query = diesel::sql_query(sql).into_boxed::<MultiBackend>();
    for b in binds {
        query = query.bind::<Text, _>((*b).to_owned());
    }
    Ok(query.get_result::<CountRow>(conn)?.n)
}

fn apply(conn: &mut AnyConnection, plan: &Plan) -> Result<Outcome, Fail> {
    let versions: Vec<String> = diesel::sql_query("SELECT version FROM __diesel_schema_migrations")
        .load::<VersionRow>(conn)
        .map_err(|e| Fail::Check(format!("not a database rustic-isles made ({e})")))?
        .into_iter()
        .map(|v| v.version)
        .collect();
    missing(&versions).map_err(Fail::Check)?;
    let keeps_timeouts = versions.iter().any(|v| v == TIMEOUTS);
    let now = plan.now.text.as_str();
    let by = format!("legacy-import:{}", plan.source);
    let mut out = Outcome::default();

    for account in &plan.accounts {
        let user_id = account.user_id.trim();
        if user_id.is_empty() {
            out.skip("account", user_id, "it has no userId");
            continue;
        }
        let id = rustic_isles_db::ids::derived_account_id(&plan.node, user_id);
        if count(conn, "SELECT count(*) AS n FROM accounts WHERE id = $1", &[&id])? > 0 {
            *out.unchanged.entry("accounts".into()).or_default() += 1;
            continue;
        }
        if count(
            conn,
            "SELECT count(*) AS n FROM accounts WHERE user_id = $1",
            &[user_id],
        )? > 0
        {
            out.skip("account", user_id, "a 1.14.7 account already has this userId");
            continue;
        }
        let name = account.name.trim();
        if name.is_empty() {
            out.skip("account", user_id, "it has no name");
            continue;
        }
        let normalized = name.to_lowercase();
        if count(
            conn,
            "SELECT count(*) AS n FROM launcher_accounts WHERE display_name_normalized = $1 AND deleted_at IS NULL",
            &[&normalized],
        )? > 0
        {
            out.skip("account", user_id, format!("the name {name:?} is taken on this server"));
            continue;
        }
        diesel::sql_query("INSERT INTO accounts (id, user_id, name, notes) VALUES ($1, $2, $3, $4)")
            .into_boxed::<MultiBackend>()
            .bind::<Text, _>(&id)
            .bind::<Text, _>(user_id)
            .bind::<Text, _>(name)
            .bind::<BigInt, _>(account.notes)
            .execute(conn)?;

        // The ban: a timeout where the target keeps them, else a ban
        let mut status = "active";
        let mut timeout: Option<(String, Option<String>)> = None;
        if let Some(ban) = &account.ban {
            match ban.expires_at.and_then(format_ms) {
                Some(until) if keeps_timeouts => {
                    timeout = Some((until, ban.reason.clone()));
                    out.count("timeouts");
                }
                _ => {
                    status = "banned";
                    out.count("bans");
                    if ban.expires_at.is_some() {
                        out.count("temporary bans made permanent");
                    }
                }
            }
        }
        let columns = "account_id, display_name_normalized, display_name, status, approval_status, \
                       approval_updated_at, approval_updated_by, roles, created_at, username_set";
        let values = "$1, $2, $3, $4, 'approved', $5, $6, '[\"user\"]', $7, 1";
        let query = match &timeout {
            Some((until, reason)) => diesel::sql_query(format!(
                "INSERT INTO launcher_accounts ({columns}, timeout_until, timeout_since, timeout_reason) \
                 VALUES ({values}, $8, $9, $10)"
            ))
            .into_boxed::<MultiBackend>()
            .bind::<Text, _>(&id)
            .bind::<Text, _>(&normalized)
            .bind::<Text, _>(name)
            .bind::<Text, _>(status)
            .bind::<Text, _>(now)
            .bind::<Text, _>(&by)
            .bind::<Text, _>(now)
            .bind::<Text, _>(until)
            .bind::<Text, _>(now)
            .bind::<Nullable<Text>, _>(reason.clone()),
            None => diesel::sql_query(format!("INSERT INTO launcher_accounts ({columns}) VALUES ({values})"))
                .into_boxed::<MultiBackend>()
                .bind::<Text, _>(&id)
                .bind::<Text, _>(&normalized)
                .bind::<Text, _>(name)
                .bind::<Text, _>(status)
                .bind::<Text, _>(now)
                .bind::<Text, _>(&by)
                .bind::<Text, _>(now),
        };
        query.execute(conn)?;
        out.count("accounts");
        if account.is_admin {
            out.count("1.4.4 admins imported as players");
        }

        if let Some(link) = &account.link {
            let taken = count(
                conn,
                "SELECT count(*) AS n FROM auth_identities WHERE provider = 'discord' AND provider_subject = $1",
                &[&link.discord_id],
            )?;
            if taken > 0 {
                out.skip(
                    "discord_link",
                    &link.discord_id,
                    "that Discord account is linked to another 1.14.7 account",
                );
            } else {
                // 1.4.4 kept no Discord username; 1.14.7 refreshes it at the next sign-in
                diesel::sql_query(
                    "INSERT INTO auth_identities (provider, provider_subject, account_id, provider_username, \
                     provider_avatar_url, linked_at) VALUES ('discord', $1, $2, $3, NULL, $4)",
                )
                .into_boxed::<MultiBackend>()
                .bind::<Text, _>(&link.discord_id)
                .bind::<Text, _>(&id)
                .bind::<Text, _>(&link.discord_id)
                .bind::<Text, _>(&link.linked_at)
                .execute(conn)?;
                out.count("discord links");
            }
        }
    }

    for invite in &plan.invites {
        if count(
            conn,
            "SELECT count(*) AS n FROM reforged_invites WHERE code = $1",
            &[&invite.code],
        )? > 0
        {
            *out.unchanged.entry("invites".into()).or_default() += 1;
            continue;
        }
        if !invite.infinite && invite.uses <= 0 {
            out.skip("invite", &invite.code, "it has no uses left");
            continue;
        }
        diesel::sql_query(
            "INSERT INTO reforged_invites (code, uses_remaining, infinite_uses, note, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .into_boxed::<MultiBackend>()
        .bind::<Text, _>(&invite.code)
        .bind::<BigInt, _>(invite.uses.max(0))
        .bind::<Bool, _>(invite.infinite)
        .bind::<Text, _>(format!("imported from {}", plan.source))
        .bind::<Text, _>(now)
        .execute(conn)?;
        out.count("invites");
    }
    Ok(out)
}
