// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The hash chain (DD23 layer 3, DH5): each entry carries the hash of the one before it (`prev_hash`) and
//! its own (`hash`), over [`entry_hash`]'s encoding of its fields and `prev_hash`. The first entry's
//! `prev_hash` is [`GENESIS`]. A head shipped off the server (DH4) then commits to every entry up to it.
//!
//! The hashes are computed here, in Rust, by [`seal`] at the end of every write, before it commits: SQLite
//! has no SHA-256 to call from a trigger, and one encoding shared by the writer and [`verify`] can't drift
//! between two dialects. The log's update guard lets an entry be sealed once, and changed no other way.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use sha2::{Digest, Sha256};

use super::{CHANGE_LOG, ChangeEntry, changes};
use crate::Error;
use crate::connection::AnyConnection;
use crate::portable::quote_ident;

/// The `prev_hash` of the first entry: of a new log, and of one written before the chain, which the
/// first open with the chain seals from here (DH5).
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Separates this encoding from any other SHA-256 input; a new encoding gets a new one.
const DOMAIN: &[u8] = b"dainty-lass change log v1\n";

/// Entries read per round trip while sealing or verifying.
const PAGE: i64 = 1000;

/// An entry by its id and hash: the log's last sealed entry, what DH4 ships with the log and what
/// [`verify`] takes as known good.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Head {
    pub id: i64,
    pub hash: String,
}

/// Lower-case hex SHA-256 of `DOMAIN`, then `prev_hash`, `id`, `at`, `actor`, `table_name`, `row_key`,
/// `op`, `before` and `after`: integers as 8 big-endian bytes, text as its UTF-8 length in 8 big-endian
/// bytes then the bytes, a missing `before` or `after` as `0`, a present one as `1` then the text. The
/// text is hashed as stored, so a log moved between engines (`db move`) keeps its hashes.
pub fn entry_hash(prev_hash: &str, entry: &ChangeEntry) -> String {
    fn text(hasher: &mut Sha256, value: &str) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    fn optional(hasher: &mut Sha256, value: Option<&str>) {
        match value {
            None => hasher.update([0]),
            Some(v) => {
                hasher.update([1]);
                text(hasher, v);
            }
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    text(&mut hasher, prev_hash);
    hasher.update(entry.id.to_be_bytes());
    hasher.update(entry.at.to_be_bytes());
    text(&mut hasher, &entry.actor);
    text(&mut hasher, &entry.table_name);
    text(&mut hasher, &entry.row_key);
    text(&mut hasher, &entry.op);
    optional(&mut hasher, entry.before.as_deref());
    optional(&mut hasher, entry.after.as_deref());
    hasher
        .finalize()
        .iter()
        .flat_map(|&b| [b >> 4, b & 15])
        .filter_map(|n| char::from_digit(u32::from(n), 16))
        .collect()
}

/// 64 lower-case hex digits, as [`entry_hash`] writes them.
pub fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where the chain breaks: the first entry that was edited, removed or reordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Break {
    /// The entry's hash isn't the hash of what it holds: it was changed after it was written.
    Edited { id: i64 },
    /// The entry has no hash: it was put in the log outside `store`'s write path.
    Unsealed { id: i64 },
    /// The entry doesn't name the one before it (`after`; `None` for the start of the walk): entries
    /// between them were removed or reordered, or `after` was changed and its hash recomputed.
    Unlinked { id: i64, after: Option<i64> },
    /// The entry's id isn't above the one before it (a stream read back out of order).
    OutOfOrder { id: i64, after: i64 },
    /// The trusted head's entry isn't in the log: the log was cut short, or that entry removed.
    HeadMissing { id: i64 },
    /// The trusted head's entry has another hash: it, or an entry before it, was changed and the chain
    /// recomputed from there.
    HeadMismatch { id: i64 },
}

impl Break {
    /// The entry the break is found at.
    pub fn id(&self) -> i64 {
        match self {
            Self::Edited { id }
            | Self::Unsealed { id }
            | Self::Unlinked { id, .. }
            | Self::OutOfOrder { id, .. }
            | Self::HeadMissing { id }
            | Self::HeadMismatch { id } => *id,
        }
    }
}

impl std::fmt::Display for Break {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edited { id } => write!(f, "entry {id} was changed after it was written"),
            Self::Unsealed { id } => write!(f, "entry {id} has no hash: it was put in the log around store"),
            Self::Unlinked { id, after: Some(after) } => write!(
                f,
                "entry {id} doesn't follow entry {after}: entries between them were removed or reordered, \
                 or entry {after} was changed and its hash recomputed"
            ),
            Self::Unlinked { id, after: None } => write!(
                f,
                "entry {id} doesn't follow the start of the chain: entries before it were removed"
            ),
            Self::OutOfOrder { id, after } => write!(f, "entry {id} comes after entry {after}: out of order"),
            Self::HeadMissing { id } => write!(
                f,
                "entry {id}, the trusted head, is gone: the log was cut short or the entry removed"
            ),
            Self::HeadMismatch { id } => write!(
                f,
                "entry {id} isn't the trusted head any more: it or an entry before it was changed and the \
                 chain recomputed"
            ),
        }
    }
}

