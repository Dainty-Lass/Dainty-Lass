//! What the directory's nodes say to each other over a [`crate::link::PeerLink`] (`Service::Directory`):
//! JSON, versioned, every field known. A home asks the leader to register or rename its accounts; a
//! replica pulls the leader's log after the last entry it holds.

use module_api::network::{GlobalId, NodeId};
use protocol::account::UsernameCheck;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::link::MAX_MESSAGE;
use crate::username::Username;

/// The directory protocol's version; a request with another is refused (DD16: versioned so
/// federation can come later).
pub const DIRECTORY_VERSION: u32 = 1;

/// The most entries one pull returns.
pub const MAX_PULL: u32 = 256;

/// One change to the directory: the log every node holds the same copy of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectoryChange {
    /// A new account, homed on the node that owns its id, claiming a username
    Register {
        account: GlobalId,
        username: Username,
        home: NodeId,
    },
    /// A joining server's account whose username clashed with an earlier-registered account and was given a
    /// temporary suffixed name (DN13); prompts for a free rename at next login
    RegisterClash {
        account: GlobalId,
        username: Username,
        home: NodeId,
    },
    /// An account's new username; the old one is free from this entry on
    Rename { account: GlobalId, username: Username },
    /// An existing account whose username clashed with an earlier-registered joining account and was given a
    /// temporary suffixed name (DN13); prompts for a free rename at next login
    RenameClash { account: GlobalId, username: Username },
}

impl DirectoryChange {
    pub fn account(&self) -> &GlobalId {
        match self {
            Self::Register { account, .. }
            | Self::RegisterClash { account, .. }
            | Self::Rename { account, .. }
            | Self::RenameClash { account, .. } => account,
        }
    }

    pub fn username(&self) -> &Username {
        match self {
            Self::Register { username, .. }
            | Self::RegisterClash { username, .. }
            | Self::Rename { username, .. }
            | Self::RenameClash { username, .. } => username,
        }
    }

    pub fn home(&self) -> Option<&NodeId> {
        match self {
            Self::Register { home, .. } | Self::RegisterClash { home, .. } => Some(home),
            Self::Rename { .. } | Self::RenameClash { .. } => None,
        }
    }

    /// Whether this entry gave the account a temporary suffixed name on a join clash (DN13).
    pub fn is_clash(&self) -> bool {
        matches!(self, Self::RegisterClash { .. } | Self::RenameClash { .. })
    }
}

/// An existing server's account joining the network (DN13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoiningAccount {
    pub account: GlobalId,
    pub username: String,
    /// When the account was registered on its server, in unix milliseconds; the earlier registration keeps
    /// the name on a clash
    pub registered_at: i64,
}

/// An account whose username clashed during a server join and was assigned a temporary suffixed name (DN13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClashedAccount {
    pub account: GlobalId,
    pub home: NodeId,
    pub old_username: Username,
    pub new_username: Username,
}

/// What joining an existing server's accounts to the directory did (DN13).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinOutcome {
    pub entries: Vec<DirectoryEntry>,
    pub clashed: Vec<ClashedAccount>,
}

