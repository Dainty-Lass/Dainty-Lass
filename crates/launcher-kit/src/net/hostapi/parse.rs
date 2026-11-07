// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The host API's replies, read as untrusted input: each parser takes a status and a body and never fails
//! louder than the outcome the launcher shows (spec §2.6.3).

use bytes::Bytes;
use protocol::account::is_plausible_account_key;
use protocol::manifest::{
    DEFAULT_CHANGELIST, MAX_RUNTIME_FILE_BYTES, RUNTIME_FILE_NAMES, RuntimeFile, RuntimeManifest, runtime_file_slot,
};
use protocol::status::{ServerStatus, clean_text, iso_date, parse_server_status};
use protocol::text::{is_js_space, is_unsafe_char, js_length, js_trim};
use serde::Serialize;
use serde_json::Value;

use crate::bridge::types::{ErrorCode, LauncherError};
use crate::net::runtime_files::{MAX_SAFE_INTEGER, RuntimeManifestResult};

/// Where branding images are served; a background's `url` must be under it.
pub const BRANDING_PREFIX: &str = "/content/v1/branding/";
/// The most news items kept.
pub const MAX_NEWS_ITEMS: usize = 30;
/// The most backgrounds kept.
pub const MAX_BACKGROUNDS: usize = 8;

const MAX_REGISTER_MESSAGE: usize = 200;
const MAX_BAN_REASON: usize = 500;
const MAX_USER_ID: usize = 128;
const MAX_USERNAME: usize = 32;
const MAX_MANIFEST_VERSION: usize = 48;
const MAX_NEWS_TITLE: usize = 120;
const MAX_NEWS_BODY_UNITS: usize = 4000;
const MAX_CREDIT: usize = 120;
const MAX_BRANDING_STEM: usize = 101;
const BRANDING_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];

/// The body as JSON, invalid UTF-8 replaced; none when empty or not JSON.
pub fn json_body(body: &[u8]) -> Option<Value> {
    if body.is_empty() {
        return None;
    }
    serde_json::from_str(&String::from_utf8_lossy(body)).ok()
}

/// What the status route's reply means before the older route is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusReply {
    /// A status this launcher understands.
    Status(Box<ServerStatus>),
    /// 404: the server may be an older one, so its probe route is asked.
    Probe,
    /// Anything else.
    BadAnswer,
}

/// The status route's reply: 200 with a parseable status, 404, or a bad answer.
pub fn status_reply(status: u16, body: &[u8]) -> StatusReply {
    match status {
        200 => match json_body(body).as_ref().and_then(parse_server_status) {
            Some(parsed) => StatusReply::Status(Box::new(parsed)),
            None => StatusReply::BadAnswer,
        },
        404 => StatusReply::Probe,
        _ => StatusReply::BadAnswer,
    }
}

/// The probe route's reply: a 200 JSON object holding `show-status` is a server this launcher cannot play on.
pub fn probe_reply(status: u16, body: &[u8]) -> bool {
    status == 200
        && json_body(body)
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|obj| obj.contains_key("show-status"))
}

/// The register route's reply: the new account key, or why there is none.
pub fn register_reply(status: u16, body: &[u8]) -> Result<String, LauncherError> {
    let json = json_body(body);
    let obj = json.as_ref().and_then(Value::as_object);
    match status {
        200 => obj
            .and_then(|o| o.get("UUK"))
            .and_then(Value::as_str)
            .filter(|key| is_plausible_account_key(key))
            .map(str::to_owned)
            .ok_or_else(|| LauncherError::new(ErrorCode::ServerError)),
        400 => {
            let code = match obj.and_then(|o| o.get("error")).and_then(Value::as_str) {
                Some("username_invalid") => ErrorCode::UsernameInvalid,
                Some("registration_closed") => ErrorCode::RegistrationClosed,
                _ => ErrorCode::BadRequest,
            };
            Err(LauncherError {
                code,
                detail: clean_text(obj.and_then(|o| o.get("message")), MAX_REGISTER_MESSAGE),
            })
        }
        401 => Err(LauncherError::new(ErrorCode::InviteInvalid)),
        409 => Err(LauncherError::new(ErrorCode::UsernameTaken)),
        413 | 429 => Err(LauncherError::new(ErrorCode::RateLimited)),
        _ => Err(LauncherError::new(ErrorCode::ServerError)),
    }
}

/// The account a key belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    /// The server's id for the account, `^[A-Za-z0-9_-]{1,128}$`.
    pub user_id: String,
    /// The account's name, cleaned.
    pub username: String,
    /// Whether the account is staff on that server.
    pub is_admin: bool,
}

/// The user info object, or none when its id or name is unusable.
pub fn parse_user_info(raw: &Value) -> Option<UserInfo> {
    let obj = raw.as_object()?;
    let user_id = clean_text(obj.get("UserId"), MAX_USER_ID)?;
    if !user_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some(UserInfo {
        user_id,
        username: clean_text(obj.get("Username"), MAX_USERNAME)?,
        is_admin: obj.get("IsAdmin") == Some(&Value::Bool(true)),
    })
}

