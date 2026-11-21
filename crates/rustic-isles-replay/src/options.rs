// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Running from options (spec §10.2, §10.3): the key map file, the two sides, and the `--out` report.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::credentials::{ReplayTargetAuth, sha256_hex};
use crate::normalize::NormalizerConfig;
use crate::runner::{ReplayConfig, ReplayEndpoint, ReplayReport, ReplayTarget, run_replay};
use crate::{ReplayError, load_corpus};

/// What a run is given. `Debug` prints paths, URLs, the filter and switches, and for each secret only whether it is
/// set (D2).
#[derive(Clone)]
pub struct ReplayOptions {
    /// Capture file, or a directory of `*.jsonl` capture files.
    pub corpus: PathBuf,
    /// A base URL, or `recorded`/`corpus` (any case) for the capture's own answers
    pub left: String,
    /// The right side's base URL.
    pub right: String,
    /// The left side's Director base URL.
    pub left_director: Option<String>,
    /// The right side's Director base URL.
    pub right_director: Option<String>,
    /// The left side's chat `host:port`.
    pub left_chat: Option<String>,
    /// The right side's chat `host:port`.
    pub right_chat: Option<String>,
    /// The left side's HS256 secret.
    pub left_jwt_secret: Option<String>,
    /// The right side's HS256 secret.
    pub right_jwt_secret: Option<String>,
    /// The left side's base64 RSA signing key.
    pub left_signing_key: Option<String>,
    /// The right side's base64 RSA signing key.
    pub right_signing_key: Option<String>,
    /// The left side's game-server key.
    pub left_gs_key: Option<String>,
    /// The right side's game-server key.
    pub right_gs_key: Option<String>,
    /// Both sides
    pub gateway_secret: Option<String>,
    /// A JSON file of account key hashes to raw keys.
    pub key_map_path: Option<PathBuf>,
    /// Send core's replay clock headers (default on).
    pub pin_clock: bool,
    /// Keep only HTTP records whose raw target or route contains this.
    pub filter: Option<String>,
    /// Where the JSON report is also written.
    pub out_path: Option<PathBuf>,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            corpus: PathBuf::new(),
            left: "recorded".into(),
            right: String::new(),
            left_director: None,
            right_director: None,
            left_chat: None,
            right_chat: None,
            left_jwt_secret: None,
            right_jwt_secret: None,
            left_signing_key: None,
            right_signing_key: None,
            left_gs_key: None,
            right_gs_key: None,
            gateway_secret: None,
            key_map_path: None,
            pin_clock: true,
            filter: None,
            out_path: None,
        }
    }
}

impl std::fmt::Debug for ReplayOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayOptions")
            .field("corpus", &self.corpus)
            .field("left", &self.left)
            .field("right", &self.right)
            .field("left_director", &self.left_director)
            .field("right_director", &self.right_director)
            .field("left_chat", &self.left_chat)
            .field("right_chat", &self.right_chat)
            .field("left_jwt_secret", &self.left_jwt_secret.is_some())
            .field("right_jwt_secret", &self.right_jwt_secret.is_some())
            .field("left_signing_key", &self.left_signing_key.is_some())
            .field("right_signing_key", &self.right_signing_key.is_some())
            .field("left_gs_key", &self.left_gs_key.is_some())
            .field("right_gs_key", &self.right_gs_key.is_some())
            .field("gateway_secret", &self.gateway_secret.is_some())
            .field("key_map_path", &self.key_map_path)
            .field("pin_clock", &self.pin_clock)
            .field("filter", &self.filter)
            .field("out_path", &self.out_path)
            .finish()
    }
}

/// The key map of §10.2 from parsed JSON; `None` when it is neither an object nor an array.
pub fn key_map_from_json(value: &Value) -> Option<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    match value {
        Value::Array(items) => {
            for raw in items.iter().filter_map(Value::as_str) {
                map.insert(sha256_hex(raw.as_bytes()), raw.to_owned());
            }
        }
        Value::Object(members) => {
            for (name, value) in members {
                let Some(raw) = value.as_str() else {
                    continue;
                };
                let hex = if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
                    name.to_ascii_lowercase()
                } else {
                    sha256_hex(raw.as_bytes())
                };
                map.insert(hex, raw.to_owned());
            }
        }
        _ => return None,
    }
    Some(map)
}

