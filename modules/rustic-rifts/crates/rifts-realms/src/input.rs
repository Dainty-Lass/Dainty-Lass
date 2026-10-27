//! Strict parsers for what a realms route reads (`realms/realmInput.ts`). Types are checked, never coerced; only
//! the fields a route names are copied out. Bodies arrive parsed (`JSON.parse`); query strings as text.

use serde_json::{Map, Value};

use crate::realm::{
    HOSTING_MAX_AFK_SECONDS, HOSTING_MAX_PLAYERS_LIMIT, HOSTING_MAX_SERVERS_LIMIT, INVITE_DEFAULT_MAX_USES,
    INVITE_DEFAULT_TTL_HOURS, INVITE_MAX_TTL_HOURS, INVITE_MAX_USES_LIMIT, JoinPolicy, NameRefusal, js_trim,
    normalize_invite_code, validate_realm_name,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    /// `REALM_VALIDATION_FAILED`, `REALM_NAME_INVALID` or `REALM_NAME_RESERVED`.
    pub code: &'static str,
    pub message: String,
}

pub type Parsed<T> = Result<T, ParseFailure>;

fn fail<T>(message: impl Into<String>) -> Parsed<T> {
    Err(ParseFailure {
        code: "REALM_VALIDATION_FAILED",
        message: message.into(),
    })
}

fn fail_code<T>(code: &'static str, message: &str) -> Parsed<T> {
    Err(ParseFailure {
        code,
        message: message.to_owned(),
    })
}

/// A JSON object body; none counts as an empty object, anything else is refused.
fn as_object(body: Option<&Value>) -> Option<Map<String, Value>> {
    match body {
        None | Some(Value::Null) => Some(Map::new()),
        Some(Value::Object(map)) => Some(map.clone()),
        Some(_) => None,
    }
}

/// `Number.isSafeInteger`, which also takes `5.0`.
pub fn whole_number(value: &Value) -> Option<i64> {
    const SAFE: i64 = 9_007_199_254_740_991;
    let Value::Number(number) = value else {
        return None;
    };
    if let Some(n) = number.as_i64() {
        return (-SAFE..=SAFE).contains(&n).then_some(n);
    }
    let f = number.as_f64()?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let n = f as i64;
    (f.fract() == 0.0 && (-SAFE..=SAFE).contains(&n) && n as f64 == f).then_some(n)
}

fn host_key_shaped(key: &str) -> bool {
    (16..=512).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'_' | b'=' | b'-'))
}

fn parse_join_policy(value: &Value) -> Option<JoinPolicy> {
    value.as_str().and_then(JoinPolicy::parse)
}

/// The host launcher's public key as text (base64, base64url or hex): alphabet and size only.
pub fn parse_host_public_key(value: &Value) -> Option<String> {
    let key = js_trim(value.as_str()?);
    host_key_shaped(key).then(|| key.to_owned())
}

pub fn parse_version(value: &Value) -> Option<String> {
    let version = js_trim(value.as_str()?);
    let ok = (1..=64).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'));
    ok.then(|| version.to_owned())
}

pub fn parse_account_id_param(value: &str) -> Option<String> {
    let ok = (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
    ok.then(|| value.to_owned())
}

/// Node's `Has`: present and not undefined. JSON has no undefined, so present.
fn get<'a>(source: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    source.get(key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRealmInput {
    /// Absent: the realm takes the host's username.
    pub name: Option<String>,
    pub host_public_key: String,
    pub join_policy: JoinPolicy,
    pub listed: bool,
    pub version: Option<String>,
}

