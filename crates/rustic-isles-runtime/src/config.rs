//! R1's readers: the core crate's parsers fed from this process's environment, command line and number files.
//! Init resolves the backend address, the role and the launch once; later slices read the rest lazily,
//! each value at most once per process.

use crate::ffi::{flags, win};
use rustic_isles_runtime_core::config::auth::{self, AuthMode};
use rustic_isles_runtime_core::config::backend::{self, Backend, BackendUrl};
use rustic_isles_runtime_core::config::hub::{self, AfkSetting, HubCap, Sources};
use rustic_isles_runtime_core::config::launch::{Launch, PortError, parse_launch};
use rustic_isles_runtime_core::config::relay::{self, Refusal, RelayTarget};
use rustic_isles_runtime_core::config::{pacing, trials};
use rustic_isles_runtime_core::{wide, wide_contains};
use std::sync::OnceLock;

/// The process's role and launch, fixed in Init.
#[derive(Debug, Clone)]
pub struct Role {
    /// `-server` on the command line.
    pub server: bool,
    /// The server's auth mode (`GlobalGameserver` on a client, where nothing is parsed).
    pub auth: AuthMode,
    /// The server's launch; `None` on a client.
    pub launch: Option<Launch>,
}

/// A frame cap and, when its default was forced, the note for R8's line.
pub type FpsSetting = (i32, Option<String>);

static BACKEND: OnceLock<Backend> = OnceLock::new();
static ROLE: OnceLock<Role> = OnceLock::new();

/// R1 4: resolves the backend address once.
pub fn resolve_backend() -> &'static Backend {
    BACKEND.get_or_init(|| backend::resolve(|name| win::env_raw(name, backend::READ_CAP)))
}

/// The backend address; the built-in one if Init has not resolved it.
pub fn backend_url() -> BackendUrl {
    BACKEND
        .get()
        .map_or_else(BackendUrl::built_in, |backend| backend.url.clone())
}

/// The in-game relay's target, when the launcher named a server; read once, by Init on a client.
pub fn relay_target() -> Option<Result<RelayTarget, Refusal>> {
    let read = |name| win::env_raw(name, relay::READ_CAP);
    relay::resolve(
        &backend_url(),
        &read(relay::HOST_VAR),
        &read(relay::PORT_VAR),
        &read(relay::PIN_VAR),
    )
}

/// Init's role step: `-server` anywhere on the command line (case-sensitive), then for a server the auth mode and
/// the launch. `Err` is a full launch with a refused port.
pub fn resolve_role() -> Result<&'static Role, PortError> {
    if let Some(role) = ROLE.get() {
        return Ok(role);
    }
    let line = win::command_line();
    let server = wide_contains(&line, &wide("-server"));
    let role = if server {
        let auth = auth::auth_mode(&win::env_raw(auth::VARIABLE, auth::READ_CAP));
        let launch = parse_launch(&win::argv(), &line, auth)?;
        Role {
            server,
            auth,
            launch: Some(launch),
        }
    } else {
        Role {
            server,
            auth: AuthMode::GlobalGameserver,
            launch: None,
        }
    };
    Ok(ROLE.get_or_init(|| role))
}

/// True on a server (after Init).
pub fn is_server() -> bool {
    ROLE.get().is_some_and(|role| role.server)
}

/// The server's launch.
pub fn launch() -> Option<&'static Launch> {
    ROLE.get().and_then(|role| role.launch.as_ref())
}

/// The auth mode.
pub fn auth_mode() -> AuthMode {
    ROLE.get().map_or(AuthMode::GlobalGameserver, |role| role.auth)
}

/// The log's port: the launch's, 0 on a client.
pub fn port() -> u16 {
    launch().map_or(0, |launch| launch.port)
}

/// `launcher` is R10's hub override: `Some` only for a hub with a valid bootstrap (spec-runtime-r10 11).
fn sources(launcher: Option<i64>, env: &str, file: &str) -> Sources {
    Sources {
        launcher,
        env: win::env_value(env, hub::ENV_MAX),
        file: flags::read_number_file(file, hub::FILE_MAX_BYTES),
    }
}

/// The hub cap (R1 10), read once.
pub fn hub_cap() -> HubCap {
    static CAP: OnceLock<HubCap> = OnceLock::new();
    *CAP.get_or_init(|| {
        hub::hub_cap(&sources(
            crate::player_host::hosted_role().and_then(|role| role.max_players.map(i64::from)),
            hub::ENV_HUB_MAX_PLAYERS,
            hub::FILE_HUB_MAX_PLAYERS,
        ))
    })
}

/// The hub AFK timeout, read once.
pub fn hub_afk() -> AfkSetting {
    static AFK: OnceLock<AfkSetting> = OnceLock::new();
    *AFK.get_or_init(|| {
        hub::hub_afk(&sources(
            crate::player_host::hosted_role().and_then(|role| role.hub_afk_seconds.map(i64::from)),
            hub::ENV_HUB_AFK,
            hub::FILE_HUB_AFK,
        ))
    })
}

/// The hunt AFK timeout, read once.
pub fn hunt_afk() -> AfkSetting {
    static AFK: OnceLock<AfkSetting> = OnceLock::new();
    *AFK.get_or_init(|| hub::hunt_afk(&sources(None, hub::ENV_HUNT_AFK, hub::FILE_HUNT_AFK)))
}

/// The active and idle frame caps with their notes (R1 8), read once.
pub fn frame_pacing() -> &'static (FpsSetting, FpsSetting) {
    static PACING: OnceLock<(FpsSetting, FpsSetting)> = OnceLock::new();
    PACING.get_or_init(|| {
        (
            pacing::max_fps(&win::env_raw(pacing::MAX_FPS, pacing::READ_CAP)),
            pacing::idle_fps(&win::env_raw(pacing::IDLE_FPS, pacing::READ_CAP)),
        )
    })
}

/// The trials rotation in minutes (R1 9), read once.
pub fn trials_rotation_minutes() -> i64 {
    static MINUTES: OnceLock<i64> = OnceLock::new();
    *MINUTES.get_or_init(|| trials::rotation_minutes(&win::env_raw(trials::ROTATION_VARIABLE, trials::ROTATION_CAP)))
}

/// The pinned trials week.
pub fn trials_week() -> Option<u32> {
    static WEEK: OnceLock<Option<u32>> = OnceLock::new();
    *WEEK.get_or_init(|| trials::trials_week(&win::env_raw(trials::WEEK_VARIABLE, trials::WEEK_CAP)))
}
