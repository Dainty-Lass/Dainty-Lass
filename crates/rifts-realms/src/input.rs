// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Strict parsing of the launcher's realm requests: types are checked, never coerced. A key that is present, even
//! `null`, is checked; unknown keys are ignored.

use serde_json::{Map, Value};

use crate::realm::{
    INVITE_DEFAULT_MAX_USES, INVITE_DEFAULT_TTL_HOURS, INVITE_MAX_TTL_HOURS, INVITE_MAX_USES_LIMIT, JoinPolicy,
    NameRefusal, existing_realm_name, js_trim, normalize_invite_code, utf16_len, validate_realm_name,
};

pub const LIST_DEFAULT_LIMIT: u32 = 25;
pub const LIST_MAX_LIMIT: u32 = 50;
pub const LIST_MAX_OFFSET: u32 = 10_000;
pub const BAN_REASON_MAX_LENGTH: usize = 200;
pub const USERNAME_MAX_LENGTH: usize = 64;

const VALIDATION: &str = "REALM_VALIDATION_FAILED";
const NAME_INVALID: &str = "REALM_NAME_INVALID";
const NOT_A_NAME: &str = "That is not a realm name.";
const NOT_A_CODE: &str = "That is not an invite code.";
const KNOWN_KEY_MALFORMED: &str = "knownHostKey is malformed.";
const VERSION_MALFORMED: &str = "version is malformed.";
const POLICY_MESSAGE: &str = "joinPolicy must be SOLO, INVITE_ONLY or PUBLIC.";
const LISTED_MESSAGE: &str = "listed must be true or false.";

/// `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    pub code: &'static str,
    pub message: String,
}

pub type Parsed<T> = Result<T, ParseFailure>;

fn fail<T>(code: &'static str, message: &str) -> Parsed<T> {
    Err(ParseFailure {
        code,
        message: message.to_owned(),
    })
}

