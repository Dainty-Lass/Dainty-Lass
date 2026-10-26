//! Installing Dauntless from the player's own Epic library (docs/dainty-lass-plan.md DD22), for the launcher and
//! the host kit. DE1 lays it down: `manifest` and `chunk` read Epic's formats; `auth`, `library` and `download`
//! are typed stubs until DE2, DE3 and DE5. `docs/port/spike-de1-epic.md` says why the formats are read here and
//! not by `egs-api`, and `docs/security/epic.md` is the checklist.
//!
//! Nothing here logs. Tokens have no `Debug` or `Display` (`auth::Token`), and a manifest is parsed only once its
//! SHA-1 matches the pin (`manifest::Manifest::parse`).

pub mod auth;
pub mod chunk;
pub mod download;
pub mod library;
pub mod manifest;

/// The row that will fill in a stub, so a caller can say what is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not implemented yet ({row})")]
pub struct NotImplemented {
    pub row: &'static str,
}
