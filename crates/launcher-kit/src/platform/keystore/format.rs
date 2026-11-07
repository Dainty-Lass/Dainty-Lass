// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! What the key files hold once decrypted, and the framing of the encrypted files (spec §2.7.1).

use protocol::account::is_plausible_account_key;
use protocol::invite::is_valid_fingerprint;

/// The first line of the current text format.
pub const TEXT_MAGIC: &str = "RRK2";
/// Stands for "no certificate" on the fingerprint line.
pub const NO_FINGERPRINT: &str = "-";
/// The prefix of a file sealed by the Linux encryptor.
pub const LINUX_MAGIC: &[u8; 3] = b"rr1";
/// The prefix of an Electron file sealed with AES-GCM (Windows) or the fixed password (Linux).
pub const ELECTRON_V10: &[u8; 3] = b"v10";
/// The prefix of an Electron file sealed with the Secret Service's password (Linux).
pub const ELECTRON_V11: &[u8; 3] = b"v11";
/// An AES-GCM nonce's size.
pub const NONCE_BYTES: usize = 12;
/// An AES-GCM tag's size.
pub const TAG_BYTES: usize = 16;
/// An AES block's size.
pub const BLOCK_BYTES: usize = 16;

/// A stored account key and the certificate it was saved for.
#[derive(Clone, PartialEq, Eq)]
pub struct Stored {
    /// The certificate's SHA-256, or `None` for a bare key.
    pub fingerprint: Option<String>,
    /// The account key.
    pub key: String,
}

// The key never reaches a `{:?}`.
impl std::fmt::Debug for Stored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stored")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl Stored {
    /// `RRK2\n<fingerprint or ->\n<key>`, with no trailing newline.
    #[must_use]
    pub fn to_text(&self) -> String {
        let fp = self.fingerprint.as_deref().unwrap_or(NO_FINGERPRINT);
        format!("{TEXT_MAGIC}\n{fp}\n{}", self.key)
    }
}

/// The decrypted text of a key file: the `RRK2` format or the first format's bare key. Anything else, including
/// another product's `DRK2`, is no key.
#[must_use]
pub fn parse_text(plain: &[u8]) -> Option<Stored> {
    let text = std::str::from_utf8(plain).ok()?;
    let Some(rest) = text.strip_prefix(TEXT_MAGIC).and_then(|r| r.strip_prefix('\n')) else {
        return is_plausible_account_key(text).then(|| Stored {
            fingerprint: None,
            key: text.to_owned(),
        });
    };
    let (fp, key) = rest.split_once('\n')?;
    if !is_plausible_account_key(key) {
        return None;
    }
    let fingerprint = match fp {
        NO_FINGERPRINT => None,
        fp if is_valid_fingerprint(fp) => Some(fp.to_owned()),
        _ => return None,
    };
    Some(Stored {
        fingerprint,
        key: key.to_owned(),
    })
}

/// `prefix` + nonce + ciphertext and tag, split; `None` when it is not that shape.
#[must_use]
pub fn split_gcm<'a>(sealed: &'a [u8], prefix: &[u8; 3]) -> Option<(&'a [u8; NONCE_BYTES], &'a [u8])> {
    let body = sealed.strip_prefix(prefix.as_slice())?;
    if body.len() < NONCE_BYTES + TAG_BYTES {
        return None;
    }
    let (nonce, rest) = body.split_at(NONCE_BYTES);
    Some((nonce.try_into().ok()?, rest))
}

/// `prefix` + nonce + ciphertext and tag.
#[must_use]
pub fn join_gcm(prefix: &[u8; 3], nonce: &[u8; NONCE_BYTES], sealed: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + NONCE_BYTES + sealed.len());
    out.extend_from_slice(prefix);
    out.extend_from_slice(nonce);
    out.extend_from_slice(sealed);
    out
}

/// The CBC part of a Linux Electron file and whether it is `v11`; it must be non-empty whole blocks.
#[must_use]
pub fn split_cbc(sealed: &[u8]) -> Option<(bool, &[u8])> {
    let (v11, body) = if let Some(body) = sealed.strip_prefix(ELECTRON_V11.as_slice()) {
        (true, body)
    } else {
        (false, sealed.strip_prefix(ELECTRON_V10.as_slice())?)
    };
    (!body.is_empty() && body.len() % BLOCK_BYTES == 0).then_some((v11, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "RRK_abcdefghijklmnopqrstuvwx";
    const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn text_round_trips() {
        for fingerprint in [None, Some(FP.to_owned())] {
            let stored = Stored {
                fingerprint,
                key: KEY.to_owned(),
            };
            let text = stored.to_text();
            assert!(!text.ends_with('\n'));
            assert_eq!(parse_text(text.as_bytes()), Some(stored));
        }
        assert_eq!(
            Stored {
                fingerprint: None,
                key: KEY.into()
            }
            .to_text(),
            format!("RRK2\n-\n{KEY}")
        );
    }

    #[test]
    fn bare_key_is_the_first_format() {
        let stored = parse_text(KEY.as_bytes()).unwrap();
        assert_eq!(stored.fingerprint, None);
        assert_eq!(stored.key, KEY);
    }

    #[test]
    fn refuses_what_is_not_a_key() {
        for text in [
            format!("DRK2\n-\n{KEY}"),
            format!("RRK2\n{}\n{KEY}", FP.to_uppercase()),
            format!("RRK2\nabc\n{KEY}"),
            format!("RRK2\n-\n{KEY}\n"),
            "RRK2\n-\nshort".to_owned(),
            "RRK2\n-".to_owned(),
            "RRK2".to_owned(),
            format!("{KEY}\n"),
            format!("rrk2\n-\n{KEY}"),
            String::new(),
        ] {
            assert_eq!(parse_text(text.as_bytes()), None, "{text:?}");
        }
        assert_eq!(parse_text(&[0xff, 0xfe]), None);
    }

    #[test]
    fn debug_hides_the_key() {
        let stored = parse_text(KEY.as_bytes()).unwrap();
        assert!(!format!("{stored:?}").contains(KEY));
    }

    #[test]
    fn gcm_framing() {
        let nonce = [7u8; NONCE_BYTES];
        let sealed = [9u8; TAG_BYTES + 3];
        let file = join_gcm(LINUX_MAGIC, &nonce, &sealed);
        assert_eq!(&file[..3], b"rr1");
        assert_eq!(split_gcm(&file, LINUX_MAGIC), Some((&nonce, sealed.as_slice())));
        assert_eq!(split_gcm(&file, ELECTRON_V10), None);
        assert_eq!(split_gcm(&file[..3 + NONCE_BYTES + TAG_BYTES - 1], LINUX_MAGIC), None);
        assert!(split_gcm(&file[..3 + NONCE_BYTES + TAG_BYTES], LINUX_MAGIC).is_some());
    }

    #[test]
    fn cbc_framing() {
        assert_eq!(
            split_cbc(&[b"v10".as_slice(), &[1; 32]].concat()),
            Some((false, [1u8; 32].as_slice()))
        );
        assert_eq!(
            split_cbc(&[b"v11".as_slice(), &[1; 16]].concat()),
            Some((true, [1u8; 16].as_slice()))
        );
        assert_eq!(split_cbc(b"v10"), None);
        assert_eq!(split_cbc(&[b"v10".as_slice(), &[1; 17]].concat()), None);
        assert_eq!(split_cbc(&[b"v12".as_slice(), &[1; 16]].concat()), None);
    }
}
