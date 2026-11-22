//! The catalog exporter (`docs/port/spec-catalog-exporter.md`, phase 1): waits for the live catalog and writes the
//! catalog and the tables the Rustic Isles generators read. `dll/` loads it into the game as `CatalogExporter.dll`.
//!
//! Plain code that reads memory through `memory::Memory`, so the output layer and the readers are tested on any OS
//! against a synthetic image; the process itself is `dll/src/sys.rs`.
#![forbid(unsafe_code)]

pub mod clock;
pub mod engine;
pub mod export;
pub mod facts;
pub mod flags;
pub mod json;
pub mod memory;
pub mod paths;
pub mod run;
pub mod sdk_parse;
pub mod status;

/// The layout facts `build.rs` took from the SDK (`facts.rs` names them).
pub mod sdk {
    use crate::sdk_parse::Kind::*;
    use crate::sdk_parse::Member;
    include!(concat!(env!("OUT_DIR"), "/sdk_layout.rs"));
}

#[cfg(test)]
mod testdir;
