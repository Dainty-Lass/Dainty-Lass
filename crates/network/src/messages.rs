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
        /// The new account.
        account: GlobalId,
        /// The name it claims.
        username: Username,
        /// Its home node.
        home: NodeId,
    },
    /// A joining server's account whose username clashed with an earlier-registered account and was given a
    /// temporary suffixed name (DN13); prompts for a free rename at next login
    RegisterClash {
        /// The joining account.
        account: GlobalId,
        /// The temporary suffixed name it was given.
        username: Username,
        /// Its home node.
        home: NodeId,
    },
    /// An account's new username; the old one is free from this entry on
    Rename {
        /// The account renamed.
        account: GlobalId,
        /// Its new name.
        username: Username,
    },
    /// An existing account whose username clashed with an earlier-registered joining account and was given a
    /// temporary suffixed name (DN13); prompts for a free rename at next login
    RenameClash {
        /// The account renamed.
        account: GlobalId,
        /// The temporary suffixed name it was given.
        username: Username,
    },
    /// A newly elected leader took the log over in the entry's term; changes no account. Raft's no-op (8): a
    /// leader acknowledges nothing before one of its own term is on a majority (`docs/distributed-mode.md`)
    Lead {
        /// The leader.
        node: NodeId,
    },
}

impl DirectoryChange {
    /// The account the change is about; none for [`Self::Lead`].
    pub fn account(&self) -> Option<&GlobalId> {
        match self {
            Self::Register { account, .. }
            | Self::RegisterClash { account, .. }
            | Self::Rename { account, .. }
            | Self::RenameClash { account, .. } => Some(account),
            Self::Lead { .. } => None,
        }
    }

    /// The username the change gives it; none for [`Self::Lead`].
    pub fn username(&self) -> Option<&Username> {
        match self {
            Self::Register { username, .. }
            | Self::RegisterClash { username, .. }
            | Self::Rename { username, .. }
            | Self::RenameClash { username, .. } => Some(username),
            Self::Lead { .. } => None,
        }
    }

    /// The home node, for a registration.
    pub fn home(&self) -> Option<&NodeId> {
        match self {
            Self::Register { home, .. } | Self::RegisterClash { home, .. } => Some(home),
            Self::Rename { .. } | Self::RenameClash { .. } | Self::Lead { .. } => None,
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
    /// The account's global id.
    pub account: GlobalId,
    /// Its username on the joining server, checked by the leader.
    pub username: String,
    /// When the account was registered on its server, in unix milliseconds; the earlier registration keeps
    /// the name on a clash
    pub registered_at: i64,
}

/// An account whose username clashed during a server join and was assigned a temporary suffixed name (DN13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClashedAccount {
    /// The account renamed.
    pub account: GlobalId,
    /// Its home node.
    pub home: NodeId,
    /// The name it held.
    pub old_username: Username,
    /// The temporary suffixed name it now holds.
    pub new_username: Username,
}

/// What joining an existing server's accounts to the directory did (DN13).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinOutcome {
    /// The log entries the join wrote, registrations and clash renames alike.
    pub entries: Vec<DirectoryEntry>,
    /// The accounts given a temporary name.
    pub clashed: Vec<ClashedAccount>,
}

/// An entry of the log, numbered by the leader from 1 without gaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryEntry {
    /// The entry's number.
    pub seq: u64,
    /// When the leader wrote it, in unix milliseconds
    pub at: i64,
    /// The election term it was written in; 0 outside failover, and left off the wire then
    #[serde(default, skip_serializing_if = "is_zero")]
    pub term: u64,
    /// What it changed.
    pub change: DirectoryChange,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// A directory call as sent: [`DirectoryCall`] with the protocol version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryRequest {
    /// [`DIRECTORY_VERSION`].
    pub version: u32,
    /// The call.
    pub call: DirectoryCall,
}

/// What a node asks the directory's leader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectoryCall {
    /// From the account's home. `username` is checked by the leader, so a refusal says why. `actor` is
    /// who asked at the home, as `store`'s change log writes it, recorded in the leader's log too.
    Register {
        /// The account, homed on the caller.
        account: GlobalId,
        /// The username asked for.
        username: String,
        /// Who asked, as the change log writes actors.
        actor: String,
    },
    /// From the account's home.
    Rename {
        /// The account, homed on the caller.
        account: GlobalId,
        /// The new username.
        username: String,
        /// Who asked, as the change log writes actors.
        actor: String,
    },
    /// Join an existing server's accounts to the directory with the caller as their home (DN13).
    Join {
        /// The accounts to register.
        accounts: Vec<JoiningAccount>,
        /// Who asked, as the change log writes actors.
        actor: String,
    },
    /// The entries after `after`, at most `limit` (capped at [`MAX_PULL`]). `last` is the puller's entry
    /// `after`, which the leader must hold the same, or the two logs have diverged.
    Pull {
        /// The number of the puller's newest entry.
        after: u64,
        /// The puller's entry `after`, none when `after` is 0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last: Option<DirectoryEntry>,
        /// The most entries wanted.
        limit: u32,
    },
    /// From the leader of `term` under failover: pull from me now and say how far you got. A write is
    /// acknowledged once a majority answers with its entry (`docs/distributed-mode.md`)
    Notify {
        /// The caller's term as leader.
        term: u64,
    },
}

