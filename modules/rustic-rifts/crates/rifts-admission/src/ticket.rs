//! MPT2 admission tickets, byte for byte as `p2p/security/admissionTickets.ts` writes them and the transport reads
//! them (`src/admission_ticket.cpp`): `mpt1.<payload>.<signature>`, base64url without padding, an ES256 signature in
//! IEEE P1363 form (r || s) over the payload.
//!
//! The payload is 106 fixed bytes then the four identifiers: "MPT2", version 2, the key id's length (u8), the
//! account's, hunt's and party's lengths (u16), bridge protocol, virtual port, a zero u16, generation and build (u32),
//! party revision, issued at, not before, expires at, joiner and host peer ids (u64), the session UUID and the jti
//! (16 bytes each), then key id, account, hunt definition and party id. Big-endian throughout.

use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use rifts_seam::{AccountId, PeerId, Secret};
use sha2::Digest;

use crate::KeyId;

pub const TOKEN_PREFIX: &str = "mpt1";
pub const PAYLOAD_MAGIC: &[u8; 4] = b"MPT2";
pub const PAYLOAD_VERSION: u8 = 2;
pub const FIXED_BYTES: usize = 106;
pub const MAX_TOKEN_BYTES: usize = 4096;
pub const MIN_LIFETIME_SECONDS: u32 = 15;
pub const MAX_LIFETIME_SECONDS: u64 = 180;
pub const CLOCK_SKEW_SECONDS: u64 = 30;
/// `P2P_ADMISSION_TICKET_TTL_SECONDS`' default.
pub const DEFAULT_LIFETIME_SECONDS: u32 = 75;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Node's error messages, so a log reads the same; consume answers `P2P_TICKET_EXPIRED` for [`TicketError::Expired`]
/// and `P2P_TICKET_INVALID` for the rest.
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
    /// Node's test is `message.includes("expired")`, which only this message passes.
    pub fn is_expired(&self) -> bool {
        matches!(self, Self::Expired)
    }
}

/// A session id as the ticket carries it: a canonical RFC 4122 UUID, version 1 to 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionUuid(pub [u8; 16]);

impl SessionUuid {
    /// `UUID` in admissionTickets.ts: version nibble 1 to 5, variant 8 to b.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, TicketError> {
        let version = bytes[6] >> 4;
        let variant = bytes[8] >> 4;
        if (1..=5).contains(&version) && (8..=0xb).contains(&variant) {
            Ok(Self(bytes))
        } else {
            Err(TicketError::SessionNotUuid)
        }
    }
}

impl FromStr for SessionUuid {
    type Err = TicketError;

    /// Either case, as the regex is `/i`; written back lower case.
    fn from_str(text: &str) -> Result<Self, TicketError> {
        let bytes = text.as_bytes();
        let dashes_ok = bytes.len() == 36 && [8, 13, 18, 23].iter().all(|&at| bytes[at] == b'-');
        if !dashes_ok {
            return Err(TicketError::SessionNotUuid);
        }
        let hex: Vec<u8> = bytes.iter().copied().filter(|&b| b != b'-').collect();
        let parsed = parse_hex::<16>(&hex).ok_or(TicketError::SessionNotUuid)?;
        Self::from_bytes(parsed)
    }
}

impl fmt::Display for SessionUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rifts_relay::wire::format_uuid(&self.0))
    }
}

/// The ticket's 128-bit id; only its SHA-256 is stored ([`hash_jti`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Jti(pub [u8; 16]);

impl Jti {
    pub fn random() -> Result<Self, TicketError> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| TicketError::Random)?;
        Ok(Self(bytes))
    }

    /// `^[a-f0-9]{32}$/i`.
    pub fn parse(text: &str) -> Result<Self, TicketError> {
        parse_hex::<16>(text.as_bytes())
            .map(Self)
            .ok_or(TicketError::InvalidJti)
    }

    /// `HashAdmissionJti`: SHA-256 of the 16 bytes, lower-case hex. Also the peer admission's id.
    pub fn digest(&self) -> String {
        rifts_relay::wire::hex(&sha2::Sha256::digest(self.0))
    }
}

impl fmt::Display for Jti {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rifts_relay::wire::hex(&self.0))
    }
}

/// `HashAdmissionJti` from the hex text.
pub fn hash_jti(jti: &str) -> Result<String, TicketError> {
    Jti::parse(jti).map(|jti| jti.digest())
}

