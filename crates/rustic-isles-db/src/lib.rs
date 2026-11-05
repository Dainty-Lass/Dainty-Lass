// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' database tool (RD27, written clean-room from `docs/port/spec-rustic-isles-db.md`): `migrate`
//! moves a MongoDB 1.14.7 deployment into the store with the RB11 repairs on the way (RU7), `verify` compares
//! the two table by table, and `export-mongo` writes the store back into MongoDB (RB13's rollback, behind the
//! default-on `export-mongo` feature RB15 removes). A library with a command-line front, [`main_with`], for
//! restricted's `isles-db` binary; the 1.14.7 starter values are the caller's ([`LegacyStarter`]).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod cli;
#[cfg(feature = "export-mongo")]
mod export;
#[doc(hidden)]
pub mod extract;
pub mod ids;
pub mod mongo;
#[doc(hidden)]
pub mod read;
mod repair;
mod tables;
mod target;
mod verify;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use module_api::NodeId;
use rustic_isles_economy::inventory::storage_policy::{POLICY_FILE, StoragePolicy};
use serde::{Deserialize, Serialize};
use store::Database;

pub use cli::{Command, main_with, parse_args, print_export_report, print_migrate_report, print_verify_report};
#[cfg(feature = "export-mongo")]
pub use export::{Collections, state_to_mongo_collections};
pub use repair::LegacyStarter;
pub use tables::TargetState;
pub use target::parse_postgres_url;
pub use verify::{CollectionVerification, VerifyReport};

use crate::extract::{Settings, Source};
use crate::ids::Resolver;
use crate::mongo::{MongoClient, MongoError, MongoUri};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("mongo: {0}")]
    Mongo(#[from] MongoError),
    #[error("store: {0}")]
    Store(#[from] store::Error),
    #[error("diesel: {0}")]
    Diesel(#[from] diesel::result::Error),
    #[error("invalid argument: {0}")]
    Config(String),
    #[error("invalid document in {collection}: {detail}")]
    InvalidDocument { collection: String, detail: String },
    #[error("verification failed: {0} collection(s) did not match")]
    VerificationFailed(usize),
    /// `replay` is unit 10's command; the front routes it there (§3.1)
    #[error("replay is not this crate's command: run it through rustic-isles-replay")]
    Replay(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Area {
    Auth,
    Characters,
    Inventory,
    Progression,
    Bounties,
    Parties,
    Reforged,
}

impl Area {
    pub const ALL: [Area; 7] = [
        Self::Auth,
        Self::Characters,
        Self::Inventory,
        Self::Progression,
        Self::Bounties,
        Self::Parties,
        Self::Reforged,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Characters => "characters",
            Self::Inventory => "inventory",
            Self::Progression => "progression",
            Self::Bounties => "bounties",
            Self::Parties => "parties",
            Self::Reforged => "reforged",
        }
    }

    /// One `--area` item, trimmed and lower-cased; the old area names still work.
    pub fn parse(text: &str) -> Result<Area, DbError> {
        let lowered = text.trim().to_ascii_lowercase();
        Ok(match lowered.as_str() {
            "auth" | "accounts" => Self::Auth,
            "characters" | "loadouts" | "characters-loadouts" | "characters_loadouts" => Self::Characters,
            "inventory" | "inventory-reforge" | "inventory_reforge" => Self::Inventory,
            "progression" => Self::Progression,
            "bounties" | "bounties-store" | "bounties_store" | "wallets" => Self::Bounties,
            "parties" | "matchmaking" | "chat" | "friendships" => Self::Parties,
            "reforged" | "reforged-api" | "reforged_api" | "admin" | "guard" => Self::Reforged,
            _ => {
                return Err(DbError::Config(format!(
                    "unknown area {lowered:?}; valid areas: auth, characters, inventory, progression, bounties, \
                     parties, reforged"
                )));
            }
        })
    }
}

/// `user:password@` replaced in a URI, for `Debug`.
fn redact(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_owned();
    };
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    match rest[..authority_end].rsplit_once('@') {
        Some((credentials, _)) => {
            let user = credentials.split_once(':').map_or(credentials, |(u, _)| u);
            format!("{scheme}://{user}:***@{}", &rest[credentials.len() + 1..])
        }
        None => uri.to_owned(),
    }
}

fn effective(areas: &BTreeSet<Area>) -> BTreeSet<Area> {
    if areas.is_empty() {
        Area::ALL.into_iter().collect()
    } else {
        areas.clone()
    }
}

#[derive(Clone)]
pub struct MigrateOptions {
    pub from_uri: String,
    pub to_target: String,
    pub node: NodeId,
    pub dry_run: bool,
    /// Empty: all seven
    pub areas: BTreeSet<Area>,
    pub skip_invalid: bool,
    pub game_data_dir: Option<PathBuf>,
    pub allow_remote_postgres: bool,
    pub starter: LegacyStarter,
}

impl MigrateOptions {
    pub fn effective_areas(&self) -> BTreeSet<Area> {
        effective(&self.areas)
    }
}

// Every field but the starter, which is the caller's constant
impl PartialEq for MigrateOptions {
    fn eq(&self, other: &Self) -> bool {
        self.from_uri == other.from_uri
            && self.to_target == other.to_target
            && self.node == other.node
            && self.dry_run == other.dry_run
            && self.areas == other.areas
            && self.skip_invalid == other.skip_invalid
            && self.game_data_dir == other.game_data_dir
            && self.allow_remote_postgres == other.allow_remote_postgres
    }
}

impl fmt::Debug for MigrateOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MigrateOptions")
            .field("from_uri", &redact(&self.from_uri))
            .field("to_target", &redact(&self.to_target))
            .field("node", &self.node)
            .field("dry_run", &self.dry_run)
            .field("areas", &self.areas)
            .field("skip_invalid", &self.skip_invalid)
            .field("game_data_dir", &self.game_data_dir)
            .field("allow_remote_postgres", &self.allow_remote_postgres)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub from_source: String,
    pub to_uri: String,
    pub areas: BTreeSet<Area>,
    pub dry_run: bool,
    pub allow_remote_postgres: bool,
}

impl ExportOptions {
    pub fn effective_areas(&self) -> BTreeSet<Area> {
        effective(&self.areas)
    }
}

impl fmt::Debug for ExportOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExportOptions")
            .field("from_source", &redact(&self.from_source))
            .field("to_uri", &redact(&self.to_uri))
            .field("areas", &self.areas)
            .field("dry_run", &self.dry_run)
            .field("allow_remote_postgres", &self.allow_remote_postgres)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrateReport {
    pub dry_run: bool,
    /// All 37 tables by name, 0 for an area not selected
    pub counts: BTreeMap<String, usize>,
    pub skipped_invalid: usize,
    pub rb11_launcher_approvals_backfilled: usize,
    pub rb11_inventory_arrays_transformed: usize,
    pub rb11_starter_instances_repaired: usize,
    pub rb11_loadout_slots_reconciled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub dry_run: bool,
    pub counts: BTreeMap<String, usize>,
}

/// The storage policy under `--game-data-dir`, at its root or in `data/`; none loaded is no repair, silently
/// (P10).
fn load_policy(dir: Option<&Path>) -> Option<StoragePolicy> {
    let dir = dir?;
    [dir.join(POLICY_FILE), dir.join("data").join(POLICY_FILE)]
        .iter()
        .find_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            serde_json::from_str(&text)
                .ok()
                .map(|doc| StoragePolicy::from_document(&doc))
        })
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, DbError> + Send + 'static) -> Result<T, DbError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| DbError::Store(store::Error::Panicked))?
}

