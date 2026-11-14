//! Easy Anti-Cheat on a private client (R4 5): CL-5's message test and CL-7's write rule.

use crate::memory::{Region, region_allows};
use crate::{wide, wide_contains};

/// The most `FString` units CL-5 reads.
pub const MAX_UNITS: usize = 512;
/// CL-5 logs its first absorbs and its first forwards, this many of each.
pub const LOGGED: u32 = 4;

/// How many units to read from an `FString` whose `Num` (terminator included) is `num`.
pub fn units_to_read(num: i32) -> usize {
    usize::try_from(num).map_or(0, |num| num.min(MAX_UNITS))
}

/// The read units minus the last (the NUL, or the 512th unit of a longer text, as the AGPL reads it).
pub fn message_text(read: &[u16]) -> &[u16] {
    read.split_last().map_or(&[], |(_, text)| text)
}

/// Whether the message names `IGameClient` (case-sensitive).
pub fn names_game_client(text: &[u16]) -> bool {
    wide_contains(text, &wide("IGameClient"))
}

/// The expected client-init error, absorbed so EAC's modal dialog never opens.
pub fn absorbs(text: &[u16]) -> bool {
    !text.is_empty() && (names_game_client(text) || wide_contains(text, &wide("Failed to create")))
}

/// The absorb line; the message itself is never logged.
pub fn absorbed_line(text: &[u16]) -> String {
    format!(
        "[MitRuntime] [EAC] absorbed the expected client-init error ({} chars, IGameClient={})",
        text.len(),
        u8::from(names_game_client(text))
    )
}

/// The forward line.
pub fn forwarded_line(text: &[u16]) -> String {
    format!(
        "[MitRuntime] [EAC] forwarded an EAC error to the game ({} chars)",
        text.len()
    )
}

/// What CL-7 does with the `EnableEasyAntiCheatFeatures` byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cl7 {
    /// Write 0.
    Write,
    /// Not in a committed, accessible region.
    Unreadable,
    /// Readable, but the page does not allow writing.
    NotWritable,
}

const PAGE_READWRITE: u32 = 0x04;
const PAGE_WRITECOPY: u32 = 0x08;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;

/// R4 5.1's rule (a call: the AGPL checks only that the byte reads), so a wrong RVA in read-only data is a logged
/// failure, not a fault.
pub fn cl7_decision(ptr: usize, region: Option<Region>) -> Cl7 {
    if !region_allows(ptr, 1, region, false) {
        return Cl7::Unreadable;
    }
    let writable = PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
    if region.is_some_and(|region| region.protect & writable != 0) {
        Cl7::Write
    } else {
        Cl7::NotWritable
    }
}

impl Cl7 {
    /// The status line's result.
    pub fn status(self) -> Result<(), &'static str> {
        match self {
            Cl7::Write => Ok(()),
            Cl7::Unreadable => Err("unreadable"),
            Cl7::NotWritable => Err("not writable"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS};

    fn w(text: &str) -> Vec<u16> {
        wide(text)
    }

    #[test]
    fn message_test() {
        assert!(absorbs(&w("Failed to initialize IGameClient interface")));
        assert!(absorbs(&w("Failed to create game client")));
        assert!(!absorbs(&w("failed to init igameclient")));
        assert!(!absorbs(&w("")));
        assert!(!absorbs(&w("Integrity violation")));
        assert_eq!(
            absorbed_line(&w("x IGameClient")),
            "[MitRuntime] [EAC] absorbed the expected client-init error (13 chars, IGameClient=1)"
        );
        assert_eq!(
            forwarded_line(&w("abc")),
            "[MitRuntime] [EAC] forwarded an EAC error to the game (3 chars)"
        );
    }

    #[test]
    fn num_handling() {
        assert_eq!(units_to_read(0), 0);
        assert_eq!(units_to_read(-5), 0);
        assert_eq!(units_to_read(1), 1);
        assert_eq!(units_to_read(2), 2);
        assert_eq!(units_to_read(600), 512);
        let read = w("ab");
        assert_eq!(message_text(&read[..units_to_read(1)]), &[] as &[u16]);
        assert_eq!(message_text(&read[..units_to_read(2)]), &w("a")[..]);
        let long = vec![u16::from(b'x'); 600];
        assert_eq!(message_text(&long[..units_to_read(600)]).len(), 511);
        assert_eq!(message_text(&[]), &[] as &[u16]);
    }

    fn region(protect: u32) -> Option<Region> {
        Some(Region {
            base: 0x1000,
            size: 0x1000,
            state: MEM_COMMIT,
            protect,
        })
    }

    #[test]
    fn cl7_rule() {
        for protect in [0x04, 0x08, 0x40, 0x80] {
            assert_eq!(cl7_decision(0x1801, region(protect)), Cl7::Write, "{protect:#x}");
        }
        assert_eq!(cl7_decision(0x1801, region(0x02)), Cl7::NotWritable);
        assert_eq!(cl7_decision(0x1801, region(0x20)), Cl7::NotWritable);
        assert_eq!(cl7_decision(0x1801, region(PAGE_NOACCESS)), Cl7::Unreadable);
        assert_eq!(cl7_decision(0x1801, region(0x04 | PAGE_GUARD)), Cl7::Unreadable);
        assert_eq!(cl7_decision(0x1801, None), Cl7::Unreadable);
        assert_eq!(
            cl7_decision(0x3000, region(0x04)),
            Cl7::Unreadable,
            "outside the region"
        );
        assert_eq!(Cl7::NotWritable.status(), Err("not writable"));
    }
}
