//! `gen/strings.json`: the page's language catalogs in `ui/packages/i18n/catalogs` (DU2), one table per language,
//! `en` first and each sorted by key. They stay AGPL (spec §3.1, L1), so the file is made here; the launcher's
//! own strings are launcher-kit's MIT catalog. The test fails when the file is stale;
//! `RR_UPDATE_GENERATED=1 cargo test -p launcher generated` rewrites it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test-only: a panic is how a test fails"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn catalogs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/packages/i18n/catalogs")
}

fn strings_export() -> String {
    let dir = catalogs();
    let mut languages: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let code = path.file_name()?.to_str()?.strip_suffix(".json")?.to_owned();
            Some((code, path))
        })
        .collect();
    languages.sort_by(|a, b| (a.0 != "en", &a.0).cmp(&(b.0 != "en", &b.0)));
    let mut out = serde_json::Map::new();
    for (code, path) in languages {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let table: BTreeMap<String, String> =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let table = table.into_iter().map(|(key, text)| (key, text.into())).collect();
        out.insert(code, serde_json::Value::Object(table));
    }
    serde_json::to_string_pretty(&out).unwrap_or_default() + "\n"
}

#[test]
fn generated_strings_are_current() {
    // Refused rather than skipped
    assert!(
        catalogs().join("en.json").is_file(),
        "{} has no language catalogs",
        catalogs().display()
    );
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("gen/strings.json");
    let content = strings_export();
    let current = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    if current == content {
        return;
    }
    if std::env::var("RR_UPDATE_GENERATED").is_ok_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        return;
    }
    panic!(
        "stale {} (run RR_UPDATE_GENERATED=1 cargo test -p launcher generated)",
        path.display()
    );
}

/// The backend's 14 keys are launcher-kit's own text now (L1); the page's catalogs still name each of them, so
/// the page and the backend agree on what a key is.
#[test]
fn generated_strings_hold_the_backends_keys() {
    let strings: serde_json::Value = serde_json::from_str(&strings_export()).unwrap();
    for language in launcher_kit::core::i18n::Language::ALL {
        let table = &strings[language.code()];
        assert!(table.is_object(), "no {} table", language.code());
        for key in launcher_kit::core::i18n::KEYS {
            assert!(table.get(key).is_some(), "{} lacks {key}", language.code());
        }
    }
}
