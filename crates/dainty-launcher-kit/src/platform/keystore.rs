// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The account key store (spec §2.7.1): one slot per server, an OS-sealed `<id>.key2` file in `<userData>/keys`,
//! the old Electron launcher's `<id>.key` read and migrated, and a plain recovery copy in Documents. Every key
//! read or saved is registered as a log secret; log lines name a slot by its id, never its key. The calls block
//! (see [`os`]).

pub mod epic;
pub mod format;
pub mod legacy;
pub mod os;

use std::fmt;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::NaiveDate;
use dainty_protocol::account::is_plausible_account_key;
use dainty_protocol::invite::{ServerMode, is_valid_fingerprint};
use sha2::{Digest as _, Sha256};

use self::format::{Stored, parse_text};
use self::legacy::{ElectronReader, LegacyReader};
use self::os::{Encryptor, SealError, platform_encryptor};
use super::log;
use super::paths::Paths;

/// This launcher's sealed key file.
pub const KEY2_EXTENSION: &str = "key2";
/// The Electron launcher's sealed key file, read only.
pub const LEGACY_EXTENSION: &str = "key";
/// The plain recovery copy.
pub const RECOVERY_EXTENSION: &str = "txt";

/// The store's key for one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Public slots are bound to a certificate.
    pub mode: ServerMode,
    /// The server's host, as the invite has it.
    pub host: String,
    /// The server's port.
    pub port: u16,
}

impl Slot {
    /// The slot for `mode`, `host` and `port`.
    #[must_use]
    pub fn new(mode: ServerMode, host: &str, port: u16) -> Self {
        Self {
            mode,
            host: host.to_owned(),
            port,
        }
    }

    /// See [`slot_id`].
    #[must_use]
    pub fn id(&self) -> String {
        slot_id(self.mode, &self.host, self.port)
    }

    fn is_public(&self) -> bool {
        self.mode == ServerMode::Public
    }
}

/// Lower-case hex of the first 12 bytes of SHA-256 of `public:<host>:<port>` or `<host>:<port>`. Settings key
/// usernames and backup offers by it too (§2.7.2), so it must never change.
#[must_use]
pub fn slot_id(mode: ServerMode, host: &str, port: u16) -> String {
    let name = match mode {
        ServerMode::Public => format!("public:{host}:{port}"),
        ServerMode::Private => format!("{host}:{port}"),
    };
    Sha256::digest(name.as_bytes())[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What certificate a slot's key is stored for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCertificate {
    /// No key is stored.
    Nothing,
    /// A key with no certificate (the first format, or a private slot).
    Bare,
    /// A key bound to this certificate's SHA-256.
    Fingerprint(String),
}

/// Why a store call failed. No variant carries a key.
#[derive(Debug)]
pub enum KeyStoreError {
    /// The OS can't seal or open key files here.
    Unavailable,
    /// The key isn't a plausible account key.
    InvalidKey,
    /// A public slot without a valid certificate fingerprint.
    InvalidFingerprint,
    /// The OS protection failed to seal the key.
    Failed,
    /// The key file could not be written.
    Io(io::Error),
}

impl fmt::Display for KeyStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("key storage is unavailable"),
            Self::InvalidKey => f.write_str("that is not an account key"),
            Self::InvalidFingerprint => f.write_str("a public server needs a valid certificate fingerprint"),
            Self::Failed => f.write_str("the key could not be sealed"),
            Self::Io(e) => write!(f, "the key file could not be written: {e}"),
        }
    }
}

impl std::error::Error for KeyStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SealError> for KeyStoreError {
    fn from(e: SealError) -> Self {
        match e {
            SealError::Unavailable => Self::Unavailable,
            SealError::Failed => Self::Failed,
        }
    }
}

/// The account key store.
pub struct KeyStore {
    keys: PathBuf,
    recovery: PathBuf,
    encryptor: Arc<dyn Encryptor>,
    legacy: Option<Arc<dyn LegacyReader>>,
}

impl fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyStore")
            .field("keys", &self.keys)
            .field("recovery", &self.recovery)
            .field("legacy", &self.legacy.is_some())
            .finish_non_exhaustive()
    }
}

impl KeyStore {
    /// A store over `keys` and `recovery` with the given protection; `legacy` reads the Electron files.
    #[must_use]
    pub fn new(
        keys: PathBuf,
        recovery: PathBuf,
        encryptor: Arc<dyn Encryptor>,
        legacy: Option<Arc<dyn LegacyReader>>,
    ) -> Self {
        Self {
            keys,
            recovery,
            encryptor,
            legacy,
        }
    }

    /// The store this PC's launcher uses: `<userData>/keys`, the Documents recovery folder and the OS's encryptor.
    #[must_use]
    pub fn for_paths(paths: &Paths) -> Self {
        let legacy: Option<Arc<dyn LegacyReader>> =
            ElectronReader::supported().then(|| Arc::new(ElectronReader::new(&paths.user_data)) as _);
        Self::new(paths.keys(), paths.recovery(), platform_encryptor(), legacy)
    }