fn parse_hex<const N: usize>(text: &[u8]) -> Option<[u8; N]> {
    if text.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(text.as_chunks::<2>().0) {
        let digit = |b: u8| char::from(b).to_digit(16);
        *slot = u8::try_from(digit(pair[0])? * 16 + digit(pair[1])?).ok()?;
    }
    Some(out)
}

/// What a ticket binds the joiner to: everything but the key, the jti and the times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketBindings {
    pub account: AccountId,
    /// `joinerSteamId`: the joiner's transport peer id (nonzero).
    pub joiner_peer: PeerId,
    /// `hostSteamId`.
    pub host_peer: PeerId,
    pub session: SessionUuid,
    pub generation: u32,
    pub party_id: String,
    /// At most 2^53 - 1, a JavaScript safe integer.
    pub party_revision: u64,
    pub hunt_definition_id: String,
    pub build: u32,
    pub bridge_protocol: u16,
    /// 1 to 999, the Steam logical port range the transport keeps.
    pub virtual_port: u16,
}

/// The claims of one MPT2 ticket (`AdmissionTicketClaims`; issuer, audience, purpose and version are constants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketClaims {
    pub key_id: KeyId,
    pub jti: Jti,
    pub bindings: TicketBindings,
    /// Unix seconds.
    pub issued_at: u64,
    pub not_before: u64,
    pub expires_at: u64,
}

impl TicketClaims {
    pub fn jti_digest(&self) -> String {
        self.jti.digest()
    }
}

