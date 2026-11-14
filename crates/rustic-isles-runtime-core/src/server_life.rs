//! Slice R8's pure half (spec-runtime-r8): the frame pacer, the hub role rule, the AFK write plan, the watchdog's
//! sample and timers, the reserve checks, the perf window and every log text. The DLL feeds them the game's numbers.

use crate::config::auth::AuthMode;
use crate::config::backend::is_dotted_quad;
use crate::config::hub::{AfkSetting, HubCap};

/// An idle frame cap applies after this long with no client (4.2).
pub const IDLE_AFTER_MS: u64 = 15_000;
/// The tick-side and watchdog empty limit after a client left (5.5, 7.3; the surface's 50 s is older).
pub const EMPTY_LIMIT_S: u64 = 60;
/// The watchdog's limit when no client ever arrived.
pub const NEVER_JOINED_LIMIT_S: u64 = 300;
/// The watchdog's poll.
pub const POLL_MS: u32 = 2000;
/// A sample older than this is unknown.
pub const FRESH_MS: u64 = 10_000;
/// The most players or connections a sample may claim.
pub const MAX_COUNT: i32 = 64;
/// The stale-tick reap.
pub const STALE_REAP_MS: u64 = 180_000;
/// The stale-tick notice.
pub const STALE_NOTICE_MS: u64 = 60_000;
/// The port must be free this long, strictly more, before the server exits.
pub const PORT_FREE_MS: u64 = 45_000;
/// The idle hub trim waits this long with no client.
pub const TRIM_AFTER_MS: u64 = 60_000;
/// The trim's connection stamp must be this fresh.
pub const TRIM_FRESH_MS: u64 = 1000;
/// The reserve is released this long after the watchdog starts.
pub const RESERVE_AFTER_MS: u64 = 30_000;
/// A frame longer than this is a hitch (µs).
pub const HITCH_US: u64 = 100_000;
/// The engine's out-of-memory reserve.
pub const RESERVE_SIZE: usize = 0x200_0000;
/// The most "implausible counts" lines per process.
pub const IMPLAUSIBLE_LINES: u32 = 4;

/// The flag files R8 reads (11).
pub mod flags {
    /// No AFK write.
    pub const DISABLE_AFK_TIMEOUT_OVERRIDE: &str = "DISABLE_AFK_TIMEOUT_OVERRIDE";
    /// No expected-player sync.
    pub const DISABLE_EXPECTED_PLAYER_ZERO: &str = "DISABLE_EXPECTED_PLAYER_ZERO";
    /// The reserve stays.
    pub const KEEP_OOM_POOL: &str = "KEEP_OOM_POOL";
    /// No idle hub trim.
    pub const KEEP_WORKING_SET: &str = "KEEP_WORKING_SET";
    /// A player-hosted named hub with launcher role `HUNT` stays a hub.
    pub const KEEP_HOSTED_HUB_ROLE: &str = "KEEP_HOSTED_HUB_ROLE";
    /// No `[Perf]`/`[Memory]` lines.
    pub const DISABLE_PERF_LOG: &str = "DISABLE_PERF_LOG";
}

// ---- 4.2: the frame pacer ----

/// What one frame of the pacer decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pace {
    /// Sleep this long before the frame (µs).
    pub sleep_us: u64,
    /// The idle edge line, when the idle state changed.
    pub edge: Option<String>,
}

/// The pacer: times in, "sleep `n` µs" out. The caller calls `mark` with the time after its sleep.
#[derive(Debug, Clone, Default)]
pub struct Pacer {
    previous_us: Option<u64>,
    idle_since_ms: Option<u64>,
    was_idle: bool,
}

impl Pacer {
    /// A pacer before its first frame.
    pub const fn new() -> Self {
        Self {
            previous_us: None,
            idle_since_ms: None,
            was_idle: false,
        }
    }

    /// One frame: `active`/`idle` caps (0 = none / the same as active), whether R9 listens, R8's last count.
    pub fn step(&mut self, now_us: u64, now_ms: u64, active: i32, idle: i32, listening: bool, count: i32) -> Pace {
        if idle > 0 && listening && count == 0 {
            let since = *self.idle_since_ms.get_or_insert(now_ms);
            let idle_now = now_ms.saturating_sub(since) >= IDLE_AFTER_MS;
            self.edge(idle_now, active, idle, now_us)
        } else {
            self.idle_since_ms = None;
            self.edge(false, active, idle, now_us)
        }
    }

