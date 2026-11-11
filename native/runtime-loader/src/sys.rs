//! The kernel32 calls the loader makes, declared by hand so nothing else is linked.

use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub type Module = *mut c_void;

/// Room for a long path (`\\?\` prefixed), in UTF-16 units.
const PATH_UNITS: u32 = 32_768;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleFileNameW(module: Module, buffer: *mut u16, size: u32) -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> Module;
    pub fn GetProcAddress(module: Module, name: *const u8) -> usize;
}

/// Reads a path through a `(buffer, size) -> length` call; a failed or filled buffer gives `None`.
fn read_path(call: impl FnOnce(*mut u16, u32) -> u32) -> Option<PathBuf> {
    let mut buffer = vec![0u16; PATH_UNITS as usize];
    let len = call(buffer.as_mut_ptr(), PATH_UNITS);
    if len == 0 || len >= PATH_UNITS {
        return None;
    }
    buffer.truncate(len as usize);
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer)))
}

pub fn module_path(module: Module) -> Option<PathBuf> {
    // SAFETY: the buffer is `size` units long and the call writes at most that many.
    read_path(|buffer, size| unsafe { GetModuleFileNameW(module, buffer, size) })
}

pub fn system_directory() -> Option<PathBuf> {
    // SAFETY: as above.
    read_path(|buffer, size| unsafe { GetSystemDirectoryW(buffer, size) })
}

/// `LoadLibraryW` by full path; never unloaded.
pub fn load(path: &Path) -> Option<Module> {
    let wide = wide(path.as_os_str())?;
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    (!module.is_null()).then_some(module)
}

/// NUL-terminated UTF-16; a name with an inner NUL would load a different file, so it is refused.
fn wide(text: &OsStr) -> Option<Vec<u16>> {
    let mut wide: Vec<u16> = text.encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    Some(wide)
}
