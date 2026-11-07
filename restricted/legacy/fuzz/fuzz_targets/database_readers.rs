//! A 1.4.4 database or offline save is the import's most hostile input: whoever handed the host the file chose
//! every byte. Nothing may panic, and only a database that reaches the final 1.4.4 schema may come out.
#![no_main]

use std::sync::LazyLock;

use dainty_legacy_import::revived::drsave::read_save;
use dainty_legacy_import::revived::source::{Origin, prepare};
use libfuzzer_sys::fuzz_target;
use tempfile::TempDir;

static DIR: LazyLock<Option<TempDir>> = LazyLock::new(|| tempfile::tempdir().ok());

fuzz_target!(|data: &[u8]| {
    let Some(dir) = DIR.as_ref() else {
        return;
    };
    let file = dir.path().join("input");
    let unpacked = dir.path().join("unpacked.db");
    let _ = std::fs::remove_file(&unpacked);
    if std::fs::write(&file, data).is_err() {
        return;
    }
    for origin in [Origin::Server, Origin::OfflineSave] {
        if let Ok(prepared) = prepare(&file, origin) {
            assert!(prepared.path().is_file());
        }
    }
    if read_save(&file, &unpacked).is_ok() {
        assert!(unpacked.is_file(), "a save that reads has its database");
    }
});