    fn edge(&mut self, idle_now: bool, active: i32, idle: i32, now_us: u64) -> Pace {
        let edge = (idle_now != self.was_idle).then(|| idle_edge_line(idle_now, active, idle));
        self.was_idle = idle_now;
        let fps = if idle_now { idle } else { active };
        let sleep_us = match (self.previous_us, u64::try_from(fps)) {
            (Some(previous), Ok(fps)) if fps > 0 => (previous + 1_000_000 / fps).saturating_sub(now_us),
            _ => 0,
        };
        Pace { sleep_us, edge }
    }

    /// The time read after the sleep, on every frame.
    pub fn mark(&mut self, now_us: u64) {
        self.previous_us = Some(now_us);
    }
}

/// `no client for 15 s: <idle> fps` / `client connected: <active> fps|no cap`.
pub fn idle_edge_line(idle_now: bool, active: i32, idle: i32) -> String {
    if idle_now {
        format!("[MitRuntime] [FramePace] no client for 15 s: {idle} fps")
    } else if active == 0 {
        "[MitRuntime] [FramePace] client connected: no cap".to_owned()
    } else {
        format!("[MitRuntime] [FramePace] client connected: {active} fps")
    }
}

// ---- 5.2: expected players ----

/// The sync's first line.
pub fn roster_line(disabled: bool, value: u32) -> String {
    let state = if disabled {
        "off (DISABLE_EXPECTED_PLAYER_ZERO.flag)"
    } else {
        "on"
    };
    format!("[MitRuntime] [ExpectedPlayers] roster sync {state} launchRoster={value}")
}

/// The first write to the game mode or game state.
pub fn expected_write_line(which: &str, object: usize, class: &str, before: i32, value: u32) -> String {
    format!("[MitRuntime] [ExpectedPlayers] {which} 0x{object:x} class={class} {before} -> {value}")
}

// ---- 5.4: the hub role ----

/// Whether a server's life is the Director's (or the launcher's) or its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubRole {
    /// Not classified yet: never reaps, never writes AFK.
    Unknown,
    /// A persistent hub: never reaped empty or stale.
    Hub,
    /// Ends when empty.
    Disposable,
}

impl HubRole {
    /// The heartbeat's text.
    pub fn text(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Hub => "hub",
            Self::Disposable => "disposable",
        }
    }

    /// The stored form for an atomic.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Hub => 1,
            Self::Disposable => 2,
        }
    }

    /// Back from `to_u8`.
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Hub,
            2 => Self::Disposable,
            _ => Self::Unknown,
        }
    }
}

/// `hub_role(class_name, mode, hosted_role_is_hub, keep_flag)`: the role and the line to log once, if any.
pub fn hub_role(class_name: &str, mode: AuthMode, hosted_hub: bool, keep_flag: bool) -> (HubRole, Option<String>) {
    let named = class_name.contains("City") || class_name.contains("TrainingGrounds");
    if mode != AuthMode::PlayerHost {
        return (if named { HubRole::Hub } else { HubRole::Disposable }, None);
    }
    if !named {
        let line = hosted_hub.then(|| {
            format!(
                "[MitRuntime] [HubRole] WARNING: {class_name} is disposable although the launcher role is HUB: not a \
                 named hub class (City / TrainingGrounds), so the empty reap and the hunt AFK rule apply"
            )
        });
        return (HubRole::Disposable, line);
    }
    let (role, text) = if hosted_hub {
        (HubRole::Hub, "persistent hub (launcher role)")
    } else if keep_flag {
        (
            HubRole::Hub,
            "persistent hub (KEEP_HOSTED_HUB_ROLE.flag, launcher role HUNT)",
        )
    } else {
        (
            HubRole::Disposable,
            "disposable (launcher role HUNT): reaped when empty, hunt AFK rule",
        )
    };
    (role, Some(format!("[MitRuntime] [HubRole] {class_name} -> {text}")))
}

