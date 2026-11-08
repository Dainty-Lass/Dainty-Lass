// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The key store's logic (spec §2.7.1) over a stand-in encryptor and Electron reader. A process of its own, since
//! it logs and registers secrets.

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use dainty_launcher_kit::platform::keystore::format::{Stored, parse_text};
use dainty_launcher_kit::platform::keystore::legacy::LegacyReader;
use dainty_launcher_kit::platform::keystore::os::{Encryptor, SealError, open_rr1, seal_rr1};
use dainty_launcher_kit::platform::keystore::{KeyStore, KeyStoreError, Slot, StoredCertificate};
use dainty_launcher_kit::platform::log;
use dainty_protocol::invite::ServerMode;

const FP: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const FP2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// Real AES-GCM under a fixed key; can be switched off, or made to corrupt what it seals.
struct Fake {
    key: [u8; 32],
    on: AtomicBool,
    corrupt_seals: AtomicBool,
    seals: AtomicUsize,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            key: [7; 32],
            on: AtomicBool::new(true),
            corrupt_seals: AtomicBool::new(false),
            seals: AtomicUsize::new(0),
        })
    }
}

impl Encryptor for Fake {
    fn available(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        if !self.available() {
            return Err(SealError::Unavailable);
        }
        self.seals.fetch_add(1, Ordering::SeqCst);
        let mut sealed = seal_rr1(&self.key, plain)?;
        if self.corrupt_seals.load(Ordering::SeqCst) {
            sealed[5] ^= 1;
        }
        Ok(sealed)
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        if !self.available() {
            return Err(SealError::Unavailable);
        }
        open_rr1(&self.key, sealed)
    }
}

/// An Electron "file" is `old:` + its plain text.
struct OldReader;

impl LegacyReader for OldReader {
    fn open(&self, file: &[u8]) -> Option<Vec<u8>> {
        file.strip_prefix(b"old:").map(<[u8]>::to_vec)
    }
}

struct Setup {
    _tmp: tempfile::TempDir,
    keys: PathBuf,
    recovery: PathBuf,
    fake: Arc<Fake>,
    store: KeyStore,
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let keys = tmp.path().join("keys");
    let recovery = tmp.path().join("Documents").join("DaintyLass").join("Account Keys");
    let fake = Fake::new();
    let store = KeyStore::new(keys.clone(), recovery.clone(), fake.clone(), Some(Arc::new(OldReader)));
    Setup {
        _tmp: tmp,
        keys,
        recovery,
        fake,
        store,
    }
}

fn public() -> Slot {
    Slot::new(ServerMode::Public, "203.0.113.10", 443)
}

fn private() -> Slot {
    Slot::new(ServerMode::Private, "100.64.0.1", 61000)
}

// Unique per test: the log's secrets are process-wide. No `RRK_` prefix, so only registration hides them.
fn key(name: &str) -> String {
    format!("plainkey_{name}_0123456789")
}

static ALL_KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn remember(key: &str) -> String {
    ALL_KEYS.lock().unwrap().push(key.to_owned());
    key.to_owned()
}

#[test]
fn save_then_load_writes_both_copies() {
    let s = setup();
    let k = remember(&key("save"));
    s.store.save(&public(), &k, Some(FP)).unwrap();
    let id = public().id();
    let key2 = std::fs::read(s.keys.join(format!("{id}.key2"))).unwrap();
    assert_eq!(&key2[..3], b"rr1");
    assert!(!String::from_utf8_lossy(&key2).contains(&k), "the .key2 file is sealed");
    let plain = std::fs::read_to_string(s.recovery.join(format!("{id}.txt"))).unwrap();
    assert_eq!(plain, format!("RRK2\n{FP}\n{k}"));
    assert!(!s.keys.join(format!("{id}.key2.tmp")).exists());

    assert_eq!(s.store.load(&public(), Some(FP)).unwrap(), Some(k.clone()));
    assert_eq!(s.store.load(&public(), Some(FP2)).unwrap(), None);
    assert_eq!(s.store.load(&public(), None).unwrap(), None);
    assert_eq!(
        s.store.certificate(&public()).unwrap(),
        StoredCertificate::Fingerprint(FP.into())
    );
    assert_eq!(log::redact(&k), log::HIDDEN);
}

#[test]
fn private_slots_store_no_certificate() {
    let s = setup();
    let k = remember(&key("private"));
    s.store.save(&private(), &k, Some(FP)).unwrap();
    assert_eq!(s.store.certificate(&private()).unwrap(), StoredCertificate::Bare);
    assert_eq!(s.store.load(&private(), None).unwrap(), Some(k.clone()));
    assert_eq!(s.store.load(&private(), Some(FP2)).unwrap(), Some(k));
}

