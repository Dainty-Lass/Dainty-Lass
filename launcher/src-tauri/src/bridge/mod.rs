//! The bridge between the page and the backend (rows L6 to L9): the types that cross it, the fixed list
//! of commands, and the commands themselves.

pub mod backend;
pub mod commands;
#[cfg_attr(
    not(feature = "server-gui"),
    allow(
        dead_code,
        reason = "the server GUI's module panels use it; its checks are tested in every build"
    )
)]
pub mod modules;
pub mod progress;
pub mod spec;
pub mod types;

#[cfg(test)]
mod generate;