// ---- 5.5: the tick-side empty reap ----

/// The game thread's empty timer (5.5).
#[derive(Debug, Clone, Copy, Default)]
pub struct TickReap {
    empty_seconds: f32,
}

impl TickReap {
    /// A timer at zero.
    pub const fn new() -> Self {
        Self { empty_seconds: 0.0 }
    }

    /// One frame: `game_mode_empty` is `Some(true)` empty, `Some(false)` occupied, `None` unknown. Returns the reap
    /// reason when the limit is reached on a disposable server.
    pub fn step(
        &mut self,
        game_mode_empty: Option<bool>,
        has_connection: bool,
        delta: f32,
        role: HubRole,
    ) -> Option<String> {
        let empty = match game_mode_empty {
            Some(empty) => empty,
            None => !has_connection,
        };
        if empty {
            self.empty_seconds += delta.max(0.0);
        } else {
            self.empty_seconds = 0.0;
        }
        #[allow(clippy::cast_precision_loss, reason = "60 is exact in f32")]
        let limit = EMPTY_LIMIT_S as f32;
        (self.empty_seconds >= limit && role == HubRole::Disposable).then(|| {
            let mode = if game_mode_empty.is_some() { "empty" } else { "unknown" };
            format!("tick: disposable instance empty 60 s (game mode {mode})")
        })
    }
}

// ---- 6: AFK ----

/// The two floats of an object.
pub type AfkPair = (f32, f32);

/// What `apply` writes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AfkWrites {
    /// The value, `None` for no override.
    pub value: Option<f32>,
    /// Write both game-state floats.
    pub game_state: bool,
    /// Write both game-mode floats.
    pub game_mode: bool,
}

/// `afk_plan(setting, gs_now, gm_now)` (6.3 step 5).
#[allow(clippy::float_cmp, reason = "a write happens when the stored value differs at all")]
pub fn afk_plan(seconds: Option<i64>, gs_now: AfkPair, gm_now: Option<AfkPair>) -> AfkWrites {
    let Some(seconds) = seconds else {
        return AfkWrites {
            value: None,
            game_state: false,
            game_mode: false,
        };
    };
    #[allow(clippy::cast_precision_loss, reason = "at most 604800, exact in f32")]
    let value = seconds as f32;
    let differs = |pair: AfkPair| pair.0 != value || pair.1 != value;
    AfkWrites {
        value: Some(value),
        game_state: differs(gs_now),
        game_mode: gm_now.is_some_and(differs),
    }
}

fn afk_text(setting: AfkSetting) -> String {
    match setting.seconds {
        None => format!("native ({})", setting.source),
        Some(seconds) => format!("{seconds} s ({})", setting.source),
    }
}

/// 6.1's config line; the hub cap is logged here for every server (call 8).
pub fn afk_config_line(hub: AfkSetting, hunt: AfkSetting, cap: HubCap) -> String {
    format!(
        "[MitRuntime] [AfkTimeout] config hub={} hunt={} maxPlayers={} ({})",
        afk_text(hub),
        afk_text(hunt),
        cap.value,
        cap.source
    )
}

/// 6.1's line when `DISABLE_AFK_TIMEOUT_OVERRIDE.flag` is present.
pub const AFK_OFF: &str = "[MitRuntime] [AfkTimeout] override off (DISABLE_AFK_TIMEOUT_OVERRIDE.flag): every role keeps \
                           the game's own timeout";

/// 6.1's warnings for a 0 timeout.
pub fn afk_warnings(hub: AfkSetting, hunt: AfkSetting) -> Vec<String> {
    let mut lines = Vec::new();
    if hub.seconds == Some(0) {
        lines.push(format!(
            "[MitRuntime] [AfkTimeout] WARNING: the hub timeout is 0 ({}): hub players are never idle-kicked and can \
             hold every hub slot",
            hub.source
        ));
    }
    if hunt.seconds == Some(0) {
        lines.push(format!(
            "[MitRuntime] [AfkTimeout] WARNING: the hunt timeout is 0 ({}): hunt players are never idle-kicked",
            hunt.source
        ));
    }
    lines
}

