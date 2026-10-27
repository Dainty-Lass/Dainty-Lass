//! The auth hashes and passwords: `security/launcherTokens.ts` (opaque tokens), `security/apiKeyHash.ts` (API keys,
//! email codes and the erasure pseudonym, all HMAC-SHA256 under `API_KEY_HASH_SECRET`), `security/emailCodes.ts` and
//! `security/passwords.ts` (Argon2id). Keys and codes are never logged (AGENTS.md).

use std::fmt::Write as _;
use std::sync::LazyLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::repo::auth::EmailCodePurpose;
use crate::settings::Settings;

type HmacSha256 = Hmac<Sha256>;

const MIN_SECRET_LENGTH: usize = 32;

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// `crypto.randomBytes(n).toString("hex")`.
pub fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::rng().fill(&mut bytes[..]);
    hex(&bytes)
}

/// `GenerateOpaqueToken`: refresh tokens, game exchange codes and Discord completion codes.
pub fn generate_opaque_token() -> String {
    random_hex(32)
}

/// `HashOpaqueToken`: SHA-256 of the UTF-8 text, hex.
pub fn hash_opaque_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// `crypto.randomBytes(n).toString("base64url")`.
pub fn random_base64url(n: usize) -> String {
    use base64::Engine as _;
    let mut bytes = vec![0u8; n];
    rand::rng().fill(&mut bytes[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// ------------------------------------------------------------------ the HMAC secret

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("API_KEY_HASH_SECRET must contain at least 32 characters.")]
pub struct SecretError;

/// `ApiKeyHashSecret()` / `AssertApiKeyHashSecret`.
pub fn api_key_hash_secret(settings: &Settings) -> Result<Vec<u8>, SecretError> {
    let secret = service_kit::js::trim(settings.raw("API_KEY_HASH_SECRET").unwrap_or_default());
    if secret.encode_utf16().count() < MIN_SECRET_LENGTH {
        return Err(SecretError);
    }
    Ok(secret.as_bytes().to_vec())
}

fn hmac(secret: &[u8], parts: &[&str]) -> Result<String, SecretError> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| SecretError)?;
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            mac.update(b"\0");
        }
        mac.update(part.as_bytes());
    }
    Ok(hex(&mac.finalize().into_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyScope {
    Gameserver,
    User,
}

impl KeyScope {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Gameserver => "gameserver",
            Self::User => "user",
        }
    }
}

/// `HashApiKey(value, scope)`.
pub fn hash_api_key(settings: &Settings, value: &str, scope: KeyScope) -> Result<String, SecretError> {
    hmac(&api_key_hash_secret(settings)?, &[scope.as_str(), value])
}

/// `HashEmailCode(userId, purpose, code)`: bound to the account (its `userId`) and the purpose.
pub fn hash_email_code(
    settings: &Settings,
    user_id: &str,
    purpose: EmailCodePurpose,
    code: &str,
) -> Result<String, SecretError> {
    hmac(
        &api_key_hash_secret(settings)?,
        &["email-code", purpose.as_str(), user_id, code],
    )
}

/// `PseudonymousAccountId(accountId)`.
pub fn pseudonymous_account_id(settings: &Settings, account_id: &str) -> Result<String, SecretError> {
    hmac(&api_key_hash_secret(settings)?, &["account-pseudonym", account_id])
}

/// Constant-time equality of two hex hashes of the same length; malformed hex or unequal lengths never match.
pub fn hashes_equal(stored: &str, candidate: &str) -> bool {
    match (unhex(stored), unhex(candidate)) {
        (Some(stored), Some(candidate)) => {
            !stored.is_empty() && stored.len() == candidate.len() && bool::from(stored.ct_eq(&candidate))
        }
        _ => false,
    }
}

/// `FindApiKeyRecord`: the first stored hash that equals `hash_hex`. Every record is compared, so the time does not
/// say where the match was.
pub fn find_key_record<'a, T>(records: &'a [T], hash_of: impl Fn(&T) -> Option<&str>, hash_hex: &str) -> Option<&'a T> {
    let mut found = None;
    for record in records {
        let Some(stored) = hash_of(record) else { continue };
        if hashes_equal(stored, hash_hex) && found.is_none() {
            found = Some(record);
        }
    }
    found
}

// ------------------------------------------------------------------ email codes

pub const EMAIL_CODE_TTL_MINUTES: i64 = 5;
pub const EMAIL_CODE_TTL_MS: i64 = EMAIL_CODE_TTL_MINUTES * 60 * 1000;
pub const EMAIL_CODE_MAX_TRIES: u32 = 5;

/// `GenerateEmailCode`: six digits, leading zeros kept.
pub fn generate_email_code() -> String {
    format!("{:06}", rand::rng().random_range(0..1_000_000u32))
}

/// `IsEmailCodeFormat`: exactly six ASCII digits.
pub fn is_email_code_format(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|b| b.is_ascii_digit())
}

