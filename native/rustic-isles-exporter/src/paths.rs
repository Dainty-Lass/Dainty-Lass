//! Where the flags are read and the output written (spec §3.3).

use std::path::{Path, PathBuf};

/// The compile-time export root (spec §12 call 6); a host's build never sets it.
pub const EXPORT_ROOT: Option<&str> = option_env!("RUSTIC_ISLES_EXPORT_ROOT");

pub const FLAGS_FILE: &str = "export_flags.txt";
pub const OUTPUT_FOLDER: &str = "Items_Analysis";

/// The flags file candidates, in order: `<root>\native\catalog-exporter\export_flags.txt`, then the working folder's.
pub fn flag_candidates(root: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = root {
        candidates.push(root.join("native").join("catalog-exporter").join(FLAGS_FILE));
    }
    candidates.push(PathBuf::from(".").join(FLAGS_FILE));
    candidates
}

/// The first candidate that is a regular file.
pub fn find_flags(root: Option<&Path>) -> Option<PathBuf> {
    flag_candidates(root).into_iter().find(|path| path.is_file())
}

/// The first output folder that exists; when none does, the first is created (not recursively) and used.
pub fn output_dir(root: Option<&Path>) -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(root) = root {
        candidates.push(root.join(OUTPUT_FOLDER));
    }
    candidates.push(PathBuf::from(".").join(OUTPUT_FOLDER));
    if let Some(found) = candidates.iter().find(|path| path.is_dir()) {
        return found.clone();
    }
    let first = candidates.swap_remove(0);
    let _ = std::fs::create_dir(&first);
    first
}

/// A subfolder of the output, created when missing (not recursively).
pub fn subfolder(out: &Path, name: &str) -> PathBuf {
    let path = out.join(name);
    let _ = std::fs::create_dir(&path);
    path
}

pub fn root() -> Option<&'static Path> {
    EXPORT_ROOT.filter(|root| !root.is_empty()).map(Path::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_root_first() {
        let root = Path::new("C:\\dl-diff\\new");
        assert_eq!(
            flag_candidates(Some(root)),
            [
                root.join("native").join("catalog-exporter").join("export_flags.txt"),
                PathBuf::from(".").join("export_flags.txt")
            ]
        );
        assert_eq!(flag_candidates(None), [PathBuf::from(".").join("export_flags.txt")]);
    }

    #[test]
    fn makes_the_first_output_folder_only_when_none_exists() {
        let dir = crate::testdir::new("paths");
        let root = dir.join("root");
        std::fs::create_dir(&root).unwrap();
        assert_eq!(output_dir(Some(&root)), root.join(OUTPUT_FOLDER));
        assert!(root.join(OUTPUT_FOLDER).is_dir());
        let missing = dir.join("missing").join("deeper");
        let fallback = output_dir(Some(&missing));
        assert_eq!(
            fallback,
            missing.join(OUTPUT_FOLDER),
            "not created recursively, still the first candidate"
        );
    }
}
