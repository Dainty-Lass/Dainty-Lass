//! Arbitrary bytes as a 1.4.4 server database, a bare offline database and a `.drsave`, through the legacy
//! importer's copy, checks and reads (spec-legacy-import §5 to §7, §12): an error or a count, never a panic, and
//! nothing written beside the source.
#![no_main]

use dainty_legacy::{drsave, read_offline_save, read_server_database};
use libfuzzer_sys::fuzz_target;

const KEY: &str = "UUK_0123456789abcdef0123456789abcdef0123456789abcdef";

fuzz_target!(|data: &[u8]| {
    let Ok(dir) = tempfile::tempdir() else {
        return;
    };
    let raw = dir.path().join("source.db");
    let save = dir.path().join("player.drsave");
    if std::fs::write(&raw, data).is_err()
        || std::fs::write(
            &save,
            drsave::build(data, "UID-1", "Sam", KEY, "2026-10-21T22:00:00.000Z"),
        )
        .is_err()
    {
        return;
    }
    let _ = read_offline_save(&raw);
    let _ = read_offline_save(&save);
    let _ = read_server_database(&raw);
    // The readers work on private copies. Beside the source there may be only what it was given, and the -wal
    // and -shm any SQLite reader of a WAL database makes
    let allowed = ["source.db", "player.drsave", "source.db-wal", "source.db-shm"];
    if let Ok(entries) = std::fs::read_dir(dir.path()) {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            assert!(allowed.iter().any(|a| name == *a), "{name:?}");
        }
    }
    assert_eq!(std::fs::read(&raw).ok().as_deref(), Some(data));
});
