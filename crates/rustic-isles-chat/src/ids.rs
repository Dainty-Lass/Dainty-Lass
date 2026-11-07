// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Party and hunt ids inside room names (spec §4.3): a `GlobalId`'s own text cannot sit in a JID, so a room spells
//! it `<ULID>.<node>`; the party area appends `_<build id>` to make the client-facing party id.

use dainty_module_api::{GlobalId, IdKind, NodeId};
use ulid::Ulid;

/// The longest id text read, before the `_` suffix.
pub const MAX_ID_TEXT: usize = 300;

/// `<ULID>.<node>`.
pub fn room_id_text(id: &GlobalId) -> String {
    format!("{}.{}", id.ulid(), id.owner())
}

/// The id of kind `kind` a room name's id text names, a UUID being owned by `node`.
pub fn instance_id(text: &str, kind: IdKind, node: &NodeId) -> Option<GlobalId> {
    let text = text.split('_').next().unwrap_or("");
    if text.is_empty() || text.len() > MAX_ID_TEXT {
        return None;
    }
    if let Ok(id) = text.parse::<GlobalId>() {
        return (id.kind() == kind).then_some(id);
    }
    if let Some((ulid, owner)) = text.split_once('.') {
        let parsed = Ulid::from_string(ulid).ok()?;
        // Canonical upper case only, as GlobalId reads it
        if parsed.to_string() != ulid {
            return None;
        }
        let owner = NodeId::new(owner).ok()?;
        return Some(GlobalId::from_parts(kind, parsed, owner));
    }
    ulid_of_uuid(text).map(|ulid| GlobalId::from_parts(kind, ulid, node.clone()))
}

/// The ULID with a UUID's 128 bits: 36 bytes, hyphens exactly at 8, 13, 18 and 23, hex digits (either case)
/// everywhere else.
fn ulid_of_uuid(text: &str) -> Option<Ulid> {
    const HYPHENS: [usize; 4] = [8, 13, 18, 23];
    if text.len() != 36 {
        return None;
    }
    let mut value: u128 = 0;
    for (at, byte) in text.bytes().enumerate() {
        if HYPHENS.contains(&at) {
            if byte != b'-' {
                return None;
            }
            continue;
        }
        let digit = char::from(byte).to_digit(16)?;
        value = (value << 4) | u128::from(digit);
    }
    Some(Ulid::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> NodeId {
        NodeId::new("local").unwrap()
    }

    #[test]
    fn room_spelling_round_trips() {
        let id = GlobalId::new(IdKind::Party, &NodeId::new("oregon-1").unwrap());
        let text = room_id_text(&id);
        assert_eq!(text, format!("{}.oregon-1", id.ulid()));
        assert_eq!(instance_id(&text, IdKind::Party, &node()), Some(id.clone()));
        assert_eq!(
            instance_id(&format!("{text}_NjQ3NDcy"), IdKind::Party, &node()),
            Some(id.clone())
        );
        assert_eq!(
            instance_id(&format!("{text}_"), IdKind::Party, &node()),
            Some(id.clone())
        );
        assert_eq!(instance_id(&id.to_string(), IdKind::Party, &node()), Some(id.clone()));
        assert_eq!(instance_id(&id.to_string(), IdKind::Hunt, &node()), None);
        // The dotted spelling carries no kind: it is read as the kind asked
        assert_eq!(
            instance_id(&text, IdKind::Hunt, &node()).map(|h| (h.kind(), h.ulid())),
            Some((IdKind::Hunt, id.ulid()))
        );
    }

    #[test]
    fn uuids_take_this_node() {
        let upper = "0189ABCD-EF01-7234-8567-89ABCDEF0123";
        let id = instance_id(upper, IdKind::Hunt, &node()).unwrap();
        assert_eq!(id.owner(), &node());
        assert_eq!(u128::from(id.ulid()), 0x0189_ABCD_EF01_7234_8567_89AB_CDEF_0123);
        assert_eq!(instance_id(&upper.to_lowercase(), IdKind::Hunt, &node()), Some(id));
        assert_eq!(
            instance_id("0189ABCD-EF01-7234-8567-89ABCDEF012", IdKind::Hunt, &node()),
            None
        );
        assert_eq!(
            instance_id("0189ABCD-EF01-7234-8567-89ABCDEF012G", IdKind::Hunt, &node()),
            None
        );
    }

    #[test]
    fn edges_and_refusals() {
        let zeros = format!("{}.local", "0".repeat(26));
        assert_eq!(
            u128::from(instance_id(&zeros, IdKind::Party, &node()).unwrap().ulid()),
            0
        );
        let ones = format!("7{}.local", "Z".repeat(25));
        assert_eq!(
            u128::from(instance_id(&ones, IdKind::Party, &node()).unwrap().ulid()),
            u128::MAX
        );
        let ulid = Ulid::from(42u128).to_string();
        for bad in [
            String::new(),
            "_x".to_owned(),
            "xyz".to_owned(),
            ulid.clone(),
            format!("{}.local", ulid.to_lowercase()),
            format!("{ulid}.Oregon"),
            format!("{ulid}."),
            format!("8{}.local", "Z".repeat(25)),
            format!("{ulid}.{}", "a".repeat(MAX_ID_TEXT)),
        ] {
            assert_eq!(instance_id(&bad, IdKind::Party, &node()), None, "{bad}");
        }
        let at_limit = format!("{ulid}.{}", "a".repeat(MAX_ID_TEXT - 27));
        assert_eq!(at_limit.len(), MAX_ID_TEXT);
        // 300 bytes is read; the node name itself is then too long
        assert_eq!(instance_id(&at_limit, IdKind::Party, &node()), None);
    }
}