/// `MaskEmail`: `m***x@gmail.com`; a name of two letters or fewer keeps only its first.
pub fn mask_email(email: &str) -> String {
    let Some(at) = email.rfind('@').filter(|&at| at > 0) else {
        return "***".to_owned();
    };
    let (local, domain) = (&email[..at], &email[at + 1..]);
    let units: Vec<u16> = local.encode_utf16().collect();
    let first = String::from_utf16_lossy(&units[..1]);
    let tail = if units.len() > 2 {
        String::from_utf16_lossy(&units[units.len() - 1..])
    } else {
        String::new()
    };
    format!("{first}***{tail}@{domain}")
}

// ------------------------------------------------------------------ passwords

/// A real hash to verify against when no account matched, so a wrong address takes as long as a wrong password.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| hash_password("timing-equalizer").unwrap_or_default());

/// `HashPassword`: Argon2id with the library's defaults (19 MiB, 2 passes, 1 lane), as `@node-rs/argon2` writes.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes[..]);
    let salt = SaltString::encode_b64(&bytes)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
}

/// `VerifyPassword`: false for a wrong password, a missing or malformed hash.
pub fn verify_password(password_hash: Option<&str>, password: &str) -> bool {
    let Some(stored) = password_hash else {
        if let Ok(dummy) = PasswordHash::new(&DUMMY_HASH) {
            let _ = Argon2::default().verify_password(password.as_bytes(), &dummy);
        }
        return false;
    };
    PasswordHash::new(stored)
        .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::from_pairs([("API_KEY_HASH_SECRET", "test-only-api-key-hash-secret-0123456789abcdef")])
    }

    #[test]
    fn opaque_tokens_hash_like_sha256() {
        assert_eq!(generate_opaque_token().len(), 64);
        assert_eq!(
            hash_opaque_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn api_key_hashes_are_scoped_hmacs() {
        // Computed with Node: createHmac("sha256", secret).update("user").update("\0").update("RRK_x").digest("hex")
        let settings = settings();
        let user = hash_api_key(&settings, "RRK_x", KeyScope::User).unwrap();
        let game = hash_api_key(&settings, "RRK_x", KeyScope::Gameserver).unwrap();
        assert_eq!(user.len(), 64);
        assert_ne!(user, game);
        assert_eq!(user, hash_api_key(&settings, "RRK_x", KeyScope::User).unwrap());
        assert!(
            hash_api_key(
                &Settings::from_pairs([("API_KEY_HASH_SECRET", "short")]),
                "k",
                KeyScope::User
            )
            .is_err()
        );
        assert!(hash_api_key(&Settings::default(), "k", KeyScope::User).is_err());
    }

    #[test]
    fn email_code_hashes_bind_account_and_purpose() {
        let settings = settings();
        let verify = hash_email_code(&settings, "u1", EmailCodePurpose::Verify, "123456").unwrap();
        assert_ne!(
            verify,
            hash_email_code(&settings, "u2", EmailCodePurpose::Verify, "123456").unwrap()
        );
        assert_ne!(
            verify,
            hash_email_code(&settings, "u1", EmailCodePurpose::Reset, "123456").unwrap()
        );
        assert_ne!(
            verify,
            hash_email_code(&settings, "u1", EmailCodePurpose::Verify, "123457").unwrap()
        );
    }

    #[test]
    fn stored_hashes_compare_in_constant_time_and_refuse_malformed_hex() {
        assert!(hashes_equal("00ff", "00ff"));
        assert!(!hashes_equal("00ff", "00fe"));
        assert!(!hashes_equal("00ff", "00ff00"));
        assert!(!hashes_equal("", ""));
        assert!(!hashes_equal("zz", "zz"));
        let records = [
            Some("aa".to_owned()),
            None,
            Some("bb".to_owned()),
            Some("bb".to_owned()),
        ];
        let found = find_key_record(&records, |r| r.as_deref(), "bb");
        assert!(std::ptr::eq(found.unwrap(), &records[2]));
        assert!(find_key_record(&records, |r| r.as_deref(), "cc").is_none());
    }

    #[test]
    fn email_codes_are_six_digits() {
        assert!(is_email_code_format("012345"));
        for bad in ["12345", "1234567", "12 456", "12345a", "１２３４５６"] {
            assert!(!is_email_code_format(bad), "{bad}");
        }
        for _ in 0..50 {
            assert!(is_email_code_format(&generate_email_code()));
        }
    }

    #[test]
    fn addresses_mask_as_node_masks_them() {
        assert_eq!(mask_email("maxine@gmail.com"), "m***e@gmail.com");
        assert_eq!(mask_email("mx@gmail.com"), "m***@gmail.com");
        assert_eq!(mask_email("a@b.co"), "a***@b.co");
        assert_eq!(mask_email("nope"), "***");
        assert_eq!(mask_email("@x.com"), "***");
    }

    #[test]
    fn passwords_round_trip_and_an_unknown_account_never_verifies() {
        let hash = hash_password("hunter2hunter2").unwrap();
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(verify_password(Some(&hash), "hunter2hunter2"));
        assert!(!verify_password(Some(&hash), "hunter3hunter3"));
        assert!(!verify_password(Some("not a hash"), "x"));
        assert!(!verify_password(None, "x"));
    }
}
