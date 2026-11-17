//! The chat plugin's hooks (DN10, DD21): a game module that declares the chat capability tells the plugin
//! each player's JID and who may join and speak in each room. Stubs: the plugin (DN10a) is what asks.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::network::{GlobalId, Region};

/// The most bytes a JID part may hold (RFC 7622 caps each part at 1023; ours are names, not prose).
pub const MAX_JID_PART: usize = 255;

/// An XMPP address, `local@domain/resource`, restricted to what a game module needs: no `@`, `/` or
/// whitespace inside a part and no control characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Jid {
    local: String,
    domain: String,
    resource: Option<String>,
}

/// Text that is not a JID this crate accepts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a JID is local@domain or local@domain/resource, each part 1 to {MAX_JID_PART} bytes")]
pub struct JidError;

fn check_part(part: &str) -> Result<(), JidError> {
    let ok = !part.is_empty()
        && part.len() <= MAX_JID_PART
        && !part
            .chars()
            .any(|c| c == '@' || c == '/' || c.is_whitespace() || c.is_control());
    if ok { Ok(()) } else { Err(JidError) }
}

impl Jid {
    /// A JID from its parts, when each part fits.
    pub fn new(local: &str, domain: &str, resource: Option<&str>) -> Result<Self, JidError> {
        check_part(local)?;
        check_part(domain)?;
        if let Some(resource) = resource {
            check_part(resource)?;
        }
        Ok(Self {
            local: local.to_owned(),
            domain: domain.to_owned(),
            resource: resource.map(str::to_owned),
        })
    }

    /// The part before `@`.
    pub fn local(&self) -> &str {
        &self.local
    }

    /// The part between `@` and `/`.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The part after `/`, if any.
    pub fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }
}

impl FromStr for Jid {
    type Err = JidError;

    fn from_str(text: &str) -> Result<Self, JidError> {
        let (local, rest) = text.split_once('@').ok_or(JidError)?;
        match rest.split_once('/') {
            Some((domain, resource)) => Self::new(local, domain, Some(resource)),
            None => Self::new(local, rest, None),
        }
    }
}

impl TryFrom<String> for Jid {
    type Error = JidError;

    fn try_from(text: String) -> Result<Self, JidError> {
        text.parse()
    }
}

impl From<Jid> for String {
    fn from(jid: Jid) -> String {
        jid.to_string()
    }
}

impl fmt::Display for Jid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.local, self.domain)?;
        if let Some(resource) = &self.resource {
            write!(f, "/{resource}")?;
        }
        Ok(())
    }
}

/// The rooms DD21 names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RoomKind {
    /// The whole network's room.
    Network,
    /// One region's room.
    Region(Region),
    /// A party's room, by the party's id.
    Party(GlobalId),
    /// A hunt's room, by the hunt's id.
    Hunt(GlobalId),
}

/// What a player may do in a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomRules {
    /// The player may enter the room.
    pub may_join: bool,
    /// The player may post in the room.
    pub may_speak: bool,
    /// Whether the room's history is kept with its owner
    pub history: bool,
}

/// Event emitted by the chat plugin when an account's online presence changes (`Payload`: [`PresenceEvent`]).
pub const PRESENCE_EVENT: &str = "chat.presence";

/// Payload of [`PRESENCE_EVENT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceEvent {
    /// The player's account.
    pub account: GlobalId,
    /// The player's id inside the game module.
    pub user_id: String,
    /// Whether the player is now online.
    pub online: bool,
}

/// Event emitted by a game module when staff kick, ban or time out a player: chat ends their sessions
/// (`Payload`: [`DisconnectEvent`]).
pub const DISCONNECT_EVENT: &str = "chat.disconnect";

/// Payload of [`DISCONNECT_EVENT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisconnectEvent {
    /// The player's id inside the game module: the JID local part.
    pub user_id: String,
    /// Words for the log.
    pub reason: String,
}

/// Event emitted by a game module to push an XMPP `<message type="headline">` stanza (`Payload`: [`Headline`]).
pub const HEADLINE_EVENT: &str = "isles.party.headline";

/// One recipient of a [`Headline`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadlineRecipient {
    /// The player's account.
    pub account: GlobalId,
    /// The player's id inside the game module.
    pub user_id: String,
}

/// Payload of [`HEADLINE_EVENT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Headline {
    /// The stanza's id.
    pub id: String,
    /// Who it goes to.
    pub recipients: Vec<HeadlineRecipient>,
    /// The message body.
    pub body: String,
}

