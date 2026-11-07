// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The runtime and launcher update feeds, their publishing, and the signed Guard manifests on disk (public spec §6).

use std::path::{Path, PathBuf};

use axum::body::Bytes;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, StatusCode, header};
use rustic_isles::{Call, Settings, Shared};
use serde_json::{Map, Value};

use crate::guard_manifest::{self, GuardPayload, PLATFORM, Trusted, ed25519_verify, is_version};
use crate::launcher_auth::launcher_sign_in;
use crate::state::LauncherApiState;
use crate::util::{
    self, basename, encode_uri_component as enc, is_segment, iso, json, lenient_base64, now_ms, object, status_text,
};

pub use crate::util::encode_uri_component;

/// `cache-control` for feeds and manifests.
pub const NO_CACHE: &str = "no-cache, no-store, must-revalidate";
/// `cache-control` for versioned files.
pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const DEFAULT_BASE: &str = "https://paradox.mysticfox.dev";
const DEFAULT_PREFIXES: &str = "https://github.com/pranav158/Mystic-Paradox/releases/download/";
const MAX_UPLOAD: usize = 200 * 1024 * 1024;

// ------------------------------------------------------------------ settings and layout (§6.1)

/// `LAUNCHER_UPDATE_ROOT` as set (even empty), else `updates`, made absolute against the working folder.
pub fn update_root(settings: &Settings) -> PathBuf {
    absolute(settings.raw("LAUNCHER_UPDATE_ROOT").unwrap_or("updates"))
}

pub(crate) fn absolute(text: &str) -> PathBuf {
    let path = PathBuf::from(text);
    if path.is_absolute() {
        return path;
    }
    match std::env::current_dir() {
        Ok(cwd) if text.is_empty() => cwd,
        Ok(cwd) => cwd.join(path),
        Err(_) => path,
    }
}

fn base_url(settings: &Settings) -> String {
    let raw = settings.raw("UPDATE_PUBLIC_BASE_URL").unwrap_or(DEFAULT_BASE);
    raw.strip_suffix('/').unwrap_or(raw).to_owned()
}

/// The configured release prefixes.
fn configured_prefixes(settings: &Settings) -> Vec<String> {
    settings
        .raw("LAUNCHER_UPDATE_URL_PREFIXES")
        .unwrap_or(DEFAULT_PREFIXES)
        .split(',')
        .map(str::trim)
        .filter(|p| p.starts_with("https://") && p.ends_with('/') && !p.contains(['?', '#']))
        .map(str::to_owned)
        .collect()
}

fn all_prefixes(settings: &Settings) -> Vec<String> {
    let mut list = vec![format!("{}/launcher/v1/updates/", base_url(settings))];
    list.extend(configured_prefixes(settings));
    list
}

fn extra_allowlist(settings: &Settings) -> Vec<String> {
    let mut list = vec!["winmm.dll".to_owned()];
    for name in settings.raw("RUNTIME_EXTRA_FILES").unwrap_or("").split(',') {
        let name = name.trim().to_lowercase();
        if name.len() <= 96 && is_segment(&name) {
            list.push(name);
        }
    }
    list
}

/// Trimmed, a leading `::ffff:` removed, cut at the first `%`, lower-cased.
pub fn normalize_address(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("::ffff:").unwrap_or(t);
    let t = t.split('%').next().unwrap_or("");
    t.to_lowercase()
}

fn changelist(shared: &Shared) -> u32 {
    shared.version().tables.target_changelist
}

fn trusted(shared: &Shared) -> Trusted {
    shared
        .slot::<Trusted>()
        .map_or_else(Trusted::embedded, |t| (*t).clone())
}

fn extension_roles(shared: &Shared) -> &'static [&'static str] {
    shared.rifts().guard().guard_manifest_roles()
}

fn read_json_file(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    service_kit::jsjson::parse(&text).ok()
}

/// `JSON.stringify(value, null, 2)` and one newline.
pub fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&service_kit::jsjson::to_js(value)).unwrap_or_default();
    text.push('\n');
    text
}

/// The serving switch: on unless `serving.json` says exactly `false`.
pub fn runtime_serving(root: &Path) -> bool {
    read_json_file(&root.join("serving.json"))
        .and_then(|v| v.get("runtimeEnabled").cloned())
        .is_none_or(|v| v != Value::Bool(false))
}

/// The current runtime manifest and its folder, else Node's layout for the same changelist.
fn locate_runtime(
    root: &Path,
    changelist: u32,
    target: &str,
    channel: &str,
    platform: &str,
) -> Option<(Value, PathBuf)> {
    let current = root
        .join("runtime")
        .join(changelist.to_string())
        .join(target)
        .join(channel)
        .join(platform);
    if let Some(manifest) = read_json_file(&current.join("latest.json")) {
        return Some((manifest, current));
    }
    let legacy = root.join("runtime").join(target).join(channel).join(platform);
    let manifest = read_json_file(&legacy.join("latest.json"))?;
    let same = service_kit::js::safe_integer(manifest.get("targetChangelist")) == Some(i64::from(changelist));
    same.then_some((manifest, legacy))
}

