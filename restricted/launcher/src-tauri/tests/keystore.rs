//! dainty-launcher-kit's key store (spec §2.7.1, row L27), as the shell's `platform/keystore` tests had it: Reforged's
//! `keystore.test.ts` cases, the key files' text, the Electron files' framing and decryption, the OS protection,
//! the backup file, and an Electron profile from the harness in `test/electron-keystore`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    reason = "a panic is how a test fails"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use dainty_launcher_kit::platform::keystore::format::{self, ELECTRON_V10, Stored, parse_text};
use dainty_launcher_kit::platform::keystore::legacy::{self, LegacyReader};
use dainty_launcher_kit::platform::keystore::os::{Encryptor, SealError, open_rr1, seal_rr1};
use dainty_launcher_kit::platform::keystore::{
    BACKUP_OTHER_PC, BACKUP_PRIVACY, Backup, KeyStore, KeyStoreError, Slot, StoredCertificate, backup_text, slot_id,
};
use dainty_launcher_kit::platform::log;
use dainty_protocol::invite::ServerMode;

/// "enc:" + base64, as the TypeScript tests' encryptor.
struct Fake;

impl Fake {
    fn encrypt(&self, plain: &str) -> Vec<u8> {
        self.seal(plain.as_bytes()).unwrap()
    }

    fn decrypt(&self, data: &[u8]) -> String {
        String::from_utf8(self.open(data).unwrap()).unwrap()
    }
}

impl Encryptor for Fake {
    fn available(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        Ok(format!("enc:{}", base64::engine::general_purpose::STANDARD.encode(plain)).into_bytes())
    }

    fn open(&self, data: &[u8]) -> Result<Vec<u8>, SealError> {
        let text = std::str::from_utf8(data)
            .ok()
            .and_then(|text| text.strip_prefix("enc:"))
            .ok_or(SealError::Failed)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| SealError::Failed)
    }
}

/// The Electron launcher's safeStorage, as another encryptor.
struct Old;

impl Old {
    fn encrypt(&self, plain: &str) -> Vec<u8> {
        format!("old:{plain}").into_bytes()
    }
}

impl LegacyReader for Old {
    fn open(&self, data: &[u8]) -> Option<Vec<u8>> {
        Some(data.strip_prefix(b"old:")?.to_vec())
    }
}

const KEY: &str = concat!("DLK_", "abababababababababababab", "abababababababababababab");

/// A slot with the certificate its key is asked for (the shell's `KeySlot`).
struct KeySlot {
    slot: Slot,
    fp: Option<String>,
}

impl KeySlot {
    fn id(&self) -> String {
        self.slot.id()
    }
}

/// The shell's store with its file names; the recovery folder is a separate one in the same temporary folder.
struct Keys {
    store: KeyStore,
    dir: PathBuf,
}

impl Keys {
    fn new(dir: &Path, encryptor: Arc<dyn Encryptor>, legacy: Option<Arc<dyn LegacyReader>>) -> Self {
        Self::with_recovery(dir, encryptor, legacy, dir.join("recovery"))
    }

    fn with_recovery(
        dir: &Path,
        encryptor: Arc<dyn Encryptor>,
        legacy: Option<Arc<dyn LegacyReader>>,
        recovery: PathBuf,
    ) -> Self {
        let keys = dir.join("keys");
        std::fs::create_dir_all(&keys).unwrap();
        Self {
            store: KeyStore::new(keys.clone(), recovery, encryptor, legacy),
            dir: keys,
        }
    }

    fn save(&self, slot: &KeySlot, key: &str) -> Result<(), KeyStoreError> {
        self.store.save(&slot.slot, key, slot.fp.as_deref())
    }

    fn load(&self, slot: &KeySlot) -> Result<Option<String>, KeyStoreError> {
        self.store.load(&slot.slot, slot.fp.as_deref())
    }

    fn has(&self, slot: &KeySlot) -> bool {
        self.load(slot).unwrap().is_some()
    }