/// What 6.3's per-game-state line reports.
#[derive(Debug, Clone, Copy)]
pub struct AfkReport<'a> {
    /// The role used.
    pub hub: bool,
    /// The game mode's class, if any.
    pub class: Option<&'a str>,
    /// The game mode's floats before, if any.
    pub gm_before: Option<AfkPair>,
    /// The game state's floats before.
    pub gs_before: AfkPair,
    /// The setting's seconds.
    pub seconds: Option<i64>,
    /// `tick` or `postlogin`.
    pub why: &'a str,
    /// The game state.
    pub game_state: usize,
}

/// 6.3 step 6, floats printed with no decimals.
pub fn afk_apply_line(report: &AfkReport) -> String {
    let role = if report.hub { "hub" } else { "hunt" };
    let class = report.class.unwrap_or("?");
    let gm = report
        .gm_before
        .map_or_else(|| "?".to_owned(), |(a, b)| format!("{a:.0}/{b:.0}"));
    let (a, b) = report.gs_before;
    let applied = match report.seconds {
        None => "no override (native)".to_owned(),
        Some(0) => "applied 0 s (never kick)".to_owned(),
        Some(seconds) => format!("applied {seconds} s"),
    };
    format!(
        "[MitRuntime] [AfkTimeout] role={role} class={class} before GM={gm} GS={a:.0}/{b:.0} -> {applied} (why={}) \
         GS=0x{:x}",
        report.why, report.game_state
    )
}

// ---- 7: the watchdog ----

/// The state the game thread publishes, as the thread reads it (5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    /// `ClientConnections.Num`, -1 before the first.
    pub conn_count: i32,
    /// Its stamp, 0 before the first.
    pub conn_stamp: u64,
    /// `NumPlayers`, -1 before the first.
    pub num_players: i32,
    /// `NumTravellingPlayers`, -1 before the first.
    pub num_travelling: i32,
    /// Their stamp.
    pub counts_stamp: u64,
}

/// One poll's view of the server (7.2 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// Fresh and plausible counts.
    pub counts_usable: bool,
    /// A fresh and plausible connection count.
    pub conn_usable: bool,
    /// Empty: usable, no connection, no traveller.
    pub empty: bool,
    /// Counts out of range with a non-zero stamp.
    pub implausible: bool,
    /// `num_players`, -1 when unusable.
    pub players: i32,
    /// `num_travelling`, -1 when unusable.
    pub travelling: i32,
    /// `conn_count`, -1 when unusable.
    pub connections: i32,
}

fn fresh(stamp: u64, now: u64, limit: u64) -> bool {
    stamp != 0 && now.saturating_sub(stamp) <= limit
}

impl Sample {
    /// `Sample::from(published, now)`.
    pub fn from(published: Published, now: u64) -> Self {
        let plausible = |count: i32| (0..=MAX_COUNT).contains(&count);
        let counts_plausible = plausible(published.num_players) && plausible(published.num_travelling);
        let counts_usable = fresh(published.counts_stamp, now, FRESH_MS) && counts_plausible;
        let conn_usable = fresh(published.conn_stamp, now, FRESH_MS) && plausible(published.conn_count);
        let empty = counts_usable && conn_usable && published.conn_count <= 0 && published.num_travelling <= 0;
        Self {
            counts_usable,
            conn_usable,
            empty,
            implausible: published.counts_stamp != 0 && !counts_plausible,
            players: if counts_usable { published.num_players } else { -1 },
            travelling: if counts_usable { published.num_travelling } else { -1 },
            connections: if conn_usable { published.conn_count } else { -1 },
        }
    }
}

/// The implausible-counts line.
pub fn implausible_line(players: i32, travelling: i32) -> String {
    format!(
        "[MitRuntime] [Watchdog] ignoring implausible counts (players={players} travelling={travelling}, at most 64): \
         no reap"
    )
}

/// The heartbeat's cadence: every 15th poll, or every 5th while a disposable server is empty.
pub fn heartbeat_due(iter: u64, empty: bool, role: HubRole) -> bool {
    iter.is_multiple_of(15) || (empty && role == HubRole::Disposable && iter.is_multiple_of(5))
}

