//! The server controller: named modules run in one process, in order, with one status registry
//! (`docs/app-structure-plan.md`; the decision row is RD26).
//!
//! The controller knows no service. The host kit composes the modules (`host::serve`): the Rust
//! services this binary was built with, through the constructors in [`modules`], and the programs
//! it supervises (MongoDB, Mystic Paradox's backend and Director until the RB rows) as modules of
//! their own. A build with no module features is a controller that starts, says so, and serves
//! nothing.

pub mod controller;
pub mod modules;

pub use controller::{Module, ModuleFuture, ModuleStart, START_TIMEOUT, STOP_GRACE, ServerController};
