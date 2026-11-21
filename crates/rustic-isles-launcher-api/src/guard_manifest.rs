// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The signed Guard manifest (public spec §6.2): the publisher signs offline, the server only verifies with the two
//! compiled-in Ed25519 keys.

use std::sync::LazyLock;

use dainty_service_kit::js::safe_integer;
use regex::Regex;
use serde_json::Value;

use crate::util::{is_hex, lenient_base64};

/// The key id of runtime manifests' signatures.
pub const RUNTIME_KEY_ID: &str = "runtime-2026-01";
/// The key id of Guard manifests' signatures.
pub const GUARD_KEY_ID: &str = "guard-2026-10";
/// The only platform a Guard manifest names.
pub const PLATFORM: &str = "windows-x86_64";
const RUNTIME_SPKI: &str = "MCowBQYDK2VwAyEA3ZMtA7qUgs1F+1NQs2kmSG2zbOvXfjsh6+axI6eC/tc=";
const GUARD_SPKI: &str = "MCowBQYDK2VwAyEAcqOh1F/NOXtZ4ZSyefheKk0Md//2L85OgMiQH8Jyi/U=";
const MAX_PAYLOAD: usize = 256 * 1024;
const MAX_LIFETIME_MS: i64 = 365 * 86_400_000;
const FUTURE_SKEW_MS: i64 = 5 * 60_000;
const MIB: i64 = 1024 * 1024;
const ROLES: [&str; 5] = ["GAME", "LAUNCHER", "CONTENT", "INTERNAL_SERVER", "UDP_PROXY"];

static VERSION: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$").ok());

/// `x.y.z` with an optional `-` or `+` suffix.
pub fn is_version(text: &str) -> bool {
    VERSION.as_ref().is_some_and(|re| re.is_match(text))
}

pub use crate::util::is_segment;

/// The two public keys the server trusts (raw 32-byte Ed25519).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trusted {
    /// The runtime manifests' key.
    pub runtime: [u8; 32],
    /// The Guard manifests' key.
    pub guard: [u8; 32],
}

fn raw_key(spki: &str) -> [u8; 32] {
    let bytes = lenient_base64(spki);
    let mut out = [0u8; 32];
    if bytes.len() >= 32 {
        out.copy_from_slice(&bytes[bytes.len() - 32..]);
    }
    out
}

impl Trusted {
    /// The compiled-in keys: the last 32 bytes of each SPKI.
    pub fn embedded() -> Self {
        Self {
            runtime: raw_key(RUNTIME_SPKI),
            guard: raw_key(GUARD_SPKI),
        }
    }

    /// Other keys (tests sign with their own pair).
    pub fn with_keys(runtime: [u8; 32], guard: [u8; 32]) -> Self {
        Self { runtime, guard }
    }

    fn by_id(&self, key_id: &str) -> Option<&[u8; 32]> {
        match key_id {
            RUNTIME_KEY_ID => Some(&self.runtime),
            GUARD_KEY_ID => Some(&self.guard),
            _ => None,
        }
    }
}

/// Ed25519 over `message` by a raw public key.
pub fn ed25519_verify(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    signature.len() == 64
        && aws_lc_rs::signature::UnparsedPublicKey::new(&aws_lc_rs::signature::ED25519, public_key)
            .verify(message, signature)
            .is_ok()
}

/// A file the Guard manifest vouches for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardArtifact {
    /// What it is: `GAME`, `LAUNCHER`, `CONTENT`, `INTERNAL_SERVER` or `UDP_PROXY`.
    pub role: String,
    /// Its file name.
    pub name: String,
    /// Lower-cased
    pub sha256: String,
}

/// A verified Guard manifest's payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardPayload {
    /// The manifest's id.
    pub manifest_id: String,
    /// Its sequence number; a newer manifest has a higher one.
    pub sequence: i64,
    /// The update channel.
    pub channel: String,
    /// Always [`PLATFORM`].
    pub platform: String,
    /// The game build it is for.
    pub changelist: i64,
    /// The oldest launcher, `x.y.z`, that may use it.
    pub minimum_launcher_version: String,
    /// When it expires, ISO text.
    pub expires_at: String,
    /// Most private executable memory the game may hold, bytes.
    pub max_private_executable_bytes: i64,
    /// Most writable executable memory the game may hold, bytes.
    pub max_writable_executable_bytes: i64,
    /// The files it vouches for.
    pub artifacts: Vec<GuardArtifact>,
}

const INVALID_ENVELOPE: &str = "Invalid Guard manifest envelope.";
const UNKNOWN_KEY: &str = "Unknown Guard manifest signing key.";
const BAD_SIGNATURE: &str = "Guard manifest signature verification failed.";
const INVALID_PAYLOAD: &str = "Invalid Guard manifest payload.";
const INVALID_ARTIFACT: &str = "Invalid Guard artifact entry.";
const INVALID_REGION: &str = "Invalid Guard protected-region entry.";
const INVALID_MEMORY: &str = "Invalid Guard executable-memory policy.";

