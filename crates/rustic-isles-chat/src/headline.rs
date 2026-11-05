// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Party headlines (spec §4.4, §5.11): the public event body the party outbox sends, and the stanza a deliverer
//! writes for each recipient connection.

use serde_json::{Map, Value};

pub use module_api::chat::{HEADLINE_EVENT, Headline, HeadlineRecipient};

/// The outbox's event types and the names the 1.14.7 client reads.
const NAMES: [(&str, &str); 7] = [
    ("PARTY_CREATED", "party.created"),
    ("PARTY_INVITE_CREATED", "party.invited"),
    ("PARTY_MEMBERSHIP_CHANGED", "party.membership_changed"),
    ("PARTY_LEADER_CHANGED", "party.leader_changed"),
    ("PARTY_ACTIVITY_CHANGED", "party.activity_changed"),
    ("PARTY_PRESENCE_CHANGED", "party.presence_changed"),
    ("PARTY_HUNT_STATUS_CHANGED", "party.hunt_status_changed"),
];

pub fn event_name(event_type: &str) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|(stored, _)| *stored == event_type)
        .map(|(_, name)| *name)
}

/// The body, keys in Node's order; a type with no public name has no `eventType` (`JSON.stringify` drops
/// `undefined`).
pub fn public_event(
    event_id: &str,
    event_type: &str,
    occurred_at: &str,
    party_id: &str,
    revision: i64,
    payload: &Value,
) -> Value {
    let mut out = Map::new();
    out.insert("eventId".into(), event_id.into());
    if let Some(name) = event_name(event_type) {
        out.insert("eventType".into(), name.into());
    }
    out.insert("schemaVersion".into(), 1.into());
    out.insert("occurredAt".into(), occurred_at.into());
    out.insert("partyId".into(), party_id.into());
    out.insert("partyRevision".into(), revision.into());
    out.insert("payload".into(), payload.clone());
    Value::Object(out)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Node's frame: no `xmlns`, every value escaped.
pub fn frame(user_id: &str, event_id: &str, body: &str) -> String {
    format!(
        "<message type=\"headline\" from=\"{}\" to=\"{}@{}\" id=\"{}\"><body>{}</body></message>",
        crate::HEADLINE_FROM,
        escape(user_id),
        crate::XMPP_DOMAIN,
        escape(event_id),
        escape(body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_events_in_node_order() {
        let body = public_event(
            "e1",
            "PARTY_INVITE_CREATED",
            "2026-10-20T12:00:00.000Z",
            "p1_QnVpbGQ=",
            3,
            &json!({"accountId": "bob", "action": "KICK"}),
        );
        assert_eq!(
            service_kit::jsjson::stringify(&body),
            r#"{"eventId":"e1","eventType":"party.invited","schemaVersion":1,"occurredAt":"2026-10-20T12:00:00.000Z","partyId":"p1_QnVpbGQ=","partyRevision":3,"payload":{"accountId":"bob","action":"KICK"}}"#
        );
        let unknown = public_event("e2", "PARTY_GONE", "t", "p", 1, &json!(null));
        assert_eq!(
            service_kit::jsjson::stringify(&unknown),
            r#"{"eventId":"e2","schemaVersion":1,"occurredAt":"t","partyId":"p","partyRevision":1,"payload":null}"#
        );
    }

    #[test]
    fn every_name() {
        let names: Vec<_> = NAMES.iter().map(|(t, _)| event_name(t)).collect();
        assert_eq!(
            names,
            [
                Some("party.created"),
                Some("party.invited"),
                Some("party.membership_changed"),
                Some("party.leader_changed"),
                Some("party.activity_changed"),
                Some("party.presence_changed"),
                Some("party.hunt_status_changed"),
            ]
        );
        assert_eq!(event_name("party_created"), None);
    }

    #[test]
    fn headlines_round_trip_and_frames_escape() {
        let headline = Headline {
            id: "e1".into(),
            recipients: vec![HeadlineRecipient {
                account: "account:01J00000000000000000000000@local".parse().unwrap(),
                user_id: "u1".into(),
            }],
            body: r#"{"a":"<&>"}"#.into(),
        };
        let text = serde_json::to_string(&headline).unwrap();
        assert_eq!(serde_json::from_str::<Headline>(&text).unwrap(), headline);
        assert_eq!(
            frame("a&b", "e<1>", r#"{"q":"it's"}"#),
            "<message type=\"headline\" from=\"party-events@prod.ol.epicgames.com\" \
             to=\"a&amp;b@prod.ol.epicgames.com\" id=\"e&lt;1&gt;\">\
             <body>{&quot;q&quot;:&quot;it&apos;s&quot;}</body></message>"
        );
    }
}
