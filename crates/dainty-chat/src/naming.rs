// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! New here (DD21): which [`RoomKind`] a room JID the client names is. DR's room classes (`muc.rs`'s
//! `room_class_of`) and Mystic Paradox's (`RoomService.ts`, `cityRooms.ts`; the names only, no code) give the
//! spellings: `General<id>`, `City-<id>`, `Party-<id>`, `Hunt-<id>`. Guild rooms have no kind in DD21 and are
//! refused.
//!
//! - `General`, `General<id>`: the network-wide room, whatever the id.
//! - `City-<id>`: this node's region room. Every hub instance of a region shares it (DD21, patch 0025's City
//!   rooms); each occupant keeps the JID it joined with.
//! - `Region-<region>`: a region's room by name.
//! - `Party-<id>`, `Hunt-<id>`: the instance's global id. The id is `<ULID>.<node>` (the JID-safe spelling of
//!   [`GlobalId`], which a JID can't hold as it is), or a UUID, which this node owns, read as the ULID with the
//!   same 128 bits. Anything after a `_` is ignored (the 1.14.7 party id's build suffix).

use dainty_module_api::chat::RoomKind;
use dainty_module_api::{GlobalId, IdKind, NodeId, Region};
use ulid::Ulid;

/// The most bytes an id in a room name may hold (DR's limit).
const MAX_ID: usize = 64;

/// The room a client's room local part names, on a node in `region` named `node`.
pub fn room_kind_of(local: &str, node: &NodeId, region: &Region) -> Option<RoomKind> {
    if let Some(id) = local.strip_prefix("General") {
        return (id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
            .then_some(RoomKind::Network);
    }
    if let Some(id) = local.strip_prefix("City-") {
        let ok = !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        return ok.then(|| RoomKind::Region(region.clone()));
    }
    if let Some(name) = local.strip_prefix("Region-") {
        return Region::new(name).ok().map(RoomKind::Region);
    }
    if let Some(id) = local.strip_prefix("Party-") {
        return instance_id(id, IdKind::Party, node).map(RoomKind::Party);
    }
    if let Some(id) = local.strip_prefix("Hunt-") {
        return instance_id(id, IdKind::Hunt, node).map(RoomKind::Hunt);
    }
    None
}

/// `<ULID>.<node>`, or a UUID owned by `node`.
fn instance_id(text: &str, kind: IdKind, node: &NodeId) -> Option<GlobalId> {
    let id = text.split('_').next().unwrap_or("");
    if id.is_empty() || id.len() > MAX_ID {
        return None;
    }
    if let Some((ulid, owner)) = id.split_once('.') {
        let parsed = Ulid::from_string(ulid).ok().filter(|u| u.to_string() == ulid)?;
        return Some(GlobalId::from_parts(kind, parsed, NodeId::new(owner).ok()?));
    }
    uuid_bits(id).map(|bits| GlobalId::from_parts(kind, Ulid::from(bits), node.clone()))
}

/// A UUID's 128 bits: 8-4-4-4-12 hex digits, either case.
fn uuid_bits(text: &str) -> Option<u128> {
    if text.len() != 36 || [8, 13, 18, 23].into_iter().any(|i| text.as_bytes()[i] != b'-') {
        return None;
    }
    uuid::Uuid::try_parse(text).ok().map(|u| u.as_u128())
}

/// The JID-safe spelling of an instance id, as a room name holds it.
pub fn room_id_text(id: &GlobalId) -> String {
    format!("{}.{}", id.ulid(), id.owner())
}

/// How a room is stored and logged: `network`, `region:<name>`, `party:<id>` or `hunt:<id>`.
pub fn room_key(kind: &RoomKind) -> String {
    match kind {
        RoomKind::Network => String::from("network"),
        RoomKind::Region(region) => format!("region:{region}"),
        RoomKind::Party(id) | RoomKind::Hunt(id) => id.to_string(),
    }
}

/// Short, for log lines: the kind without the id.
pub fn room_class(kind: &RoomKind) -> &'static str {
    match kind {
        RoomKind::Network => "network",
        RoomKind::Region(_) => "region",
        RoomKind::Party(_) => "party",
        RoomKind::Hunt(_) => "hunt",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooms_are_named_as_the_clients_name_them() {
        let node = NodeId::new("oregon-1").unwrap();
        let region = Region::new("oregon").unwrap();
        let kind = |local: &str| room_kind_of(local, &node, &region);
        assert_eq!(kind("General"), Some(RoomKind::Network));
        assert_eq!(kind("General7"), Some(RoomKind::Network));
        assert_eq!(kind("City-abc-1"), Some(RoomKind::Region(region.clone())));
        assert_eq!(kind("City-"), None);
        assert_eq!(
            kind("Region-germany"),
            Some(RoomKind::Region(Region::new("germany").unwrap()))
        );
        assert_eq!(kind("Guild-a"), None);
        assert_eq!(kind("city-1"), None);

        let party = GlobalId::new(IdKind::Party, &NodeId::new("germany-1").unwrap());
        let text = format!("Party-{}", room_id_text(&party));
        assert_eq!(kind(&text), Some(RoomKind::Party(party.clone())));
        assert_eq!(
            kind(&format!("{text}_QnVpbGQ=")),
            Some(RoomKind::Party(party)),
            "the build suffix is ignored"
        );
        let uuid = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let Some(RoomKind::Hunt(hunt)) = kind(&format!("Hunt-{uuid}")) else {
            panic!("a UUID names a hunt on this node");
        };
        assert_eq!(hunt.owner(), &node);
        assert_eq!(u128::from(hunt.ulid()), 0x0f8f_ad5b_d9cb_469f_a165_7086_7728_950e);
        assert_eq!(
            kind(&format!("Hunt-{}", uuid.to_uppercase())),
            Some(RoomKind::Hunt(hunt))
        );
        for bad in [
            "Party-",
            "Party-xyz",
            "Party-01j9z3n5c8q0v2x4y6w8t0r2p4.oregon-1",
            "Party-01J9Z3N5C8Q0V2X4Y6W8T0R2P4.Oregon",
            "Hunt-0f8fad5b-d9cb-469f-a165-70867728950",
        ] {
            assert_eq!(kind(bad), None, "{bad}");
        }
    }

    #[test]
    fn rooms_have_one_key() {
        let region = Region::new("oregon").unwrap();
        assert_eq!(room_key(&RoomKind::Network), "network");
        assert_eq!(room_key(&RoomKind::Region(region)), "region:oregon");
        let hunt = GlobalId::new(IdKind::Hunt, &NodeId::new("oregon-1").unwrap());
        assert_eq!(room_key(&RoomKind::Hunt(hunt.clone())), hunt.to_string());
    }
}
