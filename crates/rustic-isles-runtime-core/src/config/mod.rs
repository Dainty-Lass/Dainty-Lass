//! R1: every value the runtime takes from its environment, command line and number files, parsed into typed
//! values the other slices read (spec-runtime-r1). Pure; the DLL's readers feed it.

pub mod auth;
pub mod backend;
pub mod hub;
pub mod launch;
pub mod numbers;
pub mod pacing;
pub mod relay;
pub mod roster;
pub mod trials;
