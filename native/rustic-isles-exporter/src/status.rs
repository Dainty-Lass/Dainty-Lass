//! `catalog_export_status.txt`: appended across runs, never truncated (spec §6.17). Its bytes need not match the old
//! one's (spec §12 call 3); the lines a host reads keep their text.

use std::io::Write as _;
use std::path::{Path, PathBuf};

pub const FILE: &str = "catalog_export_status.txt";

pub struct Status {
    file: PathBuf,
    debug: fn(&str),
}

impl Status {
    pub fn new(out: &Path, debug: fn(&str)) -> Self {
        Self {
            file: out.join(FILE),
            debug,
        }
    }

    /// One line, `prefix` naming the exporter (`[Hunts] `), also sent to the debugger.
    pub fn line(&self, prefix: &str, text: &str) {
        let line = format!("{prefix}{text}");
        (self.debug)(&format!("[CatalogExporter] {line}\n"));
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&self.file) {
            let _ = file.write_all(&crate::json::crlf(format!("{line}\n").as_bytes()));
        }
    }
}

pub fn no_debug(_: &str) {}
