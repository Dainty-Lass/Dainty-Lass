// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The public server status (`/dainty/api/ServerStatus`) and its sanitizing parser for untrusted replies.

use chrono::{Datelike, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::text::{clean_display_text, js_trim};

/// Route answering with a [`ServerStatus`].
pub const STATUS_PATH: &str = "/dainty/api/ServerStatus";

const MAX_NAME_CHARS: usize = 64;
const MAX_VERSION_CHARS: usize = 48;
const MAX_COMMIT_CHARS: usize = 64;
const MAX_SOURCE_URL_LEN: usize = 512;
const MAX_PLAYER_NAME_CHARS: usize = 32;
const MAX_INSTANCE_ID_CHARS: usize = 64;
const MAX_INSTANCE_TITLE_CHARS: usize = 64;
const MAX_INSTANCE_MAP_CHARS: usize = 96;
const MAX_BEHEMOTH_CHARS: usize = 64;
const MAX_BUILD_CHARS: usize = 64;
const MAX_PLAYERS_LISTED: usize = 500;
const MAX_INSTANCES_LISTED: usize = 100;
const MAX_PLAYERS_ONLINE: u32 = 100_000;
const MAX_INSTANCE_PLAYERS: u32 = 1_000;
const MAX_UPTIME_SECONDS: u64 = 3_153_600_000;
const MAX_CHANGELIST: u64 = 100_000_000;
const MAX_STARTED_AT_RAW_LEN: usize = 64;
/// The longest cell fusion a server may ask for: seven days, the host kit's and the runtime's ceiling (R15).
pub const MAX_CELL_FUSION_MINUTES: u32 = 10_080;

/// Who may register, serialized uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "UPPERCASE")]
pub enum RegistrationMode {
    /// Anyone may register.
    Open,
    /// Registration needs an invite code.
    Invitecode,
    /// Registration is closed.
    None,
}

/// Where a player is, as the status reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum PlayerWhere {
    /// In the menus, not in an instance.
    Menu,
    /// In a city instance.
    City,
    /// On a hunt.
    Hunt,
    /// In the dojo.
    Dojo,
    /// In the tutorial.
    Tutorial,
    /// Unrecognised or missing in the reply.
    Unknown,
}

/// Kind of a running game instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum InstanceKind {
    /// City (Ramsgate).
    City,
    /// Hunt.
    Hunt,
    /// Dojo.
    Dojo,
    /// Tutorial.
    Tutorial,
}

/// Where the server's game data comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameDataKind {
    /// Captured from the real game.
    Real,
    /// Generated in place of missing captures.
    Synthetic,
    /// Not installed.
    Missing,
    /// Unrecognised or missing in the reply.
    Unknown,
}

/// An online player in the status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StatusPlayer {
    /// Display name, at most 32 characters.
    pub name: String,
    /// Where the player is, `where` on the wire.
    #[serde(rename = "where")]
    pub location: PlayerWhere,
    /// Id of the player's instance, if any.
    pub instance: Option<String>,
}

/// A running game instance in the status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StatusInstance {
    /// Instance id, at most 64 characters.
    pub id: String,
    /// What kind of instance it is.
    pub kind: InstanceKind,
    /// Display title, empty when the server sends none.
    pub title: String,
    /// Map name, empty when the server sends none.
    pub map: String,
    /// Behemoth being hunted, if any.
    pub behemoth: Option<String>,
    /// Players in the instance, clamped to 1000.
    pub players: u32,
    /// Instance capacity, clamped to 1000.
    pub max_players: u32,
    /// Start time as an ISO 8601 UTC string, when valid.
    pub started_at: Option<String>,
}

/// Game build the server runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StatusGame {
    /// Build name.
    pub build: String,
    /// Game changelist, if reported.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub changelist: Option<u32>,
}

