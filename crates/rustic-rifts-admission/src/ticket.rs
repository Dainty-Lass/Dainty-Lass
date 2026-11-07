// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The signed join ticket: `rpt1.<payload>.<signature>`, an ES256 signature over a fixed big-endian payload. The
//! host's transport parses the same bytes, so the layout and every refusal are a contract.

use std::{fmt, str::FromStr};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64URL};
use p256::ecdsa::{
    Signature, SigningKey, VerifyingKey,
    signature::{Signer as _, Verifier as _},
};
use rand::TryRng as _;
use rustic_rifts_relay::wire;
use rustic_rifts_seam::{AccountId, PeerId, Secret, UnixMillis};
use sha2::Digest as _;

use crate::{AdmissionError, KeyId, TicketAuthority};

#[cfg(test)]
mod tests;

/// Umbrella spec D3: `rpt1`, lower case although the payload says `RPT2`.
pub const TOKEN_PREFIX: &str = "rpt1";
/// The magic the signed payload starts with.
pub const PAYLOAD_MAGIC: &[u8; 4] = b"RPT2";
/// The payload format version.
pub const PAYLOAD_VERSION: u8 = 2;
/// The payload's fixed-size part, in bytes, before its variable-length identifiers.
pub const FIXED_BYTES: usize = 106;
/// The longest ticket text accepted or minted, in bytes.
pub const MAX_TOKEN_BYTES: usize = 4096;
/// The shortest ticket lifetime allowed, in seconds.
pub const MIN_LIFETIME_SECONDS: u32 = 15;
/// The longest ticket lifetime allowed, in seconds.
pub const MAX_LIFETIME_SECONDS: u64 = 180;
/// How far either clock may be off when a ticket's times are checked, in seconds.
pub const CLOCK_SKEW_SECONDS: u64 = 30;
/// The ticket lifetime when the setting is absent, in seconds.
pub const DEFAULT_LIFETIME_SECONDS: u32 = 75;

/// The texts are compared with Node's messages by the recorded vectors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TicketError {
    /// A key id, account or other identifier has characters or a length a ticket refuses.
    UnsafeIdentifier,
    /// The named field does not fit its unsigned range.
    OutOfRange(&'static str),
    /// The virtual port is 0.
    ZeroVirtualPort,
    /// A peer id is not a valid SteamID64.
    InvalidPeerId,
    /// The session is not a canonical RFC 4122 UUID.
    SessionNotUuid,
    /// The lifetime is not whole seconds from 15 to 180.
    LifetimeSetting,
    /// A minted ticket would exceed 4096 bytes.
    TokenTooLarge,
    /// The ticket text is missing or over 4096 bytes.
    MissingOrOversized,
    /// The text is not `rpt1.<payload>.<signature>`.
    MalformedToken,
    /// The base64url is not in its canonical form.
    NoncanonicalEncoding,
    /// The signature does not verify.
    BadSignature,
    /// The payload could not be read.
    MalformedPayload,
    /// The payload's magic, version or lengths are wrong.
    Framing,
    /// The payload decodes but is not in its canonical form.
    NoncanonicalPayload,
    /// The ticket names a key other than the active one.
    KeyNotActive,
    /// The ticket's times are out of order or span more than 180 seconds.
    InvalidLifetime,
    /// The ticket's start is in the future, beyond the allowed skew.
    NotYetValid,
    /// The ticket expired, beyond the allowed skew.
    Expired,
    /// The jti is not 32 hex digits.
    InvalidJti,
    /// Minting needs the signing key, which is not set.
    NoSigningKey,
    /// The system random source failed.
    Random,
}

impl TicketError {
    /// Whether this is [`TicketError::Expired`].
    pub fn is_expired(&self) -> bool {
        *self == Self::Expired
    }

