//! The signed Guard manifest (`routes/launcherUpdates.ts`): the envelope an offline publisher signs, checked against the
//! runtime key or Guard's own key, and the window and shape rules the launcher repeats. The server never holds a
//! release private key.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;

use super::util::{base64_decode, date_parse, now_ms};

/// SPKI (`MCowBQYDK2VwAyEA` + 32 bytes) of the runtime's release key.
const RUNTIME_KEY_SPKI: &str = "MCowBQYDK2VwAyEA3ZMtA7qUgs1F+1NQs2kmSG2zbOvXfjsh6+axI6eC/tc=";
/// Guard's own signing key (launch plan P6.1, the key split).
const GUARD_KEY_2026_10_SPKI: &str = "MCowBQYDK2VwAyEAcqOh1F/NOXtZ4ZSyefheKk0Md//2L85OgMiQH8Jyi/U=";

pub const MAX_GUARD_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1000;
/// build-guard-manifest-payload.ps1's ValidateRange(1,365)
const MAX_LIFETIME_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// INTERNAL_SERVER is the runtime DLL and UDP_PROXY the winmm proxy that loads it: every manifest has them.
pub const CORE_ARTIFACT_ROLES: [&str; 5] = ["GAME", "LAUNCHER", "CONTENT", "INTERNAL_SERVER", "UDP_PROXY"];

fn key_from_spki(spki: &str) -> Option<VerifyingKey> {
    let der = base64_decode(spki);
    let raw: [u8; 32] = der.get(der.len().checked_sub(32)?..)?.try_into().ok()?;
    VerifyingKey::from_bytes(&raw).ok()
}

/// The keys a release is signed with: the runtime's, and Guard's own (`keyId` picks one).
#[derive(Debug, Clone)]
pub struct Trusted {
    runtime: VerifyingKey,
    guard: VerifyingKey,
}

impl Trusted {
    /// The keys compiled in. `None` only if a constant stopped being a key, which the tests catch.
    pub fn embedded() -> Option<Self> {
        Some(Self {
            runtime: key_from_spki(RUNTIME_KEY_SPKI)?,
            guard: key_from_spki(GUARD_KEY_2026_10_SPKI)?,
        })
    }

    /// Other keys, for the tests that sign their own manifests.
    pub fn with_keys(runtime: VerifyingKey, guard: VerifyingKey) -> Self {
        Self { runtime, guard }
    }

    /// `crypto.verify(null, data, runtimeKey, signature)`.
    pub fn verify_runtime(&self, data: &[u8], signature: &[u8]) -> bool {
        verify(&self.runtime, data, signature)
    }

    fn key_for(&self, key_id: &str) -> Option<&VerifyingKey> {
        match key_id {
            "runtime-2026-01" => Some(&self.runtime),
            "guard-2026-10" => Some(&self.guard),
            _ => None,
        }
    }
}

fn verify(key: &VerifyingKey, data: &[u8], signature: &[u8]) -> bool {
    Signature::from_slice(signature).is_ok_and(|signature| key.verify(data, &signature).is_ok())
}

/// `validateGuardManifestWindow`.
pub fn validate_window(issued_at: &str, expires_at: &str, now: i64) -> Result<(), &'static str> {
    let (Some(issued), Some(expires)) = (date_parse(issued_at), date_parse(expires_at)) else {
        return Err("Guard manifest timestamps are invalid.");
    };
    if expires <= issued {
        return Err("Guard manifest expiry is not after its issue time.");
    }
    if expires - issued > MAX_LIFETIME_MS {
        return Err("Guard manifest lifetime exceeds the maximum allowed window.");
    }
    if issued > now + MAX_FUTURE_SKEW_MS {
        return Err("Guard manifest issue time is too far in the future.");
    }
    if expires <= now {
        return Err("Guard manifest has expired.");
    }
    Ok(())
}