/// A directory answer as sent: [`DirectoryReply`] with the protocol version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryResponse {
    /// [`DIRECTORY_VERSION`].
    pub version: u32,
    /// The answer.
    pub reply: DirectoryReply,
}

/// The leader's answer to a [`DirectoryCall`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectoryReply {
    /// The entry a register or rename wrote (or wrote before, for a register repeated after a lost answer)
    Applied(DirectoryEntry),
    /// The entries and clash renames a server join wrote (DN13)
    Joined(JoinOutcome),
    /// A page of the log, and the leader's newest entry
    Changes {
        /// The entries, oldest first.
        entries: Vec<DirectoryEntry>,
        /// The leader's newest entry.
        head: u64,
    },
    /// A replica's answer to [`DirectoryCall::Notify`]: its newest entry after pulling
    Caught {
        /// The replica's newest entry.
        head: u64,
    },
    /// The call was refused.
    Refused(Refusal),
}

/// Why the directory said no. Crosses the link, so a home can tell its player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Refusal {
    /// The username fails the shared username rules.
    #[error("not a username ({})", .check.as_str())]
    InvalidUsername {
        /// The rule it fails.
        check: UsernameCheck,
    },
    /// Another account holds the name, in any case
    #[error("the username is taken")]
    UsernameTaken,
    /// The account is already registered.
    #[error("the account is already registered")]
    AccountExists,
    /// The directory holds no such account.
    #[error("the directory has no such account")]
    UnknownAccount,
    /// The id is not an account's.
    #[error("that is not an account id")]
    NotAnAccount,
    /// Only an account's home registers or renames it
    #[error("only the account's home node can change it")]
    NotHome,
    /// The node asked is not the leader.
    #[error("this node is not the directory's leader")]
    NotLeader {
        /// The leader it knows of, if any.
        leader: Option<NodeId>,
    },
    /// The puller's log isn't a prefix of the leader's
    #[error("the logs have diverged")]
    Diverged,
    /// The actor is not one the change log can record.
    #[error("not an actor the change log can record")]
    BadActor,
    /// The request's version is not the leader's.
    #[error("directory protocol {supported} only")]
    UnsupportedVersion {
        /// The version the leader speaks.
        supported: u32,
    },
    /// The message is not a directory message.
    #[error("not a directory message")]
    Malformed,
    /// The leader failed to answer (its database); ask again later
    #[error("the directory can't answer just now")]
    Unavailable,
    /// A leader of an older term called: a newer one has been elected since
    #[error("a leader of term {term} has been elected since")]
    Superseded {
        /// The newest term the answering node knows.
        term: u64,
    },
    /// The leader wrote the change but a majority of the nodes doesn't hold it yet; ask again, and the same
    /// change answers once they do
    #[error("the change isn't on a majority of the nodes yet")]
    NoQuorum,
}

/// Why a directory message could not be decoded or encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Over the node link's message limit.
    #[error("a node-link message is over {MAX_MESSAGE} bytes")]
    TooLarge,
    /// Another protocol version; holds the one received.
    #[error("directory protocol {0}")]
    Version(u32),
    /// Not the expected JSON.
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

/// A request decoded; refuses another version.
pub fn decode_request(bytes: &[u8]) -> Result<DirectoryRequest, DecodeError> {
    decode(bytes, |r: &DirectoryRequest| r.version)
}

/// A response decoded; refuses another version.
pub fn decode_response(bytes: &[u8]) -> Result<DirectoryResponse, DecodeError> {
    decode(bytes, |r: &DirectoryResponse| r.version)
}

/// `message` as JSON; errors when over the node link's message limit.
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, DecodeError> {
    let bytes = serde_json::to_vec(message).map_err(|e| DecodeError::Json(e.to_string()))?;
    if bytes.len() > MAX_MESSAGE {
        return Err(DecodeError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn messages_have_one_spelling() {
        let account: GlobalId = "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@germany-1".parse().unwrap();
        let entry = DirectoryEntry {
            seq: 7,
            at: 1_790_000_000_000,
            term: 0,
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
