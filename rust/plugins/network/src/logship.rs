//! A node's change log and database snapshots shipped to a peer region's node over `Service::LogShip`
//! (DD23 layer 2, DH7).
//!
//! [`PeerBucket`] implements [`history_ship::Bucket`] (write-only) over a [`PeerLink`], so the existing
//! [`history_ship::Shipper`] ships gzipped `history::record` segments (`head_hash` in the header, verified
//! against [`store::history::Chain`] before leaving) and gzipped database snapshots to a peer node exactly
//! as it ships to S3.
//!
//! [`PeerLogStore`] is the receiving side (`NodeService` for `Service::LogShip` + [`history_ship::BucketRead`]
//! for [`history_ship::restore_to`]):
//! - Accepts puts only from listed peers (`peers` in `[[network.nodes]]`).
//! - Enforces that the sender (`from`, authenticated by the node link's mTLS certificate) only writes under
//!   its own prefix (`"{from}/log/..."` or `"{from}/snapshot/..."`), that the key parses as a valid
//!   [`SegmentName`] or [`SnapshotName`] for that prefix, and that the SHA-256 of `body` matches the
//!   64-hex digest at the end of the key.
//! - Stores objects in a per-origin append-only folder (`root/<from>/log/...` and `root/<from>/snapshot/...`),
//!   never overwriting or deleting an existing file (if a re-shipped key already exists with the same
//!   content-addressed digest, the put is an idempotent success) and stamps each newly received object's
//!   arrival time (`last_modified`) from the receiver's own [`Clock`].
//! - Implements [`history_ship::BucketRead`] over `root`, so [`history_ship::restore_to`] can restore a
//!   destroyed node's database from its peer's folder with the hash chain and `trusted_head` verified.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::rand;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use history_ship::layout::{SegmentName, SnapshotName, sha256_hex};
use history_ship::{Bucket, BucketRead, Error as ShipError, ObjectInfo};
use module_api::BoxFuture;
use module_api::network::NodeId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::home::protocol::CleanText;
use crate::home::{KeyError, SigningKey};
use crate::link::{Clock, LinkError, MAX_MESSAGE, NodeService, PeerLink, Service};

pub const LOG_SHIP_VERSION: u32 = 1;

/// Magic header prepended to peer-shipped objects encrypted at rest (`DLENC1\0` + 12-byte random nonce + AES-256-GCM ciphertext + 16-byte tag).
pub const ENCRYPTED_MAGIC: &[u8; 7] = b"DLENC1\0";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const LOG_SHIP_KEY_CONTEXT: &[u8] = b"dainty-lass/log-ship-key/v1\n";

/// A node-owned 256-bit key used to encrypt peer-shipped history segments and database snapshots at rest (DH7)
/// so the receiving storage peer cannot read player history.
#[derive(Clone, PartialEq, Eq)]
pub struct LogShipKey([u8; 32]);

impl std::fmt::Debug for LogShipKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogShipKey")
            .field("kid", &self.kid())
            .finish_non_exhaustive()
    }
}

