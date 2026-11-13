//! The built DLL's tables, read from the file (R0 10.1): the five seam exports by name, and no dynamic CRT
//! import (the game folder ships an old msvcp140.dll the loader would prefer).
#![cfg(all(windows, target_arch = "x86_64"))]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::PathBuf;

/// `target/<profile>/rustic_isles_runtime.dll`, next to (or one above) this test's `deps` folder.
fn dll_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let deps = exe.parent().unwrap();
    [deps.to_path_buf(), deps.parent().unwrap().to_path_buf()]
        .into_iter()
        .map(|dir| dir.join("rustic_isles_runtime.dll"))
        .find(|path| path.exists())
        .expect("cargo builds the cdylib beside the tests")
}

struct Pe {
    bytes: Vec<u8>,
    sections: Vec<(u32, u32, u32)>,
    directories: usize,
}

impl Pe {
    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes(self.bytes[at..at + 2].try_into().unwrap())
    }
    fn u32(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap())
    }
    fn parse(bytes: Vec<u8>) -> Pe {
        let mut pe = Pe {
            bytes,
            sections: Vec::new(),
            directories: 0,
        };
        assert_eq!(&pe.bytes[0..2], b"MZ");
        let nt = pe.u32(0x3C) as usize;
        assert_eq!(&pe.bytes[nt..nt + 4], b"PE\0\0");
        let count = pe.u16(nt + 6) as usize;
        let optional = nt + 24;
        assert_eq!(pe.u16(optional), 0x20B, "PE32+");
        pe.directories = optional + 112;
        let table = optional + pe.u16(nt + 20) as usize;
        for index in 0..count {
            let at = table + index * 40;
            pe.sections
                .push((pe.u32(at + 12), pe.u32(at + 8).max(pe.u32(at + 16)), pe.u32(at + 20)));
        }
        pe
    }
    fn offset(&self, rva: u32) -> usize {
        let (base, _, raw) = self
            .sections
            .iter()
            .find(|(base, size, _)| rva >= *base && rva < base + size)
            .unwrap();
        (rva - base + raw) as usize
    }
    fn directory(&self, index: usize) -> (u32, u32) {
        let at = self.directories + index * 8;
        (self.u32(at), self.u32(at + 4))
    }
    fn text(&self, rva: u32) -> String {
        let at = self.offset(rva);
        let end = self.bytes[at..].iter().position(|&byte| byte == 0).unwrap();
        String::from_utf8_lossy(&self.bytes[at..at + end]).into_owned()
    }
    fn exports(&self) -> Vec<String> {
        let (rva, _) = self.directory(0);
        let at = self.offset(rva);
        let count = self.u32(at + 24);
        let names = self.offset(self.u32(at + 32));
        (0..count as usize)
            .map(|index| self.text(self.u32(names + index * 4)))
            .collect()
    }
    fn imports(&self) -> Vec<String> {
        let (rva, _) = self.directory(1);
        let mut at = self.offset(rva);
        let mut names = Vec::new();
        while self.u32(at + 12) != 0 {
            names.push(self.text(self.u32(at + 12)));
            at += 20;
        }
        names
    }
}

#[test]
fn seam_exports_by_name() {
    let pe = Pe::parse(std::fs::read(dll_path()).unwrap());
    let exports = pe.exports();
    for name in [
        "rir_seam_version",
        "rir_owns",
        "rir_chain_add_inner",
        "rir_chain_remove_inner",
        "rir_original",
    ] {
        assert!(
            exports.iter().any(|export| export == name),
            "{name} missing from {exports:?}"
        );
    }
    assert_eq!(rustic_isles_runtime::ffi::exports::rir_seam_version(), 1);
}

#[test]
fn no_dynamic_crt_import() {
    let pe = Pe::parse(std::fs::read(dll_path()).unwrap());
    let imports = pe.imports();
    assert!(!imports.is_empty());
    for name in &imports {
        let lower = name.to_ascii_lowercase();
        assert!(
            !lower.starts_with("msvcp140")
                && !lower.starts_with("vcruntime140")
                && !lower.starts_with("api-ms-win-crt-"),
            "{name} imported: {imports:?}"
        );
    }
}

#[test]
fn imports_ws2_32_statically() {
    // The loader maps it before DllMain, so CL-8/9 never miss their exports (R4 6.1)
    let pe = Pe::parse(std::fs::read(dll_path()).unwrap());
    let imports = pe.imports();
    assert!(
        imports.iter().any(|name| name.eq_ignore_ascii_case("ws2_32.dll")),
        "{imports:?}"
    );
}
