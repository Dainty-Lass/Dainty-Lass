// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The signed join ticket: `rpt1.<payload>.<signature>`, an ES256 signature over a fixed big-endian payload. The
//! host's transport parses the same bytes, so the layout and every refusal are a contract.

use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use rand::TryRng as _;
use rifts_relay::wire::{format_uuid, hex};
use rifts_seam::{AccountId, PeerId, Secret, UnixMillis};
use sha2::{Digest as _, Sha256};

use crate::{AdmissionError, KeyId, TicketAuthority};

#[cfg(test)]
mod tests;

/// Umbrella spec D3: `rpt1`, lower case although the payload says `RPT2`.
pub const TOKEN_PREFIX: &str = "rpt1";
pub const PAYLOAD_MAGIC: &[u8; 4] = b"RPT2";
pub const PAYLOAD_VERSION: u8 = 2;
pub const FIXED_BYTES: usize = 106;
pub const MAX_TOKEN_BYTES: usize = 4096;
pub const MIN_LIFETIME_SECONDS: u32 = 15;
pub const MAX_LIFETIME_SECONDS: u64 = 180;
pub const CLOCK_SKEW_SECONDS: u64 = 30;
pub const DEFAULT_LIFETIME_SECONDS: u32 = 75;

// JavaScript's Number.MAX_SAFE_INTEGER: Node holds the revision as a number.
const MAX_PARTY_REVISION: u64 = (1 << 53) - 1;
const MAX_VIRTUAL_PORT: u16 = 999;
const SIGNATURE_BYTES: usize = 64;

/// The texts are compared with Node's messages by the recorded vectors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TicketError {
    #[error("admission ticket contains an unsafe identifier")]
    UnsafeIdentifier,
    #[error("{0} is outside its unsigned range")]
    OutOfRange(&'static str),
    #[error("virtualPort must be a nonzero Steam logical port")]
    ZeroVirtualPort,
    #[error("invalid SteamID64")]
    InvalidPeerId,
    #[error("sessionId must be a canonical UUID")]
    SessionNotUuid,
    #[error("P2P admission ticket lifetime must be between 15 and 180 seconds")]
    LifetimeSetting,
    #[error("admission token exceeds size limit")]
    TokenTooLarge,
    #[error("admission token is missing or oversized")]
    MissingOrOversized,
    #[error("malformed admission token")]
    MalformedToken,
    #[error("noncanonical admission token encoding")]
    NoncanonicalEncoding,
    #[error("invalid admission token signature")]
    BadSignature,
    #[error("malformed admission ticket payload")]
    MalformedPayload,
    #[error("invalid admission ticket framing")]
    Framing,
    #[error("noncanonical admission ticket payload")]
    NoncanonicalPayload,
    #[error("admission token key is not active")]
    KeyNotActive,
    #[error("invalid admission token lifetime")]
    InvalidLifetime,
    #[error("admission token is not yet valid")]
    NotYetValid,
    #[error("admission token expired")]
    Expired,
    #[error("invalid admission JTI")]
    InvalidJti,
    #[error("Missing required P2P admission key P2P_ADMISSION_SIGNING_PRIVKEY_B64.")]
    NoSigningKey,
    #[error("the system random source failed")]
    Random,
}

impl TicketError {
    pub fn is_expired(&self) -> bool {
        matches!(self, Self::Expired)
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_16(text: &[u8]) -> Option<[u8; 16]> {
    if text.len() != 32 {
        return None;
    }
    let mut out = [0_u8; 16];
    for (slot, [high, low]) in out.iter_mut().zip(text.as_chunks::<2>().0) {
        *slot = (hex_value(*high)? << 4) | hex_value(*low)?;
    }
    Some(out)
}

/// A hunt's session as the ticket carries it: an RFC 4122 UUID of version 1 to 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionUuid(pub [u8; 16]);

impl SessionUuid {
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, TicketError> {
        let version = bytes[6] >> 4;
        let variant = bytes[8] >> 4;
        if (1..=5).contains(&version) && (0x8..=0xb).contains(&variant) {
            Ok(Self(bytes))
        } else {
            Err(TicketError::SessionNotUuid)
        }
    }
}

impl FromStr for SessionUuid {
    type Err = TicketError;

    fn from_str(text: &str) -> Result<Self, TicketError> {
        let bytes = text.as_bytes();
        let dashes_placed = bytes.len() == 36 && [8, 13, 18, 23].iter().all(|&at| bytes[at] == b'-');
        if !dashes_placed {
            return Err(TicketError::SessionNotUuid);
        }
        let digits: Vec<u8> = bytes.iter().copied().filter(|&b| b != b'-').collect();
        Self::from_bytes(hex_16(&digits).ok_or(TicketError::SessionNotUuid)?)
    }
}

impl fmt::Display for SessionUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_uuid(&self.0))
    }
}