fn is_hex64(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn safe_int(value: Option<&Value>) -> Option<i64> {
    service_kit::js::safe_integer(value)
}

/// `/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/`
pub fn is_version(text: &str) -> bool {
    let (core, suffix) = match text.find(['-', '+']) {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && suffix
            .is_none_or(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
}

/// `/^[A-Za-z0-9._-]+$/`
pub fn is_segment_text(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Payload {
    pub manifest_id: String,
    pub sequence: i64,
    pub channel: String,
    pub platform: String,
    pub target_changelist: i64,
    pub minimum_launcher_version: String,
    pub expires_at: String,
    pub maximum_private_executable_bytes: i64,
    pub maximum_writable_executable_bytes: i64,
    /// `(role, name, sha256)`
    pub artifacts: Vec<(String, String, String)>,
}

/// What `GetPublishedGuardManifest` gives other areas.
pub type Published = Payload;

/// `parseAndVerifyGuardEnvelope`: the envelope's key id, and the payload it signs; the message is the error text.
pub fn parse_and_verify(
    value: &Value,
    changelist: u32,
    extension_roles: &[&str],
    now: i64,
    trusted: &Trusted,
) -> Result<(String, Payload), String> {
    let envelope = value.as_object();
    let (Some(envelope), true) = (
        envelope,
        value.get("schema").is_some_and(|s| safe_int(Some(s)) == Some(1)),
    ) else {
        return Err("Invalid Guard manifest envelope.".into());
    };
    let (Some(Value::String(key_id)), Some(Value::String(payload_b64)), Some(Value::String(signature_text))) = (
        envelope.get("keyId"),
        envelope.get("payloadBase64"),
        envelope.get("signature"),
    ) else {
        return Err("Invalid Guard manifest envelope.".into());
    };
    let Some(key) = trusted.key_for(key_id) else {
        return Err("Unknown Guard manifest signing key.".into());
    };
    let payload_bytes = base64_decode(payload_b64);
    let signature = base64_decode(signature_text);
    if payload_bytes.is_empty()
        || payload_bytes.len() > MAX_GUARD_MANIFEST_BYTES
        || signature.len() != 64
        || !verify(key, &payload_bytes, &signature)
    {
        return Err("Guard manifest signature verification failed.".into());
    }
    let payload: Value = serde_json::from_slice(&payload_bytes).map_err(|e| e.to_string())?;
    let invalid = || "Invalid Guard manifest payload.".to_owned();
    let text = |name: &str| payload.get(name).and_then(Value::as_str);
    let manifest_id =
        text("manifestId").filter(|id| id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
    let sequence = safe_int(payload.get("sequence")).filter(|s| *s >= 1);
    let artifacts = payload
        .get("artifacts")
        .and_then(Value::as_array)
        .filter(|a| (1..=128).contains(&a.len()));
    let (
        Some(manifest_id),
        Some(sequence),
        Some(channel),
        Some(platform),
        Some(minimum),
        Some(issued),
        Some(expires),
        Some(artifacts),
    ) = (
        manifest_id,
        sequence,
        text("channel").filter(|c| is_segment_text(c)),
        text("platform").filter(|p| *p == "windows-x86_64"),
        text("minimumLauncherVersion").filter(|v| is_version(v)),
        text("issuedAt"),
        text("expiresAt"),
        artifacts,
    )
    else {
        return Err(invalid());
    };
    let declared_changelist = safe_int(payload.get("targetChangelist"));
    if payload.get("schema").and_then(|s| safe_int(Some(s))) != Some(1)
        || declared_changelist != Some(i64::from(changelist))
    {
        return Err(invalid());
    }
    validate_window(issued, expires, now).map_err(str::to_owned)?;
    let mut names: Vec<String> = Vec::new();
    let mut listed = Vec::new();
    for artifact in artifacts {
        let entry = artifact.as_object();
        let name = entry.and_then(|e| e.get("name")).and_then(Value::as_str);
        let role = entry.and_then(|e| e.get("role")).and_then(Value::as_str);
        let bad = || "Invalid Guard artifact entry.".to_owned();
        let (Some(entry), Some(name), Some(role)) = (entry, name, role) else {
            return Err(bad());
        };
        let lower = name.to_lowercase();
        if name.contains(['/', '\\'])
            || name.is_empty()
            || names.contains(&lower)
            || safe_int(entry.get("size")).is_none_or(|s| s < 1)
            || !entry.get("sha256").is_some_and(is_hex64)
            || !entry.get("required").is_some_and(Value::is_boolean)
            || !(CORE_ARTIFACT_ROLES.contains(&role) || extension_roles.contains(&role))
        {
            return Err(bad());
        }
        names.push(lower);
        if let Some(regions) = entry.get("protectedRegions").filter(|r| !r.is_null()) {
            let ok = regions.as_array().is_some_and(|regions| {
                regions.len() <= 256
                    && regions.iter().all(|region| {
                        safe_int(region.get("rva")).is_some_and(|r| r >= 0)
                            && safe_int(region.get("size")).is_some_and(|s| (1..=1024 * 1024).contains(&s))
                            && region.get("sha256").is_some_and(is_hex64)
                    })
            });
            if !ok {
                return Err("Invalid Guard protected-region entry.".into());
            }
        }
        let sha = entry
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        listed.push((role.to_owned(), name.to_owned(), sha));
    }
    let policy = payload.get("executableMemoryPolicy").and_then(Value::as_object);
    let private = policy.and_then(|p| safe_int(p.get("maximumPrivateExecutableBytes")));
    let writable_value = policy
        .and_then(|p| p.get("maximumWritableExecutableBytes"))
        .filter(|v| !v.is_null());
    let writable = match writable_value {
        None => Some(0),
        Some(v) => safe_int(Some(v)),
    };
    let hashes_ok = policy
        .and_then(|p| p.get("allowlistedModuleHashes"))
        .and_then(Value::as_array)
        .is_some_and(|hashes| hashes.len() <= 512 && hashes.iter().all(is_hex64));
    let (Some(private), Some(writable), true) = (private, writable, hashes_ok) else {
        return Err("Invalid Guard executable-memory policy.".into());
    };
    if !(0..=64 * 1024 * 1024).contains(&private) || !(0..=16 * 1024 * 1024).contains(&writable) {
        return Err("Invalid Guard executable-memory policy.".into());
    }
    Ok((
        key_id.clone(),
        Payload {
            manifest_id: manifest_id.to_owned(),
            sequence,
            channel: channel.to_owned(),
            platform: platform.to_owned(),
            target_changelist: i64::from(changelist),
            minimum_launcher_version: minimum.to_owned(),
            expires_at: expires.to_owned(),
            maximum_private_executable_bytes: private,
            maximum_writable_executable_bytes: writable,
            artifacts: listed,
        },
    ))
}

pub fn now() -> i64 {
    now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_enforced_like_the_publisher() {
        let now = 1_792_454_400_000; // 2026-10-20T00:00:00Z
        let ok = validate_window("2026-10-19T00:00:00Z", "2026-11-19T00:00:00Z", now);
        assert_eq!(ok, Ok(()));
        assert_eq!(
            validate_window("soon", "2026-11-19T00:00:00Z", now),
            Err("Guard manifest timestamps are invalid.")
        );
        assert_eq!(
            validate_window("2026-11-19T00:00:00Z", "2026-10-19T00:00:00Z", now),
            Err("Guard manifest expiry is not after its issue time.")
        );
        assert_eq!(
            validate_window("2026-01-01T00:00:00Z", "2028-01-01T00:00:00Z", now),
            Err("Guard manifest lifetime exceeds the maximum allowed window.")
        );
        assert_eq!(
            validate_window("2026-10-20T00:06:00Z", "2026-11-19T00:00:00Z", now),
            Err("Guard manifest issue time is too far in the future.")
        );
        assert_eq!(
            validate_window("2026-10-20T00:05:00Z", "2026-11-19T00:00:00Z", now),
            Ok(())
        );
        assert_eq!(
            validate_window("2026-09-19T00:00:00Z", "2026-10-19T00:00:00Z", now),
            Err("Guard manifest has expired.")
        );
    }

    #[test]
    fn versions_and_segments_match_the_regexes() {
        for good in ["1.2.3", "10.20.30-beta.1", "0.1.65+build-7"] {
            assert!(is_version(good), "{good}");
        }
        for bad in ["1.2", "1.2.3.4", "a.b.c", "1.2.3-", "1.2.3-é", ""] {
            assert!(!is_version(bad), "{bad}");
        }
        assert!(
            is_segment_text("stable") && is_segment_text("a.b_c-d") && !is_segment_text("a/b") && !is_segment_text("")
        );
    }

    #[test]
    fn an_envelope_with_a_wrong_shape_or_key_is_refused() {
        let trusted = Trusted::embedded().unwrap();
        let parse = |value: Value| parse_and_verify(&value, 647_472, &[], 1_792_454_400_000, &trusted).unwrap_err();
        assert_eq!(parse(serde_json::json!({})), "Invalid Guard manifest envelope.");
        assert_eq!(
            parse(serde_json::json!({"schema": 2, "keyId": "x", "payloadBase64": "", "signature": ""})),
            "Invalid Guard manifest envelope."
        );
        assert_eq!(
            parse(serde_json::json!({"schema": 1, "keyId": "other", "payloadBase64": "", "signature": ""})),
            "Unknown Guard manifest signing key."
        );
        assert_eq!(
            parse(
                serde_json::json!({"schema": 1, "keyId": "guard-2026-10", "payloadBase64": "e30=", "signature": "AAAA"})
            ),
            "Guard manifest signature verification failed."
        );
    }

    #[test]
    fn the_embedded_keys_are_ed25519_keys() {
        let trusted = Trusted::embedded().unwrap();
        assert!(!trusted.verify_runtime(b"data", &[0u8; 64]));
        assert!(!trusted.verify_runtime(b"data", b"short"));
    }
}
