// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Ramsgate Reborn saves on demand (`docs/port/spec-legacy-ramsgate.md`): when an account first asks for its
//! characters and has none, its old save is read from the Ramsgate Reborn MongoDB and written into the 1.14.7
//! database before the starter character would be made. The account is matched by its Discord link (the old
//! database's `authIdentities`) or by a claim code staff gave the player; each old account fills one account, once.

mod fetch;
pub mod transform;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

use dainty_module_api::{Actor, ActorContext, BoxFuture, ModuleId, NodeId};
use rustic_isles_characters::FirstSave;
use rustic_isles_db::LegacyStarter;
use rustic_isles_db::mongo::{MongoClient, MongoUri};
use rustic_isles_store::repo::auth::AccountRecord;
use rustic_isles_store::repo::legacy::{self, ImportRecord, RAMSGATE_REBORN};
use rustic_isles_store::{IslesStore, RepoError};
use serde_json::{Map, Value};

pub use fetch::{DISCORD, Found, Who, barred};

/// The setting holding the old MongoDB's URI, with its credentials (a secret: secrets.env).
pub const URI_SETTING: &str = "RUSTIC_ISLES_LEGACY_MONGO_URI";
/// RR's database when the URI names none (`MONGODB_DB`'s default, spec section 1).
pub const DEFAULT_DATABASE: &str = "mysticparadox";

/// What one first-character read ended as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The save was written; the old user id and the rows per table.
    Imported(String, BTreeMap<String, usize>),
    /// Neither a claim nor a Discord link names an old account.
    NoMatch,
    /// The old database has no such account, or it has no character.
    NoSave,
    /// The old account may not be carried (a ban, `disabled`, approval refused), or its save is in another account.
    Refused(String),
    /// The account had a save already by the time the write ran.
    AlreadyHasOne,
}

/// The on-demand importer the characters area asks ([`FirstSave`]).
pub struct RamsgateImporter {
    uri: MongoUri,
    node: NodeId,
    starter: LegacyStarter,
    catalog: Option<Arc<HashSet<String>>>,
}

// The URI holds the old database's password
impl fmt::Debug for RamsgateImporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RamsgateImporter")
            .field("uri", &self.uri)
            .field("node", &self.node)
            .field("catalog", &self.catalog.as_ref().map(|c| c.len()))
            .finish_non_exhaustive()
    }
}

