//! Cold-boot auto-provisioning for zero-pre-setup server execution (`docs/app-structure-plan.md`).
//!
//! Ensures that starting any server binary on a clean directory automatically provisions:
//! 1. `.workspace/config/secrets.env` (and `.workspace/secrets/secrets.env`) with all 8 CSPRNG
//!    32-byte hex keys and a 4096-bit RSA token keypair (`0600` on Unix).
//! 2. `.workspace/certs/gateway.crt` and `gateway.key` via `gateway::cert::load_or_generate`,
//!    logging the SHA-256 fingerprint (`fp`) and ready-to-use `reforged-revival://join?...` invite.
//! 3. `.workspace/db/` SQLite database directory and initial preset seed record.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::{KeyPair, KeySize};
use aws_lc_rs::signature::KeyPair as _;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use protocol::invite::{Invite, ServerMode, format_invite};

use crate::config::ServerConfig;

pub const HEX_SECRET_KEYS: &[&str] = &[
    "GATEWAY_OPERATOR_SECRET",
    "GATEWAY_ALLOWLIST_SECRET",
    "GATEWAY_CAPTURE_TOKEN",
    "GAMESERVER_DEDICATED_KEY",
    "REFORGED_ADMIN_KEY",
    "ADMIN_API_KEY",
    "EOS_METRICS_SECRET",
    "EOS_SANCTIONS_SECRET",
];

#[derive(Debug, Clone)]
pub struct ProvisionedWorkspace {
    pub workspace_dir: PathBuf,
    pub secrets_file: PathBuf,
    pub secrets: BTreeMap<String, String>,
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
    pub cert_fingerprint: String,
    pub invite_url: String,
    pub db_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub branding_dir: PathBuf,
    pub admin_timeouts_file: PathBuf,
}

