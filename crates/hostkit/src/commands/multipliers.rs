// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::path::Path;
use std::process::ExitCode;

use crate::cli::MultipliersArgs;
use crate::commands::setup::secrets;
use crate::error::HostError;
use crate::hostconfig::{
    Platform, build_env_files, load_host_config, load_or_create_secrets, multipliers_on, save_host_config,
    write_env_files,
};
use crate::paths::Paths;

#[must_use]
pub fn asked(args: &MultipliersArgs) -> bool {
    args.xp.is_some() || args.rewards.is_some() || args.off
}

fn format_mult(n: f64) -> String {
    service_kit::jsjson::number(n)
}

pub fn run(paths: &Paths, args: &MultipliersArgs) -> Result<ExitCode, HostError> {
    let source_commit = |repo: &Path| -> Result<String, HostError> {
        Ok(crate::selfupdate::read_manifest(repo)
            .ok()
            .flatten()
            .map(|m| m.short_commit().to_owned())
            .or_else(|| {
                crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(repo))
                    .ok()
                    .flatten()
            })
            .unwrap_or_default())
    };
    run_with(paths, args, Platform::current(), &source_commit, &mut |line| {
        println!("{line}")
    })
}

pub fn run_with(
    paths: &Paths,
    args: &MultipliersArgs,
    platform: Platform,
    source_commit: &dyn Fn(&Path) -> Result<String, HostError>,
    out: &mut dyn FnMut(&str),
) -> Result<ExitCode, HostError> {
    let mut config =
        load_host_config(paths)?.ok_or_else(|| HostError::message("not set up yet: run dainty-host setup first"))?;

    if !asked(args) {
        if let Some(m) = config.multipliers.filter(|_| multipliers_on(&config)) {
            out(&format!(
                "Test multipliers ON: XP x{}, rewards x{}.",
                format_mult(m.xp),
                format_mult(m.rewards)
            ));
        } else {
            out("Test multipliers off.");
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut current = config.multipliers.unwrap_or_default();
    if args.off {
        config.multipliers = None;
    } else {
        if let Some(ref xp_raw) = args.xp {
            current.xp = parse_multiplier("--xp", xp_raw)?;
        }
        if let Some(ref rewards_raw) = args.rewards {
            current.rewards = parse_multiplier("--rewards", rewards_raw)?;
        }
        if (current.xp - 1.0).abs() < f64::EPSILON && (current.rewards - 1.0).abs() < f64::EPSILON {
            config.multipliers = None;
        } else {
            config.multipliers = Some(current);
        }
    }

    save_host_config(paths, &config)?;
    if paths.config.join("backend.env").is_file() {
        let (sec, _) = load_or_create_secrets(&paths.config, secrets::new_secrets)?;
        let commit = source_commit(&paths.repo)?;
        let env_files = build_env_files(&config, &sec, paths, &commit, platform);
        write_env_files(paths, &env_files)?;
    }

    if let Some(m) = config.multipliers {
        out(&format!(
            "Test multipliers ON: XP x{}, rewards x{}.",
            format_mult(m.xp),
            format_mult(m.rewards)
        ));
    } else {
        out("Test multipliers off.");
    }
    out("Restart the server if it is running so the backend picks up the change.");

    Ok(ExitCode::SUCCESS)
}

fn parse_multiplier(flag: &str, raw: &str) -> Result<f64, HostError> {
    let trimmed = raw.trim();
    let num_str = trimmed
        .strip_prefix('x')
        .or_else(|| trimmed.strip_prefix('X'))
        .unwrap_or(trimmed);
    let value = num_str
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && (1.0..=100.0).contains(v))
        .ok_or_else(|| HostError::message(format!("{flag} must be a number from 1 to 100 (got {raw})")))?;
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
            no_multipliers: false,
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
            no_multipliers: false,
        };
        run(&paths, &off_args).unwrap();
        let after_off = load_host_config(&paths).unwrap().unwrap();
        assert!(!multipliers_on(&after_off));
    }
}