/// The game servers' supply caps and fusion time (spec-runtime-r15 Q-1), which the launcher hands the player's game.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StatusEconomy {
    /// `true` keeps the game's stack caps on supplies; unset leaves the game's default (uncapped).
    pub keep_supply_stack_caps: Option<bool>,
    /// Minutes a cell fusion takes, 0 for the game's own; unset leaves the game's default (30).
    pub cell_fusion_minutes: Option<u32>,
    /// Wire names of the fields sent but refused, so the launcher can say so without echoing the value.
    pub refused: Vec<String>,
}

/// Public server status, camelCase on the wire; every field is sanitized by [`parse_server_status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    /// Server name, at most 64 characters.
    pub name: String,
    /// Whether the game backend is up; false when missing.
    pub online: bool,
    /// Server software version, empty when missing.
    pub version: String,
    /// Server software commit, empty when missing.
    pub commit: String,
    /// Source code link, `https` only.
    pub source_url: Option<String>,
    /// Registration mode, if reported.
    pub registration: Option<RegistrationMode>,
    /// Count of online players, zero in a limited view.
    pub players_online: u32,
    /// Online players, at most 500; empty in a limited view.
    pub players: Vec<StatusPlayer>,
    /// Running instances, at most 100; empty in a limited view.
    pub instances: Vec<StatusInstance>,
    /// Seconds since the server started.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub uptime_seconds: u32,
    /// Game build, if reported.
    pub game: Option<StatusGame>,
    /// Where the game data comes from.
    pub game_data: GameDataKind,
    /// Whether the server runs chat.
    pub chat: bool,
    /// Port of the content server, when it is published separately.
    pub content_port: Option<u16>,
    /// Whether player and instance lists were withheld (see [`limited_view`]).
    pub limited: bool,
    /// The game servers' economy settings, when the server sends any; absent from Node's and older servers'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub economy: Option<StatusEconomy>,
}

/// Cleans an optional JSON string value by stripping control/bidi characters, collapsing
/// whitespace, and truncating to `max` Unicode scalar values; returns `None` if missing or empty.
#[must_use]
pub fn clean_text(value: Option<&Value>, max: usize) -> Option<String> {
    let s = value?.as_str()?;
    let cleaned = clean_display_text(s, max);
    if cleaned.is_empty() { None } else { Some(cleaned) }
}

fn clean_str_or_empty(val: Option<&Value>, max_chars: usize) -> String {
    clean_text(val, max_chars).unwrap_or_default()
}

fn trunc_clamped_u64(val: Option<&Value>, max: u64) -> Option<u64> {
    let n = val?.as_f64()?;
    if !n.is_finite() {
        return None;
    }
    let t = n.trunc();
    if t <= 0.0 {
        return Some(0);
    }
    if t >= max as f64 {
        return Some(max);
    }
    Some(t as u64)
}

fn trunc_clamped_u32(val: Option<&Value>, max: u32) -> u32 {
    trunc_clamped_u64(val, u64::from(max)).unwrap_or(0) as u32
}

/// Validates and normalizes an optional JSON string as a credential-free `https://` URL
/// of at most 512 characters.
#[must_use]
pub fn clean_https_url(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?;
    let cleaned = clean_display_text(raw, MAX_SOURCE_URL_LEN + 1);
    if cleaned.is_empty() || cleaned.chars().count() > MAX_SOURCE_URL_LEN {
        return None;
    }
    let url = Url::parse(&cleaned).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.host_str().is_none() {
        return None;
    }
    let out: String = url.into();
    if out.len() > MAX_SOURCE_URL_LEN {
        return None;
    }
    Some(out)
}

fn parse_registration(val: Option<&Value>) -> Option<RegistrationMode> {
    match val?.as_str()? {
        "OPEN" => Some(RegistrationMode::Open),
        "INVITECODE" => Some(RegistrationMode::Invitecode),
        "NONE" => Some(RegistrationMode::None),
        _ => None,
    }
}

