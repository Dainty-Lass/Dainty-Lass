//! `routes/launcherUpdates.ts`: the runtime feed (the DLL and its companions, one feed per changelist), the signed
//! Guard manifests, the launcher's own Tauri updater feed and the publisher's switch that pauses the runtime feed.
//!
//! Node-scope: a node running both modules serves both. The runtime and Guard manifests are signed for one
//! `TARGET_CHANGELIST`, so each module keeps its feed under `<root>/runtime/<changelist>/...` and
//! `<root>/guard/<changelist>/...`; the layout Node wrote (`<root>/runtime/<target>/...`) is still read when its
//! manifest names this module's changelist, and never written. The launcher's feed (`launcher/...`) and the serving
//! switch (`serving.json`) are the node's own.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use axum::response::Response;
use serde_json::{Map, Value, json};

use super::guard_manifest::{self, Payload, Trusted, is_segment_text, is_version};
use super::launcher_auth::{LauncherFailure, has_launcher_auth};
use super::probes::update_root;
use super::util::{
    self, BodyKind, NO_CACHE, Req, base64_decode, constant_time_eq, is_rate_limited, now_iso, sha256_hex,
};
use crate::http::{Call, Handlers, Shared};
use crate::routes::{Method, Service};

const RUNTIME_TARGETS: [&str; 2] = ["client", "server"];
const PLATFORM: &str = "windows-x86_64";
const MAX_UPLOAD: usize = 200 * 1024 * 1024;
/// Express's default JSON limit; the route-level parsers come after the app-wide one
const JSON_LIMIT: usize = 100 * 1024;
const DEFAULT_RELEASE_PREFIX: &str = "https://github.com/pranav158/Mystic-Paradox/releases/download/";

// ---------------------------------------------------------------- context

/// The in-crate tests sign their own manifests: these keys stand in for the embedded ones.
#[cfg(test)]
pub(crate) static TEST_TRUSTED: std::sync::RwLock<Option<Trusted>> = std::sync::RwLock::new(None);

/// The settings and paths one request of this area works with.
pub struct Ctx {
    pub root: PathBuf,
    pub changelist: u32,
    pub base_url: String,
    publisher_ips: Vec<String>,
    publisher_key: String,
    extra_files: Vec<String>,
    guard_roles: Vec<&'static str>,
    pub trusted: Trusted,
    pub prefixes: Vec<String>,
}

pub fn normalize_ip(value: &str) -> String {
    let ip = value.trim();
    let ip = ip.strip_prefix("::ffff:").unwrap_or(ip);
    let ip = ip.split('%').next().unwrap_or(ip);
    ip.to_lowercase()
}

impl Ctx {
    pub fn new(shared: &Shared) -> Option<Self> {
        #[cfg(test)]
        if let Some(trusted) = TEST_TRUSTED.read().unwrap_or_else(PoisonError::into_inner).clone() {
            return Some(Self::with(shared, trusted));
        }
        Some(Self::with(shared, Trusted::embedded()?))
    }

    pub fn with(shared: &Shared, trusted: Trusted) -> Self {
        let setting = |name: &str| shared.settings.raw(name).unwrap_or_default();
        let base_url = shared
            .settings
            .raw("UPDATE_PUBLIC_BASE_URL")
            .unwrap_or("https://paradox.mysticfox.dev");
        let base_url = base_url.strip_suffix('/').unwrap_or(base_url).to_owned();
        let configured = shared
            .settings
            .raw("LAUNCHER_UPDATE_URL_PREFIXES")
            .unwrap_or(DEFAULT_RELEASE_PREFIX)
            .split(',')
            .map(|prefix| prefix.trim().to_owned())
            .filter(|p| p.starts_with("https://") && p.ends_with('/') && !p.contains(['?', '#']));
        let mut extra_files = vec!["winmm.dll".to_owned()];
        extra_files.extend(
            setting("RUNTIME_EXTRA_FILES")
                .split(',')
                .map(|name| name.trim().to_lowercase())
                .filter(|name| name.len() <= 96 && is_segment_text(name)),
        );
        Self {
            root: update_root(shared),
            changelist: shared.version.tables.target_changelist,
            prefixes: std::iter::once(format!("{base_url}/launcher/v1/updates/"))
                .chain(configured)
                .collect(),
            base_url,
            publisher_ips: setting("UPDATE_PUBLISHER_ALLOWED_IPS")
                .split(',')
                .map(normalize_ip)
                .filter(|ip| !ip.is_empty())
                .collect(),
            publisher_key: setting("UPDATE_PUBLISHER_API_KEY").trim().to_owned(),
            extra_files,
            guard_roles: shared.rifts.guard().guard_manifest_roles().to_vec(),
            trusted,
        }
    }

    fn runtime_dir(&self, target: &str, channel: &str, platform: &str) -> PathBuf {
        self.root
            .join("runtime")
            .join(self.changelist.to_string())
            .join(target)
            .join(channel)
            .join(platform)
    }

    fn legacy_runtime_dir(&self, target: &str, channel: &str, platform: &str) -> PathBuf {
        self.root.join("runtime").join(target).join(channel).join(platform)
    }

    fn guard_dir(&self, channel: &str, platform: &str) -> PathBuf {
        self.root
            .join("guard")
            .join(self.changelist.to_string())
            .join(channel)
            .join(platform)
    }