/// The heartbeat line.
pub fn heartbeat_line(role: HubRole, sample: &Sample, had_client: bool, empty_seconds: u64, limit: u64) -> String {
    format!(
        "[MitRuntime] [Watchdog] role={} players={} travelling={} connections={} hadClient={} countsUsable={} \
         empty={} emptySec={empty_seconds}/{limit}",
        role.text(),
        sample.players,
        sample.travelling,
        sample.connections,
        u8::from(had_client),
        u8::from(sample.counts_usable),
        u8::from(sample.empty),
    )
}

/// The watchdog's empty reap (7.3).
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyReap {
    /// Seconds of continuous emptiness.
    pub empty_seconds: u64,
}

/// The empty limit for `had_client`.
pub fn empty_limit(had_client: bool) -> u64 {
    if had_client {
        EMPTY_LIMIT_S
    } else {
        NEVER_JOINED_LIMIT_S
    }
}

impl EmptyReap {
    /// A timer at zero.
    pub const fn new() -> Self {
        Self { empty_seconds: 0 }
    }

    /// One poll; the reap reason at the limit.
    pub fn step(&mut self, empty: bool, role: HubRole, had_client: bool) -> Option<&'static str> {
        if !(empty && role == HubRole::Disposable) {
            self.empty_seconds = 0;
            return None;
        }
        self.empty_seconds += u64::from(POLL_MS / 1000);
        (self.empty_seconds >= empty_limit(had_client)).then_some(if had_client {
            "disposable server empty 60 s after its last client left"
        } else {
            "disposable server never joined by a client within 300 s"
        })
    }
}

/// What the stale-tick rule asks for (7.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    /// Nothing.
    Quiet,
    /// The 60 s notice line.
    Notice(String),
    /// The line, then the reap exit with reason `tick stale`.
    Reap(String),
}

/// The stale-tick rule (7.4).
pub fn stale_tick(role: HubRole, saw_player: bool, last_tick_ms: u64, now: u64, iter: u64, players: i32) -> Stale {
    if role != HubRole::Disposable || !saw_player || last_tick_ms == 0 {
        return Stale::Quiet;
    }
    let since = now.saturating_sub(last_tick_ms);
    if since >= STALE_REAP_MS {
        Stale::Reap(format!(
            "[MitRuntime] [Watchdog] disposable server's game thread silent for {} s (players={players}): a hung game \
             thread or a ghost connection",
            since / 1000
        ))
    } else if since >= STALE_NOTICE_MS && iter.is_multiple_of(15) {
        Stale::Notice(format!(
            "[MitRuntime] [Watchdog] game thread silent for {} s (players={players}); reap at 180 s",
            since / 1000
        ))
    } else {
        Stale::Quiet
    }
}

/// The probe's address (7.5): `None` → `0.0.0.0`, a dotted quad → itself; `Err` is the "off" line.
pub fn probe_address(multihome: Option<&str>) -> Result<[u8; 4], String> {
    let Some(text) = multihome else {
        return Ok([0, 0, 0, 0]);
    };
    if is_dotted_quad(text)
        && let Ok(address) = text.parse::<std::net::Ipv4Addr>()
    {
        return Ok(address.octets());
    }
    Err(format!(
        "[MitRuntime] [Watchdog] port probe off: MULTIHOME {text} is not an IPv4 address"
    ))
}

/// The port timer (7.5).
#[derive(Debug, Clone, Copy, Default)]
pub struct PortTimer {
    free_since: Option<u64>,
}

impl PortTimer {
    /// A timer with no free poll.
    pub const fn new() -> Self {
        Self { free_since: None }
    }

    /// One poll: `free` is the probe's answer. The exit line when the port has been free too long.
    pub fn step(&mut self, ever_had_counts: bool, free: bool, now: u64, port: u16) -> Option<String> {
        if !ever_had_counts || !free {
            self.free_since = None;
            return None;
        }
        let since = *self.free_since.get_or_insert(now);
        (now.saturating_sub(since) > PORT_FREE_MS).then(|| {
            format!(
                "[MitRuntime] [Watchdog] UDP port {port} has been free for 45 s after this server had served: it is \
                 alive but not listening; exiting so its supervisor restarts it"
            )
        })
    }
}

