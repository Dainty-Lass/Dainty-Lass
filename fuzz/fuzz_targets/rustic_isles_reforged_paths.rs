//! Every path the Reforged API builds from a request (spec-rustic-isles-reforged-public §6, §7): arbitrary segments
//! and file names through the installer path, the log names and the account-log removal against a temporary root.
//! A built path never leaves its root: every component is a plain name.
#![no_main]

use std::path::Component;

use libfuzzer_sys::fuzz_target;
use rustic_isles_reforged::logs::{is_account_folder, is_launcher_file_name, is_session_id, remove_account_logs};
use rustic_isles_reforged::updates::launcher_installer_path;
use rustic_isles_reforged::util::{basename, is_segment};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let parts: Vec<&str> = text.split('\n').collect();
    let get = |i: usize| parts.get(i).copied().unwrap_or("");
    if let Some(path) = launcher_installer_path(get(0), get(1), get(2), get(3)) {
        assert!(path.components().all(|c| matches!(c, Component::Normal(_))));
    }
    for name in [get(0), basename(get(1))] {
        if is_segment(name) || is_session_id(name) || is_account_folder(name) || is_launcher_file_name(name) {
            assert!(!name.contains(['/', '\\']) && name != "." && name != "..");
        }
    }
    let root = std::env::temp_dir().join("rustic_isles_reforged_paths_fuzz");
    let _ = std::fs::create_dir_all(&root);
    if remove_account_logs(&root, get(0)).is_ok() {
        assert!(is_account_folder(get(0)));
    }
    assert!(root.exists());
});
