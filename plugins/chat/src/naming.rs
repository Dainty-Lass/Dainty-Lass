// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! CLEAN-ROOM STUB (RD27): stands in for the Category A `naming.rs` (room names to `RoomKind`, DD21), which the
//! integration pass copies in unchanged. Behaviour from spec-rd27-chat §2.13.1 and §3.2 only.

use module_api::chat::RoomKind;
use module_api::network::{NodeId, Region};

/// `General` and `General<id>` the network room, `City-<id>` this node's region, `Region-<name>` a region by name,
/// `Party-<id>` and `Hunt-<id>` the instance's global id.
pub fn room_kind_of(local: &str, node: &NodeId, region: &Region) -> Option<RoomKind> {
    if local.starts_with("General") {
        return Some(RoomKind::Network);
    }
    if local.strip_prefix("City-").is_some_and(|id| !id.is_empty()) {
        return Some(RoomKind::Region(region.clone()));
    }
    if let Some(name) = local.strip_prefix("Region-") {
        return Region::new(name).ok().map(RoomKind::Region);
    }
    // A JID local part holds no `@`: the stub takes the ULID, owned by this node
    if let Some(id) = local.strip_prefix("Party-") {
        return format!("party:{id}@{node}").parse().ok().map(RoomKind::Party);
    }
    if let Some(id) = local.strip_prefix("Hunt-") {
        return format!("hunt:{id}@{node}").parse().ok().map(RoomKind::Hunt);
    }
    None
}

pub fn room_key(kind: &RoomKind) -> String {
    match kind {
        RoomKind::Network => String::from("network"),
        RoomKind::Region(region) => format!("region:{}", region.as_str()),
        RoomKind::Party(id) | RoomKind::Hunt(id) => id.to_string(),
    }
}

pub fn room_class(kind: &RoomKind) -> &'static str {
    match kind {
        RoomKind::Network => "network",
        RoomKind::Region(_) => "region",
        RoomKind::Party(_) => "party",
        RoomKind::Hunt(_) => "hunt",
    }
}
