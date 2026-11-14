//! Slice R10 (spec-runtime-r10): the player host's bootstrap check, hosted role, pipe messages and log texts.
//! Pure; the DLL's `player_host` reads the environment and speaks the pipes. The names are RT-8's (R10 4).

use crate::config::launch::{Launch, Mode};
use crate::env::Raw;

/// The ready and control pipes' prefix (RT-8; `rustic-rifts-host` writes it).
pub const PIPE_PREFIX: &str = r"\\.\pipe\RusticInternalServer.";
/// The tag on every pipe message (RT-8, call 1).
pub const TAG: &str = "RIS1";
/// The host capability header (D3).
pub const HEADER: &str = "x-rustic-host-capability";
/// The only listen address a player host takes.
pub const BIND: &str = "127.0.0.1";
/// The engine argument the launcher must pass (check 3, ASCII case ignored).
pub const MULTIHOME_ARG: &str = "-MULTIHOME=127.0.0.1";

/// `RUSTIC_ISLES_BIND_ADDRESS`.
pub const BIND_ADDRESS: &str = "RUSTIC_ISLES_BIND_ADDRESS";
/// `RUSTIC_ISLES_SESSION_ID`.
pub const SESSION_ID: &str = "RUSTIC_ISLES_SESSION_ID";
/// `RUSTIC_ISLES_SESSION_GENERATION`.
pub const SESSION_GENERATION: &str = "RUSTIC_ISLES_SESSION_GENERATION";
/// `RUSTIC_ISLES_HOST_RUNTIME_CAPABILITY`, a secret.
pub const CAPABILITY: &str = "RUSTIC_ISLES_HOST_RUNTIME_CAPABILITY";
/// `RUSTIC_ISLES_CONTROL_PIPE`.
pub const CONTROL_PIPE: &str = "RUSTIC_ISLES_CONTROL_PIPE";
/// `RUSTIC_ISLES_READY_PIPE`.
pub const READY_PIPE: &str = "RUSTIC_ISLES_READY_PIPE";
/// `RUSTIC_ISLES_GAMESERVER_LAUNCH_ID`.
pub const LAUNCH_ID: &str = "RUSTIC_ISLES_GAMESERVER_LAUNCH_ID";
/// `RUSTIC_ISLES_LAUNCHER_PID`.
pub const LAUNCHER_PID: &str = "RUSTIC_ISLES_LAUNCHER_PID";
/// `RUSTIC_ISLES_HOSTED_ROLE`.
pub const HOSTED_ROLE: &str = "RUSTIC_ISLES_HOSTED_ROLE";
/// `RUSTIC_ISLES_MAX_PLAYERS`.
pub const MAX_PLAYERS: &str = "RUSTIC_ISLES_MAX_PLAYERS";
/// `RUSTIC_ISLES_HUB_AFK_SECONDS`.
pub const HUB_AFK_SECONDS: &str = "RUSTIC_ISLES_HUB_AFK_SECONDS";
/// `RUSTIC_ISLES_HUNT_GUARD_EVENTS`, read only for the Hunt Guard line.
pub const HUNT_GUARD_EVENTS: &str = "RUSTIC_ISLES_HUNT_GUARD_EVENTS";

/// An invalid bootstrap, on the first frame.
pub const INVALID_EXIT_CODE: u32 = 0xE103;
/// The control thread could not start.
pub const THREAD_EXIT_CODE: u32 = 0xE104;
/// The game thread did not apply a fence within 5 s.
pub const FENCE_TIMEOUT_EXIT_CODE: u32 = 0xE105;
/// A fence applied on the game thread.
pub const FENCE_EXIT_CODE: u32 = 0;
/// The control thread's wait for the game thread (departure 3).
pub const FENCE_TIMEOUT_MS: u32 = 5000;
/// The longest control message read (a longer one fails the read).
pub const MAX_MESSAGE: usize = 511;
/// R10's SP-3 before handler (RT-17): before R5's -1000 and R11's -500.
pub const TICK_ORDER: i32 = -2000;
/// R10's HTTP-1 wrapper, after R2's 0 (R2 3.2).
pub const HEADER_ORDER: i32 = 100;
/// The ready pipe's wait, and the budget for retrying a busy instance.
pub const READY_WAIT_MS: u32 = 5000;
/// The sleep between busy retries.
pub const READY_RETRY_MS: u32 = 100;

