// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The sealed frame between a host and one joiner: `RX`, version, type, a big-endian counter, then AES-256-GCM
//! ciphertext and tag. The header is the associated data; the nonce is a direction prefix and the counter.

use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::TryRng as _;

/// Umbrella spec D3: `RX` in place of the old magic, same length.
pub const MAGIC: [u8; 2] = *b"RX";
/// The frame format version.
pub const VERSION: u8 = 1;
/// The header's size: magic, version, type and an 8-byte counter.
pub const HEADER_BYTES: usize = 12;
/// The AES-GCM tag's size.
pub const TAG_BYTES: usize = 16;
/// What sealing adds to a payload, in bytes.
pub const OVERHEAD: usize = HEADER_BYTES + TAG_BYTES;
/// The largest payload one frame carries, in bytes.
pub const MAX_PLAINTEXT: usize = 65536;
const WINDOW: u64 = 1024;
const HOST_TO_JOINER: [u8; 4] = *b"HJ\0\x01";
const JOINER_TO_HOST: [u8; 4] = *b"JH\0\x01";

/// What a frame carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    /// Admission requests and results.
    Control = 1,
    /// Game datagrams, once admitted.
    Gameplay = 2,
    /// One piece of a message too big for one frame.
    Fragment = 3,
}

impl FrameType {
    /// The frame type with this byte, `None` for any other.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Control),
            2 => Some(Self::Gameplay),
            3 => Some(Self::Fragment),
            _ => None,
        }
    }
}

/// Which end of a host-joiner pair a cipher is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The host; it seals host-to-joiner frames.
    Host,
    /// The joiner; it seals joiner-to-host frames.
    Joiner,
}

/// 43 unpadded base64url characters for 32 bytes; the last character's two spare bits must be zero.
pub fn decode_pair_key(text: &str) -> Option<[u8; 32]> {
    if text.len() != 43
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(text).ok()?;
    bytes.try_into().ok()
}

fn random_start() -> Option<u64> {
    loop {
        let mut bytes = [0_u8; 8];
        rand::rngs::SysRng.try_fill_bytes(&mut bytes).ok()?;
        // A rejoin reuses the pair key, so a fixed start would reuse nonces.
        let start = u64::from_be_bytes(bytes) & (u64::MAX >> 1);
        if start != 0 {
            return Some(start);
        }
    }
}

fn nonce(prefix: [u8; 4], counter: u64) -> [u8; 12] {
    let mut out = [0_u8; 12];
    out[..4].copy_from_slice(&prefix);
    out[4..].copy_from_slice(&counter.to_be_bytes());
    out
}

struct Sealer {
    cipher: Aes256Gcm,
    prefix: [u8; 4],
    counter: u64,
}

impl Sealer {
    fn seal(&mut self, kind: FrameType, payload: &[u8]) -> Option<Vec<u8>> {
        if payload.is_empty() || payload.len() > MAX_PLAINTEXT || self.counter == u64::MAX {
            return None;
        }
        self.counter += 1;
        let mut header = [0_u8; HEADER_BYTES];
        header[..2].copy_from_slice(&MAGIC);
        header[2] = VERSION;
        header[3] = kind as u8;
        header[4..].copy_from_slice(&self.counter.to_be_bytes());
        let sealed = self
            .cipher
            .encrypt(
                &Nonce::from(nonce(self.prefix, self.counter)),
                Payload {
                    msg: payload,
                    aad: &header,
                },
            )
            .ok()?;
        let mut out = Vec::with_capacity(HEADER_BYTES + sealed.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&sealed);
        Some(out)
    }
}

/// The highest counter taken and a bitmap of it and the 1023 below it (bit `i` is `highest - i`).
#[derive(Default)]
struct Window {
    highest: u64,
    seen: [u64; 16],
}

impl Window {
    fn bit(&self, back: u64) -> bool {
        let back = back as usize;
        self.seen[back / 64] & (1 << (back % 64)) != 0
    }

    fn set(&mut self, back: u64) {
        let back = back as usize;
        self.seen[back / 64] |= 1 << (back % 64);
    }

