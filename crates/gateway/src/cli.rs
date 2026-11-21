// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `gateway serve [--env-file <file>]` (the default) and `gateway make-cert <args...>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::json;
use service_kit::env::Env;
use service_kit::linelog;

use crate::config::Config;

fn fatal(message: &str) -> ExitCode {
    linelog::error(message, json!({}));
    ExitCode::from(1)
}

/// `--env-file <file>`, else a `.env` in the working folder if it is a file.
pub(crate) fn env_file(args: &[String], extra: &[&str]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let mut file = None;
    let mut flags = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--env-file" => {
                let value = iter.next().ok_or_else(|| "--env-file needs a value".to_owned())?;
                file = Some(PathBuf::from(value));
            }
            flag if extra.contains(&flag) => flags.push(flag.to_owned()),
            other => return Err(format!("unknown argument {}", crate::json_string(other))),
        }
    }
    let file = file.or_else(|| Path::new(".env").is_file().then(|| PathBuf::from(".env")));
    Ok((file, flags))
}

/// The gateway's command line; `args` are the arguments after the program's name.
pub fn main_from(args: Vec<String>) -> ExitCode {
    let rest: Vec<String> = match args.split_first() {
        Some((first, rest)) if first == "make-cert" => return crate::cert::make_cert_main(rest),
        Some((first, rest)) if first == "serve" => rest.to_vec(),
        _ => args,
    };
    let (file, _) = match env_file(&rest, &[]) {
        Ok(found) => found,
        Err(e) => return fatal(&format!("Configuration: {e}")),
    };
    let env = match Env::load(file.as_deref()) {
        Ok(env) => env,
        Err(e) => return fatal(&format!("Configuration: {e}")),
    };
    let config = match Config::from_env(&env) {
        Ok(config) => config,
        Err(e) => return fatal(&format!("Configuration: {e}")),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => return fatal(&format!("Could not start the runtime: {e}")),
    };
    runtime.block_on(async {
        let capture = match crate::server::load_capture(&env) {
            Ok(capture) => capture,
            Err(e) => return fatal(&e),
        };
        match crate::server::run(config, capture).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fatal(&e),
        }
    })
}