    fn text(&self) -> &'static str {
        use TicketError as E;
        match self {
            E::UnsafeIdentifier => "admission ticket contains an unsafe identifier",
            E::OutOfRange(_) => " is outside its unsigned range",
            E::ZeroVirtualPort => "virtualPort must be a nonzero Steam logical port",
            E::InvalidPeerId => "invalid SteamID64",
            E::SessionNotUuid => "sessionId must be a canonical UUID",
            E::LifetimeSetting => "P2P admission ticket lifetime must be between 15 and 180 seconds",
            E::TokenTooLarge => "admission token exceeds size limit",
            E::MissingOrOversized => "admission token is missing or oversized",
            E::MalformedToken => "malformed admission token",
            E::NoncanonicalEncoding => "noncanonical admission token encoding",
            E::BadSignature => "invalid admission token signature",
            E::MalformedPayload => "malformed admission ticket payload",
            E::Framing => "invalid admission ticket framing",
            E::NoncanonicalPayload => "noncanonical admission ticket payload",
            E::KeyNotActive => "admission token key is not active",
            E::InvalidLifetime => "invalid admission token lifetime",
            E::NotYetValid => "admission token is not yet valid",
            E::Expired => "admission token expired",
            E::InvalidJti => "invalid admission JTI",
            E::NoSigningKey => "Missing required P2P admission key P2P_ADMISSION_SIGNING_PRIVKEY_B64.",
            E::Random => "the system random source failed",
        }
    }
}

impl fmt::Display for TicketError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::OutOfRange(field) = self {
            out.write_str(field)?;
        }
        out.write_str(self.text())
    }
}

impl std::error::Error for TicketError {}

/// Exactly 32 hex digits of either case, as 16 bytes.
fn sixteen_from_hex(text: &[u8]) -> Option<[u8; 16]> {
    let digits = <&[u8; 32]>::try_from(text).ok()?;
    let nibble = |at: usize| char::from(digits[at]).to_digit(16);
    let mut bytes = [0_u8; 16];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::try_from(nibble(2 * index)? << 4 | nibble(2 * index + 1)?).ok()?;
    }
    Some(bytes)
}

/// A hunt's session as the ticket carries it: an RFC 4122 UUID of version 1 to 5.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionUuid(pub [u8; 16]);

impl SessionUuid {
    /// The UUID if its version is 1 to 5 and its variant RFC 4122's.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, TicketError> {
        let version_ok = matches!(bytes[6] >> 4, 1..=5);
        let variant_ok = bytes[8] & 0xc0 == 0x80;
        (version_ok && variant_ok)
            .then_some(Self(bytes))
            .ok_or(TicketError::SessionNotUuid)
    }
}

impl FromStr for SessionUuid {
    type Err = TicketError;

    fn from_str(text: &str) -> Result<Self, TicketError> {
        let groups: Vec<&str> = text.split('-').collect();
        let shaped = groups.iter().map(|group| group.len()).eq([8, 4, 4, 4, 12]);
        let digits = groups.concat();
        shaped
            .then(|| sixteen_from_hex(digits.as_bytes()))
            .flatten()
            .ok_or(TicketError::SessionNotUuid)
            .and_then(Self::from_bytes)
    }
}

impl fmt::Display for SessionUuid {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&wire::format_uuid(&self.0))
    }
}

/// A ticket's one-time id. Only its digest is stored.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Jti(pub [u8; 16]);

impl Jti {
    /// A jti from the system random source.
    pub fn random() -> Result<Self, TicketError> {
        let mut fresh = Self([0; 16]);
        match rand::rngs::SysRng.try_fill_bytes(&mut fresh.0) {
            Ok(()) => Ok(fresh),
            Err(_) => Err(TicketError::Random),
        }
    }

    /// A jti from 32 hex digits of either case.
    pub fn parse(text: &str) -> Result<Self, TicketError> {
        sixteen_from_hex(text.as_bytes())
            .map(Self)
            .ok_or(TicketError::InvalidJti)
    }