fn parse_player_where(val: Option<&Value>) -> PlayerWhere {
    match val.and_then(Value::as_str) {
        Some("menu") => PlayerWhere::Menu,
        Some("city") => PlayerWhere::City,
        Some("hunt") => PlayerWhere::Hunt,
        Some("dojo") => PlayerWhere::Dojo,
        Some("tutorial") => PlayerWhere::Tutorial,
        _ => PlayerWhere::Unknown,
    }
}

fn parse_instance_kind(val: Option<&Value>) -> Option<InstanceKind> {
    match val?.as_str()? {
        "city" => Some(InstanceKind::City),
        "hunt" => Some(InstanceKind::Hunt),
        "dojo" => Some(InstanceKind::Dojo),
        "tutorial" => Some(InstanceKind::Tutorial),
        _ => None,
    }
}

fn parse_game_data(val: Option<&Value>) -> GameDataKind {
    match val.and_then(Value::as_str) {
        Some("real") => GameDataKind::Real,
        Some("synthetic") => GameDataKind::Synthetic,
        Some("missing") => GameDataKind::Missing,
        _ => GameDataKind::Unknown,
    }
}

fn parse_exact_digits(s: &str, len: usize) -> Option<u32> {
    if s.len() != len || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok()
}

/// Parses an ECMAScript ISO-8601 date/date-time string and formats it as canonical UTC
/// `YYYY-MM-DDTHH:MM:SS.sssZ`.
#[must_use]
pub fn iso_date(text: &str) -> Option<String> {
    if text.is_empty() || text.len() > MAX_STARTED_AT_RAW_LEN {
        return None;
    }
    let s = js_trim(text);
    if s.is_empty() {
        return None;
    }

    let (date_part, time_part) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };

    let (year, after_year) = if let Some(rest) = date_part.strip_prefix('+') {
        let y_str = rest.get(..6)?;
        if !y_str.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let y = y_str.parse::<i32>().ok()?;
        (y, &rest[6..])
    } else if let Some(rest) = date_part.strip_prefix('-') {
        let y_str = rest.get(..6)?;
        if !y_str.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let y = y_str.parse::<i32>().ok()?;
        if y == 0 {
            return None;
        }
        (-y, &rest[6..])
    } else {
        let y_str = date_part.get(..4)?;
        let y = parse_exact_digits(y_str, 4)? as i32;
        (y, &date_part[4..])
    };

    let (month, day) = if after_year.is_empty() {
        (1, 1)
    } else {
        let after_dash1 = after_year.strip_prefix('-')?;
        let m_str = after_dash1.get(..2)?;
        let m = parse_exact_digits(m_str, 2)?;
        let after_m = &after_dash1[2..];
        if after_m.is_empty() {
            (m, 1)
        } else {
            let after_dash2 = after_m.strip_prefix('-')?;
            let d = parse_exact_digits(after_dash2, 2)?;
            (m, d)
        }
    };

    let naive_date = NaiveDate::from_ymd_opt(year, month, day)?;

    let (hour, min, sec, ms, offset_secs) = if let Some(t) = time_part {
        let tz_pos = t.find(['Z', '+', '-'])?;
        let (clock_part, tz_part) = t.split_at(tz_pos);
        let offset = if tz_part == "Z" {
            0i32
        } else {
            let sign = if tz_part.starts_with('+') { 1i32 } else { -1i32 };
            let tz_rest = &tz_part[1..];
            let (tz_h_str, tz_m_str) = tz_rest.split_once(':')?;
            let tz_h = parse_exact_digits(tz_h_str, 2)?;
            let tz_m = parse_exact_digits(tz_m_str, 2)?;
            if tz_h > 23 || tz_m > 59 {
                return None;
            }
            sign * ((tz_h as i32) * 3600 + (tz_m as i32) * 60)
        };

        let (hms_part, frac_part) = match clock_part.split_once('.') {
            Some((hms, frac)) => {
                if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                (hms, Some(frac))
            }
            None => (clock_part, None),
        };

        let mut parts = hms_part.split(':');
        let h = parse_exact_digits(parts.next()?, 2)?;
        let m = parse_exact_digits(parts.next()?, 2)?;
        let sc = match parts.next() {
            Some(sc_str) => parse_exact_digits(sc_str, 2)?,
            None => 0,
        };
        if parts.next().is_some() || h > 24 || m > 59 || sc > 59 {
            return None;
        }

        let millis = match frac_part {
            Some(f) => {
                let mut padded = [b'0'; 3];
                for (i, b) in f.bytes().take(3).enumerate() {
                    padded[i] = b;
                }
                std::str::from_utf8(&padded).ok()?.parse::<u32>().ok()?
            }
            None => 0,
        };
        if h == 24 && (m != 0 || sc != 0 || millis != 0 || frac_part.is_some_and(|f| f.bytes().any(|b| b != b'0'))) {
            return None;
        }

        (h, m, sc, millis, offset)
    } else {
        (0, 0, 0, 0, 0)
    };

    let (naive_date, hour) = if hour == 24 {
        (naive_date.succ_opt()?, 0)
    } else {
        (naive_date, hour)
    };
    let naive_dt = naive_date.and_hms_milli_opt(hour, min, sec, ms)?;
    let fixed = FixedOffset::east_opt(offset_secs)?;
    let dt_utc = fixed.from_local_datetime(&naive_dt).single()?.with_timezone(&Utc);

    let ts_ms = dt_utc.timestamp_millis();
    if !(-8_640_000_000_000_000..=8_640_000_000_000_000).contains(&ts_ms) {
        return None;
    }

    let utc_year = dt_utc.year();
    let year_fmt = if (0..=9999).contains(&utc_year) {
        format!("{utc_year:04}")
    } else if utc_year < 0 {
        format!("-{:06}", utc_year.unsigned_abs())
    } else {
        format!("+{utc_year:06}")
    };

    Some(format!(
        "{year_fmt}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        dt_utc.month(),
        dt_utc.day(),
        dt_utc.hour(),
        dt_utc.minute(),
        dt_utc.second(),
        dt_utc.timestamp_subsec_millis()
    ))
}

