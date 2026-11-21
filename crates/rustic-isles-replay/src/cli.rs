// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The standalone command line (spec §10.5): what restricted's `isles-db replay` calls until unit 11.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::options::{ReplayOptions, run_replay_cli};

/// The help text `--help` prints.
pub const USAGE: &str = "\
Usage: replay --corpus <path> --left <url|recorded> --right <url> [options]

Replay Options:
  --corpus <path>               Capture file, or a directory of *.jsonl capture files
  --left <url|recorded>         Left side: a backend base URL, or recorded for the capture's own answers
  --right <url>                 Right side: a backend base URL
  --left-director <url>         Director base URL for the left side's director records
  --right-director <url>        Director base URL for the right side (director records are skipped without it)
  --left-chat <host:port>       Plain XMPP-over-TCP address to replay the left side's chat against
  --right-chat <host:port>      Plain XMPP-over-TCP address to replay the right side's chat against
  --jwt-secret <secret>         HS256 secret for both sides' minted tokens
  --left-jwt-secret <secret>    HS256 secret for the left side
  --right-jwt-secret <secret>   HS256 secret for the right side
  --signing-key <base64>        RS256 private key (AUTH_SIGNING_PRIVKEY_B64 form) for both sides
  --left-signing-key <base64>   RS256 private key for the left side
  --right-signing-key <base64>  RS256 private key for the right side
  --gs-key <key>                Game-server key for both sides' <gskey> placeholders
  --left-gs-key <key>           Game-server key for the left side
  --right-gs-key <key>          Game-server key for the right side
  --gateway-secret <secret>     Sent as x-gateway-secret to both sides
  --key-map <path>              JSON map of raw account keys for <key:...> placeholders
  --pin-clock                   Pin each request's clock and id seed with the replay headers (default)
  --no-pin-clock                Send no replay headers
  --filter <substring>          Replay only HTTP records whose target or route contains it
  --out <file>                  Write the JSON report to this file
  --json                        Print the JSON report instead of the summary
  -h, --help                    Show this help
";

/// A parsed command line.
#[derive(Debug, Clone)]
pub enum CliCommand {
    /// `-h`/`--help`: print [`USAGE`].
    Help,
    /// Replay with these options.
    Run(Box<CliArgs>),
}

/// A replay run's options and output format.
#[derive(Debug, Clone)]
pub struct CliArgs {
    /// The run's options.
    pub options: ReplayOptions,
    /// Print the report as JSON (`--json`).
    pub json: bool,
}

fn value_error(flag: &str) -> String {
    let what = match flag {
        "--corpus" | "--key-map" => "a path",
        "--left" => "a URL or 'recorded'",
        "--right" | "--left-director" | "--right-director" => "a URL",
        "--left-chat" | "--right-chat" => "a host:port",
        "--jwt-secret" | "--left-jwt-secret" | "--right-jwt-secret" | "--gateway-secret" => "a secret",
        "--signing-key" | "--left-signing-key" | "--right-signing-key" => "a base64 key",
        "--gs-key" | "--left-gs-key" | "--right-gs-key" => "a key",
        "--filter" => "a substring",
        _ => "a file path",
    };
    format!("{flag} requires {what}")
}

