//! The flag-file cache policy (R0 4.2): `<exe folder>\<NAME>.flag` presence, remembered for 4 s per name.

use std::sync::{Mutex, OnceLock};

/// Names the table remembers.
pub const TABLE_SIZE: usize = 128;
/// The longest name (UTF-16 units) the table remembers; longer ones are probed on every call.
pub const MAX_NAME_UNITS: usize = 63;
/// How long an answer is kept.
pub const REFRESH_MS: u64 = 4000;
/// The flag that turns the cache off for the process.
pub const UNCACHED_FLAG: &str = "UNCACHED_FLAG_FILES";

/// R0's flags (RT-14 for R0); later slices add their own names where they read them.
pub mod names {
    /// Servers: skip the traces a slice marks production-quiet.
    pub const PRODUCTION_LOG_PROFILE: &str = "PRODUCTION_LOG_PROFILE";
    /// High-volume traces, only with the development profile.
    pub const VERBOSE_DIAG: &str = "VERBOSE_DIAG";
    /// The server's frame pointer cache off.
    pub const NO_FRAME_POINTER_CACHE: &str = "NO_FRAME_POINTER_CACHE";
    /// The server builds the full UFunction name per call.
    pub const PE_FULLNAME_UNCACHED: &str = "PE_FULLNAME_UNCACHED";
}

/// Asks the file system whether `<exe folder>\<name>.flag` exists.
pub trait Probe {
    /// True when the entry exists (a folder counts).
    fn exists(&self, name: &str) -> bool;
}

/// A millisecond tick (`GetTickCount64` in the DLL).
pub trait Clock {
    /// Milliseconds since an arbitrary start.
    fn now_ms(&self) -> u64;
}

#[derive(Clone, Copy)]
struct Entry {
    name: [u16; MAX_NAME_UNITS],
    len: usize,
    present: bool,
    tick: u64,
}

impl Entry {
    fn name(&self) -> &[u16] {
        &self.name[..self.len]
    }
}

/// The process's flag cache. `const`-constructible so the DLL keeps one in a static.
pub struct FlagCache {
    uncached: OnceLock<bool>,
    table: Mutex<Vec<Entry>>,
}

impl Default for FlagCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FlagCache {
    /// An empty cache whose mode is not decided yet.
    pub const fn new() -> Self {
        Self {
            uncached: OnceLock::new(),
            table: Mutex::new(Vec::new()),
        }
    }

    /// True when every probe goes to the file system. Decided once, on the first call, by probing
    /// `UNCACHED_FLAG_FILES` directly.
    pub fn uncached(&self, probe: &dyn Probe) -> bool {
        *self.uncached.get_or_init(|| probe.exists(UNCACHED_FLAG))
    }

    /// Whether flag `name` is present, by the 4 s rule.
    pub fn is_present(&self, name: &str, probe: &dyn Probe, clock: &dyn Clock) -> bool {
        if self.uncached(probe) {
            return probe.exists(name);
        }
        let units: Vec<u16> = name.encode_utf16().collect();
        if units.len() > MAX_NAME_UNITS {
            return probe.exists(name);
        }
        let now = clock.now_ms();
        if let Ok(table) = self.table.lock()
            && let Some(entry) = table.iter().find(|entry| entry.name() == units.as_slice())
            && now.saturating_sub(entry.tick) < REFRESH_MS
        {
            return entry.present;
        }
        // The probe runs outside the lock (R0 4.2).
        let present = probe.exists(name);
        let tick = clock.now_ms().max(1);
        if let Ok(mut table) = self.table.lock() {
            if let Some(entry) = table.iter_mut().find(|entry| entry.name() == units.as_slice()) {
                entry.present = present;
                entry.tick = tick;
            } else if table.len() < TABLE_SIZE {
                let mut stored = [0u16; MAX_NAME_UNITS];
                stored[..units.len()].copy_from_slice(&units);
                table.push(Entry {
                    name: stored,
                    len: units.len(),
                    present,
                    tick,
                });
            }
        }
        present
    }

