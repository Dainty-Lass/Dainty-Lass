// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Everything the server reads from outside, as pure functions the fuzz targets drive: the request target and
//! headers, `runtime.json`, the news document, and the art pack's names and `branding.json` values.

use protocol::manifest::{
    DEFAULT_CHANGELIST, MAX_RUNTIME_FILE_BYTES, RUNTIME_FILE_NAMES, RuntimeFile, RuntimeManifest, runtime_file_slot,
};
use serde_json::{Map, Value, json};
use service_kit::js;

use crate::log;

/// The account key's header (the backend's own name for it).
pub const KEY_HEADER: &str = "x-reforged-user-api-key";

const MANIFEST: &str = "/content/v1/runtime/manifest";
const BRANDING: &str = "/content/v1/branding";
const NEWS: &str = "/content/v1/news";
const BRANDING_IMAGE: &str = "/content/v1/branding/";
const RUNTIME_FILE: &str = "/content/v1/runtime/files/";
const VERSIONS: &str = "/content/v1/versions";
const VERSION_PREFIX: &str = "/content/v1/versions/";

/// The route a raw request path names, byte for byte: no decoding, no dot segments, no case folding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route<'a> {
    RuntimeManifest,
    Branding,
    News,
    BrandingImage(&'a str),
    RuntimeFile(&'a str),
    Versions,
    ClientProfile(&'a str),
    EpicManifest(&'a str),
    Unknown,
}

impl<'a> Route<'a> {
    pub fn of(path_only: &'a str) -> Route<'a> {
        match path_only {
            MANIFEST => return Route::RuntimeManifest,
            BRANDING => return Route::Branding,
            NEWS => return Route::News,
            _ => {}
        }
        if let Some(name) = path_only.strip_prefix(BRANDING_IMAGE) {
            return Route::BrandingImage(name);
        }
        if let Some(name) = path_only.strip_prefix(RUNTIME_FILE) {
            return Route::RuntimeFile(name);
        }
        if path_only == VERSIONS {
            return Route::Versions;
        }
        let Some(rest) = path_only.strip_prefix(VERSION_PREFIX) else {
            return Route::Unknown;
        };
        if let Some(version) = rest.strip_suffix("/client")
            && is_version_segment(version)
        {
            return Route::ClientProfile(version);
        }
        if let Some(version) = rest.strip_suffix("/epic/manifest")
            && is_version_segment(version)
        {
            return Route::EpicManifest(version);
        }
        Route::Unknown
    }
}

/// 1-64 bytes of lowercase letters, digits, `.` and `-`.
pub fn is_version_segment(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// A header as Node's `req.headers` reads it: every line of that name, bytes as Latin-1, joined with `", "`.
pub fn header_text(headers: &http::HeaderMap, name: &str) -> Option<String> {
    let mut values = headers.get_all(name).iter().peekable();
    values.peek()?;
    let parts: Vec<String> = values
        .map(|value| value.as_bytes().iter().map(|&b| char::from(b)).collect())
        .collect();
    Some(parts.join(", "))
}

/// Whether `If-None-Match` names `etag`: `*` alone, or one listed tag (weak or not) equal to it.
pub fn if_none_match_hits(header: Option<&str>, etag: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    if js::trim(header) == "*" {
        return true;
    }
    header.split(',').any(|part| {
        let part = js::trim(part);
        part.strip_prefix("W/").unwrap_or(part) == etag
    })
}

/// 1-256 bytes of printable ASCII with no space: anything else is refused without asking the backend.
pub fn is_plausible_key(key: &str) -> bool {
    (1..=256).contains(&key.len()) && key.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// `Rustic-Isles.dll`, `winmm.dll` or the pre-DX4 `MysticParadox.dll`, letter case included.
pub fn is_runtime_file_name(name: &str) -> bool {
    runtime_file_slot(name).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionKey {
    /// `version=` or `game=`: a version or an invite alias
    Version,
    /// `edition=`: `reborn` or `legacy`
    Edition,
}

/// The first `version`, `game` or `edition` pair of a form-encoded query; the rest is ignored.
pub fn version_query(query: Option<&str>) -> Option<(VersionKey, String)> {
    url::form_urlencoded::parse(query?.as_bytes()).find_map(|(key, value)| match key.as_ref() {
        "version" | "game" => Some((VersionKey::Version, value.into_owned())),
        "edition" => Some((VersionKey::Edition, value.into_owned())),
        _ => None,
    })
}

/// `runtime.json`, read as JavaScript reads it, into the manifest players are sent.
pub fn parse_runtime_json(bytes: &[u8]) -> Result<RuntimeManifest, String> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value = service_kit::jsjson::parse(text).map_err(|e| format!("runtime.json: {e}"))?;
    runtime_shape(&value).map_err(|reason| format!("runtime.json: {reason}"))
}

fn runtime_shape(value: &Value) -> Result<RuntimeManifest, String> {
    let Value::Object(index) = value else {
        return Err("runtime.json must be an object".to_owned());
    };
    let version = js::trim(index.get("version").and_then(Value::as_str).unwrap_or_default());
    let version_ok = (1..=48).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'));
    if !version_ok {
        return Err("version must be 1-48 letters, digits, dots, dashes, plus signs or underscores".to_owned());
    }
    let mut changelist = DEFAULT_CHANGELIST;
    if let Some(Value::Object(build)) = index.get("build")
        && let Some(given) = build.get("changelist")
    {
        changelist = js::safe_integer(Some(given))
            .filter(|n| *n > 0)
            .ok_or_else(|| "build.changelist must be a positive whole number".to_owned())? as u64;
    }
    let Some(Value::Array(entries)) = index.get("files") else {
        return Err("files must be a list".to_owned());
    };
    let empty = Map::new();
    let mut files: Vec<RuntimeFile> = Vec::new();
    for entry in entries {
        let entry = entry.as_object().unwrap_or(&empty);
        let name = match entry.get("name") {
            Some(Value::String(name)) if is_runtime_file_name(name) => name.clone(),
            other => {
                let shown = other.map_or_else(|| "undefined".to_owned(), service_kit::jsjson::stringify);
                return Err(format!("unknown file {shown} (only Rustic-Isles.dll and winmm.dll)"));
            }
        };
        if files
            .iter()
            .any(|f| runtime_file_slot(&f.name) == runtime_file_slot(&name))
        {
            return Err(format!("{name} is listed twice"));
        }
        let size = js::safe_integer(entry.get("size"))
            .filter(|n| *n >= 1 && (*n as u64) <= MAX_RUNTIME_FILE_BYTES)
            .ok_or_else(|| format!("{name}: size must be 1 to {MAX_RUNTIME_FILE_BYTES} bytes"))?;
        let sha256 = match entry.get("sha256") {
            Some(Value::String(hash)) if is_sha256_hex(hash) => hash.clone(),
            _ => return Err(format!("{name}: sha256 must be 64 lowercase hex characters")),
        };
        files.push(RuntimeFile {
            name,
            size: size as u64,
            sha256,
        });
    }
    for slot in RUNTIME_FILE_NAMES {
        if !files.iter().any(|f| runtime_file_slot(&f.name) == Some(slot)) {
            return Err(format!("{slot} is missing from the list"));
        }
    }
    Ok(RuntimeManifest::new(version.to_owned(), changelist, files))
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsItem {
    pub date: String,
    pub title: String,
    pub body: String,
}

impl NewsItem {
    pub fn to_json(&self) -> Value {
        json!({"date": self.date, "title": self.title, "body": self.body})
    }
}

pub const MAX_NEWS_ITEMS: usize = 50;
pub const MAX_TITLE: usize = 200;
pub const MAX_BODY: usize = 10_000;
/// JavaScript's `Date` range, in ms either side of 1970
const MAX_DATE_MS: i64 = 8_640_000_000_000_000;

/// The news document: an array or `{"items": [...]}`. Each unusable entry is skipped with a warning; the rest
/// come newest first, at most [`MAX_NEWS_ITEMS`].
pub fn parse_news(doc: &Value) -> Result<Vec<NewsItem>, String> {
    let entries = match doc {
        Value::Array(entries) => entries,
        Value::Object(map) => match map.get("items") {
            Some(Value::Array(entries)) => entries,
            _ => return Err(r#"news file must be {"items": [...]} or an array"#.to_owned()),
        },
        _ => return Err(r#"news file must be {"items": [...]} or an array"#.to_owned()),
    };
    let mut items = Vec::new();
    for entry in entries {
        let Value::Object(entry) = entry else {
            log::warn("news item skipped: not an object", json!({}));
            continue;
        };
        let (Some(Value::String(date)), Some(Value::String(title))) = (entry.get("date"), entry.get("title")) else {
            log::warn("news item skipped: needs a date and a title", json!({}));
            continue;
        };
        let Some(date_text) = js::date_parse(date).and_then(iso_date) else {
            log::warn(
                "news item skipped: date not understood",
                json!({"date": cut_utf16(date, 40)}),
            );
            continue;
        };
        let title = clean_title(title);
        if title.is_empty() {
            log::warn("news item skipped: empty title", json!({}));
            continue;
        }
        let body = match entry.get("body") {
            Some(Value::String(body)) => clean_body(body),
            _ => String::new(),
        };
        items.push(NewsItem {
            date: date_text,
            title,
            body,
        });
    }
    // Stable: equal dates keep their order
    items.sort_by(|a, b| b.date.cmp(&a.date));
    items.truncate(MAX_NEWS_ITEMS);
    Ok(items)
}

/// `Date.prototype.toISOString`: `YYYY-MM-DDTHH:MM:SS.mmmZ`, with a signed six-digit year outside 0-9999.
fn iso_date(ms: i64) -> Option<String> {
    if !(-MAX_DATE_MS..=MAX_DATE_MS).contains(&ms) {
        return None;
    }
    use chrono::{Datelike as _, Timelike as _};
    let at = chrono::DateTime::from_timestamp_millis(ms)?;
    let year = at.year();
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -i64::from(year))
    } else {
        format!("+{year:06}")
    };
    Some(format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.month(),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.timestamp_subsec_millis()
    ))
}

fn newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Control characters become spaces (runs are kept), then trimmed and cut to [`MAX_TITLE`].
pub fn clean_title(text: &str) -> String {
    let spaced: String = newlines(text)
        .chars()
        .map(|c| if c < ' ' || c == '\u{7f}' { ' ' } else { c })
        .collect();
    cut_utf16(js::trim(&spaced), MAX_TITLE)
}

/// Control characters other than tab and newline are removed, then trimmed and cut to [`MAX_BODY`].
pub fn clean_body(text: &str) -> String {
    let kept: String = newlines(text)
        .chars()
        .filter(|&c| !(c < ' ' && c != '\t' && c != '\n') && c != '\u{7f}')
        .collect();
    cut_utf16(js::trim(&kept), MAX_BODY)
}

/// The first `max` UTF-16 units, as whole characters: one that would cross the limit is dropped.
pub fn cut_utf16(text: &str, max: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out
}

/// An art pack image name: at most 100 bytes, `jpg`, `jpeg`, `png` or `webp` in any case, and a stem of
/// dot-separated parts of letters, digits, `_` and `-` whose first part starts with a letter or digit.
pub fn is_safe_image_name(name: &str) -> bool {
    if name.len() > 100 {
        return false;
    }
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    if !matches!(extension.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp") {
        return false;
    }
    let parts_ok = stem.split('.').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    });
    parts_ok && stem.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
}

/// The media type of a safe image name, by its extension.
pub fn image_content_type(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

/// Whether a file's first bytes are the signature of `content_type`.
pub fn signature_matches(content_type: &str, head: &[u8]) -> bool {
    match content_type {
        "image/png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "image/webp" => head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP",
        _ => head.starts_with(&[0xff, 0xd8, 0xff]),
    }
}

/// `#rgb` or `#rrggbb`, trimmed and lowercased; anything else is none.
pub fn clean_accent(value: Option<&Value>) -> Option<String> {
    let text = js::trim(value?.as_str()?);
    let digits = text.strip_prefix('#')?;
    ((digits.len() == 3 || digits.len() == 6) && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

pub const MAX_CREDIT: usize = 200;

/// A credit line: control characters and whitespace runs become one space, trimmed, at most [`MAX_CREDIT`].
pub fn clean_credit(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        let c = if c < ' ' || c == '\u{7f}' { ' ' } else { c };
        if js::is_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    let credit = cut_utf16(js::trim(&out), MAX_CREDIT);
    (!credit.is_empty()).then_some(credit)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn r1_if_none_match() {
        assert!(!if_none_match_hits(None, "\"abc\""));
        assert!(if_none_match_hits(Some(" * "), "\"abc\""));
        assert!(if_none_match_hits(Some("\"x\", \"abc\""), "\"abc\""));
        assert!(if_none_match_hits(Some("W/\"abc\""), "\"abc\""));
        assert!(!if_none_match_hits(Some("\"abcd\""), "\"abc\""));
        assert!(!if_none_match_hits(Some("*, \"x\""), "\"abc\""));
        assert!(if_none_match_hits(Some("\"nope\", \"abc\""), "\"abc\""));
    }

    #[test]
    fn r2_routes_on_the_raw_path() {
        assert_eq!(Route::of("/content/v1/runtime/manifest"), Route::RuntimeManifest);
        assert_eq!(Route::of("/content/v1/runtime/manifest/"), Route::Unknown);
        assert_eq!(Route::of("/content/v1/News"), Route::Unknown);
        let traversal = Route::of("/content/v1/runtime/files/%2e%2e/runtime.json");
        assert_eq!(traversal, Route::RuntimeFile("%2e%2e/runtime.json"));
        assert!(!is_runtime_file_name("%2e%2e/runtime.json"));
        assert_eq!(Route::of("/content/v1/branding/"), Route::BrandingImage(""));
        assert_eq!(Route::of("/content/v1/runtime/files/"), Route::RuntimeFile(""));
        assert_eq!(Route::of("/content/v1/files/x"), Route::Unknown);
        assert_eq!(Route::of("/content/v1/manifest"), Route::Unknown);
        assert!(!is_runtime_file_name("winmm.DLL"));
        assert!(is_runtime_file_name("winmm.dll") && is_runtime_file_name("MysticParadox.dll"));
        assert!(!is_runtime_file_name("mysticparadox.dll"));
    }

    #[test]
    fn r3_version_routes() {
        assert_eq!(Route::of("/content/v1/versions"), Route::Versions);
        assert_eq!(
            Route::of("/content/v1/versions/1.14.7/client"),
            Route::ClientProfile("1.14.7")
        );
        assert_eq!(
            Route::of("/content/v1/versions/reforged/epic/manifest"),
            Route::EpicManifest("reforged")
        );
        for unknown in [
            "/content/v1/versions/",
            "/content/v1/versions/client",
            "/content/v1/versions//client",
            "/content/v1/versions/a/b/client",
            "/content/v1/versions/%2e%2e/client",
            "/content/v1/versions/1.14.7/Client",
            "/content/v1/versions/1.14.7/client/",
            "/content/v1/versions/1.14.7/epic",
            "/content/v1/versions/1.14.7",
        ] {
            assert_eq!(Route::of(unknown), Route::Unknown, "{unknown}");
        }
        assert!(!is_version_segment(&"a".repeat(65)) && is_version_segment(&"a".repeat(64)));
    }

    #[test]
    fn r4_version_query() {
        let v = |q| version_query(Some(q));
        assert_eq!(v("game=revived"), Some((VersionKey::Version, "revived".into())));
        assert_eq!(v("x=1&version=1.12.0"), Some((VersionKey::Version, "1.12.0".into())));
        assert_eq!(
            v("edition=reborn&game=revived"),
            Some((VersionKey::Edition, "reborn".into()))
        );
        assert_eq!(v("game=a&game=b"), Some((VersionKey::Version, "a".into())));
        assert_eq!(v("game=a+b%21"), Some((VersionKey::Version, "a b!".into())));
        assert_eq!(v("x=1"), None);
        assert_eq!(version_query(None), None);
    }

    #[test]
    fn r5_headers_join() {
        let mut headers = http::HeaderMap::new();
        assert_eq!(header_text(&headers, KEY_HEADER), None);
        headers.append(KEY_HEADER, http::HeaderValue::from_static("RRK_a"));
        headers.append(KEY_HEADER, http::HeaderValue::from_static("RRK_b"));
        assert_eq!(header_text(&headers, KEY_HEADER).as_deref(), Some("RRK_a, RRK_b"));
        headers.insert("x-latin", http::HeaderValue::from_bytes(&[0xe4]).unwrap());
        assert_eq!(header_text(&headers, "x-latin").as_deref(), Some("\u{e4}"));
    }

    #[test]
    fn keys() {
        assert!(is_plausible_key("RRK_abc") && is_plausible_key(&"k".repeat(256)));
        for bad in ["", "a b", "a\nb", "RRK_a, RRK_b", "ä"] {
            assert!(!is_plausible_key(bad), "{bad:?}");
        }
        assert!(!is_plausible_key(&"k".repeat(257)));
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn index(files: &str) -> String {
        format!(r#"{{"version":"v1","files":{files}}}"#)
    }

    #[test]
    fn t1_files_sorted_with_the_fixed_build() {
        let text = index(&format!(
            r#"[{{"name":"winmm.dll","size":3,"sha256":"{A}"}},{{"name":"Rustic-Isles.dll","size":10,"sha256":"{A}"}}]"#
        ));
        let manifest = parse_runtime_json(text.as_bytes()).unwrap();
        let names: Vec<&str> = manifest.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Rustic-Isles.dll", "winmm.dll"]);
        assert_eq!(manifest.build.name, "rel-1.14.7-Archon");
        assert_eq!(manifest.build.changelist, 647_472);
    }

    #[test]
    fn t2_refusals() {
        let file = |name: &str, size: &str, sha: &str| format!(r#"{{"name":"{name}","size":{size},"sha256":"{sha}"}}"#);
        let two = |a: String| index(&format!("[{a},{}]", file("winmm.dll", "3", A)));
        let rustic = |size: &str| two(file("Rustic-Isles.dll", size, A));
        let with_build = |cl: &str| {
            format!(
                r#"{{"version":"v1","build":{{"changelist":{cl}}},"files":[{},{}]}}"#,
                file("Rustic-Isles.dll", "3", A),
                file("winmm.dll", "3", A)
            )
        };
        let bad = [
            "null".to_owned(),
            "[]".to_owned(),
            index("[]").replace("\"v1\"", "\"\""),
            index("[]").replace("\"v1\"", "\"a b\""),
            index("{}"),
            index(&format!("[{}]", file("winmm.dll", "3", A))),
            index(&format!(
                "[{},{},{}]",
                file("Rustic-Isles.dll", "3", A),
                file("winmm.dll", "3", A),
                file("winmm.dll", "3", A)
            )),
            two(file("..\\\\Rustic-Isles.dll", "3", A)),
            two(file("dxgi.dll", "3", A)),
            two(file("Rustic-Isles.dll", "3", &A.to_uppercase())),
            rustic("0"),
            rustic("1e9"),
            with_build("-1"),
            with_build("null"),
            with_build("1.5"),
            with_build("\"1\""),
            with_build("9007199254740992"),
        ];
        for text in bad {
            assert!(parse_runtime_json(text.as_bytes()).is_err(), "{text}");
        }
        assert!(parse_runtime_json(with_build("5").as_bytes()).is_ok());
    }

    #[test]
    fn t3_read_as_javascript_reads_it() {
        let text = format!(
            "\u{feff}{{\"version\":\" v1 \",\"build\":[],\"files\":[{{\"name\":\"winmm.dll\",\"size\":5.0,\"sha256\":\"{A}\"}},{{\"name\":\"Rustic-Isles.dll\",\"size\":1e1,\"sha256\":\"{A}\"}}]}}"
        );
        let body = parse_runtime_json(text.as_bytes()).unwrap().to_json();
        assert!(
            body.starts_with(
                r#"{"version":"v1","build":{"name":"rel-1.14.7-Archon","changelist":647472},"files":[{"name":"Rustic-Isles.dll","size":10,"#
            ),
            "{body}"
        );
    }

    #[test]
    fn t4_exact_texts() {
        let err = |text: String| parse_runtime_json(text.as_bytes()).unwrap_err();
        assert_eq!(
            err(index(r#"[{"name":null}]"#)),
            "runtime.json: unknown file null (only Rustic-Isles.dll and winmm.dll)"
        );
        assert_eq!(
            err(index("[5]")),
            "runtime.json: unknown file undefined (only Rustic-Isles.dll and winmm.dll)"
        );
        assert_eq!(
            err(index(r#"[{"name":"dxgi.dll"}]"#)),
            r#"runtime.json: unknown file "dxgi.dll" (only Rustic-Isles.dll and winmm.dll)"#
        );
        assert_eq!(
            err(index(&format!(
                r#"[{{"name":"Rustic-Isles.dll","size":1,"sha256":"{A}"}},{{"name":"MysticParadox.dll"}}]"#
            ))),
            "runtime.json: MysticParadox.dll is listed twice"
        );
        assert_eq!(
            err(index(&format!(r#"[{{"name":"winmm.dll","size":1,"sha256":"{A}"}}]"#))),
            "runtime.json: Rustic-Isles.dll is missing from the list"
        );
        assert_eq!(err("[]".into()), "runtime.json: runtime.json must be an object");
        assert!(err("{".into()).starts_with("runtime.json: "));
    }

    #[test]
    fn recorded_manifests() {
        for (_, input, body) in protocol::test_data::RUNTIME_MANIFESTS {
            assert_eq!(parse_runtime_json(input.as_bytes()).unwrap().to_json(), body);
        }
    }

    #[test]
    fn news_parse_example() {
        let doc = json!({"items": [
            {"date": "2026-01-01T00:00:00Z", "title": "Old", "body": "a\r\nb\rc\u{0}\u{7f}\td"},
            "not an object",
            {"date": "2026-03-01T12:00:00.5Z", "title": "  Newest\n title ", "body": 5},
            {"date": "someday", "title": "Bad date"},
            {"title": "No date"},
            {"date": "2026-02-01", "title": " \u{1} "},
            {"date": "2026-02-01T00:00:00Z", "title": "Mid", "body": " x "},
        ]});
        let items = parse_news(&doc).unwrap();
        assert_eq!(
            items,
            [
                NewsItem {
                    date: "2026-03-01T12:00:00.500Z".into(),
                    title: "Newest  title".into(),
                    body: String::new()
                },
                NewsItem {
                    date: "2026-02-01T00:00:00.000Z".into(),
                    title: "Mid".into(),
                    body: "x".into()
                },
                NewsItem {
                    date: "2026-01-01T00:00:00.000Z".into(),
                    title: "Old".into(),
                    body: "a\nb\nc\td".into()
                },
            ]
        );
        assert_eq!(parse_news(&json!([])).unwrap(), []);
        assert!(parse_news(&json!({"news": []})).is_err());
        assert!(parse_news(&json!("x")).is_err());
        let many: Vec<Value> = (0..60)
            .map(|i| json!({"date": format!("2026-01-{:02}", i % 28 + 1), "title": format!("t{i}")}))
            .collect();
        assert_eq!(parse_news(&Value::Array(many)).unwrap().len(), 50);
        assert_eq!(clean_title(&"é".repeat(300)).chars().count(), 200);
        assert_eq!(cut_utf16("a\u{1f600}", 2), "a");
    }

    #[test]
    fn image_names_and_signatures() {
        for good in [
            "Harbour-Dusk_2.JPEG",
            "bg.v2.webp",
            "a.png",
            &format!("{}.jpg", "a".repeat(96)),
        ] {
            assert!(is_safe_image_name(good), "{good}");
        }
        for bad in [
            ".a.jpg",
            "a..jpg",
            "a b.jpg",
            "a/b.jpg",
            "a.jpg.exe",
            "a.gif",
            "_a.jpg",
            "jpg",
            &format!("{}.jpg", "a".repeat(100)),
        ] {
            assert!(!is_safe_image_name(bad), "{bad}");
        }
        assert_eq!(image_content_type("a.PNG"), "image/png");
        assert_eq!(image_content_type("a.webp"), "image/webp");
        assert_eq!(image_content_type("a.jpeg"), "image/jpeg");
        assert!(signature_matches("image/jpeg", &[0xff, 0xd8, 0xff, 0]));
        assert!(signature_matches("image/png", b"\x89PNG\r\n\x1a\n"));
        assert!(signature_matches("image/webp", b"RIFF\0\0\0\0WEBP"));
        assert!(!signature_matches("image/webp", b"RIFF\0\0\0\0WEB"));
        assert!(!signature_matches("image/png", b"fake"));
    }

    #[test]
    fn accent_and_credit_cleaning() {
        assert_eq!(clean_accent(Some(&json!("#C8A24A"))).as_deref(), Some("#c8a24a"));
        assert_eq!(clean_accent(Some(&json!(" #abc "))).as_deref(), Some("#abc"));
        for bad in [
            json!("red"),
            json!("#abcd"),
            json!("#12345g"),
            json!("url(x)"),
            json!(5),
            Value::Null,
        ] {
            assert_eq!(clean_accent(Some(&bad)), None, "{bad}");
        }
        assert_eq!(
            clean_credit(Some(&json!("  Photo\nby   a friend "))).as_deref(),
            Some("Photo by a friend")
        );
        assert_eq!(clean_credit(Some(&json!(""))), None);
        assert_eq!(clean_credit(Some(&json!(7))), None);
        assert_eq!(clean_credit(Some(&json!("x".repeat(500)))).unwrap().len(), 200);
    }
}