/// The user info route's reply.
pub fn user_info_reply(status: u16, body: &[u8]) -> Result<UserInfo, LauncherError> {
    let json = json_body(body);
    let obj = json.as_ref().and_then(Value::as_object);
    match status {
        // A 200 without a usable account is the server's fault, not the key's
        200 => json
            .as_ref()
            .and_then(parse_user_info)
            .ok_or_else(|| LauncherError::new(ErrorCode::ServerError)),
        403 if matches!(
            obj.and_then(|o| o.get("error")).and_then(Value::as_str),
            Some("account_banned" | "account_disabled")
        ) =>
        {
            Err(LauncherError {
                code: ErrorCode::AccountBanned,
                detail: clean_text(obj.and_then(|o| o.get("reason")), MAX_BAN_REASON),
            })
        }
        401 | 403 => Err(LauncherError::new(ErrorCode::KeyRejected)),
        429 => Err(LauncherError::new(ErrorCode::RateLimited)),
        _ => Err(LauncherError::new(ErrorCode::ServerError)),
    }
}

/// Removes unsafe characters, collapses whitespace runs to a space, trims and cuts to `max_units` UTF-16 units.
fn clean_units(value: &str, max_units: usize) -> String {
    let mut collapsed = String::with_capacity(value.len());
    let mut after_space = false;
    for ch in value.chars().filter(|&c| !is_unsafe_char(c)) {
        if is_js_space(ch) {
            if !after_space {
                collapsed.push(' ');
            }
            after_space = true;
        } else {
            collapsed.push(ch);
            after_space = false;
        }
    }
    js_trim(cut_units(js_trim(&collapsed), max_units)).to_owned()
}

/// The longest prefix of `text` within `max_units` UTF-16 units that does not split a character.
fn cut_units(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (at, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > max_units {
            return &text[..at];
        }
    }
    text
}

/// A safe integer read as JavaScript reads JSON numbers: `1.0` is one, `1.5` and anything past 2^53−1 are not.
fn safe_integer(value: Option<&Value>) -> Option<f64> {
    let n = value?.as_f64()?;
    (n.is_finite() && n.fract() == 0.0 && n.abs() <= MAX_SAFE_INTEGER as f64).then_some(n)
}

fn is_lower_hex_64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The runtime manifest, files sorted by name; none unless each runtime slot is listed once with a lowercase
/// SHA-256 and a size from 1 byte to 64 MiB. A changelist that is not a safe integer becomes the default, since
/// the protocol's manifest always carries one.
pub fn parse_runtime_manifest(raw: &Value) -> Option<RuntimeManifest> {
    let obj = raw.as_object()?;
    let version = clean_units(obj.get("version")?.as_str()?, MAX_MANIFEST_VERSION);
    if version.is_empty() {
        return None;
    }
    let listed = obj.get("files")?.as_array()?;
    if listed.len() != RUNTIME_FILE_NAMES.len() {
        return None;
    }
    let mut files: Vec<RuntimeFile> = Vec::with_capacity(listed.len());
    for entry in listed {
        let entry = entry.as_object()?;
        let name = entry.get("name")?.as_str()?;
        let slot = runtime_file_slot(name)?;
        if files.iter().any(|f| runtime_file_slot(&f.name) == Some(slot)) {
            return None;
        }
        let sha256 = entry.get("sha256")?.as_str()?;
        if !is_lower_hex_64(sha256) {
            return None;
        }
        let size = safe_integer(entry.get("size"))?;
        if !(1.0..=MAX_RUNTIME_FILE_BYTES as f64).contains(&size) {
            return None;
        }
        files.push(RuntimeFile {
            name: name.to_owned(),
            size: size as u64,
            sha256: sha256.to_owned(),
        });
    }
    files.sort_by(|a, b| a.name.encode_utf16().cmp(b.name.encode_utf16()));
    let changelist = safe_integer(obj.get("build").and_then(|b| b.get("changelist")))
        .filter(|n| *n >= 0.0)
        .map_or(DEFAULT_CHANGELIST, |n| n as u64);
    Some(RuntimeManifest::new(version, changelist, files))
}

/// The runtime manifest route's reply: 404 and 503 mean no runtime is offered.
pub fn runtime_manifest_reply(status: u16, body: &[u8]) -> RuntimeManifestResult {
    match status {
        200 => match json_body(body).as_ref().and_then(parse_runtime_manifest) {
            Some(manifest) => RuntimeManifestResult::Ok(manifest),
            None => RuntimeManifestResult::BadAnswer,
        },
        404 | 503 => RuntimeManifestResult::None,
        _ => RuntimeManifestResult::BadAnswer,
    }
}

/// One news item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewsItem {
    /// When it was posted, canonical ISO UTC.
    pub date: Option<String>,
    /// Headline, cleaned.
    pub title: String,
    /// Text, line breaks kept.
    pub body: String,
}