fn random_hex_32() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    aws_lc_rs::rand::fill(&mut bytes).map_err(|_| "CSPRNG fill failed".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn generate_rsa_token_keys_b64() -> Result<(String, String), String> {
    let pair = KeyPair::generate(KeySize::Rsa4096).map_err(|_| "RSA-4096 key generation failed".to_string())?;
    let pkcs8 = AsDer::<Pkcs8V1Der>::as_der(&pair).map_err(|_| "PKCS#8 encoding failed".to_string())?;
    let spki = AsDer::<PublicKeyX509Der>::as_der(pair.public_key()).map_err(|_| "SPKI encoding failed".to_string())?;
    let pub_pem = pem_block("PUBLIC KEY", spki.as_ref());
    let priv_pem = pem_block("PRIVATE KEY", pkcs8.as_ref());
    Ok((BASE64.encode(pub_pem.as_bytes()), BASE64.encode(priv_pem.as_bytes())))
}

fn pem_block(label: &str, der: &[u8]) -> String {
    let b64 = BASE64.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        if let Ok(line) = std::str::from_utf8(chunk) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn write_secret_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("could not chmod 0600 {}: {e}", path.display()))?;
        f.write_all(content.as_bytes())
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Ensures `.workspace/config/secrets.env` (and `.workspace/secrets/secrets.env`) exists and holds
/// all 8 CSPRNG 32-byte hex keys and the 4096-bit RSA token keypair.
pub fn ensure_secrets(workspace_dir: &Path) -> Result<(PathBuf, BTreeMap<String, String>), String> {
    let config_secrets = workspace_dir.join("config").join("secrets.env");
    let legacy_secrets = workspace_dir.join("secrets").join("secrets.env");
    let mut map = BTreeMap::new();
    for candidate in [&legacy_secrets, &config_secrets] {
        if candidate.is_file()
            && let Ok(existing) = service_kit::env::Env::load(Some(candidate))
        {
            for (k, v) in existing.vars() {
                if !v.is_empty() {
                    map.insert(k.to_owned(), v.to_owned());
                }
            }
        }
    }
    let mut changed = !config_secrets.is_file();
    for &key in HEX_SECRET_KEYS {
        if !map.contains_key(key) {
            map.insert(key.to_string(), random_hex_32()?);
            changed = true;
        }
    }
    if !map.contains_key("TOKEN_PUBLIC_KEY_B64") || !map.contains_key("TOKEN_PRIVATE_KEY_B64") {
        let (pub_b64, priv_b64) = generate_rsa_token_keys_b64()?;
        map.insert("TOKEN_PUBLIC_KEY_B64".into(), pub_b64);
        map.insert("TOKEN_PRIVATE_KEY_B64".into(), priv_b64);
        changed = true;
    }
    if changed {
        let mut body = String::from("# Auto-generated by ServerController — never share or commit this file.\n");
        for &key in HEX_SECRET_KEYS {
            if let Some(val) = map.get(key) {
                body.push_str(&format!("{key}={val}\n"));
            }
        }
        for key in ["TOKEN_PUBLIC_KEY_B64", "TOKEN_PRIVATE_KEY_B64"] {
            if let Some(val) = map.get(key) {
                body.push_str(&format!("{key}={val}\n"));
            }
        }
        write_secret_file(&config_secrets, &body)?;
        write_secret_file(&legacy_secrets, &body)?;
        tracing::info!(
            path = %config_secrets.display(),
            "provisioned server secrets (8 CSPRNG keys + 4096-bit RSA keypair)"
        );
    }
    let _ = SystemRandom::new();
    Ok((config_secrets, map))
}

/// Ensures the gateway's self-signed TLS certificate exists via `gateway::cert::load_or_generate`
/// and computes the ready-to-use `reforged-revival://join?...` invite URL.
pub fn ensure_tls_certificate(
    workspace_dir: &Path,
    config: &ServerConfig,
) -> Result<(PathBuf, PathBuf, String, String), String> {
    let certs_dir = workspace_dir.join("certs");
    std::fs::create_dir_all(&certs_dir).map_err(|e| format!("could not create {}: {e}", certs_dir.display()))?;
    let cert_file = certs_dir.join("gateway.crt");
    let key_file = certs_dir.join("gateway.key");
    let mut hosts = vec![
        config.public_host.clone(),
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ];
    hosts.dedup();
    let (loaded, _created) = gateway::cert::load_or_generate(&cert_file, &key_file, &hosts)?;
    let fp = loaded.info.fingerprint;

    let invite = if config.server_mode.eq_ignore_ascii_case("public") {
        let host = if config.public_host == "127.0.0.1" || config.public_host == "localhost" {
            "play.example.org".to_string()
        } else {
            config.public_host.clone()
        };
        Invite {
            mode: ServerMode::Public,
            host,
            port: config.gateway_port,
            code: "open".into(),
            name: config.server_name.clone(),
            share: None,
            fp: Some(fp.clone()),
            game: protocol::game::Game::Reborn,
        }
    } else {
        let host = if protocol::invite::is_private_mode_host(&config.public_host) {
            config.public_host.clone()
        } else {
            "127.0.0.1".to_string()
        };
        Invite {
            mode: ServerMode::Private,
            host,
            port: config.editions.reforged.backend_port,
            code: "open".into(),
            name: config.server_name.clone(),
            share: None,
            fp: None,
            game: protocol::game::Game::Reborn,
        }
    };
    let invite_url = format_invite(&invite).unwrap_or_else(|_| {
        format!(
            "reforged-revival://join?v=2&mode=public&host={}&port={}&fp={fp}&code=open&name=Server",
            config.public_host, config.gateway_port
        )
    });
    tracing::info!(
        fingerprint = %fp,
        invite = %invite_url,
        "gateway TLS certificate ready"
    );
    Ok((cert_file, key_file, fp, invite_url))
}

/// Ensures `.workspace/db/` exists and records the initial preset seed state if a preset folder is present.
pub fn ensure_database_and_preset(workspace_dir: &Path, preset_dir: Option<&Path>) -> Result<PathBuf, String> {
    let db_dir = workspace_dir.join("db");
    std::fs::create_dir_all(&db_dir).map_err(|e| format!("could not create {}: {e}", db_dir.display()))?;
    let seed_marker = db_dir.join("preset-seeded.json");
    if !seed_marker.is_file() {
        let resolved_preset = preset_dir.map(Path::to_path_buf).or_else(|| {
            let candidate = PathBuf::from("preset");
            candidate.is_dir().then_some(candidate)
        });
        let info = serde_json::json!({
            "seeded": true,
            "presetDir": resolved_preset.as_ref().map(|p| p.display().to_string()),
        });
        std::fs::write(&seed_marker, info.to_string())
            .map_err(|e| format!("could not write {}: {e}", seed_marker.display()))?;
    }
    Ok(db_dir)
}

/// Runs all cold-boot provisioning steps on `config.workspace_dir`.
pub fn provision_workspace(config: &ServerConfig) -> Result<ProvisionedWorkspace, String> {
    let ws = &config.workspace_dir;
    let runtime_dir = ws.join("content").join("runtime");
    let branding_dir = ws.join("content").join("branding");
    let admin_dir = ws.join("admin");
    let logs_dir = ws.join("logs");
    for dir in [
        &runtime_dir,
        &runtime_dir.join("reforged"),
        &runtime_dir.join("refined"),
        &branding_dir,
        &admin_dir,
        &logs_dir,
    ] {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let (secrets_file, secrets) = ensure_secrets(ws)?;
    let (cert_file, key_file, cert_fingerprint, invite_url) = ensure_tls_certificate(ws, config)?;
    let db_dir = ensure_database_and_preset(ws, config.preset_dir.as_deref())?;
    let admin_timeouts_file = admin_dir.join("timeouts.json");
    Ok(ProvisionedWorkspace {
        workspace_dir: ws.clone(),
        secrets_file,
        secrets,
        cert_file,
        key_file,
        cert_fingerprint,
        invite_url,
        db_dir,
        runtime_dir,
        branding_dir,
        admin_timeouts_file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::edition::EditionSelection;

    #[test]
    fn cold_boot_provisions_secrets_cert_and_db() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = ServerConfig::default_for(EditionSelection::Both, tmp.path().to_path_buf());
        let provisioned = provision_workspace(&cfg).unwrap();
        assert!(provisioned.secrets_file.is_file());
        for &key in HEX_SECRET_KEYS {
            assert_eq!(provisioned.secrets.get(key).map(String::len), Some(64), "{key}");
        }
        assert!(provisioned.secrets.contains_key("TOKEN_PUBLIC_KEY_B64"));
        assert!(provisioned.secrets.contains_key("TOKEN_PRIVATE_KEY_B64"));
        assert!(provisioned.cert_file.is_file());
        assert!(provisioned.key_file.is_file());
        assert_eq!(provisioned.cert_fingerprint.len(), 64);
        assert!(provisioned.invite_url.starts_with("reforged-revival://join?"));
        assert!(provisioned.db_dir.join("preset-seeded.json").is_file());
    }
}