    /// Also the peer admission's id.
    pub fn digest(&self) -> String {
        wire::hex(sha2::Sha256::digest(self.0).as_slice())
    }
}

impl fmt::Display for Jti {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&wire::hex(&self.0))
    }
}

/// The SHA-256 hex digest of a jti given as 32 hex digits.
pub fn hash_jti(jti: &str) -> Result<String, TicketError> {
    Ok(Jti::parse(jti)?.digest())
}

/// Everything a ticket admits a joiner to; the host checks each against its own config.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketBindings {
    /// The joiner's account.
    pub account: AccountId,
    /// The joiner's peer id.
    pub joiner_peer: PeerId,
    /// The host's peer id.
    pub host_peer: PeerId,
    /// The hunt session.
    pub session: SessionUuid,
    /// The hunt generation.
    pub generation: u32,
    /// The party the hunt was made for.
    pub party_id: String,
    /// The party's revision, at most 2^53 - 1.
    pub party_revision: u64,
    /// The hunt definition being played.
    pub hunt_definition_id: String,
    /// The game build.
    pub build: u32,
    /// The bridge protocol version.
    pub bridge_protocol: u16,
    /// The game's virtual port, never 0.
    pub virtual_port: u16,
}

/// Times are Unix seconds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketClaims {
    /// The key that signs the ticket.
    pub key_id: KeyId,
    /// The ticket's one-time id.
    pub jti: Jti,
    /// What the ticket admits to.
    pub bindings: TicketBindings,
    /// When it was minted, in Unix seconds.
    pub issued_at: u64,
    /// When it starts being valid, in Unix seconds.
    pub not_before: u64,
    /// When it expires, in Unix seconds.
    pub expires_at: u64,
}

impl TicketClaims {
    /// The jti's SHA-256 hex digest, which is also the admission's id.
    pub fn jti_digest(&self) -> String {
        self.jti.digest()
    }
}

/// 1 to `longest` bytes of ASCII letters, digits and the given punctuation.
fn is_identifier(text: &str, longest: usize, punctuation: &str) -> bool {
    !text.is_empty()
        && text.len() <= longest
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || punctuation.contains(c))
}

/// 1 to 32 ASCII letters, digits, `.`, `_` or `-`.
pub fn safe_key_id(text: &str) -> bool {
    is_identifier(text, 32, "._-")
}

/// Node holds the revision as a number: at most `Number.MAX_SAFE_INTEGER`.
fn revision_fits(revision: u64) -> bool {
    revision < 1 << 53
}

fn check_claims(claims: &TicketClaims) -> Result<(), TicketError> {
    let to = &claims.bindings;
    let identifiers = [
        (claims.key_id.0.as_str(), 32, "._-"),
        (to.account.0.as_str(), 96, "._:-"),
        (to.hunt_definition_id.as_str(), 128, "._:/-"),
        (to.party_id.as_str(), 192, "._:+/=-"),
    ];
    let rules: [(bool, TicketError); 5] = [
        (
            identifiers
                .iter()
                .all(|&(text, longest, punctuation)| is_identifier(text, longest, punctuation)),
            TicketError::UnsafeIdentifier,
        ),
        (to.virtual_port < 1000, TicketError::OutOfRange("virtualPort")),
        (to.virtual_port != 0, TicketError::ZeroVirtualPort),
        (
            revision_fits(to.party_revision),
            TicketError::OutOfRange("partyRevision"),
        ),
        (to.joiner_peer.0 != 0 && to.host_peer.0 != 0, TicketError::InvalidPeerId),
    ];
    if let Some((_, refusal)) = rules.into_iter().find(|(passed, _)| !passed) {
        return Err(refusal);
    }
    SessionUuid::from_bytes(to.session.0).map(|_| ())
}