/// An entry of the log, numbered by the leader from 1 without gaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryEntry {
    pub seq: u64,
    /// When the leader wrote it, in unix milliseconds
    pub at: i64,
    pub change: DirectoryChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryRequest {
    pub version: u32,
    pub call: DirectoryCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectoryCall {
    /// From the account's home. `username` is checked by the leader, so a refusal says why. `actor` is
    /// who asked at the home, as `store`'s change log writes it, recorded in the leader's log too.
    Register {
        account: GlobalId,
        username: String,
        actor: String,
    },
    /// From the account's home.
    Rename {
        account: GlobalId,
        username: String,
        actor: String,
    },
    /// Join an existing server's accounts to the directory with the caller as their home (DN13).
    Join {
        accounts: Vec<JoiningAccount>,
        actor: String,
    },
    /// The entries after `after`, at most `limit` (capped at [`MAX_PULL`]). `last` is the puller's entry
    /// `after`, which the leader must hold the same, or the two logs have diverged.
    Pull {
        after: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last: Option<DirectoryEntry>,
        limit: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryResponse {
    pub version: u32,
    pub reply: DirectoryReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectoryReply {
    /// The entry a register or rename wrote (or wrote before, for a register repeated after a lost answer)
    Applied(DirectoryEntry),
    /// The entries and clash renames a server join wrote (DN13)
    Joined(JoinOutcome),
    /// A page of the log, and the leader's newest entry
    Changes {
        entries: Vec<DirectoryEntry>,
        head: u64,
    },
    Refused(Refusal),
}

/// Why the directory said no. Crosses the link, so a home can tell its player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Refusal {
    #[error("not a username ({})", .check.as_str())]
    InvalidUsername { check: UsernameCheck },
    /// Another account holds the name, in any case
    #[error("the username is taken")]
    UsernameTaken,
    #[error("the account is already registered")]
    AccountExists,
    #[error("the directory has no such account")]
    UnknownAccount,
    #[error("that is not an account id")]
    NotAnAccount,
    /// Only an account's home registers or renames it
    #[error("only the account's home node can change it")]
    NotHome,
    #[error("this node is not the directory's leader")]
    NotLeader { leader: Option<NodeId> },
    /// The puller's log isn't a prefix of the leader's
    #[error("the logs have diverged")]
    Diverged,
    #[error("not an actor the change log can record")]
    BadActor,
    #[error("directory protocol {supported} only")]
    UnsupportedVersion { supported: u32 },
    #[error("not a directory message")]
    Malformed,
    /// The leader failed to answer (its database); ask again later
    #[error("the directory can't answer just now")]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("a node-link message is over {MAX_MESSAGE} bytes")]
    TooLarge,
    #[error("directory protocol {0}")]
    Version(u32),
    #[error("{0}")]
    Json(String),
}

/// Something with a version, read before anything else in it.
#[derive(Deserialize)]
struct Versioned {
    version: u32,
}

fn decode<T: DeserializeOwned>(bytes: &[u8], version_of: impl Fn(&T) -> u32) -> Result<T, DecodeError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(DecodeError::TooLarge);
    }
    match serde_json::from_slice::<T>(bytes) {
        Ok(message) if version_of(&message) == DIRECTORY_VERSION => Ok(message),
        Ok(message) => Err(DecodeError::Version(version_of(&message))),
        // A newer peer's message may not parse as this version's: say it's the version, not the shape
        Err(error) => match serde_json::from_slice::<Versioned>(bytes) {
            Ok(Versioned { version }) if version != DIRECTORY_VERSION => Err(DecodeError::Version(version)),
            _ => Err(DecodeError::Json(error.to_string())),
        },
    }
}

pub fn decode_request(bytes: &[u8]) -> Result<DirectoryRequest, DecodeError> {
    decode(bytes, |r: &DirectoryRequest| r.version)
}

pub fn decode_response(bytes: &[u8]) -> Result<DirectoryResponse, DecodeError> {
    decode(bytes, |r: &DirectoryResponse| r.version)
}

pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, DecodeError> {
    let bytes = serde_json::to_vec(message).map_err(|e| DecodeError::Json(e.to_string()))?;
    if bytes.len() > MAX_MESSAGE {
        return Err(DecodeError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn messages_have_one_spelling() {
        let account: GlobalId = "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@germany-1".parse().unwrap();
        let entry = DirectoryEntry {
            seq: 7,
            at: 1_790_000_000_000,
            change: DirectoryChange::Register {
                account: account.clone(),
                username: Username::new("Fox").unwrap(),
                home: NodeId::new("germany-1").unwrap(),
            },
        };
        let pull = DirectoryRequest {
            version: DIRECTORY_VERSION,
            call: DirectoryCall::Pull {
                after: 7,
                last: Some(entry.clone()),
                limit: 100,
            },
        };
        let json = String::from_utf8(encode(&pull).unwrap()).unwrap();
        assert_eq!(
            json,
            r#"{"version":1,"call":{"pull":{"after":7,"last":{"seq":7,"at":1790000000000,"change":{"register":{"account":"account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@germany-1","username":"Fox","home":"germany-1"}}},"limit":100}}}"#
        );
        assert_eq!(decode_request(json.as_bytes()).unwrap(), pull);

        let refused = DirectoryResponse {
            version: DIRECTORY_VERSION,
            reply: DirectoryReply::Refused(Refusal::InvalidUsername {
                check: UsernameCheck::TooLong,
            }),
        };
        let json = String::from_utf8(encode(&refused).unwrap()).unwrap();
        assert_eq!(
            json,
            r#"{"version":1,"reply":{"refused":{"invalid_username":{"check":"too_long"}}}}"#
        );
        assert_eq!(decode_response(json.as_bytes()).unwrap(), refused);
    }

    #[test]
    fn unknown_fields_versions_and_sizes_are_refused() {
        assert_eq!(
            decode_request(br#"{"version":2,"call":{"hello":{}}}"#),
            Err(DecodeError::Version(2))
        );
        assert_eq!(
            decode_request(br#"{"version":2,"call":{"pull":{"after":0,"limit":1}}}"#),
            Err(DecodeError::Version(2))
        );
        assert!(matches!(
            decode_request(br#"{"version":1,"call":{"pull":{"after":0,"limit":1,"extra":1}}}"#),
            Err(DecodeError::Json(_))
        ));
        assert!(matches!(
            decode_response(br#"{"version":1,"reply":{"changes":{"entries":[{"seq":1,"at":0,"change":{"rename":{"account":"account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@a","username":"no way"}}}],"head":1}}}"#),
            Err(DecodeError::Json(_))
        ));
        let big = vec![b' '; MAX_MESSAGE + 1];
        assert_eq!(decode_request(&big), Err(DecodeError::TooLarge));
    }
}
