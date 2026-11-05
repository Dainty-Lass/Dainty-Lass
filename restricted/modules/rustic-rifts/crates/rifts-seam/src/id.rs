//! Ids that cross nodes (DD18): a ULID tagged with the node that owns the thing it names.
//!
//! Local stand-ins until DF2 publishes `module-api` with the network seams (DN2); then these become re-exports of its
//! types and the text form follows whatever it chose. Nothing here is persisted yet, so the switch costs no migration.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("a ULID is 26 Crockford base32 characters")]
    BadUlid,
    #[error("a node id is 1 to 32 of a-z, 0-9 and '-', not starting or ending with '-'")]
    BadNode,
    #[error("a global id is <ULID>@<node>")]
    BadGlobal,
}

/// 128 bits: 48 of Unix milliseconds, then 80 random. Generation is `module-api`'s (DF2); this crate only carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ulid(pub u128);

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

impl Ulid {
    pub const fn from_parts(unix_millis: u64, random: u128) -> Self {
        Self(((unix_millis as u128 & 0xFFFF_FFFF_FFFF) << 80) | (random & ((1 << 80) - 1)))
    }

    pub const fn unix_millis(self) -> u64 {
        (self.0 >> 80) as u64
    }
}

impl fmt::Display for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = [0u8; 26];
        for (i, slot) in text.iter_mut().enumerate() {
            let shift = 5 * (25 - i);
            *slot = CROCKFORD[((self.0 >> shift) & 31) as usize];
        }
        f.write_str(std::str::from_utf8(&text).map_err(|_| fmt::Error)?)
    }
}

impl FromStr for Ulid {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        let bytes = text.as_bytes();
        // 26 characters hold 130 bits, so the first may only be 0 to 7
        if bytes.len() != 26 || !matches!(bytes[0], b'0'..=b'7') {
            return Err(IdError::BadUlid);
        }
        let mut value = 0u128;
        for &byte in bytes {
            let upper = byte.to_ascii_uppercase();
            let digit = CROCKFORD.iter().position(|&c| c == upper).ok_or(IdError::BadUlid)?;
            value = (value << 5) | digit as u128;
        }
        Ok(Self(value))
    }
}

/// The node that owns a global id: a region node, or the one server of a host outside a network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(name: &str) -> Result<Self, IdError> {
        let valid = (1..=32).contains(&name.len())
            && !name.starts_with('-')
            && !name.ends_with('-')
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(IdError::BadNode)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `<ULID>@<node>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalId {
    pub ulid: Ulid,
    pub node: NodeId,
}

impl fmt::Display for GlobalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.ulid, self.node)
    }
}

impl FromStr for GlobalId {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        let (ulid, node) = text.split_once('@').ok_or(IdError::BadGlobal)?;
        Ok(Self {
            ulid: ulid.parse()?,
            node: NodeId::new(node)?,
        })
    }
}

macro_rules! global_ids {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub GlobalId);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, IdError> {
                text.parse().map(Self)
            }
        }
    )*};
}

global_ids! {
    /// A hunt or a realm hub generation's session (`p2pHuntSessions.sessionId`). Node's is a 36-character UUID, which
    /// the runtime's launch check and the transport's `bridge.conf` expect; the wire form is RF5's and RF7's call.
    SessionId,
    /// A party (DD18).
    PartyId,
    /// A matchmaking ticket (DD18).
    TicketId,
    /// A realm; owned by the node that holds its record.
    RealmId,
    /// One admitted peer of a session (`p2pPeerAdmissions.admissionId`).
    AdmissionId,
}

/// An account. DD17 gives accounts global ids owned by their home; RU4's seams and DN2 define them, so this stays
/// the backend's account id text until then.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountId(pub String);

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The transport's 64-bit peer id (`host_peer_id`, `AddPeerH`); local to a session, not a global id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_round_trips_through_text() {
        let ulid = Ulid::from_parts(1_790_000_000_000, 0x1234_5678_9ABC_DEF0_1234);
        let text = ulid.to_string();
        assert_eq!(text.len(), 26);
        assert_eq!(text.parse::<Ulid>(), Ok(ulid));
        assert_eq!(text.to_ascii_lowercase().parse::<Ulid>(), Ok(ulid));
        assert_eq!(ulid.unix_millis(), 1_790_000_000_000);
        assert_eq!(Ulid(u128::MAX).to_string(), "7ZZZZZZZZZZZZZZZZZZZZZZZZZ");
    }

    #[test]
    fn ulid_refuses_overflow_and_foreign_letters() {
        assert_eq!("8ZZZZZZZZZZZZZZZZZZZZZZZZZ".parse::<Ulid>(), Err(IdError::BadUlid));
        assert_eq!("01ARZ3NDEKTSV4RRFFQ69G5FAU".parse::<Ulid>(), Err(IdError::BadUlid));
        assert_eq!("01ARZ3NDEKTSV4RRFFQ69G5FA".parse::<Ulid>(), Err(IdError::BadUlid));
    }

    #[test]
    fn global_id_carries_its_node() {
        let id: SessionId = "01ARZ3NDEKTSV4RRFFQ69G5FAV@oregon".parse().unwrap();
        assert_eq!(id.0.node.as_str(), "oregon");
        assert_eq!(id.to_string(), "01ARZ3NDEKTSV4RRFFQ69G5FAV@oregon");
        assert_eq!(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<GlobalId>(),
            Err(IdError::BadGlobal)
        );
        assert_eq!(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV@-x".parse::<GlobalId>(),
            Err(IdError::BadNode)
        );
        assert_eq!(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV@New-York".parse::<GlobalId>(),
            Err(IdError::BadNode)
        );
    }
}
