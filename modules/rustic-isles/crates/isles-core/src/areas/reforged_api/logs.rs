//! Launcher session logs: the tester upload (`routes/launcherLogs.ts`), the names it accepts
//! (`services/launcherLogNames.ts`) and the sync script's read-only listing (`routes/adminLauncherLogs.ts`).

use std::fs;
use std::path::{Path, PathBuf};

use axum::response::Response;
use serde_json::{Value, json};

use super::launcher_auth::{LauncherFailure, has_launcher_auth};
use super::util::{self, BodyKind, Req, constant_time_eq, date_parse, is_rate_limited, iso};
use crate::http::{Call, Handlers, Shared};
use crate::routes::{Method, Service};

const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Per-account caps so a tester can't fill the disk with session ids or re-uploads
const MAX_ACCOUNT_STORAGE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_SESSIONS_PER_ACCOUNT: usize = 200;
pub const TESTER_ROLE: &str = "tester";

/// `LAUNCHER_LOG_STORAGE_DIR`, else `../MysticLauncherLogs`: outside the working tree, so a redeploy never touches it.
pub fn log_storage_root(shared: &Shared) -> PathBuf {
    let root = shared
        .settings
        .raw("LAUNCHER_LOG_STORAGE_DIR")
        .map_or_else(|| PathBuf::from("..").join("MysticLauncherLogs"), PathBuf::from);
    std::path::absolute(&root).unwrap_or(root)
}

