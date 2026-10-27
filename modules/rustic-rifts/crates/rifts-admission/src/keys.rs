//! The admission key pair: P-256, the backend's signing key as base64 PEM in `secrets.env`, its public half as base64
//! PEM for the backend and as `p2p-admission-public.xy` (raw X || Y) for the transport. Ported from
//! `scripts/p2p/generate-admission-key.mjs` and the key loading in `admissionTickets.ts`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use p256::ecdsa::{SigningKey, VerifyingKey};
use p256::elliptic_curve::Generate;
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rifts_relay::wire::js_number;
use rifts_seam::{Secret, Settings};

use crate::ticket::{DEFAULT_LIFETIME_SECONDS, Es256Tickets, safe_key_id};
use crate::{AdmissionError, GeneratedKeyPair, KeyGenerator, KeyId, PublicXy};

pub const KEY_ID_SETTING: &str = "P2P_ADMISSION_KEY_ID";
pub const SIGNING_KEY_SETTING: &str = "P2P_ADMISSION_SIGNING_PRIVKEY_B64";
pub const VERIFY_KEY_SETTING: &str = "P2P_ADMISSION_VERIFY_PUBKEY_B64";
pub const LIFETIME_SETTING: &str = "P2P_ADMISSION_TICKET_TTL_SECONDS";
/// The settings that are secrets: made once, kept in `secrets.env`, never logged.
pub const SECRET_SETTINGS: &[&str] = &[SIGNING_KEY_SETTING];

/// The four files the generator writes, in the folder it creates.
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
    #[error("The output parent does not exist: {0}")]
    NoParent(PathBuf),
    #[error("Refusing to overwrite an existing key directory: {0}")]
    Exists(PathBuf),
    #[error("could not write the key set: {0}")]
    Io(String),
}

/// `Buffer.from(text, "base64")`: skips characters outside the alphabet, takes the URL-safe letters too, and stops at
/// the first `=`.
fn lenient_base64(text: &str) -> Option<Vec<u8>> {
    let cleaned: String = text
        .chars()
        .take_while(|&c| c != '=')
        .filter_map(|c| match c {
            '-' => Some('+'),
            '_' => Some('/'),
            c if c.is_ascii_alphanumeric() || c == '+' || c == '/' => Some(c),
            _ => None,
        })
        .collect();
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone),
    );
    // A dangling sixth bit is dropped, as Node does
    let usable = if cleaned.len() % 4 == 1 {
        &cleaned[..cleaned.len() - 1]
    } else {
        &cleaned[..]
    };
    engine.decode(usable).ok()
}

