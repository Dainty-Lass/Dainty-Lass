//! Loads the `winmm.dll` cargo built and checks it against the system's own.
#![cfg(windows)]
#![allow(clippy::unwrap_used, reason = "test helpers fail the test")]
#![allow(unsafe_code, reason = "loads and calls the built DLL through kernel32")]

use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const EXPECTED: &[(&str, usize)] = &include!("expected_exports.in");

type Module = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> Module;
    fn GetProcAddress(module: Module, name: *const u8) -> usize;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
}

fn load(path: &Path) -> Module {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    assert!(!module.is_null(), "LoadLibraryW({})", path.display());
    module
}

fn proc(module: Module, name: &str) -> usize {
    let name = format!("{name}\0");
    unsafe { GetProcAddress(module, name.as_ptr()) }
}

fn system_directory() -> PathBuf {
    let mut buffer = vec![0u16; 32_768];
    let len = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), 32_768) };
    assert!(len > 0 && len < 32_768);
    PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len as usize]))
}

/// `target/<profile>/deps/<test>.exe` -> `target/<profile>/winmm.dll`, built first: `cargo test` alone builds no
/// cdylib for an integration test, and an older one would be tested instead.
fn built_dll() -> &'static Path {
    static DLL: OnceLock<PathBuf> = OnceLock::new();
    DLL.get_or_init(|| {
        let exe = std::env::current_exe().unwrap();
        let profile_dir = exe.parent().unwrap().parent().unwrap();
        let profile = match profile_dir.file_name().unwrap().to_str().unwrap() {
            "debug" => "dev",
            other => other,
        };
        let mut cargo = Command::new(env!("CARGO"));
        cargo
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["build", "--lib", "--profile", profile]);
        let parent = profile_dir.parent().unwrap();
        // The target folder holds CACHEDIR.TAG; anything else is a `--target` triple's folder inside it.
        if parent.join("CACHEDIR.TAG").exists() {
            cargo.arg("--target-dir").arg(parent);
        } else {
            cargo
                .arg("--target")
                .arg(parent.file_name().unwrap())
                .arg("--target-dir")
                .arg(parent.parent().unwrap());
        }
        assert!(cargo.status().unwrap().success(), "cargo build");
        profile_dir.join("winmm.dll")
    })
}

fn loader() -> Module {
    load(built_dll())
}

/// A NUL-terminated name at `rva` in a loaded image.
unsafe fn name_at(base: *const u8, rva: usize) -> String {
    unsafe { std::ffi::CStr::from_ptr(base.add(rva).cast()) }
        .to_str()
        .unwrap()
        .to_owned()
}

/// The RVA of data directory `index` in a loaded PE32+ image.
unsafe fn directory(base: *const u8, index: usize) -> usize {
    unsafe {
        let pe = read_u32(base, 0x3c);
        assert_eq!(std::slice::from_raw_parts(base.add(pe), 4), b"PE\0\0");
        let optional = pe + 24;
        assert_eq!(base.add(optional).cast::<u16>().read_unaligned(), 0x20b, "not PE32+");
        read_u32(base, optional + 112 + 8 * index)
    }
}

unsafe fn read_u32(base: *const u8, offset: usize) -> usize {
    unsafe { base.add(offset).cast::<u32>().read_unaligned() as usize }
}

fn exported_names(module: Module) -> Vec<String> {
    let base = module as *const u8;
    unsafe {
        let exports = directory(base, 0);
        let names = read_u32(base, exports + 32);
        (0..read_u32(base, exports + 24))
            .map(|i| name_at(base, read_u32(base, names + 4 * i)))
            .collect()
    }
}

fn imported_dlls(module: Module) -> Vec<String> {
    let base = module as *const u8;
    let mut dlls = Vec::new();
    unsafe {
        let mut descriptor = directory(base, 1);
        while read_u32(base, descriptor + 12) != 0 {
            dlls.push(name_at(base, read_u32(base, descriptor + 12)));
            descriptor += 20;
        }
    }
    dlls
}

#[test]
fn imports_neither_dll_it_stands_in_for() {
    let dlls = imported_dlls(loader());
    assert!(
        dlls.iter().any(|dll| dll.eq_ignore_ascii_case("kernel32.dll")),
        "{dlls:?}"
    );
    let own = dlls
        .iter()
        .filter(|dll| dll.eq_ignore_ascii_case("winmm.dll") || dll.eq_ignore_ascii_case("version.dll"));
    assert_eq!(own.count(), 0, "{dlls:?}");
}

#[test]
fn exports_exactly_the_spec_names_and_dllmain() {
    let mut actual = exported_names(loader());
    actual.sort_unstable();
    let mut expected: Vec<String> = EXPECTED.iter().map(|(name, _)| (*name).to_owned()).collect();
    expected.push("DllMain".to_owned());
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn every_name_resolves() {
    let module = loader();
    let missing: Vec<&str> = EXPECTED
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| proc(module, name) == 0)
        .collect();
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn calls_reach_the_system_winmm() {
    let ours = loader();
    let system = load(&system_directory().join("winmm.dll"));
    assert_ne!(ours, system);
    unsafe {
        type Nullary = unsafe extern "system" fn() -> u32;
        let call = |module: Module, name: &str| {
            let address = proc(module, name);
            assert_ne!(address, 0, "{name}");
            std::mem::transmute::<usize, Nullary>(address)()
        };
        let (before, through, after) = (
            call(system, "timeGetTime"),
            call(ours, "timeGetTime"),
            call(system, "timeGetTime"),
        );
        assert!(before <= through && through <= after, "{before} {through} {after}");
        assert!(after - before < 1_000);
        assert_eq!(call(ours, "waveOutGetNumDevs"), call(system, "waveOutGetNumDevs"));
        assert_eq!(call(ours, "mmsystemGetVersion"), call(system, "mmsystemGetVersion"));
    }
}

#[test]
fn calls_reach_the_system_version_dll() {
    let ours = loader();
    let system = load(&system_directory().join("version.dll"));
    let kernel32: Vec<u16> = OsStr::new(&system_directory().join("kernel32.dll"))
        .encode_wide()
        .chain([0])
        .collect();
    type SizeW = unsafe extern "system" fn(*const u16, *mut u32) -> u32;
    let size = |module: Module| unsafe {
        let mut handle = 0u32;
        std::mem::transmute::<usize, SizeW>(proc(module, "GetFileVersionInfoSizeW"))(kernel32.as_ptr(), &mut handle)
    };
    let theirs = size(system);
    assert_ne!(theirs, 0);
    assert_eq!(size(ours), theirs);
}