    fn legacy_guard_dir(&self, channel: &str, platform: &str) -> PathBuf {
        self.root.join("guard").join(channel).join(platform)
    }

    /// The directory holding this module's current runtime manifest, and the manifest.
    pub fn locate_runtime(&self, target: &str, channel: &str, platform: &str) -> Result<Option<(PathBuf, Value)>, ()> {
        let (target, channel, platform) = (segment(target)?, segment(channel)?, segment(platform)?);
        let current = self.runtime_dir(&target, &channel, &platform);
        if let Some(manifest) = read_json(&current.join("latest.json")) {
            return Ok(Some((current, manifest)));
        }
        let legacy = self.legacy_runtime_dir(&target, &channel, &platform);
        Ok(read_json(&legacy.join("latest.json"))
            .filter(|m| m.get("targetChangelist").and_then(Value::as_i64) == Some(i64::from(self.changelist)))
            .map(|manifest| (legacy, manifest)))
    }

    fn locate_guard(&self, channel: &str, platform: &str) -> Result<Option<(PathBuf, Value)>, ()> {
        let (channel, platform) = (segment(channel)?, segment(platform)?);
        let current = self.guard_dir(&channel, &platform);
        if let Some(envelope) = read_json(&current.join("latest.json")) {
            return Ok(Some((current, envelope)));
        }
        let legacy = self.legacy_guard_dir(&channel, &platform);
        // A legacy manifest signed for another changelist is another module's, not this one's
        Ok(read_json(&legacy.join("latest.json"))
            .filter(|envelope| !guard_manifest::signed_for_other_changelist(envelope, self.changelist, &self.trusted))
            .map(|envelope| (legacy, envelope)))
    }

    pub fn verify_envelope(&self, envelope: &Value) -> Result<(String, Payload), String> {
        guard_manifest::parse_and_verify(
            envelope,
            self.changelist,
            &self.guard_roles,
            guard_manifest::now(),
            &self.trusted,
        )
    }

    fn has_valid_publisher_key(&self, req: &Req) -> bool {
        let provided = req.header("x-update-api-key").map(service_kit::js::trim);
        match provided {
            Some(provided) if !self.publisher_key.is_empty() => {
                constant_time_eq(self.publisher_key.as_bytes(), provided.as_bytes())
            }
            _ => false,
        }
    }

    fn is_publisher_ip_allowed(&self, req: &Req) -> bool {
        if self.publisher_ips.is_empty() {
            return false;
        }
        [Some(req.ip()), req.remote_address()]
            .into_iter()
            .flatten()
            .map(|ip| normalize_ip(&ip))
            .any(|ip| !ip.is_empty() && self.publisher_ips.contains(&ip))
    }
}

// ---------------------------------------------------------------- files

/// `segment`: a plain name, never a path.
pub fn segment(value: &str) -> Result<String, ()> {
    if is_segment_text(value) && value != "." && value != ".." {
        Ok(value.to_owned())
    } else {
        Err(())
    }
}

fn read_json(file: &Path) -> Option<Value> {
    let text = fs::read_to_string(file).ok()?;
    service_kit::jsjson::parse(&text).ok()
}

/// `path.basename`.
fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed)
}

/// `JSON.stringify(value, null, 2)` and a newline.
fn pretty(value: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap_or_default())
}

fn serving_flag(root: &Path) -> PathBuf {
    root.join("serving.json")
}

fn runtime_serving_enabled(root: &Path) -> bool {
    read_json(&serving_flag(root)).is_none_or(|flag| flag.get("runtimeEnabled") != Some(&Value::Bool(false)))
}

fn set_runtime_serving(root: &Path, enabled: bool) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    let flag = json!({ "runtimeEnabled": enabled, "updatedAt": now_iso() });
    fs::write(serving_flag(root), pretty(&flag))
}

