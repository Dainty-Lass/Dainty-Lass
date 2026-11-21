// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Ticket checks on the host side, on `rifts-admission`'s parser so the issuer and the transport share one format.
//! The codes are the ones a joiner receives in the admission result.

use std::collections::HashSet;
use std::fs;
use std::io::Read as _;
use std::path::Path;

use p256::ecdsa::VerifyingKey;
use p256::ecdsa::signature::Verifier as _;
use rifts_admission::ticket::{self, MAX_TOKEN_BYTES, TicketError};

use crate::files::Bindings;

const KEY_FILE_BYTES: u64 = 64;
const MAX_PAYLOAD: usize = 2048;

/// The 64-byte raw X‖Y file the launcher writes (`p2p-admission-public.xy`).
pub fn load_public_key(path: &Path) -> Result<VerifyingKey, String> {
    let bad_file = || "admission public key must be a regular 64-byte raw X||Y file".to_owned();
    let metadata = fs::symlink_metadata(path).map_err(|_| bad_file())?;
    if !metadata.file_type().is_file() || metadata.len() != KEY_FILE_BYTES {
        return Err(bad_file());
    }
    let mut bytes = Vec::with_capacity(65);
    bytes.push(0x04);
    fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .map_err(|_| "could not read admission public key".to_owned())?;
    if bytes.len() != 65 {
        return Err("could not read admission public key".to_owned());
    }
    VerifyingKey::from_sec1_bytes(&bytes).map_err(|_| bad_file())
}

/// What the session expects a ticket to name.
#[derive(Debug, Clone, Copy)]
pub struct Expected<'a> {
    pub bindings: &'a Bindings,
    pub joiner_peer: u64,
    pub host_peer: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub account: String,
    pub jti: [u8; 16],
}

fn code(error: &TicketError) -> &'static str {
    match error {
        TicketError::BadSignature => "bad_signature",
        TicketError::InvalidLifetime => "bad_lifetime",
        TicketError::NotYetValid => "not_yet_valid",
        TicketError::Expired => "expired",
        _ => "malformed",
    }
}

/// Checks in the transport's order; the first failure's code is returned.
pub fn verify(
    key: &VerifyingKey,
    token: &str,
    expected: Expected<'_>,
    consumed: &HashSet<[u8; 16]>,
    now_seconds: u64,
) -> Result<Verified, &'static str> {
    if token.len() > MAX_TOKEN_BYTES || token.bytes().filter(|b| *b == b'.').count() != 2 {
        return Err("malformed");
    }
    // The payload decodes to at most 2048 bytes; its text is refused before any signature work.
    let payload_text = token.split('.').nth(1).unwrap_or_default();
    if payload_text.len() * 6 / 8 > MAX_PAYLOAD {
        return Err("malformed");
    }
    let claims = ticket::parse_signed(token, |payload, signature| key.verify(payload, signature).is_ok())
        .map_err(|e| code(&e))?;
    ticket::check_lifetime(&claims, now_seconds).map_err(|e| code(&e))?;
    let bindings = expected.bindings;
    let named = &claims.bindings;
    if claims.key_id.0 != bindings.key_id {
        return Err("wrong_key");
    }
    if named.session.to_string() != bindings.session_id {
        return Err("wrong_session");
    }
    if named.generation != bindings.generation {
        return Err("wrong_generation");
    }
    if named.party_id != bindings.party_id || named.party_revision != bindings.party_revision {
        return Err("wrong_party");
    }
    if named.hunt_definition_id != bindings.hunt_definition_id {
        return Err("wrong_hunt");
    }
    if named.joiner_peer.0 != expected.joiner_peer {
        return Err("wrong_joiner");
    }
    if named.host_peer.0 != expected.host_peer {
        return Err("wrong_host");
    }
    if named.build != bindings.build {
        return Err("wrong_build");
    }
    if named.bridge_protocol != bindings.bridge_protocol || named.virtual_port != bindings.virtual_port {
        return Err("wrong_protocol");
    }
    if consumed.contains(&claims.jti.0) {
        return Err("replay");
    }
    Ok(Verified {
        account: named.account.0.clone(),
        jti: claims.jti.0,
    })
}