/// What the chat plugin asks a game module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatQuery {
    /// The JID a player has in this game
    Jid {
        /// The player.
        account: GlobalId,
    },
    /// What a player may do in a room
    Room {
        /// The room.
        room: RoomKind,
        /// The player.
        account: GlobalId,
    },
    /// Verifies a SASL `PLAIN` login token and returns the player's account and display name
    Login {
        /// The SASL authentication identity.
        authcid: String,
        /// The login token the client presented.
        token: String,
    },
    /// Accepted friends of `account` for initial presence fan-out
    Friends {
        /// The player whose friends are asked for.
        account: GlobalId,
    },
    /// Whether `from` may send a direct message to `to` (neither player blocked the other)
    MayWhisper {
        /// The sender.
        from: GlobalId,
        /// The recipient.
        to: GlobalId,
    },
    /// Parses a module-specific MUC room localpart (such as `Party-<id>` or `City-<id>`) into a [`RoomKind`]
    RoomOf {
        /// The room's localpart.
        name: String,
    },
    /// Resolves a JID localpart (`userId` or display name) to a local player's account id on this node
    AccountOf {
        /// The JID localpart.
        local: String,
    },
}

/// A game module's answer to a [`ChatQuery`], the variant matching the query's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatAnswer {
    /// The player's JID.
    Jid(Jid),
    /// What the player may do in the room.
    Room(RoomRules),
    /// The token was valid.
    Login {
        /// The player's account.
        account: GlobalId,
        /// The player's display name.
        name: String,
    },
    /// The player's accepted friends.
    Friends(Vec<GlobalId>),
    /// Whether the whisper may be sent.
    MayWhisper(bool),
    /// The room the localpart names, if any.
    RoomOf(Option<RoomKind>),
    /// The local player the localpart names, if any.
    AccountOf(Option<GlobalId>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jids_round_trip_and_refuse_extra_separators() {
        for good in ["player@isles.local", "player@isles.local/launcher", "x@y"] {
            let jid: Jid = good.parse().unwrap();
            assert_eq!(jid.to_string(), good);
        }
        assert_eq!("a@b".parse::<Jid>().unwrap().resource(), None);
        let long = format!("{}@d", "a".repeat(MAX_JID_PART + 1));
        for bad in [
            "", "@d", "a@", "a@b/", "a@b/c/d", "a@@b", "a b@c", "a@b\n", "nodomain", &long,
        ] {
            assert!(bad.parse::<Jid>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn chat_queries_answers_and_events_round_trip() {
        let account: GlobalId = "account:01J00000000000000000000000@oregon-1".parse().unwrap();
        let other: GlobalId = "account:01J00000000000000000000001@oregon-1".parse().unwrap();
        for query in [
            ChatQuery::Login {
                authcid: String::from("user-1"),
                token: String::from("tok"),
            },
            ChatQuery::Friends {
                account: account.clone(),
            },
            ChatQuery::MayWhisper {
                from: account.clone(),
                to: other.clone(),
            },
            ChatQuery::RoomOf {
                name: String::from("Party-01J00000000000000000000000"),
            },
            ChatQuery::AccountOf {
                local: String::from("user-1"),
            },
        ] {
            let json = serde_json::to_string(&query).unwrap();
            assert_eq!(serde_json::from_str::<ChatQuery>(&json).unwrap(), query);
        }
        for answer in [
            ChatAnswer::Login {
                account: account.clone(),
                name: String::from("Slayer"),
            },
            ChatAnswer::Friends(vec![other.clone()]),
            ChatAnswer::MayWhisper(true),
            ChatAnswer::RoomOf(Some(RoomKind::Network)),
            ChatAnswer::AccountOf(Some(account.clone())),
        ] {
            let json = serde_json::to_string(&answer).unwrap();
            assert_eq!(serde_json::from_str::<ChatAnswer>(&json).unwrap(), answer);
        }
        let presence = PresenceEvent {
            account: account.clone(),
            user_id: String::from("user-1"),
            online: true,
        };
        assert_eq!(
            serde_json::from_str::<PresenceEvent>(&serde_json::to_string(&presence).unwrap()).unwrap(),
            presence
        );
        let headline = Headline {
            id: String::from("h-1"),
            recipients: vec![HeadlineRecipient {
                account,
                user_id: String::from("user-1"),
            }],
            body: String::from("{\"Type\":\"Party.Invite\"}"),
        };
        assert_eq!(
            serde_json::from_str::<Headline>(&serde_json::to_string(&headline).unwrap()).unwrap(),
            headline
        );
    }
}
