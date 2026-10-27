//! `launcherApiRules.ts`: the rules of the Reforged API, kept apart from the database and the settings. The contract
//! is the launcher's (net/hostapi), the content server's auth and the admin service's upstream.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use super::util::{constant_time_eq, random_bytes, random_int, sha256_hex};

pub const USER_KEY_HEADER: &str = "x-reforged-user-api-key";
pub const ADMIN_KEY_HEADER: &str = "x-reforged-admin-key";
/// The gateway stamps every request it forwards; whatever the socket says, a request carrying it came from outside.
pub const GATEWAY_HEADER: &str = "x-reforged-gateway";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationMode {
    Open,
    InviteCode,
    None,
}

impl RegistrationMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::InviteCode => "INVITECODE",
            Self::None => "NONE",
        }
    }

    /// `REGISTRATION_MODE`: OPEN or NONE when it says so; invite codes otherwise, unset included.
    pub fn from_setting(raw: Option<&str>) -> Self {
        match raw.unwrap_or_default().trim().to_uppercase().as_str() {
            "OPEN" => Self::Open,
            "NONE" => Self::None,
            _ => Self::InviteCode,
        }
    }
}

pub const USERNAME_MIN: usize = 3;
pub const USERNAME_MAX: usize = 16;

/// `NormalizeUsername`: trimmed, 3 to 16 letters, digits and underscores.
pub fn normalize_username(raw: Option<&Value>) -> Option<String> {
    let Some(Value::String(text)) = raw else {
        return None;
    };
    let name = service_kit::js::trim(text);
    let length = name.encode_utf16().count();
    let valid =
        (USERNAME_MIN..=USERNAME_MAX).contains(&length) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    valid.then(|| name.to_owned())
}

// No 0 and O, no 1, I and L
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// `MakeInviteCode`: three groups of four, about 60 random bits.
pub fn make_invite_code() -> String {
    make_invite_code_with(&mut random_int)
}

