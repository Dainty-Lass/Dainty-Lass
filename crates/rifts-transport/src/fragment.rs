// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Messages over 1100 bytes go as 2..=4 Fragment frames: inner type, a big-endian message id, the piece index and
//! count, then the piece. 1100-byte pieces keep each datagram under 1280 bytes after the seal, TURN and IP/UDP
//! headers; a relay run lost every 1400- and 2000-byte datagram.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::seal::FrameType;

/// The largest piece of a fragmented message, in bytes.
pub const PIECE_BYTES: usize = 1100;
/// The most pieces one message may take.
pub const MAX_PIECES: usize = 4;
const PIECE_HEADER: usize = 5;
const MAX_PARTIAL: usize = 32;
const PARTIAL_LIFETIME: Duration = Duration::from_secs(1);

/// The plaintexts of the Fragment frames for one message, or `None` when it needs more than 4 pieces.
pub fn split(inner: FrameType, message_id: u16, payload: &[u8]) -> Option<Vec<Vec<u8>>> {
    let count = payload.len().div_ceil(PIECE_BYTES);
    if !(2..=MAX_PIECES).contains(&count) {
        return None;
    }
    let pieces = payload
        .chunks(PIECE_BYTES)
        .enumerate()
        .map(|(index, piece)| {
            let mut out = Vec::with_capacity(PIECE_HEADER + piece.len());
            out.push(inner as u8);
            out.extend_from_slice(&message_id.to_be_bytes());
            out.push(index as u8);
            out.push(count as u8);
            out.extend_from_slice(piece);
            out
        })
        .collect();
    Some(pieces)
}

struct Partial {
    id: u16,
    inner: FrameType,
    pieces: Vec<Option<Vec<u8>>>,
    started: Instant,
}

impl Partial {
    fn new(id: u16, inner: FrameType, count: usize, now: Instant) -> Self {
        Self {
            id,
            inner,
            pieces: vec![None; count],
            started: now,
        }
    }
}

/// What one Fragment frame did to its message.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    /// Not a valid piece: counts `frames_rejected`.
    Rejected,
    /// A valid piece; the message still lacks others.
    Pending,
    /// The last piece arrived: the inner frame type and the whole message.
    Complete(FrameType, Vec<u8>),
}

/// Puts fragmented messages back together, keeping a few partial ones for up to a second.
#[derive(Default)]
pub struct Reassembler {
    partial: VecDeque<Partial>,
    /// Partials evicted to make room.
    pub queue_drops: u64,
}

impl Reassembler {
    /// Takes one Fragment frame's plaintext.
    pub fn add(&mut self, plain: &[u8], now: Instant) -> Piece {
        if plain.len() <= PIECE_HEADER {
            return Piece::Rejected;
        }
        let inner = match FrameType::from_byte(plain[0]) {
            Some(kind @ (FrameType::Control | FrameType::Gameplay)) => kind,
            _ => return Piece::Rejected,
        };
        let id = u16::from_be_bytes([plain[1], plain[2]]);
        let index = usize::from(plain[3]);
        let count = usize::from(plain[4]);
        if !(2..=MAX_PIECES).contains(&count) || index >= count {
            return Piece::Rejected;
        }
        self.partial
            .retain(|p| now.saturating_duration_since(p.started) <= PARTIAL_LIFETIME);

        let position = match self.partial.iter().position(|p| p.id == id) {
            Some(at) => {
                let existing = &mut self.partial[at];
                if existing.pieces.len() != count || existing.inner != inner {
                    *existing = Partial::new(id, inner, count, now);
                }
                at
            }
            None => {
                if self.partial.len() >= MAX_PARTIAL {
                    self.partial.pop_front();
                    self.queue_drops += 1;
                }
                self.partial.push_back(Partial::new(id, inner, count, now));
                self.partial.len() - 1
            }
        };
        let entry = &mut self.partial[position];
        if entry.pieces[index].is_none() {
            entry.pieces[index] = Some(plain[PIECE_HEADER..].to_vec());
        }
        if entry.pieces.iter().all(Option::is_some) {
            let done = self.partial.remove(position);
            let Some(done) = done else {
                return Piece::Pending;
            };
            let message = done.pieces.into_iter().flatten().flatten().collect();
            return Piece::Complete(done.inner, message);
        }
        Piece::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_join() {
        let payload: Vec<u8> = (0..2500_u32).map(|i| i as u8).collect();
        let pieces = split(FrameType::Gameplay, 7, &payload).unwrap();
        assert_eq!(pieces.len(), 3);
        assert_eq!(pieces[0][..5], [2, 0, 7, 0, 3]);
        let mut joiner = Reassembler::default();
        let now = Instant::now();
        assert_eq!(joiner.add(&pieces[2], now), Piece::Pending);
        assert_eq!(joiner.add(&pieces[2], now), Piece::Pending);
        assert_eq!(joiner.add(&pieces[0], now), Piece::Pending);
        assert_eq!(
            joiner.add(&pieces[1], now),
            Piece::Complete(FrameType::Gameplay, payload.clone())
        );
        assert!(split(FrameType::Gameplay, 1, &[0; 1100]).is_none());
        assert!(split(FrameType::Gameplay, 1, &[0; 4401]).is_none());
        assert_eq!(joiner.add(&[2, 0, 1, 2, 2, 9], now), Piece::Rejected);
        assert_eq!(joiner.add(&[3, 0, 1, 0, 2, 9], now), Piece::Rejected);
        assert_eq!(joiner.add(&[2, 0, 1, 0, 2], now), Piece::Rejected);
    }

    #[test]
    fn stale_and_evicted() {
        let mut joiner = Reassembler::default();
        let now = Instant::now();
        for id in 0..33_u16 {
            let [high, low] = id.to_be_bytes();
            joiner.add(&[2, high, low, 0, 2, 1], now);
        }
        assert_eq!(joiner.queue_drops, 1);
        // Message 0 was evicted, so its second piece starts over.
        assert_eq!(joiner.add(&[2, 0, 0, 1, 2, 1], now), Piece::Pending);
        let later = now + Duration::from_millis(1500);
        assert_eq!(joiner.add(&[2, 0, 5, 1, 2, 1], later), Piece::Pending);
    }
}
