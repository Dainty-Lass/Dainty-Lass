//! The server's per-frame pointer cache (R0 5.2): positive `readable` answers kept until the next epoch.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Slots in the table.
pub const SLOTS: usize = 256;

/// The slot for a pointer.
pub fn slot(ptr: usize) -> usize {
    ((ptr >> 3) ^ (ptr >> 12)) & 0xFF
}

struct Slot {
    ptr: AtomicUsize,
    size: AtomicUsize,
    epoch: AtomicU32,
}

/// The cache. Only the SP-3 thread uses it; the atomics only make it a `static`.
pub struct FrameCache {
    slots: [Slot; SLOTS],
    epoch: AtomicU32,
}

impl Default for FrameCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCache {
    /// Empty, epoch 1.
    pub const fn new() -> Self {
        Self {
            slots: [const {
                Slot {
                    ptr: AtomicUsize::new(0),
                    size: AtomicUsize::new(0),
                    epoch: AtomicU32::new(0),
                }
            }; SLOTS],
            epoch: AtomicU32::new(1),
        }
    }

    /// The current epoch.
    pub fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Relaxed)
    }

    /// A new epoch: at the top of every SP-3 tick and on both sides of the engine tick. A wrap to 0 clears
    /// every slot and restarts at 1.
    pub fn bump(&self) {
        let next = self.epoch.load(Ordering::Relaxed).wrapping_add(1);
        if next == 0 {
            for slot in &self.slots {
                slot.ptr.store(0, Ordering::Relaxed);
                slot.size.store(0, Ordering::Relaxed);
                slot.epoch.store(0, Ordering::Relaxed);
            }
            self.epoch.store(1, Ordering::Relaxed);
        } else {
            self.epoch.store(next, Ordering::Relaxed);
        }
    }

    /// Sets the epoch (tests of the wrap).
    pub fn set_epoch(&self, epoch: u32) {
        self.epoch.store(epoch, Ordering::Relaxed);
    }

    /// `readable_this_frame`: a hit, else `check` (the real `readable`), remembering only a yes.
    pub fn readable(&self, ptr: usize, size: usize, check: impl FnOnce(usize, usize) -> bool) -> bool {
        let epoch = self.epoch();
        let slot = &self.slots[slot(ptr)];
        if slot.ptr.load(Ordering::Relaxed) == ptr
            && slot.epoch.load(Ordering::Relaxed) == epoch
            && slot.size.load(Ordering::Relaxed) >= size
            && ptr != 0
        {
            return true;
        }
        let answer = check(ptr, size);
        if answer {
            slot.ptr.store(ptr, Ordering::Relaxed);
            slot.size.store(size, Ordering::Relaxed);
            slot.epoch.store(epoch, Ordering::Relaxed);
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn positive_answers_kept_for_one_epoch() {
        let cache = FrameCache::new();
        let calls = Cell::new(0);
        let check = |answer: bool| {
            let calls = &calls;
            move |_: usize, _: usize| {
                calls.set(calls.get() + 1);
                answer
            }
        };
        assert!(cache.readable(0x1000, 16, check(true)));
        assert!(cache.readable(0x1000, 8, check(false)), "smaller size hits");
        assert!(!cache.readable(0x1000, 32, check(false)), "bigger size misses");
        assert_eq!(calls.get(), 2);
        cache.bump();
        assert!(!cache.readable(0x1000, 8, check(false)), "new epoch misses");
        assert!(!cache.readable(0x2000, 8, check(false)));
        assert!(!cache.readable(0x2000, 8, check(false)), "negatives not stored");
        assert_eq!(calls.get(), 5);
    }

    #[test]
    fn wrap_clears() {
        let cache = FrameCache::new();
        cache.set_epoch(u32::MAX);
        assert!(cache.readable(0x1000, 8, |_, _| true));
        cache.bump();
        assert_eq!(cache.epoch(), 1);
        assert!(!cache.readable(0x1000, 8, |_, _| false));
    }

    #[test]
    fn slot_formula() {
        assert_eq!(slot(0x1000), ((0x1000 >> 3) ^ 1) & 0xFF);
        assert!(slot(usize::MAX) < SLOTS);
    }
}