pub fn make_invite_code_with(pick: &mut dyn FnMut(u32) -> u32) -> String {
    (0..3)
        .map(|_| {
            (0..4)
                .map(|_| char::from(CODE_ALPHABET[pick(CODE_ALPHABET.len() as u32) as usize]))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// `IsInviteCode`: anything a host could have typed, 4 to 64 letters, digits and dashes.
pub fn is_invite_code(raw: &str) -> bool {
    (4..=64).contains(&raw.len()) && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub const MAX_INVITE_USES: i64 = 1000;

/// `InviteNoteFrom`: control characters become spaces, then trimmed and capped at 64.
pub fn invite_note(raw: Option<&Value>) -> Option<String> {
    let Some(Value::String(text)) = raw else {
        return None;
    };
    let spaced: String = text
        .chars()
        .map(|c| {
            if c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let note: String = service_kit::js::trim(&spaced).chars().take(64).collect();
    (!note.is_empty()).then_some(note)
}

pub const ACCOUNT_KEY_PREFIX: &str = "RRK_";

/// `MakeAccountKey`: the prefix and 48 hex digits (192 random bits).
pub fn make_account_key() -> String {
    format!("{ACCOUNT_KEY_PREFIX}{}", super::util::hex(&random_bytes(24)))
}

/// `LooksLikeAccountKey`: worth looking up at all.
pub fn looks_like_account_key(raw: &str) -> bool {
    (8..=128).contains(&raw.len()) && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    BadRequest,
    InviteInvalid,
    NotFound,
    RegistrationClosed,
    UsernameInvalid,
    UsernameTaken,
}

impl Refusal {
    pub const fn code(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::InviteInvalid => "invite_invalid",
            Self::NotFound => "not_found",
            Self::RegistrationClosed => "registration_closed",
            Self::UsernameInvalid => "username_invalid",
            Self::UsernameTaken => "username_taken",
        }
    }

    pub fn message(self) -> String {
        match self {
            Self::BadRequest => "The body is not JSON, or a field is missing or has the wrong type.".into(),
            Self::InviteInvalid => "That invite code is not valid, or has been used up.".into(),
            Self::NotFound => "No account has that name.".into(),
            Self::RegistrationClosed => "This server is not taking registrations.".into(),
            Self::UsernameInvalid => {
                format!("A username is {USERNAME_MIN} to {USERNAME_MAX} characters: letters, digits and underscore.")
            }
            Self::UsernameTaken => "That name is taken.".into(),
        }
    }

    /// `{error, message}`: the code the launcher maps to its own text, and a sentence for anyone else.
    pub fn body(self) -> Value {
        json!({ "error": self.code(), "message": self.message() })
    }
}

/// `IsLocalAdminCall`: a loopback socket, nothing forwarded (no gateway stamp, no X-Forwarded-For), and the
/// host's key.
pub fn is_local_admin_call(
    headers: &axum::http::HeaderMap,
    remote_address: Option<&str>,
    admin_key: Option<&str>,
) -> bool {
    let Some(admin_key) = admin_key.filter(|k| k.encode_utf16().count() >= 32) else {
        return false;
    };
    let loopback = remote_address.is_some_and(|a| {
        a == "::1"
            || a.strip_prefix("::ffff:")
                .unwrap_or(a)
                .split('.')
                .collect::<Vec<_>>()
                .len()
                == 4
                && a.strip_prefix("::ffff:").unwrap_or(a).starts_with("127.")
                && a.strip_prefix("::ffff:")
                    .unwrap_or(a)
                    .split('.')
                    .all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
    });
    if !loopback || headers.contains_key(GATEWAY_HEADER) || headers.contains_key("x-forwarded-for") {
        return false;
    }
    let Some(given) = headers.get(ADMIN_KEY_HEADER).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    constant_time_eq(
        sha256_hex(given.as_bytes()).as_bytes(),
        sha256_hex(admin_key.as_bytes()).as_bytes(),
    )
}

// ---------------------------------------------------------------- the status

/// `AssetLeaf`: "/Game/Maps/islands/foo/foo_persistent" is "foo_persistent"; "/Game/Behemoths/Skarn.Skarn_C" is "Skarn".
pub fn asset_leaf(asset: &str) -> String {
    let last = &asset[asset.rfind('/').map_or(0, |i| i + 1)..];
    last.split('.').next().unwrap_or("").chars().take(64).collect()
}

const INSTANCE_KINDS: [&str; 4] = ["city", "hunt", "dojo", "tutorial"];
const MAX_INSTANCES: usize = 100;
const MAX_PLAYERS_PER_HUNT: usize = 16;

fn text(value: Option<&Value>, max: usize) -> Option<String> {
    match value {
        Some(Value::String(s)) => Some(s.chars().take(max).collect()),
        _ => None,
    }
}

/// `Count`: a finite positive number, floored; 0 otherwise.
fn count(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|f| f.is_finite() && *f > 0.0)
            .map_or(0, |f| f.floor() as u64),
        _ => 0,
    }
}

fn iso_date(value: Option<&Value>) -> Option<String> {
    let Some(Value::String(s)) = value else {
        return None;
    };
    super::util::date_parse(s).map(super::util::iso)
}

/// What the launcher lists an instance as: hunts are named after the behemoth, else the map.
pub fn instance_title(kind: &str, behemoth: Option<&str>, map: &str) -> String {
    match kind {
        "city" => return "Ramsgate".into(),
        "dojo" => return "Training Grounds".into(),
        "tutorial" => return "Tutorial".into(),
        _ => {}
    }
    if let Some(behemoth) = behemoth.filter(|b| !b.is_empty()) {
        let leaf = asset_leaf(behemoth);
        let leaf = strip_blueprint_prefix(&leaf);
        let leaf = leaf.strip_suffix("_C").unwrap_or(leaf);
        return leaf.replace('_', " ").chars().take(64).collect();
    }
    let leaf = asset_leaf(map);
    let lower = leaf.to_lowercase();
    let leaf = if lower.ends_with("_persistent") {
        &leaf[..leaf.len() - "_persistent".len()]
    } else {
        &leaf
    };
    let from_map: String = leaf.replace('_', " ").chars().take(64).collect();
    if from_map.is_empty() { "Hunt".into() } else { from_map }
}

/// `/^BPC?_/`
fn strip_blueprint_prefix(text: &str) -> &str {
    text.strip_prefix("BPC_")
        .or_else(|| text.strip_prefix("BP_"))
        .unwrap_or(text)
}

/// The Director's `GET /api/instances` read field by field: one odd entry must never break the status. Returns the
/// instances and which hunt each player id is in.
pub fn status_instances(raw: &Value) -> (Vec<Value>, HashMap<String, String>) {
    let mut instances = Vec::new();
    let mut hunt_of = HashMap::new();
    let Value::Array(entries) = raw else {
        return (instances, hunt_of);
    };
    for entry in entries.iter().take(MAX_INSTANCES) {
        let Value::Object(item) = entry else {
            continue;
        };
        let Some(id) = text(item.get("id"), 64).filter(|id| !id.is_empty()) else {
            continue;
        };
        let Some(kind) = INSTANCE_KINDS
            .iter()
            .find(|known| item.get("kind") == Some(&Value::String((**known).to_owned())))
        else {
            continue;
        };
        let map = text(item.get("map"), 160).unwrap_or_default();
        let behemoth = text(item.get("behemoth"), 160);
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        out.insert("kind".into(), json!(kind));
        out.insert("title".into(), json!(instance_title(kind, behemoth.as_deref(), &map)));
        out.insert("map".into(), json!(asset_leaf(&map)));
        out.insert(
            "behemoth".into(),
            behemoth.as_deref().map_or(Value::Null, |b| json!(asset_leaf(b))),
        );
        out.insert("players".into(), json!(count(item.get("players"))));
        out.insert("maxPlayers".into(), json!(count(item.get("maxPlayers"))));
        out.insert(
            "startedAt".into(),
            iso_date(item.get("startedAt")).map_or(Value::Null, Value::from),
        );
        instances.push(Value::Object(out));
        if *kind == "hunt"
            && let Some(Value::Array(ids)) = item.get("playerIds")
        {
            for player in ids.iter().take(MAX_PLAYERS_PER_HUNT) {
                if let Value::String(player) = player
                    && player.encode_utf16().count() <= 128
                {
                    hunt_of.insert(player.clone(), id.clone());
                }
            }
        }
    }
    (instances, hunt_of)
}

// ---------------------------------------------------------------- game data

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataKind {
    Real,
    Synthetic,
    Missing,
}

impl GameDataKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Synthetic => "synthetic",
            Self::Missing => "missing",
        }
    }
}

