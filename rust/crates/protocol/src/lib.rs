//! Wire types shared by the launcher and the servers, and the checks both sides must answer alike.
//!
//! Each type here has one definition that both sides compile against, so a changed field breaks the
//! build on both sides instead of failing at runtime. Their JSON form is the one the TypeScript code
//! reads and writes today (`legacy/ReforgedLauncher/src/shared`, `legacy/ReforgedContent`). A game is
//! named by its exact version (`module_api::GameVersion`, DD4); Dauntless Revived's 1.4.4 servers are
//! read only with the `legacy` feature ([`invite::played_versions`]).

pub use module_api::GameVersion;

pub mod account;
pub mod invite;
pub mod manifest;
pub mod status;
pub mod text;
