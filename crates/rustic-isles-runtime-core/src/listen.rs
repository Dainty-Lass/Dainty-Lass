//! Slice R9's pure half (spec-runtime-r9): who listens, when the listen is asked for, the engine string and `FURL`
//! plans, the driver lookups' array checks, the ready lines and the log texts.

use crate::config::auth::AuthMode;
use crate::config::launch::Launch;
use crate::engine::sane_pointer_array;
use crate::memory::Memory;
use crate::offsets::r9;

/// The variable a DeployServer's launch id comes in.
pub const LAUNCH_ID_VARIABLE: &str = "RUSTIC_ISLES_GAMESERVER_LAUNCH_ID";
/// The longest launch id read (R0's `env_value` cap).
pub const LAUNCH_ID_MAX: usize = 127;
/// The listen is asked for this long after the handler's first call even without a world (5.1).
pub const WORLD_WAIT_MS: u64 = 30_000;
/// The most world contexts walked (R9 call 7).
pub const MAX_WORLD_CONTEXTS: i32 = 64;
/// The most named drivers on a context.
pub const MAX_NAMED_DRIVERS: i32 = 128;
/// The most client connections read.
pub const MAX_CLIENT_CONNECTIONS: i32 = 1024;
/// The most open channels read on a connection.
pub const MAX_OPEN_CHANNELS: i32 = 4096;
/// The most units of `InitListen`'s error text logged.
pub const MAX_ERROR_UNITS: usize = 512;
/// The engine name R9 makes and binds.
pub const GAME_NET_DRIVER: &str = "GameNetDriver";

/// Who listens in this process (6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The MIT runtime, for a DeployServer and a player host alike (spec-runtime-r21 4.5).
    Mit,
}

/// `listen_decision`; `None` for `Invalid`, which ends the process before the listen.
pub fn listen_decision(auth: AuthMode) -> Option<Decision> {
    match auth {
        AuthMode::GlobalGameserver | AuthMode::PlayerHost => Some(Decision::Mit),
        AuthMode::Invalid => None,
    }
}

/// Why the listen was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// `GWorld` was set.
    WorldPresent,
    /// 30 s since the handler's first call with no world.
    Timeout,
}

/// The listen request's schedule (5.1 step 4): asked for once, on the first call with a world or 30 s after the
/// first call.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ListenSchedule {
    first_ms: Option<u64>,
    requested: bool,
}

impl ListenSchedule {
    /// A schedule not yet started.
    pub const fn new() -> Self {
        Self {
            first_ms: None,
            requested: false,
        }
    }

    /// One handler call at `now_ms`: the request on the call that makes it, `None` before and after.
    pub fn poll(&mut self, now_ms: u64, world_present: bool) -> Option<Request> {
        if self.requested {
            return None;
        }
        let first = *self.first_ms.get_or_insert(now_ms);
        let request = if world_present {
            Request::WorldPresent
        } else if now_ms.saturating_sub(first) >= WORLD_WAIT_MS {
            Request::Timeout
        } else {
            return None;
        };
        self.requested = true;
        Some(request)
    }

    /// Whether the listen was asked for.
    pub fn requested(&self) -> bool {
        self.requested
    }
}

/// What writing `count` units into the engine's `FString` takes (4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FStringPlan {
    /// Grow (or allocate) the buffer with `FMemory::Realloc` to `count` units first.
    pub realloc: bool,
    /// The new `Num`.
    pub num: i32,
    /// The new `Max`.
    pub max: i32,
}

/// `fstring_plan(num, max, data_null, count)`; `count` includes the NUL.
pub fn fstring_plan(_num: i32, max: i32, data_null: bool, count: i32) -> FStringPlan {
    FStringPlan {
        realloc: data_null || max < count,
        num: count,
        max: max.max(count),
    }
}

/// The engine's world list: 0 to 64 entries, and the array readable for them (6.2 step 1).
pub fn world_list_ok(count: i32, readable: bool) -> bool {
    (0..=MAX_WORLD_CONTEXTS).contains(&count) && readable
}