fn parse_iso_started_at(val: Option<&Value>) -> Option<String> {
    iso_date(val?.as_str()?)
}

fn parse_game(val: Option<&Value>) -> Option<StatusGame> {
    let obj = val?.as_object()?;
    let build = clean_text(obj.get("build"), MAX_BUILD_CHARS)?;
    let changelist = match obj.get("changelist") {
        Some(v) if v.is_number() => trunc_clamped_u64(Some(v), MAX_CHANGELIST).map(|n| n as u32),
        _ => None,
    };

    Some(StatusGame { build, changelist })
}

/// A server's value is refused, never clamped: a wrong time on the Middleman's screen is worse than the default.
fn parse_economy(val: Option<&Value>) -> Option<StatusEconomy> {
    let val = val.filter(|v| !v.is_null())?;
    let mut economy = StatusEconomy::default();
    let Some(obj) = val.as_object() else {
        economy.refused.push("economy".to_owned());
        return Some(economy);
    };
    match obj.get("keepSupplyStackCaps") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(keep)) => economy.keep_supply_stack_caps = Some(*keep),
        Some(_) => economy.refused.push("keepSupplyStackCaps".to_owned()),
    }
    match obj.get("cellFusionMinutes") {
        None | Some(Value::Null) => {}
        Some(v) => match v
            .as_u64()
            .and_then(|m| u32::try_from(m).ok())
            .filter(|&m| m <= MAX_CELL_FUSION_MINUTES)
        {
            Some(minutes) => economy.cell_fusion_minutes = Some(minutes),
            None => economy.refused.push("cellFusionMinutes".to_owned()),
        },
    }
    Some(economy)
}

/// Returns a copy of `status` with `limited: true` and all player/instance lists cleared.
#[must_use]
pub fn limited_view(status: ServerStatus) -> ServerStatus {
    ServerStatus {
        players_online: 0,
        players: Vec::new(),
        instances: Vec::new(),
        limited: true,
        ..status
    }
}