fn invalid<T>(message: &str) -> Parsed<T> {
    fail(VALIDATION, message)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRealmInput {
    pub name: Option<String>,
    pub host_public_key: String,
    pub join_policy: JoinPolicy,
    pub listed: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsInput {
    pub join_policy: Option<JoinPolicy>,
    pub listed: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteInput {
    pub max_uses: i64,
    pub ttl_hours: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinInput {
    pub name: Option<String>,
    pub invite_code: Option<String>,
    pub known_host_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostingInput {
    pub max_servers: i64,
    pub max_players: i64,
    pub afk_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatInput {
    pub online: Option<bool>,
    pub version: Option<String>,
    pub hosting: Option<HostingInput>,
}

/// The `realm` of a game-session request (Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRealmInput {
    pub name: String,
    pub invite_code: Option<String>,
    pub known_host_key: Option<String>,
}

/// `Number.isSafeInteger`: `5.0` passes, `5.5`, strings and booleans do not.
pub fn whole_number(value: &Value) -> Option<i64> {
    let number = value.as_f64()?;
    if !number.is_finite() || number.fract() != 0.0 || number.abs() > MAX_SAFE_INTEGER {
        return None;
    }
    if let Some(exact) = value.as_i64() {
        return Some(exact);
    }
    // Within ±2^53 the float is exact.
    Some(number as i64)
}

fn whole_in(value: &Value, min: i64, max: i64) -> Option<i64> {
    whole_number(value).filter(|n| (min..=max).contains(n))
}

/// An absent or `null` body is an empty object.
fn object(body: Option<&Value>) -> Parsed<Map<String, Value>> {
    match body {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => invalid("The request body must be a JSON object."),
    }
}

fn host_key_text(text: &str) -> Option<String> {
    let key = js_trim(text);
    let shaped = (16..=512).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'_' | b'=' | b'-'));
    shaped.then(|| key.to_owned())
}

/// Base64, base64url or hex, told apart by alphabet only; the trimmed text is kept.
pub fn parse_host_public_key(value: &Value) -> Option<String> {
    host_key_text(value.as_str()?)
}

pub fn parse_version(value: &Value) -> Option<String> {
    let version = js_trim(value.as_str()?);
    let shaped = (1..=64).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'));
    shaped.then(|| version.to_owned())
}

fn join_policy(value: &Value) -> Parsed<JoinPolicy> {
    value
        .as_str()
        .and_then(JoinPolicy::parse)
        .map_or_else(|| invalid(POLICY_MESSAGE), Ok)
}

fn boolean(value: &Value, message: &str) -> Parsed<bool> {
    value.as_bool().map_or_else(|| invalid(message), Ok)
}

fn existing_name(value: &Value) -> Parsed<String> {
    value
        .as_str()
        .and_then(existing_realm_name)
        .map_or_else(|| fail(NAME_INVALID, NOT_A_NAME), Ok)
}

fn invite_code(value: &Value) -> Parsed<String> {
    value
        .as_str()
        .and_then(normalize_invite_code)
        .map_or_else(|| invalid(NOT_A_CODE), Ok)
}

fn known_host_key(value: &Value) -> Parsed<String> {
    parse_host_public_key(value).map_or_else(|| invalid(KNOWN_KEY_MALFORMED), Ok)
}

fn version(value: &Value) -> Parsed<String> {
    parse_version(value).map_or_else(|| invalid(VERSION_MALFORMED), Ok)
}

pub fn parse_create_realm_body(body: Option<&Value>) -> Parsed<CreateRealmInput> {
    let body = object(body)?;
    let name = match body.get("name") {
        None => None,
        Some(value) => match value.as_str().map(validate_realm_name) {
            Some(Ok(name)) => Some(name),
            Some(Err(NameRefusal::Reserved)) => return fail("REALM_NAME_RESERVED", "That realm name is reserved."),
            _ => {
                return fail(
                    NAME_INVALID,
                    "A realm name is 3-24 letters, digits or hyphens, and cannot start or end with a hyphen.",
                );
            }
        },
    };
    let Some(host_public_key) = body.get("hostPublicKey").and_then(parse_host_public_key) else {
        return invalid("hostPublicKey must be the host's public key as text.");
    };
    let join_policy = body.get("joinPolicy").map_or(Ok(JoinPolicy::InviteOnly), join_policy)?;
    let listed = body.get("listed").map_or(Ok(false), |v| boolean(v, LISTED_MESSAGE))?;
    let version = body.get("version").map(version).transpose()?;
    Ok(CreateRealmInput {
        name,
        host_public_key,
        join_policy,
        listed,
        version,
    })
}

pub fn parse_settings_body(body: Option<&Value>) -> Parsed<SettingsInput> {
    let body = object(body)?;
    let join_policy = body.get("joinPolicy").map(join_policy).transpose()?;
    let listed = body.get("listed").map(|v| boolean(v, LISTED_MESSAGE)).transpose()?;
    if join_policy.is_none() && listed.is_none() {
        return invalid("Nothing to change: send joinPolicy and/or listed.");
    }
    Ok(SettingsInput { join_policy, listed })
}

pub fn parse_invite_body(body: Option<&Value>) -> Parsed<InviteInput> {
    let body = object(body)?;
    let max_uses = match body.get("maxUses") {
        None => INVITE_DEFAULT_MAX_USES,
        Some(value) => match whole_in(value, 1, INVITE_MAX_USES_LIMIT) {
            Some(n) => n,
            None => return invalid("maxUses must be a whole number from 1 to 100."),
        },
    };
    let ttl_hours = match body.get("expiresInHours") {
        None => INVITE_DEFAULT_TTL_HOURS,
        Some(value) => match whole_in(value, 1, INVITE_MAX_TTL_HOURS) {
            Some(n) => n,
            None => return invalid("expiresInHours must be a whole number from 1 to 720."),
        },
    };
    Ok(InviteInput { max_uses, ttl_hours })
}

pub fn parse_join_body(body: Option<&Value>) -> Parsed<JoinInput> {
    let body = object(body)?;
    let name = body.get("name").map(existing_name).transpose()?;
    let invite_code = body.get("inviteCode").map(invite_code).transpose()?;
    if name.is_none() && invite_code.is_none() {
        return invalid("Send a realm name and/or an invite code.");
    }
    let known_host_key = body.get("knownHostKey").map(known_host_key).transpose()?;
    Ok(JoinInput {
        name,
        invite_code,
        known_host_key,
    })
}

pub fn parse_heartbeat_body(body: Option<&Value>) -> Parsed<HeartbeatInput> {
    let body = object(body)?;
    let online = body
        .get("online")
        .map(|v| boolean(v, "online must be true or false."))
        .transpose()?;
    let version = body.get("version").map(version).transpose()?;
    let hosting = match body.get("hosting") {
        None => None,
        Some(Value::Object(hosting)) => {
            let Some(max_servers) = hosting.get("maxServers").and_then(|v| whole_in(v, 1, 8)) else {
                return invalid("hosting.maxServers must be a whole number from 1 to 8.");
            };
            let Some(max_players) = hosting.get("maxPlayers").and_then(|v| whole_in(v, 1, 64)) else {
                return invalid("hosting.maxPlayers must be a whole number from 1 to 64.");
            };
            let afk_seconds = match hosting.get("afkSeconds") {
                None => None,
                Some(value) => match whole_in(value, 0, 7200) {
                    Some(n) => Some(n),
                    None => return invalid("hosting.afkSeconds must be a whole number from 0 to 7200."),
                },
            };
            Some(HostingInput {
                max_servers,
                max_players,
                afk_seconds,
            })
        }
        Some(_) => return invalid("hosting must be an object."),
    };
    Ok(HeartbeatInput {
        online,
        version,
        hosting,
    })
}

pub fn parse_entry_realm(value: &Value) -> Parsed<EntryRealmInput> {
    let Value::Object(realm) = value else {
        return invalid("realm must be an object with a name.");
    };
    let Some(name) = realm.get("name") else {
        return invalid("realm.name is required.");
    };
    let name = existing_name(name)?;
    let invite_code = realm.get("inviteCode").map(invite_code).transpose()?;
    let known_host_key = realm.get("knownHostKey").map(known_host_key).transpose()?;
    Ok(EntryRealmInput {
        name,
        invite_code,
        known_host_key,
    })
}

fn plain_text(text: &str) -> bool {
    !text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// An empty trimmed reason is none.
pub fn parse_ban_body(body: Option<&Value>) -> Parsed<Option<String>> {
    let body = object(body)?;
    let Some(value) = body.get("reason") else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return invalid("reason must be text.");
    };
    let reason = js_trim(text);
    if utf16_len(reason) > BAN_REASON_MAX_LENGTH || !plain_text(reason) {
        return invalid("reason must be at most 200 characters of plain text.");
    }
    Ok((!reason.is_empty()).then(|| reason.to_owned()))
}

pub fn parse_username_body(body: Option<&Value>) -> Parsed<String> {
    let body = object(body)?;
    let Some(value) = body.get("username") else {
        return invalid("username is required.");
    };
    let Some(text) = value.as_str() else {
        return invalid("username must be text.");
    };
    let username = js_trim(text);
    if !(1..=USERNAME_MAX_LENGTH).contains(&utf16_len(username)) || !plain_text(username) {
        return invalid("username must be 1 to 64 characters of plain text.");
    }
    Ok(username.to_owned())
}

/// No trim: the route's own failure is "accountId is malformed.".
pub fn parse_account_id_param(raw: &str) -> Option<String> {
    let shaped = (1..=128).contains(&raw.len())
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
    shaped.then(|| raw.to_owned())
}

fn digits_within(text: &str, max_digits: usize, min: u32, max: u32) -> Option<u32> {
    if text.is_empty() || text.len() > max_digits || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|n| (min..=max).contains(n))
}

/// No sign, no space, no empty value.
pub fn parse_list_query(limit: Option<&str>, offset: Option<&str>) -> Parsed<(u32, u32)> {
    let limit = match limit {
        None => LIST_DEFAULT_LIMIT,
        Some(text) => match digits_within(text, 3, 1, LIST_MAX_LIMIT) {
            Some(n) => n,
            None => return invalid("limit must be a whole number from 1 to 50."),
        },
    };
    let offset = match offset {
        None => 0,
        Some(text) => match digits_within(text, 5, 0, LIST_MAX_OFFSET) {
            Some(n) => n,
            None => return invalid("offset must be a whole number from 0 to 10000."),
        },
    };
    Ok((limit, offset))
}

pub fn parse_known_host_key_query(raw: Option<&str>) -> Parsed<Option<String>> {
    match raw {
        None => Ok(None),
        Some(text) => host_key_text(text).map_or_else(|| invalid(KNOWN_KEY_MALFORMED), |key| Ok(Some(key))),
    }
}

pub fn parse_name_param(raw: Option<&str>) -> Parsed<String> {
    raw.and_then(existing_realm_name)
        .map_or_else(|| fail(NAME_INVALID, NOT_A_NAME), Ok)
}

pub fn parse_invite_code_param(raw: &str) -> Parsed<String> {
    normalize_invite_code(raw).map_or_else(|| invalid(NOT_A_CODE), Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whole_numbers_are_safe_integers() {
        assert_eq!(whole_number(&json!(5)), Some(5));
        assert_eq!(whole_number(&json!(5.0)), Some(5));
        assert_eq!(whole_number(&json!(5.5)), None);
        assert_eq!(whole_number(&json!("5")), None);
        assert_eq!(whole_number(&json!(true)), None);
        assert_eq!(
            whole_number(&json!(9_007_199_254_740_991_i64)),
            Some(9_007_199_254_740_991)
        );
        assert_eq!(whole_number(&json!(9_007_199_254_740_992.0)), None);
    }
}
