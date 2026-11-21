// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Network-wide ids: a ULID tagged with the node that minted it (DD18).

use std::{fmt, str::FromStr};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdError {
    BadUlid,
    BadNode,
    BadGlobal,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadGlobal => "a global id is <ULID>@<node>",
            Self::BadNode => "a node id is 1 to 32 of a-z, 0-9 and '-', not starting or ending with '-'",
            Self::BadUlid => "a ULID is 26 Crockford base32 characters",
        })
    }
}

impl std::error::Error for IdError {}

/// Crockford's base32: no I, L, O or U.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const DIGITS: usize = 26;
const TIME_SHIFT: u32 = 80;

/// Each ASCII byte's digit value (either case), or `NONE`.
const NONE: u8 = 0xff;
const DIGIT_OF: [u8; 128] = {
    let mut table = [NONE; 128];
    let mut value = 0;
    while value < ALPHABET.len() {
        let upper = ALPHABET[value];
        table[upper as usize] = value as u8;
        table[upper.to_ascii_lowercase() as usize] = value as u8;
        value += 1;
    }
    table
};

/// 48 bits of Unix milliseconds over 80 random bits.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Ulid(pub u128);

impl Ulid {
    /// Bits beyond 48 of `unix_millis` or 80 of `random` are dropped.
    pub const fn from_parts(unix_millis: u64, random: u128) -> Self {
        let time = (unix_millis as u128) << TIME_SHIFT;
        let random = random << (128 - TIME_SHIFT) >> (128 - TIME_SHIFT);
        Self(time | random)
    }

    pub const fn unix_millis(self) -> u64 {
        (self.0 >> TIME_SHIFT) as u64
    }
}

impl fmt::Display for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = (0..DIGITS)
            .rev()
            .map(|place| char::from(ALPHABET[((self.0 >> (5 * place)) & 0x1f) as usize]))
            .collect();
        f.write_str(&text)
    }
}

impl FromStr for Ulid {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        // 26 digits hold 130 bits, so the first may only carry the top 3 of the 128.
        let leading_ok = matches!(text.as_bytes().first(), Some(b'0'..=b'7'));
        if text.len() != DIGITS || !leading_ok {
            return Err(IdError::BadUlid);
        }
        text.bytes().try_fold(Self(0), |Self(acc), byte| {
            let digit = DIGIT_OF.get(usize::from(byte)).copied().filter(|d| *d != NONE);
            digit.map(|d| Self(acc << 5 | u128::from(d))).ok_or(IdError::BadUlid)
        })
    }
}

/// The name of a node in the network.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(name: &str) -> Result<Self, IdError> {
        let charset = name.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
        let edges = !(name.starts_with('-') || name.ends_with('-'));
        match name.len() {
            1..=32 if charset && edges => Ok(Self(name.into())),
            _ => Err(IdError::BadNode),
        }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// `<ULID>@<node>`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GlobalId {
    pub ulid: Ulid,
    pub node: NodeId,
}

impl FromStr for GlobalId {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        let Some((ulid, node)) = text.split_once('@') else {
            return Err(IdError::BadGlobal);
        };
        let ulid = Ulid::from_str(ulid)?;
        NodeId::new(node).map(|node| Self { ulid, node })
    }
}

/// A transport peer id.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerId(pub u64);

/// An account as the module names it; no format is imposed here.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AccountId(pub String);

// Ids shown as their text; the ULID-backed ones also parse back.
macro_rules! shown_as {
    ($($name:ty => |$it:ident| $text:expr;)*) => {$(
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let $it = self;
                fmt::Display::fmt($text, f)
            }
        }
    )*};
}

shown_as! {
    NodeId => |id| id.as_str();
    AccountId => |id| id.0.as_str();
    GlobalId => |id| &format_args!("{}@{}", id.ulid, id.node);
}

