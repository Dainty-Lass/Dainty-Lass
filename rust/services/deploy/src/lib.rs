//! Dauntless Revived's deploy server (`UndauntedDeployServer`, DR `rust/services/deploy` at `24b7624`),
//! for legacy (1.4.4) servers: starts, tracks and stops the game-server processes of one machine for
//! the metagame on the same machine. It has no authentication, so it listens on loopback only and
//! refuses anything a proxy relayed (`docs/security/deploy.md`).

pub mod app;
pub mod capacity;
pub mod cities;
pub mod cpu;
pub mod gameservers;
pub mod kit;
pub mod lifetime;
pub mod matchmaker;
pub mod occupancy;
pub mod process;
pub mod readiness;
pub mod remote;
pub mod service;
pub mod settings;
pub mod system;
pub mod tables;
pub mod trials;

/// Milliseconds since the epoch (`Date.now()`).
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
