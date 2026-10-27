//! A party's or hunt's id as the client writes it into a room name, and as the plugin reads it (`naming::room_kind_of`
//! in DaintyLass's chat plugin): `Party-<id>` where `<id>` is `<ULID>.<node>` (the JID-safe spelling of a global id)
//! or a UUID that this node owns, with anything after a `_` ignored (the build suffix of `MakePartyId`). The party
//! area must hand the client ids in one of these spellings for its room to be found.

use module_api::{GlobalId, IdKind, NodeId};

/// The most bytes of id text read: a global id is a kind, 26 characters and a node name of at most 255.
const MAX_TEXT: usize = 300;

/// `<ULID>.<node>`: how a global instance id is written in a room name.
pub fn room_id_text(id: &GlobalId) -> String {
    format!("{}.{}", id.ulid(), id.owner())
}

/// The global id a party or hunt id the client names stands for on `node`: the full `kind:ULID@node` text, the room
/// spelling `<ULID>.<node>`, or a UUID read as the ULID with the same 128 bits.
pub fn instance_id(text: &str, kind: IdKind, node: &NodeId) -> Option<GlobalId> {
    let id = text.split('_').next().unwrap_or_default();
    if id.is_empty() || id.len() > MAX_TEXT {
        return None;
    }
    if let Ok(global) = id.parse::<GlobalId>() {
        return (global.kind() == kind).then_some(global);
    }
    let canonical = match id.split_once('.') {
        Some((ulid, owner)) => format!("{}:{ulid}@{owner}", kind.as_str()),
        None => format!("{}:{}@{node}", kind.as_str(), ulid_text(uuid_bits(id)?)),
    };
    canonical.parse().ok()
}

/// A UUID's 128 bits: 8-4-4-4-12 hex digits, either case.
fn uuid_bits(text: &str) -> Option<u128> {
    let groups: Vec<&str> = text.split('-').collect();
    let shape = [8, 4, 4, 4, 12];
    if groups.len() != shape.len()
        || groups
            .iter()
            .zip(shape)
            .any(|(group, length)| group.len() != length || !group.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    u128::from_str_radix(&groups.concat(), 16).ok()
}

/// A ULID's canonical text (Crockford base32: 26 characters, the first holding three bits).
fn ulid_text(bits: u128) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    (0..26)
        .rev()
        .map(|i| char::from(ALPHABET[usize::try_from((bits >> (i * 5)) & 31).unwrap_or(0)]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> NodeId {
        "oregon-1".parse().unwrap()
    }

    #[test]
    fn a_party_id_has_three_spellings_that_name_one_instance() {
        let party = GlobalId::new(IdKind::Party, &"germany-1".parse().unwrap());
        let room = room_id_text(&party);
        assert_eq!(instance_id(&room, IdKind::Party, &node()), Some(party.clone()));
        assert_eq!(
            instance_id(&party.to_string(), IdKind::Party, &node()),
            Some(party.clone())
        );
        // the build suffix is ignored
        assert_eq!(
            instance_id(&format!("{room}_QnVpbGQ="), IdKind::Party, &node()),
            Some(party.clone())
        );
        // a hunt's id is not a party's
        assert_eq!(instance_id(&party.to_string(), IdKind::Hunt, &node()), None);
    }

    #[test]
    fn a_uuid_is_the_ulid_with_the_same_bits_on_this_node() {
        let uuid = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let id = instance_id(uuid, IdKind::Party, &node()).unwrap();
        assert_eq!(id.owner(), &node());
        assert_eq!(u128::from(id.ulid()), 0x0f8f_ad5b_d9cb_469f_a165_7086_7728_950e);
        assert_eq!(
            instance_id(&uuid.to_uppercase(), IdKind::Party, &node()),
            Some(id.clone())
        );
        assert_eq!(
            instance_id(&format!("{uuid}_QnVpbGQ="), IdKind::Party, &node()),
            Some(id)
        );
        assert_eq!(ulid_text(0), "0".repeat(26));
        assert_eq!(ulid_text(u128::MAX), format!("7{}", "Z".repeat(25)));
    }

    #[test]
    fn anything_else_names_nothing() {
        for bad in [
            "",
            "xyz",
            "0f8fad5b-d9cb-469f-a165-70867728950",
            "01j9z3n5c8q0v2x4y6w8t0r2p4.oregon-1",
            "01J9Z3N5C8Q0V2X4Y6W8T0R2P4.Oregon",
            "01J9Z3N5C8Q0V2X4Y6W8T0R2P4",
        ] {
            assert_eq!(instance_id(bad, IdKind::Party, &node()), None, "{bad}");
        }
    }
}
