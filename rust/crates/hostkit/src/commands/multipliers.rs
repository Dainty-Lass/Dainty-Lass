// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::process::ExitCode;

use crate::cli::MultipliersArgs;
use crate::commands::setup::secrets;
use crate::error::HostError;
use crate::hostconfig::{
    Multipliers, Platform, build_env_files, load_host_config, load_or_create_secrets,
    multipliers_on, save_host_config, write_env_files,
};
use crate::paths::Paths;

pub fn run(paths: &Paths, args: &MultipliersArgs) -> Result<ExitCode, HostError> {
    let mut config = load_host_config(paths)?.ok_or_else(|| {
        HostError::message("server is not configured yet: run reforged-host setup first")
    })?;

    let mut current = config.multipliers.unwrap_or_default();
    let mut changed = false;

    if args.off {
        current = Multipliers::default();
        config.multipliers = None;
        changed = true;
    } else {
        if let Some(ref xp_raw) = args.xp {
            current.xp = parse_multiplier("--xp", xp_raw)?;
            changed = true;
        }
        if let Some(ref rewards_raw) = args.rewards {
            current.rewards = parse_multiplier("--rewards", rewards_raw)?;
            changed = true;
        }
        if changed {
            if (current.xp - 1.0).abs() < f64::EPSILON
                && (current.rewards - 1.0).abs() < f64::EPSILON
            {
                config.multipliers = None;
            } else {
                config.multipliers = Some(current);
            }
        }
    }

    if changed {
        save_host_config(paths, &config)?;
        if paths.config.join("backend.env").is_file() {
            let (sec, _) = load_or_create_secrets(&paths.config, secrets::new_secrets)?;
            let commit = crate::selfupdate::read_manifest(&paths.repo)
                .ok()
                .flatten()
                .map(|m| m.short_commit().to_owned())
                .unwrap_or_default();
            let env_files = build_env_files(&config, &sec, paths, &commit, Platform::current());
            write_env_files(paths, &env_files)?;
        }
        println!(
            "Multipliers updated: XP = {}x, rewards = {}x. Restart the server if it is running.",
            current.xp, current.rewards
        );
    } else {
        let state = if multipliers_on(&config) {
            "active"
        } else {
            "default (1x)"
        };
        println!(
            "Multipliers ({state}): XP = {}x, rewards = {}x",
            current.xp, current.rewards
        );
    }

    Ok(ExitCode::SUCCESS)
}

fn parse_multiplier(flag: &str, raw: &str) -> Result<f64, HostError> {
    let value = raw
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 1000.0)
        .ok_or_else(|| {
            HostError::message(format!(
                "{flag} must be a number greater than 0 and at most 1000 (got {raw:?})"
            ))
        })?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::hostconfig::HostConfig;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn updates_and_turns_off_multipliers() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let cfg = HostConfig::from_json(&json!({
            "mode": "private",
            "host": "100.64.0.1",
            "gameAddress": "100.64.0.1",
            "name": "Test",
            "registration": "OPEN",
            "maxHuntServers": 4
        }))
        .unwrap();
        save_host_config(&paths, &cfg).unwrap();

        let set_args = MultipliersArgs {
            xp: Some("2.5".to_owned()),
            rewards: Some("3".to_owned()),
            off: false,
        };
        run(&paths, &set_args).unwrap();
        let updated = load_host_config(&paths).unwrap().unwrap();
        let mult = updated.multipliers.unwrap();
        assert_eq!(mult.xp, 2.5);
        assert_eq!(mult.rewards, 3.0);

        let off_args = MultipliersArgs {
            xp: None,
            rewards: None,
            off: true,
        };
        run(&paths, &off_args).unwrap();
        let after_off = load_host_config(&paths).unwrap().unwrap();
        assert!(!multipliers_on(&after_off));
    }

    #[test]
    fn rejects_invalid_multiplier_values() {
        assert!(parse_multiplier("--xp", "0").is_err());
        assert!(parse_multiplier("--rewards", "1001").is_err());
        assert!(parse_multiplier("--xp", "abc").is_err());
        assert_eq!(parse_multiplier("--xp", "2.0").unwrap(), 2.0);
    }
}
