//! `security/totp.ts`: the admin sign-in's authenticator code (RFC 6238: HMAC-SHA1, 30-second steps, six digits).

use aws_lc_rs::hmac;
use serde_json::Value;
use subtle::ConstantTimeEq;

const STEP_MS: i64 = 30_000;

/// `DecodeBase32`: `None` for what Node throws on (a character outside the alphabet).
fn decode_base32(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits = 0_u32;
    let mut count = 0_u32;
    let mut bytes = Vec::new();
    for ch in input
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '=' | '-'))
    {
        let index = ALPHABET.iter().position(|a| char::from(*a) == ch)?;
        bits = (bits << 5) | u32::try_from(index).ok()?;
        count += 5;
        if count >= 8 {
            count -= 8;
            bytes.push(u8::try_from((bits >> count) & 0xff).ok()?);
            bits &= (1 << count) - 1;
        }
    }
    Some(bytes)
}

fn code_at(secret: &[u8], counter: i64) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let digest = hmac::sign(&key, &counter.to_be_bytes());
    let digest = digest.as_ref();
    let offset = usize::from(digest.last().copied().unwrap_or(0) & 0x0f);
    let word = digest
        .get(offset..offset + 4)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map_or(0, |b| u32::from_be_bytes(b) & 0x7fff_ffff);
    format!("{:06}", word % 1_000_000)
}

/// `VerifyTotp(base32Secret, candidate, nowMs)`: six digits, this step or one either side.
pub fn verify_totp_at(base32_secret: &str, candidate: Option<&Value>, now_ms: i64) -> bool {
    let Some(candidate) = candidate.and_then(Value::as_str) else {
        return false;
    };
    if candidate.len() != 6 || !candidate.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Some(secret) = decode_base32(base32_secret) else {
        return false;
    };
    let counter = now_ms.div_euclid(STEP_MS);
    [-1, 0, 1].iter().any(|window| {
        code_at(&secret, counter + window)
            .as_bytes()
            .ct_eq(candidate.as_bytes())
            .into()
    })
}

pub fn verify_totp(base32_secret: &str, candidate: Option<&Value>) -> bool {
    verify_totp_at(base32_secret, candidate, super::util::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // RFC 6238 appendix B, SHA-1, secret "12345678901234567890", T = 59 s: 94287082 (its last six digits)
    const SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn the_rfc_vector_and_its_neighbouring_steps_pass() {
        assert!(verify_totp_at(SECRET, Some(&json!("287082")), 59_000));
        assert!(verify_totp_at(SECRET, Some(&json!("287082")), 59_000 + 30_000));
        assert!(!verify_totp_at(SECRET, Some(&json!("287082")), 59_000 + 90_000));
    }

    #[test]
    fn anything_but_six_digits_of_a_decodable_secret_is_refused() {
        assert!(!verify_totp_at(SECRET, Some(&json!(287_082)), 59_000));
        assert!(!verify_totp_at(SECRET, Some(&json!("28708")), 59_000));
        assert!(!verify_totp_at(SECRET, Some(&json!("28708x")), 59_000));
        assert!(!verify_totp_at(SECRET, None, 59_000));
        assert!(!verify_totp_at("not*base32", Some(&json!("287082")), 59_000));
    }

    #[test]
    fn the_secret_is_read_as_node_reads_it() {
        assert!(verify_totp_at(
            "gezd gnbv-gy3t qojq gezd gnbv gy3t qojq====",
            Some(&json!("287082")),
            59_000
        ));
    }
}