async fn resolver(database: Option<&Database>, node: &NodeId) -> Result<Resolver, DbError> {
    let (accounts, parties) = match database {
        Some(db) => db.read(tables::existing_ids).await?,
        None => (Vec::new(), Vec::new()),
    };
    Ok(Resolver::new(&node.to_string(), accounts, parties))
}

/// The source, connected before the target is opened (§7.1, §10.1). Blocking: the client is synchronous.
async fn connect_source(opts: &MigrateOptions) -> Result<MongoClient, DbError> {
    let uri = MongoUri::parse(&opts.from_uri)?;
    blocking(move || Ok(MongoClient::connect(&uri)?)).await
}

/// Every selected area out of the source.
async fn extract_source(
    opts: &MigrateOptions,
    mut client: MongoClient,
    resolver: Resolver,
) -> Result<extract::Extraction, DbError> {
    let areas = opts.effective_areas();
    let skip_invalid = opts.skip_invalid;
    let policy = load_policy(opts.game_data_dir.as_deref());
    let starter = opts.starter.clone();
    blocking(move || {
        let settings = Settings {
            areas: &areas,
            skip_invalid,
            policy: policy.as_ref(),
            starter: &starter,
        };
        extract::extract(&mut client as &mut dyn Source, &settings, resolver)
    })
    .await
}

fn migrate_report(dry_run: bool, extraction: &extract::Extraction) -> MigrateReport {
    let counts = tables::TABLES
        .iter()
        .map(|t| (t.name.to_owned(), extraction.state.count(t.name)))
        .collect();
    let c = extraction.counters;
    MigrateReport {
        dry_run,
        counts,
        skipped_invalid: c.skipped_invalid,
        rb11_launcher_approvals_backfilled: c.approvals,
        rb11_inventory_arrays_transformed: c.inventory_arrays,
        rb11_starter_instances_repaired: c.starter_instances,
        rb11_loadout_slots_reconciled: c.loadout_slots,
    }
}

