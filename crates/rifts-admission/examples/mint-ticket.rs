// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `mint-ticket --env <metagame-admission.env> --out <ticket file> [bindings]`: a signed ticket for a transport test.
//! Every binding has a default; the ticket goes to the file, never to the terminal.

use std::{collections::HashMap, fs, io::Write as _, process::ExitCode, str::FromStr, time::SystemTime};

use rifts_admission::{Es256Tickets, SessionUuid, TicketBindings};
use rifts_seam::{AccountId, PeerId};

/// The command line as `--flag value` pairs; the first occurrence of a flag wins.
struct Flags(Vec<String>);

impl Flags {
    fn value(&self, flag: &str) -> Option<&str> {
        let pair = self.0.windows(2).find(|pair| pair[0] == flag)?;
        Some(&pair[1])
    }

    fn or(&self, flag: &str, fallback: &str) -> String {
        self.value(flag).map_or_else(|| fallback.to_owned(), str::to_owned)
    }

    fn parsed<T: FromStr>(&self, flag: &str, fallback: T) -> Result<T, String> {
        let Some(text) = self.value(flag) else {
            return Ok(fallback);
        };
        T::from_str(text).map_err(|_| format!("{flag} is not a number"))
    }
}

/// `KEY=value` lines, split at the first `=` and trimmed; nothing else is interpreted.
fn read_env(path: &str) -> Result<HashMap<String, String>, String> {
    let contents = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let mut settings = HashMap::new();
    for line in contents.lines() {
        if let Some((name, value)) = line.split_once('=') {
            settings.insert(name.trim().to_owned(), value.trim().to_owned());
        }
    }
    Ok(settings)
}

fn bindings(flags: &Flags) -> Result<TicketBindings, String> {
    let session = flags.or("--session", "01234567-89ab-4def-8123-456789abcdef");
    Ok(TicketBindings {
        session: SessionUuid::from_str(&session).map_err(|error| error.to_string())?,
        account: AccountId(flags.or("--account", "account-1")),
        party_id: flags.or("--party", "party_abc=="),
        hunt_definition_id: flags.or("--hunt", "hunt/island-1"),
        joiner_peer: PeerId(flags.parsed("--joiner", 76_561_198_000_000_001)?),
        host_peer: PeerId(flags.parsed("--host", 76_561_198_000_000_002)?),
        generation: flags.parsed("--generation", 7)?,
        party_revision: flags.parsed("--party-revision", 12)?,
        build: flags.parsed("--build", 392_819)?,
        bridge_protocol: flags.parsed("--bridge-protocol", 1)?,
        virtual_port: flags.parsed("--virtual-port", 1)?,
    })
}

/// Truncates or creates `path`, owner-only on Unix, and writes `text` to it.
fn write_private(path: &str, text: &str) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let failed = |error: std::io::Error| format!("{path}: {error}");
    options
        .open(path)
        .map_err(failed)?
        .write_all(text.as_bytes())
        .map_err(failed)
}

fn run(flags: &Flags, env: &str, out: &str) -> Result<String, String> {
    let authority = Es256Tickets::from_settings(&read_env(env)?).map_err(|error| error.to_string())?;
    let bindings = bindings(flags)?;
    let clock = SystemTime::UNIX_EPOCH.elapsed().map_or(0, |elapsed| elapsed.as_secs());
    let now = flags.parsed("--now", clock)?;
    let lifetime = match flags.value("--lifetime") {
        Some(_) => Some(flags.parsed::<u32>("--lifetime", 0)?),
        None => None,
    };
    let minted = authority
        .mint(bindings, now, lifetime)
        .map_err(|error| error.to_string())?;
    write_private(out, minted.text.expose())?;
    Ok(format!(
        "jti digest {} expires at {}",
        minted.jti_digest, minted.claims.expires_at
    ))
}

fn main() -> ExitCode {
    let flags = Flags(std::env::args().skip(1).collect());
    let (Some(env), Some(out)) = (flags.value("--env"), flags.value("--out")) else {
        eprintln!("Usage: mint-ticket --env <metagame-admission.env> --out <ticket file> [bindings]");
        return ExitCode::FAILURE;
    };
    let outcome = run(&flags, env, out);
    match &outcome {
        Ok(summary) => println!("{summary}"),
        Err(error) => eprintln!("{error}"),
    }
    if outcome.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