/// Parses and sanitizes an untrusted `ServerStatus` JSON response payload.
#[must_use]
pub fn parse_server_status(raw: &Value) -> Option<ServerStatus> {
    let obj = raw.as_object()?;
    let name = clean_text(obj.get("name"), MAX_NAME_CHARS)?;
    let online = obj.get("online").and_then(Value::as_bool).unwrap_or(false);
    let version = clean_str_or_empty(obj.get("version"), MAX_VERSION_CHARS);
    let commit = clean_str_or_empty(obj.get("commit"), MAX_COMMIT_CHARS);
    let source_url = clean_https_url(obj.get("sourceUrl"));
    let registration = parse_registration(obj.get("registration"));
    let limited = obj.get("limited").and_then(Value::as_bool).unwrap_or(false);

    let mut players = Vec::new();
    if !limited && let Some(arr) = obj.get("players").and_then(Value::as_array) {
        for item in arr {
            if players.len() >= MAX_PLAYERS_LISTED {
                break;
            }
            let Some(p_obj) = item.as_object() else {
                continue;
            };
            let Some(p_name) = clean_text(p_obj.get("name"), MAX_PLAYER_NAME_CHARS) else {
                continue;
            };
            let location = parse_player_where(p_obj.get("where"));
            let instance = clean_text(p_obj.get("instance"), MAX_INSTANCE_ID_CHARS);
            players.push(StatusPlayer {
                name: p_name,
                location,
                instance,
            });
        }
    }

    let mut instances = Vec::new();
    if !limited && let Some(arr) = obj.get("instances").and_then(Value::as_array) {
        for item in arr {
            if instances.len() >= MAX_INSTANCES_LISTED {
                break;
            }
            let Some(i_obj) = item.as_object() else {
                continue;
            };
            let Some(id) = clean_text(i_obj.get("id"), MAX_INSTANCE_ID_CHARS) else {
                continue;
            };
            let Some(kind) = parse_instance_kind(i_obj.get("kind")) else {
                continue;
            };
            let title = clean_str_or_empty(i_obj.get("title"), MAX_INSTANCE_TITLE_CHARS);
            let map = clean_str_or_empty(i_obj.get("map"), MAX_INSTANCE_MAP_CHARS);
            let behemoth = clean_text(i_obj.get("behemoth"), MAX_BEHEMOTH_CHARS);
            let inst_players = trunc_clamped_u32(i_obj.get("players"), MAX_INSTANCE_PLAYERS);
            let max_players = trunc_clamped_u32(i_obj.get("maxPlayers"), MAX_INSTANCE_PLAYERS);
            let started_at = parse_iso_started_at(i_obj.get("startedAt"));

            instances.push(StatusInstance {
                id,
                kind,
                title,
                map,
                behemoth,
                players: inst_players,
                max_players,
                started_at,
            });
        }
    }

    let players_online = if limited {
        0
    } else if obj.get("playersOnline").is_some_and(Value::is_number) {
        trunc_clamped_u32(obj.get("playersOnline"), MAX_PLAYERS_ONLINE)
    } else {
        players.len() as u32
    };

    let uptime_seconds = trunc_clamped_u64(obj.get("uptimeSeconds"), MAX_UPTIME_SECONDS).unwrap_or(0) as u32;
    let game = parse_game(obj.get("game"));
    let game_data = parse_game_data(obj.get("gameData"));
    let chat = obj.get("chat").and_then(Value::as_bool).unwrap_or(false);
    let content_port = trunc_clamped_u64(obj.get("contentPort"), 65535)
        .filter(|&p| p >= 1)
        .map(|p| p as u16);

    Some(ServerStatus {
        name,
        online,
        version,
        commit,
        source_url,
        registration,
        players_online,
        players,
        instances,
        uptime_seconds,
        game,
        game_data,
        chat,
        content_port,
        limited,
        economy: parse_economy(obj.get("economy")),
    })
}
