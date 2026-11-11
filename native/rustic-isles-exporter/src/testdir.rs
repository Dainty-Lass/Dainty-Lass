//! A fresh folder per test under the system temp folder.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn new(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "catalog-exporter-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_default();
    dir
}
