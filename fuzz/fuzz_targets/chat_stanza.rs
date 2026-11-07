//! The chat plugin's stanza parser and the readers of client text (docs/security/chat.md): the frame parser,
//! JIDs, room names, authcids and nicknames. Whatever a client sends must neither panic nor take time beyond
//! linear; a frame that parses gives back only what was in it, and a room name maps to a room only in the
//! spellings `naming.rs` allows.
#![no_main]

use dainty_chat::naming::{room_id_text, room_kind_of};
use dainty_chat::nick::{check_nickname, name_part_of};
use dainty_chat::server::is_authcid_shape;
use dainty_chat::xmpp::{Element, Node, escape, has_markup_declaration, log_text, parse_frame, parse_jid, redact_frame};
use libfuzzer_sys::fuzz_target;
use dainty_module_api::chat::RoomKind;
use dainty_module_api::{NodeId, Region};

/// Every name, attribute and text in the tree came from the input, and the tree is no deeper than the input
/// is long. Walked without recursion.
fn check(root: &Element, input_len: usize) {
    let mut pending = vec![(root, 0usize)];
    while let Some((element, depth)) = pending.pop() {
        assert!(element.name.len() <= input_len);
        assert!(depth <= input_len);
        for (name, value) in &element.attrs {
            assert!(name.len() <= input_len && value.len() <= input_len);
        }
        for child in &element.children {
            match child {
                Node::Element(inner) => pending.push((inner, depth + 1)),
                Node::Text(text) => assert!(text.len() <= input_len),
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    // The chat reads every frame as UTF-8, lossily (text and binary frames alike)
    let raw = String::from_utf8_lossy(data);
    if !has_markup_declaration(&raw)
        && let Some(element) = parse_frame(&raw)
    {
        check(&element, raw.len());
        let _ = element.text();
        let _ = element.child("body");
    }
    let _ = redact_frame(&raw);
    let _ = log_text(&raw, 80);
    let _ = escape(&raw);
    if is_authcid_shape(&raw) {
        assert!(!raw.contains('@') && !raw.contains('/') && raw.len() <= 128);
    }
    let (Ok(node), Ok(region)) = (NodeId::new("oregon-1"), Region::new("oregon")) else {
        return;
    };
    if let Some(jid) = parse_jid(&raw) {
        match room_kind_of(&jid.local, &node, &region) {
            Some(RoomKind::Party(id) | RoomKind::Hunt(id)) => {
                // An instance room's id has one JID-safe spelling, and it holds no separator
                let text = room_id_text(&id);
                assert!(!text.contains('@') && !text.contains('/'));
            }
            Some(RoomKind::Region(name)) => assert!(!name.as_str().is_empty()),
            Some(RoomKind::Network) => assert!(jid.local.starts_with("General")),
            None => {}
        }
        let nick = jid.resource.unwrap_or_default();
        let _ = check_nickname(&nick, &jid.local, &jid.domain, &[Some("Ada")]);
        let _ = name_part_of(&nick);
    }
});