fn file_response(path: &Path, extra: &[(&'static str, String)]) -> Response {
    match fs::read(path) {
        Ok(bytes) => {
            let mut response = util::text(200, "application/octet-stream", bytes);
            for (name, value) in extra {
                util::set_header(&mut response, name, value);
            }
            response
        }
        Err(_) => util::send_status(404),
    }
}

// ---------------------------------------------------------------- gates

fn rate_limited(req: &Req, bucket: &str) -> Option<Response> {
    let max = if bucket == "update-download" { 180 } else { 30 };
    let ip = normalize_ip(&req.ip());
    let key = format!("{bucket}:{}", if ip.is_empty() { "unknown" } else { &ip });
    is_rate_limited(&key, max, 60_000)
        .then(|| util::json(429, &json!({ "error": "Too many requests. Please retry later." })))
}

/// `isNonStableChannelAllowed`: "stable" stays public; the others need a signed-in admin account.
async fn non_stable_refused(shared: &Shared, req: &Req, channel: &str) -> Option<Response> {
    if channel == "stable" {
        return None;
    }
    match has_launcher_auth(shared, req.header("authorization")).await {
        Err(failure) => Some(failure.response()),
        Ok(caller) if caller.record.roles.iter().any(|r| r == "admin") => None,
        Ok(_) => Some(
            LauncherFailure::new(
                409,
                "AUTH_CHANNEL_MISMATCH",
                "Your account currently requires the stable runtime channel.",
            )
            .response(),
        ),
    }
}

fn publisher_refused(ctx: &Ctx, req: &Req, what: &str, ip_error: &str) -> Option<Response> {
    if !ctx.is_publisher_ip_allowed(req) {
        tracing::warn!("[update] {what} denied for {}", normalize_ip(&req.ip()));
        return Some(util::json(403, &json!({ "error": ip_error })));
    }
    if !ctx.has_valid_publisher_key(req) {
        return Some(util::json(
            401,
            &json!({ "error": "Invalid update publisher credentials." }),
        ));
    }
    None
}

// ---------------------------------------------------------------- the runtime feed

fn serve_runtime_manifest(ctx: &Ctx, target: &str, channel: &str, platform: &str) -> Response {
    if !runtime_serving_enabled(&ctx.root) {
        let mut response = util::send_status(204);
        util::cache_control(&mut response, NO_CACHE);
        return response;
    }
    match ctx.locate_runtime(target, channel, platform) {
        Err(()) => util::send_status(400),
        Ok(None) => util::send_status(404),
        Ok(Some((_, manifest))) => {
            let mut response = util::json(200, &manifest);
            util::cache_control(&mut response, NO_CACHE);
            response
        }
    }
}

fn serve_runtime_download(ctx: &Ctx, target: &str, channel: &str, platform: &str) -> Response {
    let Ok(located) = ctx.locate_runtime(target, channel, platform) else {
        return util::send_status(400);
    };
    let manifest = located.as_ref().map(|(_, manifest)| manifest);
    let version = match manifest.and_then(|m| m.get("version")).and_then(Value::as_str) {
        Some(version) => match segment(version) {
            Ok(version) => Some(version),
            Err(()) => return util::send_status(400),
        },
        None => None,
    };
    let file = manifest
        .and_then(|m| m.get("file"))
        .and_then(Value::as_str)
        .filter(|file| basename(file) == *file);
    let (Some((dir, _)), Some(version), Some(file)) = (&located, version, file) else {
        return util::send_status(404);
    };
    let artifact = dir.join(version).join(file);
    if !artifact.exists() {
        return util::send_status(404);
    }
    let size = fs::metadata(&artifact).map_or(0, |m| m.len());
    file_response(
        &artifact,
        &[
            ("content-length", size.to_string()),
            ("cache-control", "public, max-age=31536000, immutable".to_owned()),
        ],
    )
}

fn runtime_target(target: &str) -> bool {
    RUNTIME_TARGETS.contains(&target)
}

type Read = (std::sync::Arc<Shared>, Req, crate::routes::Params);

async fn read_get(call: Call) -> Result<Read, Response> {
    let (shared, params) = (call.shared.clone(), call.params.clone());
    let req = Req::read(call.request, BodyKind::None).await?;
    Ok((shared, req, params))
}

macro_rules! ctx_or_500 {
    ($shared:expr) => {
        match Ctx::new(&$shared) {
            Some(ctx) => ctx,
            None => return util::empty(500),
        }
    };
}

async fn runtime_manifest(call: Call, with_target: bool) -> Response {
    let (shared, req, params) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    let (channel, platform) = (
        params.get("channel").unwrap_or_default(),
        params.get("platform").unwrap_or_default(),
    );
    let target = if with_target {
        params.get("target").unwrap_or_default()
    } else {
        "client"
    };
    if with_target && !runtime_target(target) {
        return util::send_status(400);
    }
    if let Some(refused) = non_stable_refused(&shared, &req, channel).await {
        return refused;
    }
    serve_runtime_manifest(&ctx, target, channel, platform)
}

async fn runtime_download(call: Call, with_target: bool) -> Response {
    let (shared, req, params) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    if let Some(limited) = rate_limited(&req, "update-download") {
        return limited;
    }
    let (channel, platform) = (
        params.get("channel").unwrap_or_default(),
        params.get("platform").unwrap_or_default(),
    );
    let target = if with_target {
        params.get("target").unwrap_or_default()
    } else {
        "client"
    };
    if with_target && !runtime_target(target) {
        return util::send_status(400);
    }
    if let Some(refused) = non_stable_refused(&shared, &req, channel).await {
        return refused;
    }
    serve_runtime_download(&ctx, target, channel, platform)
}

async fn runtime_extra(call: Call) -> Response {
    let (shared, req, params) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    if let Some(limited) = rate_limited(&req, "update-download") {
        return limited;
    }
    let target = params.get("target").unwrap_or_default();
    if !runtime_target(target) {
        return util::send_status(400);
    }
    let channel = params.get("channel").unwrap_or_default();
    if let Some(refused) = non_stable_refused(&shared, &req, channel).await {
        return refused;
    }
    let platform = params.get("platform").unwrap_or_default();
    let filename = params.get("filename").unwrap_or_default();
    let Ok(filename) = segment(basename(filename)) else {
        return util::send_status(400);
    };
    let Ok(located) = ctx.locate_runtime(target, channel, platform) else {
        return util::send_status(400);
    };
    let Some((dir, manifest)) = located else {
        return util::send_status(404);
    };
    let version = match manifest.get("version").and_then(Value::as_str).map(segment) {
        Some(Ok(version)) => version,
        Some(Err(())) => return util::send_status(400),
        None => return util::send_status(404),
    };
    let known = manifest
        .get("extraFiles")
        .and_then(Value::as_array)
        .is_some_and(|extras| {
            extras
                .iter()
                .any(|e| e.get("name").and_then(Value::as_str) == Some(filename.as_str()))
        });
    let artifact = dir.join(version).join(&filename);
    if !known || !artifact.exists() {
        return util::send_status(404);
    }
    let size = fs::metadata(&artifact).map_or(0, |m| m.len());
    file_response(
        &artifact,
        &[
            ("content-length", size.to_string()),
            ("cache-control", "public, max-age=31536000, immutable".to_owned()),
        ],
    )
}

// ---------------------------------------------------------------- publishing

/// `encodeURIComponent`: everything but letters, digits and `-_.!~*'()`.
pub(super) const COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

fn encode_uri_component(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, COMPONENT).to_string()
}

/// `POST /launcher/v1/admin/updates/runtime/:target/:channel/:platform`.
pub fn publish_runtime(ctx: &Ctx, req: &Req, params: &crate::routes::Params) -> Response {
    if let Some(limited) = rate_limited(req, "update-publish") {
        return limited;
    }
    if let Some(refused) = publisher_refused(
        ctx,
        req,
        "publish",
        "Update publishing is not allowed from this address.",
    ) {
        return refused;
    }
    let bad_meta = || util::json(400, &json!({ "error": "Invalid runtime update metadata." }));
    let parts = (
        segment(params.get("target").unwrap_or_default()),
        segment(params.get("channel").unwrap_or_default()),
        segment(params.get("platform").unwrap_or_default()),
        segment(req.header("x-update-version").unwrap_or_default()),
    );
    let (Ok(target), Ok(channel), Ok(platform), Ok(version)) = parts else {
        return bad_meta();
    };
    let declared = service_kit::js::number(
        req.header("x-update-changelist")
            .map(|h| Value::String(h.to_owned()))
            .as_ref(),
    );
    let signature_text = req.header("x-update-signature").unwrap_or_default().to_owned();
    if !runtime_target(&target)
        || platform != PLATFORM
        || !is_version(&version)
        || declared != f64::from(ctx.changelist)
        || req.raw.is_none()
    {
        return bad_meta();
    }
    let bytes = req.raw.as_deref().unwrap_or_default();
    if bytes.is_empty() || bytes.len() > MAX_UPLOAD {
        return util::json(413, &json!({ "error": "Runtime update is outside the allowed size." }));
    }
    let signature = base64_decode(&signature_text);
    if signature.len() != 64 || !ctx.trusted.verify_runtime(bytes, &signature) {
        return util::json(
            400,
            &json!({ "error": "Runtime update signature verification failed." }),
        );
    }
    let sha256 = sha256_hex(bytes);
    let artifact_name = format!("ParadoxRuntime-{version}-{target}-{}.dll", ctx.changelist);
    let latest_dir = ctx.runtime_dir(&target, &channel, &platform);
    let artifact_dir = latest_dir.join(&version);
    let written = (|| -> std::io::Result<bool> {
        fs::create_dir_all(&artifact_dir)?;
        let artifact_path = artifact_dir.join(&artifact_name);
        if artifact_path.exists() {
            return Ok(false);
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&artifact_path)?
            .write_all(bytes)?;
        let manifest = json!({
            "schema": 1,
            "component": "ParadoxRuntime",
            "target": target,
            "version": version,
            "channel": channel,
            "targetChangelist": ctx.changelist,
            "platform": platform,
            "size": bytes.len(),
            "sha256": sha256,
            "signature": signature_text,
            "file": artifact_name,
            "publishedAt": now_iso(),
            "url": format!(
                "{}/launcher/v1/runtime/{}/{}/{platform}/download",
                ctx.base_url,
                encode_uri_component(&target),
                encode_uri_component(&channel)
            ),
            "extraFiles": [],
        });
        fs::write(artifact_dir.join("manifest.json"), pretty(&manifest))?;
        fs::write(latest_dir.join("latest.json"), pretty(&manifest))?;
        Ok(true)
    })();
    match written {
        Ok(true) => util::json(
            201,
            &json!({ "target": target, "channel": channel, "version": version, "sha256": sha256 }),
        ),
        Ok(false) => util::json(
            409,
            &json!({ "error": "That target/channel/version is already published and immutable." }),
        ),
        Err(error) => {
            tracing::error!("[update] publish failed: {error}");
            util::empty(500)
        }
    }
}

/// `POST .../:platform/extra`: an allowlisted, separately signed companion of the published version.
pub fn publish_extra(ctx: &Ctx, req: &Req, params: &crate::routes::Params) -> Response {
    if let Some(limited) = rate_limited(req, "update-publish") {
        return limited;
    }
    if let Some(refused) = publisher_refused(
        ctx,
        req,
        "extra-file publish",
        "Update publishing is not allowed from this address.",
    ) {
        return refused;
    }
    let bad_meta = || util::json(400, &json!({ "error": "Invalid extra-file metadata." }));
    let filename_header = req.header("x-update-filename").unwrap_or_default();
    let parts = (
        segment(params.get("target").unwrap_or_default()),
        segment(params.get("channel").unwrap_or_default()),
        segment(params.get("platform").unwrap_or_default()),
        segment(req.header("x-update-version").unwrap_or_default()),
        segment(basename(filename_header)),
    );
    let (Ok(target), Ok(channel), Ok(platform), Ok(version), Ok(filename)) = parts else {
        return bad_meta();
    };
    if !runtime_target(&target) || platform != PLATFORM || !ctx.extra_files.contains(&filename.to_lowercase()) {
        return util::json(400, &json!({ "error": "Invalid extra-file target/platform/name." }));
    }
    let bytes = match req.raw.as_deref() {
        Some(bytes) if !bytes.is_empty() && bytes.len() <= MAX_UPLOAD => bytes,
        _ => return util::json(413, &json!({ "error": "Extra file is outside the allowed size." })),
    };
    let signature = base64_decode(req.header("x-update-signature").unwrap_or_default());
    if signature.len() != 64 || !ctx.trusted.verify_runtime(bytes, &signature) {
        return util::json(400, &json!({ "error": "Extra file signature verification failed." }));
    }
    let latest_dir = ctx.runtime_dir(&target, &channel, &platform);
    let version_dir = latest_dir.join(&version);
    let manifest = read_json(&latest_dir.join("latest.json"));
    let Some(mut manifest) =
        manifest.filter(|m| version_dir.exists() && m.get("version").and_then(Value::as_str) == Some(version.as_str()))
    else {
        return util::json(
            409,
            &json!({ "error": "Publish the main runtime DLL for this version first." }),
        );
    };
    let sha256 = sha256_hex(bytes);
    let result = (|| -> std::io::Result<()> {
        fs::write(version_dir.join(&filename), bytes)?;
        let extra = json!({
            "name": filename,
            "size": bytes.len(),
            "sha256": sha256,
            "signature": util::base64_encode(&signature),
            "url": format!(
                "{}/launcher/v1/runtime/{}/{}/{platform}/extra/{}",
                ctx.base_url,
                encode_uri_component(&target),
                encode_uri_component(&channel),
                encode_uri_component(&filename)
            ),
        });
        let mut extras: Vec<Value> = manifest
            .get("extraFiles")
            .and_then(Value::as_array)
            .map(|extras| {
                extras
                    .iter()
                    .filter(|e| e.is_object() && e.get("name").and_then(Value::as_str) != Some(filename.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        extras.push(extra);
        if let Value::Object(map) = &mut manifest {
            map.insert("extraFiles".into(), Value::Array(extras));
        }
        fs::write(version_dir.join("manifest.json"), pretty(&manifest))?;
        fs::write(latest_dir.join("latest.json"), pretty(&manifest))?;
        Ok(())
    })();
    match result {
        Ok(()) => util::json(201, &json!({ "name": filename, "sha256": sha256, "size": bytes.len() })),
        Err(error) => {
            tracing::error!("[update] extra-file publish failed: {error}");
            util::empty(500)
        }
    }
}

/// `POST /launcher/v1/admin/guard-manifests/:channel/:platform`.
pub fn publish_guard(ctx: &Ctx, req: &Req, params: &crate::routes::Params) -> Response {
    if let Some(limited) = rate_limited(req, "update-publish") {
        return limited;
    }
    if !ctx.is_publisher_ip_allowed(req) {
        return util::json(403, &json!({ "error": "Not allowed from this address." }));
    }
    if !ctx.has_valid_publisher_key(req) {
        return util::json(401, &json!({ "error": "Invalid update publisher credentials." }));
    }
    let outcome = (|| -> Result<Response, String> {
        let invalid = || "Invalid Guard manifest.".to_owned();
        let channel = segment(params.get("channel").unwrap_or_default()).map_err(|()| "Invalid channel".to_owned())?;
        let platform =
            segment(params.get("platform").unwrap_or_default()).map_err(|()| "Invalid platform".to_owned())?;
        let envelope = req
            .body
            .as_ref()
            .ok_or_else(|| "Invalid Guard manifest envelope.".to_owned())?;
        let (key_id, payload) = ctx.verify_envelope(envelope)?;
        if payload.channel != channel || payload.platform != platform {
            return Ok(util::json(
                400,
                &json!({ "error": "Guard manifest route and signed payload do not match." }),
            ));
        }
        let dir = ctx.guard_dir(&channel, &platform);
        let previous = read_json(&dir.join("latest.json")).or_else(|| {
            read_json(&ctx.legacy_guard_dir(&channel, &platform).join("latest.json"))
                .filter(|e| ctx.verify_envelope(e).is_ok())
        });
        if let Some(previous) = previous {
            let (_, prior) = ctx.verify_envelope(&previous)?;
            if payload.sequence <= prior.sequence {
                return Ok(util::json(
                    409,
                    &json!({ "error": "Guard manifest sequence must increase monotonically." }),
                ));
            }
        }
        let version_dir = dir.join(payload.sequence.to_string());
        fs::create_dir_all(&version_dir).map_err(|_| invalid())?;
        let immutable = version_dir.join(format!(
            "{}.json",
            segment(&payload.manifest_id).map_err(|()| "Invalid manifestId".to_owned())?
        ));
        if immutable.exists() {
            return Ok(util::json(
                409,
                &json!({ "error": "That Guard manifest is already published and immutable." }),
            ));
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&immutable)
            .and_then(|mut file| file.write_all(pretty(envelope).as_bytes()))
            .map_err(|_| invalid())?;
        fs::write(dir.join("latest.json"), pretty(envelope)).map_err(|_| invalid())?;
        Ok(util::json(
            201,
            &json!({ "manifestId": payload.manifest_id, "sequence": payload.sequence, "keyId": key_id }),
        ))
    })();
    outcome.unwrap_or_else(|message| util::json(400, &json!({ "error": message })))
}

/// `GET /launcher/v1/guard-manifests/:channel/:platform`.
pub fn serve_guard(ctx: &Ctx, channel: &str, platform: &str) -> Response {
    let Ok(located) = ctx.locate_guard(channel, platform) else {
        return util::send_status(400);
    };
    let Some((_, envelope)) = located else {
        return util::send_status(404);
    };
    match ctx.verify_envelope(&envelope) {
        Err(_) => util::send_status(400),
        Ok((_, payload)) if payload.channel != channel || payload.platform != platform => util::send_status(404),
        Ok(_) => {
            let mut response = util::json(200, &envelope);
            util::cache_control(&mut response, NO_CACHE);
            response
        }
    }
}

/// `GetPublishedGuardManifest`: the verified payload of a channel's current manifest, for the areas that judge a
/// Guard session against it.
pub fn published_guard_manifest(shared: &Shared, channel: &str, platform: &str) -> Option<Payload> {
    let ctx = Ctx::new(shared)?;
    let (_, envelope) = ctx.locate_guard(channel, platform).ok()??;
    let (_, payload) = ctx.verify_envelope(&envelope).ok()?;
    (payload.channel == channel && payload.platform == platform).then_some(payload)
}

/// `ReadPublishedRuntimeManifest`: the runtime feed's current manifest for this module's changelist.
pub fn published_runtime_manifest(shared: &Shared, target: &str, channel: &str, platform: &str) -> Option<Value> {
    Ctx::new(shared)?
        .locate_runtime(target, channel, platform)
        .ok()?
        .map(|(_, manifest)| manifest)
}

/// `ReadPublishedGuardArtifacts`: role, name and SHA-256 of every artifact of a channel's signed manifest.
pub fn published_guard_artifacts(
    shared: &Shared,
    channel: &str,
    platform: &str,
) -> Option<Vec<(String, String, String)>> {
    published_guard_manifest(shared, channel, platform).map(|payload| payload.artifacts)
}

// ---------------------------------------------------------------- the launcher's own feed

const LAUNCHER_VERSION_LEN: std::ops::RangeInclusive<usize> = 1..=5;

/// `/^\d{1,5}\.\d{1,5}\.\d{1,5}$/`
pub fn is_launcher_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| LAUNCHER_VERSION_LEN.contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
}

fn compare_launcher_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |text: &str| {
        text.split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parse(a).cmp(&parse(b))
}

fn is_allowed_launcher_url(url: &Value, prefixes: &[String]) -> bool {
    let Some(url) = url.as_str().filter(|u| u.encode_utf16().count() <= 1024) else {
        return false;
    };
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    if url.contains(['?', '#']) || authority.is_empty() || authority.contains('@') {
        return false;
    }
    prefixes
        .iter()
        .any(|p| url.starts_with(p.as_str()) && url.len() > p.len())
}

/// The reason a launcher manifest is not offered, or `None` when it is.
pub fn decide_launcher_update(
    manifest: Option<&Value>,
    current: &str,
    prefixes: &[String],
    installer_on_disk: &dyn Fn(&str, &str) -> bool,
) -> Option<&'static str> {
    let manifest = manifest?;
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| is_launcher_version(v));
    let Some(version) = version else {
        return Some("the manifest has no x.y.z version");
    };
    if manifest
        .get("signature")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Some("the manifest has no signature");
    }
    if !manifest
        .get("url")
        .is_some_and(|u| is_allowed_launcher_url(u, prefixes))
    {
        return Some("the manifest's download url is not allowed");
    }
    if is_launcher_version(current) && compare_launcher_versions(version, current).is_le() {
        return Some("up to date");
    }
    if let Some(file) = manifest.get("file").and_then(Value::as_str)
        && !installer_on_disk(version, file)
    {
        return Some("the manifest's installer is not on this server");
    }
    None
}

/// `UpdaterManifest`: Tauri's own fields, in its order.
pub fn updater_manifest(manifest: &Value) -> Value {
    let mut out = Map::new();
    for key in ["version", "notes", "pub_date", "url", "signature"] {
        if let Some(value) = manifest.get(key) {
            out.insert(key.to_owned(), value.clone());
        }
    }
    Value::Object(out)
}

/// `LauncherInstallerPath`: a self-hosted installer of that version, or `None` for a name that isn't one.
pub fn launcher_installer_path(root: &Path, target: &str, arch: &str, version: &str, file: &str) -> Option<PathBuf> {
    let name_ok = file.len() <= 120 + "_x64-setup.exe".len()
        && file.strip_suffix("_x64-setup.exe").is_some_and(|stem| {
            (1..=120).contains(&stem.len())
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        });
    if !is_launcher_version(version) || !name_ok || !file.contains(&format!("_{version}_")) {
        return None;
    }
    Some(
        root.join("launcher")
            .join(segment(target).ok()?)
            .join(segment(arch).ok()?)
            .join(version)
            .join(file),
    )
}

enum Download {
    File { version: String, file: String },
    Redirect(String),
}

fn launcher_download_target(manifest: Option<&Value>, prefixes: &[String]) -> Option<Download> {
    let manifest = manifest?;
    if let Some(file) = manifest.get("file").and_then(Value::as_str) {
        if basename(file) != file {
            return None;
        }
        let version = segment(manifest.get("version")?.as_str()?).ok()?;
        return Some(Download::File {
            version,
            file: file.to_owned(),
        });
    }
    let url = manifest.get("url")?;
    is_allowed_launcher_url(url, prefixes.get(1..).unwrap_or_default())
        .then(|| Download::Redirect(url.as_str().unwrap_or_default().to_owned()))
}

/// `ReadServedLauncherVersion`.
pub fn served_launcher_version(shared: &Shared, target: &str, arch: &str) -> Option<String> {
    let root = update_root(shared);
    let manifest = read_json(
        &root
            .join("launcher")
            .join(segment(target).ok()?)
            .join(segment(arch).ok()?)
            .join("latest.json"),
    )?;
    manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| is_launcher_version(v))
        .map(str::to_owned)
}

