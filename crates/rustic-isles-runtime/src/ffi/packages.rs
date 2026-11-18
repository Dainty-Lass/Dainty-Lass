//! PKG-1 (spec-runtime-stub 3): on a server with `RUNTIME_STUB_PACKAGES.flag`, `FPackageName::DoesPackageExist`
//! answers false for refused packages, so neither the loader nor an import ever opens them.

use super::broker::{self, NextCell};
use super::memory::{read, read_bytes, verify_entry};
use super::{detaching, flags, log, minhook};
use rustic_isles_runtime_core::offsets::{FoundBy, target};
use rustic_isles_runtime_core::stub::{self as core, Rules};
use std::collections::HashSet;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

type DoesPackageExistFn = unsafe extern "C" fn(*const c_void, *const c_void, *mut c_void, u8) -> u8;

static NEXT: NextCell = NextCell::new();
static RULES: OnceLock<Rules> = OnceLock::new();
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static REFUSED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// The `FString` at `string` (`data`, `num` with the NUL), read through checked reads.
fn fstring(string: usize) -> Option<String> {
    let data = read::<usize>(string)?;
    let num = usize::try_from(read::<i32>(string + 8)?).ok()?;
    if data == 0 || num == 0 || num > core::MAX_NAME {
        return None;
    }
    let mut bytes = vec![0u8; (num - 1) * 2];
    if !read_bytes(data, &mut bytes) {
        return None;
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect();
    String::from_utf16(&units).ok()
}

/// True when the filter refuses the package `name` points at; logs the first distinct refusals.
fn refuse(name: usize) -> bool {
    let Some(rules) = RULES.get() else {
        return false;
    };
    let Some(long) = fstring(name).and_then(|given| core::long_name(&given)) else {
        return false;
    };
    if !rules.refuses(&long) {
        return false;
    }
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut seen) = REFUSED.lock() {
        let seen = seen.get_or_insert_with(HashSet::new);
        if seen.len() < usize::MAX && seen.insert(long.clone()) && seen.len() <= core::NAMED_REFUSALS {
            log::line(&core::refused_line(&long));
        }
    }
    true
}

/// The refusal totals: lookups refused and distinct packages.
pub fn totals() -> (u64, usize) {
    let distinct = REFUSED.lock().map_or(0, |seen| seen.as_ref().map_or(0, HashSet::len));
    (REFUSALS.load(Ordering::Relaxed), distinct)
}

unsafe extern "C" fn does_package_exist_detour(
    name: *const c_void,
    guid: *const c_void,
    out: *mut c_void,
    text: u8,
) -> u8 {
    let next = NEXT.load();
    if next == 0 {
        return 0;
    }
    if !detaching() && catch_unwind(AssertUnwindSafe(|| refuse(name as usize))).unwrap_or(false) {
        return 0;
    }
    // SAFETY: the broker wrote a function of DoesPackageExist's signature into this cell; the original arguments
    unsafe {
        let next = std::mem::transmute::<usize, DoesPackageExistFn>(next);
        next(name, guid, out, text)
    }
}

/// Installs PKG-1 when the flag is present: the rules from the list file, else the defaults.
pub fn install(base: usize) -> Result<(), &'static str> {
    if !flags::flag(core::PACKAGES_FLAG) {
        return Err("off (no RUNTIME_STUB_PACKAGES.flag)");
    }
    let unknown = minhook::status_name(minhook::MH_UNKNOWN);
    let line = target("PKG-1").ok_or(unknown)?;
    if let FoundBy::RvaAndBytes(prologue) = line.found_by
        && !verify_entry(line.rva, prologue)
    {
        return Err("prologue mismatch");
    }
    let text = flags::path_next_to_exe(core::PACKAGES_FILE).and_then(|path| std::fs::read_to_string(path).ok());
    let rules = text.as_deref().map_or_else(Rules::defaults, Rules::parse);
    log::line(&core::packages_on_line(&rules, text.is_some()));
    let _ = RULES.set(rules);
    let address = usize::try_from(line.rva)
        .ok()
        .and_then(|rva| base.checked_add(rva))
        .ok_or(unknown)?;
    broker::hook(address, does_package_exist_detour as *const () as usize, 0, &NEXT).map_err(minhook::status_name)
}
