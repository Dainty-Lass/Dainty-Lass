// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Loading the runtime and the Host tools into a game started suspended (RD9 as of 2026-11-12,
//! `docs/launcher-injection.md`). Only a process the caller just created suspended, only DLLs named by absolute
//! path whose SHA-256 is checked through a handle held deny-write until the game has loaded them, and a game whose
//! load failed is never resumed. The Windows launcher calls [`load_and_resume`] itself; under Proton it runs the
//! `dainty-inject.exe` helper ([`helper`]) inside Wine, which does the same there.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub mod helper;

/// A file bigger than this is no DLL that is loaded.
const MAX_DLL_BYTES: u64 = 256 * 1024 * 1024;

/// A DLL to load into the game, and the SHA-256 it must have when it is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectDll {
    /// Absolute path, given to `LoadLibraryW` as is
    pub path: PathBuf,
    /// Lowercase hex
    pub sha256: String,
}

impl InjectDll {
    /// `path` as it is now, for a DLL the host picked (Dumper-7) or built (CatalogExporter): the pick is the trust.
    pub fn as_found(path: &Path) -> Result<Self, InjectError> {
        let (_, sha256) = open_checked(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            sha256,
        })
    }
}

/// Why the DLLs were not loaded; the game, if started, must be killed.
#[derive(Debug)]
pub enum InjectError {
    /// Not an absolute path ending in `.dll`
    BadPath(PathBuf),
    /// The file could not be opened or read
    Unreadable(PathBuf, io::Error),
    /// The file is not the one checked
    Changed(PathBuf),
    /// The game did not load it
    NotLoaded(PathBuf, String),
    /// The game's process could not be reached or resumed
    Process(String),
    /// This OS does not inject
    Unsupported,
}

impl fmt::Display for InjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadPath(path) => write!(f, "{} is not an absolute .dll path", path.display()),
            Self::Unreadable(path, error) => write!(f, "{} could not be read: {error}", path.display()),
            Self::Changed(path) => write!(f, "{} is not the file that was checked", path.display()),
            Self::NotLoaded(path, why) => write!(f, "the game did not load {}: {why}", path.display()),
            Self::Process(why) => write!(f, "the game's process: {why}"),
            Self::Unsupported => f.write_str("this platform does not inject"),
        }
    }
}

impl std::error::Error for InjectError {}

impl From<InjectError> for io::Error {
    fn from(error: InjectError) -> Self {
        let kind = match error {
            InjectError::Unsupported => io::ErrorKind::Unsupported,
            _ => io::ErrorKind::Other,
        };
        Self::new(kind, error.to_string())
    }
}

fn is_dll_path(path: &Path) -> bool {
    path.is_absolute() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
}