/// A context's `ActiveNetDrivers` counts (6.2 step 3).
pub fn named_drivers_ok(num: i32, max: i32) -> bool {
    num >= 0 && num <= max && max <= MAX_NAMED_DRIVERS
}

/// An `FName`'s two halves.
pub type Name = (i32, u32);

/// An `FName` from its 8 bytes.
pub fn name_from_bytes(bytes: [u8; 8]) -> Name {
    let [a, b, c, d, e, f, g, h] = bytes;
    (i32::from_le_bytes([a, b, c, d]), u32::from_le_bytes([e, f, g, h]))
}

/// An `FName` as the 8-byte value `CreateNamedNetDriver` takes in a register.
pub fn name_value(name: Name) -> u64 {
    u64::from(name.0 as u32) | (u64::from(name.1) << 32)
}

/// The `FName` at `addr`.
pub fn name_at(memory: &dyn Memory, addr: usize) -> Option<Name> {
    let mut bytes = [0u8; 8];
    memory.read(addr, &mut bytes).then(|| name_from_bytes(bytes))
}

/// The engine's world context whose `World` is `world` (6.2 step 1).
pub fn find_world_context(memory: &dyn Memory, engine: usize, world: usize) -> Option<usize> {
    if world == 0 || !memory.readable(engine, r9::ENGINE_WORLD_LIST_NUM + 8) {
        return None;
    }
    let list = memory.ptr_at(engine.checked_add(r9::ENGINE_WORLD_LIST)?)?;
    let count = memory.i32_at(engine.checked_add(r9::ENGINE_WORLD_LIST_NUM)?)?;
    let readable = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(8))
        .is_some_and(|size| size == 0 || memory.readable(list, size));
    if !world_list_ok(count, readable) {
        return None;
    }
    (0..usize::try_from(count).ok()?).find_map(|index| {
        let context = memory.ptr_at(list.checked_add(index * 8)?)?;
        (context != 0
            && memory.readable(context, r9::WORLD_CONTEXT_SIZE)
            && memory.ptr_at(context + r9::WORLD_CONTEXT_WORLD) == Some(world))
        .then_some(context)
    })
}

/// The context's `ActiveNetDrivers` entries, or `None` when the array is refused (6.2 step 3).
pub fn active_drivers(memory: &dyn Memory, context: usize) -> Option<Vec<usize>> {
    let array = context.checked_add(r9::WORLD_CONTEXT_ACTIVE_NET_DRIVERS)?;
    let data = memory.ptr_at(array)?;
    let num = memory.i32_at(array + 8)?;
    let max = memory.i32_at(array + 12)?;
    if !named_drivers_ok(num, max) {
        return None;
    }
    let num = usize::try_from(num).ok()?;
    if num > 0 && !memory.readable(data, num * r9::NAMED_NET_DRIVER_STRIDE) {
        return None;
    }
    Some(
        (0..num)
            .filter_map(|index| memory.ptr_at(data + index * r9::NAMED_NET_DRIVER_STRIDE))
            .collect(),
    )
}

/// Whether `driver` is a readable net driver named `name`; `is_net_driver` is the reflection kit's `is_a`.
pub fn driver_matches(memory: &dyn Memory, driver: usize, name: Name, is_net_driver: impl Fn(usize) -> bool) -> bool {
    driver != 0
        && memory.readable(driver, r9::NET_DRIVER_SIZE)
        && name_at(memory, driver + r9::NET_DRIVER_NAME) == Some(name)
        && is_net_driver(driver)
}

/// The zeroed `FURL` R9 passes to `InitListen`: `Port` set, and `Host` only when pinned (`data`, units with the NUL).
pub fn furl_block(port: u16, host: Option<(usize, i32)>) -> [u8; r9::FURL_SIZE] {
    let mut block = [0u8; r9::FURL_SIZE];
    block[r9::FURL_PORT..r9::FURL_PORT + 4].copy_from_slice(&i32::from(port).to_le_bytes());
    if let Some((data, count)) = host {
        block[r9::FURL_HOST..r9::FURL_HOST + 8].copy_from_slice(&(data as u64).to_le_bytes());
        block[r9::FURL_HOST + 8..r9::FURL_HOST + 12].copy_from_slice(&count.to_le_bytes());
        block[r9::FURL_HOST + 12..r9::FURL_HOST + 16].copy_from_slice(&count.to_le_bytes());
    }
    block
}

