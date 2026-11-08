// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Reading the older Electron launcher's `.key` files, never writing them (spec §2.7.1). Electron seals with
//! Chromium's scheme: on Windows a DPAPI-protected AES-256-GCM key in `Local State`, on Linux AES-128-CBC under a
//! PBKDF2 key from a fixed password (`v10`) or one kept in the Secret Service (`v11`).

use std::path::{Path, PathBuf};

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeDecrypt, KeyIvInit};
use base64::Engine as _;
use sha1::Sha1;

use super::format::{ELECTRON_V10, split_cbc, split_gcm};
use super::os::open_gcm;

/// The file next to the old launcher's data that holds its sealed AES key (Windows).
pub const LOCAL_STATE_FILE: &str = "Local State";
/// What a DPAPI-sealed key in `Local State` starts with.
pub const DPAPI_PREFIX: &[u8] = b"DPAPI";
/// Chromium's PBKDF2 salt.
pub const SALT: &[u8] = b"saltysalt";
/// The password `v10` files on Linux are sealed with.
pub const V10_PASSWORD: &[u8] = b"peanuts";
/// The CBC IV: 16 spaces.
pub const CBC_IV: [u8; 16] = [b' '; 16];
/// The Secret Service item holding the password of `v11` files.
pub const V11_PASSWORD_ATTRIBUTES: [(&str, &str); 2] = [
    ("xdg:schema", "chrome_libsecret_os_crypt_password_v2"),
    ("application", "Dauntless Reforged Revival Launcher"),
];

/// Opens a `.key` file's bytes, or `None` when it can't.
pub trait LegacyReader: Send + Sync {
    /// The decrypted text of `file`.
    fn open(&self, file: &[u8]) -> Option<Vec<u8>>;
}

/// The AES-128 key Chromium derives from `password`: PBKDF2-HMAC-SHA1, `saltysalt`, one iteration.
#[must_use]
pub fn cbc_key(password: &[u8]) -> [u8; 16] {
    pbkdf2::pbkdf2_hmac_array::<Sha1, 16>(password, SALT, 1)
}

/// A Linux file: `v10` under [`V10_PASSWORD`], or `v11` under the password `v11_password` finds.
pub fn open_cbc(file: &[u8], v11_password: impl FnOnce() -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let (v11, body) = split_cbc(file)?;
    let key = if v11 {
        cbc_key(&v11_password()?)
    } else {
        cbc_key(V10_PASSWORD)
    };
    cbc::Decryptor::<aes::Aes128>::new(&key.into(), &CBC_IV.into())
        .decrypt_padded_vec::<Pkcs7>(body)
        .ok()
}

