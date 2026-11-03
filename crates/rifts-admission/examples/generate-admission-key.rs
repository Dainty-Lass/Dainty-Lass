// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `generate-admission-key --output <new-directory> --key-id <safe-id>`: a host's new admission key set. Prints no key.

use std::{path::Path, process::ExitCode, time::SystemTime};

const USAGE: &str = "Usage: generate-admission-key --output <new-directory> --key-id <safe-id>";
const AFTERWARDS: [&str; 2] = [
    "Keep the private PEM and metagame-admission.env only on the server: the signing key goes to secrets.env.",
    "Publish p2p-admission-public.xy with the runtime as an extra file (Guard role ADMISSION_KEY).",
];

/// Milliseconds since the Unix epoch; only names the temporary folder.
fn epoch_millis() -> i64 {
    let elapsed = SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default();
    elapsed.as_millis().try_into().unwrap_or(i64::MAX)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value_of = |flag: &str| {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    };
    let (Some(output), Some(key_id)) = (value_of("--output"), value_of("--key-id")) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let key_id = rifts_admission::KeyId(key_id.into());
    if let Err(error) = rifts_admission::write_key_set(Path::new(output), key_id, std::process::id(), epoch_millis()) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    println!("Generated a new P-256 admission key set in: {output}");
    AFTERWARDS.iter().for_each(|line| println!("{line}"));
    ExitCode::SUCCESS
}
