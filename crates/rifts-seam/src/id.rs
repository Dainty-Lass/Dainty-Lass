// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Network-wide ids: a ULID tagged with the node that minted it (DD18).

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

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const ULID_LEN: usize = 26;
const RANDOM_BITS: u32 = 80;
const RANDOM_MASK: u128 = (1 << RANDOM_BITS) - 1;
const MILLIS_MASK: u64 = (1 << 48) - 1;

/// 48 bits of Unix milliseconds over 80 random bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ulid(pub u128);

impl Ulid {
    pub const fn from_parts(unix_millis: u64, random: u128) -> Self {
        Self((((unix_millis & MILLIS_MASK) as u128) << RANDOM_BITS) | (random & RANDOM_MASK))
    }

    pub const fn unix_millis(self) -> u64 {
        (self.0 >> RANDOM_BITS) as u64
    }
}

impl fmt::Display for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = [0u8; ULID_LEN];
        let mut rest = self.0;
        for slot in text.iter_mut().rev() {
            *slot = CROCKFORD[(rest & 31) as usize];
            rest >>= 5;
        }
        // Every byte comes from the ASCII alphabet.
        f.write_str(std::str::from_utf8(&text).map_err(|_| fmt::Error)?)
    }
}

impl FromStr for Ulid {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        let bytes = text.as_bytes();
        // 26 digits hold 130 bits, so the first may only carry the top 3 of the 128.
        if bytes.len() != ULID_LEN || !(b'0'..=b'7').contains(&bytes[0]) {
            return Err(IdError::BadUlid);
        }
        let mut value: u128 = 0;
        for byte in bytes {
            let upper = byte.to_ascii_uppercase();
            let digit = CROCKFORD.iter().position(|&c| c == upper).ok_or(IdError::BadUlid)?;
            value = (value << 5) | digit as u128;
        }
        Ok(Self(value))
    }
}

/// The name of a node in the network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(name: &str) -> Result<Self, IdError> {
        let bytes = name.as_bytes();
        let allowed = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-';
        let valid = (1..=32).contains(&bytes.len())
            && bytes.iter().all(allowed)
            && !name.starts_with('-')
            && !name.ends_with('-');
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

macro_rules! global_newtype {
    ($($(#[$meta:meta])* $name:ident),* $(,)?) => {$(
        $(#[$meta])*
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

global_newtype!(
    /// A hunt session.
    SessionId,
    PartyId,
    /// An admission ticket.
    TicketId,
    RealmId,
    /// A peer's admission into a session.
    AdmissionId,
);

/// An account as the module names it; no format is imposed here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountId(pub String);

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A transport peer id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_text_round_trip() {
        let ulid = Ulid::from_parts(1_793_000_000_123, 0x1234_5678_9abc_def0_1234);
        assert_eq!(ulid.unix_millis(), 1_793_000_000_123);
        assert_eq!(ulid.0 & RANDOM_MASK, 0x1234_5678_9abc_def0_1234);
        let text = ulid.to_string();
        assert_eq!(text.len(), 26);
        assert_eq!(text.parse::<Ulid>(), Ok(ulid));
        assert_eq!(text.to_ascii_lowercase().parse::<Ulid>(), Ok(ulid));
        assert_eq!(Ulid(0).to_string(), "00000000000000000000000000");
        assert_eq!(Ulid(u128::MAX).to_string(), "7ZZZZZZZZZZZZZZZZZZZZZZZZZ");
        assert_eq!("7ZZZZZZZZZZZZZZZZZZZZZZZZZ".parse::<Ulid>(), Ok(Ulid(u128::MAX)));
    }

    #[test]
    fn from_parts_drops_bits_out_of_range() {
        let ulid = Ulid::from_parts(u64::MAX, u128::MAX);
        assert_eq!(ulid, Ulid(u128::MAX));
        assert_eq!(ulid.unix_millis(), (1 << 48) - 1);
        assert_eq!(Ulid::from_parts(1, 0), Ulid(1 << 80));
    }

    #[test]
    fn ulid_refusals() {
        for bad in [
            "8ZZZZZZZZZZZZZZZZZZZZZZZZZ",
            "01ARZ3NDEKTSV4RRFFQ69G5FAU",
            "01ARZ3NDEKTSV4RRFFQ69G5FA",
            "01ARZ3NDEKTSV4RRFFQ69G5FAVV",
            "01ARZ3NDEKTSV4RRFFQ69G5FAI",
            "01ARZ3NDEKTSV4RRFFQ69G5FAL",
            "01ARZ3NDEKTSV4RRFFQ69G5FAO",
            "",
        ] {
            assert_eq!(bad.parse::<Ulid>(), Err(IdError::BadUlid), "{bad}");
        }
        assert!("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<Ulid>().is_ok());
    }

    #[test]
    fn node_ids() {
        for good in ["a", "node-1", "0", "abcdefghijklmnopqrstuvwxyz012345"] {
            assert_eq!(NodeId::new(good).map(|n| n.as_str().to_owned()), Ok(good.to_owned()));
        }
        for bad in [
            "",
            "-x",
            "x-",
            "New-York",
            "a_b",
            "a.b",
            "abcdefghijklmnopqrstuvwxyz0123456",
        ] {
            assert_eq!(NodeId::new(bad), Err(IdError::BadNode), "{bad}");
        }
    }

    #[test]
    fn global_ids() {
        let text = "01ARZ3NDEKTSV4RRFFQ69G5FAV@eu-1";
        let id: GlobalId = text.parse().unwrap();
        assert_eq!(id.node.as_str(), "eu-1");
        assert_eq!(id.to_string(), text);
        let session: SessionId = text.parse().unwrap();
        assert_eq!(session.0, id);
        assert_eq!(session.to_string(), text);
        assert_eq!(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<GlobalId>(),
            Err(IdError::BadGlobal)
        );
        assert_eq!("01ARZ3NDEKTSV4RRFFQ69G5FA@eu".parse::<RealmId>(), Err(IdError::BadUlid));
        assert_eq!(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV@-x".parse::<PartyId>(),
            Err(IdError::BadNode)
        );
        assert_eq!(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV@New-York".parse::<TicketId>(),
            Err(IdError::BadNode)
        );
        assert_eq!(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV@a@b".parse::<AdmissionId>(),
            Err(IdError::BadNode)
        );
    }

    #[test]
    fn error_texts() {
        assert_eq!(IdError::BadUlid.to_string(), "a ULID is 26 Crockford base32 characters");
        assert_eq!(
            IdError::BadNode.to_string(),
            "a node id is 1 to 32 of a-z, 0-9 and '-', not starting or ending with '-'"
        );
        assert_eq!(IdError::BadGlobal.to_string(), "a global id is <ULID>@<node>");
        assert_eq!(AccountId("acct-1".into()).to_string(), "acct-1");
    }
}
