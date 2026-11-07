// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! One-time enrollment codes and URI parsing (`docs/remote-management.md` §5.2,
//! `docs/security/remote-management.md`).
//!
//! Format:
//! `dainty-lass-remote://enroll?v=1&url=<percent-encoded>&fp=<64-hex>&node=<id>&region=<region>&token=<base64url>[&label=<percent-encoded>]`

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use module_api::network::{NodeId, Region};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use url::Url;

use crate::RemoteError;

/// URI scheme of enrollment invites.
pub const ENROLL_SCHEME: &str = "dainty-lass-remote";
/// URI host of enrollment invites.
pub const ENROLL_HOST: &str = "enroll";
/// Invite format version (`v=`).
pub const ENROLL_VERSION: u32 = 1;
/// Random bytes in an enrollment token.
pub const TOKEN_BYTES: usize = 32;
/// How long an invite is valid when the operator sets nothing: 15 minutes.
pub const DEFAULT_ENROLL_TTL_SECS: i64 = 15 * 60;
/// Longest validity an invite may have: one day.
pub const MAX_ENROLL_TTL_SECS: i64 = 24 * 60 * 60;
/// Longest enrollment URI, in bytes.
pub const MAX_ENROLL_URI_LEN: usize = 2048;
/// Longest install label, in bytes.
pub const MAX_LABEL_LEN: usize = 64;

/// Parsed contents of a `dainty-lass-remote://enroll?...` one-time invitation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentInvite {
    /// The controller's base URL, `scheme://host[:port]`.
    pub url: String,
    /// SHA-256 of the controller's certificate, 64 lowercase hex: checked before the token is sent.
    pub controller_fp: String,
    /// Node id the install is enrolled as.
    pub node: NodeId,
    /// Region the install is enrolled in.
    pub region: Region,
    /// The one-time token, URL-safe base64.
    pub token: String,
    /// Display label for the install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Stored record on the controller for a minted enrollment token.
/// Only the SHA-256 digest of the token is persisted, never the raw token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEnrollment {
    /// SHA-256 of the token, lowercase hex; the token itself is never stored.
    pub token_sha256_hex: String,
    /// Node id the install will have.
    pub node: NodeId,
    /// Region it will be in.
    pub region: Region,
    /// Display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// When it was minted, in Unix milliseconds.
    pub created_at_ms: i64,
    /// When it stops being accepted, in Unix milliseconds.
    pub expires_at_ms: i64,
    /// Whether it was redeemed.
    pub used: bool,
}

