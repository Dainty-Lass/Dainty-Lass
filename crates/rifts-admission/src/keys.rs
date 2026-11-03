// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The admission key pair: read from settings, made new, and written out as a host's key set.

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD},
};
use p256::{
    SecretKey,
    ecdsa::{SigningKey, VerifyingKey},
    elliptic_curve::Generate as _,
    pkcs8::{DecodePrivateKey as _, DecodePublicKey as _, EncodePrivateKey as _, EncodePublicKey as _, LineEnding},
};
use rifts_seam::{Secret, Settings};

use crate::{
    AdmissionError, GeneratedKeyPair, KeyGenerator, KeyId, PublicXy,
    ticket::{DEFAULT_LIFETIME_SECONDS, Es256Tickets, safe_key_id},
};

pub const KEY_ID_SETTING: &str = "P2P_ADMISSION_KEY_ID";
pub const SIGNING_KEY_SETTING: &str = "P2P_ADMISSION_SIGNING_PRIVKEY_B64";
pub const VERIFY_KEY_SETTING: &str = "P2P_ADMISSION_VERIFY_PUBKEY_B64";
pub const LIFETIME_SETTING: &str = "P2P_ADMISSION_TICKET_TTL_SECONDS";
pub const SECRET_SETTINGS: &[&str] = &[SIGNING_KEY_SETTING];
pub const PRIVATE_PEM_FILE: &str = "admission-private.pkcs8.pem";
pub const PUBLIC_PEM_FILE: &str = "admission-public.spki.pem";
pub const PUBLIC_XY_FILE: &str = "p2p-admission-public.xy";
pub const ENV_FILE: &str = "metagame-admission.env";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyError {
    KeyId,
    Missing(&'static str),
    Unreadable(&'static str),
    Random,
    Usage,
    NoParent(PathBuf),
    Exists(PathBuf),
    Io(String),
}

impl fmt::Display for KeyError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyId => out.write_str("P2P_ADMISSION_KEY_ID is missing or invalid."),
            Self::Missing(setting) => write!(out, "Missing required P2P admission key {setting}."),
            Self::Unreadable(setting) => write!(out, "{setting} is not a P-256 key in base64 PEM"),
            Self::Random => out.write_str("the system random source failed"),
            Self::Usage => out.write_str("Usage: --output <new-directory> --key-id <safe-id>"),
            Self::NoParent(path) => write!(out, "The output parent does not exist: {}", path.display()),
            Self::Exists(path) => write!(
                out,
                "Refusing to overwrite an existing key directory: {}",
                path.display()
            ),
            Self::Io(detail) => write!(out, "could not write the key set: {detail}"),
        }
    }
}

impl std::error::Error for KeyError {}

fn io_failure(error: impl fmt::Display) -> KeyError {
    KeyError::Io(error.to_string())
}

/// A PKCS#8 private key PEM, or a SEC1 `EC PRIVATE KEY` one.
pub fn signing_key_from_pem(pem: &str) -> Option<SigningKey> {
    match SigningKey::from_pkcs8_pem(pem) {
        Ok(key) => Some(key),
        Err(_) => SecretKey::from_sec1_pem(pem).map(SigningKey::from).ok(),
    }
}

/// An SPKI public key PEM; nothing else.
pub fn verifying_key_from_pem(pem: &str) -> Option<VerifyingKey> {
    VerifyingKey::from_public_key_pem(pem).ok()
}

pub fn public_xy(key: &VerifyingKey) -> PublicXy {
    let encoded = key.to_sec1_point(false);
    // Uncompressed SEC1: the 0x04 tag, then X and Y.
    PublicXy(encoded.as_bytes().last_chunk::<64>().copied().unwrap_or([0; 64]))
}

const FORGIVING: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);

/// Node's `Buffer.from(text, "base64")`: stops at `=`, takes either alphabet, skips anything else, drops a lone
/// trailing character and ignores trailing bits. Settings are often wrapped or pasted with stray spaces.
fn lenient_base64(text: &str) -> Option<Vec<u8>> {
    let body = text.split('=').next().unwrap_or_default();
    let mut symbols = String::with_capacity(body.len());
    for c in body.chars() {
        match c {
            '-' => symbols.push('+'),
            '_' => symbols.push('/'),
            '+' | '/' => symbols.push(c),
            _ if c.is_ascii_alphanumeric() => symbols.push(c),
            _ => {}
        }
    }
    let whole = symbols.len() - usize::from(symbols.len() % 4 == 1);
    FORGIVING.decode(&symbols[..whole]).ok()
}

