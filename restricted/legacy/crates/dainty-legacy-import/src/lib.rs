//! `legacy-import`: old installs carried into Dainty Lass, which serves Dauntless 1.14.7 on rustic-isles.
//!
//! - [`revived`]: a Dauntless Revived 1.4.4 server's metagame database, or a player's 1.4.4 offline save, brought to
//!   the final 1.4.4 schema on a private copy, then its accounts, bans, invites and Discord links written to the
//!   1.14.7 server's account storage ([`target`]). Characters and inventory don't map across game versions and are
//!   counted, not carried.
//! - [`reforged`]: a Dauntless Reforged Revival Node-kit server (`host.json`, `secrets.env`, the old `.workspace`
//!   layout and its MongoDB) upgraded in place.
//!
//! Every import says what it did in a [`Report`], and a dry run writes nothing.

pub mod cli;
pub mod error;
pub mod reforged;
pub mod report;
pub mod revived;
pub mod target;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use error::{ImportError, Result};
pub use report::Report;
pub use revived::Options;
pub use target::Target;
