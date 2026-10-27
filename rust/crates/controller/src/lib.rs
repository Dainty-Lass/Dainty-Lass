//! The server controller: named modules run in one process, in order, with one status registry
//! (`docs/app-structure-plan.md`; the decision row is RD26).
//!
//! The controller knows no service. The host kit composes the modules (`host::serve`): the Rust
//! services this binary was built with, through the constructors in [`modules`], and the programs
//! it supervises (MongoDB, Mystic Paradox's backend and Director until the RB rows) as modules of
//! their own. A build with no module features is a controller that starts, says so, and serves
//! nothing.
//!
//! The game modules and plugins (DD3-DD5) are the [`registry`]'s: keyed by game version, compiled in or
//! started as their own process from what `host.toml`'s `[modules]` lists, loaded after the services and
//! unloaded before them, with their ports placed by [`ports`].

pub mod controller;
pub mod modules;
pub mod ports;
pub mod registry;

pub use controller::{Module, ModuleFuture, ModuleStart, START_TIMEOUT, STOP_GRACE, ServerController};
pub use registry::{
    ModuleEntry, ModuleFactory, ModuleInfo, ModuleRegistry, ModuleSource, ModuleState, RegistryError, RegistryOptions,
    WeakRegistry,
};