    /// The shell's `certificate_of`: none without a key, else the certificate it is bound to.
    fn certificate_of(&self, slot: &KeySlot) -> Option<Option<String>> {
        match self.store.certificate(&slot.slot).unwrap() {
            StoredCertificate::Nothing => None,
            StoredCertificate::Bare => Some(None),
            StoredCertificate::Fingerprint(fp) => Some(Some(fp)),
        }
    }

    fn rebind(&self, slot: &KeySlot, fp: &str) -> Result<bool, KeyStoreError> {
        self.store.rebind(&slot.slot, fp)
    }

    fn remove(&self, slot: &KeySlot) {
        self.store.remove(&slot.slot).unwrap();
    }

    fn file(&self, slot: &KeySlot) -> PathBuf {
        self.dir.join(format!("{}.key2", slot.id()))
    }

    fn legacy_file(&self, slot: &KeySlot) -> PathBuf {
        self.dir.join(format!("{}.key", slot.id()))
    }
}

fn store(dir: &Path) -> Keys {
    Keys::new(dir, Arc::new(Fake), Some(Arc::new(Old)))
}

fn public(fp: Option<&str>) -> KeySlot {
    KeySlot {
        slot: Slot::new(ServerMode::Public, "203.0.113.10", 443),
        fp: fp.map(str::to_owned),
    }
}

fn private(host: &str) -> KeySlot {
    KeySlot {
        slot: Slot::new(ServerMode::Private, host, 61000),
        fp: None,
    }
}

fn fp(pair: &str) -> String {
    pair.repeat(32)
}

#[test]
fn server_ids_are_the_typescript_ones() {
    // createHash("sha256").update(id).digest("hex").slice(0, 24), worked out by Node
    assert_eq!(
        slot_id(ServerMode::Private, "100.64.0.1", 61000),
        "2f1974fb8b73463e94a88aed"
    );
    assert_eq!(
        slot_id(ServerMode::Public, "203.0.113.10", 443),
        "4513d283ab25060967c23596"
    );
}

// keystore.test.ts: "a public key loads only for the certificate it was saved with"
#[test]
fn a_public_key_loads_only_for_the_certificate_it_was_saved_with() {
    let dir = tempfile::tempdir().unwrap();
    let keys = store(dir.path());
    let (a, b) = (fp("a1"), fp("b2"));
    keys.save(&public(Some(&a)), KEY).unwrap();
    assert_eq!(keys.load(&public(Some(&a))).unwrap().as_deref(), Some(KEY));
    assert!(keys.has(&public(Some(&a))));
    assert_eq!(
        keys.load(&public(Some(&b))).unwrap(),
        None,
        "another certificate never gets the key"
    );
    assert!(!keys.has(&public(Some(&b))));
    assert_eq!(keys.load(&public(None)).unwrap(), None);
    assert_eq!(keys.certificate_of(&public(Some(&b))), Some(Some(a)));
}

// "rebind moves the key to a new certificate"
#[test]
fn rebind_moves_the_key_to_a_new_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let keys = store(dir.path());
    let (a, b) = (fp("a1"), fp("b2"));
    keys.save(&public(Some(&a)), KEY).unwrap();
    assert!(keys.rebind(&public(Some(&a)), &b).unwrap());
    assert_eq!(keys.load(&public(Some(&b))).unwrap().as_deref(), Some(KEY));
    assert_eq!(keys.load(&public(Some(&a))).unwrap(), None);
    assert!(
        !store(tempfile::tempdir().unwrap().path())
            .rebind(&public(Some(&a)), &b)
            .unwrap(),
        "nothing to move"
    );
    assert!(matches!(
        keys.rebind(&public(Some(&b)), "not-a-fingerprint"),
        Err(KeyStoreError::InvalidFingerprint)
    ));
    assert!(matches!(
        keys.rebind(&private("100.64.0.1"), &b),
        Err(KeyStoreError::InvalidFingerprint)
    ));
}

