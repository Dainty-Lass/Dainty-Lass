//! Everything the admin API reads from outside, checked here: the bot's request (headers, body and
//! the arguments it names) and the Director's instance list. The fuzz target
//! (`fuzz/fuzz_targets/admin_request.rs`; docs/security/admin-api.md) drives these functions.

use hyper::HeaderMap;
use serde_json::Value;

use crate::upstream::Operator;

/// The biggest request body read: a query and its variables, never more.
pub const MAX_BODY: usize = 16 * 1024;
/// The header naming the Discord user an action is for, 1 to 32 digits; without it, the action is the host's.
pub const OPERATOR_HEADER: &str = "x-reforged-operator-id";
/// What reaches a ban's reason, which the player reads in the launcher.
pub const MAX_REASON: usize = 400;
/// A timeout ends within a year; longer is a ban.
pub const MAX_TIMEOUT_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// Headers that mean the request came through a proxy (the gateway among them), which nothing on
/// this machine has any reason to put in front of the admin API.
const FORWARDED: [&str; 5] = ["forwarded", "via", "x-real-ip", "x-reforged-gateway", "x-forwarded-for"];

/// Why a request is refused before any GraphQL is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// 403: through a proxy
    Forwarded,
    /// 401: no key or the wrong one
    Unauthenticated,
    /// 400: an operator id that is not 1-32 digits
    BadOperator,
    /// 400: a body that is not one GraphQL request
    BadBody,
    /// 415: not `application/json`
    NotJson,
    /// 503: the keys' database did not answer (the owner's key still works)
    KeysUnavailable,
}

impl Refusal {
    /// The HTTP status to answer with.
    pub fn status(self) -> u16 {
        match self {
            Self::Forwarded => 403,
            Self::Unauthenticated => 401,
            Self::BadOperator | Self::BadBody => 400,
            Self::NotJson => 415,
            Self::KeysUnavailable => 503,
        }
    }

    /// The `code` in the refusal's body.
    pub fn code(self) -> &'static str {
        match self {
            Self::Forwarded => "FORWARDED",
            Self::Unauthenticated => "UNAUTHENTICATED",
            Self::BadOperator => "BAD_OPERATOR",
            Self::BadBody => "BAD_REQUEST",
            Self::NotJson => "NOT_JSON",
            Self::KeysUnavailable => "KEYS_UNAVAILABLE",
        }
    }
}

fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    // A repeated header is refused, so a proxy and a client never disagree about which one counts
    if values.next().is_some() {
        return Err(());
    }
    first.to_str().map(Some).map_err(|_| ())
}

/// The key a request carries, once it is known to come straight from a caller: not forwarded, and
/// exactly one `Authorization: Bearer <key>`. Whose key it is, is `keys::Keys::authenticate`'s to say.
pub fn bearer(headers: &HeaderMap) -> Result<&str, Refusal> {
    if headers.keys().any(|name| {
        let name = name.as_str();
        FORWARDED.contains(&name) || name.starts_with("x-forwarded-")
    }) {
        return Err(Refusal::Forwarded);
    }
    let authorization = one_header(headers, "authorization").map_err(|()| Refusal::Unauthenticated)?;
    authorization
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Refusal::Unauthenticated)
}

/// The Discord user the bot names (`0`, the host, when the caller names none).
pub fn operator(headers: &HeaderMap) -> Result<Operator, Refusal> {
    match one_header(headers, OPERATOR_HEADER).map_err(|()| Refusal::BadOperator)? {
        None => Ok(Operator::host()),
        Some(id) => Operator::parse(id).ok_or(Refusal::BadOperator),
    }
}

/// Whether the one `content-type` header is `application/json`.
pub fn is_json(headers: &HeaderMap) -> bool {
    one_header(headers, "content-type").ok().flatten().is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    })
}

/// One GraphQL request (`{query, operationName?, variables?}`); a batch is refused.
pub fn parse_body(body: &[u8]) -> Result<async_graphql::Request, Refusal> {
    if body.len() > MAX_BODY {
        return Err(Refusal::BadBody);
    }
    serde_json::from_slice(body).map_err(|_| Refusal::BadBody)
}

/// A backend account id: what `/admin/v1/players?ids=` accepts, so it is also safe in a path.
pub fn is_account_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A player's name as the bot gives it: the launcher's username rule, after trimming.
pub fn username(given: &str) -> Option<&str> {
    let name = given.trim();
    dainty_protocol::account::is_valid_username(name).then_some(name)
}