    fn shift(&mut self, by: u64) {
        if by >= WINDOW {
            self.seen = [0; 16];
            return;
        }
        let words = (by / 64) as usize;
        let bits = (by % 64) as u32;
        let mut next = [0_u64; 16];
        for (index, slot) in next.iter_mut().enumerate() {
            if index < words {
                continue;
            }
            let from = index - words;
            let mut value = self.seen[from] << bits;
            if bits != 0 && from > 0 {
                value |= self.seen[from - 1] >> (64 - bits);
            }
            *slot = value;
        }
        self.seen = next;
    }

    /// Whether `counter` may be tried at all (checked before decrypting).
    fn admits(&self, counter: u64) -> bool {
        if counter > self.highest {
            return true;
        }
        let back = self.highest - counter;
        back < WINDOW && !self.bit(back)
    }

    fn accept(&mut self, counter: u64) {
        if counter > self.highest {
            self.shift(counter - self.highest);
            self.highest = counter;
            self.set(0);
        } else {
            self.set(self.highest - counter);
        }
    }
}

struct Opener {
    cipher: Aes256Gcm,
    prefix: [u8; 4],
    window: Window,
}

impl Opener {
    fn open(&mut self, frame: &[u8]) -> Option<(FrameType, Vec<u8>)> {
        if frame.len() <= OVERHEAD || frame.len() > OVERHEAD + MAX_PLAINTEXT {
            return None;
        }
        let (header, sealed) = frame.split_at(HEADER_BYTES);
        if header[..2] != MAGIC || header[2] != VERSION {
            return None;
        }
        let kind = FrameType::from_byte(header[3])?;
        let mut counter_bytes = [0_u8; 8];
        counter_bytes.copy_from_slice(&header[4..]);
        let counter = u64::from_be_bytes(counter_bytes);
        if counter == 0 || !self.window.admits(counter) {
            return None;
        }
        let plain = self
            .cipher
            .decrypt(
                &Nonce::from(nonce(self.prefix, counter)),
                Payload {
                    msg: sealed,
                    aad: header,
                },
            )
            .ok()?;
        self.window.accept(counter);
        Some((kind, plain))
    }
}

/// One side's cipher for a pair. Sealing and opening lock separately, so the network side can open while the session
/// thread seals.
pub struct PairCipher {
    sealer: Mutex<Sealer>,
    opener: Mutex<Opener>,
}

impl PairCipher {
    /// A cipher for this side of the pair; fails unless `key` is 32 bytes or with no random source for the counter.
    pub fn new(key: &[u8], side: Side) -> Result<Self, &'static str> {
        if key.len() != 32 {
            return Err("pair key must be 32 bytes");
        }
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "pair key could not be loaded")?;
        let start = random_start().ok_or("random number generator unavailable")?;
        let (seal_prefix, open_prefix) = match side {
            Side::Host => (HOST_TO_JOINER, JOINER_TO_HOST),
            Side::Joiner => (JOINER_TO_HOST, HOST_TO_JOINER),
        };
        Ok(Self {
            sealer: Mutex::new(Sealer {
                cipher: cipher.clone(),
                prefix: seal_prefix,
                counter: start,
            }),
            opener: Mutex::new(Opener {
                cipher,
                prefix: open_prefix,
                window: Window::default(),
            }),
        })
    }

    /// Seals one frame with the next counter; `None` when the payload is empty or too big.
    pub fn seal(&self, kind: FrameType, payload: &[u8]) -> Option<Vec<u8>> {
        self.sealer.lock().ok()?.seal(kind, payload)
    }

    /// Opens one frame; `None` when it is malformed, fails its tag or replays a counter seen in the 1024-frame window.
    pub fn open(&self, frame: &[u8]) -> Option<(FrameType, Vec<u8>)> {
        self.opener.lock().ok()?.open(frame)
    }

    /// The counter the next frame will carry.
    pub fn next_counter(&self) -> u64 {
        self.sealer.lock().map_or(0, |s| s.counter.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_shifts_across_words() {
        let mut window = Window::default();
        window.accept(5);
        window.accept(3);
        window.shift(70);
        assert!(window.bit(70));
        assert!(window.bit(72));
        assert!(!window.bit(71));
    }
}