/// Reads `--key-map`: a JSON array of raw keys, or an object of hex (or any name) → raw key.
pub fn load_key_map(path: &Path) -> Result<BTreeMap<String, String>, ReplayError> {
    let shown = path.display();
    let text = std::fs::read_to_string(path)
        .map_err(|e| ReplayError::Config(format!("failed to read --key-map {shown}: {e}")))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| ReplayError::Config(format!("invalid JSON in --key-map {shown}: {e}")))?;
    key_map_from_json(&value)
        .ok_or_else(|| ReplayError::Config(format!("--key-map {shown} must be a JSON object or array of strings")))
}

fn is_recorded(spec: &str) -> bool {
    let spec = spec.trim();
    spec.eq_ignore_ascii_case("recorded") || spec.eq_ignore_ascii_case("corpus")
}

struct SideOptions<'a> {
    name: &'a str,
    url: &'a str,
    director: &'a Option<String>,
    chat: &'a Option<String>,
    jwt_secret: &'a Option<String>,
    signing_key: &'a Option<String>,
    gs_key: &'a Option<String>,
}

fn build_side(side: SideOptions<'_>, options: &ReplayOptions, key_map: &BTreeMap<String, String>) -> ReplayTarget {
    let auth = ReplayTargetAuth {
        jwt_secret: side.jwt_secret.clone(),
        signing_key_b64: side.signing_key.clone(),
        gs_key: side.gs_key.clone(),
        gateway_secret: options.gateway_secret.clone(),
        key_map: key_map.clone(),
        deterministic_fallback_keys: true,
    };
    let recorded = is_recorded(side.url);
    let endpoint = if recorded {
        ReplayEndpoint::Recorded
    } else {
        ReplayEndpoint::Http {
            base_url: side.url.to_owned(),
            director_url: side.director.clone(),
            chat_addr: side.chat.clone(),
        }
    };
    ReplayTarget {
        name: side.name.to_owned(),
        endpoint,
        auth,
        pin_clock: options.pin_clock && !recorded,
    }
}

/// The report file, replaced, owner-only where modes exist (decision on §16 question 4).
fn write_report(path: &Path, json: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        open.mode(0o600);
        let mut file = open.open(path)?;
        // An existing file keeps its mode through open, so it is narrowed too
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(json.as_bytes())
    }
    #[cfg(not(unix))]
    {
        let mut file = open.open(path)?;
        file.write_all(json.as_bytes())
    }
}

/// §10.3: key map, corpus, both sides, the run, then `--out`.
pub async fn run_replay_cli(options: &ReplayOptions) -> Result<ReplayReport, ReplayError> {
    let key_map = match &options.key_map_path {
        Some(path) => load_key_map(path)?,
        None => BTreeMap::new(),
    };
    let records = load_corpus(&options.corpus).map_err(|e| ReplayError::Config(e.inner_message()))?;
    let left = build_side(
        SideOptions {
            name: "left",
            url: &options.left,
            director: &options.left_director,
            chat: &options.left_chat,
            jwt_secret: &options.left_jwt_secret,
            signing_key: &options.left_signing_key,
            gs_key: &options.left_gs_key,
        },
        options,
        &key_map,
    );
    let right = build_side(
        SideOptions {
            name: "right",
            url: &options.right,
            director: &options.right_director,
            chat: &options.right_chat,
            jwt_secret: &options.right_jwt_secret,
            signing_key: &options.right_signing_key,
            gs_key: &options.right_gs_key,
        },
        options,
        &key_map,
    );
    let config = ReplayConfig {
        left,
        right,
        normalizer: NormalizerConfig::default(),
        filter: options.filter.clone(),
    };
    let report = run_replay(&records, &config)
        .await
        .map_err(|e| ReplayError::Config(e.inner_message()))?;
    if let Some(path) = &options.out_path {
        let json = report.to_json_pretty()?;
        write_report(path, &json)
            .map_err(|e| ReplayError::Config(format!("failed to write --out {}: {e}", path.display())))?;
    }
    Ok(report)
}
