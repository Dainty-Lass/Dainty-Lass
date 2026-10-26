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

    pub fn local(&self) -> &str {
        &self.local
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

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
    Network,
    Region(Region),
    Party(GlobalId),
    Hunt(GlobalId),
}

/// What a player may do in a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomRules {
    pub may_join: bool,
    pub may_speak: bool,
    /// Whether the room's history is kept with its owner
    pub history: bool,
}

/// What the chat plugin asks a game module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatQuery {
    /// The JID a player has in this game
    Jid { account: GlobalId },
    /// What a player may do in a room
    Room { room: RoomKind, account: GlobalId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatAnswer {
    Jid(Jid),
    Room(RoomRules),
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
}