// "a public key cannot be saved without a fingerprint"
#[test]
fn a_public_key_cannot_be_saved_without_a_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let keys = store(dir.path());
    assert!(matches!(
        keys.save(&public(None), KEY),
        Err(KeyStoreError::InvalidFingerprint)
    ));
    assert!(matches!(
        keys.save(&public(Some("ABC")), KEY),
        Err(KeyStoreError::InvalidFingerprint)
    ));
    assert!(matches!(
        keys.save(&public(Some(&fp("a1"))), "not a key"),
        Err(KeyStoreError::InvalidKey)
    ));
}

// "private keys are not tied to a certificate; old key files still load for private servers only"
#[test]
fn private_keys_and_the_first_file_format() {
    let dir = tempfile::tempdir().unwrap();
    let keys = store(dir.path());
    keys.save(&private("100.64.0.1"), KEY).unwrap();
    assert_eq!(keys.load(&private("100.64.0.1")).unwrap().as_deref(), Some(KEY));
    // The first format, the key alone, in a new-format file
    std::fs::write(keys.file(&private("100.64.0.2")), Fake.encrypt(KEY)).unwrap();
    assert_eq!(keys.load(&private("100.64.0.2")).unwrap().as_deref(), Some(KEY));
    std::fs::write(keys.file(&public(None)), Fake.encrypt(KEY)).unwrap();
    assert_eq!(
        keys.load(&public(Some(&fp("a1")))).unwrap(),
        None,
        "an untied public key goes nowhere"
    );
    assert_eq!(keys.certificate_of(&public(Some(&fp("a1")))), Some(None));
}

// "remove deletes the key whatever certificate it belongs to"
#[test]
fn remove_deletes_the_key_whatever_certificate_it_belongs_to() {
    let dir = tempfile::tempdir().unwrap();
    let keys = store(dir.path());
    keys.save(&public(Some(&fp("a1"))), KEY).unwrap();
    std::fs::write(
        keys.legacy_file(&public(None)),
        Old.encrypt(&format!("DLK2\n{}\n{KEY}", fp("a1"))),
    )
    .unwrap();
    keys.remove(&public(Some(&fp("b2"))));
    assert_eq!(keys.certificate_of(&public(Some(&fp("a1")))), None);
    assert!(!keys.legacy_file(&public(None)).exists(), "the Electron file too");
}

// "Documents recovery survives lost local key storage and keeps certificate binding"
#[test]
fn the_documents_copy_survives_lost_key_storage_and_keeps_its_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let (primary, recovery) = (dir.path().join("local"), dir.path().join("Documents"));
    let a = fp("a1");
    let keys = Keys::with_recovery(&primary, Arc::new(Fake), None, recovery.clone());
    keys.save(&public(Some(&a)), KEY).unwrap();
    assert_eq!(
        std::fs::read_to_string(recovery.join(format!("{}.txt", public(None).id()))).unwrap(),
        format!("DLK2\n{a}\n{KEY}"),
        "the Electron launcher's Documents file, byte for byte"
    );
    std::fs::remove_dir_all(&primary).unwrap();
    let restored = Keys::with_recovery(&primary, Arc::new(Fake), None, recovery);
    assert_eq!(restored.load(&public(Some(&fp("b2")))).unwrap(), None);
    assert_eq!(restored.load(&public(Some(&a))).unwrap().as_deref(), Some(KEY));
    restored.remove(&public(Some(&a)));
    assert_eq!(restored.load(&public(Some(&a))).unwrap(), None);
}