impl LogShipKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn generate() -> Result<Self, KeyError> {
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes).map_err(|_| KeyError::Random)?;
        Ok(Self(bytes))
    }

    /// Derives a deterministic, domain-separated 256-bit key from the node's private [`SigningKey`] (`session-key`).
    /// Peers hold only the node's public [`crate::home::VerifyingKey`] and cannot compute this key.
    pub fn from_signing_key(signing_key: &SigningKey) -> Self {
        let sig = signing_key.sign(LOG_SHIP_KEY_CONTEXT);
        let digest = Sha256::digest(sig);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }

    pub fn kid(&self) -> String {
        crate::home::keys::hex(&Sha256::digest(self.0)[..8])
    }

    /// Encrypts `plaintext` with AES-256-GCM bound to `object_key` as Additional Authenticated Data.
    pub fn encrypt(&self, object_key: &str, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let unbound = UnboundKey::new(&AES_256_GCM, &self.0).map_err(|_| "invalid AES-256-GCM key".to_owned())?;
        let key = LessSafeKey::new(unbound);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::fill(&mut nonce_bytes).map_err(|_| "random source failed".to_owned())?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut out = Vec::with_capacity(ENCRYPTED_MAGIC.len() + NONCE_LEN + plaintext.len() + TAG_LEN);
        out.extend_from_slice(ENCRYPTED_MAGIC);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(plaintext);
        let payload_start = ENCRYPTED_MAGIC.len() + NONCE_LEN;
        let tag = key
            .seal_in_place_separate_tag(nonce, Aad::from(object_key.as_bytes()), &mut out[payload_start..])
            .map_err(|_| "AES-256-GCM seal failed".to_owned())?;
        out.extend_from_slice(tag.as_ref());
        Ok(out)
    }

    /// Decrypts and authenticates an encrypted `DLENC1\0` object bound to `object_key`.
    pub fn decrypt(&self, object_key: &str, wire: &[u8]) -> Result<Vec<u8>, String> {
        if !is_encrypted_object(wire) {
            return Err("object does not have the DLENC1 encryption header".to_owned());
        }
        let min_len = ENCRYPTED_MAGIC.len() + NONCE_LEN + TAG_LEN;
        if wire.len() < min_len {
            return Err("encrypted object is truncated".to_owned());
        }
        let unbound = UnboundKey::new(&AES_256_GCM, &self.0).map_err(|_| "invalid AES-256-GCM key".to_owned())?;
        let key = LessSafeKey::new(unbound);
        let nonce_slice = &wire[ENCRYPTED_MAGIC.len()..ENCRYPTED_MAGIC.len() + NONCE_LEN];
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(nonce_slice);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut buf = wire[ENCRYPTED_MAGIC.len() + NONCE_LEN..].to_vec();
        let plaintext = key
            .open_in_place(nonce, Aad::from(object_key.as_bytes()), &mut buf)
            .map_err(|_| {
                "could not decrypt peer-shipped object (wrong LogShipKey or tampered ciphertext)".to_owned()
            })?;
        let len = plaintext.len();
        buf.truncate(len);
        Ok(buf)
    }
}

pub fn is_encrypted_object(bytes: &[u8]) -> bool {
    bytes.starts_with(ENCRYPTED_MAGIC)
}

/// Maximum raw object body (gzipped segment or snapshot) in one `Service::LogShip` message, leaving
/// room for base64 and JSON framing inside [`MAX_MESSAGE`].
pub const MAX_LOG_SHIP_OBJECT_BYTES: usize = 700 * 1024;

/// Maximum wire bytes of an encoded [`LogShipPutRequest`] or [`LogShipPutResponse`].
pub const MAX_LOG_SHIP_WIRE: usize = MAX_MESSAGE;