const SESSION_LEN: usize = 36;
const CAPABILITY_LEN: usize = 43;
const PIPE_MAX: usize = 240;
const PIPE_SUFFIX: std::ops::RangeInclusive<usize> = 16..=96;

/// The launcher's role, as checked (5.3). The numbers are kept for a hunt too, for its line only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LauncherRole {
    /// `HUB`; absent or `HUNT` is false.
    pub hub: bool,
    /// `RUSTIC_ISLES_MAX_PLAYERS`, 1 to 64.
    pub max_players: Option<u8>,
    /// `RUSTIC_ISLES_HUB_AFK_SECONDS`, 0 to 7200.
    pub hub_afk_seconds: Option<u16>,
}

/// The hosted role other slices read (11): the numbers only for a hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostedRole {
    /// A persistent hub.
    pub hub: bool,
    /// `Some` only for a hub.
    pub max_players: Option<u8>,
    /// `Some` only for a hub.
    pub hub_afk_seconds: Option<u16>,
}

impl LauncherRole {
    /// What R1 and R8 see.
    pub fn hosted(&self) -> HostedRole {
        HostedRole {
            hub: self.hub,
            max_players: self.max_players.filter(|_| self.hub),
            hub_afk_seconds: self.hub_afk_seconds.filter(|_| self.hub),
        }
    }
}

/// The values a valid bootstrap keeps for the process (5.2: never read again). Never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct Bootstrap {
    /// 36 safe characters.
    pub session: String,
    /// 1 to 10 digits, as text.
    pub generation: String,
    /// The host capability: 43 safe characters, a secret.
    pub capability: String,
    /// The control pipe's full name.
    pub control_pipe: String,
    /// The ready pipe's full name.
    pub ready_pipe: String,
    /// 16 to 96 safe characters.
    pub launch_id: String,
    /// The launcher's pid, 1 or more.
    pub launcher_pid: u32,
    /// The launcher's role.
    pub role: LauncherRole,
    /// `RUSTIC_ISLES_HUNT_GUARD_EVENTS` is `1`.
    pub guard_opt_in: bool,
}

impl std::fmt::Debug for Bootstrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The capability, session, launch id and pipe names are never logged (14.1)
        f.debug_struct("Bootstrap")
            .field("role", &self.role)
            .field("guard_opt_in", &self.guard_opt_in)
            .finish_non_exhaustive()
    }
}

/// 5.4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every check passed.
    Valid(Box<Bootstrap>),
    /// The first failed 5.2 check by label, and the first role problem.
    Invalid {
        /// A 5.2 label; `None` when only the role failed.
        failed_check: Option<&'static str>,
        /// A 5.3 problem text.
        role_problem: Option<&'static str>,
    },
}

