// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `progression_config.json` from the host's progression source (spec §5.1).

use std::path::Path;

use serde_json::{Map, Value};

use super::io::{back_up, create_dir, drop_bom, parse, read_text, show, write_file};
use super::pretty::stringify_pretty;

/// The source read with one BOM dropped: an unreadable file also says "not valid JSON".
pub(crate) fn read_source(path: &Path, what: &str) -> Result<Value, String> {
    if !path.is_file() {
        return Err(format!(
            "ERROR: source not found at {}\nProvide the {what} payload from your own installation (see \
             docs/GENERATING_GAME_DATA.md).",
            show(path)
        ));
    }
    let invalid = |e: String| format!("ERROR: source is not valid JSON: SyntaxError: {e}");
    let text = read_text(path).map_err(|e| invalid(e.to_string()))?;
    parse(drop_bom(&text)).map_err(invalid)
}

/// Generates `progression_config.json` (§5.2) from `progression_config.source.json` in `raw_dir`; without `apply` it is a dry run that only reports.
pub fn generate_progression_config(raw_dir: &Path, out_dir: &Path, apply: bool) -> Result<String, String> {
    let source = read_source(&raw_dir.join("progression_config.source.json"), "progression")?;
    let Some(paths) = source.pointer("/payload/paths").and_then(Value::as_array) else {
        return Err(
            "ERROR: source has no payload.paths[] array — refusing to write a shape the backend cannot read."
                .to_owned(),
        );
    };
    let tiers: usize = paths
        .iter()
        .map(|p| p.get("free_rewards").and_then(Value::as_array).map_or(0, Vec::len))
        .sum();
    let count = paths.len();
    let mut out = Map::new();
    out.insert("code".into(), source.get("code").cloned().unwrap_or(Value::Null));
    let message = source.get("message").filter(|m| m.is_string()).cloned();
    out.insert("message".into(), message.unwrap_or_else(|| Value::from("OK")));
    out.insert("payload".into(), source.get("payload").cloned().unwrap_or(Value::Null));

    let mut report = vec![
        format!("source paths      : {count}"),
        format!("free_rewards tiers : {tiers}"),
    ];
    if !apply {
        report.push("mode              : DRY RUN (pass --apply to write)".into());
        report.push(String::new());
        report.push("Dry run complete. Nothing written.".into());
        return Ok(report.join("\n"));
    }
    report.push("mode              : APPLY".into());
    create_dir(out_dir)?;
    let target = out_dir.join("progression_config.json");
    back_up(&target)?;
    write_file(&target, &stringify_pretty(&Value::Object(out), 2))?;
    report.push(String::new());
    report.push(format!("Wrote progression_config.json ({count} paths)"));
    Ok(report.join("\n"))
}
