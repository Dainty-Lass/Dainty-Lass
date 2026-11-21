// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Writes ten-minute turn-rest credentials for the transport's relay self-test, from the backend's env file:
//! `mint-relay-file <output file> <backend env file>`. Prints no secret.

use std::{collections::HashMap, fs, io::Write as _, process::ExitCode, time::SystemTime};

use rifts_relay::{RelayConfig, TurnProvider, mint_turn_rest};
use rifts_seam::{AccountId, UnixMillis};

// Only the relay tag in the username depends on it.
const SELF_TEST_ACCOUNT: &str = "rustic-selftest";
const TTL_SECONDS: u32 = 600;

/// `KEY=value` lines; blank lines and `#` comments skipped, one pair of matching quotes taken off a value.
fn parse_env(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| {
            let value = value.trim();
            let inner = ["\"", "'"]
                .into_iter()
                .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
                .unwrap_or(value);
            (name.trim().to_owned(), inner.to_owned())
        })
        .collect()
}

/// Truncates or creates `path`, owner-only on Unix.
fn write_owner_only(path: &str, contents: &str) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(contents.as_bytes())
}

/// The self-test speaks UDP to the relay: `turn:` URLs with no transport or `?transport=udp`.
fn speaks_udp(url: &str) -> bool {
    url.starts_with("turn:") && (url.ends_with("?transport=udp") || !url.contains('?'))
}

fn run(args: &[String]) -> Result<String, String> {
    let [output, env_file] = args else {
        return Err("usage: mint-relay-file <output file> <backend env file>".to_owned());
    };
    let contents = fs::read_to_string(env_file).map_err(|error| format!("{env_file}: {error}"))?;
    let config = RelayConfig::from_settings(&parse_env(&contents));
    let turn_rest = match (config.provider, config.turn_rest.as_ref()) {
        (TurnProvider::TurnRest, Some(turn_rest)) => turn_rest,
        _ => {
            return Err(format!(
                "P2P_TURN_PROVIDER=turn-rest, P2P_TURN_REST_URLS and a P2P_TURN_REST_SECRET of 16 or more characters are not all set in {env_file}"
            ));
        }
    };
    let elapsed = SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default();
    let now = UnixMillis(elapsed.as_millis().try_into().unwrap_or(i64::MAX));
    let minted = mint_turn_rest(turn_rest, &AccountId(SELF_TEST_ACCOUNT.into()), TTL_SECONDS, now);
    let udp: Vec<&str> = minted
        .urls
        .iter()
        .map(String::as_str)
        .filter(|url| speaks_udp(url))
        .collect();
    if udp.is_empty() {
        return Err("P2P_TURN_REST_URLS names no UDP relay (turn:host:port or ?transport=udp)".to_owned());
    }
    let lines = [
        ("ice_stun_urls", config.stun_urls.join(",")),
        ("ice_turn_urls", udp.join(",")),
        ("turn_username", minted.username.clone()),
        ("turn_credential", minted.credential.expose().to_owned()),
    ];
    let file: String = lines.iter().map(|(name, value)| format!("{name}={value}\n")).collect();
    write_owner_only(output, &file).map_err(|error| format!("{output}: {error}"))?;
    Ok(format!(
        "Wrote relay test file (10-minute turn-rest credentials, {} UDP relay URLs).",
        udp.len()
    ))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}
