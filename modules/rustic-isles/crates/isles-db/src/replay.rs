//! RU8 shadow replay CLI options and re-exports from [`isles_core::replay`].

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub use isles_core::replay::*;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::migrate::DbError;

/// CLI options for `isles-db replay`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOptions {
    /// Path to a `.jsonl` capture file or directory of `.jsonl` files.
    pub corpus: PathBuf,
    /// Left target URL (`http://host:port`) or `"recorded"` to use the corpus's recorded responses.
    pub left: String,
    /// Right target URL (`http://host:port`) or `"recorded"`.
    pub right: String,
    /// Optional Director URL (`http://host:port`) for the left target.
    pub left_director: Option<String>,
    /// Optional Director URL (`http://host:port`) for the right target.
    pub right_director: Option<String>,
    /// Optional XMPP chat address (`host:port`) for the left target.
    pub left_chat: Option<String>,
    /// Optional XMPP chat address (`host:port`) for the right target.
    pub right_chat: Option<String>,
    /// Optional HS256 `JWT_SECRET` for `<token:...>` substitution on the left target.
    pub left_jwt_secret: Option<String>,
    /// Optional HS256 `JWT_SECRET` for `<token:...>` substitution on the right target.
    pub right_jwt_secret: Option<String>,
    /// Optional RS256 `AUTH_SIGNING_PRIVKEY_B64` for `<token:...>` substitution on the left target.
    pub left_signing_key: Option<String>,
    /// Optional RS256 `AUTH_SIGNING_PRIVKEY_B64` for `<token:...>` substitution on the right target.
    pub right_signing_key: Option<String>,
    /// Optional `GAMESERVER_API_KEY` for `<gskey>` substitution on the left target.
    pub left_gs_key: Option<String>,
    /// Optional `GAMESERVER_API_KEY` for `<gskey>` substitution on the right target.
    pub right_gs_key: Option<String>,
    /// Optional `x-gateway-secret` header injected on replayed HTTP requests.
    pub gateway_secret: Option<String>,
    /// Optional JSON file mapping `<sha256>` -> raw `RRK_...`/`UUK_...` key (or JSON array of raw keys).
    pub key_map_path: Option<PathBuf>,
    /// Whether to send `x-isles-replay-now-ms` and `x-isles-replay-seed` headers.
    pub pin_clock: bool,
    /// Optional substring filter on request target path or route.
    pub filter: Option<String>,
    /// Optional file path to write the full JSON replay report.
    pub out_path: Option<PathBuf>,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            corpus: PathBuf::new(),
            left: "recorded".to_owned(),
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

/// Loads a key map from a JSON file: supports either `{"<sha256>": "RRK_..."}` or `["RRK_...", "UUK_..."]`.
pub fn load_key_map(path: &std::path::Path) -> Result<HashMap<String, String>, DbError> {
    let text = fs::read_to_string(path)
        .map_err(|e| DbError::Config(format!("failed to read --key-map {}: {e}", path.display())))?;
    let val: Value = serde_json::from_str(&text)
        .map_err(|e| DbError::Config(format!("invalid JSON in --key-map {}: {e}", path.display())))?;
    let mut map = HashMap::new();
    match val {
        Value::Object(obj) => {
            for (k, v) in obj {
                if let Some(raw_key) = v.as_str() {
                    if k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit()) {
                        map.insert(k.to_ascii_lowercase(), raw_key.to_owned());
                    } else {
                        let hash = sha256_hex(raw_key);
                        map.insert(hash, raw_key.to_owned());
                    }
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                if let Some(raw_key) = item.as_str() {
                    map.insert(sha256_hex(raw_key), raw_key.to_owned());
                }
            }
        }
        _ => {
            return Err(DbError::Config(format!(
                "--key-map {} must be a JSON object or array of strings",
                path.display()
            )));
        }
    }
    Ok(map)
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn build_endpoint(spec: &str, director_url: Option<String>, chat_addr: Option<String>) -> ReplayEndpoint {
    if spec.eq_ignore_ascii_case("recorded") || spec.eq_ignore_ascii_case("corpus") {
        ReplayEndpoint::Recorded
    } else {
        ReplayEndpoint::Http {
            base_url: spec.to_owned(),
            director_url,
            chat_addr,
        }
    }
}

/// Runs `isles-db replay` from parsed [`ReplayOptions`].
pub async fn run_replay_cli(opts: &ReplayOptions) -> Result<ReplayReport, DbError> {
    let key_map = match &opts.key_map_path {
        Some(path) => load_key_map(path)?,
        None => HashMap::new(),
    };

    let records = load_corpus(&opts.corpus).map_err(|e| DbError::Config(e.to_string()))?;

    let left_endpoint = build_endpoint(&opts.left, opts.left_director.clone(), opts.left_chat.clone());
    let right_endpoint = build_endpoint(&opts.right, opts.right_director.clone(), opts.right_chat.clone());

    let left_is_recorded = matches!(left_endpoint, ReplayEndpoint::Recorded);
    let right_is_recorded = matches!(right_endpoint, ReplayEndpoint::Recorded);

    let left_target = ReplayTarget {
        name: "left".to_owned(),
        endpoint: left_endpoint,
        auth: ReplayTargetAuth {
            jwt_secret: opts.left_jwt_secret.clone(),
            signing_key_b64: opts.left_signing_key.clone(),
            gs_key: opts.left_gs_key.clone(),
            gateway_secret: opts.gateway_secret.clone(),
            key_map: key_map.clone(),
            deterministic_fallback_keys: true,
        },
        pin_clock: opts.pin_clock && !left_is_recorded,
    };

    let right_target = ReplayTarget {
        name: "right".to_owned(),
        endpoint: right_endpoint,
        auth: ReplayTargetAuth {
            jwt_secret: opts.right_jwt_secret.clone(),
            signing_key_b64: opts.right_signing_key.clone(),
            gs_key: opts.right_gs_key.clone(),
            gateway_secret: opts.gateway_secret.clone(),
            key_map,
            deterministic_fallback_keys: true,
        },
        pin_clock: opts.pin_clock && !right_is_recorded,
    };

    let config = ReplayConfig {
        left: left_target,
        right: right_target,
        normalizer: NormalizerConfig::default(),
        filter: opts.filter.clone(),
    };

    let report = run_replay(&records, &config)
        .await
        .map_err(|e| DbError::Config(e.to_string()))?;

    if let Some(out_path) = &opts.out_path {
        let json_bytes = serde_json::to_vec_pretty(&report)
            .map_err(|e| DbError::Config(format!("failed to serialize replay report: {e}")))?;
        fs::write(out_path, json_bytes)
            .map_err(|e| DbError::Config(format!("failed to write --out {}: {e}", out_path.display())))?;
    }

    Ok(report)
}