/// `/^[a-f0-9-]{8,64}$/i`
pub fn is_session_id(text: &str) -> bool {
    (8..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' | b'-'))
}

/// The files the launcher's `launch::logs` module writes into a session folder; anything else is refused on upload
/// and never listed or read.
pub fn is_allowed_file_name(name: &str) -> bool {
    if name == "metadata.json" || name == "launcher.log" {
        return true;
    }
    // /^runtime-mysticparadox_dll_port\d+\.log$/i
    let lower = name.to_lowercase();
    lower
        .strip_prefix("runtime-mysticparadox_dll_port")
        .and_then(|rest| rest.strip_suffix(".log"))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// `/^[A-Za-z0-9._-]+$/` and not `.` or `..`
pub fn is_safe_segment(value: &str) -> bool {
    super::guard_manifest::is_segment_text(value) && value != "." && value != ".."
}

fn account_usage(root: &Path, user_id: &str) -> (u64, usize) {
    let user_dir = root.join(user_id);
    let Ok(sessions) = fs::read_dir(&user_dir) else {
        return (0, 0);
    };
    let (mut total, mut count) = (0u64, 0usize);
    for session in sessions.flatten() {
        let dir = session.path();
        if !dir.is_dir() {
            continue;
        }
        count += 1;
        for file in fs::read_dir(&dir).into_iter().flatten().flatten() {
            total += file.metadata().map_or(0, |m| m.len());
        }
    }
    (total, count)
}

fn rate_refusal() -> Response {
    util::json(
        429,
        &json!({ "error": { "code": "LOG_UPLOAD_RATE_LIMITED", "message": "Too many uploads. Try again later." } }),
    )
}

fn quota_refusal(message: &str) -> Response {
    util::json(
        413,
        &json!({ "error": { "code": "LOG_UPLOAD_QUOTA", "message": message } }),
    )
}

fn validation(message: &str) -> Response {
    LauncherFailure::new(400, "AUTH_VALIDATION_FAILED", message).response()
}

async fn upload(call: Call) -> Response {
    let (shared, params) = (call.shared.clone(), call.params.clone());
    let (mut req, body) = Req::head(call.request);
    // HasLauncherAuth runs before the route's raw-body parser
    let caller = match has_launcher_auth(&shared, req.header("authorization")).await {
        Ok(caller) => caller,
        Err(failure) => return failure.response(),
    };
    if let Err(response) = req.load(body, BodyKind::Raw(MAX_UPLOAD_BYTES)).await {
        return response;
    }
    if !caller.record.roles.iter().any(|r| r == TESTER_ROLE) {
        return LauncherFailure::new(403, "AUTH_TESTER_REQUIRED", "Tester access is required to upload logs.")
            .response();
    }
    if is_rate_limited(&format!("log-upload:{}", caller.user_id), 120, 3_600_000)
        || is_rate_limited(&format!("log-upload-ip:{}", req.ip()), 300, 3_600_000)
    {
        return rate_refusal();
    }
    let session_id = params.get("launchSessionId").unwrap_or_default();
    let file_name = params.get("fileName").unwrap_or_default();
    if !is_session_id(session_id) || !is_allowed_file_name(file_name) {
        return validation("Invalid session id or file name.");
    }
    let bytes = match req.raw.as_deref() {
        Some(bytes) if !bytes.is_empty() => bytes,
        _ => return validation("The uploaded file was empty."),
    };
    if bytes.len() > MAX_UPLOAD_BYTES {
        return validation("The uploaded file is too large.");
    }
    let root = log_storage_root(&shared);
    let session_dir = root.join(&caller.user_id).join(session_id);
    let is_new_session = !session_dir.exists();
    let (total, sessions) = account_usage(&root, &caller.user_id);
    if is_new_session && sessions >= MAX_SESSIONS_PER_ACCOUNT {
        return quota_refusal("Session limit reached for this account.");
    }
    if total + bytes.len() as u64 > MAX_ACCOUNT_STORAGE_BYTES {
        return quota_refusal("Storage quota reached for this account.");
    }
    let uploaded_at = util::now_iso();
    let written = fs::create_dir_all(&session_dir)
        .and_then(|()| fs::write(session_dir.join(file_name), bytes))
        .and_then(|()| {
            fs::write(
                session_dir.join(".uploaded-at.json"),
                format!("{}\n", json!({ "lastUploadedAt": uploaded_at })),
            )
        });
    match written {
        Ok(()) => util::json(201, &json!({ "uploadedAt": uploaded_at })),
        Err(error) => {
            tracing::error!("[logs] upload not saved: {error}");
            util::empty(500)
        }
    }
}

// ---------------------------------------------------------------- the sync script's reads

/// `LAUNCHER_LOG_SYNC_TOKEN` in `x-sync-token`: a credential of its own for an unattended script.
fn sync_token_valid(shared: &Shared, req: &Req) -> bool {
    let expected = shared
        .settings
        .raw("LAUNCHER_LOG_SYNC_TOKEN")
        .map(service_kit::js::trim)
        .unwrap_or_default();
    let provided = req
        .header("x-sync-token")
        .map(service_kit::js::trim)
        .unwrap_or_default();
    !expected.is_empty() && !provided.is_empty() && constant_time_eq(expected.as_bytes(), provided.as_bytes())
}

fn sync_refusal() -> Response {
    util::json(401, &json!({ "error": "Invalid sync credentials." }))
}

/// One uploaded session as the sync listing shows it; the instant is the marker's, else the folder's.
fn uploaded_at(dir: &Path) -> String {
    let marker = fs::read_to_string(dir.join(".uploaded-at.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|marker| {
            marker
                .get("lastUploadedAt")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });
    marker.unwrap_or_else(|| {
        let modified = fs::metadata(dir)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
        iso(modified)
    })
}

pub fn list_sessions(root: &Path, since: Option<&str>) -> Vec<Value> {
    let Ok(users) = fs::read_dir(root) else {
        return Vec::new();
    };
    // `Date.parse(sinceIso)`; NaN compares false with everything, so an unparseable value filters nothing
    let since = since.filter(|s| !s.is_empty()).and_then(date_parse);
    let mut entries: Vec<(String, Value)> = Vec::new();
    let mut users: Vec<_> = users.flatten().collect();
    users.sort_by_key(|e| e.file_name());
    for user in users {
        let user_id = user.file_name().to_string_lossy().into_owned();
        if !is_safe_segment(&user_id) || !user.path().is_dir() {
            continue;
        }
        let mut sessions: Vec<_> = fs::read_dir(user.path()).into_iter().flatten().flatten().collect();
        sessions.sort_by_key(|e| e.file_name());
        for session in sessions {
            let id = session.file_name().to_string_lossy().into_owned();
            if !is_safe_segment(&id) || !session.path().is_dir() {
                continue;
            }
            let uploaded = uploaded_at(&session.path());
            if let (Some(since), Some(at)) = (since, date_parse(&uploaded))
                && at <= since
            {
                continue;
            }
            let mut files: Vec<String> = fs::read_dir(session.path())
                .into_iter()
                .flatten()
                .flatten()
                .map(|f| f.file_name().to_string_lossy().into_owned())
                .filter(|name| name != ".uploaded-at.json")
                .collect();
            files.sort();
            entries.push((
                uploaded.clone(),
                json!({ "userId": user_id, "launchSessionId": id, "uploadedAt": uploaded, "files": files }),
            ));
        }
    }
    // localeCompare on ISO instants; a stable sort keeps the directory order for equal ones
    entries.sort_by(|a, b| service_kit::collate::locale_compare(&a.0, &b.0));
    entries.into_iter().map(|(_, entry)| entry).collect()
}

async fn sync_list(call: Call) -> Response {
    let shared = call.shared.clone();
    let Ok(req) = Req::read(call.request, BodyKind::Json(100 * 1024)).await else {
        return util::empty(400);
    };
    if !sync_token_valid(&shared, &req) {
        return sync_refusal();
    }
    let root = log_storage_root(&shared);
    let sessions = list_sessions(&root, req.query_string("since"));
    util::json(200, &json!({ "sessions": sessions }))
}

async fn sync_file(call: Call) -> Response {
    let (shared, params) = (call.shared.clone(), call.params.clone());
    let Ok(req) = Req::read(call.request, BodyKind::Json(100 * 1024)).await else {
        return util::empty(400);
    };
    if !sync_token_valid(&shared, &req) {
        return sync_refusal();
    }
    let parts = ["userId", "launchSessionId", "fileName"].map(|name| params.get(name).unwrap_or_default().to_owned());
    if !parts.iter().all(|p| is_safe_segment(p)) {
        return util::json(400, &json!({ "error": "Invalid path." }));
    }
    let path = log_storage_root(&shared)
        .join(&parts[0])
        .join(&parts[1])
        .join(&parts[2]);
    if !path.is_file() {
        return util::send_status(404);
    }
    match fs::read(&path) {
        Ok(bytes) => {
            let mut response = util::text(200, "application/octet-stream", bytes.clone());
            util::set_header(&mut response, "content-length", &bytes.len().to_string());
            response
        }
        Err(_) => util::send_status(404),
    }
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(
        b,
        Method::Put,
        "/launcher/v1/logs/sessions/:launchSessionId/:fileName",
        |call| Box::pin(upload(call)),
    );
    handlers.on(b, Method::Get, "/admin/v1/launcher-logs", |call| {
        Box::pin(sync_list(call))
    });
    handlers.on(
        b,
        Method::Get,
        "/admin/v1/launcher-logs/:userId/:launchSessionId/:fileName",
        |call| Box::pin(sync_file(call)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // services/launcherLogNames.test.ts
    #[test]
    fn only_the_launchers_own_files_are_named() {
        for good in [
            "metadata.json",
            "launcher.log",
            "runtime-mysticparadox_dll_port7777.log",
            "RUNTIME-MysticParadox_DLL_PORT1.LOG",
        ] {
            assert!(is_allowed_file_name(good), "{good}");
        }
        for bad in [
            "",
            "other.log",
            "runtime-mysticparadox_dll_port.log",
            "runtime-mysticparadox_dll_portx.log",
            "../launcher.log",
            "Metadata.json",
            "launcher.log.exe",
        ] {
            assert!(!is_allowed_file_name(bad), "{bad}");
        }
        assert!(
            is_session_id("abcdef12-3456")
                && is_session_id(&"a".repeat(64))
                && !is_session_id("short")
                && !is_session_id("xyzxyzxyz")
        );
        assert!(
            is_safe_segment("a.b-c_d")
                && !is_safe_segment("..")
                && !is_safe_segment(".")
                && !is_safe_segment("a/b")
                && !is_safe_segment("")
        );
    }

    #[test]
    fn the_listing_is_oldest_first_and_filtered_by_since() {
        let dir = tempfile::tempdir().unwrap();
        let session = |user: &str, id: &str, at: &str, files: &[&str]| {
            let d = dir.path().join(user).join(id);
            fs::create_dir_all(&d).unwrap();
            for f in files {
                fs::write(d.join(f), "x").unwrap();
            }
            fs::write(
                d.join(".uploaded-at.json"),
                format!("{{\"lastUploadedAt\":\"{at}\"}}\n"),
            )
            .unwrap();
        };
        session(
            "u1",
            "aaaaaaaa-1",
            "2026-10-02T00:00:00.000Z",
            &["launcher.log", "metadata.json"],
        );
        session("u2", "bbbbbbbb-2", "2026-10-01T00:00:00.000Z", &["launcher.log"]);
        fs::create_dir_all(dir.path().join("bad name").join("aaaaaaaa-9")).unwrap();
        let all = list_sessions(dir.path(), None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["userId"], "u2");
        assert_eq!(all[1]["files"], json!(["launcher.log", "metadata.json"]));
        let later = list_sessions(dir.path(), Some("2026-10-01T12:00:00Z"));
        assert_eq!(later.len(), 1);
        assert_eq!(later[0]["userId"], "u1");
        assert_eq!(list_sessions(dir.path(), Some("garbage")).len(), 2);
        assert!(list_sessions(&dir.path().join("none"), None).is_empty());
    }
}
