//! Signing keys for session tickets (DN5): one Ed25519 key per node, which the node holds and nobody else
//! does, and the public keys of the other nodes, learned over node links.
//!
//! A node's private key lives in one file next to its other secrets, `dlk1.<64 hex>`, made on first start and
//! never logged (`Debug` shows the key id only). Its public key is not secret; peers fetch it over the
//! mutually authenticated link, so a key learned from node X was told by X, and no third node can supply one.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{PoisonError, RwLock};

use aws_lc_rs::rand;
use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dainty_module_api::network::NodeId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Ed25519 public keys and signatures are these long.
pub const PUBLIC_KEY_LEN: usize = 32;
/// An Ed25519 signature's length in bytes.
pub const SIGNATURE_LEN: usize = 64;
/// Keys one node may have at once: its current one and the ones still verifying tickets made before a rotation.
pub const MAX_KEYS_PER_NODE: usize = 4;
/// Nodes whose keys a ring holds, a cluster being at most this many.
pub const MAX_NODES: usize = 64;
const FILE_PREFIX: &str = "dlk1.";

/// Why a ticket signing key could not be made, read or saved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The file is not a `dlk1.` key.
    #[error("the signing key file is not a ticket key")]
    NotAKey,
    /// The key file is readable by other users; holds its path.
    #[error("the signing key file {0} is readable by other users; make it private to the server's account")]
    Exposed(String),
    /// Reading or writing the file failed.
    #[error("{0}")]
    Io(String),
    /// The system's random source failed.
    #[error("the system's random source failed")]
    Random,
}

/// A key's id: the first 8 bytes of the SHA-256 of its public key, in 16 lowercase hex characters.
fn key_id(public: &[u8]) -> String {
    hex::encode(&Sha256::digest(public)[..8])
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    // Lower case only, as the keys are written
    if text.bytes().any(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let mut out = [0u8; N];
    hex::decode_to_slice(text, &mut out).ok()?;
    Some(out)
}

/// Random bytes from the system, in hex.
pub(crate) fn random_hex<const N: usize>() -> Result<String, KeyError> {
    let mut bytes = [0u8; N];
    rand::fill(&mut bytes).map_err(|_| KeyError::Random)?;
    Ok(hex::encode(bytes))
}

/// A node's private signing key.
pub struct SigningKey {
    pair: Ed25519KeyPair,
    public: VerifyingKey,
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKey")
            .field("kid", &self.public.kid)
            .finish_non_exhaustive()
    }
}

impl SigningKey {
    fn from_seed(seed: &[u8; 32]) -> Result<Self, KeyError> {
        let pair = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| KeyError::NotAKey)?;
        let public = VerifyingKey::from_bytes(pair.public_key().as_ref()).ok_or(KeyError::NotAKey)?;
        Ok(Self { pair, public })
    }

    /// A new random key, not saved.
    pub fn generate() -> Result<Self, KeyError> {
        let mut seed = [0u8; 32];
        rand::fill(&mut seed).map_err(|_| KeyError::Random)?;
        Self::from_seed(&seed)
    }

    /// The key in `path`, made there first if the file isn't. On Unix a file other users can read is refused.
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    let mode = std::fs::metadata(path)
                        .map_err(|e| KeyError::Io(e.to_string()))?
                        .permissions()
                        .mode();
                    if mode & 0o077 != 0 {
                        return Err(KeyError::Exposed(path.display().to_string()));
                    }
                }
                let seed = text
                    .trim()
                    .strip_prefix(FILE_PREFIX)
                    .and_then(unhex::<32>)
                    .ok_or(KeyError::NotAKey)?;
                Self::from_seed(&seed)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut seed = [0u8; 32];
                rand::fill(&mut seed).map_err(|_| KeyError::Random)?;
                let key = Self::from_seed(&seed)?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt as _;
                    options.mode(0o600);
                }
                let mut file = options.open(path).map_err(|e| KeyError::Io(e.to_string()))?;
                std::io::Write::write_all(&mut file, format!("{FILE_PREFIX}{}\n", hex::encode(seed)).as_bytes())
                    .map_err(|e| KeyError::Io(e.to_string()))?;
                Ok(key)
            }
            Err(error) => Err(KeyError::Io(error.to_string())),
        }
    }

    /// The public half.
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.public
    }

    /// The key's id: 16 lowercase hex characters of its public key's SHA-256.
    pub fn kid(&self) -> &str {
        &self.public.kid
    }

    /// An Ed25519 signature of `message`.
    pub fn sign(&self, message: &[u8]) -> [u8; SIGNATURE_LEN] {
        let mut out = [0u8; SIGNATURE_LEN];
        out.copy_from_slice(self.pair.sign(message).as_ref());
        out
    }
}

/// A node's public key, as it crosses a link: its id and the key in base64url.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "WireKey", into = "WireKey")]
pub struct VerifyingKey {
    kid: String,
    bytes: [u8; PUBLIC_KEY_LEN],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireKey {
    kid: String,
    key: String,
}

impl VerifyingKey {
    /// The key in `bytes`, `None` unless they are 32 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes: [u8; PUBLIC_KEY_LEN] = bytes.try_into().ok()?;
        Some(Self {
            kid: key_id(&bytes),
            bytes,
        })
    }

