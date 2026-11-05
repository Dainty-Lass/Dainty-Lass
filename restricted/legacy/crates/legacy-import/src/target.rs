//! The 1.14.7 server's account storage, as rustic-isles' `isles-core` keeps it (`migrations/*/0001_accounts`,
//! `0020_auth`, `0200_reforged_invites` at rustic-isles `af9c125`, and `0610_timeouts` where the server has it):
//! `accounts`, `launcher_accounts`,
//! `auth_identities` and `reforged_invites`. The database is opened as it is, through `store::Database` with no
//! migrations of its own, so every row written here is in the change log under `host:legacy-import` (DD23) and an
//! import can be reverted like any other write.
//!
//! An account's global id is derived the way `isles-db migrate` derives it (`transforms::deterministic_account_id`),
//! so a second run finds what the first one wrote, and a userId that both a Mongo migration and this import carry
//! lands on one account.

use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use module_api::{GlobalId, NodeId};
use sha2::{Digest, Sha256};
use store::{Actor, AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema};

use crate::error::{ImportError, Result};
use crate::report::Report;

/// isles-core's soft-deleted tables (`db.rs`'s `SCHEMA`): the triggers `store` puts back on open must be the ones
/// the 1.14.7 backend runs with, or its deletes would change meaning until it restarts.
const SOFT_DELETE: &[&str] = &[
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

/// No migrations: the 1.14.7 backend owns the schema, and an import never changes it.
const AS_IT_IS: DatabaseSchema = DatabaseSchema {
    sqlite: || diesel_migrations::EmbeddedMigrations::new(&[]),
    postgres: || diesel_migrations::EmbeddedMigrations::new(&[]),
    soft_delete: SOFT_DELETE,
};

/// The isles migrations whose tables an import writes.
const NEEDED: [&str; 3] = ["0001", "0020", "0200"];
/// rustic-isles' timeouts (`launcher_accounts.timeout_*`); without it a 1.4.4 timed ban is imported as a ban.
const TIMEOUTS: &str = "0610";

pub const ACTOR: &str = "legacy-import";
const DISCORD: &str = "discord";

/// A 1.4.4 ban as 1.14.7 can keep it: an account is banned or it isn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ban {
    /// Unix milliseconds; none is permanent
    pub expires_at: Option<i64>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordLink {
    pub subject: String,
    pub linked_at: String,
}

/// One old account, with what of it carries to 1.14.7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyAccount {
    pub user_id: String,
    pub name: String,
    pub notes: i64,
    pub admin: bool,
    pub ban: Option<Ban>,
    pub discord: Option<DiscordLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub code: String,
    pub uses_remaining: i64,
    pub infinite_uses: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    pub accounts: Vec<LegacyAccount>,
    pub invites: Vec<Invite>,
}