/// A news body: line breaks unified to LF, unsafe characters but LF removed, at most two LFs in a row, trimmed,
/// cut to 4000 UTF-16 units plus `…`. None when nothing is left.
pub fn clean_news_body(raw: &str) -> Option<String> {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut kept = String::with_capacity(unified.len());
    let mut breaks = 0;
    for ch in unified.chars().filter(|&c| c == '\n' || !is_unsafe_char(c)) {
        if ch == '\n' {
            breaks += 1;
            if breaks > 2 {
                continue;
            }
        } else {
            breaks = 0;
        }
        kept.push(ch);
    }
    let trimmed = js_trim(&kept);
    if trimmed.is_empty() {
        return None;
    }
    if js_length(trimmed) <= MAX_NEWS_BODY_UNITS {
        return Some(trimmed.to_owned());
    }
    Some(format!("{}…", cut_units(trimmed, MAX_NEWS_BODY_UNITS)))
}

fn parse_news_item(raw: &Value) -> Option<NewsItem> {
    let obj = raw.as_object()?;
    Some(NewsItem {
        date: obj.get("date").and_then(Value::as_str).and_then(iso_date),
        title: clean_text(obj.get("title"), MAX_NEWS_TITLE)?,
        body: clean_news_body(obj.get("body")?.as_str()?)?,
    })
}

/// The content server's `{"items": [...]}`: the first 30 usable items. None when the shape is wrong, so the
/// caller keeps what it showed.
pub fn parse_news(raw: &Value) -> Option<Vec<NewsItem>> {
    let items = raw.as_object()?.get("items")?.as_array()?;
    Some(items.iter().filter_map(parse_news_item).take(MAX_NEWS_ITEMS).collect())
}

/// The news route's reply; none means keep the current list.
pub fn news_reply(status: u16, body: &[u8]) -> Option<Vec<NewsItem>> {
    if status != 200 {
        return None;
    }
    json_body(body).as_ref().and_then(parse_news)
}

/// One background image and who made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Background {
    /// A branding path on the same server.
    pub url: String,
    /// Credit line, cleaned.
    pub credit: Option<String>,
}

/// The server's look: background images and an accent colour.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Branding {
    /// At most eight backgrounds.
    pub backgrounds: Vec<Background>,
    /// `#rrggbb`, lowercase.
    pub accent: Option<String>,
}

/// `path` names one image under [`BRANDING_PREFIX`]: no `..`, a plain file name, a known image extension.
pub fn is_branding_path(path: &str) -> bool {
    let Some(prefix) = path.get(..BRANDING_PREFIX.len()) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case(BRANDING_PREFIX) || path.contains("..") {
        return false;
    }
    let Some((stem, extension)) = path[BRANDING_PREFIX.len()..].rsplit_once('.') else {
        return false;
    };
    let mut chars = stem.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    first_ok
        && stem.len() <= MAX_BRANDING_STEM
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && BRANDING_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(extension))
}

fn accent(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let hex = text.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_ascii_lowercase())
}

/// The branding index: backgrounds with a branding path, up to eight, and a `#rrggbb` accent.
pub fn parse_branding(raw: &Value) -> Branding {
    let Some(obj) = raw.as_object() else {
        return Branding::default();
    };
    let backgrounds = obj
        .get("backgrounds")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    let entry = entry.as_object()?;
                    let url = entry.get("url")?.as_str()?;
                    is_branding_path(url).then(|| Background {
                        url: url.to_owned(),
                        credit: clean_text(entry.get("credit"), MAX_CREDIT),
                    })
                })
                .take(MAX_BACKGROUNDS)
                .collect()
        })
        .unwrap_or_default();
    Branding {
        backgrounds,
        accent: accent(obj.get("accent")),
    }
}

/// The branding route's reply; anything but 200 is no branding.
pub fn branding_reply(status: u16, body: &[u8]) -> Branding {
    match (status, json_body(body)) {
        (200, Some(json)) => parse_branding(&json),
        _ => Branding::default(),
    }
}

/// An image type the art store serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageType {
    /// `FF D8 FF`.
    Jpeg,
    /// The PNG signature.
    Png,
    /// `RIFF....WEBP`.
    Webp,
}

impl ImageType {
    /// Its media type.
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

/// The image type from the first bytes; the server's content type is never trusted.
pub fn sniff_image(bytes: &[u8]) -> Option<ImageType> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageType::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageType::Png)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(ImageType::Webp)
    } else {
        None
    }
}

/// A downloaded background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandingImage {
    /// The sniffed type.
    pub kind: ImageType,
    /// The image file.
    pub bytes: Bytes,
}

/// A branding image's reply: only a 200 whose bytes are a known image.
pub fn branding_image_reply(status: u16, body: Bytes) -> Option<BrandingImage> {
    if status != 200 {
        return None;
    }
    sniff_image(&body).map(|kind| BrandingImage { kind, bytes: body })
}
