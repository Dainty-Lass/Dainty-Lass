//! The design `version_data.json` the host installs (`modules/rustic-isles/version-data/1.14.7`), kept equal to the
//! design part of the document built from restricted's sources (RD27, port plan item 9, rulings V-1 to V-3). Regenerate
//! with `RR_UPDATE_GENERATED=1 cargo test -p rustic-isles-mit-parity --test version_data`.
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
        serde_json::to_string_pretty(&common::version_data::design()).unwrap()
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

/// The split keeps the module's loaded view: dainty-lass's facts and the design make today's whole document, but for
/// the storage token seeds, which V-3 replaces with the MIT crates' own ids.
#[test]
fn the_facts_and_the_design_make_the_whole_document() {
    use common::version_data::{design, document, facts};
    let mut whole = document();
    whole["storageTokenSeeds"] = facts()["storageTokenSeeds"].clone();
    let merged = rustic_isles_1_14_7::VersionData::from_parts(Some(&facts()), &design()).unwrap();
    assert_eq!(merged, rustic_isles_1_14_7::VersionData::from_json(&whole).unwrap());
    // Member for member too (a map compares regardless of order), so a field the struct drops cannot hide a difference
    let mut merged_doc = rustic_isles_1_14_7::merge(&facts(), &design()).unwrap();
    for doc in [&mut whole, &mut merged_doc] {
        doc.as_object_mut().unwrap().remove("storageTokenSeeds");
    }
    assert_eq!(merged_doc, whole);
}

#[test]
fn the_shipped_document_has_every_member() {
    let data = common::version_data::restricted();
    assert!(data.legacy_starter().is_some());
    assert!(data.generator_inputs().is_some());
}