static LAST_REFUSED: Mutex<String> = Mutex::new(String::new());

async fn launcher_download(call: Call) -> Response {
    let (shared, req, params) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    if let Some(limited) = rate_limited(&req, "update-download") {
        return limited;
    }
    let (Ok(target), Ok(arch)) = (
        segment(params.get("target").unwrap_or_default()),
        segment(params.get("arch").unwrap_or_default()),
    ) else {
        return util::send_status(400);
    };
    let dir = ctx.root.join("launcher").join(&target).join(&arch);
    let manifest = read_json(&dir.join("latest.json"));
    match launcher_download_target(manifest.as_ref(), &ctx.prefixes) {
        Some(Download::Redirect(url)) => {
            let mut response = util::redirect(302, &url);
            util::cache_control(&mut response, NO_CACHE);
            response
        }
        None => util::send_status(404),
        Some(Download::File { version, file }) => {
            let artifact = dir.join(version).join(file);
            if !artifact.exists() {
                return util::send_status(404);
            }
            let size = fs::metadata(&artifact).map_or(0, |m| m.len());
            // This url serves whichever version is current, so it must not be cached
            file_response(
                &artifact,
                &[
                    ("content-length", size.to_string()),
                    ("cache-control", NO_CACHE.to_owned()),
                ],
            )
        }
    }
}