/// The runtime manifest a launcher is served, when the three names are segments and one is published.
pub fn published_runtime_manifest(shared: &Shared, target: &str, channel: &str, platform: &str) -> Option<Value> {
    if !is_segment(target) || !is_segment(channel) || !is_segment(platform) {
        return None;
    }
    let root = update_root(shared.settings());
    locate_runtime(&root, changelist(shared), target, channel, platform).map(|(m, _)| m)
}

fn guard_dir(root: &Path, changelist: u32, channel: &str, platform: &str) -> PathBuf {
    root.join("guard")
        .join(changelist.to_string())
        .join(channel)
        .join(platform)
}

/// Whether an envelope is validly signed for another changelist (§6.2 steps 2 and 3).
fn signed_for_other(envelope: &Value, changelist: u32, trusted: &Trusted) -> bool {
    let key = match envelope.get("keyId").and_then(Value::as_str) {
        Some(guard_manifest::RUNTIME_KEY_ID) => trusted.runtime,
        Some(guard_manifest::GUARD_KEY_ID) => trusted.guard,
        _ => return false,
    };
    let payload = lenient_base64(envelope.get("payloadBase64").and_then(Value::as_str).unwrap_or(""));
    let signature = lenient_base64(envelope.get("signature").and_then(Value::as_str).unwrap_or(""));
    if payload.is_empty() || !ed25519_verify(&key, &payload, &signature) {
        return false;
    }
    let body: Option<Value> = serde_json::from_slice(&payload).ok();
    body.and_then(|b| service_kit::js::safe_integer(b.get("targetChangelist")))
        .is_some_and(|c| c != i64::from(changelist))
}

/// The current envelope, else Node's unless it is signed for another changelist.
fn locate_guard(root: &Path, changelist: u32, channel: &str, platform: &str, trusted: &Trusted) -> Option<Value> {
    if let Some(envelope) = read_json_file(&guard_dir(root, changelist, channel, platform).join("latest.json")) {
        return Some(envelope);
    }
    let legacy = read_json_file(&root.join("guard").join(channel).join(platform).join("latest.json"))?;
    (!signed_for_other(&legacy, changelist, trusted)).then_some(legacy)
}

/// The located envelope that verifies now and names the channel and platform asked.
pub fn published_guard_manifest(shared: &Shared, channel: &str, platform: &str) -> Option<(Value, GuardPayload)> {
    if !is_segment(channel) || !is_segment(platform) {
        return None;
    }
    let root = update_root(shared.settings());
    let trusted = trusted(shared);
    let c = changelist(shared);
    let envelope = locate_guard(&root, c, channel, platform, &trusted)?;
    let payload =
        guard_manifest::verify_envelope(&envelope, i64::from(c), extension_roles(shared), now_ms(), &trusted).ok()?;
    (payload.channel == channel && payload.platform == platform).then_some((envelope, payload))
}

/// The `version` of the launcher feed's manifest when it is a launcher version.
pub fn served_launcher_version(shared: &Shared, target: &str, arch: &str) -> Option<String> {
    if !is_segment(target) || !is_segment(arch) {
        return None;
    }
    let root = update_root(shared.settings());
    let manifest = read_json_file(&root.join("launcher").join(target).join(arch).join("latest.json"))?;
    manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| is_launcher_version(v))
        .map(str::to_owned)
}

// ------------------------------------------------------------------ gates (§6.3)

fn limited(shared: &Shared, key: String, max: usize) -> bool {
    shared.auth().limiter().is_limited_at(&key, max, 60_000, now_ms())
}

fn too_many() -> Response {
    json(
        StatusCode::TOO_MANY_REQUESTS,
        &object(vec![("error", Value::from("Too many requests. Please retry later."))]),
    )
}

fn client_key(call: &Call) -> String {
    let client = normalize_address(&util::client_ip(&call.request));
    if client.is_empty() {
        "unknown".to_owned()
    } else {
        client
    }
}

fn download_limited(call: &Call) -> bool {
    limited(&call.shared, format!("update-download:{}", client_key(call)), 180)
}

fn publish_limited(call: &Call) -> bool {
    limited(&call.shared, format!("update-publish:{}", client_key(call)), 30)
}

/// Channels other than `stable` are for signed-in administrators.
async fn channel_gate(shared: &Shared, headers: &HeaderMap, channel: &str) -> Result<(), Response> {
    if channel == "stable" {
        return Ok(());
    }
    let caller = launcher_sign_in(shared, headers).await?;
    if caller.has_role("admin") {
        return Ok(());
    }
    Err(util::launcher_error(
        "AUTH_CHANNEL_MISMATCH",
        "Your account currently requires the stable runtime channel.",
    ))
}

fn publisher_address_ok(call: &Call) -> bool {
    let allow: Vec<String> = call
        .shared
        .settings()
        .raw("UPDATE_PUBLISHER_ALLOWED_IPS")
        .unwrap_or("")
        .split(',')
        .map(normalize_address)
        .filter(|a| !a.is_empty())
        .collect();
    let client = normalize_address(&util::client_ip(&call.request));
    let peer = normalize_address(&util::peer_text(&call.request));
    !allow.is_empty() && (allow.contains(&client) || allow.contains(&peer))
}

fn publisher_key_ok(call: &Call) -> bool {
    let configured = call
        .shared
        .settings()
        .raw("UPDATE_PUBLISHER_API_KEY")
        .unwrap_or("")
        .trim()
        .to_owned();
    let sent = util::header_lossy(call.request.headers(), "x-update-api-key").unwrap_or_default();
    !configured.is_empty() && util::ct_eq(util::trim(&sent).as_bytes(), configured.as_bytes())
}

