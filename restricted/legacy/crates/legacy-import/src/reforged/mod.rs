//! A Dauntless Reforged Revival Node-kit server upgraded in place to DaintyLass: the old `.workspace` layout's
//! folders moved, its MongoDB moved into the 1.14.7 database, `secrets.env` kept, and `host.json` turned into
//! `host.toml`. In that order, as DaintyLass's setup did it: a MongoDB move that fails leaves the old install as it
//! was. `dainty-host setup` runs next and writes the services' env files from the new host.toml.

pub mod hostjson;
pub mod layout;
pub mod mongo;
pub mod secrets;

use std::path::Path;

use serde_json::Value;

use crate::error::{ImportError, Result};
use crate::report::Report;
use layout::Layout;
use mongo::MongoPlan;

/// DaintyLass's `MAX_HOST_CONFIG_BYTES`
const MAX_HOST_CONFIG_BYTES: u64 = 256 * 1024;

fn read_host_json(file: &Path) -> Result<Option<String>> {
    let Ok(meta) = std::fs::metadata(file) else {
        return Ok(None);
    };
    if !meta.is_file() || meta.len() > MAX_HOST_CONFIG_BYTES {
        return Err(ImportError::invalid(format!(
            "{}: not a host.json of at most {MAX_HOST_CONFIG_BYTES} bytes",
            file.display()
        )));
    }
    std::fs::read_to_string(file)
        .map(Some)
        .map_err(|e| ImportError::io(file, e))
}

/// Writes host.toml beside host.json and keeps the old file as host.json.bak (DaintyLass's `save_host_config`).
fn switch_to_toml(l: &Layout, toml: &str) -> Result<()> {
    let partial = l.config.join("host.toml.partial");
    std::fs::write(&partial, toml).map_err(|e| ImportError::io(&partial, e))?;
    std::fs::rename(&partial, &l.host_toml).map_err(|e| ImportError::io(&l.host_toml, e))?;
    let bak = l.config.join("host.json.bak");
    if bak.exists() {
        std::fs::remove_file(&bak).map_err(|e| ImportError::io(&bak, e))?;
    }
    std::fs::rename(&l.host_json, &bak).map_err(|e| ImportError::io(&l.host_json, e))
}

/// Upgrades the old install whose checkout is at `root`.
pub fn upgrade(root: &Path, mongo_plan: &MongoPlan<'_>, dry_run: bool) -> Result<Report> {
    let l = Layout::new(root);
    let host_json = read_host_json(&l.host_json)?;
    if host_json.is_none() && !l.host_toml.is_file() {
        return Err(ImportError::invalid(format!(
            "{}: no .workspace/config/host.json or host.toml, so not a Reforged install",
            root.display()
        )));
    }
    // Checked before anything moves, so a bad file changes nothing
    let converted = match (&host_json, l.host_toml.is_file()) {
        (Some(text), false) => Some(hostjson::convert(text)?),
        _ => None,
    };
    let mongo_uri = host_json
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|doc| doc["mongo"]["uri"].as_str().map(str::to_owned));

    let mut report = Report::new("reforged", dry_run);
    mongo::upgrade(&l, mongo_uri.as_deref(), mongo_plan, dry_run, &mut report)?;
    layout::migrate_old_workspace(&l, dry_run, &mut report)?;
    secrets::check(&l.config.join("secrets.env"), dry_run, &mut report)?;

    match converted {
        Some(converted) => {
            if !dry_run {
                switch_to_toml(&l, &converted.toml)?;
            }
            report.count("host.toml");
            report.note("host.json is now host.toml; the old file is kept as host.json.bak");
            if !converted.dropped.is_empty() {
                report.note(format!(
                    "left out of host.toml (1.4.4, 1.12 or unknown): {}",
                    converted.dropped.join(", ")
                ));
            }
            if converted.was_legacy_only {
                report.note(
                    "this server ran only Dauntless 1.4.4: set game_dir in host.toml to a 1.14.7 install before setup",
                );
            }
        }
        None if l.host_toml.is_file() => {
            report.unchanged("host.toml");
            if host_json.is_some() {
                report
                    .note("host.toml is already there, so host.json was left as it is: delete it once you've checked");
            }
        }
        None => {}
    }
    for folder in layout::old_folders(&l) {
        report.note(format!(
            "{} is no longer used: delete it once you've checked what moved",
            folder.strip_prefix(&l.repo).unwrap_or(&folder).display()
        ));
    }
    report.note("next: run dainty-host setup, which checks host.toml and writes the services' settings");
    Ok(report)
}