async fn launcher_file(call: Call) -> Response {
    let (shared, req, params) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    if let Some(limited) = rate_limited(&req, "update-download") {
        return limited;
    }
    let get = |name: &str| params.get(name).unwrap_or_default().to_owned();
    let artifact = launcher_installer_path(&ctx.root, &get("target"), &get("arch"), &get("version"), &get("file"));
    let Some(artifact) = artifact.filter(|a| a.is_file()) else {
        return util::send_status(404);
    };
    let size = fs::metadata(&artifact).map_or(0, |m| m.len());
    let name = artifact
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    file_response(
        &artifact,
        &[
            ("content-length", size.to_string()),
            ("content-disposition", format!("attachment; filename=\"{name}\"")),
            ("cache-control", "public, max-age=31536000, immutable".to_owned()),
        ],
    )
}

async fn launcher_manifest(call: Call) -> Response {
    let (shared, _req, params) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    let (Ok(target), Ok(arch)) = (
        segment(params.get("target").unwrap_or_default()),
        segment(params.get("arch").unwrap_or_default()),
    ) else {
        return util::send_status(400);
    };
    let file = ctx.root.join("launcher").join(&target).join(&arch).join("latest.json");
    let manifest = read_json(&file);
    let on_disk = |version: &str, name: &str| {
        let strict = launcher_installer_path(&ctx.root, &target, &arch, version, name);
        let legacy = (basename(name) == name).then(|| {
            ctx.root
                .join("launcher")
                .join(&target)
                .join(&arch)
                .join(version)
                .join(name)
        });
        [strict, legacy]
            .into_iter()
            .flatten()
            .any(|candidate| candidate.exists())
    };
    let current = params.get("currentVersion").unwrap_or_default();
    let refusal = decide_launcher_update(manifest.as_ref(), current, &ctx.prefixes, &on_disk);
    let mut response = match (refusal, &manifest) {
        (None, Some(manifest)) => util::json(200, &updater_manifest(manifest)),
        (reason, manifest) => {
            // An unusable manifest is not offered; logged once per change, not per launcher check
            if let (Some(reason), Some(_)) = (reason, manifest)
                && reason != "up to date"
            {
                let key = format!("{}:{reason}", file.display());
                let mut last = LAST_REFUSED.lock().unwrap_or_else(PoisonError::into_inner);
                if *last != key {
                    tracing::warn!("[update] launcher manifest not offered: {reason}");
                    *last = key;
                }
            }
            util::send_status(204)
        }
    };
    util::cache_control(&mut response, NO_CACHE);
    response
}