#[test]
fn save_refusals() {
    let s = setup();
    assert!(matches!(
        s.store.save(&public(), "short", Some(FP)),
        Err(KeyStoreError::InvalidKey)
    ));
    assert!(matches!(
        s.store.save(&public(), "has space in it", Some(FP)),
        Err(KeyStoreError::InvalidKey)
    ));
    let k = remember(&key("refusals"));
    assert!(matches!(
        s.store.save(&public(), &k, None),
        Err(KeyStoreError::InvalidFingerprint)
    ));
    assert!(matches!(
        s.store.save(&public(), &k, Some("abc")),
        Err(KeyStoreError::InvalidFingerprint)
    ));
    s.fake.on.store(false, Ordering::SeqCst);
    assert!(!s.store.available());
    assert!(matches!(
        s.store.save(&public(), &k, Some(FP)),
        Err(KeyStoreError::Unavailable)
    ));
    assert!(!s.keys.exists());
}

#[test]
fn unavailable_encryptor_with_a_key2_file_is_an_error() {
    let s = setup();
    let k = remember(&key("unavailable"));
    s.store.save(&private(), &k, None).unwrap();
    s.fake.on.store(false, Ordering::SeqCst);
    assert!(matches!(
        s.store.load(&private(), None),
        Err(KeyStoreError::Unavailable)
    ));
    assert!(matches!(
        s.store.certificate(&private()),
        Err(KeyStoreError::Unavailable)
    ));
}

#[test]
fn undecryptable_key2_is_no_key_even_with_a_recovery_copy() {
    let s = setup();
    let k = remember(&key("undecryptable"));
    s.store.save(&private(), &k, None).unwrap();
    let path = s.keys.join(format!("{}.key2", private().id()));
    let mut sealed = std::fs::read(&path).unwrap();
    *sealed.last_mut().unwrap() ^= 1;
    std::fs::write(&path, sealed).unwrap();
    assert_eq!(s.store.load(&private(), None).unwrap(), None);
    assert_eq!(s.store.certificate(&private()).unwrap(), StoredCertificate::Nothing);
}

#[test]
fn foreign_text_in_a_key2_file_is_no_key() {
    let s = setup();
    let k = remember(&key("foreign"));
    std::fs::create_dir_all(&s.keys).unwrap();
    let sealed = s.fake.seal(format!("DRK2\n-\n{k}").as_bytes()).unwrap();
    std::fs::write(s.keys.join(format!("{}.key2", private().id())), sealed).unwrap();
    assert_eq!(s.store.load(&private(), None).unwrap(), None);
}

#[test]
fn bare_key_of_the_first_format() {
    let s = setup();
    let k = remember(&key("bare"));
    std::fs::create_dir_all(&s.keys).unwrap();
    std::fs::write(
        s.keys.join(format!("{}.key2", public().id())),
        s.fake.seal(k.as_bytes()).unwrap(),
    )
    .unwrap();
    assert_eq!(s.store.certificate(&public()).unwrap(), StoredCertificate::Bare);
    // A public slot answers only for its exact certificate, and a bare key has none.
    assert_eq!(s.store.load(&public(), Some(FP)).unwrap(), None);
    assert!(s.store.rebind(&public(), FP).unwrap());
    assert_eq!(s.store.load(&public(), Some(FP)).unwrap(), Some(k));
}

#[test]
fn electron_file_is_migrated_and_kept() {
    let s = setup();
    let k = remember(&key("electron"));
    let id = public().id();
    std::fs::create_dir_all(&s.keys).unwrap();
    std::fs::write(s.keys.join(format!("{id}.key")), format!("old:RRK2\n{FP}\n{k}")).unwrap();
    assert_eq!(s.store.load(&public(), Some(FP)).unwrap(), Some(k.clone()));
    assert!(s.keys.join(format!("{id}.key")).exists(), "the old file stays");
    let opened = s
        .fake
        .open(&std::fs::read(s.keys.join(format!("{id}.key2"))).unwrap())
        .unwrap();
    assert_eq!(
        parse_text(&opened),
        Some(Stored {
            fingerprint: Some(FP.into()),
            key: k
        })
    );
}

#[test]
fn failed_migration_removes_the_new_file() {
    let s = setup();
    let k = remember(&key("migration"));
    let id = private().id();
    std::fs::create_dir_all(&s.keys).unwrap();
    std::fs::write(s.keys.join(format!("{id}.key")), format!("old:{k}")).unwrap();
    s.fake.corrupt_seals.store(true, Ordering::SeqCst);
    assert_eq!(
        s.store.read(&private()).unwrap().map(|stored| stored.key),
        Some(k.clone())
    );
    assert!(!s.keys.join(format!("{id}.key2")).exists());
    assert!(s.keys.join(format!("{id}.key")).exists());
}

#[test]
fn unreadable_electron_file_falls_back_to_the_recovery_copy() {
    let s = setup();
    let k = remember(&key("fallback"));
    let id = private().id();
    std::fs::create_dir_all(&s.keys).unwrap();
    std::fs::write(s.keys.join(format!("{id}.key")), b"garbage").unwrap();
    std::fs::create_dir_all(&s.recovery).unwrap();
    std::fs::write(s.recovery.join(format!("{id}.txt")), format!("RRK2\n-\n{k}")).unwrap();
    assert_eq!(s.store.load(&private(), None).unwrap(), Some(k));
    assert!(
        !s.keys.join(format!("{id}.key2")).exists(),
        "a recovery read migrates nothing"
    );
}

