//! Replays what Mystic Paradox's own `admissionTickets.ts` answered (`vectors/node-admission.json`, made by
//! `vectors/make-vectors.mts` in `node:22`): tickets Node issued, and Node's verdict on each of 349 tokens, most of
//! them payloads bent byte by byte and re-signed so the decoder's checks are reached. The Rust port must agree on
//! every one, message for message. Only Node's public key is kept here.

// The helpers are test code too, outside the #[test] functions clippy exempts
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;

use rifts_admission::ticket::{decode_claims, encode_claims, hash_jti};
use rifts_admission::{Es256Tickets, TicketClaims, TicketError, derive_ice_peer_id};
use rifts_seam::AccountId;
use serde_json::{Value, json};

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/node-admission.json")).unwrap()
}

fn authority(vectors: &Value) -> Es256Tickets {
    let settings: HashMap<String, String> = [
        ("P2P_ADMISSION_KEY_ID", vectors["keyId"].as_str().unwrap()),
        (
            "P2P_ADMISSION_VERIFY_PUBKEY_B64",
            vectors["verifyKeyB64"].as_str().unwrap(),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    Es256Tickets::from_settings(&settings).unwrap()
}

/// `AdmissionTicketClaims` as Node returns it.
fn node_claims(claims: &TicketClaims) -> Value {
    let bindings = &claims.bindings;
    json!({
        "issuer": "paradox-backend",
        "audience": "mysticparadox-steam-bridge",
        "purpose": "JOIN_HUNT",
        "version": 2,
        "keyId": claims.key_id.0,
        "jti": claims.jti.to_string(),
        "accountId": bindings.account.0,
        "joinerSteamId": bindings.joiner_peer.0.to_string(),
        "hostSteamId": bindings.host_peer.0.to_string(),
        "sessionId": bindings.session.to_string(),
        "generation": bindings.generation,
        "partyId": bindings.party_id,
        "partyRevision": bindings.party_revision,
        "huntDefinitionId": bindings.hunt_definition_id,
        "build": bindings.build,
        "bridgeProtocol": bindings.bridge_protocol,
        "virtualPort": bindings.virtual_port,
        "issuedAt": claims.issued_at,
        "notBefore": claims.not_before,
        "expiresAt": claims.expires_at,
    })
}

fn payload_of(token: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.split('.').nth(1).unwrap())
        .unwrap()
}

#[test]
fn nodes_public_key_is_read_and_its_xy_matches() {
    let vectors = vectors();
    let tickets = authority(&vectors);
    let xy: String = tickets.public_xy().0.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(xy, vectors["publicXyHex"].as_str().unwrap());
}

#[test]
fn tickets_node_issued_verify_and_re_encode_to_the_same_bytes() {
    let vectors = vectors();
    let tickets = authority(&vectors);
    for issued in vectors["issued"].as_array().unwrap() {
        let token = issued["token"].as_str().unwrap();
        let now = issued["claims"]["issuedAt"].as_u64().unwrap();
        let claims = tickets.verify(token, now).unwrap();
        // IssueAdmissionTicket echoes the caller's session id; decoding gives it in lower case
        let mut expected = issued["claims"].clone();
        expected["sessionId"] = expected["sessionId"].as_str().unwrap().to_lowercase().into();
        assert_eq!(node_claims(&claims), expected, "{token}");
        assert_eq!(claims.jti_digest(), issued["jtiDigest"].as_str().unwrap());
        let payload = payload_of(token);
        assert_eq!(encode_claims(&decode_claims(&payload).unwrap()).unwrap(), payload);
    }
}

#[test]
fn every_verdict_is_nodes() {
    let vectors = vectors();
    let tickets = authority(&vectors);
    let verdicts = vectors["vectors"].as_array().unwrap();
    assert_eq!(verdicts.len(), 349);
    for verdict in verdicts {
        let token = verdict["token"].as_str().unwrap();
        let now = verdict["now"].as_u64().unwrap();
        let rust = tickets.verify(token, now);
        match (&rust, verdict["error"].as_str()) {
            (Ok(claims), None) => assert_eq!(node_claims(claims), verdict["ok"], "{token}"),
            (Err(error), Some(node)) if node.starts_with("The value of \"value\" is out of range") => {
                // Node's writeBigUInt64BE throws for a time that Number() rounds up to 2^64; both are invalid tickets
                assert_eq!(*error, TicketError::NoncanonicalPayload, "{token}");
            }
            (Err(error), Some(node)) => assert_eq!(error.to_string(), node, "{token}"),
            (rust, node) => panic!("{token} at {now}: Rust {rust:?}, Node {node:?}"),
        }
    }
}

#[test]
fn jti_digests_and_ice_peer_ids_are_nodes() {
    let vectors = vectors();
    for entry in vectors["jtiDigests"].as_array().unwrap() {
        assert_eq!(
            hash_jti(entry["jti"].as_str().unwrap()).unwrap(),
            entry["digest"].as_str().unwrap()
        );
    }
    for entry in vectors["icePeerIds"].as_array().unwrap() {
        let peer = derive_ice_peer_id(&AccountId(entry["account"].as_str().unwrap().to_owned()));
        assert_eq!(peer.0.to_string(), entry["peerId"].as_str().unwrap());
    }
}
