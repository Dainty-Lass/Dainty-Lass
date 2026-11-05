//! Dauntless Revived 1.4.4: a server's metagame database and the offline saves players kept, carried into a 1.14.7
//! server's accounts. Both are SQLite files the Node metagame (Drizzle) or the Rust one (Diesel) made.

pub mod drsave;
pub mod read;
pub mod schema;
pub mod source;

use std::path::Path;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use sha2::{Digest, Sha256};

use crate::error::{ImportError, Result};
use crate::report::Report;
use crate::target::Target;
use source::Origin;

/// What every import takes besides its source and target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Write nothing; report what would be written
    pub dry_run: bool,
    /// Unix milliseconds: which bans are still in force, and when the new rows are made
    pub now_ms: i64,
}

impl Options {
    pub fn now(dry_run: bool) -> Self {
        Self {
            dry_run,
            now_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    pub fn rfc3339(&self) -> String {
        chrono::DateTime::from_timestamp_millis(self.now_ms)
            .unwrap_or_default()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    }
}

/// Credentials and machine-bound data an offline save never carries (DR `offline_save::CLEARED_TABLES`).
const CLEARED_TABLES: [&str; 6] = [
    "userapikeystoregister",
    "gameserverapikeystoregister",
    "gameserverapikeys",
    "accountkeyrecovery",
    "recoveryproofs",
    "recoverychallenges",
];

fn common_notes(report: &mut Report) {
    report.note(
        "1.4.4 account keys are not carried: 1.14.7 keeps keys as HMACs, so players sign in through the launcher",
    );
    if report.imported("1.4.4 admins imported as players") > 0 {
        report.note("1.4.4 admins were imported as players: give them their roles in the admin UI");
    }
    if report.imported("temporary bans made permanent") > 0 {
        report.note(
            "this 1.14.7 database keeps no timeouts yet (rustic-isles before migration 0610), so timed bans were              imported as bans: update the server and import again, or lift them in the admin UI when they run out",
        );
    }
}

/// Imports a 1.4.4 server's metagame database at `source`: its accounts, bans, Discord links and invite codes.
/// The source is read from a private copy and never written.
pub async fn import_server(source: &Path, target: &Target, options: Options) -> Result<Report> {
    let now = options.rfc3339();
    let (batch, mut report) = {
        let mut prepared = source::prepare(source, Origin::Server)?;
        let batch = read::batch(&mut prepared.conn, options.now_ms, &now, true)?;
        let mut report = Report::new("revived", options.dry_run);
        read::count_dropped(&mut prepared.conn, &mut report)?;
        (batch, report)
    };
    let unlinked = batch.accounts.iter().filter(|a| a.discord.is_none()).count();
    report = target.import(batch, report, now, "revived").await?;
    common_notes(&mut report);
    if unlinked > 0 {
        report.note(format!(
            "{unlinked} 1.4.4 account(s) had no Discord link, so their players have no way to sign in yet: \
             link one from the admin UI"
        ));
    }
    Ok(report)
}

#[derive(QueryableByName)]
struct Identity {
    #[diesel(sql_type = Text)]
    user_id: String,
    #[diesel(sql_type = Text)]
    username: String,
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The offline database's one account: the one whose key is the save's credential and which the save's metadata
/// names, with none of the cleared credentials left (DR `offline_save::identity`); or, for a bare database, its
/// only account.
fn offline_identity(conn: &mut store::Connection, save: Option<&drsave::SaveMetadata>) -> Result<String> {
    let refuse = || ImportError::invalid("not a Dauntless Revived 1.4.4 offline save: it doesn't hold one player");
    let rows = match save {
        Some(save) => diesel::sql_query(
            "SELECT u.userId AS user_id, u.name AS username FROM userapikeys k JOIN users u ON u.userId = k.userId \
             WHERE k.keyHash = ?",
        )
        .bind::<Text, _>(sha256_hex(&save.credential.key))
        .load::<Identity>(conn)?,
        None => diesel::sql_query("SELECT userId AS user_id, name AS username FROM users").load::<Identity>(conn)?,
    };
    let [row] = rows.as_slice() else {
        return Err(refuse());
    };
    if let Some(save) = save {
        if row.user_id != save.account.user_id || row.username != save.account.username {
            return Err(refuse());
        }
        for table in CLEARED_TABLES {
            let rows = diesel::sql_query(format!("SELECT count(*) AS n FROM {table}")).get_result::<Count>(conn)?;
            if rows.n != 0 {
                return Err(refuse());
            }
        }
    }
    Ok(row.user_id.clone())
}

/// Imports a player's 1.4.4 offline save at `source`, a `.drsave` file or the bare offline database: its one
/// account, with its Discord link.
pub async fn import_offline_save(source: &Path, target: &Target, options: Options) -> Result<Report> {
    let now = options.rfc3339();
    let mut head = [0u8; 8];
    let is_container = std::fs::File::open(source)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut head))
        .is_ok()
        && &head == drsave::SAVE_MAGIC;
    let unpacked = tempfile::Builder::new()
        .prefix("dainty-legacy-save-")
        .tempdir()
        .map_err(|e| ImportError::Io(format!("a private folder for the save: {e}")))?;
    let (database, metadata) = if is_container {
        let database = unpacked.path().join("offline.db");
        let metadata = drsave::read_save(source, &database).map_err(|e| match e {
            drsave::FormatError::Invalid => ImportError::invalid(e.to_string()),
            drsave::FormatError::Io(io) => ImportError::io(source, io),
        })?;
        (database, Some(metadata))
    } else {
        (source.to_path_buf(), None)
    };
    let (batch, mut report) = {
        let mut prepared = source::prepare(&database, Origin::OfflineSave)?;
        let user_id = offline_identity(&mut prepared.conn, metadata.as_ref())?;
        let mut batch = read::batch(&mut prepared.conn, options.now_ms, &now, false)?;
        batch.accounts.retain(|account| account.user_id == user_id);
        let mut report = Report::new("offline-save", options.dry_run);
        read::count_dropped(&mut prepared.conn, &mut report)?;
        (batch, report)
    };
    drop(unpacked);
    let unlinked = batch.accounts.iter().any(|a| a.discord.is_none());
    report = target.import(batch, report, now, "offline-save").await?;
    common_notes(&mut report);
    if unlinked {
        report.note("the save's account had no Discord link, so its player has no way to sign in yet: link one from the admin UI");
    }
    Ok(report)
}
