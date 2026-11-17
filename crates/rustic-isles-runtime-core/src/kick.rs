// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Staff kicks on a game server (spec-runtime-kick): the pure half. The Director writes the kicked players' ids to
//! a file next to the game's executable; the server reads it from the SP-3 tick, finds each player's connection by
//! the net id on their player state, and sends that client back to the menu.

use crate::memory::Memory;
use crate::reflect::read_fstring;

/// The flag that turns kicks off on this server.
pub const DISABLE_FLAG: &str = "DISABLE_STAFF_KICK";
/// How often the server looks for the file (ms).
pub const POLL_MS: u64 = 1000;
/// A request older than this is stale: written for a server that ran before this one (ms).
pub const STALE_MS: u64 = 120_000;
/// How long a request waits for its player to be on this server (a player joining) before it is dropped (ms).
pub const PENDING_MS: u64 = 30_000;
/// Most ids one file carries.
pub const MAX_IDS: usize = 64;
/// Longest id read or accepted, in UTF-16 units.
pub const MAX_ID: usize = 128;
/// The offsets inside the net id object the id string may sit at: `FUniqueNetIdString.UniqueNetIdStr` after the
/// vtable alone, or after the vtable and a `TSharedFromThis` weak reference (ambiguity 1).
pub const NET_ID_STRING_AT: [usize; 2] = [0x08, 0x18];
/// What the client is told, as an `FText` (call 4).
pub const REASON: &str = "You were removed from the server by staff.";
/// The client calls tried in order, each an `FText` parameter (call 3).
pub const CLIENT_CALLS: [&str; 2] = ["ClientReturnToMainMenuWithTextReason", "ClientWasKicked"];

/// The file the Director writes for the server on `port`, next to the game's executable.
pub fn file_name(port: u16) -> String {
    format!("rustic_isles_kick_port{port}.txt")
}

/// One line of the file: `<unix ms> <user id>`.
pub fn line(at_ms: u64, user_id: &str) -> String {
    format!("{at_ms} {user_id}\n")
}

/// Whether `id` can be a player id: 1 to [`MAX_ID`] printable characters, no space.
pub fn plausible_id(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= MAX_ID && id.chars().all(|c| c.is_ascii_graphic())
}

/// The ids the file asks for, each once, in the file's order: lines that are malformed, stale at `now_ms`, from the future by more than [`STALE_MS`], or past [`MAX_IDS`] are dropped.
pub fn parse(text: &str, now_ms: u64) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for raw in text.lines() {
        let Some((at, id)) = raw.trim().split_once(' ') else {
            continue;
        };
        let (Ok(at), id) = (at.parse::<u64>(), id.trim()) else {
            continue;
        };
        if now_ms.abs_diff(at) > STALE_MS || !plausible_id(id) || ids.iter().any(|seen| seen == id) {
            continue;
        }
        if ids.len() == MAX_IDS {
            break;
        }
        ids.push(id.to_owned());
    }
    ids
}

/// The player id on a player state: the net id object's string, at the first of [`NET_ID_STRING_AT`] that reads as
/// a plausible id. `net_id_field` is the player state's net id pointer field.
pub fn player_id(memory: &dyn Memory, net_id_field: usize) -> Option<String> {
    let object = memory.ptr_at(net_id_field).filter(|&p| p != 0)?;
    NET_ID_STRING_AT
        .iter()
        .filter_map(|&offset| read_fstring(memory, object.checked_add(offset)?, MAX_ID))
        .find(|id| plausible_id(id))
}

/// Requests waiting for their player, with the time each arrived.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pending(Vec<(String, u64)>);

impl Pending {
    /// No requests.
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Adds the ids read at `now_ms`; one already waiting keeps its first time.
    pub fn add(&mut self, ids: Vec<String>, now_ms: u64) {
        for id in ids {
            if !self.0.iter().any(|(seen, _)| *seen == id) && self.0.len() < MAX_IDS {
                self.0.push((id, now_ms));
            }
        }
    }

    /// Whether any request waits.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The connected players to kick now (`connected` is each connection's player id, when read), taken out of the
    /// waiting list, and the ids that waited past [`PENDING_MS`] (dropped).
    pub fn take(&mut self, connected: &[Option<String>], now_ms: u64) -> (Vec<String>, Vec<String>) {
        let mut kick = Vec::new();
        let mut expired = Vec::new();
        self.0.retain(|(id, since)| {
            if connected.iter().flatten().any(|c| c == id) {
                kick.push(id.clone());
                false
            } else if now_ms.saturating_sub(*since) > PENDING_MS {
                expired.push(id.clone());
                false
            } else {
                true
            }
        });
        (kick, expired)
    }
}

// ---- log lines (spec 6) ----

