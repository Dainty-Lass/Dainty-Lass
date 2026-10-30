// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `reforged-host db migrate | verify | export-mongo`: MongoDB-to-SQLite/Postgres migration and
//! verification commands.

use std::process::ExitCode;

use crate::cli::{DbArgs, DbCommand, DbExportMongoArgs, DbMigrateArgs, DbVerifyArgs};
use crate::commands::update::mongo_migrated_marker;
use crate::error::HostError;
use crate::hostconfig::{DATABASE_FILE, HostConfig, load_host_config, node_database_config};
use crate::paths::Paths;

pub fn run(paths: &Paths, args: &DbArgs) -> ExitCode {
    match run_inner(paths, args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(p: &Paths, args: &DbArgs) -> Result<ExitCode, HostError> {
    match &args.command {
        DbCommand::Migrate(m) => migrate(p, m),
        DbCommand::Verify(v) => verify(p, v),
        DbCommand::ExportMongo(e) => export_mongo(p, e),
    }
}

fn resolve_db_config(
    p: &Paths,
    config: Option<&HostConfig>,
    to: Option<&str>,
) -> Result<store::DatabaseConfig, HostError> {
    if let Some(target) = to {
        if target.starts_with("postgres://") || target.starts_with("postgresql://") {
            let parsed = url::Url::parse(target).map_err(|e| HostError::message(format!("--to {target}: {e}")))?;
            let host = parsed.host_str().unwrap_or("127.0.0.1").to_owned();
            let port = parsed.port().unwrap_or(5432);
            let user = if parsed.username().is_empty() {
                "dainty".to_owned()
            } else {
                parsed.username().to_owned()
            };
            let password = parsed.password().unwrap_or_default().to_owned();
            let database = parsed.path().trim_start_matches('/').to_owned();
            let database = if database.is_empty() { "dainty".to_owned() } else { database };
            return Ok(store::DatabaseConfig::Postgres(store::PostgresConfig {
                host,
                port,
                user,
                password,
                database,
                allow_remote: true,
            }));
        }
        return Ok(store::DatabaseConfig::Sqlite { file: target.into() });
    }
    if let Some(c) = config {
        node_database_config(c, p)
    } else {
        Ok(store::DatabaseConfig::Sqlite {
            file: p.config.join(DATABASE_FILE),
        })
    }
}

fn migrate(p: &Paths, args: &DbMigrateArgs) -> Result<ExitCode, HostError> {
    let config = load_host_config(p)?;
    let from = args
        .from_mongo
        .as_deref()
        .or_else(|| config.as_ref().map(|c| c.mongo.uri.as_str()))
        .unwrap_or("mongodb://127.0.0.1:27017/reforged");
    let db_cfg = resolve_db_config(p, config.as_ref(), args.to.as_deref())?;
    let (counts, mismatches) = crate::hooks::get().db_migrate(from, &db_cfg, args.dry_run)?;
    let total: usize = counts.iter().map(|(_, n)| *n).sum();
    for (name, n) in &counts {
        println!("  {name}: {n} record(s)");
    }
    if !mismatches.is_empty() {
        eprintln!("Verification found {} mismatch(es):", mismatches.len());
        for (coll, detail) in &mismatches {
            eprintln!("  {coll}: {detail}");
        }
        return Ok(ExitCode::FAILURE);
    }
    if args.dry_run {
        println!("Dry run verified {total} record(s) across {} collection(s).", counts.len());
    } else {
        let marker = mongo_migrated_marker(p);
        if let Some(parent) = marker.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&marker, format!("migrated from {from}\n"));
        println!("Migrated and verified {total} record(s) across {} collection(s).", counts.len());
    }
    Ok(ExitCode::SUCCESS)
}

fn verify(p: &Paths, args: &DbVerifyArgs) -> Result<ExitCode, HostError> {
    let config = load_host_config(p)?;
    let db_cfg = resolve_db_config(p, config.as_ref(), args.to.as_deref())?;
    let mismatches = crate::hooks::get().db_verify(&args.from_mongo, &db_cfg)?;
    if mismatches.is_empty() {
        println!("Database verification passed: all collections match.");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("Verification found {} mismatch(es):", mismatches.len());
        for (coll, detail) in &mismatches {
            eprintln!("  {coll}: {detail}");
        }
        Ok(ExitCode::FAILURE)
    }
}

fn export_mongo(p: &Paths, args: &DbExportMongoArgs) -> Result<ExitCode, HostError> {
    let config = load_host_config(p)?;
    let from = args
        .from_mongo
        .as_deref()
        .or_else(|| config.as_ref().map(|c| c.mongo.uri.as_str()))
        .unwrap_or("mongodb://127.0.0.1:27017/reforged");
    let counts = crate::hooks::get().db_export_mongo(from, &args.out)?;
    let total: usize = counts.iter().map(|(_, n)| *n).sum();
    for (name, n) in &counts {
        println!("  {name}: {n} document(s)");
    }
    println!(
        "Exported {total} document(s) across {} collection(s) to {}.",
        counts.len(),
        args.out.display()
    );
    Ok(ExitCode::SUCCESS)
}
