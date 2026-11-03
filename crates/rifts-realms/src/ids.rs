// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A session's three spellings: the wire UUID the tables, tickets and `bridge.conf` hold, its 128 bits read as a
//! ULID, and the node-tagged seam and module-api ids built from those bits (DD18).

use std::fmt::Write as _;

use module_api::{GlobalId, NodeId};
use rand::RngExt as _;
use rifts_seam::{RealmId, SessionId, Ulid, UnixMillis};

const WIRE_LEN: usize = 36;
const DASHES: [usize; 4] = [8, 13, 18, 23];

/// Keeps the text a valid version 4 UUID whatever the other bits hold.
fn version_4(mut bytes: [u8; 16]) -> u128 {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    u128::from_be_bytes(bytes)
}

fn random_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes[..]);
    bytes
}

pub fn random_uuid() -> String {
    uuid_text(version_4(random_bytes()))
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Creation time in the top 48 bits, so ids sort by age as ULIDs do.
pub fn new_session_ulid(at: UnixMillis) -> u128 {
    let mut bytes = random_bytes();
    let millis = (at.0 as u64).to_be_bytes();
    bytes[..6].copy_from_slice(&millis[2..]);
    version_4(bytes)
}

pub fn uuid_text(bits: u128) -> String {
    let mut out = hex(&bits.to_be_bytes());
    for at in DASHES {
        out.insert(at, '-');
    }
    out
}

/// Lower-case `8-4-4-4-12` hex only.
pub fn session_bits(wire: &str) -> Option<u128> {
    let bytes = wire.as_bytes();
    if bytes.len() != WIRE_LEN {
        return None;
    }
    let mut bits: u128 = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if DASHES.contains(&index) {
            if *byte != b'-' {
                return None;
            }
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => return None,
        };
        bits = (bits << 4) | u128::from(digit);
    }
    Some(bits)
}

fn seam_node(node: &NodeId) -> Option<rifts_seam::NodeId> {
    rifts_seam::NodeId::new(node.as_str()).ok()
}

fn seam_global(wire: &str, node: &NodeId) -> Option<rifts_seam::GlobalId> {
    Some(rifts_seam::GlobalId {
        ulid: Ulid(session_bits(wire)?),
        node: seam_node(node)?,
    })
}

/// `hub_instance:<ULID>@<node>` for a hub generation, `hunt:<ULID>@<node>` otherwise.
pub fn global_session_id(wire: &str, hub: bool, node: &NodeId) -> Option<GlobalId> {
    let kind = if hub { "hub_instance" } else { "hunt" };
    format!("{kind}:{}@{node}", Ulid(session_bits(wire)?)).parse().ok()
}

pub fn seam_session_id(wire: &str, node: &NodeId) -> Option<SessionId> {
    seam_global(wire, node).map(SessionId)
}

pub fn wire_session_id(session: &SessionId) -> String {
    uuid_text(session.0.ulid.0)
}

pub fn seam_realm_id(wire: &str, node: &NodeId) -> Option<RealmId> {
    seam_global(wire, node).map(RealmId)
}

pub fn wire_realm_id(realm: &RealmId) -> String {
    uuid_text(realm.0.ulid.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> NodeId {
        NodeId::new("local").unwrap()
    }

    #[test]
    fn wire_text_round_trips() {
        let wire = "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2b";
        let seam = seam_session_id(wire, &node()).unwrap();
        assert_eq!(seam.0.node.as_str(), "local");
        assert_eq!(wire_session_id(&seam), wire);
        let realm = seam_realm_id(wire, &node()).unwrap();
        assert_eq!(wire_realm_id(&realm), wire);
        assert_eq!(
            global_session_id(wire, true, &node()).unwrap().to_string(),
            format!("hub_instance:{}@local", seam.0.ulid)
        );
        assert_eq!(
            global_session_id(wire, false, &node()).unwrap().to_string(),
            format!("hunt:{}@local", seam.0.ulid)
        );
    }

    #[test]
    fn only_lower_case_wire_ids_parse() {
        for bad in [
            "0190A1B2-C3D4-4E5F-8A6B-7C8D9E0F1A2B",
            "0190a1b2c3d44e5f8a6b7c8d9e0f1a2b",
            "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2",
            "0190a1b2-c3d4-4e5f-8a6b_7c8d9e0f1a2b",
            "realm-1",
            "",
        ] {
            assert_eq!(session_bits(bad), None, "{bad}");
            assert!(global_session_id(bad, false, &node()).is_none());
        }
    }

    #[test]
    fn new_ids_are_version_4_and_carry_the_time() {
        let bits = new_session_ulid(UnixMillis(1_792_000_000_123));
        let text = uuid_text(bits);
        assert_eq!(session_bits(&text), Some(bits));
        assert_eq!(&text[14..15], "4");
        assert!(matches!(&text[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(Ulid(bits).unix_millis(), 1_792_000_000_123);
        let random = random_uuid();
        assert_eq!(random.len(), 36);
        assert_eq!(&random[14..15], "4");
        assert_ne!(random, random_uuid());
        assert_eq!(hex(&[0, 0xab, 0x10]), "00ab10");
    }
}