/// ASCII letters, digits, `.`, `_` and `-`.
pub fn is_safe(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn safe_len(text: &str, range: std::ops::RangeInclusive<usize>) -> bool {
    range.contains(&text.len()) && is_safe(text)
}

/// Checks 8 and 9: the prefix (case-sensitive), then 16 to 96 safe characters.
pub fn is_pipe_name(text: &str) -> bool {
    text.len() <= PIPE_MAX
        && text
            .strip_prefix(PIPE_PREFIX)
            .is_some_and(|suffix| safe_len(suffix, PIPE_SUFFIX))
}

fn digits(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Check 11: 1 to 10 digits whose value is 1 to 4 294 967 295.
pub fn parse_pid(text: &str) -> Option<u32> {
    if !digits(text, 10) {
        return None;
    }
    text.parse::<u64>()
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|&pid| pid != 0)
}

/// R0's `env_value` over R1's raw read: unset, over the cap, NUL, CR, LF or non-ASCII are absent.
fn env_value(raw: Raw) -> Option<String> {
    match raw {
        Raw::Value(text)
            if text
                .bytes()
                .all(|byte| byte.is_ascii() && !matches!(byte, 0 | b'\r' | b'\n')) =>
        {
            Some(text)
        }
        _ => None,
    }
}

/// A 5.3 read: `Ok(None)` absent, `Err(())` malformed.
fn role_raw(raw: Raw, exists: bool) -> Result<Option<String>, ()> {
    match raw {
        Raw::Unset if !exists => Ok(None),
        Raw::Unset | Raw::TooLong(_) => Err(()),
        Raw::Value(text) if text.chars().any(|c| c < ' ' || c == '\u{7F}') => Err(()),
        Raw::Value(text) => Ok(Some(text)),
    }
}

/// A plain decimal: digits only, no sign, no leading zero but `0` itself, at most 5 digits.
pub fn plain_decimal(text: &str) -> Option<u32> {
    if !digits(text, 5) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}

/// The 5.3 problem texts.
pub mod problems {
    /// The role variable exists but is empty or malformed.
    pub const ROLE_MALFORMED: &str = "RUSTIC_ISLES_HOSTED_ROLE is set but empty or malformed";
    /// Neither role.
    pub const ROLE_INVALID: &str = "RUSTIC_ISLES_HOSTED_ROLE is neither HUNT nor HUB";
    /// The cap is malformed.
    pub const MAX_PLAYERS_MALFORMED: &str = "RUSTIC_ISLES_MAX_PLAYERS is set but empty or malformed";
    /// The cap is out of range.
    pub const MAX_PLAYERS_INVALID: &str = "RUSTIC_ISLES_MAX_PLAYERS is not a whole number from 1 to 64";
    /// The AFK timeout is malformed.
    pub const AFK_MALFORMED: &str = "RUSTIC_ISLES_HUB_AFK_SECONDS is set but empty or malformed";
    /// The AFK timeout is out of range.
    pub const AFK_INVALID: &str = "RUSTIC_ISLES_HUB_AFK_SECONDS is not a whole number from 0 to 7200";
}

/// 5.3, in order; the first problem stops it.
pub fn launcher_role(
    read: &impl Fn(&str, usize) -> Raw,
    exists: &impl Fn(&str) -> bool,
) -> Result<LauncherRole, &'static str> {
    use problems::*;
    let hub = match role_raw(read(HOSTED_ROLE, 8), exists(HOSTED_ROLE)).map_err(|()| ROLE_MALFORMED)? {
        None => false,
        Some(role) if role == "HUNT" => false,
        Some(role) if role == "HUB" => true,
        Some(_) => return Err(ROLE_INVALID),
    };
    let number = |name: &str, malformed, invalid, range: std::ops::RangeInclusive<u32>| {
        role_raw(read(name, 5), exists(name))
            .map_err(|()| malformed)?
            .map(|text| {
                plain_decimal(&text)
                    .filter(|value| range.contains(value))
                    .ok_or(invalid)
            })
            .transpose()
    };
    let max_players = number(MAX_PLAYERS, MAX_PLAYERS_MALFORMED, MAX_PLAYERS_INVALID, 1..=64)?;
    let hub_afk_seconds = number(HUB_AFK_SECONDS, AFK_MALFORMED, AFK_INVALID, 0..=7200)?;
    Ok(LauncherRole {
        hub,
        max_players: max_players.and_then(|value| u8::try_from(value).ok()),
        hub_afk_seconds: hub_afk_seconds.and_then(|value| u16::try_from(value).ok()),
    })
}