// ---------------------------------------------------------------- the serving switch

async fn serving_get(call: Call) -> Response {
    let (shared, req, _) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    if !ctx.is_publisher_ip_allowed(&req) {
        return util::json(403, &json!({ "error": "Not allowed from this address." }));
    }
    if !ctx.has_valid_publisher_key(&req) {
        return util::json(401, &json!({ "error": "Invalid update publisher credentials." }));
    }
    util::json(200, &json!({ "runtimeEnabled": runtime_serving_enabled(&ctx.root) }))
}

async fn serving_post(call: Call) -> Response {
    let shared = call.shared.clone();
    let req = match Req::read(call.request, BodyKind::Json(JSON_LIMIT)).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    if !ctx.is_publisher_ip_allowed(&req) {
        tracing::warn!(
            "[update] serving toggle denied for {} (not in UPDATE_PUBLISHER_ALLOWED_IPS)",
            normalize_ip(&req.ip())
        );
        return util::json(403, &json!({ "error": "Not allowed from this address." }));
    }
    if !ctx.has_valid_publisher_key(&req) {
        return util::json(401, &json!({ "error": "Invalid update publisher credentials." }));
    }
    let Some(enabled) = req.fields().get("runtimeEnabled").and_then(Value::as_bool) else {
        return util::json(400, &json!({ "error": "runtimeEnabled must be a boolean." }));
    };
    if let Err(error) = set_runtime_serving(&ctx.root, enabled) {
        tracing::error!("[update] serving switch not saved: {error}");
        return util::empty(500);
    }
    tracing::warn!(
        "[update] runtime update serving {} by {}",
        if enabled { "ENABLED" } else { "DISABLED" },
        normalize_ip(&req.ip())
    );
    util::json(200, &json!({ "runtimeEnabled": enabled }))
}