fn safe(text: &str, max: usize, extra: &[u8]) -> bool {
    (1..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
}

/// `SAFE_KEY`: `^[A-Za-z0-9._-]{1,32}$`.
pub fn safe_key_id(text: &str) -> bool {
    safe(text, 32, b"._-")
}

/// `EncodeClaims`' checks, in its order, so the first failure is Node's.
fn check(claims: &TicketClaims) -> Result<(), TicketError> {
    let bindings = &claims.bindings;
    let identifiers = safe_key_id(&claims.key_id.0)
        && safe(&bindings.account.0, 96, b"._:-")
        && safe(&bindings.hunt_definition_id, 128, b"._:/-")
        && safe(&bindings.party_id, 192, b"._:+/=-");
    if !identifiers {
        return Err(TicketError::UnsafeIdentifier);
    }
    if bindings.virtual_port > 999 {
        return Err(TicketError::OutOfRange("virtualPort"));
    }
    if bindings.virtual_port == 0 {
        return Err(TicketError::ZeroVirtualPort);
    }
    if bindings.party_revision > MAX_SAFE_INTEGER {
        return Err(TicketError::OutOfRange("partyRevision"));
    }
    if bindings.joiner_peer.0 == 0 || bindings.host_peer.0 == 0 {
        return Err(TicketError::InvalidPeerId);
    }
    SessionUuid::from_bytes(bindings.session.0)?;
    Ok(())
}

/// The payload of a ticket after [`check`].
pub fn encode_claims(claims: &TicketClaims) -> Result<Vec<u8>, TicketError> {
    check(claims)?;
    let bindings = &claims.bindings;
    let key = claims.key_id.0.as_bytes();
    let account = bindings.account.0.as_bytes();
    let hunt = bindings.hunt_definition_id.as_bytes();
    let party = bindings.party_id.as_bytes();
    let mut payload = Vec::with_capacity(FIXED_BYTES + key.len() + account.len() + hunt.len() + party.len());
    payload.extend_from_slice(PAYLOAD_MAGIC);
    payload.push(PAYLOAD_VERSION);
    // The lengths fit: check() bounds them at 32, 96, 128 and 192
    for (length, wide) in [
        (key.len(), false),
        (account.len(), true),
        (hunt.len(), true),
        (party.len(), true),
    ] {
        if wide {
            payload.extend_from_slice(&(length as u16).to_be_bytes());
        } else {
            payload.push(length as u8);
        }
    }
    payload.extend_from_slice(&bindings.bridge_protocol.to_be_bytes());
    payload.extend_from_slice(&bindings.virtual_port.to_be_bytes());
    payload.extend_from_slice(&0u16.to_be_bytes());
    payload.extend_from_slice(&bindings.generation.to_be_bytes());
    payload.extend_from_slice(&bindings.build.to_be_bytes());
    for value in [
        bindings.party_revision,
        claims.issued_at,
        claims.not_before,
        claims.expires_at,
        bindings.joiner_peer.0,
        bindings.host_peer.0,
    ] {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    payload.extend_from_slice(&bindings.session.0);
    payload.extend_from_slice(&claims.jti.0);
    for part in [key, account, hunt, party] {
        payload.extend_from_slice(part);
    }
    Ok(payload)
}

fn be<const N: usize>(payload: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    if let Some(slice) = payload.get(at..at + N) {
        out.copy_from_slice(slice);
    }
    out
}

/// Node reads the u64 times through `Number`, so a value with no exact double fails its re-encoding check.
fn exact_in_double(value: u64) -> bool {
    let double = value as f64;
    double < 18_446_744_073_709_551_616.0 && double as u64 == value
}

/// `DecodeClaims`: the framing, then every identifier and range check by re-encoding, so only the canonical payload
/// for its claims passes.
pub fn decode_claims(payload: &[u8]) -> Result<TicketClaims, TicketError> {
    if payload.len() < FIXED_BYTES || &payload[..4] != PAYLOAD_MAGIC {
        return Err(TicketError::MalformedPayload);
    }
    let version = payload[4];
    let key_length = usize::from(payload[5]);
    let account_length = usize::from(u16::from_be_bytes(be(payload, 6)));
    let hunt_length = usize::from(u16::from_be_bytes(be(payload, 8)));
    let party_length = usize::from(u16::from_be_bytes(be(payload, 10)));
    let reserved = u16::from_be_bytes(be(payload, 16));
    let expected = FIXED_BYTES + key_length + account_length + hunt_length + party_length;
    if version != PAYLOAD_VERSION
        || reserved != 0
        || payload.len() != expected
        || key_length == 0
        || account_length == 0
        || hunt_length == 0
        || party_length == 0
    {
        return Err(TicketError::Framing);
    }
    let u64_at = |at: usize| u64::from_be_bytes(be(payload, at));
    let mut session = [0u8; 16];
    session.copy_from_slice(&payload[74..90]);
    let mut jti = [0u8; 16];
    jti.copy_from_slice(&payload[90..106]);
    let mut offset = FIXED_BYTES;
    let mut text = |length: usize| {
        let bytes = &payload[offset..offset + length];
        offset += length;
        // Node decodes UTF-8 with replacement characters, which the identifier check then refuses
        String::from_utf8_lossy(bytes).into_owned()
    };
    let key_id = text(key_length);
    let account = text(account_length);
    let hunt_definition_id = text(hunt_length);
    let party_id = text(party_length);
    let party_revision = u64_at(26);
    let claims = TicketClaims {
        key_id: KeyId(key_id),
        jti: Jti(jti),
        bindings: TicketBindings {
            account: AccountId(account),
            joiner_peer: PeerId(u64_at(58)),
            host_peer: PeerId(u64_at(66)),
            // Checked with the rest by encode_claims below
            session: SessionUuid(session),
            generation: u32::from_be_bytes(be(payload, 18)),
            party_id,
            party_revision,
            hunt_definition_id,
            build: u32::from_be_bytes(be(payload, 22)),
            bridge_protocol: u16::from_be_bytes(be(payload, 12)),
            virtual_port: u16::from_be_bytes(be(payload, 14)),
        },
        issued_at: u64_at(34),
        not_before: u64_at(42),
        expires_at: u64_at(50),
    };
    // A party revision past 2^53 - 1 is out of range (AssertUnsigned) before any other re-encoding difference
    let reencoded = encode_claims(&claims)?;
    let times_exact = [claims.issued_at, claims.not_before, claims.expires_at]
        .into_iter()
        .all(exact_in_double);
    if reencoded != payload || !times_exact {
        return Err(TicketError::NoncanonicalPayload);
    }
    Ok(claims)
}

fn base64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Canonical unpadded base64url only, as Node's `decoded.toString("base64url") !== part` refuses anything else.
fn canonical_base64url(text: &str) -> Option<Vec<u8>> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).ok()?;
    (base64url(&bytes) == text).then_some(bytes)
}

