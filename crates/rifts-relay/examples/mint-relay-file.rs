// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Writes ten-minute turn-rest credentials for the transport's relay self-test, from the backend's env file:
//! `mint-relay-file <output file> <backend env file>`. Prints no secret.

use std::collections::HashMap;
use std::io::Write as _;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_relay::{RelayConfig, TurnProvider, mint_turn_rest};
use rifts_seam::{AccountId, UnixMillis};

// Only the relay tag in the username depends on it.
const SELF_TEST_ACCOUNT: &str = "rustic-selftest";
const TTL_SECONDS: u32 = 600;

fn read_env(text: &str) -> HashMap<String, String> {
    let mut settings = HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let unquoted = ['"', '\'']
            .iter()
            .find_map(|quote| value.strip_prefix(*quote).and_then(|rest| rest.strip_suffix(*quote)))
            .unwrap_or(value);
        settings.insert(key.trim().to_owned(), unquoted.to_owned());
    }
    settings
}

fn write_private(path: &str, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [output, env_file] = args.as_slice() else {
        return fail("usage: mint-relay-file <output file> <backend env file>");
    };
    let text = match std::fs::read_to_string(env_file) {
        Ok(text) => text,
        Err(error) => return fail(&format!("{env_file}: {error}")),
    };
    let config = RelayConfig::from_settings(&read_env(&text));
    let (TurnProvider::TurnRest, Some(turn_rest)) = (config.provider, &config.turn_rest) else {
        return fail(&format!(
            "P2P_TURN_PROVIDER=turn-rest, P2P_TURN_REST_URLS and a P2P_TURN_REST_SECRET of 16 or more characters are not all set in {env_file}"
        ));
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let now = UnixMillis(i64::try_from(now).unwrap_or(i64::MAX));
    let credential = mint_turn_rest(turn_rest, &AccountId(SELF_TEST_ACCOUNT.to_owned()), TTL_SECONDS, now);
    // The self-test speaks UDP to the relay.
    let udp: Vec<&str> = credential
        .urls
        .iter()
        .map(String::as_str)
        .filter(|url| url.starts_with("turn:") && (!url.contains('?') || url.ends_with("?transport=udp")))
        .collect();
    if udp.is_empty() {
        return fail("P2P_TURN_REST_URLS names no UDP relay (turn:host:port or ?transport=udp)");
    }
    let file = format!(
        "ice_stun_urls={}\nice_turn_urls={}\nturn_username={}\nturn_credential={}\n",
        config.stun_urls.join(","),
        udp.join(","),
        credential.username,
        credential.credential.expose()
    );
    if let Err(error) = write_private(output, &file) {
        return fail(&format!("{output}: {error}"));
    }
    println!(
        "Wrote relay test file (10-minute turn-rest credentials, {} UDP relay URLs).",
        udp.len()
    );
    ExitCode::SUCCESS
}