#[test]
fn an_electron_key_is_moved_once_and_its_file_kept() {
    let dir = tempfile::tempdir().unwrap();
    let a = fp("a1");
    let slot = public(Some(&a));
    let keys = store(dir.path());
    let old = keys.legacy_file(&slot);
    std::fs::write(&old, Old.encrypt(&format!("DLK2\n{a}\n{KEY}"))).unwrap();
    assert_eq!(keys.load(&slot).unwrap().as_deref(), Some(KEY));
    assert_eq!(
        Fake.decrypt(&std::fs::read(keys.file(&slot)).unwrap()),
        format!("DLK2\n{a}\n{KEY}")
    );
    assert!(old.exists(), "the Electron file stays for a rollback");
    // Without the old encryptor the new file still loads
    let after = Keys::new(dir.path(), Arc::new(Fake), None);
    assert_eq!(after.load(&slot).unwrap().as_deref(), Some(KEY));
    // A bare Electron key (the first format) is moved for a private server
    let bare = private("100.64.0.9");
    std::fs::write(keys.legacy_file(&bare), Old.encrypt(KEY)).unwrap();
    assert_eq!(keys.load(&bare).unwrap().as_deref(), Some(KEY));
    assert_eq!(
        Fake.decrypt(&std::fs::read(keys.file(&bare)).unwrap()),
        format!("DLK2\n-\n{KEY}")
    );
}

#[test]
fn only_rrk2_and_bare_electron_keys_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let keys = store(dir.path());
    let a = fp("a1");
    let slot = public(Some(&a));
    // DR's tag never was on a Reforged PC (RD22)
    std::fs::write(keys.legacy_file(&slot), Old.encrypt(&format!("DRK2\n{a}\n{KEY}"))).unwrap();
    assert_eq!(keys.load(&slot).unwrap(), None);
    assert!(!keys.file(&slot).exists(), "nothing written");
    // A file the old encryptor cannot read is no key, not an error: the import flow takes over
    std::fs::write(keys.legacy_file(&private("100.64.0.3")), b"garbage").unwrap();
    assert_eq!(keys.load(&private("100.64.0.3")).unwrap(), None);
}

#[test]
fn a_read_key_is_hidden_in_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let keys = store(dir.path());
    let key = concat!("DLK_", "0f0f0f0f0f0f0f0f0f0f0f0f", "0f0f0f0f0f0f0f0f0f0f0f0f");
    std::fs::write(keys.legacy_file(&private("100.64.0.4")), Old.encrypt(key)).unwrap();
    assert_eq!(keys.load(&private("100.64.0.4")).unwrap().as_deref(), Some(key));
    let line = log::redact(&format!("argument {} and {}", key, &key[4..]));
    assert!(!line.contains(&key[4..]), "{line}");
}

#[test]
fn an_unavailable_store_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let keys = Keys::new(
        dir.path(),
        Arc::new(dainty_launcher_kit::platform::keystore::os::Unavailable),
        None,
    );
    assert!(matches!(
        keys.save(&private("100.64.0.1"), KEY),
        Err(KeyStoreError::Unavailable)
    ));
    std::fs::write(keys.file(&private("100.64.0.1")), b"x").unwrap();
    assert!(matches!(
        keys.load(&private("100.64.0.1")),
        Err(KeyStoreError::Unavailable)
    ));
}