#[derive(Debug, thiserror::Error)]
enum TxError {
    #[error("{0}")]
    Store(#[from] store::Error),
    #[error("{0}")]
    Query(#[from] diesel::result::Error),
    #[error("{0}")]
    Import(#[from] ImportError),
    /// A dry run's way out: everything it wrote is rolled back
    #[error("dry run")]
    DryRun(Box<Report>),
}

/// A 26-character Crockford Base32 ULID from `SHA-256("isles-db:<ns>:<node>:<key>")` (isles-db `transforms.rs`).
fn deterministic_ulid(namespace: &str, node: &NodeId, key: &str) -> String {
    const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut hasher = Sha256::new();
    for part in ["isles-db:", namespace, ":", node.as_str(), ":", key] {
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    let val = u128::from_be_bytes(bytes);
    // The top two bits of a 26-character ULID are 0, so the first character is 0 to 7
    (0..26)
        .map(|idx| {
            let mask = if idx == 0 { 0x07 } else { 0x1F };
            char::from(CROCKFORD[((val >> (125 - idx * 5)) & mask) as usize])
        })
        .collect()
}

/// The account's global id on `node` (DD18), as `isles-db migrate` derives it.
pub fn account_id(node: &NodeId, user_id: &str) -> Result<GlobalId> {
    let raw = format!("account:{}@{node}", deterministic_ulid("account", node, user_id.trim()));
    raw.parse::<GlobalId>()
        .map_err(|e| ImportError::invalid(format!("a derived account id: {e}")))
}

/// `Date#toISOString`, as isles keeps a timeout's end, so the text orders as the time does.
fn iso_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// `normalize_display_name` in isles-core's launcher: trimmed and lower-cased.
fn normalized(name: &str) -> String {
    name.trim().to_lowercase()
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

#[derive(QueryableByName)]
struct Version {
    #[diesel(sql_type = Text)]
    version: String,
}

fn exists(conn: &mut AnyConnection, sql: &str, binds: &[&str]) -> std::result::Result<bool, TxError> {
    let mut query = diesel::sql_query(sql).into_boxed::<store::MultiBackend>();
    for bind in binds {
        query = query.bind::<Text, _>((*bind).to_owned());
    }
    Ok(query.get_result::<Count>(conn)?.n > 0)
}

/// Whether the target keeps timeouts, once it is one an import can write.
fn check_versions(versions: Vec<Version>) -> Result<bool> {
    let have: BTreeSet<String> = versions.into_iter().map(|v| v.version).collect();
    let missing: Vec<&str> = NEEDED.iter().copied().filter(|v| !have.contains(*v)).collect();
    if !missing.is_empty() {
        return Err(ImportError::Target(format!(
            "not a 1.14.7 database rustic-isles made, or one too old: migrations {} are missing",
            missing.join(", ")
        )));
    }
    Ok(have.contains(TIMEOUTS))
}

pub struct Target {
    db: Database,
    node: NodeId,
}

impl Target {
    /// Opens the 1.14.7 database at `config`, which the 1.14.7 backend must have made. A SQLite file is checked
    /// read-only first, so a wrong path changes nothing; a Postgres database is checked in the open's transaction.
    pub fn open(config: &DatabaseConfig, node: NodeId) -> Result<Self> {
        if let DatabaseConfig::Sqlite { file } = config {
            if !file.is_file() {
                return Err(ImportError::Target(format!(
                    "{}: there is no database here; start the 1.14.7 server once so it makes one",
                    file.display()
                )));
            }
            let mut conn = store::open_read_only(file, std::time::Duration::from_secs(5))
                .map_err(|e| ImportError::Target(e.to_string()))?;
            let versions = diesel::sql_query("SELECT version FROM __diesel_schema_migrations")
                .load::<Version>(&mut conn)
                .map_err(|e| ImportError::Target(format!("not a database rustic-isles made ({e})")))?;
            check_versions(versions)?;
        }
        let db = Database::open(config, &DatabaseOptions::default(), &AS_IT_IS)
            .map_err(|e| ImportError::Target(format!("not a 1.14.7 database rustic-isles made ({e})")))?;
        Ok(Self { db, node })
    }

    pub fn node(&self) -> &NodeId {
        &self.node
    }

    /// Refuses a database the 1.14.7 backend didn't make, or one too old for the tables an import writes; whether it
    /// keeps timeouts.
    pub async fn check(&self) -> Result<bool> {
        let versions = self
            .db
            .read(|conn| -> std::result::Result<_, TxError> {
                Ok(diesel::sql_query("SELECT version FROM __diesel_schema_migrations").load::<Version>(conn)?)
            })
            .await
            .map_err(|e| ImportError::Target(format!("not a database rustic-isles made ({e})")))?;
        check_versions(versions)
    }

    /// Writes `batch` in one transaction, or on a dry run writes it and rolls it back. `now` is the RFC 3339 time
    /// the new rows are made at; `source` names the old install in the rows' notes and approvals.
    pub async fn import(&self, batch: Batch, mut report: Report, now: String, source: &'static str) -> Result<Report> {
        let timeouts = self.check().await?;
        let node = self.node.clone();
        let dry_run = report.dry_run;
        let outcome = self
            .db
            .write(Actor::Host(ACTOR.to_owned()), move |tx| {
                let conn = tx.conn();
                for account in &batch.accounts {
                    write_account(conn, &node, account, &now, source, timeouts, &mut report)?;
                }
                for invite in &batch.invites {
                    write_invite(conn, invite, &now, source, &mut report)?;
                }
                if dry_run {
                    return Err(TxError::DryRun(Box::new(report)));
                }
                Ok(report)
            })
            .await;
        match outcome {
            Ok(report) => Ok(report),
            Err(TxError::DryRun(report)) => Ok(*report),
            Err(TxError::Import(error)) => Err(error),
            Err(other) => Err(ImportError::Target(other.to_string())),
        }
    }
}

fn write_account(
    conn: &mut AnyConnection,
    node: &NodeId,
    account: &LegacyAccount,
    now: &str,
    source: &'static str,
    timeouts: bool,
    report: &mut Report,
) -> std::result::Result<(), TxError> {
    let user_id = account.user_id.trim();
    let id = account_id(node, user_id)?.to_string();
    if exists(conn, "SELECT count(*) AS n FROM accounts WHERE id = $1", &[&id])? {
        report.unchanged("accounts");
        return Ok(());
    }
    if exists(
        conn,
        "SELECT count(*) AS n FROM accounts WHERE user_id = $1",
        &[user_id],
    )? {
        report.skip("account", user_id, "a 1.14.7 account already has this userId");
        return Ok(());
    }
    let display_name = account.name.trim();
    if display_name.is_empty() {
        report.skip("account", user_id, "it has no name");
        return Ok(());
    }
    let normalized_name = normalized(display_name);
    if exists(
        conn,
        "SELECT count(*) AS n FROM launcher_accounts WHERE display_name_normalized = $1 AND deleted_at IS NULL",
        &[&normalized_name],
    )? {
        report.skip(
            "account",
            user_id,
            format!("the name {display_name:?} is taken on this server"),
        );
        return Ok(());
    }

    diesel::sql_query("INSERT INTO accounts (id, user_id, name, notes) VALUES ($1, $2, $3, $4)")
        .bind::<Text, _>(&id)
        .bind::<Text, _>(user_id)
        .bind::<Text, _>(display_name)
        .bind::<BigInt, _>(account.notes)
        .execute(conn)?;
    // A timed ban is a timeout where the target keeps them: the player may sign in, and plays again when it ends
    let timeout = account.ban.as_ref().and_then(|ban| {
        ban.expires_at
            .filter(|_| timeouts)
            .map(|end| (end, ban.reason.as_str()))
    });
    let status = if account.ban.is_some() && timeout.is_none() {
        "banned"
    } else {
        "active"
    };
    let by = format!("{ACTOR}:{source}");
    diesel::sql_query(
        "INSERT INTO launcher_accounts (account_id, display_name_normalized, display_name, status, approval_status, \
         approval_updated_at, approval_updated_by, roles, created_at, username_set) \
         VALUES ($1, $2, $3, $4, 'approved', $5, $6, '[\"user\"]', $7, 1)",
    )
    .bind::<Text, _>(&id)
    .bind::<Text, _>(&normalized_name)
    .bind::<Text, _>(display_name)
    .bind::<Text, _>(status)
    .bind::<Text, _>(now)
    .bind::<Text, _>(&by)
    .bind::<Text, _>(now)
    .execute(conn)?;
    report.count("accounts");
    if let Some((end, reason)) = timeout {
        diesel::sql_query(
            "UPDATE launcher_accounts SET timeout_until = $1, timeout_since = $2, timeout_reason = $3              WHERE account_id = $4",
        )
        .bind::<Text, _>(iso_ms(end))
        .bind::<Text, _>(now)
        .bind::<Text, _>(reason)
        .bind::<Text, _>(&id)
        .execute(conn)?;
        report.count("timeouts");
    } else if let Some(ban) = &account.ban {
        report.count("bans");
        if ban.expires_at.is_some() {
            report.count("temporary bans made permanent");
        }
    }
    if account.admin {
        report.count("1.4.4 admins imported as players");
    }

    if let Some(link) = &account.discord {
        if exists(
            conn,
            "SELECT count(*) AS n FROM auth_identities WHERE provider = $1 AND provider_subject = $2",
            &[DISCORD, &link.subject],
        )? {
            report.skip(
                "discord_link",
                user_id,
                "that Discord account is linked to another 1.14.7 account",
            );
        } else {
            diesel::sql_query(
                "INSERT INTO auth_identities (provider, provider_subject, account_id, provider_username, \
                 provider_avatar_url, linked_at) VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind::<Text, _>(DISCORD)
            .bind::<Text, _>(&link.subject)
            .bind::<Text, _>(&id)
            // 1.4.4 kept no Discord username; the 1.14.7 backend refreshes it at the next Discord sign-in
            .bind::<Text, _>(&link.subject)
            .bind::<Nullable<Text>, _>(None::<String>)
            .bind::<Text, _>(&link.linked_at)
            .execute(conn)?;
            report.count("discord links");
        }
    }
    Ok(())
}

fn write_invite(
    conn: &mut AnyConnection,
    invite: &Invite,
    now: &str,
    source: &'static str,
    report: &mut Report,
) -> std::result::Result<(), TxError> {
    if exists(
        conn,
        "SELECT count(*) AS n FROM reforged_invites WHERE code = $1",
        &[&invite.code],
    )? {
        report.unchanged("invites");
        return Ok(());
    }
    if !invite.infinite_uses && invite.uses_remaining <= 0 {
        report.skip("invite", &invite.code, "it has no uses left");
        return Ok(());
    }
    diesel::sql_query(
        "INSERT INTO reforged_invites (code, uses_remaining, infinite_uses, note, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind::<Text, _>(&invite.code)
    .bind::<BigInt, _>(invite.uses_remaining.max(0))
    .bind::<Bool, _>(invite.infinite_uses)
    .bind::<Text, _>(format!("imported from {source}"))
    .bind::<Text, _>(now)
    .execute(conn)?;
    report.count("invites");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids_are_isles_db_s() {
        let node: NodeId = "local".parse().unwrap();
        let id = account_id(&node, "UID-1").unwrap().to_string();
        assert!(id.starts_with("account:") && id.ends_with("@local"), "{id}");
        assert_eq!(id, account_id(&node, " UID-1 ").unwrap().to_string());
        assert_ne!(id, account_id(&node, "UID-2").unwrap().to_string());
        let other: NodeId = "oregon-1".parse().unwrap();
        assert_ne!(id, account_id(&other, "UID-1").unwrap().to_string());
    }
}
