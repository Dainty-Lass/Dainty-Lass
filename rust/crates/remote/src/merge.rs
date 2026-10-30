// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Configuration merger for managed installs (`docs/remote-management.md` §2.3,
//! `docs/security/remote-management.md`).
//!
//! Applies the controller's shared `host.toml` settings and per-install overrides onto a managed
//! install's `host.toml` while preserving:
//! 1. Per-host listener and network identity (`[server] host`, `game_address`, `mode`, `[ports]`,
//!    `[network]`, `[remote]`);
//! 2. Per-host filesystem and database paths (`[paths]`, `[database]`, per-node `[history]` bucket);
//! 3. Any dotted key listed in `[remote] local_overrides`.

use toml_edit::{DocumentMut, Item, Table};

use crate::RemoteError;
use crate::wire::contains_forbidden_secret_str;

/// Top-level `host.toml` tables that are always local to the managed install and never overwritten
/// by a controller sync.
pub const PRESERVED_LOCAL_TABLES: &[&str] = &["ports", "paths", "database", "network", "remote", "mongo", "wine"];

/// Individual dotted `host.toml` keys inside shared tables that belong to the local machine's
/// identity and are never overwritten by a controller sync unless explicitly absent locally.
pub const PRESERVED_LOCAL_KEYS: &[&str] = &[
    "server.host",
    "server.game_address",
    "server.mode",
    "server.port",
    "history.bucket_endpoint",
    "history.bucket_name",
    "history.bucket_prefix",
    "history.bucket_region",
];

/// Returns `true` if `dotted_key` (e.g. `"ports.gateway"`, `"server.host"`, or `"paradox.max_hunts"`)
/// is protected as a per-host local field or listed in `local_overrides`.
pub fn is_preserved_local_key(dotted_key: &str, local_overrides: &[String]) -> bool {
    let trimmed = dotted_key.trim();
    if let Some((table, _)) = trimmed.split_once('.')
        && PRESERVED_LOCAL_TABLES.contains(&table)
    {
        return true;
    }
    if PRESERVED_LOCAL_KEYS.contains(&trimmed) {
        return true;
    }
    local_overrides.iter().any(|ov| {
        let ov_trim = ov.trim();
        ov_trim == trimmed
            || (ov_trim.ends_with(".*") && trimmed.starts_with(&ov_trim[..ov_trim.len().saturating_sub(1)]))
    })
}

fn apply_table_recursive(target: &mut Table, source: &Table, prefix: &str, local_overrides: &[String]) {
    for (key, item) in source.iter() {
        let dotted = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        if is_preserved_local_key(&dotted, local_overrides) {
            continue;
        }
        if PRESERVED_LOCAL_TABLES.contains(&dotted.as_str()) {
            continue;
        }
        match item {
            Item::Table(src_sub) => {
                if !target.contains_key(key) || !target[key].is_table() {
                    target.insert(key, Item::Table(Table::new()));
                }
                if let Some(dst_sub) = target[key].as_table_mut() {
                    apply_table_recursive(dst_sub, src_sub, &dotted, local_overrides);
                }
            }
            Item::Value(val) => {
                target.insert(key, Item::Value(val.clone()));
            }
            _ => {}
        }
    }
}

/// Merge `shared_toml` and an optional `per_node_override_toml` onto `local_toml`, preserving all
/// per-host identity fields and `local_overrides` while keeping comments and layout intact.
pub fn merge_managed_host_toml(
    local_toml: &str,
    shared_toml: &str,
    per_node_override_toml: Option<&str>,
    local_overrides: &[String],
) -> Result<String, RemoteError> {
    if contains_forbidden_secret_str(local_toml)
        || contains_forbidden_secret_str(shared_toml)
        || per_node_override_toml.is_some_and(contains_forbidden_secret_str)
    {
        return Err(RemoteError::SecretLeakRefused(
            "host.toml merge input must never contain secrets".to_string(),
        ));
    }

    let mut local_doc: DocumentMut = local_toml
        .parse()
        .map_err(|e| RemoteError::InvalidInput(format!("invalid local host.toml: {e}")))?;
    let shared_doc: DocumentMut = shared_toml
        .parse()
        .map_err(|e| RemoteError::InvalidInput(format!("invalid shared host.toml: {e}")))?;

    let mut effective_overrides: Vec<String> = local_overrides.to_vec();
    if let Some(remote_tbl) = local_doc.get("remote").and_then(Item::as_table)
        && let Some(arr) = remote_tbl
            .get("local_overrides")
            .and_then(Item::as_value)
            .and_then(|v| v.as_array())
    {
        for val in arr.iter() {
            if let Some(s) = val.as_str()
                && !effective_overrides.iter().any(|existing| existing == s)
            {
                effective_overrides.push(s.to_string());
            }
        }
    }

    apply_table_recursive(
        local_doc.as_table_mut(),
        shared_doc.as_table(),
        "",
        &effective_overrides,
    );

    if let Some(node_toml) = per_node_override_toml {
        let node_doc: DocumentMut = node_toml
            .parse()
            .map_err(|e| RemoteError::InvalidInput(format!("invalid per-node override host.toml: {e}")))?;
        apply_table_recursive(local_doc.as_table_mut(), node_doc.as_table(), "", &effective_overrides);
    }

    Ok(local_doc.to_string())
}