/// 5.2's checks in order: the values, or the label of the first that failed.
fn launch_values(
    read: &impl Fn(&str, usize) -> Raw,
    launch: &Launch,
    argv: &[Vec<u16>],
) -> Result<Bootstrap, &'static str> {
    if launch.mode != Mode::Full {
        return Err("full launch");
    }
    if launch.raw_entry_1.as_deref() != Some(&[u16::from(b'-')][..]) {
        return Err("argv 1");
    }
    let multihome: Vec<u16> = MULTIHOME_ARG.encode_utf16().collect();
    let pinned = argv.iter().skip(1).any(|entry| {
        entry.len() == multihome.len()
            && entry
                .iter()
                .zip(&multihome)
                .all(|(&a, &b)| a < 0x80 && (a as u8).eq_ignore_ascii_case(&(b as u8)))
    });
    if !pinned {
        return Err("MULTIHOME");
    }
    let value = |name: &'static str, max: usize, valid: &dyn Fn(&str) -> bool| {
        env_value(read(name, max)).filter(|text| valid(text)).ok_or(name)
    };
    value(BIND_ADDRESS, 15, &|text| text == BIND)?;
    let session = value(SESSION_ID, 36, &|text| safe_len(text, SESSION_LEN..=SESSION_LEN))?;
    let generation = value(SESSION_GENERATION, 10, &|text| digits(text, 10))?;
    let capability = value(CAPABILITY, 43, &|text| safe_len(text, CAPABILITY_LEN..=CAPABILITY_LEN))?;
    let control_pipe = value(CONTROL_PIPE, PIPE_MAX, &is_pipe_name)?;
    let ready_pipe = value(READY_PIPE, PIPE_MAX, &is_pipe_name)?;
    let launch_id = value(LAUNCH_ID, 96, &|text| safe_len(text, 16..=96))?;
    let launcher_pid =
        value(LAUNCHER_PID, 10, &|text| parse_pid(text).is_some()).map(|text| parse_pid(&text).unwrap_or(0))?;
    Ok(Bootstrap {
        session,
        generation,
        capability,
        control_pipe,
        ready_pipe,
        launch_id,
        launcher_pid,
        role: LauncherRole {
            hub: false,
            max_players: None,
            hub_afk_seconds: None,
        },
        guard_opt_in: env_value(read(HUNT_GUARD_EVENTS, 8)).as_deref() == Some("1"),
    })
}

/// 5.4: the whole check over an environment (`read(name, cap)` and `exists(name)`), the launch and argv.
pub fn bootstrap(
    read: impl Fn(&str, usize) -> Raw,
    exists: impl Fn(&str) -> bool,
    launch: &Launch,
    argv: &[Vec<u16>],
) -> Verdict {
    let values = launch_values(&read, launch, argv);
    let role = launcher_role(&read, &exists);
    match (values, role) {
        (Ok(mut bootstrap), Ok(role)) => {
            bootstrap.role = role;
            Verdict::Valid(Box::new(bootstrap))
        }
        (values, role) => Verdict::Invalid {
            failed_check: values.err(),
            role_problem: role.err(),
        },
    }
}

/// `RIS1 FENCE session=<s> generation=<g>\n`, launcher to runtime.
pub fn fence_request(session: &str, generation: &str) -> String {
    format!("{TAG} FENCE session={session} generation={generation}\n")
}

/// `RIS1 FENCE_ACK session=<s> generation=<g>\n`, the runtime's answer.
pub fn fence_ack(session: &str, generation: &str) -> String {
    format!("{TAG} FENCE_ACK session={session} generation={generation}\n")
}

/// `RIS1 READY …\n` on the ready pipe.
pub fn ready_message(session: &str, generation: &str, launch_id: &str, port: u16) -> String {
    format!("{TAG} READY session={session} generation={generation} launch={launch_id} port={port} bind={BIND}\n")
}

/// Exact byte equality with `fence_request`.
pub fn is_fence(bytes: &[u8], session: &str, generation: &str) -> bool {
    bytes == fence_request(session, generation).as_bytes()
}