/// The idle hub trim's timer (7.7).
#[derive(Debug, Clone, Copy, Default)]
pub struct TrimTimer {
    idle_since: u64,
    trimmed: bool,
    trimmed_at: u64,
}

/// What the trim timer reads each poll.
#[derive(Debug, Clone, Copy)]
pub struct TrimInput {
    /// The role is `Hub`.
    pub hub: bool,
    /// The sample's counts are usable.
    pub counts_usable: bool,
    /// `conn_stamp`.
    pub conn_stamp: u64,
    /// `conn_count`.
    pub conn_count: i32,
    /// `num_travelling`.
    pub travelling: i32,
    /// `last_client_ms`.
    pub last_client_ms: u64,
    /// A Director server without `KEEP_WORKING_SET`.
    pub trim_allowed: bool,
}

impl TrimTimer {
    /// Armed, not idle.
    pub const fn new() -> Self {
        Self {
            idle_since: 0,
            trimmed: false,
            trimmed_at: 0,
        }
    }

    /// One poll: `Some(idle seconds)` when the trim is due now (it is then marked done).
    pub fn step(&mut self, input: TrimInput, now: u64) -> Option<u64> {
        if self.trimmed && input.last_client_ms > self.trimmed_at {
            self.trimmed = false;
        }
        let idle_now = input.hub
            && input.counts_usable
            && fresh(input.conn_stamp, now, TRIM_FRESH_MS)
            && input.conn_count == 0
            && input.travelling == 0;
        if !idle_now {
            self.idle_since = 0;
            return None;
        }
        if self.idle_since == 0 || input.last_client_ms >= self.idle_since {
            self.idle_since = now;
        }
        let idle_for = now.saturating_sub(self.idle_since);
        if input.trim_allowed && !self.trimmed && idle_for >= TRIM_AFTER_MS {
            self.trimmed = true;
            self.trimmed_at = now;
            return Some(idle_for / 1000);
        }
        None
    }
}

/// 7.1's two start lines.
pub fn watchdog_start_lines(keep_oom: bool, trim: Result<(), &'static str>, perf: bool) -> [String; 2] {
    let reserve = if keep_oom {
        "kept (KEEP_OOM_POOL.flag)"
    } else {
        "released at listen + 30 s"
    };
    let trim = match trim {
        Ok(()) => "on (hubs only, after 60 s with no client, once per idle period)",
        Err(reason) => reason,
    };
    let perf = if perf {
        "every 60 s"
    } else {
        "off (DISABLE_PERF_LOG.flag)"
    };
    [
        "[MitRuntime] [Watchdog] started (poll 2 s; reaps a disposable server with no client for 60 s after its last \
         client left, or 300 s if none ever arrived)"
            .to_owned(),
        format!("[MitRuntime] [Memory] reserve: {reserve}; idle hub trim: {trim}; [Perf]/[Memory] lines: {perf}"),
    ]
}

/// The trim's state for 7.1's line: `Ok` on, `Err` the reason it is off.
pub fn trim_state(director: bool, keep_working_set: bool) -> Result<(), &'static str> {
    if keep_working_set {
        Err("off (KEEP_WORKING_SET.flag)")
    } else if !director {
        Err("off (not a Director server)")
    } else {
        Ok(())
    }
}

/// The thread could not start.
pub fn watchdog_failed_line(error: &str) -> String {
    format!("[MitRuntime] [Watchdog] could not start: {error}; no empty-server reap in this process")
}

// ---- 7.6: the out-of-memory reserve ----

/// `SetupMemoryPools`' first 21 bytes.
pub const SETUP_POOLS_BYTES: [u8; 21] = [
    0x48, 0x83, 0xEC, 0x28, 0xB9, 0x00, 0x00, 0x00, 0x02, 0xE8, 0xF2, 0xC4, 0x13, 0x00, 0x48, 0x89, 0x05, 0x2B, 0x73,
    0x88, 0x04,
];
/// `OnOutOfMemory` at `+0x57`: the load of the global.
pub const OOM_LOAD_OFFSET: usize = 0x57;
/// Its bytes.
pub const OOM_LOAD_BYTES: [u8; 7] = [0x48, 0x8B, 0x0D, 0x72, 0x2F, 0x88, 0x04];
/// `OnOutOfMemory` at `+0x68`: the null test.
pub const OOM_TEST_OFFSET: usize = 0x68;
/// Its bytes.
pub const OOM_TEST_BYTES: [u8; 5] = [0x48, 0x85, 0xC9, 0x0F, 0x84];