/// Checks entries one after another, oldest first: the log read back here, or a stream read back during
/// a restore (DH4).
#[derive(Debug, Clone)]
pub struct Chain {
    prev: String,
    last: Option<i64>,
}

impl Chain {
    /// A chain whose first entry must name [`GENESIS`].
    pub fn genesis() -> Self {
        Self {
            prev: GENESIS.to_owned(),
            last: None,
        }
    }

    /// A chain that goes on after `head`, an entry already known good.
    pub fn after(head: &Head) -> Self {
        Self {
            prev: head.hash.clone(),
            last: Some(head.id),
        }
    }

    /// The last entry checked, or the head this chain started after.
    pub fn head(&self) -> Option<Head> {
        self.last.map(|id| Head {
            id,
            hash: self.prev.clone(),
        })
    }

    /// Checks that `entry` is sealed, unchanged and the next one; on success it becomes the head. A
    /// break leaves the chain as it was.
    pub fn check(&mut self, entry: &ChangeEntry) -> Result<(), Break> {
        let id = entry.id;
        let (Some(prev), Some(hash)) = (&entry.prev_hash, &entry.hash) else {
            return Err(Break::Unsealed { id });
        };
        if !is_hash(prev) || !is_hash(hash) || entry_hash(prev, entry) != *hash {
            return Err(Break::Edited { id });
        }
        if let Some(after) = self.last
            && id <= after
        {
            return Err(Break::OutOfOrder { id, after });
        }
        if *prev != self.prev {
            return Err(Break::Unlinked { id, after: self.last });
        }
        self.prev.clone_from(hash);
        self.last = Some(id);
        Ok(())
    }
}

/// What [`verify`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// Entries that checked out.
    pub entries: u64,
    /// The last of them: the head to compare with, or to ship.
    pub head: Option<Head>,
    /// The first break, where the walk stopped; `None` when the whole walk checked out.
    pub broken: Option<Break>,
}

#[derive(QueryableByName)]
struct HeadRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    hash: String,
}

/// The last sealed entry, `None` while nothing is: what DH4 ships with the log.
pub fn head(conn: &mut AnyConnection) -> Result<Option<Head>, Error> {
    Ok(diesel::sql_query(format!(
        "SELECT id, hash FROM {} WHERE hash IS NOT NULL ORDER BY id DESC LIMIT 1",
        quote_ident(CHANGE_LOG)
    ))
    .load::<HeadRow>(conn)?
    .pop()
    .map(|row| Head {
        id: row.id,
        hash: row.hash,
    }))
}

