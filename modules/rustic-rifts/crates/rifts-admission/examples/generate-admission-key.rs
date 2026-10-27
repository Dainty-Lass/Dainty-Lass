//! `generate-admission-key.mjs` as a command, which the 1.14.7 module exposes through its own CLI:
//!
//!   cargo run -p rifts-admission --example generate-admission-key -- --output <new-directory> --key-id <safe-id>
//!
//! Writes the PKCS#8 and SPKI PEMs, `p2p-admission-public.xy` and `metagame-admission.env` into a new folder. Prints
//! no key.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use rifts_admission::{KeyId, write_key_set};

fn argument(name: &str) -> Option<String> {
    let mut args = std::env::args();
    args.position(|arg| arg == format!("--{name}"))?;
    args.next()
}

fn main() -> ExitCode {
    let (Some(output), Some(key_id)) = (argument("output"), argument("key-id")) else {
        eprintln!("Usage: generate-admission-key --output <new-directory> --key-id <safe-id>");
        return ExitCode::from(2);
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
    match write_key_set(&PathBuf::from(&output), KeyId(key_id), std::process::id(), now) {
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