/// What the DLL read for the two code checks.
#[derive(Debug, Clone, Default)]
pub struct ReserveSites {
    /// `function_entry(SETUP_MEMORY_POOLS)`.
    pub setup_entry: Option<u32>,
    /// Its first bytes.
    pub setup_bytes: Option<Vec<u8>>,
    /// `function_entry(ON_OUT_OF_MEMORY)`.
    pub oom_entry: Option<u32>,
    /// The bytes at `+0x57`.
    pub oom_load: Option<Vec<u8>>,
    /// The bytes at `+0x68`.
    pub oom_test: Option<Vec<u8>>,
}

/// 7.6 checks 1 and 2; `Err` is the reason.
pub fn reserve_sites_ok(sites: &ReserveSites, setup_rva: u32, oom_rva: u32) -> Result<(), &'static str> {
    if sites.setup_entry != Some(setup_rva) || sites.setup_bytes.as_deref() != Some(&SETUP_POOLS_BYTES[..]) {
        return Err("SetupMemoryPools not as verified");
    }
    if sites.oom_entry != Some(oom_rva)
        || sites.oom_load.as_deref() != Some(&OOM_LOAD_BYTES[..])
        || sites.oom_test.as_deref() != Some(&OOM_TEST_BYTES[..])
    {
        return Err("OnOutOfMemory not as verified");
    }
    Ok(())
}

/// `VirtualQuery`'s answer for the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRegion {
    /// `BaseAddress`.
    pub base: usize,
    /// `AllocationBase`.
    pub allocation_base: usize,
    /// `RegionSize`.
    pub size: usize,
    /// `State`.
    pub state: u32,
    /// `Type`.
    pub kind: u32,
    /// `Protect`.
    pub protect: u32,
}

/// `MEM_PRIVATE`.
pub const MEM_PRIVATE: u32 = 0x2_0000;
/// `PAGE_READWRITE`.
pub const PAGE_READWRITE: u32 = 0x04;

/// 7.6 check 4.
pub fn reserve_block_ok(block: usize, region: Option<BlockRegion>) -> bool {
    block != 0
        && region.is_some_and(|region| {
            region.base == block
                && region.allocation_base == block
                && region.size == RESERVE_SIZE
                && region.state == crate::memory::MEM_COMMIT
                && region.kind == MEM_PRIVATE
                && region.protect == PAGE_READWRITE
        })
}

/// A check failed.
pub fn reserve_left_line(reason: &str) -> String {
    format!("[MitRuntime] [Memory] reserve left alone: {reason}")
}

/// 7.6 check 4's reason.
pub const BLOCK_REASON: &str = "block is not the expected 32 MB private read-write allocation";

/// `VirtualFree` failed.
pub fn reserve_kept_line(error: u32) -> String {
    format!("[MitRuntime] [Memory] reserve kept: VirtualFree failed (error {error})")
}

/// The process's memory figures (`K32GetProcessMemoryInfo`), in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryInfo {
    /// `WorkingSetSize`.
    pub working_set: usize,
    /// `PeakWorkingSetSize`.
    pub peak_working_set: usize,
    /// `PrivateUsage`.
    pub private_commit: usize,
    /// `PageFaultCount`.
    pub page_faults: u32,
}

/// MiB with one decimal, `?` when unavailable.
pub fn mib(bytes: Option<usize>) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a display figure")]
    bytes.map_or_else(|| "?".to_owned(), |bytes| format!("{:.1}", bytes as f64 / 1_048_576.0))
}

/// 7.6 step 7.
pub fn reserve_released_line(before: Option<MemoryInfo>, after: Option<MemoryInfo>) -> String {
    format!(
        "[MitRuntime] [Memory] released the engine's 32 MB out-of-memory reserve: private commit {} -> {} MiB, \
         working set {} -> {} MiB",
        mib(before.map(|info| info.private_commit)),
        mib(after.map(|info| info.private_commit)),
        mib(before.map(|info| info.working_set)),
        mib(after.map(|info| info.working_set)),
    )
}