async fn guard_get(call: Call) -> Response {
    let (shared, req, params) = match read_get(call).await {
        Ok(read) => read,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    let channel = params.get("channel").unwrap_or_default();
    if let Some(refused) = non_stable_refused(&shared, &req, channel).await {
        return refused;
    }
    serve_guard(&ctx, channel, params.get("platform").unwrap_or_default())
}

async fn guard_post(call: Call) -> Response {
    let (shared, params) = (call.shared.clone(), call.params.clone());
    let req = match Req::read(call.request, BodyKind::Json(JSON_LIMIT)).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    publish_guard(&ctx, &req, &params)
}

async fn upload(call: Call, extra: bool) -> Response {
    let (shared, params) = (call.shared.clone(), call.params.clone());
    let req = match Req::read(call.request, BodyKind::Raw(MAX_UPLOAD)).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let ctx = ctx_or_500!(shared);
    let ctx_run = move || {
        if extra {
            publish_extra(&ctx, &req, &params)
        } else {
            publish_runtime(&ctx, &req, &params)
        }
    };
    tokio::task::spawn_blocking(ctx_run)
        .await
        .unwrap_or_else(|_| util::empty(500))
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(
        b,
        Method::Get,
        "/launcher/v1/runtime/:target/:channel/:platform",
        |call| Box::pin(runtime_manifest(call, true)),
    );
    handlers.on(b, Method::Get, "/launcher/v1/runtime/:channel/:platform", |call| {
        Box::pin(runtime_manifest(call, false))
    });
    handlers.on(
        b,
        Method::Get,
        "/launcher/v1/runtime/:target/:channel/:platform/download",
        |call| Box::pin(runtime_download(call, true)),
    );
    // Shadowed by the three-segment manifest route above, as in Node (RU3a); registered all the same
    handlers.on(
        b,
        Method::Get,
        "/launcher/v1/runtime/:channel/:platform/download",
        |call| Box::pin(runtime_download(call, false)),
    );
    handlers.on(
        b,
        Method::Get,
        "/launcher/v1/guard-manifests/:channel/:platform",
        |call| Box::pin(guard_get(call)),
    );
    handlers.on(
        b,
        Method::Post,
        "/launcher/v1/admin/guard-manifests/:channel/:platform",
        |call| Box::pin(guard_post(call)),
    );
    handlers.on(
        b,
        Method::Post,
        "/launcher/v1/admin/updates/runtime/:target/:channel/:platform",
        |call| Box::pin(upload(call, false)),
    );
    handlers.on(
        b,
        Method::Post,
        "/launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra",
        |call| Box::pin(upload(call, true)),
    );
    handlers.on(
        b,
        Method::Get,
        "/launcher/v1/runtime/:target/:channel/:platform/extra/:filename",
        |call| Box::pin(runtime_extra(call)),
    );
    handlers.on(b, Method::Get, "/launcher/v1/admin/updates/serving", |call| {
        Box::pin(serving_get(call))
    });
    handlers.on(b, Method::Post, "/launcher/v1/admin/updates/serving", |call| {
        Box::pin(serving_post(call))
    });
    handlers.on(b, Method::Get, "/launcher/v1/updates/:target/:arch/download", |call| {
        Box::pin(launcher_download(call))
    });
    handlers.on(
        b,
        Method::Get,
        "/launcher/v1/updates/:target/:arch/files/:version/:file",
        |call| Box::pin(launcher_file(call)),
    );
    handlers.on(
        b,
        Method::Get,
        "/launcher/v1/updates/:target/:arch/:currentVersion",
        |call| Box::pin(launcher_manifest(call)),
    );
}