/// The issue and expiry times at `now`.
pub fn validate_window(issued_at: &str, expires_at: &str, now_ms: i64) -> Result<(), &'static str> {
    let (Some(issued), Some(expires)) = (
        dainty_service_kit::js::date_parse(issued_at),
        dainty_service_kit::js::date_parse(expires_at),
    ) else {
        return Err("Guard manifest timestamps are invalid.");
    };
    if expires <= issued {
        return Err("Guard manifest expiry is not after its issue time.");
    }
    if expires - issued > MAX_LIFETIME_MS {
        return Err("Guard manifest lifetime exceeds the maximum allowed window.");
    }
    if issued > now_ms + FUTURE_SKEW_MS {
        return Err("Guard manifest issue time is too far in the future.");
    }
    if expires <= now_ms {
        return Err("Guard manifest has expired.");
    }
    Ok(())
}

fn safe_in(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    safe_integer(value).filter(|n| (min..=max).contains(n))
}

fn is_hex64_value(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| is_hex(s, 64))
}

fn artifacts(list: &[Value], extension_roles: &[&str]) -> Result<Vec<GuardArtifact>, &'static str> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for entry in list {
        let Some(map) = entry.as_object() else {
            return Err(INVALID_ARTIFACT);
        };
        let (Some(name), Some(role)) = (
            map.get("name").and_then(Value::as_str),
            map.get("role").and_then(Value::as_str),
        ) else {
            return Err(INVALID_ARTIFACT);
        };
        let lower = name.to_lowercase();
        let ok = !name.is_empty()
            && !name.contains(['/', '\\'])
            && !seen.contains(&lower)
            && safe_in(map.get("size"), 1, i64::MAX).is_some()
            && is_hex64_value(map.get("sha256"))
            && map.get("required").is_some_and(Value::is_boolean)
            && (ROLES.contains(&role) || extension_roles.contains(&role));
        if !ok {
            return Err(INVALID_ARTIFACT);
        }
        seen.push(lower);
        match map.get("protectedRegions") {
            None | Some(Value::Null) => {}
            Some(Value::Array(regions)) if regions.len() <= 256 => {
                for region in regions {
                    let ok = safe_in(region.get("rva"), 0, i64::MAX).is_some()
                        && safe_in(region.get("size"), 1, MIB).is_some()
                        && is_hex64_value(region.get("sha256"));
                    if !ok {
                        return Err(INVALID_REGION);
                    }
                }
            }
            Some(_) => return Err(INVALID_REGION),
        }
        out.push(GuardArtifact {
            role: role.to_owned(),
            name: name.to_owned(),
            sha256: map.get("sha256").and_then(Value::as_str).unwrap_or("").to_lowercase(),
        });
    }
    Ok(out)
}

fn memory_policy(value: Option<&Value>) -> Result<(i64, i64), &'static str> {
    let Some(map) = value.and_then(Value::as_object) else {
        return Err(INVALID_MEMORY);
    };
    let private = safe_in(map.get("maximumPrivateExecutableBytes"), 0, 64 * MIB).ok_or(INVALID_MEMORY)?;
    let writable = match map.get("maximumWritableExecutableBytes") {
        None | Some(Value::Null) => 0,
        other => safe_in(other, 0, 16 * MIB).ok_or(INVALID_MEMORY)?,
    };
    let hashes_ok = match map.get("allowlistedModuleHashes") {
        Some(Value::Array(list)) => list.len() <= 512 && list.iter().all(|h| is_hex64_value(Some(h))),
        _ => false,
    };
    if !hashes_ok {
        return Err(INVALID_MEMORY);
    }
    Ok((private, writable))
}

