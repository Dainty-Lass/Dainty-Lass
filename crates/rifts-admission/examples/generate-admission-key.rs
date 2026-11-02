// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `generate-admission-key --output <new-directory> --key-id <safe-id>`: a host's new admission key set. Prints no key.

use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_admission::{KeyId, write_key_set};

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let at = args.iter().position(|arg| arg == name)?;
    args.get(at + 1).map(String::as_str)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(output), Some(key_id)) = (flag(&args, "--output"), flag(&args, "--key-id")) else {
        eprintln!("Usage: generate-admission-key --output <new-directory> --key-id <safe-id>");
        return ExitCode::from(2);
    };
    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX));
    match write_key_set(
        Path::new(output),
        KeyId(key_id.to_owned()),
        std::process::id(),
        now_millis,
    ) {
        Ok(_) => {
            println!("Generated a new P-256 admission key set in: {output}");
            println!(
                "Keep the private PEM and metagame-admission.env only on the server: the signing key goes to secrets.env."
            );
            println!("Publish p2p-admission-public.xy with the runtime as an extra file (Guard role ADMISSION_KEY).");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