fn names_database(text: &str) -> bool {
    let rest = text.split_once("://").map_or(text, |(_, r)| r);
    let path = rest.split_once('/').map_or("", |(_, p)| p);
    !path.split(['?', '#']).next().unwrap_or_default().is_empty()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl RamsgateImporter {
    /// From [`URI_SETTING`]'s value; `catalog` is 1.14.7's item ids when the host has the catalog export.
    pub fn new(
        uri: &str,
        node: NodeId,
        starter: LegacyStarter,
        catalog: Option<HashSet<String>>,
    ) -> Result<Self, String> {
        let mut parsed = MongoUri::parse(uri.trim()).map_err(|e| format!("{URI_SETTING}: {e}"))?;
        if !names_database(uri.trim()) {
            DEFAULT_DATABASE.clone_into(&mut parsed.database);
        }
        Ok(Self {
            uri: parsed,
            node,
            starter,
            catalog: catalog.map(Arc::new),
        })
    }

    fn actor() -> ActorContext {
        match "legacy".parse::<ModuleId>() {
            Ok(module) => ActorContext::new(Actor::Module { module }),
            Err(_) => ActorContext::new(Actor::System),
        }
    }

    /// Who the account is on the old server: a redeemed claim first, then its Discord link.
    async fn who(&self, store: &IslesStore, account: &AccountRecord) -> Result<Option<(Who, &'static str)>, RepoError> {
        let id = account.id.clone();
        store
            .read(move |conn, repos| {
                if legacy::imported_into(conn, RAMSGATE_REBORN, &id)?.is_some() {
                    return Ok(None);
                }
                if let Some(user) = legacy::redeemed_by(conn, RAMSGATE_REBORN, &id)? {
                    return Ok(Some((Who::User(user), "claim")));
                }
                let link = repos.auth_identities.find_account_at_provider(conn, DISCORD, &id)?;
                Ok(link.map(|l| (Who::Discord(l.provider_subject), "discord")))
            })
            .await
    }

    /// The whole import for one account.
    pub async fn run(&self, store: &IslesStore, account: &AccountRecord) -> Result<Outcome, String> {
        let Some((who, matched_by)) = self.who(store, account).await.map_err(|e| e.to_string())? else {
            return Ok(Outcome::NoMatch);
        };
        let now = now_ms();
        let uri = self.uri.clone();
        let read = who.clone();
        let found = tokio::task::spawn_blocking(move || {
            let mut client = MongoClient::connect(&uri)?;
            fetch::fetch(&mut client, &read, now)
        })
        .await
        .map_err(|_| "the old database's read stopped".to_owned())?
        .map_err(|e| format!("the old database: {e}"))?;
        let Some(found) = found else {
            return Ok(Outcome::NoSave);
        };
        if let Some(why) = found.barred {
            return Ok(Outcome::Refused(why.to_owned()));
        }
        let target = match transform::to_target(
            found.save,
            &found.user,
            account.user_id.as_str(),
            self.catalog.as_deref(),
            now,
        ) {
            Ok(target) => target,
            Err(transform::Refused::NoCharacter) => return Ok(Outcome::NoSave),
            Err(e) => return Ok(Outcome::Refused(e.to_string())),
        };
        let (id, node, starter, old) = (account.id.clone(), self.node.clone(), self.starter.clone(), found.user);
        let written = store
            .account_write(&account.id, &Self::actor(), move |tx, repos| {
                if !repos.characters.find_many_by_account(tx.conn(), &id)?.is_empty() {
                    return Ok(Err(Outcome::AlreadyHasOne));
                }
                if let Some(holder) = legacy::taken_by(tx.conn(), RAMSGATE_REBORN, &old)? {
                    return Ok(Err(Outcome::Refused(format!(
                        "that save is already in account {holder}"
                    ))));
                }
                let rows = rustic_isles_db::write_player_save(tx.conn(), &node, target.collections, &starter, None)
                    .map_err(|e| RepoError::Data(e.to_string()))?;
                let mut report = Map::new();
                report.insert("rows".into(), serde_json::to_value(&rows).unwrap_or(Value::Null));
                report.insert(
                    "dropped".into(),
                    serde_json::to_value(&target.dropped).unwrap_or(Value::Null),
                );
                legacy::record_import(
                    tx,
                    &ImportRecord {
                        source: RAMSGATE_REBORN,
                        source_user: &old,
                        account: &id,
                        matched_by,
                        imported_at: now,
                        report: &Value::Object(report).to_string(),
                    },
                )?;
                Ok(Ok((old, rows)))
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok(match written {
            Ok((old, rows)) => Outcome::Imported(old, rows),
            Err(outcome) => outcome,
        })
    }
}

impl FirstSave for RamsgateImporter {
    fn import<'a>(&'a self, store: &'a IslesStore, account: &'a AccountRecord) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let user = account.user_id.as_str();
            match self.run(store, account).await {
                Ok(Outcome::Imported(old, rows)) => {
                    let total: usize = rows.values().sum();
                    tracing::info!("[legacy] Ramsgate Reborn save {old} imported into {user}: {total} rows");
                    true
                }
                Ok(Outcome::NoMatch | Outcome::NoSave) => false,
                Ok(Outcome::AlreadyHasOne) => true,
                Ok(Outcome::Refused(why)) => {
                    tracing::warn!("[legacy] Ramsgate Reborn save for {user} not imported: {why}");
                    false
                }
                Err(e) => {
                    tracing::warn!("[legacy] Ramsgate Reborn import for {user} failed, starting fresh: {e}");
                    false
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_database_defaults_to_ramsgates() {
        assert!(names_database("mongodb://u:p@h:27017/live?authSource=admin"));
        assert!(!names_database("mongodb://u:p@h:27017/?authSource=admin"));
        assert!(!names_database("mongodb://h:27017"));
    }
}
