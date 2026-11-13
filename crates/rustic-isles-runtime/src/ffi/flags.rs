//! Flag files and number files next to the game executable (R0 4.2, R1 10).

use super::win::{env_value, tick};
use rustic_isles_runtime_core::env::{PROFILE_VARIABLE, is_development};
use rustic_isles_runtime_core::flags::{Clock, FlagCache, Probe, names};
use std::io::Read;
use std::sync::OnceLock;
use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;

const MAX_PATH: usize = 260;

static FOLDER: OnceLock<Vec<u16>> = OnceLock::new();
static CACHE: FlagCache = FlagCache::new();

/// The executable's folder up to and including its last `\` or `/`; `None` for a path of 260 or more characters.
fn resolve_folder() -> Option<Vec<u16>> {
    let mut buffer = [0u16; MAX_PATH];
    // SAFETY: the buffer holds MAX_PATH units; null names the executable
    let len = unsafe { GetModuleFileNameW(std::ptr::null_mut(), buffer.as_mut_ptr(), MAX_PATH as u32) } as usize;
    if len == 0 || len >= MAX_PATH {
        return None;
    }
    let end = buffer[..len]
        .iter()
        .rposition(|&unit| unit == u16::from(b'\\') || unit == u16::from(b'/'))?;
    Some(buffer[..=end].to_vec())
}

/// The folder, resolved once when it can be; an unresolved one is resolved again by every caller.
pub fn folder() -> Option<Vec<u16>> {
    if let Some(folder) = FOLDER.get() {
        return Some(folder.clone());
    }
    let folder = resolve_folder()?;
    Some(FOLDER.get_or_init(|| folder).clone())
}

/// `<exe folder><name>`.
pub fn path_next_to_exe(name: &str) -> Option<std::path::PathBuf> {
    let mut units = folder()?;
    units.extend(name.encode_utf16());
    Some(std::path::PathBuf::from(String::from_utf16(&units).ok()?))
}

struct FileProbe;

impl Probe for FileProbe {
    fn exists(&self, name: &str) -> bool {
        let Some(mut path) = folder() else {
            return false;
        };
        path.extend(name.encode_utf16());
        path.extend(".flag".encode_utf16());
        if path.len() >= MAX_PATH {
            return false;
        }
        path.push(0);
        // SAFETY: NUL-terminated path
        unsafe { GetFileAttributesW(path.as_ptr()) != INVALID_FILE_ATTRIBUTES }
    }
}

struct TickClock;

impl Clock for TickClock {
    fn now_ms(&self) -> u64 {
        tick()
    }
}

/// Init's step 1: the folder and the cache mode. Returns true when the cache is off.
pub fn init() -> bool {
    let _ = folder();
    CACHE.uncached(&FileProbe)
}

/// Whether the cache is off (`UNCACHED_FLAG_FILES.flag`).
pub fn uncached() -> bool {
    CACHE.uncached(&FileProbe)
}

/// Whether `<NAME>.flag` is present. Always false after detach (R0 3.3).
pub fn flag(name: &str) -> bool {
    if super::detaching() {
        return false;
    }
    CACHE.is_present(name, &FileProbe, &TickClock)
}

/// The launcher's development profile, read once.
pub fn development_profile() -> bool {
    static PROFILE: OnceLock<bool> = OnceLock::new();
    *PROFILE.get_or_init(|| is_development(env_value(PROFILE_VARIABLE, 32).as_deref()))
}

/// `PRODUCTION_LOG_PROFILE.flag`, servers only, read once at first use.
pub fn production_log_profile() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| crate::config::is_server() && flag(names::PRODUCTION_LOG_PROFILE))
}

/// `VERBOSE_DIAG.flag`, only with the development profile.
pub fn verbose_diag() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| development_profile() && flag(names::VERBOSE_DIAG))
}

/// `NO_FRAME_POINTER_CACHE.flag`, read once at the first SP-3 tick.
pub fn no_frame_pointer_cache() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| flag(names::NO_FRAME_POINTER_CACHE))
}

/// `PE_FULLNAME_UNCACHED.flag`, read once at the first SP-2 dispatch.
pub fn pe_fullname_uncached() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| flag(names::PE_FULLNAME_UNCACHED))
}

/// The first `max` bytes of a file next to the executable, or `None` when it cannot be opened (R1 10).
pub fn read_number_file(name: &str, max: usize) -> Option<Vec<u8>> {
    if super::detaching() {
        return None;
    }
    let file = std::fs::File::open(path_next_to_exe(name)?).ok()?;
    let mut bytes = Vec::with_capacity(max);
    file.take(u64::try_from(max).ok()?).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}