/// Chains every entry after `from`: the write's own, from the head taken as it began, so an entry the
/// closure sealed itself is refused by the guard; at open, a log written before the chain, from
/// [`GENESIS`]. Writes are serial, so nothing else is sealing (DH1).
pub(crate) fn seal(conn: &mut AnyConnection, from: Option<Head>) -> Result<(), Error> {
    let (mut prev, mut after) = match from {
        Some(head) => (head.hash, head.id),
        None => (GENESIS.to_owned(), 0),
    };
    let update = format!(
        "UPDATE {} SET prev_hash = $1, hash = $2 WHERE id = $3",
        quote_ident(CHANGE_LOG)
    );
    loop {
        let page = changes(conn, after, PAGE)?;
        for entry in &page {
            let hash = entry_hash(&prev, entry);
            diesel::sql_query(&update)
                .bind::<Text, _>(&prev)
                .bind::<Text, _>(&hash)
                .bind::<BigInt, _>(entry.id)
                .execute(conn)?;
            prev = hash;
            after = entry.id;
        }
        if page.len() < usize::try_from(PAGE).unwrap_or(usize::MAX) {
            return Ok(());
        }
    }
}

fn entry_by_id(conn: &mut AnyConnection, id: i64) -> Result<Option<ChangeEntry>, Error> {
    Ok(changes(conn, id.saturating_sub(1), 1)?.pop().filter(|e| e.id == id))
}

/// Walks the whole log from [`GENESIS`] and names the first entry that was edited, removed or
/// reordered. With `trusted` (a head shipped off the server), its entry must be on the way with that
/// hash, which also finds a log cut short or rewritten consistently from some entry on.
pub fn verify(conn: &mut AnyConnection, trusted: Option<&Head>) -> Result<Verification, Error> {
    let mut chain = Chain::genesis();
    let mut entries = 0;
    let mut seen = trusted.is_none();
    let mut after = 0;
    let broken = 'walk: loop {
        let page = changes(conn, after, PAGE)?;
        for entry in &page {
            if let Some(t) = trusted
                && !seen
                && entry.id > t.id
            {
                break 'walk Some(Break::HeadMissing { id: t.id });
            }
            if let Err(broken) = chain.check(entry) {
                break 'walk Some(broken);
            }
            if let Some(t) = trusted
                && entry.id == t.id
            {
                if entry.hash.as_deref() != Some(t.hash.as_str()) {
                    break 'walk Some(Break::HeadMismatch { id: t.id });
                }
                seen = true;
            }
            entries += 1;
            after = entry.id;
        }
        if page.len() < usize::try_from(PAGE).unwrap_or(usize::MAX) {
            break match trusted {
                Some(t) if !seen => Some(Break::HeadMissing { id: t.id }),
                _ => None,
            };
        }
    };
    Ok(Verification {
        entries,
        head: chain.head(),
        broken,
    })
}

