//! What a game server writes and the runtime's occupancy log (spec-rustic-isles-director §5.6, §7.4, §15): any bytes.
//! The readiness scanner, the edge parser and the occupancy follower never panic, keep their buffers under the caps,
//! and accept only a well-formed marker for the exact launch id and port.
#![no_main]

use std::io::Write as _;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use rustic_isles_director::occupancy::{RuntimeLogOccupancy, parse_edge};
use rustic_isles_director::process::{ReadinessScanner, parse_marker};

const ID: &str = "0e1a7c2b-3f4d-4b5a-9c8d-112233445566";

fn log_dir() -> &'static std::path::Path {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("rustic-isles-director-fuzz-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}

fuzz_target!(|data: &[u8]| {
    let mut scanner = ReadinessScanner::new(ID, 8780);
    let mut ready = false;
    for chunk in data.chunks(97) {
        ready |= scanner.feed(chunk);
        assert!(scanner.buffered() <= 65_536 + 97);
    }
    if ready {
        let text = String::from_utf8_lossy(data);
        let exact = text
            .split('\n')
            .any(|line| parse_marker(line) == Some((ID, "8780")));
        assert!(exact, "accepted without this launch's marker");
    }
    let text: String = data.iter().map(|&b| char::from(b)).collect();
    for line in text.split('\n') {
        if let Some(Some(count)) = parse_edge(line) {
            assert!(count >= 0);
        }
    }
    let path = log_dir().join("rustic_isles_dll_port8776.log");
    let _ = std::fs::write(&path, b"");
    let mut follower = RuntimeLogOccupancy::new(&path);
    follower.begin();
    for chunk in data.chunks(301) {
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&path) {
            let _ = file.write_all(chunk);
        }
        let _ = follower.poll();
        assert!(follower.partial_len() <= 2 * 65_536);
    }
});
