//! `tools/mint-relay-test-file.cjs` for the `turn-rest` provider: 10-minute credentials from the backend's env file
//! (`P2P_TURN_PROVIDER=turn-rest`, `P2P_TURN_REST_URLS`, `P2P_TURN_REST_SECRET`, `P2P_ICE_STUN_URLS`), written for
//! `ParadoxTransportTest ice --relay-file <file>` against turn-rs or coturn (RF8). Prints no secret.
//!
//!   cargo run -p rifts-relay --example mint-relay-file -- <output file> <backend env file>

use std::collections::HashMap;
use std::io::Write as _;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_relay::{RelayConfig, TurnProvider, mint_turn_rest};
use rifts_seam::{AccountId, UnixMillis};

fn run() -> Result<String, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [output, env_file] = args.as_slice() else {
        return Err("usage: mint-relay-file <output file> <backend env file>".to_owned());
    };
    let text = std::fs::read_to_string(env_file).map_err(|error| format!("{env_file}: {error}"))?;
    // The cjs tool's reader: KEY=value, # comments, one pair of surrounding quotes
    let settings: HashMap<String, String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| {
            let value = value.trim();
            let unquoted = [('"', '"'), ('\'', '\'')]
                .iter()
                .find_map(|(open, close)| value.strip_prefix(*open)?.strip_suffix(*close))
                .unwrap_or(value);
            (key.trim().to_owned(), unquoted.to_owned())
        })
        .collect();
    let config = RelayConfig::from_settings(&settings);
    let rest = match (config.provider, &config.turn_rest) {
        (TurnProvider::TurnRest, Some(rest)) => rest,
        _ => {
            return Err(format!(
                "P2P_TURN_PROVIDER=turn-rest, P2P_TURN_REST_URLS and a P2P_TURN_REST_SECRET of 16 or more characters \
                 are not all set in {env_file}"
            ));
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
    let credential = mint_turn_rest(rest, &AccountId("paradox-selftest".to_owned()), 600, UnixMillis(now));
    // The transport's relay test takes UDP relays only, as the cjs tool filters them
    let udp: Vec<&str> = credential
        .urls
        .iter()
        .map(String::as_str)
        .filter(|url| url.starts_with("turn:") && (!url.contains('?') || url.ends_with("?transport=udp")))
        .collect();
    if udp.is_empty() {
        return Err("P2P_TURN_REST_URLS names no UDP relay (turn:host:port or ?transport=udp)".to_owned());
    }
    let file = format!(
        "ice_stun_urls={}\nice_turn_urls={}\nturn_username={}\nturn_credential={}\n",
        config.stun_urls.join(","),
        udp.join(","),
        credential.username,
        credential.credential.expose()
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(output)
        .and_then(|mut handle| handle.write_all(file.as_bytes()))
        .map_err(|error| format!("{output}: {error}"))?;
    Ok(format!(
        "Wrote relay test file (10-minute turn-rest credentials, {} UDP relay URLs).",
        udp.len()
    ))
}

fn main() -> ExitCode {
    match run() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
