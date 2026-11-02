//! `admin key issue|list|revoke` (run as `reforged-host admin key ...`): the owner's way to give each person a
//! key of their own and to take one away (DD23 layer 4, DH2). The host kit has the machine and `admin.env`, so
//! these need no key; each change is logged under `host:admin key <command>`.

use std::path::Path;
use std::process::ExitCode;

use service_kit::env::Env;
use store::Actor;

use crate::cli::KeyCommand;
use crate::config::Config;
use crate::keys::{KeyInfo, Keys, Role};

fn actor(command: &str) -> Actor {
    Actor::Host(format!("admin key {command}"))
}

fn keys(env_file: Option<&Path>) -> Result<Keys, String> {
    let env = Env::load(env_file).map_err(|e| e.to_string())?;
    let config = Config::from_env(&env)?;
    Keys::open(&Keys::database_for(&config.database), &config.api_key)
}

fn when(ms: i64) -> String {
    if ms > 0 { crate::input::iso(ms) } else { "-".to_owned() }
}

/// One row of `key list`.
fn row(key: &KeyInfo) -> String {
    let state = match (key.revoked_at, &key.revoked_by) {
        (Some(at), Some(by)) => format!("revoked {} by {by}", when(at)),
        (Some(at), None) => format!("revoked {}", when(at)),
        _ => "active".to_owned(),
    };
    format!(
        "{:<10}{:<11}{:<24}{:<22}{:<26}{state}",
        key.id,
        key.role.as_str(),
        key.name,
        when(key.created_at),
        key.created_by
    )
}

fn issue(keys: &Keys, name: &str, role: &str) -> Result<(), String> {
    let role = Role::parse(role).ok_or("--role is viewer, moderator, admin or owner")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let (info, key) = runtime
        .block_on(keys.issue(name, role, actor("issue"), crate::now_ms()))
        .map_err(|e| e.to_string())?;
    println!("Admin key {} for {} ({}).", info.id, info.name, info.role);
    println!();
    println!("{key}");
    println!();
    println!(
        "This is the only time the key is shown: only its hash is kept. Give it to {} now.",
        info.name
    );
    println!(
        "They send it as Authorization: Bearer <key>. Revoke it with: reforged-host admin key revoke {}",
        info.id
    );
    Ok(())
}

pub fn run(command: KeyCommand) -> ExitCode {
    let done = match command {
        KeyCommand::Issue { name, role, env_file } => keys(env_file.as_deref()).and_then(|k| issue(&k, &name, &role)),
        KeyCommand::List { env_file } => keys(env_file.as_deref()).and_then(|k| list(&k)),
        KeyCommand::Revoke { id, env_file } => keys(env_file.as_deref()).and_then(|k| revoke(&k, &id)),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\nError: {error}");
            ExitCode::FAILURE
        }
    }
}

fn list(keys: &Keys) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let all = runtime.block_on(keys.list()).map_err(|e| e.to_string())?;
    println!(
        "{:<10}{:<11}{:<24}{:<22}{:<26}STATE",
        "ID", "ROLE", "NAME", "ISSUED", "BY"
    );
    for key in &all {
        println!("{}", row(key));
    }
    Ok(())
}

fn revoke(keys: &Keys, id: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let info = runtime
        .block_on(keys.revoke(id.trim(), actor("revoke"), crate::now_ms()))
        .map_err(|e| e.to_string())?;
    println!(
        "Revoked key {} ({}, {}): its next request is refused.",
        info.id, info.name, info.role
    );
    Ok(())
}