/// A profile the Electron launcher's own code wrote (`launcher/test/electron-keystore`, with real safeStorage, as
/// the same user on the same PC): every key reads back through dainty-launcher-kit's Electron reader, is moved to this
/// launcher's files once, and the 1.4.4 server in its settings loads. Never a real profile.
#[test]
#[ignore = "needs DAINTY_ELECTRON_PROFILE, a profile made by test/electron-keystore (see its README)"]
fn an_electron_profile_reads_back() {
    use dainty_launcher_kit::core::i18n::Language;
    use dainty_launcher_kit::platform::keystore::legacy::ElectronReader;
    use dainty_launcher_kit::platform::keystore::os::platform_encryptor;
    use dainty_launcher_kit::platform::paths::Paths;
    use dainty_launcher_kit::platform::settings::{Os, SettingsStore};
    use serde_json::Value;

    let profile = PathBuf::from(std::env::var_os("DAINTY_ELECTRON_PROFILE").expect("DAINTY_ELECTRON_PROFILE"));
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(profile.join("expected.json")).unwrap()).unwrap();
    let paths = Paths {
        user_data: profile.join("User Data"),
        documents: profile.join("Documents"),
        home: profile.clone(),
    };
    paths.migrate_legacy();
    let crypto = platform_encryptor();
    assert!(ElectronReader::supported(), "the Electron launcher's Local State key");
    let legacy: Arc<dyn LegacyReader> = Arc::new(ElectronReader::new(&paths.user_data));
    let slots: Vec<(KeySlot, Option<String>, Option<String>, bool)> = expected["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| {
            let mode = if slot["mode"] == "public" {
                ServerMode::Public
            } else {
                ServerMode::Private
            };
            (
                KeySlot {
                    slot: Slot::new(
                        mode,
                        slot["host"].as_str().unwrap(),
                        slot["port"].as_u64().unwrap() as u16,
                    ),
                    fp: slot["fp"].as_str().map(str::to_owned),
                },
                slot["key"].as_str().map(str::to_owned),
                slot["certificate"].as_str().map(str::to_owned),
                slot["documents"] == true,
            )
        })
        .collect();
    assert!(slots.len() >= 8);

    let open = |legacy: Option<Arc<dyn LegacyReader>>, recovery: PathBuf| Keys {
        store: KeyStore::new(paths.keys(), recovery, Arc::clone(&crypto), legacy),
        dir: paths.keys(),
    };
    let nowhere = tempfile::tempdir().unwrap();
    let keys = open(Some(legacy), nowhere.path().join("recovery"));
    for (slot, key, certificate, _) in &slots {
        assert_eq!(
            &keys.load(slot).unwrap(),
            key,
            "{}:{} {:?}",
            slot.slot.host,
            slot.slot.port,
            slot.fp
        );
        if slot.slot.mode == ServerMode::Public {
            assert_eq!(
                keys.certificate_of(slot),
                Some(certificate.clone()),
                "{}",
                slot.slot.host
            );
        }
        assert!(keys.legacy_file(slot).exists(), "the Electron file stays");
    }
    // Moved: the same keys without the Electron reader
    let moved = open(None, nowhere.path().join("recovery"));
    for (slot, key, _, _) in &slots {
        if key.is_some() {
            assert!(moved.file(slot).exists());
        }
        assert_eq!(&moved.load(slot).unwrap(), key, "{} after the move", slot.slot.host);
    }
    // The Documents copies alone, as after reinstalling the system
    let empty = tempfile::tempdir().unwrap();
    let restored = Keys::with_recovery(empty.path(), Arc::clone(&crypto), None, paths.recovery());
    for (slot, key, _, documents) in &slots {
        if *documents {
            assert_eq!(&restored.load(slot).unwrap(), key, "{} from Documents", slot.slot.host);
        }
    }

    let os = if expected["platform"] == "win32" {
        Os::Windows
    } else {
        Os::Linux
    };
    let settings = SettingsStore::load(&paths.user_data, Language::En, os);
    let saved = settings.map()["savedServers"].as_array().unwrap();
    assert_eq!(
        settings.map()["server"],
        Value::Null,
        "a 1.4.4 server is not the current one"
    );
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0]["game"], "revived");
    assert_eq!(saved[1]["game"], "reforged");
    assert_eq!(settings.map()["usernames"].as_object().unwrap().len(), 1);
}

/// The backup file's lines, as Reforged's `backupFileText` wrote them; its two advisory sentences are dainty-launcher-kit's
/// own (L3).
#[test]
fn the_backup_file() {
    let text = backup_text(&Backup {
        key: KEY,
        username: None,
        server_name: "Hunts",
        host: "play.example.org",
        port: 443,
        fingerprint: Some("ab"),
        saved: chrono::NaiveDate::from_ymd_opt(2026, 10, 21).unwrap(),
    });
    assert_eq!(
        text,
        format!(
            "Dainty Lass account key\r\n\r\nServer: Hunts (play.example.org:443)\r\nCertificate: ab\r\nUsername: -\r\nSaved: 2026-10-21\r\nKey: DLK_abababababababababababababababababababababababab\r\n\r\n{BACKUP_PRIVACY}\r\n{BACKUP_OTHER_PC}\r\n"
        )
    );
    assert_eq!(dainty_protocol::account::extract_account_key(&text), Some(KEY));
}

