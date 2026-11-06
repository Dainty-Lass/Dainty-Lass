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
pub const KEY_HEADER: &str = "x-dainty-user-api-key";

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
    /// The runtime manifest.
    RuntimeManifest,
    /// The art pack's JSON.
    Branding,
    /// The news JSON.
    News,
    /// One art pack image, by file name.
    BrandingImage(&'a str),
    /// One runtime file, by name.
    RuntimeFile(&'a str),
    /// The versions this server serves.
    Versions,
    /// A version's client profile.
    ClientProfile(&'a str),
    /// A version's pinned Epic manifest.
    EpicManifest(&'a str),
    /// Any other path.
    Unknown,
}

impl<'a> Route<'a> {
    /// The route `path_only` (no query) names.
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

/// Which query key named the version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionKey {
    /// `version=` or `game=`: a version or an invite alias
    Version,
    /// `edition=`: `reborn` or `legacy`
    Edition,
}

/// The version a request's query asks for: the first `version=`, `game=` or `edition=` pair, form-decoded.
pub fn version_query(query: Option<&str>) -> Option<(VersionKey, String)> {
    url::form_urlencoded::parse(query?.as_bytes()).find_map(|(key, value)| {
        let kind = match &*key {
            "version" | "game" => VersionKey::Version,
            "edition" => VersionKey::Edition,
            _ => return None,
        };
        Some((kind, value.into_owned()))
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

/// One news entry as served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsItem {
    /// ISO 8601 date, the sort key (newest first).
    pub date: String,
    /// Title, cleaned and cut to [`MAX_TITLE`].
    pub title: String,
    /// Body, cleaned and cut to [`MAX_BODY`]; may be empty.
    pub body: String,
}

impl NewsItem {
    /// The entry as served in the news JSON.
    pub fn to_json(&self) -> Value {
        json!({"date": self.date, "title": self.title, "body": self.body})
    }
}

/// Most news entries served.
pub const MAX_NEWS_ITEMS: usize = 50;
/// Longest title, in UTF-16 code units.
pub const MAX_TITLE: usize = 200;
/// Longest body, in UTF-16 code units.
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

const IMAGE_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];

/// An art pack file name the server will serve: up to 100 bytes, a known image extension in any case, and a stem
/// of dot-separated words of letters, digits, `_` and `-` that opens with a letter or digit.
pub fn is_image_file_name(name: &str) -> bool {
    if name.len() > 100 {
        return false;
    }
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let word = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    IMAGE_EXTENSIONS
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
        && stem.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && stem.split('.').all(word)
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

/// `branding.json`'s `accent`: `#rgb` or `#rrggbb` once trimmed, lowercased; anything else is none.
pub fn accent_colour(value: Option<&Value>) -> Option<String> {
    let text = js::trim(value?.as_str()?);
    let digits = text.strip_prefix('#')?;
    let well_formed = matches!(digits.len(), 3 | 6) && digits.bytes().all(|b| b.is_ascii_hexdigit());
    well_formed.then(|| text.to_ascii_lowercase())
}

/// Longest image credit, in UTF-16 code units.
pub const MAX_CREDIT: usize = 200;

/// What JavaScript's `\s` matches: Unicode white space less U+0085, plus the BOM.
fn is_js_space(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

/// An image's `credit`: control characters and every run of white space become one space, then trimmed and cut
/// to [`MAX_CREDIT`]. Only a non-empty string credits anyone.
pub fn credit_line(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?;
    let mut collapsed = String::with_capacity(raw.len());
    let mut after_space = false;
    for c in raw.chars() {
        let c = if c < ' ' || c == '\u{7f}' { ' ' } else { c };
        if is_js_space(c) {
            if !after_space {
                collapsed.push(' ');
            }
            after_space = true;
        } else {
            collapsed.push(c);
            after_space = false;
        }
    }
    let credit = cut_utf16(js::trim(&collapsed), MAX_CREDIT);
    (!credit.is_empty()).then_some(credit)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
    use super::*;

    const SORTED_START: &str = r#"{"version":"v1","build":{"name":"rel-1.14.7-Archon","changelist":647472},"files":[{"name":"Rustic-Isles.dll","size":10,"#;

    fn manifest_of(doc: Value) -> Result<RuntimeManifest, String> {
        parse_runtime_json(doc.to_string().as_bytes())
    }

    fn sha(fill: char) -> String {
        std::iter::repeat_n(fill, 64).collect()
    }

    fn two_files(version: Value) -> Value {
        json!({
            "version": version,
            "files": [
                {"name": "winmm.dll", "size": 3, "sha256": sha('b')},
                {"name": "Rustic-Isles.dll", "size": 10, "sha256": sha('a')},
            ],
        })
    }

    #[test]
    fn conditions_match_star_lists_and_weak_tags() {
        let tag = "\"abc\"";
        assert!(!if_none_match_hits(None, tag));
        assert!(if_none_match_hits(Some(" * "), tag));
        assert!(if_none_match_hits(Some("\"x\", \"abc\""), tag));
        assert!(if_none_match_hits(Some("W/\"abc\""), tag));
        assert!(!if_none_match_hits(Some("\"abcd\""), tag));
        assert!(!if_none_match_hits(Some("*, \"x\""), tag));
    }

    #[test]
    fn paths_route_on_their_raw_bytes() {
        assert_eq!(Route::of("/content/v1/runtime/manifest"), Route::RuntimeManifest);
        assert_eq!(Route::of("/content/v1/runtime/manifest/"), Route::Unknown);
        assert_eq!(Route::of("/content/v1/branding"), Route::Branding);
        assert_eq!(Route::of("/content/v1/news"), Route::News);
        let dots = Route::of("/content/v1/runtime/files/%2e%2e/runtime.json");
        assert_eq!(dots, Route::RuntimeFile("%2e%2e/runtime.json"));
        assert!(!is_runtime_file_name("%2e%2e/runtime.json"));
        assert_eq!(Route::of("/content/v1/branding/"), Route::BrandingImage(""));
        assert_eq!(Route::of("/content/v1/files/x"), Route::Unknown);
        assert!(!is_runtime_file_name("winmm.DLL"));
        assert!(is_runtime_file_name("winmm.dll") && is_runtime_file_name("MysticParadox.dll"));
    }

    #[test]
    fn version_routes_need_a_version_segment() {
        assert_eq!(Route::of("/content/v1/versions"), Route::Versions);
        assert_eq!(
            Route::of("/content/v1/versions/1.14.7/client"),
            Route::ClientProfile("1.14.7")
        );
        assert_eq!(
            Route::of("/content/v1/versions/reforged/epic/manifest"),
            Route::EpicManifest("reforged")
        );
        for tail in [
            "",
            "client",
            "/client",
            "a/b/client",
            "%2e%2e/client",
            "1.14.7/Client",
            "1.14.7/client/",
            "1.14.7/epic",
            "1.14.7",
        ] {
            let path = format!("/content/v1/versions/{tail}");
            assert_eq!(Route::of(&path), Route::Unknown, "{path}");
        }
        assert!(is_version_segment(&"9".repeat(64)) && !is_version_segment(&"9".repeat(65)));
    }

    #[test]
    fn the_first_version_pair_of_the_query_decides() {
        let asked = |q: &str| version_query(Some(q));
        let version = |v: &str| Some((VersionKey::Version, v.to_owned()));
        assert_eq!(asked("game=revived"), version("revived"));
        assert_eq!(asked("x=1&version=1.12.0"), version("1.12.0"));
        assert_eq!(
            asked("edition=reborn&game=revived"),
            Some((VersionKey::Edition, "reborn".to_owned()))
        );
        assert_eq!(asked("game=one&game=two"), version("one"));
        assert_eq!(asked("game=a%2Eb+c"), version("a.b c"));
        assert_eq!(asked("x=1"), None);
        assert_eq!(version_query(None), None);
    }

    #[test]
    fn repeated_header_lines_are_joined() {
        let mut headers = http::HeaderMap::new();
        headers.append(KEY_HEADER, http::HeaderValue::from_static("RRK_a"));
        headers.append(KEY_HEADER, http::HeaderValue::from_static("RRK_b"));
        let joined = header_text(&headers, KEY_HEADER).unwrap();
        assert_eq!(joined, "RRK_a, RRK_b");
        assert!(!is_plausible_key(&joined));
        assert_eq!(header_text(&headers, "if-none-match"), None);
    }

    #[test]
    fn keys_are_printable_ascii_up_to_256_bytes() {
        assert!(is_plausible_key("RRK_abc") && is_plausible_key(&"k".repeat(256)));
        for key in ["", "a b", "a\nb", "\u{e4}"] {
            assert!(!is_plausible_key(key), "{key:?}");
        }
        assert!(!is_plausible_key(&"k".repeat(257)));
    }

    #[test]
    fn runtime_files_are_sorted_with_the_fixed_build() {
        let body = manifest_of(two_files(json!("v1"))).unwrap().to_json();
        assert!(body.starts_with(SORTED_START), "{body}");
        assert!(body.find("Rustic-Isles.dll") < body.find("winmm.dll"));
    }

    #[test]
    fn runtime_json_is_read_as_javascript_reads_it() {
        let mut doc = two_files(json!(" v1 "));
        doc["build"] = json!([]);
        let text = format!("\u{feff}{doc}")
            .replace("\"size\":10", "\"size\":1e1")
            .replace("\"size\":3", "\"size\":3.0");
        let body = parse_runtime_json(text.as_bytes()).unwrap().to_json();
        assert!(body.starts_with(SORTED_START), "{body}");
        let mut named = two_files(json!("x"));
        named["build"] = json!({"name": "ignored", "changelist": 651_234});
        let body = manifest_of(named).unwrap().to_json();
        assert!(body.contains("\"changelist\":651234") && !body.contains("ignored"));
    }

    #[test]
    fn bad_runtime_json_is_refused() {
        let refused = |doc: Value| manifest_of(doc).is_err();
        assert!(refused(Value::Null) && refused(json!([])));
        assert!(refused(two_files(json!(""))) && refused(two_files(json!(" "))));
        let mut not_a_list = two_files(json!("v1"));
        not_a_list["files"] = json!({});
        assert!(refused(not_a_list));
        let with_files = |files: Value| json!({"version": "v1", "files": files});
        let entry = |name: &str, size: Value, hash: String| json!({"name": name, "size": size, "sha256": hash});
        let rustic = || entry("Rustic-Isles.dll", json!(1), sha('a'));
        let winmm = || entry("winmm.dll", json!(1), sha('b'));
        assert!(refused(with_files(json!([rustic()]))));
        assert!(refused(with_files(json!([
            rustic(),
            winmm(),
            entry("dxgi.dll", json!(1), sha('c'))
        ]))));
        assert!(refused(with_files(json!([
            entry("..\\Rustic-Isles.dll", json!(1), sha('a')),
            winmm()
        ]))));
        assert!(refused(with_files(json!([
            rustic(),
            entry("winmm.dll", json!(1), sha('B'))
        ]))));
        assert!(refused(with_files(json!([
            rustic(),
            entry("winmm.dll", json!(0), sha('b'))
        ]))));
        assert!(refused(with_files(json!([
            rustic(),
            entry("winmm.dll", json!(1e9), sha('b'))
        ]))));
        for changelist in [
            json!(-1),
            Value::Null,
            json!(1.5),
            json!("1"),
            json!(9_007_199_254_740_992_u64),
        ] {
            let mut doc = two_files(json!("v1"));
            doc["build"] = json!({ "changelist": changelist });
            assert!(refused(doc), "{changelist}");
        }
    }

    #[test]
    fn runtime_json_errors_name_the_entry() {
        let error = |files: Value| manifest_of(json!({"version": "v1", "files": files})).unwrap_err();
        assert_eq!(
            error(json!([{"name": null}])),
            "runtime.json: unknown file null (only Rustic-Isles.dll and winmm.dll)"
        );
        assert_eq!(
            error(json!([7])),
            "runtime.json: unknown file undefined (only Rustic-Isles.dll and winmm.dll)"
        );
        let old = json!({"name": "MysticParadox.dll", "size": 1, "sha256": sha('a')});
        let new = json!({"name": "Rustic-Isles.dll", "size": 1, "sha256": sha('a')});
        assert_eq!(
            error(json!([new.clone(), old])),
            "runtime.json: MysticParadox.dll is listed twice"
        );
        assert_eq!(error(json!([new])), "runtime.json: winmm.dll is missing from the list");
    }

    #[test]
    fn the_recorded_manifests_come_out_byte_for_byte() {
        for (name, input, body) in protocol::test_data::RUNTIME_MANIFESTS {
            let manifest = parse_runtime_json(input.as_bytes()).unwrap();
            assert_eq!(manifest.to_json(), body.trim_end(), "{name}");
        }
    }

    #[test]
    fn news_is_cleaned_sorted_and_filtered() {
        let doc = json!({"items": [
            {"date": "2026-01-02", "title": "Older", "body": "a\r\nb\rc\u{1}\td"},
            "not an object",
            {"date": "2026-03-04T05:06:07Z", "title": "  Newest\n title ", "body": 5},
            {"date": "whenever", "title": "Bad date"},
            {"title": "No date"},
            {"date": "2026-02-01", "title": " \u{7f} "},
        ]});
        let items = parse_news(&doc).unwrap();
        let seen: Vec<(&str, &str, &str)> = items
            .iter()
            .map(|i| (i.date.as_str(), i.title.as_str(), i.body.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                ("2026-03-04T05:06:07.000Z", "Newest  title", ""),
                ("2026-01-02T00:00:00.000Z", "Older", "a\nb\nc\td"),
            ]
        );
        assert_eq!(
            parse_news(&json!([{"date": "2026-01-01", "title": "x"}]))
                .unwrap()
                .len(),
            1
        );
        assert!(parse_news(&json!({"news": []})).is_err() && parse_news(&json!("x")).is_err());
        let many: Vec<Value> = (0..60).map(|_| json!({"date": "2026-01-01", "title": "t"})).collect();
        assert_eq!(parse_news(&Value::Array(many)).unwrap().len(), MAX_NEWS_ITEMS);
    }

    #[test]
    fn cuts_keep_whole_characters() {
        assert_eq!(cut_utf16("ab\u{1f600}", 3), "ab");
        assert_eq!(cut_utf16("ab\u{1f600}", 4), "ab\u{1f600}");
        assert_eq!(clean_title(&"t".repeat(300)).len(), MAX_TITLE);
    }

    #[test]
    fn image_names_types_and_signatures() {
        for name in ["Harbour-Dusk_2.JPEG", "bg.v2.webp", "a.png", "9.jpg"] {
            assert!(is_image_file_name(name), "{name}");
        }
        for name in [
            ".a.jpg",
            "a..jpg",
            "a b.jpg",
            "a/b.jpg",
            "a.jpg.exe",
            "a.gif",
            "jpg",
            "_a.png",
            "a.jpg.",
        ] {
            assert!(!is_image_file_name(name), "{name}");
        }
        let sized = |stem_len: usize| format!("{}.jpg", "n".repeat(stem_len));
        assert!(is_image_file_name(&sized(96)) && !is_image_file_name(&sized(100)));
        assert_eq!(image_content_type("x.PNG"), "image/png");
        assert_eq!(image_content_type("x.webp"), "image/webp");
        assert_eq!(image_content_type("x.jpeg"), "image/jpeg");
        assert!(signature_matches("image/jpeg", &[0xff, 0xd8, 0xff, 0]));
        assert!(signature_matches("image/png", b"\x89PNG\r\n\x1a\n...."));
        assert!(signature_matches("image/webp", b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!signature_matches("image/webp", b"RIFF\0\0\0\0WEB"));
        assert!(!signature_matches("image/png", &[0xff, 0xd8, 0xff]));
    }

    #[test]
    fn accents_are_short_or_long_hex() {
        let accent = |v: &Value| accent_colour(Some(v));
        assert_eq!(accent(&json!("#C8A24A")).as_deref(), Some("#c8a24a"));
        assert_eq!(accent(&json!(" #abc ")).as_deref(), Some("#abc"));
        for bad in [
            json!("red"),
            json!("#abcd"),
            json!("#12345g"),
            json!("url(x)"),
            json!(5),
            Value::Null,
        ] {
            assert_eq!(accent(&bad), None, "{bad}");
        }
        assert_eq!(accent_colour(None), None);
    }

    #[test]
    fn credits_collapse_white_space() {
        let credit = |v: Value| credit_line(Some(&v));
        assert_eq!(
            credit(json!("  Photo\nby   a friend ")).as_deref(),
            Some("Photo by a friend")
        );
        assert_eq!(credit(json!("x\u{a0}\u{3000}\u{feff}y")).as_deref(), Some("x y"));
        assert_eq!(credit(json!("x\u{85}y")).as_deref(), Some("x\u{85}y"));
        assert_eq!(credit(json!("")), None);
        assert_eq!(credit(json!(7)), None);
        assert_eq!(credit(json!("w".repeat(500))).map(|c| c.len()), Some(MAX_CREDIT));
    }
}