    /// The key's id: 16 lowercase hex characters of its SHA-256.
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The raw Ed25519 public key.
    pub fn as_bytes(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.bytes
    }

    /// Whether `signature` is this key's Ed25519 signature of `message`.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        UnparsedPublicKey::new(&ED25519, &self.bytes)
            .verify(message, signature)
            .is_ok()
    }
}

impl TryFrom<WireKey> for VerifyingKey {
    type Error = String;

    fn try_from(wire: WireKey) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(&wire.key)
            .map_err(|_| "not a public key".to_owned())?;
        let key = Self::from_bytes(&bytes).ok_or("not a public key")?;
        // The id is derived, never taken from the sender
        if key.kid != wire.kid || URL_SAFE_NO_PAD.encode(key.bytes) != wire.key {
            return Err("the key id is not the key's".to_owned());
        }
        Ok(key)
    }
}

impl From<VerifyingKey> for WireKey {
    fn from(key: VerifyingKey) -> Self {
        Self {
            key: URL_SAFE_NO_PAD.encode(key.bytes),
            kid: key.kid,
        }
    }
}

/// The public keys of the cluster's nodes, as each told them to this node over its own link.
#[derive(Debug, Default)]
pub struct KeyRing {
    keys: RwLock<HashMap<NodeId, Vec<VerifyingKey>>>,
}

impl KeyRing {
    /// Replaces what is known of `node`'s keys with `keys` (at most [`MAX_KEYS_PER_NODE`], newest first). A
    /// ring never grows past [`MAX_NODES`] nodes.
    pub fn replace(&self, node: &NodeId, keys: Vec<VerifyingKey>) -> bool {
        if keys.is_empty() || keys.len() > MAX_KEYS_PER_NODE {
            return false;
        }
        let mut map = self.keys.write().unwrap_or_else(PoisonError::into_inner);
        if !map.contains_key(node) && map.len() >= MAX_NODES {
            return false;
        }
        map.insert(node.clone(), keys);
        true
    }

    /// `node`'s key with id `kid`, if known.
    pub fn find(&self, node: &NodeId, kid: &str) -> Option<VerifyingKey> {
        let map = self.keys.read().unwrap_or_else(PoisonError::into_inner);
        map.get(node)?.iter().find(|key| key.kid == kid).cloned()
    }

    /// Whether any key of `node` is known.
    pub fn knows(&self, node: &NodeId) -> bool {
        self.keys
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(node)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn a_key_is_made_once_kept_in_a_file_and_never_shown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ticket.key");
        let first = SigningKey::load_or_create(&path).unwrap();
        let again = SigningKey::load_or_create(&path).unwrap();
        assert_eq!(first.kid(), again.kid());
        assert_eq!(first.verifying_key(), again.verifying_key());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("dlk1.") && text.trim().len() == 5 + 64);
        let seed = text.trim().trim_start_matches("dlk1.");
        assert!(!format!("{first:?}").contains(seed));
        assert_eq!(first.kid().len(), 16);
        std::fs::write(&path, "dlk1.nothex\n").unwrap();
        assert_eq!(SigningKey::load_or_create(&path).unwrap_err(), KeyError::NotAKey);
    }

    #[test]
    fn a_signature_verifies_only_for_its_key_and_message() {
        let (a, b) = (SigningKey::generate().unwrap(), SigningKey::generate().unwrap());
        let sig = a.sign(b"hello");
        assert!(a.verifying_key().verify(b"hello", &sig));
        assert!(!a.verifying_key().verify(b"hellp", &sig));
        assert!(!b.verifying_key().verify(b"hello", &sig));
        assert!(!a.verifying_key().verify(b"hello", &sig[..63]));
    }

    #[test]
    fn a_public_key_crosses_a_link_with_its_derived_id_only() {
        let key = SigningKey::generate().unwrap();
        let json = serde_json::to_string(key.verifying_key()).unwrap();
        assert_eq!(
            serde_json::from_str::<VerifyingKey>(&json).unwrap(),
            *key.verifying_key()
        );
        let other = SigningKey::generate().unwrap();
        let forged = json.replace(key.kid(), other.kid());
        assert!(serde_json::from_str::<VerifyingKey>(&forged).is_err());
        assert!(serde_json::from_str::<VerifyingKey>(r#"{"kid":"00","key":"AA"}"#).is_err());
        assert!(serde_json::from_str::<VerifyingKey>(&json.replace('}', r#","x":1}"#)).is_err());
    }

    #[test]
    fn a_ring_holds_what_each_node_told_it() {
        let ring = KeyRing::default();
        let node: NodeId = "uk-1".parse().unwrap();
        let key = SigningKey::generate().unwrap();
        assert!(!ring.knows(&node));
        assert!(!ring.replace(&node, Vec::new()));
        assert!(ring.replace(&node, vec![key.verifying_key().clone()]));
        assert_eq!(ring.find(&node, key.kid()).unwrap(), *key.verifying_key());
        assert!(ring.find(&"oregon-1".parse().unwrap(), key.kid()).is_none());
        assert!(ring.find(&node, "0000000000000000").is_none());
    }
}
