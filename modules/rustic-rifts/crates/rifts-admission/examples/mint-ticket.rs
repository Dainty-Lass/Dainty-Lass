//! Mints one MPT2 ticket with a key set's `metagame-admission.env`, for checking a transport by hand (RF7, RF8) and
//! for the Node cross-check:
//!
//!   cargo run -p rifts-admission --example mint-ticket -- --env <metagame-admission.env> --out <ticket file>
//!       [--account ID] [--joiner PEER] [--host PEER] [--session UUID] [--generation N] [--party ID]
//!       [--party-revision N] [--hunt ID] [--build N] [--bridge-protocol N] [--virtual-port N] [--lifetime SECONDS]
//!       [--now UNIX-SECONDS]
//!
//! The defaults are admissionTickets.test.ts' bindings. Writes the ticket, owner-only where the platform has modes,
//! and prints only its jti digest and expiry.

use std::collections::HashMap;
use std::io::Write as _;
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_admission::{Es256Tickets, SessionUuid, TicketBindings};
use rifts_seam::{AccountId, PeerId};

fn argument(name: &str) -> Option<String> {
    let mut args = std::env::args();
    args.position(|arg| arg == format!("--{name}"))?;
    args.next()
}

fn number<T: FromStr>(name: &str, default: T) -> Result<T, String> {
    argument(name).map_or(Ok(default), |value| {
        value.parse().map_err(|_| format!("--{name} is not a number"))
    })
}

fn read_env(path: &str) -> Result<HashMap<String, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    Ok(text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect())
}

fn run() -> Result<(), String> {
    let (Some(env), Some(out)) = (argument("env"), argument("out")) else {
        return Err("Usage: mint-ticket --env <metagame-admission.env> --out <ticket file> [bindings]".to_owned());
    };
    let tickets = Es256Tickets::from_settings(&read_env(&env)?).map_err(|error| error.to_string())?;
    let session = argument("session").unwrap_or_else(|| "01234567-89ab-4def-8123-456789abcdef".to_owned());
    let bindings = TicketBindings {
        account: AccountId(argument("account").unwrap_or_else(|| "account-1".to_owned())),
        joiner_peer: PeerId(number("joiner", 76_561_198_000_000_001)?),
        host_peer: PeerId(number("host", 76_561_198_000_000_002)?),
        session: SessionUuid::from_str(&session).map_err(|error| error.to_string())?,
        generation: number("generation", 7)?,
        party_id: argument("party").unwrap_or_else(|| "party_abc==".to_owned()),
        party_revision: number("party-revision", 12)?,
        hunt_definition_id: argument("hunt").unwrap_or_else(|| "hunt/island-1".to_owned()),
        build: number("build", 392_819)?,
        bridge_protocol: number("bridge-protocol", 1)?,
        virtual_port: number("virtual-port", 1)?,
    };
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let now = number("now", clock)?;
    let lifetime = argument("lifetime")
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| "--lifetime is not a number".to_owned())
        })
        .transpose()?;
    let minted = tickets
        .mint(bindings, now, lifetime)
        .map_err(|error| error.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&out).map_err(|error| format!("{out}: {error}"))?;
    file.write_all(minted.text.expose().as_bytes())
        .map_err(|error| format!("{out}: {error}"))?;
    println!(
        "jti digest {} expires at {}",
        minted.jti_digest, minted.claims.expires_at
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
