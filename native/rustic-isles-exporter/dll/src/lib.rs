//! `CatalogExporter.dll` (spec-catalog-exporter §9): `DllMain` starts the worker, which runs
//! `catalog_exporter::run` on the game's own memory.
#![cfg(windows)]
#![allow(
    non_snake_case,
    reason = "the DLL's name, CatalogExporter.dll, is the launcher's and the host kit's contract"
)]
#![allow(
    unsafe_code,
    reason = "DllMain, kernel32 and the ProcessEvent call; reviewed as a trust boundary"
)]

mod sys;