/// A reason as the player will read it: one line, 3 to [`MAX_REASON`] characters.
pub fn reason(given: &str) -> Option<String> {
    let one_line: String = given.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let text = one_line.trim();
    (3..=MAX_REASON)
        .contains(&text.chars().count())
        .then(|| text.to_owned())
}

/// When a timeout ends, in ms since 1970: `minutes` from `now`, or an RFC 3339 time. Exactly one
/// is given, and the end is in the future and within [`MAX_TIMEOUT_MS`].
pub fn timeout_end(minutes: Option<i64>, until: Option<&str>, now: i64) -> Result<i64, &'static str> {
    let end = match (minutes, until) {
        (Some(minutes), None) => minutes
            .checked_mul(60_000)
            .and_then(|ms| now.checked_add(ms))
            .ok_or("minutes is out of range")?,
        (None, Some(until)) => chrono::DateTime::parse_from_rfc3339(until.trim())
            .map_err(|_| "until must be an RFC 3339 time, such as 2026-10-25T18:00:00Z")?
            .timestamp_millis(),
        _ => return Err("give either minutes or until"),
    };
    if end <= now {
        return Err("a timeout must end in the future");
    }
    if end - now > MAX_TIMEOUT_MS {
        return Err("a timeout ends within a year; use ban for longer");
    }
    Ok(end)
}

/// The most characters of a module's error message passed on to the caller.
pub const MAX_MODULE_MESSAGE: usize = 200;

/// The game a caller names (`game: "1.4.4"`): a game version as the modules declare it (DD4), shown
/// the one way it can be written.
pub fn game(given: &str) -> Option<String> {
    let text = given.trim();
    if text.len() > 32 {
        return None;
    }
    text.parse::<dainty_module_api::GameVersion>()
        .ok()
        .map(|v| v.to_string())
}

/// A module's error message as the caller reads it: one line, capped. A module out of process is
/// another program, so its text is not trusted to be short or printable.
pub fn module_message(given: &str) -> String {
    let one_line: String = given
        .chars()
        .take(MAX_MODULE_MESSAGE)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    one_line.trim().to_owned()
}

/// Unix milliseconds as RFC 3339 to the second, in UTC; empty when out of range.
pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

// ------------------------------------------------------------------ the Director's instances

/// What a Director instance runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceKind {
    /// Ramsgate.
    City,
    /// A hunt.
    Hunt,
    /// The Training Grounds.
    Dojo,
    /// The tutorial.
    Tutorial,
}

/// A game server as the Director lists it, checked and clipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// The Director's instance id.
    pub id: String,
    /// What it runs.
    pub kind: InstanceKind,
    /// "Ramsgate", "Training Grounds", or the hunt's behemoth or island.
    pub title: String,
    /// The map's name.
    pub map: String,
    /// The behemoth, for a hunt.
    pub behemoth: Option<String>,
    /// Players in it now.
    pub players: u32,
    /// The most it takes.
    pub max_players: u32,
    /// Account ids of the players in it.
    pub player_ids: Vec<String>,
    /// When it started, as the Director gives it.
    pub started_at: Option<String>,
}

