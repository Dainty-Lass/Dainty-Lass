//! The party events the client reads as XMPP headlines (`realtime/PartyOutboxDispatcher.ts`): a `<message
//! type="headline">` from `party-events@` to each recipient, whose body is the JSON of [`public_event`]. The outbox
//! (claiming, publishing, marking) is the party area's; this is the wire shape, byte for byte, and the event the
//! module hands the plugin to deliver.
//!
//! Frame, as Node writes it, with the body XML-escaped and `to` one recipient's `userId`:
//! `<message type="headline" from="party-events@prod.ol.epicgames.com" to="{userId}@prod.ol.epicgames.com"
//! id="{eventId}"><body>{body}</body></message>`.

use module_api::GlobalId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The event a module emits for the plugin to deliver as a headline; registered under this name (DF2).
pub const HEADLINE_EVENT: &str = "isles.party.headline";

/// `from` of every headline, a bare JID.
pub const HEADLINE_FROM: &str = "party-events@prod.ol.epicgames.com";

/// `PartyOutboxRecord`'s public face, as the plugin needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Headline {
    /// The outbox record's `eventId`: the stanza id
    pub id: String,
    /// The recipients' accounts and their `userId`s, which the JIDs carry
    pub recipients: Vec<Recipient>,
    /// The JSON text of [`public_event`]
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub account: GlobalId,
    pub user_id: String,
}

/// The public name of an outbox event type; `None` for a type Paradox has no name for (its JSON then leaves the
/// field out, as `JSON.stringify` drops `undefined`).
pub fn event_name(event_type: &str) -> Option<&'static str> {
    Some(match event_type {
        "PARTY_CREATED" => "party.created",
        "PARTY_INVITE_CREATED" => "party.invited",
        "PARTY_MEMBERSHIP_CHANGED" => "party.membership_changed",
        "PARTY_LEADER_CHANGED" => "party.leader_changed",
        "PARTY_ACTIVITY_CHANGED" => "party.activity_changed",
        "PARTY_PRESENCE_CHANGED" => "party.presence_changed",
        "PARTY_HUNT_STATUS_CHANGED" => "party.hunt_status_changed",
        _ => return None,
    })
}

/// `PublicEvent(record)`, in Node's key order.
pub fn public_event(
    event_id: &str,
    event_type: &str,
    occurred_at: &str,
    party_id: &str,
    party_revision: i64,
    payload: &Value,
) -> Value {
    let mut event = Map::new();
    event.insert("eventId".into(), json!(event_id));
    if let Some(name) = event_name(event_type) {
        event.insert("eventType".into(), json!(name));
    }
    event.insert("schemaVersion".into(), json!(1));
    event.insert("occurredAt".into(), json!(occurred_at));
    event.insert("partyId".into(), json!(party_id));
    event.insert("partyRevision".into(), json!(party_revision));
    event.insert("payload".into(), payload.clone());
    Value::Object(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_event_is_nodes_json() {
        let event = public_event(
            "e1",
            "PARTY_INVITE_CREATED",
            "2026-10-20T12:00:00.000Z",
            "p1_QnVpbGQ=",
            3,
            &json!({ "accountId": "bob", "action": "KICK" }),
        );
        assert_eq!(
            event.to_string(),
            r#"{"eventId":"e1","eventType":"party.invited","schemaVersion":1,"occurredAt":"2026-10-20T12:00:00.000Z","partyId":"p1_QnVpbGQ=","partyRevision":3,"payload":{"accountId":"bob","action":"KICK"}}"#
        );
        // an event type with no public name has no eventType key, as `undefined` has none in JSON.stringify
        let unnamed = public_event("e2", "PARTY_SOMETHING", "t", "p", 0, &json!({}));
        assert!(unnamed.get("eventType").is_none());
        assert_eq!(
            unnamed.to_string(),
            r#"{"eventId":"e2","schemaVersion":1,"occurredAt":"t","partyId":"p","partyRevision":0,"payload":{}}"#
        );
        for (kind, name) in [
            ("PARTY_CREATED", "party.created"),
            ("PARTY_MEMBERSHIP_CHANGED", "party.membership_changed"),
            ("PARTY_LEADER_CHANGED", "party.leader_changed"),
            ("PARTY_ACTIVITY_CHANGED", "party.activity_changed"),
            ("PARTY_PRESENCE_CHANGED", "party.presence_changed"),
            ("PARTY_HUNT_STATUS_CHANGED", "party.hunt_status_changed"),
        ] {
            assert_eq!(event_name(kind), Some(name));
        }
    }

    #[test]
    fn a_headline_crosses_the_event_bus_as_json() {
        let node = "oregon-1".parse().unwrap();
        let headline = Headline {
            id: "e1".into(),
            recipients: vec![Recipient {
                account: GlobalId::new(module_api::IdKind::Account, &node),
                user_id: "alice".into(),
            }],
            body: "{}".into(),
        };
        let back: Headline = serde_json::from_value(serde_json::to_value(&headline).unwrap()).unwrap();
        assert_eq!(back, headline);
    }
}