// ---------------------------------------------------------------- the files' text (format.rs)

#[test]
fn rrk2_and_bare_keys_only() {
    let fp = fp("a1");
    let stored = |fingerprint: Option<String>| Stored {
        fingerprint,
        key: KEY.into(),
    };
    assert_eq!(
        parse_text(format!("DLK2\n{fp}\n{KEY}").as_bytes()),
        Some(stored(Some(fp.clone())))
    );
    assert_eq!(parse_text(format!("DLK2\n-\n{KEY}").as_bytes()), Some(stored(None)));
    assert_eq!(
        parse_text(KEY.as_bytes()),
        Some(stored(None)),
        "the first format: the key alone"
    );
    for bad in [
        format!("DRK2\n{fp}\n{KEY}"),
        format!("DLK2\nABC\n{KEY}"),
        format!("DLK2\n{fp}\n{KEY}\n"),
        "DLK2\n-\nkey with spaces".into(),
        format!("DLK2\n-\n{KEY}\r"),
        format!("{KEY}\n"),
        String::new(),
        "short".into(),
    ] {
        assert_eq!(parse_text(bad.as_bytes()), None, "{bad:?}");
    }
    let stored = parse_text(format!("DLK2\n{fp}\n{KEY}").as_bytes()).unwrap();
    assert_eq!(parse_text(stored.to_text().as_bytes()), Some(stored));
}

#[test]
fn electron_framing() {
    let unprotect = |data: &[u8]| data.strip_prefix(b"protected:").map(<[u8]>::to_vec);
    let state = |encrypted: &[u8]| {
        serde_json::json!({ "os_crypt": { "encrypted_key": base64::engine::general_purpose::STANDARD.encode(encrypted) } })
            .to_string()
    };
    let master = [5u8; 32];
    let wrapped = [b"DPAPI".as_slice(), b"protected:", &master].concat();
    assert_eq!(legacy::master_key(&state(&wrapped), unprotect), Some(master));
    // "DPAPI" and nothing after it, the prefix in the wrong case, and Local States without the key
    assert_eq!(legacy::master_key(&state(b"DPAPI"), unprotect), None);
    let lower = [b"dpapi".as_slice(), b"protected:", &master].concat();
    assert_eq!(legacy::master_key(&state(&lower), unprotect), None);
    for bad in ["{}", r#"{"os_crypt":{"encrypted_key":1}}"#, "not json", ""] {
        assert_eq!(legacy::master_key(bad, unprotect), None, "{bad:?}");
    }
    let blob = [b"v10".as_slice(), &[0; 12], &[1; 16]].concat();
    assert_eq!(
        format::split_gcm(&blob, ELECTRON_V10),
        Some((&[0u8; 12], &[1u8; 16][..]))
    );
    assert_eq!(format::split_gcm(&blob[..blob.len() - 1], ELECTRON_V10), None);
    assert_eq!(format::split_gcm(b"v11", ELECTRON_V10), None);
    assert_eq!(
        format::split_cbc(&[b"v11".as_slice(), &[0; 32]].concat()),
        Some((true, &[0u8; 32][..]))
    );
    assert_eq!(format::split_cbc(&[b"v10".as_slice(), &[0; 17]].concat()), None);
    assert_eq!(format::split_cbc(b"v10"), None);
    assert_eq!(format::split_cbc(b"v1"), None);
}

// ---------------------------------------------------------------- the Electron files (legacy.rs)