/// `POST /launcher/v1/realms`: invite-only and unlisted by default.
pub fn parse_create_realm_body(body: Option<&Value>) -> Parsed<CreateRealmInput> {
    let Some(source) = as_object(body) else {
        return fail("The request body must be a JSON object.");
    };
    let mut name = None;
    if let Some(raw) = get(&source, "name") {
        match raw.as_str().map(validate_realm_name) {
            Some(Ok(valid)) => name = Some(valid),
            Some(Err(NameRefusal::Reserved)) => return fail_code("REALM_NAME_RESERVED", "That realm name is reserved."),
            _ => {
                return fail_code(
                    "REALM_NAME_INVALID",
                    "A realm name is 3-24 letters, digits or hyphens, and cannot start or end with a hyphen.",
                );
            }
        }
    }
    let Some(host_public_key) = source.get("hostPublicKey").and_then(parse_host_public_key) else {
        return fail("hostPublicKey must be the host's public key as text.");
    };
    let mut join_policy = JoinPolicy::InviteOnly;
    if let Some(raw) = get(&source, "joinPolicy") {
        let Some(parsed) = parse_join_policy(raw) else {
            return fail("joinPolicy must be SOLO, INVITE_ONLY or PUBLIC.");
        };
        join_policy = parsed;
    }
    let mut listed = false;
    if let Some(raw) = get(&source, "listed") {
        let Some(value) = raw.as_bool() else {
            return fail("listed must be true or false.");
        };
        listed = value;
    }
    let mut version = None;
    if let Some(raw) = get(&source, "version") {
        let Some(parsed) = parse_version(raw) else {
            return fail("version is malformed.");
        };
        version = Some(parsed);
    }
    Ok(CreateRealmInput {
        name,
        host_public_key,
        join_policy,
        listed,
        version,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsInput {
    pub join_policy: Option<JoinPolicy>,
    pub listed: Option<bool>,
}

/// `POST .../mine/settings`: at least one setting.
pub fn parse_settings_body(body: Option<&Value>) -> Parsed<SettingsInput> {
    let Some(source) = as_object(body) else {
        return fail("The request body must be a JSON object.");
    };
    let mut input = SettingsInput::default();
    if let Some(raw) = get(&source, "joinPolicy") {
        let Some(parsed) = parse_join_policy(raw) else {
            return fail("joinPolicy must be SOLO, INVITE_ONLY or PUBLIC.");
        };
        input.join_policy = Some(parsed);
    }
    if let Some(raw) = get(&source, "listed") {
        let Some(value) = raw.as_bool() else {
            return fail("listed must be true or false.");
        };
        input.listed = Some(value);
    }
    if input.join_policy.is_none() && input.listed.is_none() {
        return fail("Nothing to change: send joinPolicy and/or listed.");
    }
    Ok(input)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteInput {
    pub max_uses: i64,
    pub ttl_hours: i64,
}

/// `POST .../mine/invites`: maxUses 1-100 (10), expiresInHours 1-720 (168).
pub fn parse_invite_body(body: Option<&Value>) -> Parsed<InviteInput> {
    let Some(source) = as_object(body) else {
        return fail("The request body must be a JSON object.");
    };
    let mut input = InviteInput {
        max_uses: INVITE_DEFAULT_MAX_USES,
        ttl_hours: INVITE_DEFAULT_TTL_HOURS,
    };
    if let Some(raw) = get(&source, "maxUses") {
        match whole_number(raw).filter(|n| (1..=INVITE_MAX_USES_LIMIT).contains(n)) {
            Some(n) => input.max_uses = n,
            None => return fail(format!("maxUses must be a whole number from 1 to {INVITE_MAX_USES_LIMIT}.")),
        }
    }
    if let Some(raw) = get(&source, "expiresInHours") {
        match whole_number(raw).filter(|n| (1..=INVITE_MAX_TTL_HOURS).contains(n)) {
            Some(n) => input.ttl_hours = n,
            None => return fail(format!("expiresInHours must be a whole number from 1 to {INVITE_MAX_TTL_HOURS}.")),
        }
    }
    Ok(input)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JoinInput {
    pub name: Option<String>,
    pub invite_code: Option<String>,
    pub known_host_key: Option<String>,
}

/// A name a join or a lookup reads: a well-formed reserved name passes, since it names an existing realm.
fn existing_name(raw: &Value) -> Option<String> {
    let text = raw.as_str()?;
    match validate_realm_name(text) {
        Ok(name) => Some(name),
        Err(NameRefusal::Reserved) => Some(js_trim(text).to_lowercase()),
        Err(NameRefusal::Invalid) => None,
    }
}

/// `POST /launcher/v1/realms/join`: a name and/or an invite code.
pub fn parse_join_body(body: Option<&Value>) -> Parsed<JoinInput> {
    let Some(source) = as_object(body) else {
        return fail("The request body must be a JSON object.");
    };
    let mut input = JoinInput::default();
    if let Some(raw) = get(&source, "name") {
        match existing_name(raw) {
            Some(name) => input.name = Some(name),
            None => return fail_code("REALM_NAME_INVALID", "That is not a realm name."),
        }
    }
    if let Some(raw) = get(&source, "inviteCode") {
        match raw.as_str().and_then(normalize_invite_code) {
            Some(code) => input.invite_code = Some(code),
            None => return fail("That is not an invite code."),
        }
    }
    if input.name.is_none() && input.invite_code.is_none() {
        return fail("Send a realm name and/or an invite code.");
    }
    if let Some(raw) = get(&source, "knownHostKey") {
        match parse_host_public_key(raw) {
            Some(key) => input.known_host_key = Some(key),
            None => return fail("knownHostKey is malformed."),
        }
    }
    Ok(input)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostingInput {
    pub max_servers: i64,
    pub max_players: i64,
    pub afk_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeartbeatInput {
    pub online: Option<bool>,
    pub version: Option<String>,
    pub hosting: Option<HostingInput>,
}

fn parse_hosting_input(value: &Value) -> Parsed<HostingInput> {
    let Value::Object(source) = value else {
        return fail("hosting must be an object.");
    };
    let number = |key: &str, min: i64, max: i64| {
        source
            .get(key)
            .and_then(whole_number)
            .filter(|n| (min..=max).contains(n))
    };
    let Some(max_servers) = number("maxServers", 1, HOSTING_MAX_SERVERS_LIMIT) else {
        return fail(format!("hosting.maxServers must be a whole number from 1 to {HOSTING_MAX_SERVERS_LIMIT}."));
    };
    let Some(max_players) = number("maxPlayers", 1, HOSTING_MAX_PLAYERS_LIMIT) else {
        return fail(format!("hosting.maxPlayers must be a whole number from 1 to {HOSTING_MAX_PLAYERS_LIMIT}."));
    };
    let mut afk_seconds = None;
    if source.contains_key("afkSeconds") {
        match number("afkSeconds", 0, HOSTING_MAX_AFK_SECONDS) {
            Some(n) => afk_seconds = Some(n),
            None => {
                return fail(format!("hosting.afkSeconds must be a whole number from 0 to {HOSTING_MAX_AFK_SECONDS}."));
            }
        }
    }
    Ok(HostingInput {
        max_servers,
        max_players,
        afk_seconds,
    })
}

/// `POST .../mine/heartbeat`: optional version, `online: false` to close, and `hosting`.
pub fn parse_heartbeat_body(body: Option<&Value>) -> Parsed<HeartbeatInput> {
    let Some(source) = as_object(body) else {
        return fail("The request body must be a JSON object.");
    };
    let mut input = HeartbeatInput::default();
    if let Some(raw) = get(&source, "online") {
        match raw.as_bool() {
            Some(online) => input.online = Some(online),
            None => return fail("online must be true or false."),
        }
    }
    if let Some(raw) = get(&source, "version") {
        match parse_version(raw) {
            Some(version) => input.version = Some(version),
            None => return fail("version is malformed."),
        }
    }
    if let Some(raw) = get(&source, "hosting") {
        input.hosting = Some(parse_hosting_input(raw)?);
    }
    Ok(input)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRealmInput {
    pub name: String,
    pub invite_code: Option<String>,
    pub known_host_key: Option<String>,
}

/// The optional `realm` of `POST /launcher/v1/game-sessions`; a `null` is the caller's to read as absent.
pub fn parse_entry_realm(value: &Value) -> Parsed<EntryRealmInput> {
    let Value::Object(source) = value else {
        return fail("realm must be an object with a name.");
    };
    let Some(raw) = source.get("name") else {
        return fail("realm.name is required.");
    };
    let name = parse_name_param(raw.as_str())?;
    let mut input = EntryRealmInput {
        name,
        invite_code: None,
        known_host_key: None,
    };
    if let Some(raw) = source.get("inviteCode") {
        match raw.as_str().and_then(normalize_invite_code) {
            Some(code) => input.invite_code = Some(code),
            None => return fail("That is not an invite code."),
        }
    }
    if let Some(raw) = source.get("knownHostKey") {
        match parse_host_public_key(raw) {
            Some(key) => input.known_host_key = Some(key),
            None => return fail("knownHostKey is malformed."),
        }
    }
    Ok(input)
}

pub const BAN_REASON_MAX_LENGTH: usize = 200;

fn has_control(text: &str) -> bool {
    text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// `POST .../ban`: an optional reason, at most 200 characters, no control characters.
pub fn parse_ban_body(body: Option<&Value>) -> Parsed<Option<String>> {
    let Some(source) = as_object(body) else {
        return fail("The request body must be a JSON object.");
    };
    let Some(raw) = get(&source, "reason") else {
        return Ok(None);
    };
    let Some(reason) = raw.as_str() else {
        return fail("reason must be text.");
    };
    let trimmed = js_trim(reason);
    if trimmed.encode_utf16().count() > BAN_REASON_MAX_LENGTH || has_control(trimmed) {
        return fail(format!("reason must be at most {BAN_REASON_MAX_LENGTH} characters of plain text."));
    }
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

pub const USERNAME_MAX_LENGTH: usize = 64;

/// `POST .../members/add` and `.../player-invites`: 1-64 characters, no control characters.
pub fn parse_username_body(body: Option<&Value>) -> Parsed<String> {
    let Some(source) = as_object(body) else {
        return fail("The request body must be a JSON object.");
    };
    let Some(raw) = get(&source, "username") else {
        return fail("username is required.");
    };
    let Some(text) = raw.as_str() else {
        return fail("username must be text.");
    };
    let username = js_trim(text);
    let length = username.encode_utf16().count();
    if !(1..=USERNAME_MAX_LENGTH).contains(&length) || has_control(username) {
        return fail(format!("username must be 1 to {USERNAME_MAX_LENGTH} characters of plain text."));
    }
    Ok(username.to_owned())
}

pub const LIST_DEFAULT_LIMIT: u32 = 25;
pub const LIST_MAX_LIMIT: u32 = 50;
pub const LIST_MAX_OFFSET: u32 = 10_000;

fn digits(text: &str, most: usize) -> Option<u32> {
    ((1..=most).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// `GET .../listed?limit=1-50&offset=0-10000`, digits only.
pub fn parse_list_query(limit: Option<&str>, offset: Option<&str>) -> Parsed<(u32, u32)> {
    let mut page = (LIST_DEFAULT_LIMIT, 0);
    if let Some(raw) = limit {
        match digits(raw, 3).filter(|n| (1..=LIST_MAX_LIMIT).contains(n)) {
            Some(n) => page.0 = n,
            None => return fail(format!("limit must be a whole number from 1 to {LIST_MAX_LIMIT}.")),
        }
    }
    if let Some(raw) = offset {
        match digits(raw, 5).filter(|n| *n <= LIST_MAX_OFFSET) {
            Some(n) => page.1 = n,
            None => return fail(format!("offset must be a whole number from 0 to {LIST_MAX_OFFSET}.")),
        }
    }
    Ok(page)
}

/// `?knownHostKey=`: the key the launcher last showed the player.
pub fn parse_known_host_key_query(value: Option<&str>) -> Parsed<Option<String>> {
    match value {
        None => Ok(None),
        Some(raw) => match parse_host_public_key(&Value::String(raw.to_owned())) {
            Some(key) => Ok(Some(key)),
            None => fail("knownHostKey is malformed."),
        },
    }
}

/// A realm name from a path: format only; a reserved name is simply not found.
pub fn parse_name_param(value: Option<&str>) -> Parsed<String> {
    match value.and_then(|text| existing_name(&Value::String(text.to_owned()))) {
        Some(name) => Ok(name),
        None => fail_code("REALM_NAME_INVALID", "That is not a realm name."),
    }
}

pub fn parse_invite_code_param(value: &str) -> Parsed<String> {
    match normalize_invite_code(value) {
        Some(code) => Ok(code),
        None => fail("That is not an invite code."),
    }
}