fn is_manifest_id(text: &str) -> bool {
    text.len() == 36 && text.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn is_channel(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Verifies an envelope for changelist `changelist` at `now_ms`; the error is the text the routes answer.
pub fn verify_envelope(
    envelope: &Value,
    changelist: i64,
    extension_roles: &[&str],
    now_ms: i64,
    trusted: &Trusted,
) -> Result<GuardPayload, String> {
    let Some(map) = envelope.as_object() else {
        return Err(INVALID_ENVELOPE.into());
    };
    let (Some(key_id), Some(payload_text), Some(signature_text)) = (
        map.get("keyId").and_then(Value::as_str),
        map.get("payloadBase64").and_then(Value::as_str),
        map.get("signature").and_then(Value::as_str),
    ) else {
        return Err(INVALID_ENVELOPE.into());
    };
    if safe_integer(map.get("schema")) != Some(1) {
        return Err(INVALID_ENVELOPE.into());
    }
    let Some(key) = trusted.by_id(key_id) else {
        return Err(UNKNOWN_KEY.into());
    };
    let payload = lenient_base64(payload_text);
    let signature = lenient_base64(signature_text);
    if payload.is_empty() || payload.len() > MAX_PAYLOAD || !ed25519_verify(key, &payload, &signature) {
        return Err(BAD_SIGNATURE.into());
    }
    let body: Value = serde_json::from_slice(&payload).map_err(|e| e.to_string())?;
    let field = |name: &str| body.get(name);
    let manifest_id = field("manifestId")
        .and_then(Value::as_str)
        .filter(|t| is_manifest_id(t));
    let sequence = safe_in(field("sequence"), 1, i64::MAX);
    let channel = field("channel").and_then(Value::as_str).filter(|t| is_channel(t));
    let platform = field("platform").and_then(Value::as_str).filter(|p| *p == PLATFORM);
    let minimum = field("minimumLauncherVersion")
        .and_then(Value::as_str)
        .filter(|v| is_version(v));
    let issued = field("issuedAt").and_then(Value::as_str);
    let expires = field("expiresAt").and_then(Value::as_str);
    let list = field("artifacts")
        .and_then(Value::as_array)
        .filter(|l| (1..=128).contains(&l.len()));
    let schema_ok = safe_integer(field("schema")) == Some(1);
    let changelist_ok = safe_integer(field("targetChangelist")) == Some(changelist);
    let (
        Some(manifest_id),
        Some(sequence),
        Some(channel),
        Some(platform),
        Some(minimum),
        Some(issued),
        Some(expires),
        Some(list),
        true,
        true,
    ) = (
        manifest_id,
        sequence,
        channel,
        platform,
        minimum,
        issued,
        expires,
        list,
        schema_ok,
        changelist_ok,
    )
    else {
        return Err(INVALID_PAYLOAD.into());
    };
    validate_window(issued, expires, now_ms).map_err(str::to_owned)?;
    let artifacts = artifacts(list, extension_roles).map_err(str::to_owned)?;
    let (private, writable) = memory_policy(field("executableMemoryPolicy")).map_err(str::to_owned)?;
    Ok(GuardPayload {
        manifest_id: manifest_id.to_owned(),
        sequence,
        channel: channel.to_owned(),
        platform: platform.to_owned(),
        changelist,
        minimum_launcher_version: minimum.to_owned(),
        expires_at: expires.to_owned(),
        max_private_executable_bytes: private,
        max_writable_executable_bytes: writable,
        artifacts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: &str = "2026-10-20T00:00:00Z";

    fn now() -> i64 {
        dainty_service_kit::js::date_parse(NOW).unwrap()
    }

    #[test]
    fn windows() {
        let n = now();
        assert_eq!(
            validate_window("2026-10-19T00:00:00Z", "2026-11-19T00:00:00Z", n),
            Ok(())
        );
        assert_eq!(
            validate_window("soon", "2026-11-19T00:00:00Z", n),
            Err("Guard manifest timestamps are invalid.")
        );
        assert_eq!(
            validate_window("2026-11-19T00:00:00Z", "2026-10-19T00:00:00Z", n),
            Err("Guard manifest expiry is not after its issue time.")
        );
        assert_eq!(
            validate_window("2026-01-01T00:00:00Z", "2028-01-01T00:00:00Z", n),
            Err("Guard manifest lifetime exceeds the maximum allowed window.")
        );
        assert_eq!(
            validate_window("2026-10-20T00:06:00Z", "2026-11-19T00:00:00Z", n),
            Err("Guard manifest issue time is too far in the future.")
        );
        assert_eq!(
            validate_window("2026-10-20T00:05:00Z", "2026-11-19T00:00:00Z", n),
            Ok(())
        );
        assert_eq!(
            validate_window("2026-10-01T00:00:00Z", "2026-10-10T00:00:00Z", n),
            Err("Guard manifest has expired.")
        );
    }

    #[test]
    fn versions() {
        for yes in ["1.2.3", "10.20.30-beta.1", "0.1.65+build-7"] {
            assert!(is_version(yes), "{yes}");
        }
        for no in ["1.2", "1.2.3.4", "a.b.c", "1.2.3-", "1.2.3-é", "", "١.2.3"] {
            assert!(!is_version(no), "{no}");
        }
    }

    #[test]
    fn envelopes() {
        let t = Trusted::embedded();
        let n = now();
        let check = |v: Value| verify_envelope(&v, 647_472, &[], n, &t).unwrap_err();
        assert_eq!(check(json!({})), INVALID_ENVELOPE);
        assert_eq!(
            check(json!({"schema":2,"keyId":"guard-2026-10","payloadBase64":"","signature":""})),
            INVALID_ENVELOPE
        );
        assert_eq!(
            check(json!({"schema":1,"keyId":"other","payloadBase64":"","signature":""})),
            UNKNOWN_KEY
        );
        assert_eq!(
            check(json!({"schema":1,"keyId":"guard-2026-10","payloadBase64":"e30=","signature":"AAAA"})),
            BAD_SIGNATURE
        );
        for key in [t.runtime, t.guard] {
            assert_ne!(key, [0u8; 32]);
            assert!(!ed25519_verify(&key, b"{}", &[0u8; 64]));
        }
    }
}