/// Walks only the entries after `trusted`, whose entry must still be there with that hash: the range
/// written since the head last shipped. What comes before it is checked by [`verify`], or against the
/// shipped copy.
pub fn verify_after(conn: &mut AnyConnection, trusted: &Head) -> Result<Verification, Error> {
    let found = |broken| {
        Ok(Verification {
            entries: 0,
            head: None,
            broken: Some(broken),
        })
    };
    let Some(start) = entry_by_id(conn, trusted.id)? else {
        return found(Break::HeadMissing { id: trusted.id });
    };
    match (&start.prev_hash, &start.hash) {
        (Some(prev), Some(hash)) if entry_hash(prev, &start) == *hash => {
            if *hash != trusted.hash {
                return found(Break::HeadMismatch { id: trusted.id });
            }
        }
        (Some(_), Some(_)) => return found(Break::Edited { id: trusted.id }),
        _ => return found(Break::Unsealed { id: trusted.id }),
    }
    let mut chain = Chain::after(trusted);
    let mut entries = 0;
    let mut after = trusted.id;
    let broken = 'walk: loop {
        let page = changes(conn, after, PAGE)?;
        for entry in &page {
            if let Err(broken) = chain.check(entry) {
                break 'walk Some(broken);
            }
            entries += 1;
            after = entry.id;
        }
        if page.len() < usize::try_from(PAGE).unwrap_or(usize::MAX) {
            break None;
        }
    };
    Ok(Verification {
        entries,
        head: chain.head(),
        broken,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, prev: &str) -> ChangeEntry {
        let mut entry = ChangeEntry {
            id,
            at: 1_700_000_000_000,
            actor: "player:7".into(),
            table_name: "accounts".into(),
            row_key: r#"{"id":1}"#.into(),
            op: "insert".into(),
            before: None,
            after: Some(r#"{"id":1,"name":"alex"}"#.into()),
            prev_hash: Some(prev.into()),
            hash: None,
        };
        entry.hash = Some(entry_hash(prev, &entry));
        entry
    }

    // The encoding is a contract with every head already shipped (DH4): it never changes in place.
    #[test]
    fn the_encoding_is_pinned() {
        assert_eq!(
            entry(1, GENESIS).hash.unwrap(),
            "eb16e697a86829977d30bb3a41d5cff513eb441750c2df44f61b6dd465e858cb"
        );
    }

    #[test]
    fn every_field_is_hashed() {
        let base = entry(1, GENESIS);
        let hash = |e: &ChangeEntry| entry_hash(GENESIS, e);
        let changed: [fn(&mut ChangeEntry); 9] = [
            |e| e.id = 2,
            |e| e.at += 1,
            |e| e.actor = "player:8".into(),
            |e| e.table_name = "items".into(),
            |e| e.row_key = r#"{"id":2}"#.into(),
            |e| e.op = "update".into(),
            |e| e.before = Some(String::new()),
            |e| e.after = None,
            |e| e.after = Some(r#"{"id":1,"name":"alexa"}"#.into()),
        ];
        for change in changed {
            let mut e = base.clone();
            change(&mut e);
            assert_ne!(hash(&e), hash(&base), "{e:?}");
        }
        assert_ne!(entry_hash(&"1".repeat(64), &base), hash(&base));
        // Lengths keep one field's end from passing for the next's start.
        let mut shifted = base.clone();
        shifted.actor = "player:7a".into();
        shifted.table_name = "ccounts".into();
        assert_ne!(hash(&shifted), hash(&base));
    }

    #[test]
    fn the_chain_names_the_first_break() {
        let first = entry(1, GENESIS);
        let second = entry(2, first.hash.as_deref().unwrap());
        let third = entry(3, second.hash.as_deref().unwrap());

        let mut chain = Chain::genesis();
        for e in [&first, &second, &third] {
            chain.check(e).unwrap();
        }
        assert_eq!(chain.head().unwrap().id, 3);

        let mut edited = second.clone();
        edited.after = Some(r#"{"id":1,"name":"mallory"}"#.into());
        let mut chain = Chain::genesis();
        chain.check(&first).unwrap();
        assert_eq!(chain.check(&edited), Err(Break::Edited { id: 2 }));
        assert_eq!(chain.head().unwrap().id, 1, "a break moves nothing");

        let mut chain = Chain::genesis();
        chain.check(&first).unwrap();
        assert_eq!(chain.check(&third), Err(Break::Unlinked { id: 3, after: Some(1) }));
        assert_eq!(
            Chain::genesis().check(&second),
            Err(Break::Unlinked { id: 2, after: None })
        );

        let mut chain = Chain::after(&Head {
            id: 2,
            hash: second.hash.clone().unwrap(),
        });
        chain.check(&third).unwrap();
        assert_eq!(chain.clone().check(&first), Err(Break::OutOfOrder { id: 1, after: 3 }));

        let mut unsealed = third.clone();
        unsealed.hash = None;
        assert_eq!(Chain::genesis().check(&unsealed), Err(Break::Unsealed { id: 3 }));
        let mut upper = first.clone();
        upper.hash = upper.hash.map(|h| h.to_uppercase());
        assert_eq!(Chain::genesis().check(&upper), Err(Break::Edited { id: 1 }));
    }
}