/// The payload bytes, after the same checks the transport makes.
pub fn encode_claims(claims: &TicketClaims) -> Result<Vec<u8>, TicketError> {
    check_claims(claims)?;
    let to = &claims.bindings;
    let texts = [
        claims.key_id.0.as_bytes(),
        to.account.0.as_bytes(),
        to.hunt_definition_id.as_bytes(),
        to.party_id.as_bytes(),
    ];
    // The identifier rules cap every length (32, 96, 128, 192), so a failed narrowing means a broken rule.
    let narrow = |text: &[u8]| u16::try_from(text.len()).map_err(|_| TicketError::UnsafeIdentifier);
    let key_length = u8::try_from(texts[0].len()).map_err(|_| TicketError::UnsafeIdentifier)?;
    let header: [&[u8]; 18] = [
        PAYLOAD_MAGIC,
        &[PAYLOAD_VERSION, key_length],
        &narrow(texts[1])?.to_be_bytes(),
        &narrow(texts[2])?.to_be_bytes(),
        &narrow(texts[3])?.to_be_bytes(),
        &to.bridge_protocol.to_be_bytes(),
        &to.virtual_port.to_be_bytes(),
        &[0; 2],
        &to.generation.to_be_bytes(),
        &to.build.to_be_bytes(),
        &to.party_revision.to_be_bytes(),
        &claims.issued_at.to_be_bytes(),
        &claims.not_before.to_be_bytes(),
        &claims.expires_at.to_be_bytes(),
        &to.joiner_peer.0.to_be_bytes(),
        &to.host_peer.0.to_be_bytes(),
        &to.session.0,
        &claims.jti.0,
    ];
    Ok(header
        .iter()
        .chain(texts.iter())
        .flat_map(|part| part.iter().copied())
        .collect())
}

/// Takes big-endian fields off the front of a payload whose fixed part is already known to be whole.
struct Cursor<'a> {
    rest: &'a [u8],
}

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let (head, tail) = self.rest.split_at_checked(N).unwrap_or((&[], &[]));
        self.rest = tail;
        head.try_into().unwrap_or([0; N])
    }

    fn byte(&mut self) -> u8 {
        u8::from_be_bytes(self.take())
    }

    fn short(&mut self) -> u16 {
        u16::from_be_bytes(self.take())
    }

    fn word(&mut self) -> u32 {
        u32::from_be_bytes(self.take())
    }

    fn long(&mut self) -> u64 {
        u64::from_be_bytes(self.take())
    }

    fn text(&mut self, length: usize) -> String {
        let (head, tail) = self.rest.split_at(length.min(self.rest.len()));
        self.rest = tail;
        String::from_utf8_lossy(head).into_owned()
    }
}

/// Node holds times as numbers, so a time with no exact double cannot have come from it.
fn exact_double(value: u64) -> bool {
    value == 0 || value.ilog2() - value.trailing_zeros() < f64::MANTISSA_DIGITS
}

/// The claims of a payload, accepted only in the one canonical encoding.
pub fn decode_claims(payload: &[u8]) -> Result<TicketClaims, TicketError> {
    if !payload.starts_with(PAYLOAD_MAGIC) || payload.len() < FIXED_BYTES {
        return Err(TicketError::MalformedPayload);
    }
    let mut read = Cursor { rest: &payload[4..] };
    let version = read.byte();
    let lengths = [
        usize::from(read.byte()),
        usize::from(read.short()),
        usize::from(read.short()),
        usize::from(read.short()),
    ];
    let bridge_protocol = read.short();
    let virtual_port = read.short();
    let reserved = read.short();
    let framed = version == PAYLOAD_VERSION
        && reserved == 0
        && lengths.iter().all(|&length| length > 0)
        && payload.len() - FIXED_BYTES == lengths.iter().sum::<usize>();
    if !framed {
        return Err(TicketError::Framing);
    }
    let generation = read.word();
    let build = read.word();
    let party_revision = read.long();
    let [issued_at, not_before, expires_at] = [read.long(), read.long(), read.long()];
    let joiner_peer = PeerId(read.long());
    let host_peer = PeerId(read.long());
    let session = SessionUuid(read.take());
    let jti = Jti(read.take());
    let [key_id, account, hunt_definition_id, party_id] = lengths.map(|length| read.text(length));

    let claims = TicketClaims {
        key_id: KeyId(key_id),
        jti,
        bindings: TicketBindings {
            account: AccountId(account),
            joiner_peer,
            host_peer,
            session,
            generation,
            party_id,
            party_revision,
            hunt_definition_id,
            build,
            bridge_protocol,
            virtual_port,
        },
        issued_at,
        not_before,
        expires_at,
    };
    let canonical =
        encode_claims(&claims)? == payload && [issued_at, not_before, expires_at].into_iter().all(exact_double);
    canonical.then_some(claims).ok_or(TicketError::NoncanonicalPayload)
}