macro_rules! global_newtype {
    ($($(#[$meta:meta])* $name:ident),* $(,)?) => {$(
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub GlobalId);

        shown_as! { $name => |id| &id.0; }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, IdError> {
                GlobalId::from_str(text).map(Self)
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

    #[test]
    fn ulid_text_round_trip() {
        let ulid = Ulid::from_parts(1_793_000_000_123, 0x1234_5678_9abc_def0_1234);
        assert_eq!(ulid.unix_millis(), 1_793_000_000_123);
        assert_eq!(ulid.0 & ((1 << 80) - 1), 0x1234_5678_9abc_def0_1234);
        let text = ulid.to_string();
        assert_eq!(text.len(), 26);
        for spelling in [text.clone(), text.to_ascii_lowercase()] {
            assert_eq!(spelling.parse::<Ulid>(), Ok(ulid));
        }
        let extremes = [
            (Ulid(0), "0".repeat(26)),
            (Ulid(u128::MAX), format!("7{}", "Z".repeat(25))),
        ];
        for (value, shown) in extremes {
            assert_eq!(value.to_string(), shown);
            assert_eq!(shown.parse::<Ulid>(), Ok(value));
        }
    }

    #[test]
    fn from_parts_drops_bits_out_of_range() {
        let full = Ulid::from_parts(u64::MAX, u128::MAX);
        assert_eq!((full, full.unix_millis()), (Ulid(u128::MAX), (1 << 48) - 1));
        assert_eq!(Ulid::from_parts(1, 0), Ulid(1 << 80));
    }

    #[test]
    fn ulid_refusals() {
        let stem = &SAMPLE[..25];
        let bad = [
            "8ZZZZZZZZZZZZZZZZZZZZZZZZZ".to_owned(),
            format!("{stem}U"),
            stem.to_owned(),
            format!("{SAMPLE}V"),
            format!("{stem}I"),
            format!("{stem}L"),
            format!("{stem}O"),
            String::new(),
        ];
        for text in &bad {
            assert_eq!(text.parse::<Ulid>(), Err(IdError::BadUlid), "{text}");
        }
        assert!(SAMPLE.parse::<Ulid>().is_ok());
    }

    #[test]
    fn node_ids() {
        let longest = "abcdefghijklmnopqrstuvwxyz012345";
        for good in ["a", "node-1", "0", longest] {
            assert_eq!(NodeId::new(good).map(|n| n.to_string()), Ok(good.to_owned()));
        }
        let too_long = format!("{longest}6");
        for bad in ["", "-x", "x-", "New-York", "a_b", "a.b", too_long.as_str()] {
            assert_eq!(NodeId::new(bad), Err(IdError::BadNode), "{bad}");
        }
    }

    #[test]
    fn global_ids() {
        let text = format!("{SAMPLE}@eu-1");
        let id: GlobalId = text.parse().unwrap();
        assert_eq!((id.node.as_str(), id.to_string()), ("eu-1", text.clone()));
        let session: SessionId = text.parse().unwrap();
        assert_eq!((&session.0, session.to_string()), (&id, text));

        assert_eq!(SAMPLE.parse::<GlobalId>(), Err(IdError::BadGlobal));
        assert_eq!(
            format!("{}@eu", &SAMPLE[..25]).parse::<RealmId>(),
            Err(IdError::BadUlid)
        );
        assert_eq!(format!("{SAMPLE}@-x").parse::<PartyId>(), Err(IdError::BadNode));
        assert_eq!(format!("{SAMPLE}@New-York").parse::<TicketId>(), Err(IdError::BadNode));
        assert_eq!(format!("{SAMPLE}@a@b").parse::<AdmissionId>(), Err(IdError::BadNode));
    }

    #[test]
    fn error_texts() {
        let texts = [
            (IdError::BadUlid, "a ULID is 26 Crockford base32 characters"),
            (
                IdError::BadNode,
                "a node id is 1 to 32 of a-z, 0-9 and '-', not starting or ending with '-'",
            ),
            (IdError::BadGlobal, "a global id is <ULID>@<node>"),
        ];
        for (error, text) in texts {
            assert_eq!(error.to_string(), text);
        }
        assert_eq!(AccountId("acct-1".into()).to_string(), "acct-1");
    }
}