    /// Whether keys can be saved here.
    #[must_use]
    pub fn available(&self) -> bool {
        self.encryptor.available()
    }

    fn file(&self, slot: &Slot, extension: &str) -> PathBuf {
        self.keys.join(format!("{}.{extension}", slot.id()))
    }

    fn recovery_file(&self, slot: &Slot) -> PathBuf {
        self.recovery.join(format!("{}.{RECOVERY_EXTENSION}", slot.id()))
    }

    /// What the slot holds, from the `.key2` file, else the Electron `.key` file (migrated), else the recovery
    /// copy. An undecryptable `.key2` file is no key.
    pub fn read(&self, slot: &Slot) -> Result<Option<Stored>, KeyStoreError> {
        let key2 = self.file(slot, KEY2_EXTENSION);
        if let Ok(sealed) = std::fs::read(&key2) {
            if !self.encryptor.available() {
                return Err(KeyStoreError::Unavailable);
            }
            return Ok(self.encryptor.open(&sealed).ok().and_then(|plain| parse_text(&plain)));
        }
        if let Some(stored) = self.read_legacy(slot) {
            self.migrate(slot, &stored);
            return Ok(Some(stored));
        }
        Ok(std::fs::read(self.recovery_file(slot))
            .ok()
            .and_then(|plain| parse_text(&plain)))
    }

    fn read_legacy(&self, slot: &Slot) -> Option<Stored> {
        let reader = self.legacy.as_ref()?;
        let sealed = std::fs::read(self.file(slot, LEGACY_EXTENSION)).ok()?;
        parse_text(&reader.open(&sealed)?)
    }

    // The old file stays either way, so a failed migration loses nothing.
    fn migrate(&self, slot: &Slot, stored: &Stored) {
        let id = slot.id();
        let key2 = self.file(slot, KEY2_EXTENSION);
        if let Err(e) = self.write_key2(&key2, stored) {
            log::warn(format!("key store: could not migrate the old key file of {id}: {e}"));
            return;
        }
        let back = std::fs::read(&key2)
            .ok()
            .and_then(|sealed| self.encryptor.open(&sealed).ok())
            .and_then(|plain| parse_text(&plain));
        if back.as_ref() == Some(stored) {
            log::info(format!("key store: migrated the old key file of {id}"));
        } else {
            let _ = std::fs::remove_file(&key2);
            log::warn(format!(
                "key store: the migrated key of {id} did not read back; kept the old file"
            ));
        }
    }

    fn write_key2(&self, path: &Path, stored: &Stored) -> Result<(), KeyStoreError> {
        let sealed = self.encryptor.seal(stored.to_text().as_bytes())?;
        write_private(path, &sealed).map_err(KeyStoreError::Io)
    }

    /// Saves `key` for `slot`, bound to `fingerprint` when the slot is public (a private slot stores none).
    pub fn save(&self, slot: &Slot, key: &str, fingerprint: Option<&str>) -> Result<(), KeyStoreError> {
        if !is_plausible_account_key(key) {
            return Err(KeyStoreError::InvalidKey);
        }
        let fingerprint = if slot.is_public() {
            match fingerprint {
                Some(fp) if is_valid_fingerprint(fp) => Some(fp.to_owned()),
                _ => return Err(KeyStoreError::InvalidFingerprint),
            }
        } else {
            None
        };
        if !self.encryptor.available() {
            return Err(KeyStoreError::Unavailable);
        }
        log::register_secret(key);
        let stored = Stored {
            fingerprint,
            key: key.to_owned(),
        };
        self.write_key2(&self.file(slot, KEY2_EXTENSION), &stored)?;
        if let Err(e) = write_private(&self.recovery_file(slot), stored.to_text().as_bytes()) {
            log::warn(format!(
                "key store: could not write the recovery copy of {}: {e}",
                slot.id()
            ));
        }
        Ok(())
    }

    /// The slot's key; a public slot answers only for the exact `fingerprint` it was saved with.
    pub fn load(&self, slot: &Slot, fingerprint: Option<&str>) -> Result<Option<String>, KeyStoreError> {
        let Some(stored) = self.read(slot)? else {
            return Ok(None);
        };
        if slot.is_public() && (fingerprint.is_none() || stored.fingerprint.as_deref() != fingerprint) {
            return Ok(None);
        }
        log::register_secret(&stored.key);
        if !self.recovery_file(slot).exists()
            && let Err(e) = self.save(slot, &stored.key, stored.fingerprint.as_deref())
        {
            log::warn(format!(
                "key store: could not remake the recovery copy of {}: {e}",
                slot.id()
            ));
        }
        Ok(Some(stored.key))
    }