/// A base64 PEM setting decoded to text; `None` when it is unset or empty.
fn pem_setting(settings: &dyn Settings, name: &'static str) -> Option<Result<String, KeyError>> {
    let encoded = settings.get(name)?;
    if encoded.is_empty() {
        return None;
    }
    let decoded = lenient_base64(encoded).ok_or(KeyError::Unreadable(name));
    Some(decoded.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}

pub fn key_id_from_settings(settings: &dyn Settings) -> Result<KeyId, KeyError> {
    settings
        .get(KEY_ID_SETTING)
        .filter(|id| safe_key_id(id))
        .map(|id| KeyId(id.to_owned()))
        .ok_or(KeyError::KeyId)
}

impl Es256Tickets {
    /// A verify-only authority when the signing key is not set.
    pub fn from_settings(settings: &dyn Settings) -> Result<Es256Tickets, KeyError> {
        let key_id = key_id_from_settings(settings)?;
        let public_pem =
            pem_setting(settings, VERIFY_KEY_SETTING).unwrap_or(Err(KeyError::Missing(VERIFY_KEY_SETTING)))?;
        let verifying = verifying_key_from_pem(&public_pem).ok_or(KeyError::Unreadable(VERIFY_KEY_SETTING))?;
        let signing = pem_setting(settings, SIGNING_KEY_SETTING)
            .transpose()?
            .map(|pem| signing_key_from_pem(&pem).ok_or(KeyError::Unreadable(SIGNING_KEY_SETTING)))
            .transpose()?;
        let default_lifetime = match settings.get(LIFETIME_SETTING) {
            Some(text) => rifts_relay::wire::js_number(text),
            None => f64::from(DEFAULT_LIFETIME_SECONDS),
        };
        Ok(Es256Tickets {
            verifying,
            signing,
            key_id,
            default_lifetime,
        })
    }

    pub fn public_xy(&self) -> PublicXy {
        public_xy(&self.verifying)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct P256KeyGenerator;

impl P256KeyGenerator {
    /// The pair, the private PEM and the public PEM, each PEM with LF line endings.
    pub fn generate_with_pems(&self, key_id: KeyId) -> Result<(GeneratedKeyPair, Secret, String), KeyError> {
        if !safe_key_id(&key_id.0) {
            return Err(KeyError::KeyId);
        }
        let secret = SigningKey::try_generate_from_rng(&mut rand::rngs::SysRng).map_err(|_| KeyError::Random)?;
        let public = secret.verifying_key();
        let private_pem = secret.to_pkcs8_pem(LineEnding::LF).map_err(io_failure)?;
        let public_pem = public.to_public_key_pem(LineEnding::LF).map_err(io_failure)?;
        let [signing_key, verify_key] = [private_pem.as_bytes(), public_pem.as_bytes()].map(|pem| STANDARD.encode(pem));
        let pair = GeneratedKeyPair {
            public_xy: public_xy(public),
            signing_key: Secret::new(signing_key),
            verify_key,
            key_id,
        };
        Ok((pair, Secret::new(String::from(private_pem.as_str())), public_pem))
    }
}

impl KeyGenerator for P256KeyGenerator {
    fn generate(&self, id: KeyId) -> Result<GeneratedKeyPair, AdmissionError> {
        let (pair, _, _) = self.generate_with_pems(id)?;
        Ok(pair)
    }
}

/// Created new (never over an existing file) and synced; owner-only on Unix for the secret files.
fn write_new(path: &Path, bytes: &[u8], private: bool) -> Result<(), KeyError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        if private {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path).map_err(io_failure)?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(io_failure)
}

fn fill_key_set(folder: &Path, key_id: KeyId, output: &Path) -> Result<GeneratedKeyPair, KeyError> {
    let (pair, private_pem, public_pem) = P256KeyGenerator.generate_with_pems(key_id)?;
    let env = pair.env_lines();
    let files: [(&str, &[u8], bool); 4] = [
        (PRIVATE_PEM_FILE, private_pem.expose().as_bytes(), true),
        (PUBLIC_PEM_FILE, public_pem.as_bytes(), false),
        (PUBLIC_XY_FILE, &pair.public_xy.0, false),
        (ENV_FILE, env.expose().as_bytes(), true),
    ];
    for (name, bytes, private) in files {
        write_new(&folder.join(name), bytes, private)?;
    }
    fs::rename(folder, output).map_err(io_failure)?;
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
    let target = std::path::absolute(output).map_err(io_failure)?;
    let parent = target.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    if !parent.is_dir() {
        return Err(KeyError::NoParent(parent));
    }
    if target.symlink_metadata().is_ok() {
        return Err(KeyError::Exists(target));
    }
    let staging = parent.join(format!(".admission-key-{process_id}-{now_millis}"));
    fs::create_dir(&staging).map_err(io_failure)?;
    fill_key_set(&staging, key_id, &target).inspect_err(|_| {
        // Best effort: the write's own error is the one to report.
        let _ = fs::remove_dir_all(&staging);
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use p256::ecdsa::{Signature, signature::Signer as _, signature::Verifier as _};

    use super::*;

    /// Settings with the key id `k1`, plus `extra`.
    fn with_k1(extra: &[(&str, &str)]) -> HashMap<String, String> {
        [(KEY_ID_SETTING, "k1")]
            .iter()
            .chain(extra)
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    fn load(settings: &HashMap<String, String>) -> Result<Es256Tickets, KeyError> {
        Es256Tickets::from_settings(settings)
    }

    fn fresh_pair() -> GeneratedKeyPair {
        P256KeyGenerator.generate(KeyId("k1".to_owned())).unwrap()
    }

    fn id(text: &str) -> KeyId {
        KeyId(text.to_owned())
    }

    #[test]
    fn a_generated_pair_loads_back() {
        let pair = fresh_pair();
        let loaded = load(&with_k1(&[
            (SIGNING_KEY_SETTING, pair.signing_key.expose()),
            (VERIFY_KEY_SETTING, &pair.verify_key),
        ]))
        .unwrap();
        assert_eq!((loaded.key_id.clone(), loaded.public_xy()), (id("k1"), pair.public_xy));
        assert_eq!(loaded.default_lifetime, 75.0);
        assert_eq!(
            loaded.signing.as_ref().map(|key| *key.verifying_key()),
            Some(loaded.verifying)
        );
        assert_eq!(loaded.verifying.to_sec1_point(false).as_bytes()[1..], pair.public_xy.0);

        let env = pair.env_lines();
        let lines: Vec<&str> = env.expose().lines().collect();
        assert_eq!(
            lines,
            [
                "P2P_ADMISSION_KEY_ID=k1".to_owned(),
                format!("P2P_ADMISSION_SIGNING_PRIVKEY_B64={}", pair.signing_key.expose()),
                format!("P2P_ADMISSION_VERIFY_PUBKEY_B64={}", pair.verify_key),
            ]
        );
        assert!(env.expose().ends_with('\n'));

        let debug = format!("{loaded:?} {pair:?}");
        assert!(debug.contains("Some(\"<redacted>\")"));
        assert!(!debug.contains(pair.signing_key.expose()));
        let pem = String::from_utf8(STANDARD.decode(pair.signing_key.expose()).unwrap()).unwrap();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----\n") && !pem.contains('\r'));
    }

    #[test]
    fn settings_errors() {
        let pair = fresh_pair();
        let verify = pair.verify_key.as_str();
        let refusal = |settings: HashMap<String, String>| load(&settings).unwrap_err();
        let mut bad_id = with_k1(&[(VERIFY_KEY_SETTING, verify)]);
        bad_id.insert(KEY_ID_SETTING.to_owned(), "bad id".to_owned());
        assert_eq!(refusal(bad_id), KeyError::KeyId);
        let mut no_id = with_k1(&[(VERIFY_KEY_SETTING, verify)]);
        no_id.remove(KEY_ID_SETTING);
        assert_eq!(refusal(no_id), KeyError::KeyId);

        let cases: [(&[(&str, &str)], KeyError); 5] = [
            (&[], KeyError::Missing(VERIFY_KEY_SETTING)),
            (&[(VERIFY_KEY_SETTING, "")], KeyError::Missing(VERIFY_KEY_SETTING)),
            (
                &[(VERIFY_KEY_SETTING, "bm90IGEga2V5")],
                KeyError::Unreadable(VERIFY_KEY_SETTING),
            ),
            (
                &[(VERIFY_KEY_SETTING, verify), (SIGNING_KEY_SETTING, "bm90IGEga2V5")],
                KeyError::Unreadable(SIGNING_KEY_SETTING),
            ),
            // The private key is not a public key.
            (
                &[(VERIFY_KEY_SETTING, pair.signing_key.expose())],
                KeyError::Unreadable(VERIFY_KEY_SETTING),
            ),
        ];
        for (extra, expected) in cases {
            assert_eq!(refusal(with_k1(extra)), expected, "{extra:?}");
        }

        let verify_only = load(&with_k1(&[(VERIFY_KEY_SETTING, verify)])).unwrap();
        assert!(verify_only.signing.is_none());
        assert!(format!("{verify_only:?}").contains("signing: None"));

        let lifetime = |text: &str| {
            load(&with_k1(&[(VERIFY_KEY_SETTING, verify), (LIFETIME_SETTING, text)]))
                .unwrap()
                .default_lifetime
        };
        assert_eq!(lifetime(" "), 0.0);
        assert!(lifetime("soon").is_nan());

        assert_eq!(
            P256KeyGenerator.generate(id("../x")),
            Err(AdmissionError::Key(KeyError::KeyId))
        );
        assert_eq!(
            key_id_from_settings(&with_k1(&[(KEY_ID_SETTING, "a.b_c-1")])),
            Ok(id("a.b_c-1"))
        );
    }

    #[test]
    fn sec1_keys_and_wrapped_base64_load() {
        let signing = SigningKey::try_generate_from_rng(&mut rand::rngs::SysRng).unwrap();
        let sec1 = SecretKey::from(&signing).to_sec1_pem(LineEnding::CRLF).unwrap();
        assert!(sec1.starts_with("-----BEGIN EC PRIVATE KEY-----"));
        let encoded = STANDARD.encode(sec1.as_bytes());
        let wrapped = encoded
            .as_bytes()
            .chunks(60)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n");
        let public_pem = signing.verifying_key().to_public_key_pem(LineEnding::CRLF).unwrap();
        let url_alphabet: String = STANDARD
            .encode(public_pem.as_bytes())
            .chars()
            .map(|c| match c {
                '+' => '-',
                '/' => '_',
                other => other,
            })
            .collect();
        let padded = format!(" {url_alphabet} ");
        let loaded = load(&with_k1(&[
            (SIGNING_KEY_SETTING, &wrapped),
            (VERIFY_KEY_SETTING, &padded),
        ]))
        .unwrap();
        let reloaded = loaded.signing.unwrap();
        assert_eq!(reloaded, signing);
        let signature: Signature = reloaded.sign(b"x");
        assert!(loaded.verifying.verify(b"x", &signature).is_ok());
    }

    #[test]
    fn lenient_base64_matches_node() {
        let decoded = |text: &str| lenient_base64(text).unwrap();
        for text in ["aGk=", "aGk", "a G\nk", "aGk=garbage", "aGl"] {
            assert_eq!(decoded(text), b"hi", "{text:?}");
        }
        assert_eq!(decoded("aGkx_-"), decoded("aGkx/+"));
        // A lone sixth-bit character is dropped.
        assert_eq!(decoded("aGkxa"), decoded("aGkx"));
        assert!(decoded("").is_empty());
    }

    #[test]
    fn writes_a_key_set_once() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("keys");
        let pair = write_key_set(&output, id("k1"), 42, 1_000).unwrap();
        let read = |name: &str| fs::read(output.join(name)).unwrap();
        assert_eq!(read(PUBLIC_XY_FILE), pair.public_xy.0);
        assert_eq!(read(ENV_FILE), pair.env_lines().expose().as_bytes());
        assert_eq!(STANDARD.encode(read(PRIVATE_PEM_FILE)), pair.signing_key.expose());
        assert_eq!(STANDARD.encode(read(PUBLIC_PEM_FILE)), pair.verify_key);
        assert!(!dir.path().join(".admission-key-42-1000").exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let metadata = fs::metadata(output.join(PRIVATE_PEM_FILE)).unwrap();
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        }

        let absolute = std::path::absolute(&output).unwrap();
        let again = write_key_set(&output, id("k1"), 42, 2_000).unwrap_err();
        assert_eq!(
            again.to_string(),
            format!(
                "Refusing to overwrite an existing key directory: {}",
                absolute.display()
            )
        );
        assert_eq!(again, KeyError::Exists(absolute));

        let orphan = dir.path().join("missing").join("keys");
        let parent = std::path::absolute(dir.path().join("missing")).unwrap();
        assert_eq!(write_key_set(&orphan, id("k1"), 1, 1), Err(KeyError::NoParent(parent)));
        assert_eq!(write_key_set(&output, id(""), 1, 1), Err(KeyError::Usage));
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
        let taken = dir.path().join(".admission-key-7-9");
        fs::create_dir(&taken).unwrap();
        let blocked = write_key_set(&output, id("k1"), 7, 9).unwrap_err();
        assert!(matches!(blocked, KeyError::Io(_)), "{blocked:?}");
        assert!(taken.exists(), "a folder this call did not create is left alone");
        assert!(!output.exists());
    }
}