/// Unpadded base64url that re-encodes to the same text: no `=`, no `+` or `/`, no stray trailing bits.
fn strict_base64url(text: &str) -> Result<Vec<u8>, TicketError> {
    B64URL
        .decode(text)
        .ok()
        .filter(|bytes| B64URL.encode(bytes) == text)
        .ok_or(TicketError::NoncanonicalEncoding)
}

/// Splits and decodes a token, and calls `verify` on the raw payload before reading it.
pub fn parse_signed(token: &str, verify: impl FnOnce(&[u8], &Signature) -> bool) -> Result<TicketClaims, TicketError> {
    if token.len() > MAX_TOKEN_BYTES {
        return Err(TicketError::MissingOrOversized);
    }
    let mut pieces = token.split('.');
    let (Some(TOKEN_PREFIX), Some(body), Some(seal), None) =
        (pieces.next(), pieces.next(), pieces.next(), pieces.next())
    else {
        return Err(TicketError::MalformedToken);
    };
    if body.is_empty() || seal.is_empty() {
        return Err(TicketError::MalformedToken);
    }
    let body = strict_base64url(body)?;
    let seal = Some(strict_base64url(seal)?)
        .filter(|bytes| bytes.len() == 64)
        .ok_or(TicketError::NoncanonicalEncoding)?;
    // Refuses r or s of zero or not below the curve order.
    match Signature::from_slice(&seal) {
        Ok(signature) if verify(&body, &signature) => decode_claims(&body),
        _ => Err(TicketError::BadSignature),
    }
}

/// Checks the claims' times against `now_seconds`, allowing 30 seconds of clock skew either way.
pub fn check_lifetime(claims: &TicketClaims, now_seconds: u64) -> Result<(), TicketError> {
    let TicketClaims {
        issued_at,
        not_before,
        expires_at,
        ..
    } = *claims;
    let span = expires_at.checked_sub(issued_at);
    if not_before < issued_at || span.is_none_or(|span| span > MAX_LIFETIME_SECONDS) {
        Err(TicketError::InvalidLifetime)
    } else if issued_at.max(not_before) > now_seconds.saturating_add(CLOCK_SKEW_SECONDS) {
        Err(TicketError::NotYetValid)
    } else if now_seconds > expires_at.saturating_add(CLOCK_SKEW_SECONDS) {
        Err(TicketError::Expired)
    } else {
        Ok(())
    }
}

/// Framing and payload checks only; the signature is not checked.
pub fn parse_unverified(token: &str) -> Result<TicketClaims, TicketError> {
    parse_signed(token, |_, _| true)
}

/// A ticket just signed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MintedTicket {
    /// The ticket text; a secret until it reaches the joiner.
    pub text: Secret,
    /// What it carries.
    pub claims: TicketClaims,
    /// The jti's SHA-256 hex digest.
    pub jti_digest: String,
}

