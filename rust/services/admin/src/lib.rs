//! The admin API (docs/admin-api.md): GraphQL over HTTP for the host's own tools, such as a Discord
//! bot. Moderation (kick, timeout, ban and their undo) and status (a player, the server, the hunts),
//! answered from the Node backend's admin routes and the Director, and kick, ban and lookup from the
//! module of the player's game (`modules`, DF4c). It listens on loopback only and wants its key on
//! every request; the gateway never forwards to it (docs/security/admin-api.md).

pub mod app;
pub mod cli;
pub mod config;
pub mod guard;
pub mod guarded;
pub mod guarded_cli;
pub mod history;
pub mod history_text;
pub mod hmac;
pub mod input;
pub mod key_cli;
pub mod keys;
pub mod log;
pub mod module_control;
pub mod modules;
pub mod schema;
pub mod server;
pub mod timeouts;
pub mod upstream;

/// Milliseconds since 1970.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