/// A ticket's one-time id. Only its digest is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Jti(pub [u8; 16]);

impl Jti {
    pub fn random() -> Result<Self, TicketError> {
        let mut bytes = [0_u8; 16];
        rand::rngs::SysRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| TicketError::Random)?;
        Ok(Self(bytes))
    }

    pub fn parse(text: &str) -> Result<Self, TicketError> {
        hex_16(text.as_bytes()).map(Self).ok_or(TicketError::InvalidJti)
    }

    /// Also the peer admission's id.
    pub fn digest(&self) -> String {
        hex(&Sha256::digest(self.0))
    }
}

impl fmt::Display for Jti {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

pub fn hash_jti(jti: &str) -> Result<String, TicketError> {
    Jti::parse(jti).map(|jti| jti.digest())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketBindings {
    pub account: AccountId,
    pub joiner_peer: PeerId,
    pub host_peer: PeerId,
    pub session: SessionUuid,
    pub generation: u32,
    pub party_id: String,
    pub party_revision: u64,
    pub hunt_definition_id: String,
    pub build: u32,
    pub bridge_protocol: u16,
    pub virtual_port: u16,
}

/// Times are Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketClaims {
    pub key_id: KeyId,
    pub jti: Jti,
    pub bindings: TicketBindings,
    pub issued_at: u64,
    pub not_before: u64,
    pub expires_at: u64,
}

impl TicketClaims {
    pub fn jti_digest(&self) -> String {
        self.jti.digest()
    }
}

fn identifier(text: &str, max: usize, punctuation: &[u8]) -> bool {
    (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || punctuation.contains(&byte))
}

pub fn safe_key_id(text: &str) -> bool {
    identifier(text, 32, b"._-")
}

fn check_claims(claims: &TicketClaims) -> Result<(), TicketError> {
    let bindings = &claims.bindings;
    let identifiers_safe = safe_key_id(&claims.key_id.0)
        && identifier(&bindings.account.0, 96, b"._:-")
        && identifier(&bindings.hunt_definition_id, 128, b"._:/-")
        && identifier(&bindings.party_id, 192, b"._:+/=-");
    if !identifiers_safe {
        return Err(TicketError::UnsafeIdentifier);
    }
    if bindings.virtual_port > MAX_VIRTUAL_PORT {
        return Err(TicketError::OutOfRange("virtualPort"));
    }
    if bindings.virtual_port == 0 {
        return Err(TicketError::ZeroVirtualPort);
    }
    if bindings.party_revision > MAX_PARTY_REVISION {
        return Err(TicketError::OutOfRange("partyRevision"));
    }
    if bindings.joiner_peer.0 == 0 || bindings.host_peer.0 == 0 {
        return Err(TicketError::InvalidPeerId);
    }
    SessionUuid::from_bytes(bindings.session.0).map(drop)
}

/// The payload bytes, after the same checks the transport makes.
pub fn encode_claims(claims: &TicketClaims) -> Result<Vec<u8>, TicketError> {
    check_claims(claims)?;
    let bindings = &claims.bindings;
    let key = claims.key_id.0.as_bytes();
    let account = bindings.account.0.as_bytes();
    let hunt = bindings.hunt_definition_id.as_bytes();
    let party = bindings.party_id.as_bytes();
    // The checks bound every length (32, 96, 128, 192), so none of these narrowings can fail.
    let lengths = (
        u8::try_from(key.len()),
        u16::try_from(account.len()),
        u16::try_from(hunt.len()),
        u16::try_from(party.len()),
    );
    let (Ok(key_len), Ok(account_len), Ok(hunt_len), Ok(party_len)) = lengths else {
        return Err(TicketError::UnsafeIdentifier);
    };

    let mut out = Vec::with_capacity(FIXED_BYTES + key.len() + account.len() + hunt.len() + party.len());
    out.extend_from_slice(PAYLOAD_MAGIC);
    out.push(PAYLOAD_VERSION);
    out.push(key_len);
    out.extend_from_slice(&account_len.to_be_bytes());
    out.extend_from_slice(&hunt_len.to_be_bytes());
    out.extend_from_slice(&party_len.to_be_bytes());
    out.extend_from_slice(&bindings.bridge_protocol.to_be_bytes());
    out.extend_from_slice(&bindings.virtual_port.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&bindings.generation.to_be_bytes());
    out.extend_from_slice(&bindings.build.to_be_bytes());
    out.extend_from_slice(&bindings.party_revision.to_be_bytes());
    out.extend_from_slice(&claims.issued_at.to_be_bytes());
    out.extend_from_slice(&claims.not_before.to_be_bytes());
    out.extend_from_slice(&claims.expires_at.to_be_bytes());
    out.extend_from_slice(&bindings.joiner_peer.0.to_be_bytes());
    out.extend_from_slice(&bindings.host_peer.0.to_be_bytes());
    out.extend_from_slice(&bindings.session.0);
    out.extend_from_slice(&claims.jti.0);
    out.extend_from_slice(key);
    out.extend_from_slice(account);
    out.extend_from_slice(hunt);
    out.extend_from_slice(party);
    Ok(out)
}

/// Reads big-endian fields at fixed offsets of a slice already known to hold them.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn array<const N: usize>(&self, at: usize) -> [u8; N] {
        let mut out = [0_u8; N];
        if let Some(bytes) = self.0.get(at..at + N) {
            out.copy_from_slice(bytes);
        }
        out
    }

    fn u16(&self, at: usize) -> u16 {
        u16::from_be_bytes(self.array(at))
    }

    fn u32(&self, at: usize) -> u32 {
        u32::from_be_bytes(self.array(at))
    }

    fn u64(&self, at: usize) -> u64 {
        u64::from_be_bytes(self.array(at))
    }
}

