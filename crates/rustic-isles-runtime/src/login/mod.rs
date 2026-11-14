//! Slice R4 (spec-runtime-r4): client login, Easy Anti-Cheat and the engine command line. The detour bodies are in
//! `ffi::login`; this module picks what a role installs and serves the command line.

pub mod cmdline;

use crate::ffi;
use crate::ffi::exit::Install;
use rustic_isles_runtime_core::registry;

/// R4's installs for the role, in the surface's order (R4 7.3): a server's CL-1 goes before SP-2 and SP-3, a
/// client's ids after SP-1. Each owned id gives one status result.
pub fn install(server: bool, base: usize) -> Vec<Install> {
    let ids: &[&'static str] = if server {
        &["CL-1"]
    } else {
        &["CL-2", "CL-1", "CL-8", "CL-9", "CL-7", "CL-5"]
    };
    ids.iter()
        .filter(|id| registry::owns(id.as_bytes()))
        .map(|&id| (id, ffi::login::install(id, base)))
        .collect()
}