const META_EXT: &str = ".meta.json";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogShipDecodeError {
    #[error("log-ship message is {0} bytes; max is {MAX_LOG_SHIP_WIRE}")]
    TooLarge(usize),
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("invalid field: {0}")]
    Field(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogShipPutRequest {
    pub version: u32,
    pub key: String,
    pub content_type: String,
    /// Base64-encoded (standard alphabet) segment or snapshot bytes (encrypted `DLENC1\0...` envelope when shipped via [`PeerBucket`]).
    pub body_b64: String,
    /// SHA-256 of the encrypted wire payload when `body_b64` holds a `DLENC1\0` ciphertext envelope whose outer SHA-256 differs from the plaintext SHA-256 in `key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_sha256: Option<String>,
}

impl LogShipPutRequest {
    pub fn new(
        key: impl Into<String>,
        content_type: impl Into<String>,
        body: &[u8],
    ) -> Result<Self, LogShipDecodeError> {
        if body.len() > MAX_LOG_SHIP_OBJECT_BYTES {
            return Err(LogShipDecodeError::TooLarge(body.len()));
        }
        let wire_sha256 = is_encrypted_object(body).then(|| sha256_hex(body));
        let req = Self {
            version: LOG_SHIP_VERSION,
            key: key.into(),
            content_type: content_type.into(),
            body_b64: BASE64.encode(body),
            wire_sha256,
        };
        req.validate_fields()?;
        Ok(req)
    }

    pub fn decode_body(&self) -> Result<Vec<u8>, LogShipDecodeError> {
        let bytes = BASE64
            .decode(&self.body_b64)
            .map_err(|e| LogShipDecodeError::Field(format!("body_b64 is not base64: {e}")))?;
        if bytes.len() > MAX_LOG_SHIP_OBJECT_BYTES {
            return Err(LogShipDecodeError::TooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    fn validate_fields(&self) -> Result<(), LogShipDecodeError> {
        if self.key.is_empty() || self.key.len() > 256 || self.key.chars().any(char::is_control) {
            return Err(LogShipDecodeError::Field(
                "key must be 1..=256 printable characters".into(),
            ));
        }
        if self.content_type.is_empty()
            || self.content_type.len() > 64
            || self.content_type.chars().any(char::is_control)
        {
            return Err(LogShipDecodeError::Field(
                "content_type must be 1..=64 printable characters".into(),
            ));
        }
        if let Some(ref digest) = self.wire_sha256
            && (digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        {
            return Err(LogShipDecodeError::Field(
                "wire_sha256 must be 64 lowercase hex characters".into(),
            ));
        }
        let _ = self.decode_body()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "reason", deny_unknown_fields)]
pub enum LogShipRefusal {
    #[error("log-ship protocol {supported} is required")]
    UnsupportedVersion { supported: u32 },
    #[error("the log-ship message could not be read")]
    Malformed,
    #[error("the log-ship message is larger than the link allows")]
    TooLarge,
    #[error("the caller is not a peer this node lists")]
    UnknownPeer,
    #[error("a peer may only ship objects under its own prefix ({expected_prefix})")]
    WrongPrefix { expected_prefix: CleanText },
    #[error("the object key is not a valid segment or snapshot name")]
    InvalidKey,
    #[error("the object body does not match the SHA-256 in its key")]
    HashMismatch,
    #[error("could not store the shipped object: {message}")]
    Storage { message: CleanText },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", deny_unknown_fields)]
pub enum LogShipReply {
    Stored {
        /// True when the object was written now; false when the identical content-addressed object was already present.
        created: bool,
        received_at: i64,
    },
    Refused {
        refusal: LogShipRefusal,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogShipPutResponse {
    pub version: u32,
    pub reply: LogShipReply,
}

impl LogShipPutResponse {
    pub fn stored(created: bool, received_at: i64) -> Self {
        Self {
            version: LOG_SHIP_VERSION,
            reply: LogShipReply::Stored { created, received_at },
        }
    }

    pub fn refused(refusal: LogShipRefusal) -> Self {
        Self {
            version: LOG_SHIP_VERSION,
            reply: LogShipReply::Refused { refusal },
        }
    }
}

pub fn encode_put_request(request: &LogShipPutRequest) -> Result<Vec<u8>, LogShipDecodeError> {
    request.validate_fields()?;
    let bytes = serde_json::to_vec(request).map_err(|e| LogShipDecodeError::Json(e.to_string()))?;
    if bytes.len() > MAX_LOG_SHIP_WIRE {
        return Err(LogShipDecodeError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

pub fn decode_put_request(bytes: &[u8]) -> Result<LogShipPutRequest, LogShipDecodeError> {
    if bytes.len() > MAX_LOG_SHIP_WIRE {
        return Err(LogShipDecodeError::TooLarge(bytes.len()));
    }
    let request: LogShipPutRequest =
        serde_json::from_slice(bytes).map_err(|e| LogShipDecodeError::Json(e.to_string()))?;
    request.validate_fields()?;
    Ok(request)
}

pub fn encode_put_response(response: &LogShipPutResponse) -> Result<Vec<u8>, LogShipDecodeError> {
    let bytes = serde_json::to_vec(response).map_err(|e| LogShipDecodeError::Json(e.to_string()))?;
    if bytes.len() > MAX_LOG_SHIP_WIRE {
        return Err(LogShipDecodeError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

pub fn decode_put_response(bytes: &[u8]) -> Result<LogShipPutResponse, LogShipDecodeError> {
    if bytes.len() > MAX_LOG_SHIP_WIRE {
        return Err(LogShipDecodeError::TooLarge(bytes.len()));
    }
    serde_json::from_slice(bytes).map_err(|e| LogShipDecodeError::Json(e.to_string()))
}

/// Checks that `key` starts with `"{origin}/"` and is a valid [`SegmentName`] or [`SnapshotName`] under
/// that prefix, returning `(relative_subpath, expected_sha256)`.
pub fn parse_peer_object_key(origin: &NodeId, key: &str) -> Option<(String, String)> {
    let prefix = format!("{origin}/");
    if let Some(seg) = SegmentName::parse(&prefix, key) {
        let rel = key.strip_prefix(&prefix)?.to_owned();
        return Some((rel, seg.sha256));
    }
    if let Some(snap) = SnapshotName::parse(&prefix, key) {
        let rel = key.strip_prefix(&prefix)?.to_owned();
        return Some((rel, snap.sha256));
    }
    None
}

/// Write-only [`Bucket`] that ships segments and snapshots to a peer node over `Service::LogShip` (DH7),
/// encrypted at rest when given a [`LogShipKey`].
#[derive(Clone)]
pub struct PeerBucket {
    peer: NodeId,
    link: Arc<dyn PeerLink>,
    key: Option<LogShipKey>,
}

impl PeerBucket {
    /// Ships plaintext: a restore after losing the node needs nothing that died with it (DH7).
    pub fn new(peer: NodeId, link: Arc<dyn PeerLink>) -> Self {
        Self { peer, link, key: None }
    }

    /// Ships ciphertext only `key` can read: whoever restores must hold a copy of `key` kept off the node.
    pub fn with_key(peer: NodeId, link: Arc<dyn PeerLink>, key: LogShipKey) -> Self {
        Self {
            peer,
            link,
            key: Some(key),
        }
    }

    pub fn peer(&self) -> &NodeId {
        &self.peer
    }

    pub fn key(&self) -> Option<&LogShipKey> {
        self.key.as_ref()
    }

    /// The required `ShipSettings::prefix` for `origin` when shipping to a peer (`"<origin>/"`).
    pub fn prefix_for(origin: &NodeId) -> String {
        format!("{origin}/")
    }
}

impl Bucket for PeerBucket {
    fn put(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &'static str,
    ) -> impl Future<Output = Result<(), ShipError>> + Send {
        let peer = self.peer.clone();
        let link = self.link.clone();
        let enc_key = self.key.clone();
        let key = key.to_owned();
        async move {
            let encrypted = match enc_key {
                Some(enc_key) if !is_encrypted_object(&body) => {
                    // The peer can't check a ciphertext against the key's digest, so check the plaintext here
                    if let Some((origin_str, _)) = key.split_once('/')
                        && let Ok(origin) = origin_str.parse::<NodeId>()
                        && let Some((_, expected_sha)) = parse_peer_object_key(&origin, &key)
                        && sha256_hex(&body) != expected_sha
                    {
                        return Err(ShipError::Bucket(format!(
                            "peer {peer} refused put: {}",
                            LogShipRefusal::HashMismatch
                        )));
                    }
                    enc_key
                        .encrypt(&key, &body)
                        .map_err(|e| ShipError::Bucket(format!("peer {peer}: {e}")))?
                }
                _ => body,
            };
            let request = LogShipPutRequest::new(key, content_type, &encrypted)
                .map_err(|e| ShipError::Bucket(format!("peer {peer}: {e}")))?;
            let wire = encode_put_request(&request).map_err(|e| ShipError::Bucket(format!("peer {peer}: {e}")))?;
            let answer = link
                .call(&peer, Service::LogShip, wire)
                .await
                .map_err(|e| ShipError::Bucket(format!("peer {peer}: {e}")))?;
            let response = decode_put_response(&answer)
                .map_err(|e| ShipError::Bucket(format!("peer {peer}: invalid response: {e}")))?;
            if response.version != LOG_SHIP_VERSION {
                return Err(ShipError::Bucket(format!(
                    "peer {peer}: unsupported version {}",
                    response.version
                )));
            }
            match response.reply {
                LogShipReply::Stored { .. } => Ok(()),
                LogShipReply::Refused { refusal } => {
                    Err(ShipError::Bucket(format!("peer {peer} refused put: {refusal}")))
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObjectMeta {
    received_at: i64,
    size: u64,
}

/// Receiving side for `Service::LogShip` and read-only [`BucketRead`] over the per-origin append-only folder.
///
/// On a storage peer, [`PeerLogStore::new`] holds no decryption key (`decryption_key: None`) so the peer stores
/// only `DLENC1\0` ciphertext and cannot read player history. When restoring a destroyed node from the peer's
/// folder, attach the origin node's [`LogShipKey`] via [`PeerLogStore::with_decryption_key`].
#[derive(Clone)]
pub struct PeerLogStore {
    root: PathBuf,
    peers: Arc<BTreeSet<NodeId>>,
    clock: Arc<Clock>,
    write_lock: Arc<Mutex<()>>,
    decryption_key: Option<LogShipKey>,
}

impl PeerLogStore {
    pub fn new(root: impl Into<PathBuf>, peers: impl IntoIterator<Item = NodeId>, clock: Arc<Clock>) -> Self {
        Self {
            root: root.into(),
            peers: Arc::new(peers.into_iter().collect()),
            clock,
            write_lock: Arc::new(Mutex::new(())),
            decryption_key: None,
        }
    }

    /// Returns a view of this store configured with the origin node's [`LogShipKey`] so [`history_ship::restore_to`]
    /// can decrypt and authenticate objects shipped by that node.
    #[must_use]
    pub fn with_decryption_key(&self, key: LogShipKey) -> Self {
        let mut cloned = self.clone();
        cloned.decryption_key = Some(key);
        cloned
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The filesystem directory where objects from `origin` are stored (`<root>/<origin>`).
    pub fn origin_dir(&self, origin: &NodeId) -> PathBuf {
        self.root.join(origin.as_str())
    }

    /// Reads the raw bytes stored on disk for `key` without decrypting (for verifying ciphertext-at-rest or re-shipping).
    pub fn get_raw(&self, key: &str) -> Result<Vec<u8>, ShipError> {
        let path = self.resolve_key_path(key)?;
        std::fs::read(&path).map_err(|_| ShipError::Bucket(format!("no object {key}")))
    }

    fn is_peer(&self, node: &NodeId) -> bool {
        self.peers.contains(node)
    }

    fn store_object(
        &self,
        from: &NodeId,
        key: &str,
        body: &[u8],
        wire_sha256: Option<&str>,
    ) -> Result<(bool, i64), LogShipRefusal> {
        let Some((rel, expected_sha)) = parse_peer_object_key(from, key) else {
            let prefix = format!("{from}/");
            if !key.starts_with(&prefix) {
                return Err(LogShipRefusal::WrongPrefix {
                    expected_prefix: CleanText::lossy(&prefix),
                });
            }
            return Err(LogShipRefusal::InvalidKey);
        };
        if is_encrypted_object(body) {
            let min_len = ENCRYPTED_MAGIC.len() + NONCE_LEN + TAG_LEN;
            if body.len() < min_len || wire_sha256 != Some(sha256_hex(body).as_str()) {
                return Err(LogShipRefusal::HashMismatch);
            }
        } else if sha256_hex(body) != expected_sha {
            return Err(LogShipRefusal::HashMismatch);
        }

        // Prevent any path traversal: `rel` is verified by SegmentName/SnapshotName to be
        // `log/<19d>-<19d>-<64hex>.jsonl.gz` or `snapshot/<13d>-<19d>-<64hex>.<kind>.gz`.
        let (subdir, filename) = rel.split_once('/').ok_or(LogShipRefusal::InvalidKey)?;
        if !matches!(subdir, "log" | "snapshot")
            || filename.contains('/')
            || filename.contains('\\')
            || filename.contains("..")
        {
            return Err(LogShipRefusal::InvalidKey);
        }

        let _guard = self.write_lock.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = self.root.join(from.as_str()).join(subdir);
        std::fs::create_dir_all(&dir).map_err(|e| LogShipRefusal::Storage {
            message: CleanText::lossy(&e.to_string()),
        })?;
        let data_path = dir.join(filename);
        let meta_path = dir.join(format!("{filename}{META_EXT}"));

        // Append-only: never overwrite or delete an existing object. Because names end with the
        // body's SHA-256 and we just verified `sha256_hex(body) == expected_sha`, an existing file
        // with the same name holds identical bytes.
        if data_path.exists() {
            let received_at = Self::read_meta(&meta_path, &data_path)
                .map(|m| m.received_at)
                .unwrap_or_else(|| self.clock.now_ms());
            return Ok((false, received_at));
        }

        let received_at = self.clock.now_ms();
        let tmp_data = dir.join(format!(".{filename}.tmp"));
        std::fs::write(&tmp_data, body).map_err(|e| LogShipRefusal::Storage {
            message: CleanText::lossy(&e.to_string()),
        })?;
        std::fs::rename(&tmp_data, &data_path).map_err(|e| LogShipRefusal::Storage {
            message: CleanText::lossy(&e.to_string()),
        })?;

        let meta = ObjectMeta {
            received_at,
            size: body.len() as u64,
        };
        if let Ok(json) = serde_json::to_vec(&meta) {
            let tmp_meta = dir.join(format!(".{filename}{META_EXT}.tmp"));
            if std::fs::write(&tmp_meta, json).is_ok() {
                let _ = std::fs::rename(&tmp_meta, &meta_path);
            }
        }
        Ok((true, received_at))
    }

    fn read_meta(meta_path: &Path, data_path: &Path) -> Option<ObjectMeta> {
        if let Ok(bytes) = std::fs::read(meta_path)
            && let Ok(meta) = serde_json::from_slice::<ObjectMeta>(&bytes)
        {
            return Some(meta);
        }
        let stat = std::fs::metadata(data_path).ok()?;
        let received_at = stat
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        Some(ObjectMeta {
            received_at,
            size: stat.len(),
        })
    }

    fn handle_wire(&self, from: &NodeId, body: &[u8]) -> LogShipPutResponse {
        if !self.is_peer(from) {
            return LogShipPutResponse::refused(LogShipRefusal::UnknownPeer);
        }
        if body.len() > MAX_LOG_SHIP_WIRE {
            return LogShipPutResponse::refused(LogShipRefusal::TooLarge);
        }
        let request = match decode_put_request(body) {
            Ok(req) => req,
            Err(LogShipDecodeError::TooLarge(_)) => {
                return LogShipPutResponse::refused(LogShipRefusal::TooLarge);
            }
            Err(_) => {
                return LogShipPutResponse::refused(LogShipRefusal::Malformed);
            }
        };
        if request.version != LOG_SHIP_VERSION {
            return LogShipPutResponse::refused(LogShipRefusal::UnsupportedVersion {
                supported: LOG_SHIP_VERSION,
            });
        }
        let Ok(raw_body) = request.decode_body() else {
            return LogShipPutResponse::refused(LogShipRefusal::Malformed);
        };
        match self.store_object(from, &request.key, &raw_body, request.wire_sha256.as_deref()) {
            Ok((created, received_at)) => LogShipPutResponse::stored(created, received_at),
            Err(refusal) => LogShipPutResponse::refused(refusal),
        }
    }

    fn resolve_key_path(&self, key: &str) -> Result<PathBuf, ShipError> {
        let mut parts = key.split('/');
        let origin_str = parts
            .next()
            .ok_or_else(|| ShipError::Bucket(format!("no object {key}")))?;
        let subdir = parts
            .next()
            .ok_or_else(|| ShipError::Bucket(format!("no object {key}")))?;
        let filename = parts
            .next()
            .ok_or_else(|| ShipError::Bucket(format!("no object {key}")))?;
        if parts.next().is_some()
            || origin_str.parse::<NodeId>().is_err()
            || !matches!(subdir, "log" | "snapshot")
            || filename.is_empty()
            || filename.ends_with(META_EXT)
            || filename.starts_with('.')
            || filename.contains('\\')
            || filename.contains("..")
        {
            return Err(ShipError::Bucket(format!("no object {key}")));
        }
        Ok(self.root.join(origin_str).join(subdir).join(filename))
    }

    fn collect_objects(&self) -> Result<BTreeMap<String, ObjectInfo>, ShipError> {
        let mut out = BTreeMap::new();
        let origins = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(ShipError::Bucket(e.to_string())),
        };
        for origin_entry in origins.flatten() {
            let Ok(ft) = origin_entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let Some(origin_name) = origin_entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if origin_name.parse::<NodeId>().is_err() {
                continue;
            }
            for subdir in ["log", "snapshot"] {
                let dir = origin_entry.path().join(subdir);
                let Ok(files) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for file in files.flatten() {
                    let Some(fname) = file.file_name().to_str().map(str::to_owned) else {
                        continue;
                    };
                    if fname.starts_with('.') || fname.ends_with(META_EXT) {
                        continue;
                    }
                    let data_path = file.path();
                    let meta_path = dir.join(format!("{fname}{META_EXT}"));
                    let Some(meta) = Self::read_meta(&meta_path, &data_path) else {
                        continue;
                    };
                    let key = format!("{origin_name}/{subdir}/{fname}");
                    out.insert(
                        key.clone(),
                        ObjectInfo {
                            key,
                            last_modified: meta.received_at,
                            size: meta.size,
                        },
                    );
                }
            }
        }
        Ok(out)
    }
}

impl NodeService for PeerLogStore {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::LogShip {
                return Err(LinkError::NoService {
                    node: from.clone(),
                    service,
                });
            }
            let response = self.handle_wire(from, &body);
            encode_put_response(&response).map_err(|e| LinkError::Protocol {
                node: from.clone(),
                reason: e.to_string(),
            })
        })
    }
}

impl BucketRead for PeerLogStore {
    async fn list(&self, prefix: &str, start_after: Option<&str>) -> Result<Vec<ObjectInfo>, ShipError> {
        let all = self.collect_objects()?;
        Ok(all
            .into_iter()
            .filter(|(key, _)| key.starts_with(prefix) && start_after.is_none_or(|s| key.as_str() > s))
            .map(|(_, info)| info)
            .collect())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, ShipError> {
        let bytes = self.get_raw(key)?;
        if is_encrypted_object(&bytes) {
            let dec_key = self.decryption_key.as_ref().ok_or_else(|| ShipError::Object {
                key: key.to_owned(),
                reason: "peer-shipped object is encrypted at rest; attach the origin node's LogShipKey with with_decryption_key() to restore".to_owned(),
            })?;
            dec_key.decrypt(key, &bytes).map_err(|reason| ShipError::Object {
                key: key.to_owned(),
                reason,
            })
        } else {
            Ok(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn wire_round_trip_and_bounds() {
        let req = LogShipPutRequest::new(
            format!(
                "oregon-1/log/0000000000000000000-0000000000000000001-{}.jsonl.gz",
                "a".repeat(64)
            ),
            "application/gzip",
            b"hello-world",
        )
        .unwrap();
        let encoded = encode_put_request(&req).unwrap();
        let decoded = decode_put_request(&encoded).unwrap();
        assert_eq!(decoded, req);
        assert_eq!(decoded.decode_body().unwrap(), b"hello-world");

        let stored = LogShipPutResponse::stored(true, 1_761_500_000_000);
        let stored_wire = encode_put_response(&stored).unwrap();
        assert_eq!(decode_put_response(&stored_wire).unwrap(), stored);

        let refused = LogShipPutResponse::refused(LogShipRefusal::HashMismatch);
        let refused_wire = encode_put_response(&refused).unwrap();
        assert_eq!(decode_put_response(&refused_wire).unwrap(), refused);

        let oversized = vec![0u8; MAX_LOG_SHIP_OBJECT_BYTES + 1];
        assert!(matches!(
            LogShipPutRequest::new("oregon-1/log/x", "application/gzip", &oversized),
            Err(LogShipDecodeError::TooLarge(_))
        ));
    }

    #[test]
    fn peer_object_keys_enforce_origin_prefix_and_layout() {
        let oregon: NodeId = "oregon-1".parse().unwrap();
        let digest = "b".repeat(64);
        let valid_seg = format!("oregon-1/log/0000000000000000000-0000000000000000001-{digest}.jsonl.gz");
        let (rel, sha) = parse_peer_object_key(&oregon, &valid_seg).unwrap();
        assert!(rel.starts_with("log/"));
        assert_eq!(sha, digest);

        let valid_snap = format!("oregon-1/snapshot/1761500000000-0000000000000000001-{digest}.sqlite.gz");
        let (rel_snap, sha_snap) = parse_peer_object_key(&oregon, &valid_snap).unwrap();
        assert!(rel_snap.starts_with("snapshot/"));
        assert_eq!(sha_snap, digest);

        assert!(
            parse_peer_object_key(
                &oregon,
                &format!("uk-1/log/0000000000000000000-0000000000000000001-{digest}.jsonl.gz")
            )
            .is_none()
        );
        assert!(parse_peer_object_key(&oregon, "oregon-1/log/../../etc/passwd").is_none());
    }
}