/// A player host's pin against the engine's `MULTIHOME` (6.3 step 3): `Ok` with the match line, `Err` with the
/// failure line.
pub fn pin_check(host: &str, multihome: Option<&str>) -> Result<String, String> {
    match multihome {
        Some(engine) if engine.eq_ignore_ascii_case(host) => Ok(format!(
            "[MitRuntime] [Listen] engine MULTIHOME matches the pinned listen address {host}"
        )),
        _ => Err(format!(
            "[MitRuntime] [Listen] failed: pinned listen address {host} is not the engine's MULTIHOME ({})",
            multihome.unwrap_or("none")
        )),
    }
}

/// What 6.3 step 7 found on the context's world.
pub fn world_driver_note(current: usize, driver: usize) -> &'static str {
    if current == 0 {
        "was null"
    } else if current == driver {
        "already ours"
    } else {
        "replaced another driver"
    }
}

/// The DeployServer ready line (7.1); RT-8 dropped the old marker (spec-runtime-r10 12.1).
pub fn ready_lines(launch_id: &str, port: u16) -> [String; 1] {
    [format!(
        "RUSTIC_ISLES_GAMESERVER_READY launchId={launch_id} port={port}\r\n"
    )]
}

/// `client_connections(driver)`: the non-null entries of a sane array of at most 1024 (8.3).
pub fn client_connections(memory: &dyn Memory, driver: usize) -> Vec<usize> {
    pointer_array(
        memory,
        driver,
        r9::NET_DRIVER_CLIENT_CONNECTIONS,
        MAX_CLIENT_CONNECTIONS,
    )
    .into_iter()
    .filter(|&connection| connection != 0)
    .collect()
}

fn pointer_array(memory: &dyn Memory, owner: usize, offset: usize, limit: i32) -> Vec<usize> {
    let Some(array) = owner.checked_add(offset) else {
        return Vec::new();
    };
    let (Some(data), Some(num), Some(max)) = (
        memory.ptr_at(array),
        memory.i32_at(array + 8),
        memory.i32_at(array + 12),
    ) else {
        return Vec::new();
    };
    if !sane_pointer_array(memory, data, num, max, limit) {
        return Vec::new();
    }
    (0..usize::try_from(num).unwrap_or(0))
        .map(|index| memory.ptr_at(data + index * 8).unwrap_or(0))
        .collect()
}

/// `actor_channel(connection, actor)`: the first open channel whose class is exactly `actor_channel_class` and whose
/// actor is `actor` (8.3).
pub fn actor_channel(
    memory: &dyn Memory,
    connection: usize,
    actor: usize,
    actor_channel_class: usize,
) -> Option<usize> {
    if actor == 0 || actor_channel_class == 0 || !memory.readable(connection, r9::CONNECTION_SIZE) {
        return None;
    }
    pointer_array(memory, connection, r9::CONNECTION_OPEN_CHANNELS, MAX_OPEN_CHANNELS)
        .into_iter()
        .find(|&channel| {
            channel != 0
                && channel.is_multiple_of(8)
                && memory.readable(channel, r9::CHANNEL_SIZE)
                && memory.ptr_at(channel + crate::offsets::layout::UOBJECT_CLASS) == Some(actor_channel_class)
                && memory.ptr_at(channel + r9::CHANNEL_ACTOR) == Some(actor)
        })
}

/// `channel_presence(actor, connection)`: -2 for a null actor or an unreadable connection, 1 when a channel is
/// open for it, else -1.
pub fn channel_presence(memory: &dyn Memory, actor: usize, connection: usize, actor_channel_class: usize) -> i32 {
    if actor == 0 || !memory.readable(connection, r9::CONNECTION_SIZE) {
        return -2;
    }
    if actor_channel(memory, connection, actor, actor_channel_class).is_some() {
        1
    } else {
        -1
    }
}

