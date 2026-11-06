// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The memory guard (spec §8, Director patch 0022): a new hunt starts only when the memory available now, minus
//! what hunts started recently still have to allocate, covers one more hunt and the floor.

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rustic_isles::Settings;

const MIB: u64 = 1024 * 1024;

/// The memory guard's settings (`GAMESERVER_MEMORY_GUARD` and friends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Whether the guard is on.
    pub enabled: bool,
    /// Memory (MiB) that must stay free besides the new hunt (`GAMESERVER_MIN_FREE_MB`, default 1024).
    pub min_free_mb: u64,
    /// Memory (MiB) one hunt is reckoned to need (default 1536).
    pub hunt_mb: u64,
    /// How long a start's reservation holds (`GAMESERVER_MEMORY_RESERVATION_SECONDS`, default 120).
    pub reservation_seconds: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            enabled: false,
            min_free_mb: 1024,
            hunt_mb: 1536,
            reservation_seconds: 120,
        }
    }
}

/// What the guard sees now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// Whether the guard is on.
    pub enabled: bool,
    /// Read only when enabled; none when it cannot be measured
    pub available_mb: Option<u64>,
    /// MiB still held by recent starts.
    pub reserved_mb: u64,
    /// MiB a hunt needs: its own plus the floor.
    pub needed_mb: u64,
}

/// The guard's refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntMemoryUnavailable {
    /// The text the caller and the log show.
    pub reason: String,
}

impl fmt::Display for HuntMemoryUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for HuntMemoryUnavailable {}

fn number(settings: &Settings, name: &str, default: u64, min: u64, max: u64, problems: &mut Vec<String>) -> u64 {
    let Some(raw) = settings.raw(name) else {
        return default;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return default;
    }
    let value = trimmed
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| trimmed.parse::<u64>().ok())
        .flatten()
        .filter(|v| (min..=max).contains(v));
    value.unwrap_or_else(|| {
        problems.push(format!(
            "{name}='{trimmed}' is not a whole number from {min} to {max}; using {default}"
        ));
        default
    })
}

/// The options and the problems to warn at build, even with the guard off.
pub fn options_from_settings(settings: &Settings) -> (Options, Vec<String>) {
    let enabled = settings
        .raw("GAMESERVER_MEMORY_GUARD")
        .map(|v| v.trim().to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"));
    let mut problems = Vec::new();
    let options = Options {
        enabled,
        min_free_mb: number(settings, "GAMESERVER_MIN_FREE_MB", 1024, 0, 1_048_576, &mut problems),
        hunt_mb: number(settings, "GAMESERVER_HUNT_MEMORY_MB", 1536, 1, 1_048_576, &mut problems),
        reservation_seconds: number(
            settings,
            "GAMESERVER_MEMORY_RESERVATION_SECONDS",
            120,
            0,
            3600,
            &mut problems,
        ),
    };
    (options, problems)
}

/// `MemAvailable` of a meminfo file when `linux`, else (or when it cannot be read) the free physical memory.
pub fn read_available_bytes(path: &Path, linux: bool) -> Option<u64> {
    let from_meminfo = linux
        .then(|| std::fs::read_to_string(path).ok())
        .flatten()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let rest = line.strip_prefix("MemAvailable:")?;
                let kb = rest.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
                kb.checked_mul(1024)
            })
        });
    from_meminfo.or_else(crate::os::free_physical_bytes)
}

fn system_available() -> Option<u64> {
    read_available_bytes(Path::new("/proc/meminfo"), cfg!(target_os = "linux"))
}

/// A clock in ms.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
/// Available memory in bytes; none when it cannot be measured.
pub type Available = Arc<dyn Fn() -> Option<u64> + Send + Sync>;

struct Reservation {
    id: u64,
    bytes: u64,
    until_ms: u64,
}

/// The memory guard (§8): admits a hunt only when there is room for it.
pub struct MemoryAdmission {
    options: Options,
    available: Available,
    now: Clock,
    held: Mutex<Vec<Reservation>>,
    next_id: AtomicU64,
}

impl fmt::Debug for MemoryAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryAdmission")
            .field("options", &self.options)
            .finish()
    }
}

impl MemoryAdmission {
    /// A guard reading the system's memory on the monotonic clock.
    pub fn new(options: Options) -> Self {
        Self::with(options, Arc::new(system_available), Arc::new(crate::hub::monotonic_ms))
    }