#[test]
fn store_without_electron_reader_ignores_key_files() {
    let s = setup();
    let k = remember(&key("noreader"));
    let store = KeyStore::new(s.keys.clone(), s.recovery.clone(), s.fake.clone(), None);
    std::fs::create_dir_all(&s.keys).unwrap();
    std::fs::write(s.keys.join(format!("{}.key", private().id())), format!("old:{k}")).unwrap();
    assert_eq!(store.load(&private(), None).unwrap(), None);
}

#[test]
fn load_remakes_a_missing_recovery_copy() {
    let s = setup();
    let k = remember(&key("remake"));
    s.store.save(&public(), &k, Some(FP)).unwrap();
    let copy = s.recovery.join(format!("{}.txt", public().id()));
    std::fs::remove_file(&copy).unwrap();
    assert_eq!(s.store.load(&public(), Some(FP)).unwrap(), Some(k.clone()));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), format!("RRK2\n{FP}\n{k}"));
    let seals = s.fake.seals.load(Ordering::SeqCst);
    s.store.load(&public(), Some(FP)).unwrap();
    assert_eq!(
        s.fake.seals.load(Ordering::SeqCst),
        seals,
        "a present copy is left alone"
    );
}

#[test]
fn recovery_copy_failure_only_warns() {
    let s = setup();
    let k = remember(&key("recoveryfail"));
    // A file where the recovery folder should be.
    std::fs::create_dir_all(s.recovery.parent().unwrap()).unwrap();
    std::fs::write(&s.recovery, b"in the way").unwrap();
    s.store.save(&private(), &k, None).unwrap();
    assert_eq!(s.store.load(&private(), None).unwrap(), Some(k));
}

#[test]
fn rebind() {
    let s = setup();
    let k = remember(&key("rebind"));
    assert!(matches!(
        s.store.rebind(&private(), FP),
        Err(KeyStoreError::InvalidFingerprint)
    ));
    assert!(matches!(
        s.store.rebind(&public(), "nope"),
        Err(KeyStoreError::InvalidFingerprint)
    ));
    assert!(!s.store.rebind(&public(), FP).unwrap());
    s.store.save(&public(), &k, Some(FP)).unwrap();
    assert!(s.store.rebind(&public(), FP2).unwrap());
    assert_eq!(s.store.load(&public(), Some(FP)).unwrap(), None);
    assert_eq!(s.store.load(&public(), Some(FP2)).unwrap(), Some(k));
    assert_eq!(
        s.store.certificate(&public()).unwrap(),
        StoredCertificate::Fingerprint(FP2.into())
    );
}

#[test]
fn remove_deletes_every_copy_and_unregisters() {
    let s = setup();
    let k = remember(&key("remove"));
    let id = public().id();
    s.store.save(&public(), &k, Some(FP)).unwrap();
    std::fs::write(s.keys.join(format!("{id}.key")), format!("old:{k}")).unwrap();
    assert_eq!(log::redact(&k), log::HIDDEN);
    s.store.remove(&public()).unwrap();
    for path in [
        s.keys.join(format!("{id}.key2")),
        s.keys.join(format!("{id}.key")),
        s.recovery.join(format!("{id}.txt")),
    ] {
        assert!(!path.exists(), "{}", path.display());
    }
    assert_eq!(log::redact(&k), k);
    assert_eq!(s.store.certificate(&public()).unwrap(), StoredCertificate::Nothing);
    s.store.remove(&public()).unwrap();
}

// The only test that sets the log folder; the others' lines land there too, so their keys are checked as well.
#[test]
fn no_key_reaches_the_log() {
    let s = setup();
    let logs = tempfile::tempdir().unwrap();
    log::set_folder(logs.path());
    let k = remember(&key("logged"));
    let id = public().id();
    std::fs::create_dir_all(&s.keys).unwrap();
    std::fs::write(s.keys.join(format!("{id}.key")), format!("old:RRK2\n{FP}\n{k}")).unwrap();
    s.fake.corrupt_seals.store(true, Ordering::SeqCst);
    assert_eq!(s.store.load(&public(), Some(FP)).unwrap(), Some(k.clone()));
    s.fake.corrupt_seals.store(false, Ordering::SeqCst);
    s.store.load(&public(), Some(FP)).unwrap();
    s.store.save(&public(), &k, Some(FP)).unwrap();
    s.store.load(&public(), Some(FP)).unwrap();
    log::info(format!("the key was {k}"));

    let text = std::fs::read_to_string(logs.path().join(log::LOG_FILE)).unwrap();
    assert!(text.contains(&id), "the store's lines name the slot: {text}");
    assert!(text.contains("the key was <hidden>"));
    for key in ALL_KEYS.lock().unwrap().iter() {
        assert!(!text.contains(key.as_str()), "{key} reached the log");
    }
}