/// The first SRV-1 call's line: the map path and the URL's length only, never its hunt ids or roster (section 11).
pub fn map_url_line(launch: &Launch, url_units: usize, hub_cap: u32) -> String {
    format!(
        "[MitRuntime] [MapUrl] map={} length={url_units} maxPlayers={hub_cap}",
        String::from_utf16_lossy(&launch.map_path)
    )
}

/// The section 11 lines that carry no value.
pub mod lines {
    /// SRV-1's allocation failed.
    pub const ALLOCATION_FAILED: &str = "[MitRuntime] [MapUrl] engine allocation failed; the engine keeps its own map";
    /// An unknown auth mode, before the process ends with `0xE102`.
    pub const INVALID_AUTH: &str =
        "[MitRuntime] [ServerAuth] unknown RUSTIC_ISLES_AUTH_MODE; terminating before listen";
    /// The request with a world.
    pub const REQUESTED_WORLD: &str = "[MitRuntime] [Listen] listen requested (world present)";
    /// The request without one.
    pub const REQUESTED_TIMEOUT: &str = "[MitRuntime] [Listen] listen requested (no world after 30 s; trying anyway)";
    /// The context's world could not be read.
    pub const WORLD_UNREADABLE: &str =
        "[MitRuntime] [Listen] WARNING: the world context's world is unreadable; the engine will not drive this driver";
    /// The driver's world was moved to the context's.
    pub const REALIGNED: &str = "[MitRuntime] [Listen] driver World realigned to the world context's world";
    /// No launch id.
    pub const NO_LAUNCH_ID: &str = "[MitRuntime] [Listen] no RUSTIC_ISLES_GAMESERVER_LAUNCH_ID: no ready marker";
    /// Standard output could not take the lines.
    pub const STDOUT_UNAVAILABLE: &str = "[MitRuntime] [Listen] stdout unavailable: no ready marker";
}

/// `CreateNamedNetDriver returned <true|false>`.
pub fn created_line(created: bool) -> String {
    format!("[MitRuntime] [Listen] CreateNamedNetDriver returned {created}")
}

/// `GameNetDriver 0x<p> (<active drivers|object scan>)`.
pub fn found_line(driver: usize, scanned: bool) -> String {
    let how = if scanned { "object scan" } else { "active drivers" };
    format!("[MitRuntime] [Listen] GameNetDriver {driver:#x} ({how})")
}

/// `InitListen error: <text>`.
pub fn init_listen_error_line(text: &str) -> String {
    format!("[MitRuntime] [Listen] InitListen error: {text}")
}

/// `UWorld::NetDriver on the world context's world: <note>`.
pub fn world_driver_line(note: &str) -> String {
    format!("[MitRuntime] [Listen] UWorld::NetDriver on the world context's world: {note}")
}

/// `ready marker written launchId=<id>`.
pub fn ready_written_line(launch_id: &str) -> String {
    format!("[MitRuntime] [Listen] ready marker written launchId={launch_id}")
}

/// `listening on port <n>, driver 0x<p>`.
pub fn listening_line(port: u16, driver: usize) -> String {
    format!("[MitRuntime] [Listen] listening on port {port}, driver {driver:#x}")
}

/// `failed: <reason>`.
pub fn failed_line(reason: &str) -> String {
    format!("[MitRuntime] [Listen] failed: {reason}")
}

/// Why a frame's driver maintenance was skipped (7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Not in the object array any more.
    NotLive,
    /// Not readable for the driver's span.
    Unreadable,
    /// `GWorld` is null.
    NoWorld,
}

/// `driver maintenance skipped: <reason>`.
pub fn skipped_line(skip: Skip) -> String {
    let reason = match skip {
        Skip::NotLive => "driver no longer live",
        Skip::Unreadable => "driver unreadable",
        Skip::NoWorld => "no world",
    };
    format!("[MitRuntime] [Listen] driver maintenance skipped: {reason}")
}

#[cfg(test)]
mod tests;