    /// Injected memory reading and clock (ms), for tests.
    pub fn with(options: Options, available: Available, now: Clock) -> Self {
        Self {
            options,
            available,
            now,
            held: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Whether the guard is on.
    pub fn enabled(&self) -> bool {
        self.options.enabled
    }

    /// Its settings.
    pub fn options(&self) -> Options {
        self.options
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Reservation>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn status_of(&self, held: &mut Vec<Reservation>) -> Status {
        let now = (self.now)();
        held.retain(|r| r.until_ms > now);
        let bytes: u64 = held.iter().map(|r| r.bytes).sum();
        Status {
            enabled: self.options.enabled,
            available_mb: if self.options.enabled {
                (self.available)().map(|b| b / MIB)
            } else {
                None
            },
            reserved_mb: (bytes as f64 / MIB as f64).round() as u64,
            needed_mb: self.options.hunt_mb + self.options.min_free_mb,
        }
    }

    /// What the guard sees now, expired reservations dropped.
    pub fn status(&self) -> Status {
        self.status_of(&mut self.held())
    }

    /// The reservation's id when one was made; the guard never blocks a server it cannot measure.
    pub fn reserve(&self) -> Result<Option<u64>, HuntMemoryUnavailable> {
        if !self.options.enabled {
            return Ok(None);
        }
        let mut held = self.held();
        let status = self.status_of(&mut held);
        let Some(available) = status.available_mb else {
            return Ok(None);
        };
        if (available as i128) - (status.reserved_mb as i128) < status.needed_mb as i128 {
            return Err(HuntMemoryUnavailable {
                reason: format!(
                    "Not enough free memory for another hunt server ({available} MB available, {} MB reserved for \
                     hunts still starting, {} MB needed)",
                    status.reserved_mb, status.needed_mb
                ),
            });
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        held.push(Reservation {
            id,
            bytes: self.options.hunt_mb * MIB,
            until_ms: (self.now)().saturating_add(self.options.reservation_seconds * 1000),
        });
        Ok(Some(id))
    }

    /// A start that failed gives its reservation back at once.
    pub fn give_back(&self, id: u64) {
        self.held().retain(|r| r.id != id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;

    fn guard(mb: u64, clock: Arc<AtomicU64>) -> MemoryAdmission {
        let options = Options {
            enabled: true,
            ..Options::default()
        };
        let now = Arc::clone(&clock);
        MemoryAdmission::with(
            options,
            Arc::new(move || Some(mb * MIB)),
            Arc::new(move || now.load(Ordering::SeqCst)),
        )
    }

    #[test]
    fn the_spec_table() {
        let clock = Arc::new(AtomicU64::new(0));
        assert!(guard(2560, clock.clone()).reserve().unwrap().is_some());
        assert!(guard(2559, clock.clone()).reserve().is_err());
        let g = guard(5000, clock.clone());
        g.reserve().unwrap();
        let second = g.reserve().unwrap().unwrap();
        assert_eq!(g.status().reserved_mb, 3072);
        assert_eq!(
            g.reserve().unwrap_err().to_string(),
            "Not enough free memory for another hunt server (5000 MB available, 3072 MB reserved for hunts still \
             starting, 2560 MB needed)"
        );
        g.give_back(second);
        assert!(g.reserve().unwrap().is_some());
        clock.store(120_000, Ordering::SeqCst);
        assert_eq!(g.status().reserved_mb, 0);
    }

    #[test]
    fn options() {
        let (options, problems) = options_from_settings(&Settings::default());
        assert_eq!(options, Options::default());
        assert!(problems.is_empty());
        let (options, problems) = options_from_settings(&Settings::from_pairs([
            ("GAMESERVER_MEMORY_GUARD", "1"),
            ("GAMESERVER_MIN_FREE_MB", "512"),
            ("GAMESERVER_HUNT_MEMORY_MB", "2048"),
        ]));
        assert!(options.enabled && problems.is_empty());
        assert_eq!(options.min_free_mb + options.hunt_mb, 2560);
        let (options, problems) = options_from_settings(&Settings::from_pairs([("GAMESERVER_MIN_FREE_MB", " lots ")]));
        assert_eq!(options.min_free_mb, 1024);
        assert_eq!(
            problems,
            ["GAMESERVER_MIN_FREE_MB='lots' is not a whole number from 0 to 1048576; using 1024"]
        );
        let g = MemoryAdmission::with(options, Arc::new(|| None), Arc::new(|| 0));
        assert_eq!(g.reserve(), Ok(None));
    }

    #[test]
    fn meminfo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meminfo");
        std::fs::write(&path, "MemTotal: 1 kB\nMemAvailable:    8192000 kB\n").unwrap();
        assert_eq!(read_available_bytes(&path, true), Some(8_192_000 * 1024));
        let fallback = read_available_bytes(&dir.path().join("none"), true);
        assert!(fallback.is_none_or(|b| b > 0));
    }
}