#[test]
fn a_windows_key_file() {
    use aes_gcm::aead::{Aead, KeyInit};

    let master = [7u8; 32];
    let wrapped = [b"DPAPI".as_slice(), b"protected:", &master].concat();
    let local_state = serde_json::json!({ "os_crypt": { "encrypted_key": base64::engine::general_purpose::STANDARD.encode(&wrapped) } }).to_string();
    let unprotect = |data: &[u8]| data.strip_prefix(b"protected:").map(<[u8]>::to_vec);
    assert_eq!(legacy::master_key(&local_state, unprotect), Some(master));
    assert_eq!(legacy::master_key("{}", unprotect), None);

    let cipher = aes_gcm::Aes256Gcm::new_from_slice(&master).unwrap();
    let nonce = [3u8; 12];
    let sealed = cipher
        .encrypt(
            &aes_gcm::Nonce::try_from(&nonce[..]).unwrap(),
            b"DLK2\n-\nDLK_x".as_slice(),
        )
        .unwrap();
    let blob = [b"v10".as_slice(), &nonce, &sealed].concat();
    assert_eq!(
        legacy::open_gcm_file(&blob, &master).as_deref(),
        Some(b"DLK2\n-\nDLK_x".as_slice())
    );
    assert_eq!(legacy::open_gcm_file(&blob, &[8u8; 32]), None, "another user's key");
    assert_eq!(
        legacy::open_gcm_file(&blob[..blob.len() - 1], &master),
        None,
        "a damaged tag"
    );
}

fn linux_blob(version: &[u8], password: &[u8], plain: &[u8]) -> Vec<u8> {
    use cbc::cipher::{BlockModeEncrypt, KeyIvInit};
    let encryptor = cbc::Encryptor::<aes::Aes128>::new(&legacy::cbc_key(password).into(), &[b' '; 16].into());
    [
        version,
        &encryptor.encrypt_padded_vec::<cbc::cipher::block_padding::Pkcs7>(plain),
    ]
    .concat()
}

#[test]
fn linux_key_files() {
    let open = |blob: &[u8], password: Option<&[u8]>| legacy::open_cbc(blob, || password.map(<[u8]>::to_vec));
    let v11 = linux_blob(b"v11", b"keyring-secret", b"DLK2\n-\nDLK_y");
    assert_eq!(
        open(&v11, Some(b"keyring-secret")).as_deref(),
        Some(b"DLK2\n-\nDLK_y".as_slice())
    );
    assert_eq!(open(&v11, None), None);
    assert_eq!(open(&v11, Some(b"other")), None);
    let v10 = linux_blob(b"v10", b"peanuts", b"DLK_z");
    assert_eq!(open(&v10, None).as_deref(), Some(b"DLK_z".as_slice()));
    assert_eq!(open(b"v12abc", None), None);
    // Known answer: Chromium's key for "peanuts"
    assert_eq!(
        legacy::cbc_key(b"peanuts")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "fd621fe5a2b402539dfa147ca9272778"
    );
}

// ---------------------------------------------------------------- the OS protection (os.rs)

#[cfg(windows)]
#[test]
fn a_round_trip_for_this_user() {
    use dainty_launcher_kit::platform::keystore::os::Dpapi;
    let sealed = Dpapi.seal(b"DLK2\n-\nDLK_x").unwrap();
    assert_ne!(sealed, b"DLK2\n-\nDLK_x");
    assert_eq!(Dpapi.open(&sealed).unwrap(), b"DLK2\n-\nDLK_x");
    assert!(Dpapi.open(b"not a blob").is_err());
}

#[test]
fn sealed_files_open_only_with_their_key() {
    let data = seal_rr1(&[1; 32], b"DLK2\n-\nDLK_x").unwrap();
    assert!(data.starts_with(b"rr1"));
    assert_eq!(open_rr1(&[1; 32], &data).unwrap(), b"DLK2\n-\nDLK_x");
    assert!(open_rr1(&[2; 32], &data).is_err());
    assert_ne!(
        seal_rr1(&[1; 32], b"same").unwrap(),
        seal_rr1(&[1; 32], b"same").unwrap(),
        "a fresh nonce each time"
    );
}