/// The log texts of section 15 (every line goes through R0's `[MitRuntime]` writer).
pub mod lines {
    /// The control thread is running.
    pub const WORKER_STARTED: &str = "[MitRuntime] [PlayerHost] control pipe worker started";
    /// `CreateThread` failed.
    pub const THREAD_FAILED: &str =
        "[MitRuntime] [PlayerHost] control thread creation failed; terminating before listen";
    /// A client that is not the launcher.
    pub const UNEXPECTED_PID: &str = "[MitRuntime] [PlayerHost] rejected an unexpected client pid";
    /// Anything but the exact fence.
    pub const MALFORMED: &str = "[MitRuntime] [PlayerHost] rejected a malformed fence request";
    /// The fence was taken.
    pub const FENCED: &str = "[MitRuntime] [PlayerHost] generation fenced by the launcher";
    /// The control thread's fallback.
    pub const FENCE_TIMEOUT: &str =
        "[MitRuntime] [PlayerHost] the game thread did not apply the fence within 5 s; forcing termination";
    /// The game thread ends the process.
    pub const APPLYING: &str = "[MitRuntime] [PlayerHost] applying the fence on the game thread; ending the process";
    /// The ready write failed.
    pub const READY_WRITE_FAILED: &str = "[MitRuntime] [PlayerHost] the readiness message write failed";
    /// The ready message is out.
    pub const READY_REPORTED: &str = "[MitRuntime] [PlayerHost] readiness reported to the launcher";
    /// `prepare_listen` without a valid bootstrap.
    pub const NOT_LOOPBACK: &str = "[MitRuntime] [PlayerHost] bind address is not loopback; refusing listen";
    /// The install line's reason off a player host.
    pub const NOT_A_PLAYER_HOST: &str = "skipped (not a player host)";

    /// `CreateNamedPipeW` (or its descriptor) failed.
    pub fn pipe_failed(error: u32) -> String {
        format!("[MitRuntime] [PlayerHost] control pipe creation failed error={error}")
    }

    /// `ConnectNamedPipe` failed.
    pub fn not_connected(error: u32) -> String {
        format!("[MitRuntime] [PlayerHost] the launcher did not connect error={error}")
    }

    /// `WaitNamedPipeW` failed.
    pub fn ready_unavailable(error: u32, attempt: u32) -> String {
        format!(
            "[MitRuntime] [PlayerHost] the launcher's readiness pipe did not become available error={error} \
             attempt={attempt}"
        )
    }

    /// `CreateFileW` failed.
    pub fn ready_open_failed(error: u32, attempt: u32) -> String {
        format!("[MitRuntime] [PlayerHost] the launcher's readiness pipe open failed error={error} attempt={attempt}")
    }
}

/// Step 1's lines: the failed check (or `hosted role`), then the role problem when there is one.
pub fn invalid_lines(failed_check: Option<&str>, role_problem: Option<&str>) -> Vec<String> {
    let mut out = vec![format!(
        "[MitRuntime] [PlayerHost] invalid player-host bootstrap ({}); terminating before listen",
        failed_check.unwrap_or("hosted role")
    )];
    if let Some(problem) = role_problem {
        out.push(format!("[MitRuntime] [HostedRole] invalid launcher role: {problem}"));
    }
    out
}

/// Step 2's role line.
pub fn role_line(role: &LauncherRole) -> String {
    if role.hub {
        let max = role.max_players.map_or_else(
            || "not set (host setting or default)".to_owned(),
            |n| format!("{n} (launcher)"),
        );
        let afk = role.hub_afk_seconds.map_or_else(
            || "not set (host setting or default)".to_owned(),
            |n| format!("{n} (launcher)"),
        );
        format!("[MitRuntime] [HostedRole] launcher role=HUB (persistent hub) maxPlayers={max} hubAfkSeconds={afk}")
    } else if role.max_players.is_some() || role.hub_afk_seconds.is_some() {
        "[MitRuntime] [HostedRole] launcher role=HUNT (disposable) - RUSTIC_ISLES_MAX_PLAYERS / \
         RUSTIC_ISLES_HUB_AFK_SECONDS are set but ignored (they apply to a HUB only)"
            .to_owned()
    } else {
        "[MitRuntime] [HostedRole] launcher role=HUNT (disposable)".to_owned()
    }
}

/// Step 3's Hunt Guard line.
pub fn guard_line(opt_in: bool) -> String {
    format!(
        "[MitRuntime] [HuntGuard] no evidence producer in this runtime: the hunt is unmonitored and earns no rewards \
         (opt-in {})",
        if opt_in { "1" } else { "not set" }
    )
}

#[cfg(test)]
mod tests;