/// Node holds times as numbers, so a time with no exact double cannot have come from it.
fn exact_double(value: u64) -> bool {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let back = {
        let float = value as f64;
        // 2^64 itself casts back to u64::MAX, which would compare equal.
        (float < 18_446_744_073_709_551_616.0).then_some(float as u64)
    };
    back == Some(value)
}

/// The claims of a payload, accepted only in the one canonical encoding.
pub fn decode_claims(payload: &[u8]) -> Result<TicketClaims, TicketError> {
    if payload.len() < FIXED_BYTES || &payload[..4] != PAYLOAD_MAGIC {
        return Err(TicketError::MalformedPayload);
    }
    let fields = Fields(payload);
    let key_len = usize::from(payload[5]);
    let account_len = usize::from(fields.u16(6));
    let hunt_len = usize::from(fields.u16(8));
    let party_len = usize::from(fields.u16(10));
    let lengths = [key_len, account_len, hunt_len, party_len];
    if payload[4] != PAYLOAD_VERSION
        || fields.u16(16) != 0
        || lengths.contains(&0)
        || payload.len() != FIXED_BYTES + lengths.iter().sum::<usize>()
    {
        return Err(TicketError::Framing);
    }

    let mut at = FIXED_BYTES;
    let mut text = |len: usize| {
        let value = String::from_utf8_lossy(&payload[at..at + len]).into_owned();
        at += len;
        value
    };
    let key_id = text(key_len);
    let account = text(account_len);
    let hunt_definition_id = text(hunt_len);
    let party_id = text(party_len);

    let claims = TicketClaims {
        key_id: KeyId(key_id),
        jti: Jti(fields.array(90)),
        bindings: TicketBindings {
            account: AccountId(account),
            joiner_peer: PeerId(fields.u64(58)),
            host_peer: PeerId(fields.u64(66)),
            session: SessionUuid(fields.array(74)),
            generation: fields.u32(18),
            party_id,
            party_revision: fields.u64(26),
            hunt_definition_id,
            build: fields.u32(22),
            bridge_protocol: fields.u16(12),
            virtual_port: fields.u16(14),
        },
        issued_at: fields.u64(34),
        not_before: fields.u64(42),
        expires_at: fields.u64(50),
    };
    let encoded = encode_claims(&claims)?;
    let times_exact = [claims.issued_at, claims.not_before, claims.expires_at]
        .into_iter()
        .all(exact_double);
    if encoded != payload || !times_exact {
        return Err(TicketError::NoncanonicalPayload);
    }
    Ok(claims)
}

/// Unpadded base64url that re-encodes to the same text: no `=`, no `+` or `/`, no stray trailing bits.
fn canonical_base64url(text: &str) -> Result<Vec<u8>, TicketError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| TicketError::NoncanonicalEncoding)?;
    if URL_SAFE_NO_PAD.encode(&bytes) != text {
        return Err(TicketError::NoncanonicalEncoding);
    }
    Ok(bytes)
}

/// Splits and decodes a token, and calls `verify` on the raw payload before reading it.
pub fn parse_signed(token: &str, verify: impl FnOnce(&[u8], &Signature) -> bool) -> Result<TicketClaims, TicketError> {
    if token.len() > MAX_TOKEN_BYTES {
        return Err(TicketError::MissingOrOversized);
    }
    let parts: Vec<&str> = token.split('.').collect();
    let [prefix, payload, signature] = parts.as_slice() else {
        return Err(TicketError::MalformedToken);
    };
    if *prefix != TOKEN_PREFIX || payload.is_empty() || signature.is_empty() {
        return Err(TicketError::MalformedToken);
    }
    let payload = canonical_base64url(payload)?;
    let signature = canonical_base64url(signature)?;
    if signature.len() != SIGNATURE_BYTES {
        return Err(TicketError::NoncanonicalEncoding);
    }
    // Refuses r or s of zero or not below the curve order.
    let signature = Signature::from_slice(&signature).map_err(|_| TicketError::BadSignature)?;
    if !verify(&payload, &signature) {
        return Err(TicketError::BadSignature);
    }
    decode_claims(&payload)
}

