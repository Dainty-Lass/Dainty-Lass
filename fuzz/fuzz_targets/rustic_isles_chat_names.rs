//! What Rustic Isles' chat rules read from players (spec-rustic-isles-chat §12): any bytes as a room local part, a
//! party or hunt id, an `includePending` query and a JID local part; as the Slayer link body; and as the parts of a
//! headline stanza. Nothing panics, an id over 300 bytes is never read, every id read writes back the room spelling
//! it came from, the availability list stays capped, and the stanza escapes every value.
#![no_main]

use libfuzzer_sys::fuzz_target;
use module_api::chat::Jid;
use module_api::{IdKind, NodeIdentity};
use rustic_isles_chat::headline::frame;
use rustic_isles_chat::ids::{MAX_ID_TEXT, instance_id, room_id_text};
use rustic_isles_chat::{
    HEADLINE_FROM, HubSessions, SLAYER_LINK_MAX_ACCOUNTS, XMPP_DOMAIN, include_pending, room_kind_of,
    slayer_link_availability,
};
use serde_json::Value;

/// Escaped text: no markup character, and every `&` starts one of the five entities.
fn is_escaped(text: &str) -> bool {
    if text.contains(['<', '>', '"', '\'']) {
        return false;
    }
    text.match_indices('&').all(|(at, _)| {
        let rest = &text[at..];
        ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"].iter().any(|e| rest.starts_with(e))
    })
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let node = NodeIdentity::standalone();
    let hubs = HubSessions::new();

    let _ = room_kind_of(&text, &node, &hubs, 0);
    let _ = include_pending(Some(&text));
    let _ = Jid::new(&text, XMPP_DOMAIN, None);

    for kind in [IdKind::Party, IdKind::Hunt] {
        let read = instance_id(&text, kind, &node.node);
        if text.split('_').next().unwrap_or("").len() > MAX_ID_TEXT {
            assert!(read.is_none());
        }
        if let Some(id) = read {
            assert_eq!(id.kind(), kind);
            assert_eq!(instance_id(&room_id_text(&id), kind, &node.node), Some(id));
        }
    }

    if let Ok(body) = serde_json::from_str::<Value>(&text) {
        let answer = slayer_link_availability(Some(&body));
        let listed = answer["availability"].as_array().map_or(0, |list| {
            for entry in list {
                let units = entry["account_id"].as_str().map_or(0, |id| id.encode_utf16().count());
                assert!((1..=128).contains(&units));
            }
            list.len()
        });
        assert!(listed <= SLAYER_LINK_MAX_ACCOUNTS);
    }

    let (user, rest) = text.split_at(text.floor_char_boundary(text.len() / 2));
    let (event, body) = rest.split_at(rest.floor_char_boundary(rest.len() / 2));
    let stanza = frame(user, event, body);
    let parts = stanza_parts(&stanza);
    assert!(parts.is_some_and(|(to, id, inner)| is_escaped(to) && is_escaped(id) && is_escaped(inner)));
});

/// The `to` local part, the id and the body: the first `"` after an attribute's opening one closes it.
fn stanza_parts(stanza: &str) -> Option<(&str, &str, &str)> {
    let head = format!("<message type=\"headline\" from=\"{HEADLINE_FROM}\" to=\"");
    let (to, rest) = stanza.strip_prefix(&head)?.split_once('"')?;
    let to = to.strip_suffix(&format!("@{XMPP_DOMAIN}"))?;
    let (id, rest) = rest.strip_prefix(" id=\"")?.split_once('"')?;
    let inner = rest.strip_prefix("><body>")?.strip_suffix("</body></message>")?;
    Some((to, id, inner))
}