/// `migrate` (§7 to §9): everything in one store write under `system:migrate`, or nothing.
pub async fn run_migrate(opts: &MigrateOptions) -> Result<MigrateReport, DbError> {
    let client = connect_source(opts).await?;
    if opts.dry_run {
        // Neither opened nor created
        let extraction = extract_source(opts, client, resolver(None, &opts.node).await?).await?;
        return Ok(migrate_report(true, &extraction));
    }
    let database = target::open(&opts.to_target, opts.allow_remote_postgres, false).await?;
    let extraction = extract_source(opts, client, resolver(Some(&database), &opts.node).await?).await?;
    let report = migrate_report(false, &extraction);
    let state = extraction.state;
    database
        .write(store::Actor::System("migrate".into()), move |tx| {
            tables::write_state(tx.conn(), &state)
        })
        .await?;
    Ok(report)
}

/// `verify` (§10), the report whatever it says; [`run_verify`] turns a mismatch into an error.
pub async fn verify_report(opts: &MigrateOptions) -> Result<VerifyReport, DbError> {
    // The missing-file refusal comes before any connection (coordinator item 1)
    target::config(&opts.to_target, opts.allow_remote_postgres).and_then(|c| target::must_exist(&c))?;
    let client = connect_source(opts).await?;
    let database = target::open(&opts.to_target, opts.allow_remote_postgres, true).await?;
    let expected = extract_source(opts, client, resolver(Some(&database), &opts.node).await?).await?;
    let actual = database.read(tables::read_state).await?;
    let collections = verify::compare(&expected.state, &actual, &opts.effective_areas(), opts.areas.is_empty());
    Ok(VerifyReport {
        ok: collections.iter().all(|c| c.ok),
        collections,
    })
}

pub async fn run_verify(opts: &MigrateOptions) -> Result<VerifyReport, DbError> {
    let report = verify_report(opts).await?;
    match report.collections.iter().filter(|c| !c.ok).count() {
        0 => Ok(report),
        n => Err(DbError::VerificationFailed(n)),
    }
}

/// `export-mongo` (§11): replaces each produced collection, in name order. Not transactional.
#[cfg(feature = "export-mongo")]
pub async fn run_export_mongo(opts: &ExportOptions) -> Result<ExportReport, DbError> {
    let uri = if opts.dry_run {
        None
    } else {
        Some(MongoUri::parse(&opts.to_uri)?)
    };
    let database = target::open(&opts.from_source, opts.allow_remote_postgres, true).await?;
    let state = database.read(tables::read_state).await?;
    let collections = state_to_mongo_collections(&state, &opts.effective_areas());
    let report = ExportReport {
        dry_run: opts.dry_run,
        counts: collections.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
    };
    let Some(uri) = uri else { return Ok(report) };
    blocking(move || {
        let mut writer = mongo::MongoWriter::connect(&uri)?;
        for (name, documents) in &collections {
            writer.replace_collection(name, documents)?;
        }
        Ok(())
    })
    .await?;
    Ok(report)
}

#[doc(hidden)]
pub mod fuzzing {
    //! Entry points for the fuzz targets.
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::read::Doc;

    /// Extraction, the RB11 repairs and (with `export-mongo`) the export builder over in-memory collections.
    pub fn extract_documents(
        collections: BTreeMap<String, Vec<Doc>>,
        skip_invalid: bool,
        starter: &LegacyStarter,
    ) -> Result<TargetState, DbError> {
        let areas: BTreeSet<Area> = Area::ALL.into_iter().collect();
        let settings = Settings {
            areas: &areas,
            skip_invalid,
            policy: None,
            starter,
        };
        let mut source = collections;
        let extraction = extract::extract(&mut source, &settings, Resolver::new("local", Vec::new(), Vec::new()))?;
        #[cfg(feature = "export-mongo")]
        let _ = state_to_mongo_collections(&extraction.state, &areas);
        Ok(extraction.state)
    }

    /// The rows of a table in a state, keyed as §9.2 orders them.
    pub fn rows<'a>(state: &'a TargetState, table: &str) -> Vec<&'a Doc> {
        state.rows(table).map(|(_, r)| r).collect()
    }

    /// A store read back as verify and export read it (§10.2).
    pub async fn read_target(target: &str) -> Result<TargetState, DbError> {
        let database = crate::target::open(target, false, true).await?;
        database.read(tables::read_state).await
    }
}