/// Opened so no one can write, rename or delete it while held: the bytes hashed are the bytes loaded.
fn open_held(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The held file and its SHA-256.
fn open_checked(path: &Path) -> Result<(File, String), InjectError> {
    if !is_dll_path(path) {
        return Err(InjectError::BadPath(path.to_path_buf()));
    }
    let unreadable = |error| InjectError::Unreadable(path.to_path_buf(), error);
    let mut file = open_held(path).map_err(unreadable)?;
    let meta = file.metadata().map_err(unreadable)?;
    if !meta.is_file() || meta.len() > MAX_DLL_BYTES {
        return Err(InjectError::BadPath(path.to_path_buf()));
    }
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(unreadable)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok((file, lower_hex(&hasher.finalize())))
}

/// Opens every DLL held and checks its hash; the files stay held while the result lives.
pub fn hold(dlls: &[InjectDll]) -> Result<Vec<File>, InjectError> {
    dlls.iter()
        .map(|dll| {
            let (file, sha256) = open_checked(&dll.path)?;
            if sha256.eq_ignore_ascii_case(&dll.sha256) {
                Ok(file)
            } else {
                Err(InjectError::Changed(dll.path.clone()))
            }
        })
        .collect()
}

/// Loads `dlls`, in order, into the process `process` (`pid`), which the caller just created suspended and still
/// owns, then resumes it. The caller kills the process on an error.
#[cfg(windows)]
pub fn load_and_resume(
    process: std::os::windows::io::RawHandle,
    pid: u32,
    dlls: &[InjectDll],
) -> Result<(), InjectError> {
    let process_error = |e: io::Error| InjectError::Process(e.to_string());
    let held = hold(dlls)?;
    // Before any thread of ours runs in it: a process created suspended has only its main thread
    let main = os::only_thread(pid).map_err(process_error)?;
    // A no-op thread first runs the loader's start-up, which Wine may not do on a thread that loads a DLL
    os::start_up(process).map_err(process_error)?;
    for dll in dlls {
        os::load(process, pid, &dll.path).map_err(|e| InjectError::NotLoaded(dll.path.clone(), e.to_string()))?;
    }
    drop(held);
    os::resume(main).map_err(process_error)
}

#[cfg(windows)]
mod os {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::RawHandle;
    use std::path::Path;

    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPTHREAD,
        THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
    };
    use windows::Win32::System::Threading::{
        CreateRemoteThread, GetExitCodeThread, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME, WaitForSingleObject,
    };
    use windows::core::{PCSTR, s, w};

    /// How long one remote thread (a DLL's `DllMain`, say) may take before the game is given up on.
    const THREAD_TIMEOUT_MS: u32 = 60_000;

    type ThreadStart = unsafe extern "system" fn(*mut c_void) -> u32;

    fn size_u32<T>() -> u32 {
        u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
    }

    /// The one thread of `pid`; an error when it has none or more than one.
    #[allow(unsafe_code, reason = "the Toolhelp32 thread walk; see the SAFETY notes")]
    pub(super) fn only_thread(pid: u32) -> io::Result<u32> {
        // SAFETY: no pointers in; the handle is closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }?;
        let mut entry = THREADENTRY32 {
            dwSize: size_u32::<THREADENTRY32>(),
            ..Default::default()
        };
        let mut threads = Vec::new();
        // SAFETY: `entry` is a THREADENTRY32 with its size set, alive for each call.
        let mut more = unsafe { Thread32First(snapshot, &raw mut entry) }.is_ok();
        while more {
            if entry.th32OwnerProcessID == pid {
                threads.push(entry.th32ThreadID);
            }
            // SAFETY: as above.
            more = unsafe { Thread32Next(snapshot, &raw mut entry) }.is_ok();
        }
        // SAFETY: the snapshot handle is ours and closed once.
        let _ = unsafe { CloseHandle(snapshot) };
        match threads.as_slice() {
            [only] => Ok(*only),
            _ => Err(io::Error::other(format!("{} threads, not one", threads.len()))),
        }
    }

    /// A kernel32 function's address, the same in the game: both are x64 and kernel32 has one base per boot
    /// (and one fixed base under Wine).
    #[allow(
        unsafe_code,
        reason = "GetProcAddress, and its result as a thread start; see the SAFETY notes"
    )]
    fn kernel32_start(name: PCSTR) -> io::Result<ThreadStart> {
        // SAFETY: a static name; kernel32 is loaded in every process.
        let kernel32 = unsafe { GetModuleHandleW(w!("kernel32.dll")) }?;
        // SAFETY: a static name, in a module that stays loaded.
        let proc = unsafe { GetProcAddress(kernel32, name) }.ok_or_else(io::Error::last_os_error)?;
        // SAFETY: only used for LoadLibraryW and Sleep, which take one pointer-sized argument; the u32 exit code
        // truncates LoadLibraryW's handle, which `load` does not rely on alone. Both are the "system" ABI on x64.
        Ok(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, ThreadStart>(proc) })
    }

    /// Runs `start(argument)` on a thread of its own in `process` and waits for it; its exit code.
    #[allow(unsafe_code, reason = "a remote thread in the game; see the SAFETY notes")]
    fn run_remote(process: HANDLE, start: ThreadStart, argument: Option<*const c_void>) -> io::Result<u32> {
        // SAFETY: `start` is a kernel32 function taking one argument, and `argument` is null or lives in `process`.
        let thread = unsafe { CreateRemoteThread(process, None, 0, Some(start), argument, 0, None) }?;
        // SAFETY: `thread` is ours until closed below.
        let waited = unsafe { WaitForSingleObject(thread, THREAD_TIMEOUT_MS) };
        let mut code = 0u32;
        // SAFETY: as above; `code` outlives the call.
        let got = unsafe { GetExitCodeThread(thread, &raw mut code) };
        // SAFETY: closed once.
        let _ = unsafe { CloseHandle(thread) };
        if waited != WAIT_OBJECT_0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the thread in the game did not return",
            ));
        }
        got?;
        Ok(code)
    }

    /// `Sleep(0)` in the game: the first thread to run in a suspended process does the loader's start-up.
    pub(super) fn start_up(process: RawHandle) -> io::Result<()> {
        run_remote(HANDLE(process), kernel32_start(s!("Sleep"))?, None).map(|_| ())
    }

    /// `path`'s file name is among `pid`'s modules.
    #[allow(unsafe_code, reason = "the Toolhelp32 module walk; see the SAFETY notes")]
    fn has_module(pid: u32, path: &Path) -> io::Result<bool> {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
            return Ok(false);
        };
        // SAFETY: no pointers in; the handle is closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid) }?;
        let mut entry = MODULEENTRY32W {
            dwSize: size_u32::<MODULEENTRY32W>(),
            ..Default::default()
        };
        let mut found = false;
        // SAFETY: `entry` is a MODULEENTRY32W with its size set, alive for each call.
        let mut more = unsafe { Module32FirstW(snapshot, &raw mut entry) }.is_ok();
        while more && !found {
            let len = entry
                .szModule
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szModule.len());
            found = String::from_utf16_lossy(&entry.szModule[..len]).to_lowercase() == name;
            // SAFETY: as above.
            more = unsafe { Module32NextW(snapshot, &raw mut entry) }.is_ok();
        }
        // SAFETY: the snapshot handle is ours and closed once.
        let _ = unsafe { CloseHandle(snapshot) };
        Ok(found)
    }

    /// `LoadLibraryW(path)` on a thread of its own in the game.
    #[allow(unsafe_code, reason = "remote memory in the game; see the SAFETY notes")]
    pub(super) fn load(process: RawHandle, pid: u32, path: &Path) -> io::Result<()> {
        let process = HANDLE(process);
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let bytes = wide.len() * size_of::<u16>();
        let start = kernel32_start(s!("LoadLibraryW"))?;
        // SAFETY: `process` is the live child's handle, with every access right its creator gets.
        let remote = unsafe { VirtualAllocEx(process, None, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if remote.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `remote` is `bytes` long in the game, and `wide` is `bytes` long here.
        unsafe { WriteProcessMemory(process, remote, wide.as_ptr().cast(), bytes, None) }?;
        // On a timeout the string stays: the thread may still read it, and the caller kills the game
        let code = run_remote(process, start, Some(remote.cast_const()))?;
        // SAFETY: the thread is done with `remote`.
        let _ = unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
        // A module whose low 32 bits are zero reads as 0 here, so a zero is checked against the module list
        if code != 0 || has_module(pid, path)? {
            Ok(())
        } else {
            Err(io::Error::other("LoadLibraryW returned no module"))
        }
    }

    /// Resumes the thread the process was created suspended with.
    #[allow(unsafe_code, reason = "OpenThread and ResumeThread; see the SAFETY notes")]
    pub(super) fn resume(thread_id: u32) -> io::Result<()> {
        // SAFETY: no pointers in; the handle is closed below.
        let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, thread_id) }?;
        // SAFETY: `thread` was opened with THREAD_SUSPEND_RESUME just now.
        let previous = unsafe { ResumeThread(thread) };
        let error = io::Error::last_os_error();
        // SAFETY: the thread handle is ours and closed once.
        let _ = unsafe { CloseHandle(thread) };
        if previous == u32::MAX { Err(error) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn dll(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_dll_is_held_only_with_the_checked_hash() {
        let temp = tempfile::tempdir().unwrap();
        let path = dll(temp.path(), "rustic-isles-runtime.dll", b"abc");
        let found = InjectDll::as_found(&path).unwrap();
        assert_eq!(found.sha256, ABC_SHA256);
        assert_eq!(hold(std::slice::from_ref(&found)).unwrap().len(), 1);

        let wrong = InjectDll {
            path: path.clone(),
            sha256: "0".repeat(64),
        };
        assert!(matches!(hold(&[found, wrong]), Err(InjectError::Changed(p)) if p == path));
    }

    #[test]
    fn only_absolute_dll_paths() {
        let temp = tempfile::tempdir().unwrap();
        let txt = dll(temp.path(), "notes.txt", b"abc");
        for bad in [
            PathBuf::from("rustic-isles-runtime.dll"),
            PathBuf::from("../x.dll"),
            txt,
        ] {
            assert!(
                matches!(InjectDll::as_found(&bad), Err(InjectError::BadPath(_))),
                "{}",
                bad.display()
            );
        }
        let missing = temp.path().join("Gone.dll");
        assert!(matches!(
            InjectDll::as_found(&missing),
            Err(InjectError::Unreadable(..))
        ));
        let upper = dll(temp.path(), "TOOL.DLL", b"abc");
        assert!(InjectDll::as_found(&upper).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn a_held_dll_cannot_be_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let path = dll(temp.path(), "rustic-isles-runtime.dll", b"abc");
        let held = hold(&[InjectDll::as_found(&path).unwrap()]).unwrap();
        assert!(std::fs::write(&path, b"evil").is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(held);
        std::fs::write(&path, b"fine now").unwrap();
    }
}