/// The AES-256 key in a `Local State` file's `os_crypt.encrypted_key`, opened by `unprotect` (DPAPI).
pub fn master_key(local_state: &str, unprotect: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Option<[u8; 32]> {
    let state: serde_json::Value = serde_json::from_str(local_state).ok()?;
    let encoded = state.get("os_crypt")?.get("encrypted_key")?.as_str()?;
    let sealed = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let key = unprotect(sealed.strip_prefix(DPAPI_PREFIX)?)?;
    key.as_slice().try_into().ok()
}

/// A Windows file: `v10` + nonce + AES-256-GCM ciphertext and tag under `key`.
#[must_use]
pub fn open_gcm_file(file: &[u8], key: &[u8; 32]) -> Option<Vec<u8>> {
    let (nonce, sealed) = split_gcm(file, ELECTRON_V10)?;
    open_gcm(key, nonce, sealed)
}

/// This OS's reader for the files under `user_data` (the old launcher's data, moved there by `paths`).
#[derive(Debug, Clone)]
pub struct ElectronReader {
    user_data: PathBuf,
}

impl ElectronReader {
    /// A reader for the old launcher's data in `user_data`.
    #[must_use]
    pub fn new(user_data: &Path) -> Self {
        Self {
            user_data: user_data.to_path_buf(),
        }
    }

    /// Whether this OS has a reader at all.
    #[must_use]
    pub const fn supported() -> bool {
        cfg!(any(windows, target_os = "linux"))
    }
}

impl LegacyReader for ElectronReader {
    #[cfg(windows)]
    fn open(&self, file: &[u8]) -> Option<Vec<u8>> {
        let state = std::fs::read_to_string(self.user_data.join(LOCAL_STATE_FILE)).ok()?;
        let key = master_key(&state, super::os::dpapi::unprotect)?;
        open_gcm_file(file, &key)
    }

    #[cfg(target_os = "linux")]
    fn open(&self, file: &[u8]) -> Option<Vec<u8>> {
        let _ = &self.user_data;
        open_cbc(file, || {
            super::os::secret_service::lookup(&V11_PASSWORD_ATTRIBUTES)
                .ok()
                .flatten()
        })
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn open(&self, _file: &[u8]) -> Option<Vec<u8>> {
        let _ = &self.user_data;
        None
    }
}

#[cfg(test)]
mod tests {
    use aes::cipher::BlockModeEncrypt;
    use aes_gcm::aead::{Aead, KeyInit};

    use super::*;

    // Chromium's well-known v10 key on Linux, PBKDF2-HMAC-SHA1("peanuts", "saltysalt", 1, 16).
    const PEANUTS_KEY: [u8; 16] = [
        0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27, 0x78,
    ];

    fn cbc_file(prefix: &[u8], key: &[u8; 16], plain: &[u8]) -> Vec<u8> {
        let body =
            cbc::Encryptor::<aes::Aes128>::new(&(*key).into(), &CBC_IV.into()).encrypt_padded_vec::<Pkcs7>(plain);
        [prefix, &body].concat()
    }

    #[test]
    fn peanuts_derives_chromiums_key() {
        assert_eq!(cbc_key(V10_PASSWORD), PEANUTS_KEY);
    }

    #[test]
    fn opens_v10_with_the_known_key() {
        let file = cbc_file(b"v10", &PEANUTS_KEY, b"RRK_abcdefghijklmnop");
        assert_eq!(file.len(), 3 + 32);
        assert_eq!(
            open_cbc(&file, || panic!("v10 needs no password")).unwrap(),
            b"RRK_abcdefghijklmnop"
        );
    }

    #[test]
    fn opens_v11_with_the_services_password() {
        let file = cbc_file(b"v11", &cbc_key(b"from the keyring"), b"RRK2\n-\nRRK_abcdefghijklmnop");
        assert_eq!(
            open_cbc(&file, || Some(b"from the keyring".to_vec())).unwrap(),
            b"RRK2\n-\nRRK_abcdefghijklmnop"
        );
        assert_eq!(open_cbc(&file, || None), None);
        assert_eq!(open_cbc(&file, || Some(b"peanuts".to_vec())), None);
    }

    #[test]
    fn refuses_bad_cbc_files() {
        let file = cbc_file(b"v10", &PEANUTS_KEY, b"0123456789abcdef");
        assert_eq!(open_cbc(&file[..file.len() - 1], || None), None);
        assert_eq!(open_cbc(b"v10", || None), None);
        assert_eq!(open_cbc(&[b"v12", &file[3..]].concat(), || None), None);
        let wrong = cbc_file(b"v10", &[0; 16], b"0123456789abcdef");
        assert_ne!(
            open_cbc(&wrong, || None).as_deref(),
            Some(b"0123456789abcdef".as_slice())
        );
    }

    #[test]
    fn master_key_from_local_state() {
        let key = [5u8; 32];
        let encoded = base64::engine::general_purpose::STANDARD.encode([b"DPAPI".as_slice(), b"sealed"].concat());
        let state = format!(r#"{{"os_crypt":{{"encrypted_key":"{encoded}"}},"other":1}}"#);
        let opened = master_key(&state, |sealed| (sealed == b"sealed").then(|| key.to_vec()));
        assert_eq!(opened, Some(key));

        let no_prefix = base64::engine::general_purpose::STANDARD.encode(b"sealed");
        let state = format!(r#"{{"os_crypt":{{"encrypted_key":"{no_prefix}"}}}}"#);
        assert_eq!(master_key(&state, |_| Some(key.to_vec())), None);
        assert_eq!(
            master_key(&format!(r#"{{"os_crypt":{{"encrypted_key":"{encoded}"}}}}"#), |_| Some(
                vec![1; 16]
            )),
            None
        );
        assert_eq!(master_key("{}", |_| Some(key.to_vec())), None);
        assert_eq!(master_key("not json", |_| Some(key.to_vec())), None);
    }

    #[test]
    fn opens_windows_gcm_files() {
        let key = [9u8; 32];
        let nonce = [1u8; 12];
        let sealed = aes_gcm::Aes256Gcm::new_from_slice(&key)
            .unwrap()
            .encrypt(&aes_gcm::Nonce::from(nonce), b"RRK_abcdefghijklmnop".as_slice())
            .unwrap();
        let file = [b"v10".as_slice(), &nonce, &sealed].concat();
        assert_eq!(open_gcm_file(&file, &key).unwrap(), b"RRK_abcdefghijklmnop");
        assert_eq!(open_gcm_file(&file, &[8; 32]), None);
        assert_eq!(open_gcm_file(&[b"v11".as_slice(), &file[3..]].concat(), &key), None);
    }
}