/// 7.7's line.
pub fn trim_line(
    idle_seconds: u64,
    before: Option<MemoryInfo>,
    after: Option<MemoryInfo>,
    ms: u64,
    error: Option<u32>,
) -> String {
    let failed = error.map_or_else(String::new, |error| format!(" (failed: error {error})"));
    format!(
        "[MitRuntime] [WorkingSet] trimmed (hub idle, no client for {idle_seconds} s): working set {} -> {} MiB, \
         private commit {} MiB, {ms} ms{failed}",
        mib(before.map(|info| info.working_set)),
        mib(after.map(|info| info.working_set)),
        mib(after.or(before).map(|info| info.private_commit)),
    )
}

// ---- 7.8: [Perf] and [Memory] ----

/// One drained perf window, in microseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PerfWindow {
    /// Frames measured.
    pub frames: u64,
    /// The summed intervals.
    pub sum_us: u64,
    /// The longest interval.
    pub max_us: u64,
    /// Intervals over 100 ms.
    pub hitches: u64,
}

/// What the perf lines add to the window.
#[derive(Debug, Clone, Copy)]
pub struct PerfContext {
    /// `conn_count`.
    pub connections: i32,
    /// `conn_stamp` older than 10 s.
    pub stale: bool,
    /// The role.
    pub role: HubRole,
    /// The first window since listen.
    pub first: bool,
}

fn suffixes(context: &PerfContext) -> (&'static str, &'static str) {
    (
        if context.stale { " (stale)" } else { "" },
        if context.first {
            " (first window since listen)"
        } else {
            ""
        },
    )
}

/// The `[Perf]` line.
#[allow(clippy::cast_precision_loss, reason = "display figures")]
pub fn perf_line(window: PerfWindow, context: &PerfContext) -> String {
    if window.frames == 0 {
        return "[MitRuntime] [Perf] no frames measured in this window (game thread not ticking?)".to_owned();
    }
    let (stale, first) = suffixes(context);
    let secs = window.sum_us as f64 / 1_000_000.0;
    let rate = if secs > 0.0 { window.frames as f64 / secs } else { 0.0 };
    let avg = window.sum_us as f64 / window.frames as f64 / 1000.0;
    let max = window.max_us as f64 / 1000.0;
    format!(
        "[MitRuntime] [Perf] {secs:.1}s: {} frames ({rate:.1}/s), frame avg {avg:.2} ms / max {max:.1} ms, \
         hitches>100ms {}, connections {}{stale}, role {}{first}",
        window.frames,
        window.hitches,
        context.connections,
        context.role.text(),
    )
}

/// The `[Memory]` line; `previous_faults` is the last line's count (`None` for the first: a total).
pub fn memory_line(info: Option<MemoryInfo>, previous_faults: Option<u32>, context: &PerfContext) -> String {
    let (stale, first) = suffixes(context);
    let faults = match (info, previous_faults) {
        (None, _) => "?".to_owned(),
        (Some(info), None) => format!("{} total", info.page_faults),
        (Some(info), Some(previous)) => format!("+{} since last line", info.page_faults.wrapping_sub(previous)),
    };
    format!(
        "[MitRuntime] [Memory] connections {}{stale}, working set {} MiB (peak {}), private commit {} MiB, page \
         faults {faults}{first}",
        context.connections,
        mib(info.map(|info| info.working_set)),
        mib(info.map(|info| info.peak_working_set)),
        mib(info.map(|info| info.private_commit)),
    )
}

// ---- 8: ending a server ----

/// The reap exit's line.
pub fn reap_line(reason: &str) -> String {
    format!("[MitRuntime] [Reap] {reason} -> TerminateProcess(0)")
}

/// The server quit's line.
pub fn quit_line(reason: &str) -> String {
    format!("[MitRuntime] [ServerQuit] quit requested ({reason}): ending through the reap exit")
}

/// The reap reason the server quit passes on.
pub fn quit_reason(reason: &str) -> String {
    format!("server quit ({reason})")
}

#[cfg(test)]
mod tests;