/// Walks the arguments in order (a leading `replay` is the subcommand's own name). Both-sides secret flags set both
/// sides where they stand, so the last flag wins (§15 item 15).
pub fn parse_args(args: &[String]) -> Result<CliCommand, String> {
    let args = match args.first() {
        Some(first) if first == "replay" => &args[1..],
        _ => args,
    };
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(CliCommand::Help);
    }
    let mut options = ReplayOptions::default();
    let (mut corpus, mut left, mut right) = (None, None, None);
    let (mut pin, mut no_pin, mut json) = (false, false, false);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--pin-clock" => pin = true,
            "--no-pin-clock" => no_pin = true,
            "--json" => json = true,
            name @ ("--corpus"
            | "--left"
            | "--right"
            | "--left-director"
            | "--right-director"
            | "--left-chat"
            | "--right-chat"
            | "--jwt-secret"
            | "--left-jwt-secret"
            | "--right-jwt-secret"
            | "--signing-key"
            | "--left-signing-key"
            | "--right-signing-key"
            | "--gs-key"
            | "--left-gs-key"
            | "--right-gs-key"
            | "--gateway-secret"
            | "--key-map"
            | "--filter"
            | "--out") => {
                let value = iter.next().ok_or_else(|| value_error(name))?.clone();
                let o = &mut options;
                match name {
                    "--corpus" => corpus = Some(PathBuf::from(value)),
                    "--left" => left = Some(value),
                    "--right" => right = Some(value),
                    "--left-director" => o.left_director = Some(value),
                    "--right-director" => o.right_director = Some(value),
                    "--left-chat" => o.left_chat = Some(value),
                    "--right-chat" => o.right_chat = Some(value),
                    "--jwt-secret" => {
                        o.left_jwt_secret = Some(value.clone());
                        o.right_jwt_secret = Some(value);
                    }
                    "--left-jwt-secret" => o.left_jwt_secret = Some(value),
                    "--right-jwt-secret" => o.right_jwt_secret = Some(value),
                    "--signing-key" => {
                        o.left_signing_key = Some(value.clone());
                        o.right_signing_key = Some(value);
                    }
                    "--left-signing-key" => o.left_signing_key = Some(value),
                    "--right-signing-key" => o.right_signing_key = Some(value),
                    "--gs-key" => {
                        o.left_gs_key = Some(value.clone());
                        o.right_gs_key = Some(value);
                    }
                    "--left-gs-key" => o.left_gs_key = Some(value),
                    "--right-gs-key" => o.right_gs_key = Some(value),
                    "--gateway-secret" => o.gateway_secret = Some(value),
                    "--key-map" => o.key_map_path = Some(PathBuf::from(value)),
                    "--filter" => o.filter = Some(value),
                    _ => o.out_path = Some(PathBuf::from(value)),
                }
            }
            other => return Err(format!("unexpected flag \"{other}\"")),
        }
    }
    options.corpus = corpus.ok_or("missing required --corpus <path>")?;
    options.left = left.ok_or("missing required --left <url|recorded>")?;
    options.right = right.ok_or("missing required --right <url>")?;
    options.pin_clock = !no_pin || pin;
    Ok(CliCommand::Run(Box::new(CliArgs { options, json })))
}

fn program_name(arg0: Option<&String>) -> String {
    arg0.map(std::path::Path::new)
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "isles-db".into())
}

/// `args` as `std::env::args` gives them (the program first, then `replay` and its flags). Exit 0 for help or a
/// passing run, 1 for a run error, 2 for a usage error or unexplained differences.
pub fn main_from<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let code = run_with_output(&args, &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(code)
}

/// [`main_from`] writing to the given streams and answering the exit code.
pub fn run_with_output(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let prefix = format!("{} replay", program_name(args.first()));
    let rest = args.get(1..).unwrap_or_default();
    let cli = match parse_args(rest) {
        Ok(CliCommand::Help) => {
            let _ = write!(out, "{USAGE}");
            return 0;
        }
        Ok(CliCommand::Run(cli)) => cli,
        Err(message) => {
            let _ = writeln!(
                err,
                "error: {message}

{USAGE}"
            );
            return 2;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = writeln!(err, "{prefix} failed: could not start runtime: {e}");
            return 1;
        }
    };
    let report = match runtime.block_on(run_replay_cli(&cli.options)) {
        Ok(report) => report,
        Err(e) => {
            let _ = writeln!(err, "{prefix} failed: {e}");
            return 1;
        }
    };
    if cli.json {
        match report.to_json_pretty() {
            Ok(json) => {
                let _ = writeln!(out, "{json}");
            }
            Err(e) => {
                let _ = writeln!(err, "{prefix} failed: {e}");
                return 1;
            }
        }
    } else {
        let _ = write!(out, "{}", report.to_text(&format!("{prefix}:")));
    }
    if report.passed() { 0 } else { 2 }
}