pub fn check_lifetime(claims: &TicketClaims, now_seconds: u64) -> Result<(), TicketError> {
    if claims.expires_at < claims.issued_at
        || claims.not_before < claims.issued_at
        || claims.expires_at - claims.issued_at > MAX_LIFETIME_SECONDS
    {
        return Err(TicketError::InvalidLifetime);
    }
    let latest_start = now_seconds.saturating_add(CLOCK_SKEW_SECONDS);
    if claims.issued_at > latest_start || claims.not_before > latest_start {
        return Err(TicketError::NotYetValid);
    }
    if now_seconds > claims.expires_at.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err(TicketError::Expired);
    }
    Ok(())
}

/// Framing and payload checks only; the signature is not checked.
pub fn parse_unverified(token: &str) -> Result<TicketClaims, TicketError> {
    parse_signed(token, |_, _| true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedTicket {
    pub text: Secret,
    pub claims: TicketClaims,
    pub jti_digest: String,
}

/// The ES256 ticket authority: verify-only when it has no signing key.
#[derive(Clone)]
pub struct Es256Tickets {
    pub key_id: KeyId,
    pub signing: Option<SigningKey>,
    pub verifying: VerifyingKey,
    /// A setting's `Number()`, checked only when a ticket is minted.
    pub default_lifetime: f64,
}

impl fmt::Debug for Es256Tickets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Es256Tickets")
            .field("key_id", &self.key_id)
            .field("signing", &self.signing.as_ref().map(|_| "<redacted>"))
            .field("default_lifetime", &self.default_lifetime)
            .finish()
    }
}

impl Es256Tickets {
    pub fn mint(
        &self,
        bindings: TicketBindings,
        now_seconds: u64,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, TicketError> {
        let lifetime = lifetime_seconds.map_or(self.default_lifetime, f64::from);
        let whole_in_range = lifetime.fract() == 0.0
            && lifetime >= f64::from(MIN_LIFETIME_SECONDS)
            && lifetime <= f64::from(MAX_LIFETIME_SECONDS as u32);
        if !whole_in_range {
            return Err(TicketError::LifetimeSetting);
        }
        // Whole and within 15..=180, so exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lifetime = lifetime as u64;
        let claims = TicketClaims {
            key_id: self.key_id.clone(),
            jti: Jti::random()?,
            bindings,
            issued_at: now_seconds,
            not_before: now_seconds,
            expires_at: now_seconds.saturating_add(lifetime),
        };
        self.sign(claims)
    }

    pub fn sign(&self, claims: TicketClaims) -> Result<MintedTicket, TicketError> {
        let payload = encode_claims(&claims)?;
        let signing = self.signing.as_ref().ok_or(TicketError::NoSigningKey)?;
        let signature: Signature = signing.try_sign(&payload).map_err(|_| TicketError::BadSignature)?;
        let text = format!(
            "{TOKEN_PREFIX}.{}.{}",
            URL_SAFE_NO_PAD.encode(&payload),
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        );
        if text.len() > MAX_TOKEN_BYTES {
            return Err(TicketError::TokenTooLarge);
        }
        Ok(MintedTicket {
            text: Secret::new(text),
            jti_digest: claims.jti_digest(),
            claims,
        })
    }

    /// High-S signatures pass: Node and the transport accept them, so a low-S rule here would split the two.
    pub fn verify(&self, token: &str, now_seconds: u64) -> Result<TicketClaims, TicketError> {
        let claims = parse_signed(token, |payload, signature| {
            self.verifying.verify(payload, signature).is_ok()
        })?;
        if claims.key_id != self.key_id {
            return Err(TicketError::KeyNotActive);
        }
        check_lifetime(&claims, now_seconds)?;
        Ok(claims)
    }
}

fn seconds(now: UnixMillis) -> u64 {
    u64::try_from(now.0.div_euclid(1000)).unwrap_or(0)
}

impl TicketAuthority for Es256Tickets {
    fn key_id(&self) -> KeyId {
        self.key_id.clone()
    }

    fn mint(
        &self,
        bindings: TicketBindings,
        now: UnixMillis,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, AdmissionError> {
        Ok(Self::mint(self, bindings, seconds(now), lifetime_seconds)?)
    }

    fn verify(&self, ticket: &str, now: UnixMillis) -> Result<TicketClaims, AdmissionError> {
        Ok(Self::verify(self, ticket, seconds(now))?)
    }
}
