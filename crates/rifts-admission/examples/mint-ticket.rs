// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `mint-ticket --env <metagame-admission.env> --out <ticket file> [bindings]`: a signed ticket for a transport test.
//! Every binding has a default; the ticket goes to the file, never to the terminal.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_admission::{Es256Tickets, SessionUuid, TicketBindings};
use rifts_seam::{AccountId, PeerId};

struct Args(Vec<String>);

impl Args {
    fn get(&self, name: &str) -> Option<&str> {
        let at = self.0.iter().position(|arg| arg == name)?;
        self.0.get(at + 1).map(String::as_str)
    }

    fn text(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or(default).to_owned()
    }

    fn number<T: FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.get(name) {
            None => Ok(default),
            Some(text) => text.parse().map_err(|_| format!("{name} is not a number")),
        }
    }
}

/// `KEY=value` lines, split at the first `=` and trimmed; nothing else is interpreted.
fn read_env(path: &str) -> Result<HashMap<String, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    Ok(text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect())
}

fn run(args: &Args, env: &str, out: &str) -> Result<String, String> {
    let settings = read_env(env)?;
    let tickets = Es256Tickets::from_settings(&settings).map_err(|error| error.to_string())?;
    let session: SessionUuid = args
        .text("--session", "01234567-89ab-4def-8123-456789abcdef")
        .parse()
        .map_err(|error: rifts_admission::TicketError| error.to_string())?;
    let bindings = TicketBindings {
        account: AccountId(args.text("--account", "account-1")),
        joiner_peer: PeerId(args.number("--joiner", 76_561_198_000_000_001)?),
        host_peer: PeerId(args.number("--host", 76_561_198_000_000_002)?),
        session,
        generation: args.number("--generation", 7)?,
        party_id: args.text("--party", "party_abc=="),
        party_revision: args.number("--party-revision", 12)?,
        hunt_definition_id: args.text("--hunt", "hunt/island-1"),
        build: args.number("--build", 392_819)?,
        bridge_protocol: args.number("--bridge-protocol", 1)?,
        virtual_port: args.number("--virtual-port", 1)?,
    };
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let now = args.number("--now", clock)?;
    let lifetime = match args.get("--lifetime") {
        None => None,
        Some(_) => Some(args.number::<u32>("--lifetime", 0)?),
    };
    let minted = tickets
        .mint(bindings, now, lifetime)
        .map_err(|error| error.to_string())?;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(out).map_err(|error| format!("{out}: {error}"))?;
    file.write_all(minted.text.expose().as_bytes())
        .map_err(|error| format!("{out}: {error}"))?;
    Ok(format!(
        "jti digest {} expires at {}",
        minted.jti_digest, minted.claims.expires_at
    ))
}

fn main() -> ExitCode {
    let args = Args(std::env::args().skip(1).collect());
    let (Some(env), Some(out)) = (args.get("--env"), args.get("--out")) else {
        eprintln!("Usage: mint-ticket --env <metagame-admission.env> --out <ticket file> [bindings]");
        return ExitCode::FAILURE;
    };
    match run(&args, env, out) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