/// Signs and checks MPT2 tickets with one admission key (`P2P_ADMISSION_*`).
#[derive(Clone)]
pub struct Es256Tickets {
    pub key_id: KeyId,
    pub signing: Option<SigningKey>,
    pub verifying: VerifyingKey,
    /// `P2P_ADMISSION_TICKET_TTL_SECONDS` when it is a whole number; checked at minting, as Node does.
    pub default_lifetime: f64,
}

impl fmt::Debug for Es256Tickets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Es256Tickets")
            .field("key_id", &self.key_id)
            .field("signing", &self.signing.as_ref().map(|_| "<redacted>"))
            .field("default_lifetime", &self.default_lifetime)
            .finish_non_exhaustive()
    }
}

/// A freshly signed ticket and the digest that is stored instead of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedTicket {
    /// The ticket text written to `ticket.secret`: a bearer credential until consumed.
    pub text: Secret,
    pub claims: TicketClaims,
    pub jti_digest: String,
}

impl Es256Tickets {
    /// `IssueAdmissionTicket`. `lifetime_seconds` is the caller's (the join paths pass 180), else the setting.
    pub fn mint(
        &self,
        bindings: TicketBindings,
        now_seconds: u64,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, TicketError> {
        let lifetime = lifetime_seconds.map_or(self.default_lifetime, f64::from);
        if lifetime.fract() != 0.0
            || !(f64::from(MIN_LIFETIME_SECONDS)..=MAX_LIFETIME_SECONDS as f64).contains(&lifetime)
        {
            return Err(TicketError::LifetimeSetting);
        }
        let claims = TicketClaims {
            key_id: self.key_id.clone(),
            jti: Jti::random()?,
            bindings,
            issued_at: now_seconds,
            not_before: now_seconds,
            // A whole number from 15 to 180 by the check above
            expires_at: now_seconds + lifetime as u64,
        };
        self.sign(claims)
    }

    /// Signs claims as they are: for [`Self::mint`], test vectors and the minting tool.
    pub fn sign(&self, claims: TicketClaims) -> Result<MintedTicket, TicketError> {
        let payload = encode_claims(&claims)?;
        let signing = self.signing.as_ref().ok_or(TicketError::NoSigningKey)?;
        let signature: Signature = signing.sign(&payload);
        let token = format!(
            "{TOKEN_PREFIX}.{}.{}",
            base64url(&payload),
            base64url(&signature.to_bytes())
        );
        if token.len() > MAX_TOKEN_BYTES {
            return Err(TicketError::TokenTooLarge);
        }
        Ok(MintedTicket {
            text: Secret::new(token),
            jti_digest: claims.jti_digest(),
            claims,
        })
    }

    /// `VerifyAdmissionTicket`: encoding, signature, framing, key, lifetime and clock. Single use is the store's.
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

/// The token's checks before the key: size, shape, canonical encoding, the signature by `verify`, then the payload.
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
    let payload = canonical_base64url(payload).ok_or(TicketError::NoncanonicalEncoding)?;
    let signature = canonical_base64url(signature)
        .filter(|bytes| bytes.len() == 64)
        .ok_or(TicketError::NoncanonicalEncoding)?;
    // OpenSSL answers false for r or s out of range, as from_slice refuses them
    let signature = Signature::from_slice(&signature).map_err(|_| TicketError::BadSignature)?;
    if !verify(&payload, &signature) {
        return Err(TicketError::BadSignature);
    }
    decode_claims(&payload)
}

/// The lifetime and clock checks, with Node's 30 seconds of skew.
pub fn check_lifetime(claims: &TicketClaims, now_seconds: u64) -> Result<(), TicketError> {
    if claims.expires_at < claims.issued_at
        || claims.not_before < claims.issued_at
        || claims.expires_at - claims.issued_at > MAX_LIFETIME_SECONDS
    {
        return Err(TicketError::InvalidLifetime);
    }
    let latest = now_seconds.saturating_add(CLOCK_SKEW_SECONDS);
    if claims.issued_at > latest || claims.not_before > latest {
        return Err(TicketError::NotYetValid);
    }
    if now_seconds > claims.expires_at.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err(TicketError::Expired);
    }
    Ok(())
}

/// The payload of a ticket without checking anything but its framing: what a fuzz target and a log may read.
pub fn parse_unverified(token: &str) -> Result<TicketClaims, TicketError> {
    parse_signed(token, |_, _| true)
}

#[cfg(test)]
mod tests;
