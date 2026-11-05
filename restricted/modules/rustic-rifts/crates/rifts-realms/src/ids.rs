//! Session ids in their three spellings. The runtime, the admission ticket (16 bytes) and `bridge.conf` know a session
//! by Node's 36-character UUID, so that stays the wire form and the tables' key. Its 128 bits are the session's ULID,
//! which gives module-api's network-wide id (`hunt:<ULID>@<node>`, DD18) and RF2's seam id without a lookup.

use module_api::{GlobalId, IdKind, NodeId};
use rifts_seam::{RealmId, SessionId, Ulid, UnixMillis};

/// A random version 4 UUID (`crypto.randomUUID()`), for realm, offer, message and capability ids.
pub fn random_uuid() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid_text(u128::from_be_bytes(bytes))
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A new session's ULID: its creation time, then 80 random bits with RFC 4122 v4 version/variant nibbles
/// so `uuid_text` is also a valid `SessionUuid` for admission tickets.
pub fn new_session_ulid(now: UnixMillis) -> u128 {
    let mut bytes: [u8; 16] = rand::random();
    let ts = u64::try_from(now.0).unwrap_or_default().to_be_bytes();
    bytes[..6].copy_from_slice(&ts[2..8]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    u128::from_be_bytes(bytes)
}

/// Lower-case `8-4-4-4-12` hex, as the transport writes a ticket's session bytes (`admission_ticket.cpp`).
pub fn uuid_text(bits: u128) -> String {
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The bits of a wire session id; `None` unless it is exactly the lower-case form Node and the transport write.
pub fn session_bits(wire: &str) -> Option<u128> {
    let bytes = wire.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let mut hex = String::with_capacity(32);
    for (index, &byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return None;
            }
        } else if byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) {
            hex.push(char::from(byte));
        } else {
            return None;
        }
    }
    u128::from_str_radix(&hex, 16).ok()
}

/// module-api's id of a session this node owns: a hub generation is a `hub_instance`, anything else a `hunt`.
pub fn global_session_id(wire: &str, hub: bool, node: &NodeId) -> Option<GlobalId> {
    let kind = if hub { IdKind::HubInstance } else { IdKind::Hunt };
    let text = format!("{}:{}@{}", kind.as_str(), Ulid(session_bits(wire)?), node);
    text.parse().ok()
}

/// RF2's seam id (`<ULID>@<node>`) of a wire session id.
pub fn seam_session_id(wire: &str, node: &NodeId) -> Option<SessionId> {
    format!("{}@{}", Ulid(session_bits(wire)?), node).parse().ok()
}

/// The wire form of a seam session id.
pub fn wire_session_id(session: &SessionId) -> String {
    uuid_text(session.0.ulid.0)
}

/// RF2's seam id (`<ULID>@<node>`) of a wire realm id.
pub fn seam_realm_id(wire: &str, node: &NodeId) -> Option<RealmId> {
    format!("{}@{}", Ulid(session_bits(wire)?), node).parse().ok()
}

/// The wire form of a seam realm id.
pub fn wire_realm_id(realm: &RealmId) -> String {
    uuid_text(realm.0.ulid.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_maps_between_its_spellings() {
        let node = NodeId::new("oregon-1").unwrap();
        let bits = new_session_ulid(UnixMillis(1_792_000_000_000));
        let wire = uuid_text(bits);
        assert_eq!(wire.len(), 36);
        assert_eq!(session_bits(&wire), Some(bits));
        let global = global_session_id(&wire, false, &node).unwrap();
        assert_eq!(global.kind(), IdKind::Hunt);
        assert_eq!(global.owner(), &node);
        assert!(global.to_string().starts_with("hunt:"));
        assert_eq!(
            global_session_id(&wire, true, &node).unwrap().kind(),
            IdKind::HubInstance
        );
        let seam = seam_session_id(&wire, &node).unwrap();
        assert_eq!(seam.0.ulid.unix_millis(), 1_792_000_000_000);
        assert_eq!(wire_session_id(&seam), wire);
        // Node's own v4 ids map the same way
        let node_id = "87f6c5d7-fc8c-41b9-8837-5aeac1bd462d";
        assert_eq!(wire_session_id(&seam_session_id(node_id, &node).unwrap()), node_id);
    }

    #[test]
    fn only_the_lower_case_wire_form_is_a_session_id() {
        for bad in [
            "87F6C5D7-FC8C-41B9-8837-5AEAC1BD462D",
            "87f6c5d7fc8c41b988375aeac1bd462d",
            "session-1",
            "",
        ] {
            assert_eq!(session_bits(bad), None, "{bad}");
        }
    }

    #[test]
    fn random_uuids_are_version_4() {
        let id = random_uuid();
        assert_ne!(id, random_uuid());
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
    }
}