fn take(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// `"/Game/Maps/islands/foo/foo_persistent"` → `foo_persistent` (`ShortAssetName` in
/// `launcherApiRules.ts`, so the bot and the launcher show the same names).
pub fn short_asset_name(asset: &str) -> String {
    let last = asset.rsplit('/').next().unwrap_or(asset);
    take(last.split('.').next().unwrap_or(last), 64)
}

/// `InstanceTitle` in `launcherApiRules.ts`.
pub fn instance_title(kind: InstanceKind, behemoth: Option<&str>, map: &str) -> String {
    match kind {
        InstanceKind::City => "Ramsgate".into(),
        InstanceKind::Dojo => "Training Grounds".into(),
        InstanceKind::Tutorial => "Tutorial".into(),
        InstanceKind::Hunt => {
            if let Some(behemoth) = behemoth {
                let name = short_asset_name(behemoth);
                let name = name
                    .strip_prefix("BP_")
                    .or_else(|| name.strip_prefix("BPC_"))
                    .unwrap_or(&name);
                let name = name.strip_suffix("_C").unwrap_or(name);
                return take(&name.replace('_', " "), 64);
            }
            let name = short_asset_name(map);
            let cut = name.len().saturating_sub("_persistent".len());
            let name = match name.get(cut..) {
                Some(end) if end.eq_ignore_ascii_case("_persistent") => &name[..cut],
                _ => name.as_str(),
            };
            let title = take(&name.replace('_', " "), 64);
            if title.is_empty() { "Hunt".into() } else { title }
        }
    }
}

fn count(value: Option<&Value>) -> u32 {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .map_or(0, |n| n.max(0.0).min(f64::from(u32::MAX)).floor() as u32)
}

/// The Director's `GET /api/instances`, checked field by field as `ParseDirectorInstances` does: an
/// odd entry is skipped, never an error. Players are listed for every kind, not only hunts.
pub fn parse_instances(raw: &Value) -> Vec<Instance> {
    let Some(entries) = raw.as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .take(100)
        .filter_map(|entry| {
            let id = entry
                .get("id")?
                .as_str()
                .map(|id| take(id, 64))
                .filter(|id| !id.is_empty())?;
            let kind = match entry.get("kind")?.as_str()? {
                "city" => InstanceKind::City,
                "hunt" => InstanceKind::Hunt,
                "dojo" => InstanceKind::Dojo,
                "tutorial" => InstanceKind::Tutorial,
                _ => return None,
            };
            let map = entry
                .get("map")
                .and_then(Value::as_str)
                .map(|m| take(m, 160))
                .unwrap_or_default();
            let behemoth = entry.get("behemoth").and_then(Value::as_str).map(|b| take(b, 160));
            let player_ids = entry
                .get("playerIds")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .take(16)
                        .filter_map(Value::as_str)
                        .filter(|id| id.len() <= 128)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let started_at = entry
                .get("startedAt")
                .and_then(Value::as_str)
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| iso(t.timestamp_millis()));
            Some(Instance {
                title: instance_title(kind, behemoth.as_deref(), &map),
                map: short_asset_name(&map),
                behemoth: behemoth.as_deref().map(short_asset_name),
                players: count(entry.get("players")),
                max_players: count(entry.get("maxPlayers")),
                id,
                kind,
                player_ids,
                started_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::HeaderValue;
    use serde_json::json;

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    /// The header checks in the order the HTTP side runs them (the key is `keys::Keys`'s to check).
    fn check_headers(headers: &HeaderMap, key: &str) -> Result<Operator, Refusal> {
        if bearer(headers)? != key {
            return Err(Refusal::Unauthenticated);
        }
        operator(headers)
    }

    #[test]
    fn a_request_carries_one_bearer_key_and_no_proxy_in_between() {
        let bearer = format!("Bearer {KEY}");
        assert_eq!(
            check_headers(&headers(&[("authorization", &bearer)]), KEY).map(|o| o.id().to_owned()),
            Ok("0".into())
        );
        assert_eq!(
            check_headers(&headers(&[("authorization", &bearer), (OPERATOR_HEADER, "42")]), KEY)
                .map(|o| o.id().to_owned()),
            Ok("42".into())
        );
        for (pairs, refusal) in [
            (vec![], Refusal::Unauthenticated),
            (vec![("authorization", KEY)], Refusal::Unauthenticated),
            (vec![("authorization", "Bearer nope")], Refusal::Unauthenticated),
            (
                vec![("authorization", bearer.as_str()), ("authorization", bearer.as_str())],
                Refusal::Unauthenticated,
            ),
            (
                vec![("authorization", bearer.as_str()), ("x-forwarded-for", "1.2.3.4")],
                Refusal::Forwarded,
            ),
            (
                vec![("authorization", bearer.as_str()), ("x-forwarded-host", "a")],
                Refusal::Forwarded,
            ),
            (
                vec![("authorization", bearer.as_str()), ("x-reforged-gateway", "s")],
                Refusal::Forwarded,
            ),
            (
                vec![("authorization", bearer.as_str()), ("forwarded", "for=a")],
                Refusal::Forwarded,
            ),
            (
                vec![("authorization", bearer.as_str()), (OPERATOR_HEADER, "abc")],
                Refusal::BadOperator,
            ),
        ] {
            assert_eq!(
                check_headers(&headers(&pairs), KEY).map(|_| ()),
                Err(refusal),
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn the_body_is_one_json_request() {
        assert!(is_json(&headers(&[(
            "content-type",
            "application/json; charset=utf-8"
        )])));
        assert!(!is_json(&headers(&[("content-type", "text/plain")])));
        assert!(!is_json(&headers(&[])));
        assert!(parse_body(br#"{"query":"{ serverStatus { name } }"}"#).is_ok());
        assert_eq!(parse_body(br#"[{"query":"{ a }"}]"#).map(|_| ()), Err(Refusal::BadBody));
        assert_eq!(parse_body(b"nope").map(|_| ()), Err(Refusal::BadBody));
        let big = format!(r#"{{"query":"{}"}}"#, "a".repeat(MAX_BODY));
        assert_eq!(parse_body(big.as_bytes()).map(|_| ()), Err(Refusal::BadBody));
    }

    #[test]
    fn names_ids_and_reasons() {
        assert_eq!(username(" Slayer_01 "), Some("Slayer_01"));
        for bad in ["ab", "bad name", "x/../y", &"a".repeat(17)] {
            assert_eq!(username(bad), None, "{bad}");
        }
        assert!(is_account_id("0b9d6c1e-2f4a-4c5d-8e7f-0123456789ab"));
        assert!(!is_account_id("a/b") && !is_account_id("") && !is_account_id(&"a".repeat(65)));
        assert_eq!(reason(" spamming\nchat ").as_deref(), Some("spamming chat"));
        assert_eq!(reason("no"), None);
        assert_eq!(reason(&"x".repeat(MAX_REASON + 1)), None);
    }

    #[test]
    fn games_and_module_messages() {
        assert_eq!(game(" 1.4.4 ").as_deref(), Some("1.4.4"));
        assert_eq!(game("1.14.7").as_deref(), Some("1.14.7"));
        for bad in ["", "1.04.4", "v1.4", "1..4", "reforged", &"1.".repeat(20)] {
            assert_eq!(game(bad), None, "{bad}");
        }
        assert_eq!(module_message(" no\r\nsuch\tplayer "), "no  such player");
        assert_eq!(
            module_message(&"x".repeat(MAX_MODULE_MESSAGE * 2)).chars().count(),
            MAX_MODULE_MESSAGE
        );
    }

    #[test]
    fn a_timeout_ends_in_the_future_and_within_a_year() {
        let now = 1_800_000_000_000;
        assert_eq!(timeout_end(Some(30), None, now), Ok(now + 30 * 60_000));
        // now is 2027-01-15T08:00:00Z
        assert_eq!(
            timeout_end(None, Some("2027-01-16T08:00:00Z"), now),
            Ok(now + 86_400_000)
        );
        assert!(timeout_end(None, Some("2027-01-15T08:00:00Z"), now).is_err());
        assert!(timeout_end(Some(0), None, now).is_err());
        assert!(timeout_end(Some(-5), None, now).is_err());
        assert!(timeout_end(Some(i64::MAX), None, now).is_err());
        assert!(timeout_end(Some(366 * 24 * 60), None, now).is_err());
        assert!(timeout_end(Some(5), Some("2027-01-16T08:00:00Z"), now).is_err());
        assert!(timeout_end(None, None, now).is_err());
        assert!(timeout_end(None, Some("tomorrow"), now).is_err());
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn director_instances_are_checked_field_by_field() {
        let instances = parse_instances(&json!([
            {"id": "city-8777", "kind": "city", "map": "/Game/Maps/ramsgate/ramsgate_01_persistent", "behemoth": null,
             "players": 3, "maxPlayers": 0, "playerIds": ["u3"], "startedAt": "2026-10-01T10:00:00.000Z"},
            {"id": "hunt-8770", "kind": "hunt", "map": "/Game/Maps/islands/a/IslandA_persistent",
             "behemoth": "/Game/Behemoths/BP_Skarn.BP_Skarn_C", "players": 2.7, "maxPlayers": 4,
             "playerIds": ["u1", "u2", 7], "startedAt": "nope"},
            {"id": "hunt-8769", "kind": "hunt", "map": "/Game/Maps/islands/b/Island_B_Persistent", "players": -1},
            {"id": "x", "kind": "spaceship"},
            {"kind": "hunt"},
            "junk",
            null
        ]));
        assert_eq!(instances.len(), 3);
        assert_eq!(instances[0].title, "Ramsgate");
        assert_eq!(instances[0].map, "ramsgate_01_persistent");
        assert_eq!(instances[0].player_ids, ["u3"]);
        assert_eq!(instances[0].started_at.as_deref(), Some("2026-10-01T10:00:00Z"));
        assert_eq!(instances[1].title, "Skarn");
        assert_eq!(instances[1].behemoth.as_deref(), Some("BP_Skarn"));
        assert_eq!((instances[1].players, instances[1].max_players), (2, 4));
        assert_eq!(instances[1].player_ids, ["u1", "u2"]);
        assert_eq!(instances[1].started_at, None);
        assert_eq!(instances[2].title, "Island B");
        assert_eq!(instances[2].players, 0);
        assert!(parse_instances(&json!({"not": "a list"})).is_empty());
    }
}