    /// The stored tick for `name` (tests).
    pub fn stored_tick(&self, name: &str) -> Option<u64> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let table = self.table.lock().ok()?;
        table
            .iter()
            .find(|entry| entry.name() == units.as_slice())
            .map(|entry| entry.tick)
    }
}

/// The status line for the cache mode (R0 4.3).
pub fn mode_line(uncached: bool) -> &'static str {
    if uncached {
        "[MitRuntime] flag-file cache off (UNCACHED_FLAG_FILES.flag)"
    } else {
        "[MitRuntime] flag-file cache on (re-checked every 4 s)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    #[derive(Default)]
    struct FakeProbe {
        present: RefCell<HashSet<String>>,
        calls: Cell<usize>,
    }

    impl Probe for FakeProbe {
        fn exists(&self, name: &str) -> bool {
            if name != UNCACHED_FLAG {
                self.calls.set(self.calls.get() + 1);
            }
            self.present.borrow().contains(name)
        }
    }

    struct FakeClock(Cell<u64>);

    impl Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.get()
        }
    }

    #[test]
    fn answer_kept_for_four_seconds() {
        let cache = FlagCache::new();
        let probe = FakeProbe::default();
        let clock = FakeClock(Cell::new(10_000));
        assert!(!cache.is_present("A", &probe, &clock));
        probe.present.borrow_mut().insert("A".into());
        clock.0.set(13_999);
        assert!(!cache.is_present("A", &probe, &clock));
        assert_eq!(probe.calls.get(), 1);
        clock.0.set(14_000);
        assert!(cache.is_present("A", &probe, &clock));
        assert_eq!(probe.calls.get(), 2);
    }

    #[test]
    fn full_table_probes_every_call() {
        let cache = FlagCache::new();
        let probe = FakeProbe::default();
        let clock = FakeClock(Cell::new(5));
        for index in 0..TABLE_SIZE {
            cache.is_present(&format!("F{index}"), &probe, &clock);
        }
        let before = probe.calls.get();
        cache.is_present("F128", &probe, &clock);
        cache.is_present("F128", &probe, &clock);
        assert_eq!(probe.calls.get(), before + 2);
        cache.is_present("F0", &probe, &clock);
        assert_eq!(probe.calls.get(), before + 2);
    }

    #[test]
    fn long_name_probes_every_call() {
        let cache = FlagCache::new();
        let probe = FakeProbe::default();
        let clock = FakeClock(Cell::new(5));
        let long = "N".repeat(64);
        cache.is_present(&long, &probe, &clock);
        cache.is_present(&long, &probe, &clock);
        assert_eq!(probe.calls.get(), 2);
        let longest = "N".repeat(63);
        cache.is_present(&longest, &probe, &clock);
        cache.is_present(&longest, &probe, &clock);
        assert_eq!(probe.calls.get(), 3);
    }

    #[test]
    fn uncached_mode_probes_every_call() {
        let cache = FlagCache::new();
        let probe = FakeProbe::default();
        probe.present.borrow_mut().insert(UNCACHED_FLAG.into());
        let clock = FakeClock(Cell::new(5));
        cache.is_present("A", &probe, &clock);
        cache.is_present("A", &probe, &clock);
        assert_eq!(probe.calls.get(), 2);
        // Decided once: removing the flag later changes nothing
        probe.present.borrow_mut().clear();
        assert!(cache.uncached(&probe));
    }

    #[test]
    fn tick_zero_stored_as_one() {
        let cache = FlagCache::new();
        let probe = FakeProbe::default();
        let clock = FakeClock(Cell::new(0));
        cache.is_present("Z", &probe, &clock);
        assert_eq!(cache.stored_tick("Z"), Some(1));
    }

    #[test]
    fn mode_lines() {
        assert_eq!(
            mode_line(false),
            "[MitRuntime] flag-file cache on (re-checked every 4 s)"
        );
        assert_eq!(
            mode_line(true),
            "[MitRuntime] flag-file cache off (UNCACHED_FLAG_FILES.flag)"
        );
    }
}