fn pem_setting(settings: &dyn Settings, name: &'static str) -> Result<String, KeyError> {
    let encoded = settings
        .get(name)
        .filter(|value| !value.is_empty())
        .ok_or(KeyError::Missing(name))?;
    let bytes = lenient_base64(encoded).ok_or(KeyError::Unreadable(name))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// `createPrivateKey(pem)` for the formats Node writes and reads unencrypted: PKCS#8, or SEC1 (`EC PRIVATE KEY`).
pub fn signing_key_from_pem(pem: &str) -> Option<SigningKey> {
    SigningKey::from_pkcs8_pem(pem)
        .ok()
        .or_else(|| p256::SecretKey::from_sec1_pem(pem).ok().map(SigningKey::from))
}

/// `createPublicKey(pem)` for an SPKI `PUBLIC KEY`.
pub fn verifying_key_from_pem(pem: &str) -> Option<VerifyingKey> {
    VerifyingKey::from_public_key_pem(pem).ok()
}

/// `KeyId()`: the active key id, `^[A-Za-z0-9._-]{1,32}$`.
pub fn key_id_from_settings(settings: &dyn Settings) -> Result<KeyId, KeyError> {
    let value = settings.get(KEY_ID_SETTING).unwrap_or_default();
    if safe_key_id(value) {
        Ok(KeyId(value.to_owned()))
    } else {
        Err(KeyError::KeyId)
    }
}

impl Es256Tickets {
    /// The admission settings. The signing key is optional (a verify-only reader); minting without it fails as Node's
    /// does, at the first ticket.
    pub fn from_settings(settings: &dyn Settings) -> Result<Self, KeyError> {
        let key_id = key_id_from_settings(settings)?;
        let verifying = verifying_key_from_pem(&pem_setting(settings, VERIFY_KEY_SETTING)?)
            .ok_or(KeyError::Unreadable(VERIFY_KEY_SETTING))?;
        let signing = match pem_setting(settings, SIGNING_KEY_SETTING) {
            Ok(pem) => Some(signing_key_from_pem(&pem).ok_or(KeyError::Unreadable(SIGNING_KEY_SETTING))?),
            Err(KeyError::Missing(_)) => None,
            Err(other) => return Err(other),
        };
        // Number(env ?? 75): blank is 0 and garbage NaN, both refused when a ticket is minted
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

    /// The transport's `p2p-admission-public.xy` for this key.
    pub fn public_xy(&self) -> PublicXy {
        public_xy(&self.verifying)
    }
}

/// The uncompressed point without its 0x04 tag: what the transport hands BCrypt as `BCRYPT_ECCPUBLIC_BLOB`.
pub fn public_xy(key: &VerifyingKey) -> PublicXy {
    let point = key.to_sec1_bytes();
    let mut xy = [0u8; 64];
    if let Some(coordinates) = point.get(1..65) {
        xy.copy_from_slice(coordinates);
    }
    PublicXy(xy)
}

impl GeneratedKeyPair {
    /// `metagame-admission.env`'s three lines, for `secrets.env` (the signing key) and the env file (the others).
    pub fn env_lines(&self) -> Secret {
        Secret::new(format!(
            "{KEY_ID_SETTING}={}\n{SIGNING_KEY_SETTING}={}\n{VERIFY_KEY_SETTING}={}\n",
            self.key_id.0,
            self.signing_key.expose(),
            self.verify_key
        ))
    }
}

/// The generator: a fresh key from the system's random source.
#[derive(Debug, Clone, Copy, Default)]
pub struct P256KeyGenerator;

impl P256KeyGenerator {
    /// The key pair and its PEMs: PKCS#8 and SPKI with LF line endings, as Node's `export` writes them.
    pub fn generate_with_pems(&self, key_id: KeyId) -> Result<(GeneratedKeyPair, Secret, String), KeyError> {
        if !safe_key_id(&key_id.0) {
            return Err(KeyError::KeyId);
        }
        let signing = SigningKey::try_generate_from_rng(&mut getrandom::SysRng).map_err(|_| KeyError::Random)?;
        let private_pem = signing
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|_| KeyError::Unreadable(SIGNING_KEY_SETTING))?;
        let verifying = *signing.verifying_key();
        let public_pem = verifying
            .to_public_key_pem(LineEnding::LF)
            .map_err(|_| KeyError::Unreadable(VERIFY_KEY_SETTING))?;
        let standard = base64::engine::general_purpose::STANDARD;
        let pair = GeneratedKeyPair {
            key_id,
            signing_key: Secret::new(standard.encode(private_pem.as_bytes())),
            verify_key: standard.encode(public_pem.as_bytes()),
            public_xy: public_xy(&verifying),
        };
        Ok((pair, Secret::new(private_pem.to_string()), public_pem))
    }
}

impl KeyGenerator for P256KeyGenerator {
    fn generate(&self, key_id: KeyId) -> Result<GeneratedKeyPair, AdmissionError> {
        Ok(self.generate_with_pems(key_id).map(|(pair, _, _)| pair)?)
    }
}

/// Writes one file; the private ones owner-only where the platform has modes.
fn write_file(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        if private {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// `generate-admission-key.mjs --output <dir> --key-id <id>`: a new folder with the four files, written to a
/// temporary folder beside it and renamed into place, so a failure leaves nothing half-written.
pub fn write_key_set(
    output: &Path,
    key_id: KeyId,
    process_id: u32,
    now_millis: i64,
) -> Result<GeneratedKeyPair, KeyError> {
    if !safe_key_id(&key_id.0) {
        return Err(KeyError::Usage);
    }
    let output = std::path::absolute(output).map_err(|error| KeyError::Io(error.to_string()))?;
    let parent = output.parent().map(Path::to_path_buf).unwrap_or_default();
    if !parent.is_dir() {
        return Err(KeyError::NoParent(parent));
    }
    if output.exists() {
        return Err(KeyError::Exists(output));
    }
    let temporary = parent.join(format!(".admission-key-{process_id}-{now_millis}"));
    fs::create_dir(&temporary).map_err(|error| KeyError::Io(error.to_string()))?;
    let written = (|| -> Result<GeneratedKeyPair, KeyError> {
        let (pair, private_pem, public_pem) = P256KeyGenerator.generate_with_pems(key_id)?;
        let io = |error: std::io::Error| KeyError::Io(error.to_string());
        write_file(&temporary.join(PRIVATE_PEM_FILE), private_pem.expose().as_bytes(), true).map_err(io)?;
        write_file(&temporary.join(PUBLIC_PEM_FILE), public_pem.as_bytes(), false).map_err(io)?;
        write_file(&temporary.join(PUBLIC_XY_FILE), &pair.public_xy.0, false).map_err(io)?;
        write_file(&temporary.join(ENV_FILE), pair.env_lines().expose().as_bytes(), true).map_err(io)?;
        fs::rename(&temporary, &output).map_err(io)?;
        Ok(pair)
    })();
    if written.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn settings_for(pair: &GeneratedKeyPair) -> HashMap<String, String> {
        [
            (KEY_ID_SETTING, pair.key_id.0.clone()),
            (SIGNING_KEY_SETTING, pair.signing_key.expose().to_owned()),
            (VERIFY_KEY_SETTING, pair.verify_key.clone()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }

    #[test]
    fn a_generated_pair_loads_back_and_its_xy_is_the_public_point() {
        let (pair, private_pem, public_pem) = P256KeyGenerator.generate_with_pems(KeyId("k1".to_owned())).unwrap();
        assert!(private_pem.expose().starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert!(public_pem.starts_with("-----BEGIN PUBLIC KEY-----\n"));
        let tickets = Es256Tickets::from_settings(&settings_for(&pair)).unwrap();
        assert_eq!(tickets.key_id, KeyId("k1".to_owned()));
        assert_eq!(tickets.public_xy(), pair.public_xy);
        assert_eq!(
            public_xy(tickets.signing.as_ref().unwrap().verifying_key()),
            pair.public_xy
        );
        assert_eq!(tickets.default_lifetime, 75.0);
        let lines = pair.env_lines();
        assert!(
            lines
                .expose()
                .starts_with("P2P_ADMISSION_KEY_ID=k1\nP2P_ADMISSION_SIGNING_PRIVKEY_B64=")
        );
        assert!(lines.expose().ends_with('\n'));
        assert!(!format!("{pair:?}{tickets:?}").contains(pair.signing_key.expose()));
    }

    #[test]
    fn settings_are_checked_as_node_checks_them() {
        let (pair, _, _) = P256KeyGenerator.generate_with_pems(KeyId("k1".to_owned())).unwrap();
        let mut settings = settings_for(&pair);
        settings.insert(KEY_ID_SETTING.to_owned(), "bad id".to_owned());
        assert_eq!(Es256Tickets::from_settings(&settings).unwrap_err(), KeyError::KeyId);
        let mut settings = settings_for(&pair);
        settings.remove(VERIFY_KEY_SETTING);
        assert_eq!(
            Es256Tickets::from_settings(&settings).unwrap_err(),
            KeyError::Missing(VERIFY_KEY_SETTING)
        );
        let mut settings = settings_for(&pair);
        settings.remove(SIGNING_KEY_SETTING);
        assert!(Es256Tickets::from_settings(&settings).unwrap().signing.is_none());
        let mut settings = settings_for(&pair);
        settings.insert(SIGNING_KEY_SETTING.to_owned(), "bm90IGEga2V5".to_owned());
        assert_eq!(
            Es256Tickets::from_settings(&settings).unwrap_err(),
            KeyError::Unreadable(SIGNING_KEY_SETTING)
        );
        let mut settings = settings_for(&pair);
        settings.insert(LIFETIME_SETTING.to_owned(), "".to_owned());
        assert_eq!(Es256Tickets::from_settings(&settings).unwrap().default_lifetime, 0.0);
        assert_eq!(
            P256KeyGenerator.generate(KeyId("../x".to_owned())).unwrap_err(),
            AdmissionError::Key(KeyError::KeyId)
        );
    }

    #[test]
    fn sec1_private_keys_and_wrapped_base64_load_too() {
        let (pair, private_pem, _) = P256KeyGenerator.generate_with_pems(KeyId("k1".to_owned())).unwrap();
        let key = signing_key_from_pem(private_pem.expose()).unwrap();
        let sec1 = p256::SecretKey::from(key.as_nonzero_scalar())
            .to_sec1_pem(LineEnding::LF)
            .unwrap();
        assert_eq!(signing_key_from_pem(&sec1).unwrap(), key);
        // Node's base64 decoder skips line breaks and stray characters
        let wrapped: String = pair
            .verify_key
            .as_bytes()
            .chunks(60)
            .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
            .collect::<Vec<_>>()
            .join("\r\n");
        let pem = String::from_utf8(lenient_base64(&wrapped).unwrap()).unwrap();
        assert!(verifying_key_from_pem(&pem).is_some());
    }

    #[test]
    fn the_key_set_is_written_once_and_never_over_an_existing_folder() {
        let base = std::env::temp_dir().join(format!("rifts-admission-keys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir(&base).unwrap();
        let output = base.join("keys");
        let pair = write_key_set(&output, KeyId("server-1".to_owned()), 7, 1).unwrap();
        assert_eq!(fs::read(output.join(PUBLIC_XY_FILE)).unwrap(), pair.public_xy.0);
        let env = fs::read_to_string(output.join(ENV_FILE)).unwrap();
        assert_eq!(env, pair.env_lines().expose());
        let private_pem = fs::read_to_string(output.join(PRIVATE_PEM_FILE)).unwrap();
        assert!(signing_key_from_pem(&private_pem).is_some());
        let public_pem = fs::read_to_string(output.join(PUBLIC_PEM_FILE)).unwrap();
        assert_eq!(public_xy(&verifying_key_from_pem(&public_pem).unwrap()), pair.public_xy);
        assert!(!base.join(".admission-key-7-1").exists());
        assert_eq!(
            write_key_set(&output, KeyId("server-1".to_owned()), 7, 2).unwrap_err(),
            KeyError::Exists(std::path::absolute(&output).unwrap())
        );
        assert!(matches!(
            write_key_set(&base.join("missing/keys"), KeyId("k".to_owned()), 7, 3),
            Err(KeyError::NoParent(_))
        ));
        assert_eq!(
            write_key_set(&base.join("x"), KeyId("".to_owned()), 7, 4).unwrap_err(),
            KeyError::Usage
        );
        fs::remove_dir_all(&base).unwrap();
    }
}
