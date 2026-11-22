//! What the exporter needs from the process: guarded reads and `ProcessEvent` calls (spec §4.1, §4.2, §4.4). The
//! game's process is `sys::Process`; the tests build a `Synthetic` image instead.

use std::collections::BTreeMap;

/// Reads that fail instead of faulting (spec §12 call 4).
pub trait Memory {
    /// Fills `buf` from `addr`; false when any byte of it is unreadable.
    fn read(&self, addr: u64, buf: &mut [u8]) -> bool;
}

/// Calls into game code (spec §4.2).
pub trait Calls {
    /// Runs `function` on `object` through its `ProcessEvent` with the parameter block `params` (8-aligned, as the
    /// game's code expects); false when the call could not be made.
    fn process_event(&self, object: u64, function: u64, params: &mut [u64]) -> bool;
}

/// A sparse memory image for the tests and the fuzz target: regions placed by `alloc`, everything else unreadable.
#[derive(Default)]
pub struct Synthetic {
    regions: BTreeMap<u64, Vec<u8>>,
    next: u64,
}

/// Where the first region goes; low addresses stay unmapped so a null pointer reads as a fault.
const FIRST: u64 = 0x1_0000;

impl Synthetic {
    pub fn new() -> Self {
        Self {
            regions: BTreeMap::new(),
            next: FIRST,
        }
    }

    /// A zeroed region of `len` bytes (at least one), 16-aligned, with an unmapped gap after it.
    pub fn alloc(&mut self, len: usize) -> u64 {
        let addr = self.next;
        let len = len.max(1);
        self.regions.insert(addr, vec![0; len]);
        self.next = (addr + len as u64 + 0x10 + 0xF) & !0xF;
        addr
    }

    /// A region holding `bytes`.
    pub fn put(&mut self, bytes: &[u8]) -> u64 {
        let addr = self.alloc(bytes.len());
        self.write(addr, bytes);
        addr
    }

    /// Writes into an allocated region; bytes outside every region are dropped.
    pub fn write(&mut self, addr: u64, bytes: &[u8]) {
        if let Some((start, region)) = self.regions.range_mut(..=addr).next_back() {
            let at = usize::try_from(addr - start).unwrap_or(usize::MAX);
            if at <= region.len() {
                let end = (at + bytes.len()).min(region.len());
                region[at..end].copy_from_slice(&bytes[..end - at]);
            }
        }
    }

    pub fn write_u8(&mut self, addr: u64, value: u8) {
        self.write(addr, &[value]);
    }
    pub fn write_i32(&mut self, addr: u64, value: i32) {
        self.write(addr, &value.to_le_bytes());
    }
    pub fn write_u32(&mut self, addr: u64, value: u32) {
        self.write(addr, &value.to_le_bytes());
    }
    pub fn write_u64(&mut self, addr: u64, value: u64) {
        self.write(addr, &value.to_le_bytes());
    }
    pub fn write_f32(&mut self, addr: u64, value: f32) {
        self.write(addr, &value.to_le_bytes());
    }
}

impl Memory for Synthetic {
    fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
        let Some((start, region)) = self.regions.range(..=addr).next_back() else {
            return false;
        };
        let Ok(at) = usize::try_from(addr - start) else {
            return false;
        };
        match at.checked_add(buf.len()) {
            Some(end) if end <= region.len() => {
                buf.copy_from_slice(&region[at..end]);
                true
            }
            _ => false,
        }
    }
}

/// No calls: `GetAllItems` finds nothing.
pub struct NoCalls;

impl Calls for NoCalls {
    fn process_event(&self, _object: u64, _function: u64, _params: &mut [u64]) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_inside_regions_only() {
        let mut mem = Synthetic::new();
        let a = mem.put(&[1, 2, 3, 4]);
        let mut buf = [0u8; 2];
        assert!(mem.read(a + 2, &mut buf));
        assert_eq!(buf, [3, 4]);
        assert!(!mem.read(a + 3, &mut buf), "runs past the region");
        assert!(!mem.read(0, &mut buf), "null");
        assert!(!mem.read(a + 0x100, &mut buf), "gap");
    }
}