fn error(status: StatusCode, message: &str) -> Response {
    json(status, &object(vec![("error", Value::from(message))]))
}

fn bad_credentials() -> Response {
    error(StatusCode::UNAUTHORIZED, "Invalid update publisher credentials.")
}

fn with_cache(mut response: Response, value: &str) -> Response {
    util::set_header(&mut response, "cache-control", value);
    response
}

async fn serve_file(path: &Path, cache: &str, extra: Option<(&'static str, String)>) -> Response {
    let Ok(bytes) = tokio::fs::read(path).await else {
        return status_text(StatusCode::NOT_FOUND);
    };
    let mut response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (header::CONTENT_LENGTH, bytes.len().to_string()),
            (header::CACHE_CONTROL, cache.to_owned()),
        ],
        bytes,
    )
        .into_response();
    if let Some((name, value)) = extra {
        util::set_header(&mut response, name, &value);
    }
    response
}

fn param<'a>(call: &'a Call, name: &str) -> &'a str {
    call.params.get(name).unwrap_or("")
}

fn is_target(target: &str) -> bool {
    matches!(target, "client" | "server")
}

// ------------------------------------------------------------------ the runtime feed (§6.4)

async fn runtime_manifest(call: Call, target: &str) -> Response {
    let (channel, platform) = (param(&call, "channel").to_owned(), param(&call, "platform").to_owned());
    if !is_target(target) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    if let Err(refused) = channel_gate(&call.shared, call.request.headers(), &channel).await {
        return refused;
    }
    let root = update_root(call.shared.settings());
    if !runtime_serving(&root) {
        return with_cache(util::empty(StatusCode::NO_CONTENT), NO_CACHE);
    }
    if !is_segment(target) || !is_segment(&channel) || !is_segment(&platform) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    match locate_runtime(&root, changelist(&call.shared), target, &channel, &platform) {
        Some((manifest, _)) => with_cache(util::ok(&manifest), NO_CACHE),
        None => status_text(StatusCode::NOT_FOUND),
    }
}

/// `GET /launcher/v1/runtime/:target/:channel/:platform`: a target's runtime manifest.
pub async fn runtime_by_target(call: Call) -> Response {
    let target = param(&call, "target").to_owned();
    runtime_manifest(call, &target).await
}

/// `GET /launcher/v1/runtime/:channel/:platform`: the client's runtime manifest.
pub async fn runtime_client(call: Call) -> Response {
    runtime_manifest(call, "client").await
}

async fn runtime_download(call: Call, target: &str) -> Response {
    if download_limited(&call) {
        return too_many();
    }
    if !is_target(target) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    let (channel, platform) = (param(&call, "channel").to_owned(), param(&call, "platform").to_owned());
    if let Err(refused) = channel_gate(&call.shared, call.request.headers(), &channel).await {
        return refused;
    }
    if !is_segment(&channel) || !is_segment(&platform) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    let root = update_root(call.shared.settings());
    let Some((manifest, dir)) = locate_runtime(&root, changelist(&call.shared), target, &channel, &platform) else {
        return status_text(StatusCode::NOT_FOUND);
    };
    let version = match manifest.get("version").and_then(Value::as_str) {
        Some(v) if !is_segment(v) => return status_text(StatusCode::BAD_REQUEST),
        Some(v) => v.to_owned(),
        None => return status_text(StatusCode::NOT_FOUND),
    };
    let file = match manifest.get("file").and_then(Value::as_str) {
        Some(f) if basename(f) == f && !f.is_empty() => f.to_owned(),
        _ => return status_text(StatusCode::NOT_FOUND),
    };
    serve_file(&dir.join(version).join(file), IMMUTABLE, None).await
}

/// `GET /launcher/v1/runtime/:target/:channel/:platform/download`: a target's runtime package.
pub async fn download_by_target(call: Call) -> Response {
    let target = param(&call, "target").to_owned();
    runtime_download(call, &target).await
}

/// `GET /launcher/v1/runtime/:channel/:platform/download`: the client's runtime package.
pub async fn download_client(call: Call) -> Response {
    runtime_download(call, "client").await
}

/// `GET /launcher/v1/runtime/:target/:channel/:platform/extra/:filename`: a published extra file.
pub async fn extra_download(call: Call) -> Response {
    if download_limited(&call) {
        return too_many();
    }
    let target = param(&call, "target").to_owned();
    if !is_target(&target) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    let (channel, platform) = (param(&call, "channel").to_owned(), param(&call, "platform").to_owned());
    if let Err(refused) = channel_gate(&call.shared, call.request.headers(), &channel).await {
        return refused;
    }
    let filename = basename(param(&call, "filename")).to_owned();
    if !is_segment(&filename) || !is_segment(&channel) || !is_segment(&platform) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    let root = update_root(call.shared.settings());
    let Some((manifest, dir)) = locate_runtime(&root, changelist(&call.shared), &target, &channel, &platform) else {
        return status_text(StatusCode::NOT_FOUND);
    };
    let version = match manifest.get("version").and_then(Value::as_str) {
        Some(v) if !is_segment(v) => return status_text(StatusCode::BAD_REQUEST),
        Some(v) => v.to_owned(),
        None => return status_text(StatusCode::NOT_FOUND),
    };
    let listed = manifest
        .get("extraFiles")
        .and_then(Value::as_array)
        .is_some_and(|list| {
            list.iter()
                .any(|e| e.get("name").and_then(Value::as_str) == Some(&filename))
        });
    let path = dir.join(version).join(&filename);
    if !listed || !path.is_file() {
        return status_text(StatusCode::NOT_FOUND);
    }
    serve_file(&path, IMMUTABLE, None).await
}