/// Generate a fresh 32-byte CSPRNG token encoded as URL-safe base64 without padding.
pub fn generate_enrollment_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Compute lowercase 64-character hex SHA-256 of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Check that a string is a 64-character lowercase hexadecimal SHA-256 fingerprint.
pub fn is_valid_sha256_fp(fp: &str) -> bool {
    fp.len() == 64 && fp.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Verify a DER certificate's SHA-256 fingerprint against the expected 64-char lowercase hex string
/// in constant time.
pub fn verify_cert_fingerprint(cert_der: &[u8], expected_fp: &str) -> Result<(), RemoteError> {
    if !is_valid_sha256_fp(expected_fp) {
        return Err(RemoteError::InvalidInput(
            "expected certificate fingerprint must be 64 lowercase hex characters".to_string(),
        ));
    }
    let actual_fp = sha256_hex(cert_der);
    if actual_fp.as_bytes().ct_eq(expected_fp.as_bytes()).into() {
        Ok(())
    } else {
        Err(RemoteError::CertificateMismatch {
            expected: expected_fp.to_string(),
            actual: actual_fp,
        })
    }
}

/// Validate a human-readable install label (1..=64 printable characters, no control chars).
pub fn validate_label(label: &str) -> Result<String, RemoteError> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err(RemoteError::InvalidInput("install label cannot be empty".to_string()));
    }
    if trimmed.chars().count() > MAX_LABEL_LEN {
        return Err(RemoteError::InvalidInput(format!(
            "install label exceeds {MAX_LABEL_LEN} characters"
        )));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(RemoteError::InvalidInput(
            "install label cannot contain control characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Validate the controller gateway URL carried in an enrollment invite.
/// Only `https://` URLs with an explicit host (or `http://127.0.0.1` / `http://localhost` for local tests)
/// and no credentials, query, or fragment are accepted.
pub fn validate_controller_url(raw: &str) -> Result<String, RemoteError> {
    if raw.len() > 512 {
        return Err(RemoteError::InvalidInput("controller URL is too long".to_string()));
    }
    let parsed = Url::parse(raw).map_err(|e| RemoteError::InvalidInput(format!("invalid controller URL: {e}")))?;
    let scheme = parsed.scheme();
    let host = parsed
        .host_str()
        .ok_or_else(|| RemoteError::InvalidInput("controller URL is missing a host".to_string()))?;
    let is_loopback_http = scheme == "http" && (host == "127.0.0.1" || host == "localhost" || host == "[::1]");
    if scheme != "https" && !is_loopback_http {
        return Err(RemoteError::InvalidInput(
            "controller URL must use https://".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(RemoteError::InvalidInput(
            "controller URL must not contain userinfo credentials".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(RemoteError::InvalidInput(
            "controller URL must not contain query parameters or fragments".to_string(),
        ));
    }
    if parsed.path() != "/" && !parsed.path().is_empty() {
        return Err(RemoteError::InvalidInput(
            "controller URL must point to the gateway root".to_string(),
        ));
    }
    let port_part = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    Ok(format!("{scheme}://{host}{port_part}"))
}

/// Validate a base64url-encoded 32-byte enrollment token.
pub fn validate_enrollment_token(token: &str) -> Result<[u8; TOKEN_BYTES], RemoteError> {
    if token.len() != 43 {
        return Err(RemoteError::InvalidInput(
            "enrollment token must be 43 base64url characters (32 bytes)".to_string(),
        ));
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(token.as_bytes())
        .map_err(|_| RemoteError::InvalidInput("enrollment token is not valid base64url".to_string()))?;
    let arr: [u8; TOKEN_BYTES] = decoded
        .try_into()
        .map_err(|_| RemoteError::InvalidInput("enrollment token must decode to 32 bytes".to_string()))?;
    if arr.iter().all(|&b| b == 0) {
        return Err(RemoteError::InvalidInput(
            "enrollment token cannot be all zeros".to_string(),
        ));
    }
    Ok(arr)
}

impl EnrollmentInvite {
    /// Checks every field is valid and in canonical form.
    pub fn validate(&self) -> Result<(), RemoteError> {
        let normalized_url = validate_controller_url(&self.url)?;
        if normalized_url != self.url {
            return Err(RemoteError::InvalidInput(
                "controller URL must be in canonical scheme://host[:port] form".to_string(),
            ));
        }
        if !is_valid_sha256_fp(&self.controller_fp) {
            return Err(RemoteError::InvalidInput(
                "controller fingerprint must be 64 lowercase hex characters".to_string(),
            ));
        }
        let _ = validate_enrollment_token(&self.token)?;
        if let Some(label) = &self.label {
            let checked = validate_label(label)?;
            if &checked != label {
                return Err(RemoteError::InvalidInput(
                    "label must not have leading or trailing whitespace".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// The invite as a `dainty-lass-remote://enroll?...` URI, after [`EnrollmentInvite::validate`].
    pub fn to_uri(&self) -> Result<String, RemoteError> {
        self.validate()?;
        let encoded_url = utf8_percent_encode(&self.url, NON_ALPHANUMERIC).to_string();
        let encoded_node = utf8_percent_encode(self.node.as_str(), NON_ALPHANUMERIC).to_string();
        let encoded_region = utf8_percent_encode(self.region.as_str(), NON_ALPHANUMERIC).to_string();
        let mut uri = format!(
            "{ENROLL_SCHEME}://{ENROLL_HOST}?v={ENROLL_VERSION}&url={encoded_url}&fp={}&node={encoded_node}&region={encoded_region}&token={}",
            self.controller_fp, self.token
        );
        if let Some(label) = &self.label {
            let encoded_label = utf8_percent_encode(label, NON_ALPHANUMERIC).to_string();
            uri.push_str("&label=");
            uri.push_str(&encoded_label);
        }
        if uri.len() > MAX_ENROLL_URI_LEN {
            return Err(RemoteError::InvalidInput(
                "enrollment URI exceeds maximum length".to_string(),
            ));
        }
        Ok(uri)
    }
}

/// Parse and validate a `dainty-lass-remote://enroll?...` URI.
pub fn parse_enrollment_uri(input: &str) -> Result<EnrollmentInvite, RemoteError> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_ENROLL_URI_LEN {
        return Err(RemoteError::InvalidInput(
            "enrollment URI is empty or exceeds 2048 bytes".to_string(),
        ));
    }
    let prefix = format!("{ENROLL_SCHEME}://{ENROLL_HOST}?");
    let query = trimmed
        .strip_prefix(&prefix)
        .ok_or_else(|| RemoteError::InvalidInput(format!("enrollment URI must start with {prefix}")))?;
    if query.contains('#') {
        return Err(RemoteError::InvalidInput(
            "enrollment URI must not contain a fragment".to_string(),
        ));
    }

    let mut v: Option<String> = None;
    let mut url: Option<String> = None;
    let mut fp: Option<String> = None;
    let mut node: Option<String> = None;
    let mut region: Option<String> = None;
    let mut token: Option<String> = None;
    let mut label: Option<String> = None;

    for pair in query.split('&') {
        if pair.is_empty() {
            return Err(RemoteError::InvalidInput(
                "enrollment URI contains an empty query parameter".to_string(),
            ));
        }
        let (k, raw_v) = pair
            .split_once('=')
            .ok_or_else(|| RemoteError::InvalidInput("enrollment URI parameter is missing '='".to_string()))?;
        let decoded = percent_decode_str(raw_v)
            .decode_utf8()
            .map_err(|_| RemoteError::InvalidInput("enrollment URI parameter is not valid UTF-8".to_string()))?
            .into_owned();
        let slot = match k {
            "v" => &mut v,
            "url" => &mut url,
            "fp" => &mut fp,
            "node" => &mut node,
            "region" => &mut region,
            "token" => &mut token,
            "label" => &mut label,
            other => {
                return Err(RemoteError::InvalidInput(format!(
                    "unknown enrollment URI parameter: {other}"
                )));
            }
        };
        if slot.is_some() {
            return Err(RemoteError::InvalidInput(format!(
                "duplicate enrollment URI parameter: {k}"
            )));
        }
        *slot = Some(decoded);
    }

    if v.as_deref() != Some("1") {
        return Err(RemoteError::InvalidInput(
            "unsupported enrollment URI version (expected v=1)".to_string(),
        ));
    }
    let raw_url = url.ok_or_else(|| RemoteError::InvalidInput("missing url parameter".to_string()))?;
    let canonical_url = validate_controller_url(&raw_url)?;
    let controller_fp = fp.ok_or_else(|| RemoteError::InvalidInput("missing fp parameter".to_string()))?;
    if !is_valid_sha256_fp(&controller_fp) {
        return Err(RemoteError::InvalidInput(
            "fp must be 64 lowercase hex characters".to_string(),
        ));
    }
    let node_str = node.ok_or_else(|| RemoteError::InvalidInput("missing node parameter".to_string()))?;
    let parsed_node: NodeId = node_str
        .parse()
        .map_err(|e| RemoteError::InvalidInput(format!("invalid node id: {e}")))?;
    let region_str = region.ok_or_else(|| RemoteError::InvalidInput("missing region parameter".to_string()))?;
    let parsed_region: Region = region_str
        .parse()
        .map_err(|e| RemoteError::InvalidInput(format!("invalid region: {e}")))?;
    let token_str = token.ok_or_else(|| RemoteError::InvalidInput("missing token parameter".to_string()))?;
    let _ = validate_enrollment_token(&token_str)?;
    let parsed_label = match label {
        Some(l) => Some(validate_label(&l)?),
        None => None,
    };

    let invite = EnrollmentInvite {
        url: canonical_url,
        controller_fp,
        node: parsed_node,
        region: parsed_region,
        token: token_str,
        label: parsed_label,
    };
    invite.validate()?;
    Ok(invite)
}
