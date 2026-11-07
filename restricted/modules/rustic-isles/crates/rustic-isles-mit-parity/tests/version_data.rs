//! The `version_data.json` the host compiles in and installs (`modules/rustic-isles/version-data/1.14.7`), kept equal
//! to the document built from restricted's sources (RD27, spec-rustic-isles-1-14-7 §4.3). Regenerate with
//! `RR_UPDATE_GENERATED=1 cargo test -p rustic-isles-mit-parity --test version_data`.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

mod common;

use std::path::PathBuf;

fn shipped() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../version-data/1.14.7/version_data.json")
}

#[test]
fn generated_version_data_is_current() {
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&common::version_data::document()).unwrap()
    );
    let path = shipped();
    if std::env::var_os("RR_UPDATE_GENERATED").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    assert!(
        on_disk == text,
        "{} is stale: rerun with RR_UPDATE_GENERATED=1",
        path.display()
    );
}

#[test]
fn the_shipped_document_has_every_member() {
    let data = common::version_data::restricted();
    assert!(data.legacy_starter().is_some());
    assert!(data.generator_inputs().is_some());
}
