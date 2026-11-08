// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The OS's protection for key files (spec §2.7.1): DPAPI for the current user on Windows, a random AES-256-GCM
//! key kept in the Secret Service on Linux, nothing elsewhere. Every call blocks (D-Bus, DPAPI): an async caller
//! runs it on the blocking pool.

use std::fmt;
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::TryRng as _;

use super::format::{LINUX_MAGIC, NONCE_BYTES, join_gcm, split_gcm};

/// Why sealing or opening failed. Neither variant carries the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealError {
    /// The OS has no protection to offer (no Secret Service, an unsupported OS).
    Unavailable,
    /// The protection is there but the data could not be sealed or opened.
    Failed,
}

impl fmt::Display for SealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "the OS key protection is unavailable",
            Self::Failed => "the OS key protection failed",
        })
    }
}

impl std::error::Error for SealError {}

/// Seals and opens key files for this user.
pub trait Encryptor: Send + Sync {
    /// Whether [`Encryptor::seal`] can work at all.
    fn available(&self) -> bool;
    /// `plain`, sealed.
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError>;
    /// What [`Encryptor::seal`] made, opened.
    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, SealError>;
}

/// The encryptor where the OS offers none.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unavailable;

impl Encryptor for Unavailable {
    fn available(&self) -> bool {
        false
    }

    fn seal(&self, _plain: &[u8]) -> Result<Vec<u8>, SealError> {
        Err(SealError::Unavailable)
    }

    fn open(&self, _sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        Err(SealError::Unavailable)
    }
}

/// This OS's encryptor.
#[must_use]
pub fn platform_encryptor() -> Arc<dyn Encryptor> {
    #[cfg(windows)]
    {
        Arc::new(Dpapi)
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(SecretServiceKey::default())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Arc::new(Unavailable)
    }
}

/// `rr1` + a random nonce + AES-256-GCM ciphertext and tag.
pub fn seal_rr1(key: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>, SealError> {
    let mut nonce = [0u8; NONCE_BYTES];
    rand::rngs::SysRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| SealError::Failed)?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| SealError::Failed)?;
    let sealed = cipher
        .encrypt(&Nonce::from(nonce), plain)
        .map_err(|_| SealError::Failed)?;
    Ok(join_gcm(LINUX_MAGIC, &nonce, &sealed))
}

/// What [`seal_rr1`] made with `key`, opened.
pub fn open_rr1(key: &[u8; 32], file: &[u8]) -> Result<Vec<u8>, SealError> {
    let (nonce, sealed) = split_gcm(file, LINUX_MAGIC).ok_or(SealError::Failed)?;
    open_gcm(key, nonce, sealed).ok_or(SealError::Failed)
}

pub(crate) fn open_gcm(key: &[u8; 32], nonce: &[u8; NONCE_BYTES], sealed: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher.decrypt(&Nonce::from(*nonce), sealed).ok()
}

/// DPAPI for the current user, never showing UI.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Dpapi;

#[cfg(windows)]
impl Encryptor for Dpapi {
    fn available(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        dpapi::protect(plain).ok_or(SealError::Failed)
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        dpapi::unprotect(sealed).ok_or(SealError::Failed)
    }
}