// ------------------------------------------------------------------ publishing (§6.5)

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)
}

fn write_replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

fn publisher_gate(call: &Call, denied: Option<&str>) -> Result<(), Response> {
    if publish_limited(call) {
        return Err(too_many());
    }
    if !publisher_address_ok(call) {
        if let Some(line) = denied {
            tracing::warn!("{line} {}", normalize_address(&util::client_ip(&call.request)));
        }
        return Err(error(
            StatusCode::FORBIDDEN,
            "Update publishing is not allowed from this address.",
        ));
    }
    if !publisher_key_ok(call) {
        return Err(bad_credentials());
    }
    Ok(())
}

fn header(call: &Call, name: &str) -> String {
    util::header_lossy(call.request.headers(), name).unwrap_or_default()
}

fn runtime_signature_ok(shared: &Shared, signature: &str, body: &[u8]) -> Option<Vec<u8>> {
    let bytes = lenient_base64(signature);
    (bytes.len() == 64 && ed25519_verify(&trusted(shared).runtime, body, &bytes)).then_some(bytes)
}

struct Upload {
    call: Call,
    body: Option<Bytes>,
}

async fn take_upload(mut call: Call) -> Result<Upload, Response> {
    let body = util::read_octets(&mut call.request, MAX_UPLOAD).await?;
    Ok(Upload { call, body })
}

