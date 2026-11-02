// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The admission key pair: read from settings, made new, and written out as a host's key set.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, STANDARD};
use p256::SecretKey;
use p256::ecdsa::{SigningKey, VerifyingKey};
use p256::elliptic_curve::Generate as _;
use p256::pkcs8::{
    DecodePrivateKey as _, DecodePublicKey as _, EncodePrivateKey as _, EncodePublicKey as _, LineEnding,
};
use rifts_relay::wire::js_number;
use rifts_seam::{Secret, Settings};

use crate::ticket::{DEFAULT_LIFETIME_SECONDS, Es256Tickets, safe_key_id};
use crate::{AdmissionError, GeneratedKeyPair, KeyGenerator, KeyId, PublicXy};

pub const KEY_ID_SETTING: &str = "P2P_ADMISSION_KEY_ID";
pub const SIGNING_KEY_SETTING: &str = "P2P_ADMISSION_SIGNING_PRIVKEY_B64";
pub const VERIFY_KEY_SETTING: &str = "P2P_ADMISSION_VERIFY_PUBKEY_B64";
pub const LIFETIME_SETTING: &str = "P2P_ADMISSION_TICKET_TTL_SECONDS";
pub const SECRET_SETTINGS: &[&str] = &[SIGNING_KEY_SETTING];
pub const PRIVATE_PEM_FILE: &str = "admission-private.pkcs8.pem";
pub const PUBLIC_PEM_FILE: &str = "admission-public.spki.pem";
pub const PUBLIC_XY_FILE: &str = "p2p-admission-public.xy";
pub const ENV_FILE: &str = "metagame-admission.env";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("P2P_ADMISSION_KEY_ID is missing or invalid.")]
    KeyId,
    #[error("Missing required P2P admission key {0}.")]
    Missing(&'static str),
    #[error("{0} is not a P-256 key in base64 PEM")]
    Unreadable(&'static str),
    #[error("the system random source failed")]
    Random,
    #[error("Usage: --output <new-directory> --key-id <safe-id>")]
    Usage,
    #[error("The output parent does not exist: {}", .0.display())]
    NoParent(PathBuf),
    #[error("Refusing to overwrite an existing key directory: {}", .0.display())]
    Exists(PathBuf),
    #[error("could not write the key set: {0}")]
    Io(String),
}

/// A PKCS#8 private key PEM, or a SEC1 `EC PRIVATE KEY` one.
pub fn signing_key_from_pem(pem: &str) -> Option<SigningKey> {
    SigningKey::from_pkcs8_pem(pem)
        .ok()
        .or_else(|| SecretKey::from_sec1_pem(pem).ok().map(SigningKey::from))
}

/// An SPKI public key PEM; nothing else.
pub fn verifying_key_from_pem(pem: &str) -> Option<VerifyingKey> {
    VerifyingKey::from_public_key_pem(pem).ok()
}

pub fn public_xy(key: &VerifyingKey) -> PublicXy {
    let point = key.to_sec1_point(false);
    let mut xy = [0_u8; 64];
    // The uncompressed point is the 0x04 tag and 64 bytes.
    if let Some(bytes) = point.as_bytes().get(1..65) {
        xy.copy_from_slice(bytes);
    }
    PublicXy(xy)
}

const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

/// Node's `Buffer.from(text, "base64")`: stops at `=`, takes either alphabet, skips anything else, drops a lone
/// trailing character and ignores trailing bits. Settings are often wrapped or pasted with stray spaces.
fn lenient_base64(text: &str) -> Option<Vec<u8>> {
    let mut kept: Vec<u8> = text
        .bytes()
        .take_while(|&byte| byte != b'=')
        .filter_map(|byte| match byte {
            b'-' => Some(b'+'),
            b'_' => Some(b'/'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => Some(byte),
            _ => None,
        })
        .collect();
    if kept.len() % 4 == 1 {
        kept.pop();
    }
    LENIENT.decode(&kept).ok()
}

fn pem_setting(settings: &dyn Settings, name: &'static str) -> Option<Result<String, KeyError>> {
    let value = settings.get(name).filter(|value| !value.is_empty())?;
    Some(
        lenient_base64(value)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .ok_or(KeyError::Unreadable(name)),
    )
}

pub fn key_id_from_settings(settings: &dyn Settings) -> Result<KeyId, KeyError> {
    let id = settings.get(KEY_ID_SETTING).unwrap_or("");
    if safe_key_id(id) {
        Ok(KeyId(id.to_owned()))
    } else {
        Err(KeyError::KeyId)
    }
}

impl Es256Tickets {
    /// A verify-only authority when the signing key is not set.
    pub fn from_settings(settings: &dyn Settings) -> Result<Es256Tickets, KeyError> {
        let key_id = key_id_from_settings(settings)?;
        let verifying = pem_setting(settings, VERIFY_KEY_SETTING).ok_or(KeyError::Missing(VERIFY_KEY_SETTING))??;
        let verifying = verifying_key_from_pem(&verifying).ok_or(KeyError::Unreadable(VERIFY_KEY_SETTING))?;
        let signing = match pem_setting(settings, SIGNING_KEY_SETTING) {
            None => None,
            Some(pem) => Some(signing_key_from_pem(&pem?).ok_or(KeyError::Unreadable(SIGNING_KEY_SETTING))?),
        };
        let default_lifetime = settings
            .get(LIFETIME_SETTING)
            .map_or(f64::from(DEFAULT_LIFETIME_SECONDS), js_number);
        Ok(Self {
            key_id,
            signing,
            verifying,
            default_lifetime,
        })
    }

    pub fn public_xy(&self) -> PublicXy {
        public_xy(&self.verifying)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct P256KeyGenerator;

impl P256KeyGenerator {
    /// The pair, the private PEM and the public PEM, each PEM with LF line endings.
    pub fn generate_with_pems(&self, key_id: KeyId) -> Result<(GeneratedKeyPair, Secret, String), KeyError> {
        if !safe_key_id(&key_id.0) {
            return Err(KeyError::KeyId);
        }
        let signing = SigningKey::try_generate_from_rng(&mut rand::rngs::SysRng).map_err(|_| KeyError::Random)?;
        let verifying = *signing.verifying_key();
        let private_pem = signing
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|error| KeyError::Io(error.to_string()))?;
        let public_pem = verifying
            .to_public_key_pem(LineEnding::LF)
            .map_err(|error| KeyError::Io(error.to_string()))?;
        let pair = GeneratedKeyPair {
            key_id,
            signing_key: Secret::new(STANDARD.encode(private_pem.as_bytes())),
            verify_key: STANDARD.encode(public_pem.as_bytes()),
            public_xy: public_xy(&verifying),
        };
        Ok((pair, Secret::new(private_pem.to_string()), public_pem))
    }
}

impl KeyGenerator for P256KeyGenerator {
    fn generate(&self, key_id: KeyId) -> Result<GeneratedKeyPair, AdmissionError> {
        Ok(self.generate_with_pems(key_id)?.0)
    }
}

fn io(error: &std::io::Error) -> KeyError {
    KeyError::Io(error.to_string())
}

/// Created new (never over an existing file) and synced; owner-only on Unix for the secret files.
fn write_new(path: &Path, bytes: &[u8], private: bool) -> Result<(), KeyError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file: File = options.open(path).map_err(|error| io(&error))?;
    file.write_all(bytes).map_err(|error| io(&error))?;
    file.sync_all().map_err(|error| io(&error))
}

fn fill_key_set(folder: &Path, key_id: KeyId, output: &Path) -> Result<GeneratedKeyPair, KeyError> {
    let (pair, private_pem, public_pem) = P256KeyGenerator.generate_with_pems(key_id)?;
    write_new(&folder.join(PRIVATE_PEM_FILE), private_pem.expose().as_bytes(), true)?;
    write_new(&folder.join(PUBLIC_PEM_FILE), public_pem.as_bytes(), false)?;
    write_new(&folder.join(PUBLIC_XY_FILE), &pair.public_xy.0, false)?;
    write_new(&folder.join(ENV_FILE), pair.env_lines().expose().as_bytes(), true)?;
    fs::rename(folder, output).map_err(|error| io(&error))?;
    Ok(pair)
}

/// Writes a new key set into a new folder `output`, all or nothing: the files go into a temporary sibling that is
/// renamed into place.
pub fn write_key_set(
    output: &Path,
    key_id: KeyId,
    process_id: u32,
    now_millis: i64,
) -> Result<GeneratedKeyPair, KeyError> {
    if !safe_key_id(&key_id.0) {
        return Err(KeyError::Usage);
    }
    let output = std::path::absolute(output).map_err(|error| io(&error))?;
    let parent = output.parent().map(Path::to_path_buf).unwrap_or_default();
    if !parent.is_dir() {
        return Err(KeyError::NoParent(parent));
    }
    if fs::symlink_metadata(&output).is_ok() {
        return Err(KeyError::Exists(output));
    }
    let folder = parent.join(format!(".admission-key-{process_id}-{now_millis}"));
    fs::create_dir(&folder).map_err(|error| io(&error))?;
    let written = fill_key_set(&folder, key_id, &output);
    if written.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    written
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use p256::ecdsa::signature::Signer as _;

    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn generated(id: &str) -> GeneratedKeyPair {
        P256KeyGenerator.generate(KeyId(id.to_owned())).unwrap()
    }

    #[test]
    fn a_generated_pair_loads_back() {
        let pair = generated("k1");
        let loaded = Es256Tickets::from_settings(&settings(&[
            (KEY_ID_SETTING, "k1"),
            (SIGNING_KEY_SETTING, pair.signing_key.expose()),
            (VERIFY_KEY_SETTING, &pair.verify_key),
        ]))
        .unwrap();
        assert_eq!(loaded.key_id, KeyId("k1".to_owned()));
        assert_eq!(loaded.public_xy(), pair.public_xy);
        assert_eq!(loaded.default_lifetime, 75.0);
        let signing = loaded.signing.clone().unwrap();
        assert_eq!(*signing.verifying_key(), loaded.verifying);
        let point = loaded.verifying.to_sec1_point(false);
        assert_eq!(&point.as_bytes()[1..], &pair.public_xy.0[..]);

        let lines = pair.env_lines();
        let expected_start = format!(
            "P2P_ADMISSION_KEY_ID=k1\nP2P_ADMISSION_SIGNING_PRIVKEY_B64={}\n",
            pair.signing_key.expose()
        );
        assert!(lines.expose().starts_with(&expected_start));
        assert!(
            lines
                .expose()
                .ends_with(&format!("P2P_ADMISSION_VERIFY_PUBKEY_B64={}\n", pair.verify_key))
        );
        assert_eq!(lines.expose().lines().count(), 3);

        let debug = format!("{loaded:?} {pair:?}");
        assert!(debug.contains("Some(\"<redacted>\")"));
        assert!(!debug.contains(pair.signing_key.expose()));
        let pem = String::from_utf8(STANDARD.decode(pair.signing_key.expose()).unwrap()).unwrap();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert!(!pem.contains('\r'));
    }

    #[test]
    fn settings_errors() {
        let pair = generated("k1");
        let verify = pair.verify_key.as_str();
        let error = |pairs: &[(&str, &str)]| Es256Tickets::from_settings(&settings(pairs)).unwrap_err();
        assert_eq!(
            error(&[(KEY_ID_SETTING, "bad id"), (VERIFY_KEY_SETTING, verify)]),
            KeyError::KeyId
        );
        assert_eq!(error(&[(VERIFY_KEY_SETTING, verify)]), KeyError::KeyId);
        assert_eq!(error(&[(KEY_ID_SETTING, "k1")]), KeyError::Missing(VERIFY_KEY_SETTING));
        assert_eq!(
            error(&[(KEY_ID_SETTING, "k1"), (VERIFY_KEY_SETTING, "")]),
            KeyError::Missing(VERIFY_KEY_SETTING)
        );
        assert_eq!(
            error(&[(KEY_ID_SETTING, "k1"), (VERIFY_KEY_SETTING, "bm90IGEga2V5")]),
            KeyError::Unreadable(VERIFY_KEY_SETTING)
        );
        assert_eq!(
            error(&[
                (KEY_ID_SETTING, "k1"),
                (VERIFY_KEY_SETTING, verify),
                (SIGNING_KEY_SETTING, "bm90IGEga2V5")
            ]),
            KeyError::Unreadable(SIGNING_KEY_SETTING)
        );
        // The private key is not a public key.
        assert_eq!(
            error(&[(KEY_ID_SETTING, "k1"), (VERIFY_KEY_SETTING, pair.signing_key.expose())]),
            KeyError::Unreadable(VERIFY_KEY_SETTING)
        );

        let verify_only =
            Es256Tickets::from_settings(&settings(&[(KEY_ID_SETTING, "k1"), (VERIFY_KEY_SETTING, verify)])).unwrap();
        assert!(verify_only.signing.is_none());
        assert!(format!("{verify_only:?}").contains("signing: None"));

        let blank = Es256Tickets::from_settings(&settings(&[
            (KEY_ID_SETTING, "k1"),
            (VERIFY_KEY_SETTING, verify),
            (LIFETIME_SETTING, " "),
        ]))
        .unwrap();
        assert_eq!(blank.default_lifetime, 0.0);
        let garbage = Es256Tickets::from_settings(&settings(&[
            (KEY_ID_SETTING, "k1"),
            (VERIFY_KEY_SETTING, verify),
            (LIFETIME_SETTING, "soon"),
        ]))
        .unwrap();
        assert!(garbage.default_lifetime.is_nan());

        assert_eq!(
            P256KeyGenerator.generate(KeyId("../x".to_owned())),
            Err(AdmissionError::Key(KeyError::KeyId))
        );
        assert_eq!(
            key_id_from_settings(&settings(&[(KEY_ID_SETTING, "a.b_c-1")])),
            Ok(KeyId("a.b_c-1".to_owned()))
        );
    }

    #[test]
    fn sec1_keys_and_wrapped_base64_load() {
        let signing = SigningKey::try_generate_from_rng(&mut rand::rngs::SysRng).unwrap();
        let sec1 = SecretKey::from(&signing).to_sec1_pem(LineEnding::CRLF).unwrap();
        assert!(sec1.starts_with("-----BEGIN EC PRIVATE KEY-----"));
        let encoded = STANDARD.encode(sec1.as_bytes());
        let wrapped: Vec<String> = encoded
            .as_bytes()
            .chunks(60)
            .map(|chunk| String::from_utf8(chunk.to_vec()).unwrap())
            .collect();
        let wrapped = wrapped.join("\r\n");
        let public = signing.verifying_key().to_public_key_pem(LineEnding::CRLF).unwrap();
        let public = STANDARD.encode(public.as_bytes()).replace('+', "-").replace('/', "_");
        let loaded = Es256Tickets::from_settings(&settings(&[
            (KEY_ID_SETTING, "k1"),
            (SIGNING_KEY_SETTING, &wrapped),
            (VERIFY_KEY_SETTING, &format!(" {public} ")),
        ]))
        .unwrap();
        let loaded_signing = loaded.signing.unwrap();
        assert_eq!(loaded_signing, signing);
        let signature: p256::ecdsa::Signature = loaded_signing.sign(b"x");
        assert!(p256::ecdsa::signature::Verifier::verify(&loaded.verifying, b"x", &signature).is_ok());
    }

    #[test]
    fn lenient_base64_matches_node() {
        assert_eq!(lenient_base64("aGk=").unwrap(), b"hi");
        assert_eq!(lenient_base64("aGk").unwrap(), b"hi");
        assert_eq!(lenient_base64("a G\nk").unwrap(), b"hi");
        assert_eq!(lenient_base64("aGk=garbage").unwrap(), b"hi");
        assert_eq!(lenient_base64("aGl").unwrap(), b"hi");
        assert_eq!(lenient_base64("aGkx_-").unwrap(), lenient_base64("aGkx/+").unwrap());
        // A lone sixth-bit character is dropped.
        assert_eq!(lenient_base64("aGkxa").unwrap(), lenient_base64("aGkx").unwrap());
        assert_eq!(lenient_base64("").unwrap(), b"");
    }

    #[test]
    fn writes_a_key_set_once() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("keys");
        let pair = write_key_set(&output, KeyId("k1".to_owned()), 42, 1_000).unwrap();
        let read = |name: &str| fs::read(output.join(name)).unwrap();
        assert_eq!(read(PUBLIC_XY_FILE), pair.public_xy.0.to_vec());
        assert_eq!(read(ENV_FILE), pair.env_lines().expose().as_bytes());
        let private = String::from_utf8(read(PRIVATE_PEM_FILE)).unwrap();
        assert_eq!(STANDARD.encode(private.as_bytes()), pair.signing_key.expose());
        let public = String::from_utf8(read(PUBLIC_PEM_FILE)).unwrap();
        assert_eq!(STANDARD.encode(public.as_bytes()), pair.verify_key);
        assert!(!dir.path().join(".admission-key-42-1000").exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(output.join(PRIVATE_PEM_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let absolute = std::path::absolute(&output).unwrap();
        let again = write_key_set(&output, KeyId("k1".to_owned()), 42, 2_000).unwrap_err();
        assert_eq!(again, KeyError::Exists(absolute.clone()));
        assert_eq!(
            again.to_string(),
            format!(
                "Refusing to overwrite an existing key directory: {}",
                absolute.display()
            )
        );

        let orphan = dir.path().join("missing").join("keys");
        let parent = std::path::absolute(dir.path().join("missing")).unwrap();
        assert_eq!(
            write_key_set(&orphan, KeyId("k1".to_owned()), 1, 1),
            Err(KeyError::NoParent(parent))
        );
        assert_eq!(write_key_set(&output, KeyId(String::new()), 1, 1), Err(KeyError::Usage));
        assert_eq!(
            KeyError::Usage.to_string(),
            "Usage: --output <new-directory> --key-id <safe-id>"
        );
    }

    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("keys");
        // The temporary folder's name is taken, so it cannot be created.
        let folder = dir.path().join(".admission-key-7-9");
        fs::create_dir(&folder).unwrap();
        let blocked = write_key_set(&output, KeyId("k1".to_owned()), 7, 9).unwrap_err();
        assert!(matches!(blocked, KeyError::Io(_)), "{blocked:?}");
        assert!(folder.exists(), "a folder this call did not create is left alone");
        assert!(!output.exists());
    }
}