    /// The certificate the slot's key is stored for.
    pub fn certificate(&self, slot: &Slot) -> Result<StoredCertificate, KeyStoreError> {
        Ok(match self.read(slot)? {
            None => StoredCertificate::Nothing,
            Some(Stored { fingerprint: None, .. }) => StoredCertificate::Bare,
            Some(Stored {
                fingerprint: Some(fp), ..
            }) => StoredCertificate::Fingerprint(fp),
        })
    }

    /// Binds a public slot's key to a new certificate; `false` when nothing is stored.
    pub fn rebind(&self, slot: &Slot, fingerprint: &str) -> Result<bool, KeyStoreError> {
        if !slot.is_public() || !is_valid_fingerprint(fingerprint) {
            return Err(KeyStoreError::InvalidFingerprint);
        }
        let Some(stored) = self.read(slot)? else {
            return Ok(false);
        };
        self.save(slot, &stored.key, Some(fingerprint))?;
        Ok(true)
    }

    /// Forgets the slot's key: unregisters it and deletes every copy. The first deletion that failed for another
    /// reason than the file being gone is the error, after all three were tried.
    pub fn remove(&self, slot: &Slot) -> Result<(), KeyStoreError> {
        if let Ok(Some(stored)) = self.read(slot) {
            log::unregister_secret(&stored.key);
        }
        let mut first = None;
        for path in [
            self.file(slot, KEY2_EXTENSION),
            self.file(slot, LEGACY_EXTENSION),
            self.recovery_file(slot),
        ] {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    first.get_or_insert(e);
                }
                _ => {}
            }
        }
        first.map_or(Ok(()), |e| Err(KeyStoreError::Io(e)))
    }
}

/// Writes `bytes` to `<path>.tmp`, owner-only on Unix, then renames it over `path`.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let written = (|| {
        let mut file = options.open(&tmp)?;
        // A leftover `.tmp` keeps its old mode through `open`.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// What the key backup file says (`save_key_backup`, spec §2.4.6).
#[derive(Clone)]
pub struct Backup<'a> {
    /// The server's name.
    pub server_name: &'a str,
    /// The server's host.
    pub host: &'a str,
    /// The server's port.
    pub port: u16,
    /// The certificate's SHA-256, for a public server only.
    pub fingerprint: Option<&'a str>,
    /// The account's username, when known.
    pub username: Option<&'a str>,
    /// The day the file was saved.
    pub saved: NaiveDate,
    /// The account key.
    pub key: &'a str,
}

// The key never reaches a `{:?}`.
impl fmt::Debug for Backup<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backup")
            .field("server_name", &self.server_name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("fingerprint", &self.fingerprint)
            .field("username", &self.username)
            .field("saved", &self.saved)
            .finish_non_exhaustive()
    }
}

/// The backup file's first line.
pub const BACKUP_TITLE: &str = "Dainty Lass account key";
/// The backup file's advice on keeping it private (L3: written for this launcher; nothing reads it back).
pub const BACKUP_PRIVACY: &str = "Treat this file like a password: whoever has the key above can play on this server \
                                  under your name, so never share it or post it anywhere.";
/// The backup file's advice on using it on another PC (L3).
pub const BACKUP_OTHER_PC: &str = "To play from another PC, join this server in the launcher there, choose Register, \
                                   then I already have a key, then Load a backup file, and pick this file.";

/// The backup file's text: CR LF lines ending in an empty one. Only the `Key:` line is read back
/// (`dainty_protocol::account::extract_account_key`).
#[must_use]
pub fn backup_text(backup: &Backup<'_>) -> String {
    let mut lines = vec![
        BACKUP_TITLE.to_owned(),
        String::new(),
        format!("Server: {} ({}:{})", backup.server_name, backup.host, backup.port),
    ];
    if let Some(fp) = backup.fingerprint {
        lines.push(format!("Certificate: {fp}"));
    }
    lines.extend([
        format!("Username: {}", backup.username.unwrap_or("-")),
        format!("Saved: {}", backup.saved.format("%Y-%m-%d")),
        format!("Key: {}", backup.key),
        String::new(),
        BACKUP_PRIVACY.to_owned(),
        BACKUP_OTHER_PC.to_owned(),
        String::new(),
    ]);
    lines.join("\r\n")
}

/// Whether text holding a key may be for the server at `host:port` with `pin`: a backup's `Certificate:` line, else
/// its `Server:` line, must name it (the key goes only to the server it was made for). Text with neither, a key
/// on its own, can't say, so it may be.
#[must_use]
pub fn backup_is_for(text: &str, host: &str, port: u16, pin: Option<&str>) -> bool {
    let line = |label: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(label).map(str::trim))
    };
    if let Some(certificate) = line("Certificate:") {
        return pin.is_some_and(|pin| pin.eq_ignore_ascii_case(certificate));
    }
    if let Some(server) = line("Server:") {
        let named = server
            .rsplit_once('(')
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .and_then(|address| address.rsplit_once(':'));
        return named.is_some_and(|(named_host, named_port)| {
            named_host.eq_ignore_ascii_case(host) && named_port == port.to_string()
        });
    }
    true
}

#[cfg(test)]
mod tests;