/// The ES256 ticket authority: verify-only when it has no signing key.
#[derive(Clone)]
pub struct Es256Tickets {
    /// The key id named in minted tickets and required of verified ones.
    pub key_id: KeyId,
    /// The signing key; `None` makes the authority verify-only.
    pub signing: Option<SigningKey>,
    /// The key tickets are verified with.
    pub verifying: VerifyingKey,
    /// A setting's `Number()`, checked only when a ticket is minted.
    pub default_lifetime: f64,
}

impl fmt::Debug for Es256Tickets {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = self.signing.is_some().then_some("<redacted>");
        out.debug_struct("Es256Tickets")
            .field("key_id", &self.key_id)
            .field("signing", &redacted)
            .field("default_lifetime", &self.default_lifetime)
            .finish()
    }
}

/// A lifetime in whole seconds within the allowed range; `None` for anything else, NaN included.
fn whole_lifetime(seconds: f64) -> Option<u64> {
    let allowed = f64::from(MIN_LIFETIME_SECONDS)..=MAX_LIFETIME_SECONDS as f64;
    (allowed.contains(&seconds) && seconds.trunc() == seconds).then_some(seconds as u64)
}

impl Es256Tickets {
    /// Mints a ticket valid from `now_seconds` for `lifetime_seconds` or the default lifetime.
    pub fn mint(
        &self,
        bindings: TicketBindings,
        now_seconds: u64,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, TicketError> {
        let seconds = lifetime_seconds.map(f64::from).unwrap_or(self.default_lifetime);
        let lifetime = whole_lifetime(seconds).ok_or(TicketError::LifetimeSetting)?;
        self.sign(TicketClaims {
            jti: Jti::random()?,
            key_id: self.key_id.clone(),
            bindings,
            expires_at: now_seconds.saturating_add(lifetime),
            issued_at: now_seconds,
            not_before: now_seconds,
        })
    }

    /// Signs these claims; fails without a signing key or when the text would be too long.
    pub fn sign(&self, claims: TicketClaims) -> Result<MintedTicket, TicketError> {
        let payload = encode_claims(&claims)?;
        let Some(key) = &self.signing else {
            return Err(TicketError::NoSigningKey);
        };
        let signature: Signature = key.try_sign(&payload).map_err(|_| TicketError::BadSignature)?;
        let pieces = [B64URL.encode(&payload), B64URL.encode(signature.to_bytes())];
        let text = [TOKEN_PREFIX, &pieces[0], &pieces[1]].join(".");
        if text.len() > MAX_TOKEN_BYTES {
            return Err(TicketError::TokenTooLarge);
        }
        let jti_digest = claims.jti.digest();
        Ok(MintedTicket {
            text: Secret::new(text),
            claims,
            jti_digest,
        })
    }

    /// High-S signatures pass: Node and the transport accept them, so a low-S rule here would split the two.
    pub fn verify(&self, token: &str, now_seconds: u64) -> Result<TicketClaims, TicketError> {
        let claims = parse_signed(token, |body, signature| self.verifying.verify(body, signature).is_ok())?;
        if claims.key_id == self.key_id {
            check_lifetime(&claims, now_seconds).map(|()| claims)
        } else {
            Err(TicketError::KeyNotActive)
        }
    }
}

/// Whole Unix seconds, rounded down; 0 before the epoch.
fn unix_seconds(at: UnixMillis) -> u64 {
    at.0.div_euclid(1000).try_into().unwrap_or_default()
}

impl TicketAuthority for Es256Tickets {
    fn key_id(&self) -> KeyId {
        self.key_id.clone()
    }

    fn mint(
        &self,
        to: TicketBindings,
        at: UnixMillis,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, AdmissionError> {
        Es256Tickets::mint(self, to, unix_seconds(at), lifetime_seconds).map_err(AdmissionError::from)
    }

    fn verify(&self, text: &str, at: UnixMillis) -> Result<TicketClaims, AdmissionError> {
        Es256Tickets::verify(self, text, unix_seconds(at)).map_err(AdmissionError::from)
    }
}