/// `POST /launcher/v1/admin/updates/runtime/:target/:channel/:platform`: publishes a runtime package.
pub async fn publish_runtime(call: Call) -> Response {
    let Upload { call, body } = match take_upload(call).await {
        Ok(upload) => upload,
        Err(page) => return page,
    };
    if let Err(refused) = publisher_gate(&call, Some("[update] publish denied for")) {
        return refused;
    }
    let target = param(&call, "target").to_owned();
    let channel = param(&call, "channel").to_owned();
    let platform = param(&call, "platform").to_owned();
    let version = header(&call, "x-update-version");
    let c = changelist(&call.shared);
    let changelist_ok =
        service_kit::js::number(Some(&Value::String(header(&call, "x-update-changelist")))) == f64::from(c);
    let metadata_ok = [&target, &channel, &platform, &version].iter().all(|s| is_segment(s))
        && is_target(&target)
        && platform == PLATFORM
        && is_version(&version)
        && changelist_ok
        && body.is_some();
    if !metadata_ok {
        return error(StatusCode::BAD_REQUEST, "Invalid runtime update metadata.");
    }
    let body = body.unwrap_or_default();
    if body.is_empty() || body.len() > MAX_UPLOAD {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Runtime update is outside the allowed size.",
        );
    }
    let signature = header(&call, "x-update-signature");
    if runtime_signature_ok(&call.shared, &signature, &body).is_none() {
        return error(StatusCode::BAD_REQUEST, "Runtime update signature verification failed.");
    }
    let settings = call.shared.settings();
    let platform_dir = update_root(settings)
        .join("runtime")
        .join(c.to_string())
        .join(&target)
        .join(&channel)
        .join(&platform);
    let file = format!("ParadoxRuntime-{version}-{target}-{c}.dll");
    let sha = util::sha256_hex(&body);
    let url = format!(
        "{}/launcher/v1/runtime/{}/{}/{platform}/download",
        base_url(settings),
        enc(&target),
        enc(&channel)
    );
    let manifest = object(vec![
        ("schema", Value::from(1)),
        ("component", Value::from("ParadoxRuntime")),
        ("target", Value::from(target.clone())),
        ("version", Value::from(version.clone())),
        ("channel", Value::from(channel.clone())),
        ("targetChangelist", Value::from(c)),
        ("platform", Value::from(platform.clone())),
        ("size", Value::from(body.len())),
        ("sha256", Value::from(sha.clone())),
        ("signature", Value::from(signature)),
        ("file", Value::from(file.clone())),
        ("publishedAt", Value::from(iso(now_ms()))),
        ("url", Value::from(url)),
        ("extraFiles", Value::Array(Vec::new())),
    ]);
    let version_dir = platform_dir.join(&version);
    let written = blocking(move || -> Result<bool, std::io::Error> {
        let dll = version_dir.join(&file);
        if dll.exists() {
            return Ok(false);
        }
        write_new(&dll, &body)?;
        let text = pretty(&manifest);
        write_replace(&version_dir.join("manifest.json"), text.as_bytes())?;
        write_replace(&platform_dir.join("latest.json"), text.as_bytes())?;
        Ok(true)
    })
    .await;
    match written {
        Some(Ok(true)) => {}
        Some(Ok(false)) => {
            return error(
                StatusCode::CONFLICT,
                "That target/channel/version is already published and immutable.",
            );
        }
        Some(Err(e)) => {
            tracing::error!("[update] publish failed: {e}");
            return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
        None => {
            tracing::error!("[update] publish failed: the file task stopped");
            return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    json(
        StatusCode::CREATED,
        &object(vec![
            ("target", Value::from(target)),
            ("channel", Value::from(channel)),
            ("version", Value::from(version)),
            ("sha256", Value::from(sha)),
        ]),
    )
}

enum ExtraOutcome {
    Written,
    NotPublished,
}

/// `POST /launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra`: publishes an extra file.
pub async fn publish_extra(call: Call) -> Response {
    let Upload { call, body } = match take_upload(call).await {
        Ok(upload) => upload,
        Err(page) => return page,
    };
    if let Err(refused) = publisher_gate(&call, Some("[update] extra-file publish denied for")) {
        return refused;
    }
    let target = param(&call, "target").to_owned();
    let channel = param(&call, "channel").to_owned();
    let platform = param(&call, "platform").to_owned();
    let version = header(&call, "x-update-version");
    let sent_name = header(&call, "x-update-filename");
    let filename = basename(&sent_name).to_owned();
    if ![&target, &channel, &platform, &version, &filename]
        .iter()
        .all(|s| is_segment(s))
    {
        return error(StatusCode::BAD_REQUEST, "Invalid extra-file metadata.");
    }
    let settings = call.shared.settings();
    if !is_target(&target) || platform != PLATFORM || !extra_allowlist(settings).contains(&filename.to_lowercase()) {
        return error(StatusCode::BAD_REQUEST, "Invalid extra-file target/platform/name.");
    }
    let body = match body {
        Some(b) if !b.is_empty() && b.len() <= MAX_UPLOAD => b,
        _ => return error(StatusCode::PAYLOAD_TOO_LARGE, "Extra file is outside the allowed size."),
    };
    let Some(signature) = runtime_signature_ok(&call.shared, &header(&call, "x-update-signature"), &body) else {
        return error(StatusCode::BAD_REQUEST, "Extra file signature verification failed.");
    };
    let c = changelist(&call.shared);
    let platform_dir = update_root(settings)
        .join("runtime")
        .join(c.to_string())
        .join(&target)
        .join(&channel)
        .join(&platform);
    let sha = util::sha256_hex(&body);
    let size = body.len();
    let entry = object(vec![
        ("name", Value::from(filename.clone())),
        ("size", Value::from(size)),
        ("sha256", Value::from(sha.clone())),
        ("signature", Value::from(util::standard_base64(&signature))),
        (
            "url",
            Value::from(format!(
                "{}/launcher/v1/runtime/{}/{}/{platform}/extra/{}",
                base_url(settings),
                enc(&target),
                enc(&channel),
                enc(&filename)
            )),
        ),
    ]);
    let wanted_version = version.clone();
    let name = filename.clone();
    let written = blocking(move || -> Result<ExtraOutcome, std::io::Error> {
        let Some(Value::Object(mut manifest)) = read_json_file(&platform_dir.join("latest.json")) else {
            return Ok(ExtraOutcome::NotPublished);
        };
        let version_dir = platform_dir.join(&wanted_version);
        let same = manifest.get("version").and_then(Value::as_str) == Some(wanted_version.as_str());
        if !same || !version_dir.is_dir() {
            return Ok(ExtraOutcome::NotPublished);
        }
        write_replace(&version_dir.join(&name), &body)?;
        let mut list: Vec<Value> = manifest
            .get("extraFiles")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter(|e| e.is_object() && e.get("name").and_then(Value::as_str) != Some(name.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        list.push(entry);
        manifest.insert("extraFiles".into(), Value::Array(list));
        let text = pretty(&Value::Object(manifest));
        write_replace(&version_dir.join("manifest.json"), text.as_bytes())?;
        write_replace(&platform_dir.join("latest.json"), text.as_bytes())?;
        Ok(ExtraOutcome::Written)
    })
    .await;
    match written {
        Some(Ok(ExtraOutcome::Written)) => json(
            StatusCode::CREATED,
            &object(vec![
                ("name", Value::from(filename)),
                ("sha256", Value::from(sha)),
                ("size", Value::from(size)),
            ]),
        ),
        Some(Ok(ExtraOutcome::NotPublished)) => error(
            StatusCode::CONFLICT,
            "Publish the main runtime DLL for this version first.",
        ),
        Some(Err(e)) => {
            tracing::error!("[update] extra-file publish failed: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
        None => util::empty(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn not_allowed() -> Response {
    error(StatusCode::FORBIDDEN, "Not allowed from this address.")
}

/// `POST /launcher/v1/admin/guard-manifests/:channel/:platform`: publishes a signed Guard manifest.
pub async fn publish_guard(mut call: Call) -> Response {
    // Core's reader holds the body to 100 KiB, the error page on a parse failure
    let body = match util::read_json(&mut call.request).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    if publish_limited(&call) {
        return too_many();
    }
    if !publisher_address_ok(&call) {
        return not_allowed();
    }
    if !publisher_key_ok(&call) {
        return bad_credentials();
    }
    let channel = param(&call, "channel").to_owned();
    let platform = param(&call, "platform").to_owned();
    if !is_segment(&channel) {
        return error(StatusCode::BAD_REQUEST, "Invalid channel");
    }
    if !is_segment(&platform) {
        return error(StatusCode::BAD_REQUEST, "Invalid platform");
    }
    let Some(envelope) = body else {
        return error(StatusCode::BAD_REQUEST, "Invalid Guard manifest envelope.");
    };
    let shared = &call.shared;
    let c = changelist(shared);
    let trusted = trusted(shared);
    let roles = extension_roles(shared);
    let now = now_ms();
    let payload = match guard_manifest::verify_envelope(&envelope, i64::from(c), roles, now, &trusted) {
        Ok(payload) => payload,
        Err(text) => return error(StatusCode::BAD_REQUEST, &text),
    };
    if payload.channel != channel || payload.platform != platform {
        return error(
            StatusCode::BAD_REQUEST,
            "Guard manifest route and signed payload do not match.",
        );
    }
    let root = update_root(shared.settings());
    let dir = guard_dir(&root, c, &channel, &platform);
    let previous = read_json_file(&dir.join("latest.json")).or_else(|| {
        let legacy = read_json_file(&root.join("guard").join(&channel).join(&platform).join("latest.json"))?;
        guard_manifest::verify_envelope(&legacy, i64::from(c), roles, now, &trusted)
            .is_ok()
            .then_some(legacy)
    });
    if let Some(previous) = previous {
        match guard_manifest::verify_envelope(&previous, i64::from(c), roles, now, &trusted) {
            Ok(before) if payload.sequence <= before.sequence => {
                return error(
                    StatusCode::CONFLICT,
                    "Guard manifest sequence must increase monotonically.",
                );
            }
            Ok(_) => {}
            Err(text) => return error(StatusCode::BAD_REQUEST, &text),
        }
    }
    let file = dir
        .join(payload.sequence.to_string())
        .join(format!("{}.json", payload.manifest_id));
    let text = pretty(&envelope);
    let latest = dir.join("latest.json");
    let written = blocking(move || -> Result<bool, std::io::Error> {
        if file.exists() {
            return Ok(false);
        }
        write_new(&file, text.as_bytes())?;
        write_replace(&latest, text.as_bytes())?;
        Ok(true)
    })
    .await;
    match written {
        Some(Ok(true)) => json(
            StatusCode::CREATED,
            &object(vec![
                ("manifestId", Value::from(payload.manifest_id)),
                ("sequence", Value::from(payload.sequence)),
                ("keyId", envelope.get("keyId").cloned().unwrap_or(Value::Null)),
            ]),
        ),
        Some(Ok(false)) => error(
            StatusCode::CONFLICT,
            "That Guard manifest is already published and immutable.",
        ),
        _ => error(StatusCode::BAD_REQUEST, "Invalid Guard manifest."),
    }
}

/// `GET /launcher/v1/guard-manifests/:channel/:platform`: the published Guard manifest.
pub async fn guard_manifest_route(call: Call) -> Response {
    let channel = param(&call, "channel").to_owned();
    let platform = param(&call, "platform").to_owned();
    if let Err(refused) = channel_gate(&call.shared, call.request.headers(), &channel).await {
        return refused;
    }
    if !is_segment(&channel) || !is_segment(&platform) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    let shared = &call.shared;
    let root = update_root(shared.settings());
    let c = changelist(shared);
    let trusted = trusted(shared);
    let Some(envelope) = locate_guard(&root, c, &channel, &platform, &trusted) else {
        return status_text(StatusCode::NOT_FOUND);
    };
    match guard_manifest::verify_envelope(&envelope, i64::from(c), extension_roles(shared), now_ms(), &trusted) {
        Err(_) => status_text(StatusCode::BAD_REQUEST),
        Ok(p) if p.channel != channel || p.platform != platform => status_text(StatusCode::NOT_FOUND),
        Ok(_) => with_cache(util::ok(&envelope), NO_CACHE),
    }
}

// ------------------------------------------------------------------ the serving switch (§6.6)

/// `GET /launcher/v1/admin/updates/serving`: whether runtime downloads are served.
pub async fn serving_get(call: Call) -> Response {
    if !publisher_address_ok(&call) {
        return not_allowed();
    }
    if !publisher_key_ok(&call) {
        return bad_credentials();
    }
    let enabled = runtime_serving(&update_root(call.shared.settings()));
    util::ok(&object(vec![("runtimeEnabled", Value::Bool(enabled))]))
}

/// `POST /launcher/v1/admin/updates/serving`: turns runtime serving on or off.
pub async fn serving_post(mut call: Call) -> Response {
    let body = match util::read_json(&mut call.request).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    let ip = normalize_address(&util::client_ip(&call.request));
    if !publisher_address_ok(&call) {
        tracing::warn!("[update] serving toggle denied for {ip} (not in UPDATE_PUBLISHER_ALLOWED_IPS)");
        return not_allowed();
    }
    if !publisher_key_ok(&call) {
        return bad_credentials();
    }
    let Some(enabled) = util::field(body.as_ref(), "runtimeEnabled").and_then(Value::as_bool) else {
        return error(StatusCode::BAD_REQUEST, "runtimeEnabled must be a boolean.");
    };
    let path = update_root(call.shared.settings()).join("serving.json");
    let text = pretty(&object(vec![
        ("runtimeEnabled", Value::Bool(enabled)),
        ("updatedAt", Value::from(iso(now_ms()))),
    ]));
    match blocking(move || write_replace(&path, text.as_bytes())).await {
        Some(Ok(())) => {}
        Some(Err(e)) => {
            tracing::error!("[update] serving switch not saved: {e}");
            return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
        None => return util::empty(StatusCode::INTERNAL_SERVER_ERROR),
    }
    tracing::warn!(
        "[update] runtime update serving {} by {ip}",
        if enabled { "ENABLED" } else { "DISABLED" }
    );
    util::ok(&object(vec![("runtimeEnabled", Value::Bool(enabled))]))
}

// ------------------------------------------------------------------ the launcher's own feed (§6.7)

fn version_groups(text: &str) -> Option<[u64; 3]> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut out = [0u64; 3];
    for (i, part) in parts.iter().enumerate() {
        if !(1..=5).contains(&part.len()) || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out[i] = part.parse().ok()?;
    }
    Some(out)
}

/// Three dot-separated groups of 1 to 5 digits.
pub fn is_launcher_version(text: &str) -> bool {
    version_groups(text).is_some()
}

/// Whether `url` is an `https://` URL under one of the prefixes and longer than it.
pub fn is_allowed_url(url: &str, prefixes: &[String]) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    util::utf16_len(url) <= 1024
        && !url.contains(['?', '#'])
        && !authority.is_empty()
        && !authority.contains('@')
        && prefixes
            .iter()
            .any(|p| url.starts_with(p.as_str()) && url.len() > p.len())
}

/// `launcher/<target>/<arch>/<version>/<file>` for a well-formed installer name.
pub fn launcher_installer_path(target: &str, arch: &str, version: &str, file: &str) -> Option<PathBuf> {
    if !is_launcher_version(version) || !is_segment(target) || !is_segment(arch) {
        return None;
    }
    let stem = file.strip_suffix("_x64-setup.exe")?;
    let stem_ok = (1..=120).contains(&stem.len())
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !stem_ok || !file.contains(&format!("_{version}_")) {
        return None;
    }
    Some(
        PathBuf::from("launcher")
            .join(target)
            .join(arch)
            .join(version)
            .join(file),
    )
}

/// The feed's answer: offered with the manifest's Tauri keys, or not offered and why.
#[derive(Debug, Clone, PartialEq)]
pub enum LauncherDecision {
    /// Offered, with the update's JSON.
    Offered(Value),
    /// Not offered: no manifest, or the reason.
    NotOffered(Option<&'static str>),
}

/// What a launcher at `current` is offered from the manifest; `exists` answers for paths under the update root.
pub fn decide_launcher_update(
    manifest: Option<&Value>,
    target: &str,
    arch: &str,
    current: &str,
    prefixes: &[String],
    exists: impl Fn(&Path) -> bool,
) -> LauncherDecision {
    let Some(manifest) = manifest else {
        return LauncherDecision::NotOffered(None);
    };
    let Some(version) = manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| is_launcher_version(v))
    else {
        return LauncherDecision::NotOffered(Some("the manifest has no x.y.z version"));
    };
    let signed = manifest
        .get("signature")
        .is_some_and(|s| !s.is_null() && s.as_str() != Some(""));
    if !signed {
        return LauncherDecision::NotOffered(Some("the manifest has no signature"));
    }
    if !manifest
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|u| is_allowed_url(u, prefixes))
    {
        return LauncherDecision::NotOffered(Some("the manifest's download url is not allowed"));
    }
    if let (Some(now), Some(next)) = (version_groups(current), version_groups(version))
        && next <= now
    {
        return LauncherDecision::NotOffered(Some("up to date"));
    }
    if let Some(file) = manifest.get("file").and_then(Value::as_str) {
        let installer = launcher_installer_path(target, arch, version, file).is_some_and(|p| exists(&p));
        let plain = basename(file) == file
            && exists(
                &PathBuf::from("launcher")
                    .join(target)
                    .join(arch)
                    .join(version)
                    .join(file),
            );
        if !installer && !plain {
            return LauncherDecision::NotOffered(Some("the manifest's installer is not on this server"));
        }
    }
    let mut offered = Map::new();
    for key in ["version", "notes", "pub_date", "url", "signature"] {
        if let Some(value) = manifest.get(key) {
            offered.insert(key.to_owned(), value.clone());
        }
    }
    LauncherDecision::Offered(Value::Object(offered))
}

/// `GET /launcher/v1/updates/:target/:arch/:currentVersion`: the launcher update feed; 204 when none.
pub async fn launcher_update(call: Call) -> Response {
    let target = param(&call, "target").to_owned();
    let arch = param(&call, "arch").to_owned();
    let current = param(&call, "currentVersion").to_owned();
    if !is_segment(&target) || !is_segment(&arch) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    let settings = call.shared.settings();
    let root = update_root(settings);
    let path = root.join("launcher").join(&target).join(&arch).join("latest.json");
    let manifest = read_json_file(&path);
    let decision = decide_launcher_update(
        manifest.as_ref(),
        &target,
        &arch,
        &current,
        &all_prefixes(settings),
        |p| root.join(p).is_file(),
    );
    match decision {
        LauncherDecision::Offered(body) => with_cache(util::ok(&body), NO_CACHE),
        LauncherDecision::NotOffered(reason) => {
            if let Some(reason) = reason.filter(|r| *r != "up to date")
                && LauncherApiState::of(&call.shared).launcher_warning_changed(&path.display().to_string(), reason)
            {
                tracing::warn!("[update] launcher manifest not offered: {reason}");
            }
            with_cache(util::empty(StatusCode::NO_CONTENT), NO_CACHE)
        }
    }
}

/// `GET /launcher/v1/updates/:target/:arch/download`: the latest launcher installer.
pub async fn launcher_download(call: Call) -> Response {
    if download_limited(&call) {
        return too_many();
    }
    let target = param(&call, "target").to_owned();
    let arch = param(&call, "arch").to_owned();
    if !is_segment(&target) || !is_segment(&arch) {
        return status_text(StatusCode::BAD_REQUEST);
    }
    let settings = call.shared.settings();
    let dir = update_root(settings).join("launcher").join(&target).join(&arch);
    let Some(manifest) = read_json_file(&dir.join("latest.json")) else {
        return status_text(StatusCode::NOT_FOUND);
    };
    if let Some(file) = manifest.get("file").and_then(Value::as_str) {
        let version = manifest.get("version").and_then(Value::as_str);
        let (Some(version), true) = (
            version.filter(|v| is_segment(v)),
            basename(file) == file && !file.is_empty(),
        ) else {
            return status_text(StatusCode::NOT_FOUND);
        };
        let path = dir.join(version).join(file);
        if !path.is_file() {
            return status_text(StatusCode::NOT_FOUND);
        }
        return serve_file(&path, NO_CACHE, None).await;
    }
    match manifest.get("url").and_then(Value::as_str) {
        Some(url) if is_allowed_url(url, &configured_prefixes(settings)) => {
            let mut response = (
                StatusCode::FOUND,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                format!("Found. Redirecting to {url}"),
            )
                .into_response();
            util::set_header(&mut response, "location", url);
            util::set_header(&mut response, "vary", "Accept");
            util::set_header(&mut response, "cache-control", NO_CACHE);
            response
        }
        _ => status_text(StatusCode::NOT_FOUND),
    }
}

/// `GET /launcher/v1/updates/:target/:arch/files/:version/:file`: a versioned launcher file.
pub async fn launcher_file(call: Call) -> Response {
    if download_limited(&call) {
        return too_many();
    }
    let target = param(&call, "target");
    let arch = param(&call, "arch");
    let version = param(&call, "version");
    let file = param(&call, "file").to_owned();
    let Some(relative) = launcher_installer_path(target, arch, version, &file) else {
        return status_text(StatusCode::NOT_FOUND);
    };
    let path = update_root(call.shared.settings()).join(relative);
    if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
        return status_text(StatusCode::NOT_FOUND);
    }
    let disposition = format!("attachment; filename=\"{file}\"");
    serve_file(&path, IMMUTABLE, Some(("content-disposition", disposition))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn launcher_rules() {
        assert!(is_launcher_version("0.1.65") && !is_launcher_version("1.2") && !is_launcher_version("123456.1.1"));
        let prefixes = vec!["https://x.example/launcher/v1/updates/".to_owned()];
        assert!(is_allowed_url("https://x.example/launcher/v1/updates/a.exe", &prefixes));
        assert!(!is_allowed_url("https://x.example/launcher/v1/updates/", &prefixes));
        assert!(!is_allowed_url("http://x.example/launcher/v1/updates/a", &prefixes));
        assert!(!is_allowed_url("https://x.example/launcher/v1/updates/a?b", &prefixes));
        assert!(!is_allowed_url("https://evil.example/a", &prefixes));
        assert_eq!(
            launcher_installer_path("windows", "x86_64", "0.2.0", "Rustic_0.2.0_x64-setup.exe"),
            Some(PathBuf::from(
                "launcher/windows/x86_64/0.2.0/Rustic_0.2.0_x64-setup.exe"
            ))
        );
        assert_eq!(launcher_installer_path("windows", "x86_64", "0.2.0", "x.exe"), None);
        assert_eq!(
            launcher_installer_path("windows", "x86_64", "0.2.0", "Rustic_x64-setup.exe"),
            None
        );
        let manifest = json!({"version":"0.2.0","notes":"n","pub_date":"d","url":"https://x.example/launcher/v1/updates/a","signature":"s","extra":1});
        let decision = decide_launcher_update(Some(&manifest), "w", "x", "0.1.0", &prefixes, |_| true);
        assert_eq!(
            decision,
            LauncherDecision::Offered(
                json!({"version":"0.2.0","notes":"n","pub_date":"d","url":"https://x.example/launcher/v1/updates/a","signature":"s"})
            )
        );
        assert_eq!(
            decide_launcher_update(Some(&manifest), "w", "x", "0.2.0", &prefixes, |_| true),
            LauncherDecision::NotOffered(Some("up to date"))
        );
        assert!(matches!(
            decide_launcher_update(Some(&manifest), "w", "x", "garbage", &prefixes, |_| true),
            LauncherDecision::Offered(_)
        ));
        assert_eq!(normalize_address(" ::ffff:10.0.0.1 "), "10.0.0.1");
        assert_eq!(normalize_address("fe80::1%eth0"), "fe80::1");
    }
}