#[cfg(windows)]
pub(crate) mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows::core::PCWSTR;

    fn blob(data: &[u8]) -> Option<CRYPT_INTEGER_BLOB> {
        Some(CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).ok()?,
            // DPAPI only reads its input; the type is `*mut` for the output's sake.
            pbData: data.as_ptr().cast_mut(),
        })
    }

    #[allow(
        unsafe_code,
        reason = "copies out and frees the buffer DPAPI allocated; see the SAFETY notes"
    )]
    fn take(out: &CRYPT_INTEGER_BLOB) -> Option<Vec<u8>> {
        if out.pbData.is_null() {
            return None;
        }
        let len = usize::try_from(out.cbData).ok()?;
        // SAFETY: a successful call left `cbData` initialised bytes at `pbData`.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, len) }.to_vec();
        // SAFETY: DPAPI allocated `pbData` with LocalAlloc; it is freed once, here, and not used after.
        let _ = unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
        Some(bytes)
    }

    #[allow(unsafe_code, reason = "CryptProtectData; see the SAFETY note")]
    pub(crate) fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        let input = blob(plain)?;
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: `input` points at `plain`, alive for the call; `out` is a valid blob DPAPI fills.
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .ok()?;
        take(&out)
    }

    #[allow(unsafe_code, reason = "CryptUnprotectData; see the SAFETY note")]
    pub(crate) fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
        let input = blob(sealed)?;
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`; no description is asked for.
        unsafe { CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
        take(&out)
    }
}

/// The Secret Service item that holds the Linux file key.
#[cfg(target_os = "linux")]
pub const FILE_KEY_ATTRIBUTES: [(&str, &str); 2] = [
    ("application", "io.github.freezamotoqueiro.dauntlessreforgedrevival"),
    ("purpose", "account-key-files"),
];
/// The label that item is created with.
#[cfg(target_os = "linux")]
pub const FILE_KEY_LABEL: &str = "Dainty Lass Launcher account keys";

/// AES-256-GCM with a 32-byte key kept in the Secret Service, made on first use.
#[cfg(target_os = "linux")]
#[derive(Default)]
pub struct SecretServiceKey {
    cached: std::sync::Mutex<Option<[u8; 32]>>,
}

#[cfg(target_os = "linux")]
impl SecretServiceKey {
    fn file_key(&self, create: bool) -> Result<[u8; 32], SealError> {
        let mut cached = self.cached.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(key) = *cached {
            return Ok(key);
        }
        let key = match secret_service::lookup(&FILE_KEY_ATTRIBUTES)? {
            Some(secret) => <[u8; 32]>::try_from(secret.as_slice()).map_err(|_| SealError::Failed)?,
            None if create => {
                let mut key = [0u8; 32];
                rand::rngs::SysRng
                    .try_fill_bytes(&mut key)
                    .map_err(|_| SealError::Failed)?;
                secret_service::create(&FILE_KEY_ATTRIBUTES, FILE_KEY_LABEL, &key)?;
                key
            }
            None => return Err(SealError::Failed),
        };
        *cached = Some(key);
        Ok(key)
    }
}

#[cfg(target_os = "linux")]
impl Encryptor for SecretServiceKey {
    fn available(&self) -> bool {
        secret_service::connect().is_ok()
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        seal_rr1(&self.file_key(true)?, plain)
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        open_rr1(&self.file_key(false)?, sealed)
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod secret_service {
    use std::collections::HashMap;

    use dbus_secret_service::{EncryptionType, SecretService};

    use super::SealError;

    pub(crate) fn connect() -> Result<SecretService, SealError> {
        SecretService::connect(EncryptionType::Dh).map_err(|_| SealError::Unavailable)
    }

    /// The secret of the first item with `attributes`, unlocking it first when it is locked.
    pub(crate) fn lookup(attributes: &[(&str, &str)]) -> Result<Option<Vec<u8>>, SealError> {
        let service = connect()?;
        let found = service
            .search_items(attributes.iter().copied().collect::<HashMap<_, _>>())
            .map_err(|_| SealError::Failed)?;
        let Some(item) = found.unlocked.first().or(found.locked.first()) else {
            return Ok(None);
        };
        item.ensure_unlocked().map_err(|_| SealError::Failed)?;
        item.get_secret().map(Some).map_err(|_| SealError::Failed)
    }

    pub(crate) fn create(attributes: &[(&str, &str)], label: &str, secret: &[u8]) -> Result<(), SealError> {
        let service = connect()?;
        let collection = service
            .get_default_collection()
            .or_else(|_| service.get_any_collection())
            .map_err(|_| SealError::Failed)?;
        collection.ensure_unlocked().map_err(|_| SealError::Failed)?;
        collection
            .create_item(
                label,
                attributes.iter().copied().collect(),
                secret,
                true,
                "application/octet-stream",
            )
            .map(|_| ())
            .map_err(|_| SealError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rr1_round_trip_and_tamper() {
        let key = [3u8; 32];
        let file = seal_rr1(&key, b"secret").unwrap();
        assert_eq!(&file[..3], b"rr1");
        assert_eq!(file.len(), 3 + 12 + 6 + 16);
        assert_eq!(open_rr1(&key, &file).unwrap(), b"secret");
        assert_ne!(seal_rr1(&key, b"secret").unwrap(), file, "a fresh nonce each time");
        let mut bad = file.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(open_rr1(&key, &bad), Err(SealError::Failed));
        assert_eq!(open_rr1(&[4u8; 32], &file), Err(SealError::Failed));
        assert_eq!(open_rr1(&key, &file[1..]), Err(SealError::Failed));
    }

    #[test]
    fn unavailable_refuses() {
        assert!(!Unavailable.available());
        assert_eq!(Unavailable.seal(b"x"), Err(SealError::Unavailable));
        assert_eq!(Unavailable.open(b"x"), Err(SealError::Unavailable));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip() {
        let sealed = Dpapi.seal(b"DLK2\n-\nkey").unwrap();
        assert_ne!(sealed, b"DLK2\n-\nkey");
        assert_eq!(Dpapi.open(&sealed).unwrap(), b"DLK2\n-\nkey");
        assert_eq!(Dpapi.open(b"not dpapi"), Err(SealError::Failed));
    }
}