pub const GAME_DATA_FILES: [&str; 7] = [
    "progression_config.json",
    "slayers_path.json",
    "slayers_path_definitions.json",
    "ladyluck_store.json",
    "platinum_store.json",
    "reward_cache_store.json",
    "inventory_storage_policy.json",
];

/// `ClassifyGameData`: every required file there and none the shipped example is "real"; examples in use are
/// "synthetic"; any file missing is "missing".
pub fn classify_game_data(dir: &Path, files: &[&str]) -> GameDataKind {
    let mut any_example = false;
    for name in files {
        let file = dir.join(name);
        if !file.exists() {
            return GameDataKind::Missing;
        }
        let example = dir.join(name.trim_end_matches(".json").to_owned() + ".example.json");
        match (std::fs::read(&file), std::fs::read(&example)) {
            (Ok(a), Ok(b)) if a == b => any_example = true,
            _ => {}
        }
    }
    if any_example {
        GameDataKind::Synthetic
    } else {
        GameDataKind::Real
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Value {
        Value::String(text.to_owned())
    }

    #[test]
    fn registration_mode_defaults_to_invite_codes() {
        use RegistrationMode::*;
        assert_eq!(RegistrationMode::from_setting(Some(" open ")), Open);
        assert_eq!(RegistrationMode::from_setting(Some("none")), None);
        assert_eq!(RegistrationMode::from_setting(Some("whatever")), InviteCode);
        assert_eq!(RegistrationMode::from_setting(Option::None), InviteCode);
    }

    #[test]
    fn usernames_are_trimmed_and_checked() {
        assert_eq!(normalize_username(Some(&s("  Fox_9 "))).as_deref(), Some("Fox_9"));
        assert_eq!(normalize_username(Some(&s("ab"))), None);
        assert_eq!(normalize_username(Some(&s("abcdefghijklmnopq"))), None);
        assert_eq!(normalize_username(Some(&s("a b c"))), None);
        assert_eq!(normalize_username(Some(&s("née_ab"))), None);
        assert_eq!(normalize_username(Some(&json!(5))), None);
        assert_eq!(normalize_username(Option::None), None);
    }

    #[test]
    fn invite_codes_use_the_unambiguous_alphabet() {
        let mut next = 0u32;
        let code = make_invite_code_with(&mut |bound| {
            next += 1;
            (next - 1) % bound
        });
        assert_eq!(code, "ABCD-EFGH-JKMN");
        let code = make_invite_code();
        assert_eq!(code.len(), 14);
        assert!(code.chars().all(|c| c == '-' || CODE_ALPHABET.contains(&(c as u8))));
        assert!(
            is_invite_code(&code) && is_invite_code("abc-1") && !is_invite_code("abc") && !is_invite_code("a b c d")
        );
        assert!(!is_invite_code(&"A".repeat(65)));
    }

    #[test]
    fn notes_lose_control_characters() {
        assert_eq!(invite_note(Some(&s("  for\u{0}Alex\n"))).as_deref(), Some("for Alex"));
        assert_eq!(invite_note(Some(&s("   "))), None);
        assert_eq!(invite_note(Some(&s(&"x".repeat(100)))).map(|n| n.len()), Some(64));
        assert_eq!(invite_note(Some(&json!(1))), None);
    }

    #[test]
    fn account_keys_have_the_prefix_and_192_bits() {
        let key = make_account_key();
        assert!(key.starts_with("RRK_") && key.len() == 4 + 48);
        assert!(
            looks_like_account_key(&key)
                && !looks_like_account_key("short")
                && !looks_like_account_key("a b c d e f g")
        );
    }

    #[test]
    fn refusals_carry_a_code_and_a_sentence() {
        assert_eq!(
            Refusal::UsernameInvalid.body().to_string(),
            r#"{"error":"username_invalid","message":"A username is 3 to 16 characters: letters, digits and underscore."}"#
        );
        assert_eq!(Refusal::NotFound.message(), "No account has that name.");
    }

    #[test]
    fn a_local_admin_call_needs_everything() {
        let key = "k".repeat(40);
        let headers = |pairs: &[(&str, &str)]| {
            let mut map = axum::http::HeaderMap::new();
            for (n, v) in pairs {
                map.insert(
                    axum::http::HeaderName::from_bytes(n.as_bytes()).unwrap_or_else(|_| unreachable!()),
                    v.parse().unwrap_or_else(|_| unreachable!()),
                );
            }
            map
        };
        let good = headers(&[(ADMIN_KEY_HEADER, &key)]);
        assert!(is_local_admin_call(&good, Some("127.0.0.1"), Some(&key)));
        assert!(is_local_admin_call(&good, Some("::ffff:127.0.0.1"), Some(&key)));
        assert!(is_local_admin_call(&good, Some("::1"), Some(&key)));
        assert!(!is_local_admin_call(&good, Some("203.0.113.9"), Some(&key)));
        assert!(!is_local_admin_call(&good, None, Some(&key)));
        assert!(!is_local_admin_call(&good, Some("127.0.0.1"), Some("short")));
        assert!(!is_local_admin_call(&good, Some("127.0.0.1"), None));
        assert!(!is_local_admin_call(
            &headers(&[(ADMIN_KEY_HEADER, "wrong")]),
            Some("127.0.0.1"),
            Some(&key)
        ));
        assert!(!is_local_admin_call(&headers(&[]), Some("127.0.0.1"), Some(&key)));
        let stamped = headers(&[(ADMIN_KEY_HEADER, &key), (GATEWAY_HEADER, "1")]);
        assert!(!is_local_admin_call(&stamped, Some("127.0.0.1"), Some(&key)));
        let forwarded = headers(&[(ADMIN_KEY_HEADER, &key), ("x-forwarded-for", "1.2.3.4")]);
        assert!(!is_local_admin_call(&forwarded, Some("127.0.0.1"), Some(&key)));
    }

    #[test]
    fn instances_are_read_field_by_field() {
        let raw = json!([
            {"id": "h1", "kind": "hunt", "map": "/Game/Maps/islands/foo/foo_persistent", "behemoth": "/Game/Behemoths/BPC_Skarn_Prime.BPC_Skarn_Prime_C",
             "players": 3.7, "maxPlayers": 5, "startedAt": "2026-10-20T00:00:00Z", "playerIds": ["a", 7, "b"]},
            {"id": "c1", "kind": "city", "players": -2},
            {"id": "x", "kind": "mystery"},
            "junk",
            {"kind": "hunt"},
            {"id": "h2", "kind": "hunt", "map": "/Game/Maps/Marsh_Of_Fog_persistent.Marsh"}
        ]);
        let (instances, hunt_of) = status_instances(&raw);
        assert_eq!(instances.len(), 3);
        assert_eq!(
            instances[0].to_string(),
            r#"{"id":"h1","kind":"hunt","title":"Skarn Prime","map":"foo_persistent","behemoth":"BPC_Skarn_Prime","players":3,"maxPlayers":5,"startedAt":"2026-10-20T00:00:00.000Z"}"#
        );
        assert_eq!(instances[1]["title"], "Ramsgate");
        assert_eq!(instances[1]["players"], 0);
        assert_eq!(instances[1]["startedAt"], Value::Null);
        assert_eq!(instances[2]["title"], "Marsh Of Fog");
        assert_eq!(hunt_of.get("a").map(String::as_str), Some("h1"));
        assert_eq!(hunt_of.len(), 2);
        assert!(status_instances(&json!({"not": "a list"})).0.is_empty());
        assert_eq!(instance_title("hunt", None, ""), "Hunt");
    }

    #[test]
    fn game_data_is_classified_by_its_files() {
        let dir = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let write = |name: &str, body: &str| std::fs::write(dir.path().join(name), body).unwrap_or_default();
        assert_eq!(
            classify_game_data(dir.path(), &["a.json", "b.json"]),
            GameDataKind::Missing
        );
        write("a.json", "1");
        write("b.json", "2");
        assert_eq!(
            classify_game_data(dir.path(), &["a.json", "b.json"]),
            GameDataKind::Real
        );
        write("b.example.json", "2");
        assert_eq!(
            classify_game_data(dir.path(), &["a.json", "b.json"]),
            GameDataKind::Synthetic
        );
        write("b.example.json", "different");
        assert_eq!(
            classify_game_data(dir.path(), &["a.json", "b.json"]),
            GameDataKind::Real
        );
    }
}