/// At the first SP-3 tick.
pub fn config_line(disabled: bool, port: u16) -> String {
    if disabled {
        format!("[MitRuntime] [Kick] off ({DISABLE_FLAG}.flag)")
    } else {
        format!(
            "[MitRuntime] [Kick] on: watching {} next to the executable",
            file_name(port)
        )
    }
}

/// When a file was read.
pub fn read_line(ids: usize, players: usize) -> String {
    format!("[MitRuntime] [Kick] request read: {ids} id(s), {players} player(s) connected")
}

/// When a player was sent to the menu, or the call failed.
pub fn kick_line(id: &str, call: Option<&str>) -> String {
    match call {
        Some(call) => format!("[MitRuntime] [Kick] {id} sent to the menu ({call})"),
        None => format!("[MitRuntime] [Kick] {id} could not be kicked: no client call answered"),
    }
}

/// When a request waited past [`PENDING_MS`] without its player connected.
pub fn expired_line(id: &str) -> String {
    format!("[MitRuntime] [Kick] {id} not on this server; request dropped")
}

/// The ids read on the connected players, once per request, for checking the net id read (ambiguity 1).
pub fn ids_line(connected: &[Option<String>]) -> String {
    let read: Vec<&str> = connected.iter().map(|id| id.as_deref().unwrap_or("?")).collect();
    format!("[MitRuntime] [Kick] connected ids: [{}]", read.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::BufferMemory;

    #[test]
    fn names_and_lines() {
        assert_eq!(file_name(8777), "rustic_isles_kick_port8777.txt");
        assert_eq!(line(5, "u1"), "5 u1\n");
        assert_eq!(
            kick_line("u1", Some(CLIENT_CALLS[0])),
            "[MitRuntime] [Kick] u1 sent to the menu (ClientReturnToMainMenuWithTextReason)"
        );
        assert!(config_line(true, 1).contains("DISABLE_STAFF_KICK.flag"));
        assert_eq!(
            ids_line(&[Some("a".into()), None]),
            "[MitRuntime] [Kick] connected ids: [a, ?]"
        );
    }

    #[test]
    fn the_file_is_read_with_care() {
        let now = 1_000_000;
        let text = format!(
            "{}{}{}{}garbage\n{} has space\n{}{}",
            line(now, "9c65cf9f-496a-4705-97d0-98c763563f59"),
            line(now - STALE_MS - 1, "stale"),
            line(now + STALE_MS + 1, "future"),
            line(now, "9c65cf9f-496a-4705-97d0-98c763563f59"),
            now,
            line(now, ""),
            line(now, &"x".repeat(MAX_ID + 1)),
        );
        assert_eq!(parse(&text, now), ["9c65cf9f-496a-4705-97d0-98c763563f59"]);
        let many: String = (0..100).map(|i| line(now, &format!("u{i}"))).collect();
        assert_eq!(parse(&many, now).len(), MAX_IDS);
        assert!(parse("", now).is_empty());
    }

    #[test]
    fn requests_wait_for_their_player() {
        let mut pending = Pending::new();
        pending.add(vec!["a".into(), "b".into()], 0);
        pending.add(vec!["a".into()], 10);
        let (kick, expired) = pending.take(&[Some("a".into()), None], 100);
        assert_eq!((kick, expired), (vec!["a".to_owned()], vec![]));
        let (kick, expired) = pending.take(&[], PENDING_MS + 1);
        assert_eq!((kick, expired), (vec![], vec!["b".to_owned()]));
        assert!(pending.is_empty());
    }

    /// A player state's net id field at 0x1000 pointing at an object at 0x1100 whose string is at `at`.
    fn net_id(at: usize, id: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x300];
        bytes[0..8].copy_from_slice(&0x1100_u64.to_le_bytes());
        let text = 0x1200_usize;
        let string = 0x100 + at;
        bytes[string..string + 8].copy_from_slice(&(text as u64).to_le_bytes());
        let num = i32::try_from(id.encode_utf16().count() + 1).unwrap();
        bytes[string + 8..string + 12].copy_from_slice(&num.to_le_bytes());
        bytes[string + 12..string + 16].copy_from_slice(&num.to_le_bytes());
        for (i, unit) in id.encode_utf16().enumerate() {
            bytes[0x200 + i * 2..0x200 + i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn the_net_id_reads_at_either_layout() {
        for at in NET_ID_STRING_AT {
            let bytes = net_id(at, "9c65cf9f-496a");
            let memory = BufferMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            assert_eq!(player_id(&memory, 0x1000).as_deref(), Some("9c65cf9f-496a"), "{at:#x}");
        }
        let empty = vec![0u8; 0x300];
        let memory = BufferMemory {
            base: 0x1000,
            bytes: &empty,
        };
        assert_eq!(player_id(&memory, 0x1000), None);
    }
}
