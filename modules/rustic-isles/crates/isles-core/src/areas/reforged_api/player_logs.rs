//! The operator's view of the tester uploads (`services/playerLogs.ts`, `controllers/adminPlayerLogs.ts`) and the
//! retention rules of the folders themselves (`services/launcherLogRetention.ts`).
//!
//! Paths are checked before any file is touched: a segment is a plain name, a file carries one of the launcher's
//! names, a folder or file that is a symbolic link is never followed, and a session resolves inside the root.
//!
//! The Privacy Policy owes two periodic passes over these folders: [`purge_expired`] (90 days after the last upload)
//! and [`remove_account_logs`] (with the account). The module start schedules them; neither is scheduled from here.

use std::fs;
use std::path::{Path, PathBuf};

use axum::response::Response;
use serde_json::{Value, json};

use super::admin::{Ctx, route};
use super::logs::{is_allowed_file_name, is_safe_segment, is_session_id, log_storage_root};
use super::util::{self, date_parse, iso};
use crate::http::Handlers;
use crate::repo::reforged::AdminAuditRecord;
use crate::routes::{Method, Service};

pub const FILE_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub const LIST_DEFAULT_LIMIT: usize = 50;
pub const LIST_MAX_LIMIT: usize = 200;
const REASON_MIN: usize = 8;
const REASON_MAX: usize = 200;
const DAY_MS: i64 = 86_400_000;
/// `UPLOADED_LAUNCHER_LOG_RETENTION_DAYS`
pub const RETENTION_DAYS: i64 = 90;

/// An uploaded session: `<root>/<account>/<session>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Uploaded {
    pub user_id: String,
    pub session_id: String,
    pub dir: PathBuf,
    pub uploaded_at_ms: f64,
}

/// `/^[A-Za-z0-9-]{1,64}$/`
fn is_account_folder(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// `UploadedAtOf`: the marker's time, else the folder's own.
pub fn uploaded_at_of(session_dir: &Path) -> f64 {
    let recorded = fs::read_to_string(session_dir.join(".uploaded-at.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("lastUploadedAt").and_then(Value::as_str).and_then(date_parse));
    if let Some(ms) = recorded {
        return ms as f64;
    }
    fs::metadata(session_dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// `ListUploadedLogSessions`: every session under `root` in plain-named folders; links are skipped.
pub fn list_uploaded(root: &Path, user_id: Option<&str>) -> Vec<Uploaded> {
    if !root.exists() {
        return Vec::new();
    }
    let accounts: Vec<String> = match user_id {
        Some(id) => vec![id.to_owned()],
        None => fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
    };
    let mut sessions = Vec::new();
    for user in accounts {
        let user_dir = root.join(&user);
        if !is_account_folder(&user) || !is_real_directory(&user_dir) {
            continue;
        }
        for entry in fs::read_dir(&user_dir).into_iter().flatten().flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            let dir = user_dir.join(&id);
            if !is_session_id(&id) || !is_real_directory(&dir) {
                continue;
            }
            let uploaded_at_ms = uploaded_at_of(&dir);
            sessions.push(Uploaded {
                user_id: user.clone(),
                session_id: id,
                dir,
                uploaded_at_ms,
            });
        }
    }
    sessions
}

/// `ExpiredUploadedLogSessions`.
pub fn expired(sessions: &[Uploaded], now_ms: i64, retention_days: i64) -> Vec<&Uploaded> {
    let cutoff = (now_ms - retention_days * DAY_MS) as f64;
    sessions.iter().filter(|s| s.uploaded_at_ms < cutoff).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionPass {
    pub checked: usize,
    pub expired: usize,
    pub removed: usize,
    pub failed: usize,
}

/// `PurgeExpiredUploadedLogs`: removes the sessions whose last upload is past the retention period.
pub fn purge_expired(root: &Path, now_ms: i64, retention_days: i64) -> RetentionPass {
    let all = list_uploaded(root, None);
    let old = expired(&all, now_ms, retention_days);
    let mut pass = RetentionPass {
        checked: all.len(),
        expired: old.len(),
        ..RetentionPass::default()
    };
    for session in old {
        match fs::remove_dir_all(&session.dir) {
            Ok(()) => pass.removed += 1,
            Err(_) => pass.failed += 1,
        }
    }
    pass
}

/// `RemoveUploadedLogsOfAccount`: the account's folder, with the count of its sessions; `Err` for an id that isn't a
/// plain folder name.
pub fn remove_account_logs(root: &Path, user_id: &str) -> Result<usize, &'static str> {
    if !is_account_folder(user_id) {
        return Err("Refusing to remove uploaded logs for an account id that is not a plain folder name");
    }
    let dir = root.join(user_id);
    if !dir.exists() {
        return Ok(0);
    }
    let count = list_uploaded(root, None)
        .iter()
        .filter(|s| s.user_id == user_id)
        .count();
    fs::remove_dir_all(&dir).map_err(|_| "The folder could not be removed")?;
    Ok(count)
}

// ---------------------------------------------------------------- the operator's view

#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub status: u16,
    pub code: &'static str,
    pub message: &'static str,
}

fn refuse(status: u16, code: &'static str, message: &'static str) -> Refusal {
    Refusal { status, code, message }
}

fn files_of(session_dir: &Path) -> Vec<(String, u64)> {
    let mut files: Vec<(String, u64)> = fs::read_dir(session_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            (is_allowed_file_name(&name) && meta.is_file()).then_some((name, meta.len()))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// One session as the list shows it.
pub fn summary(session: &Uploaded, uploaded_ms: i64) -> Value {
    let files = files_of(&session.dir);
    let total: u64 = files.iter().map(|f| f.1).sum();
    json!({
        "userId": session.user_id,
        "launchSessionId": session.session_id,
        "uploadedAt": iso(uploaded_ms),
        "files": files.iter().map(|(name, size)| json!({ "name": name, "size": size })).collect::<Vec<_>>(),
        "totalBytes": total,
    })
}

/// `ListPlayerLogSessions`: newest first, paged by an exclusive upper bound on the upload time.
pub fn list_sessions(root: &Path, user_id: Option<&str>, limit: usize, before: Option<i64>) -> Value {
    if user_id.is_some_and(|id| !is_safe_segment(id)) {
        return json!({ "sessions": [], "nextBefore": null });
    }
    // Whole milliseconds: the cursor is an ISO string, and a fraction would move a session across the boundary
    let mut sorted: Vec<(Uploaded, i64)> = list_uploaded(root, user_id)
        .into_iter()
        .map(|s| {
            let ms = s.uploaded_at_ms.floor() as i64;
            (s, ms)
        })
        .filter(|(_, ms)| before.is_none_or(|b| *ms < b))
        .collect();
    sorted.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.user_id.cmp(&b.0.user_id))
            .then_with(|| a.0.session_id.cmp(&b.0.session_id))
    });
    let mut take = limit.min(sorted.len());
    if sorted.len() > limit {
        let edge = sorted[limit - 1].1;
        while take < sorted.len() && sorted[take].1 == edge {
            take += 1;
        }
    }
    let page: Vec<Value> = sorted[..take].iter().map(|(s, ms)| summary(s, *ms)).collect();
    let next = (take < sorted.len()).then(|| iso(sorted[take - 1].1));
    json!({ "sessions": page, "nextBefore": next })
}

fn locate(root: &Path, user_id: &str, session_id: &str) -> Result<PathBuf, Refusal> {
    if !is_safe_segment(user_id) || !is_safe_segment(session_id) || !is_session_id(session_id) {
        return Err(refuse(
            400,
            "PLAYER_LOG_PATH",
            "The account or session id is not a valid folder name.",
        ));
    }
    let missing = || refuse(404, "PLAYER_LOG_NOT_FOUND", "No such uploaded session.");
    if !root.exists() {
        return Err(missing());
    }
    let user_dir = root.join(user_id);
    let session_dir = user_dir.join(session_id);
    if !is_real_directory(&user_dir) || !is_real_directory(&session_dir) {
        return Err(missing());
    }
    let root_real = fs::canonicalize(root).map_err(|_| missing())?;
    let session_real = fs::canonicalize(&session_dir).map_err(|_| missing())?;
    match session_real.strip_prefix(&root_real) {
        Ok(rest) if !rest.as_os_str().is_empty() => Ok(session_dir),
        _ => Err(missing()),
    }
}

/// `ReadPlayerLogFile`: the name, the content type and the bytes.
pub fn read_file(
    root: &Path,
    user_id: &str,
    session_id: &str,
    name: &str,
) -> Result<(String, &'static str, Vec<u8>), Refusal> {
    if !is_safe_segment(name) {
        return Err(refuse(
            400,
            "PLAYER_LOG_PATH",
            "The file name is not a valid file name.",
        ));
    }
    if !is_allowed_file_name(name) {
        return Err(refuse(
            400,
            "PLAYER_LOG_NAME_REFUSED",
            "Only the launcher's log files can be read (metadata.json, launcher.log, runtime logs).",
        ));
    }
    let dir = locate(root, user_id, session_id)?;
    let path = dir.join(name);
    let not_found = || refuse(404, "PLAYER_LOG_NOT_FOUND", "No such file in that session.");
    let too_large = || {
        refuse(
            413,
            "PLAYER_LOG_TOO_LARGE",
            "That file is larger than the admin view serves.",
        )
    };
    let meta = fs::symlink_metadata(&path).map_err(|_| not_found())?;
    if !meta.is_file() {
        return Err(not_found());
    }
    if meta.len() > FILE_MAX_BYTES {
        return Err(too_large());
    }
    let bytes = fs::read(&path).map_err(|_| not_found())?;
    if bytes.len() as u64 > FILE_MAX_BYTES {
        return Err(too_large());
    }
    let content_type = if name == "metadata.json" {
        "application/json; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    };
    Ok((name.to_owned(), content_type, bytes))
}

/// `DeletePlayerLogSession`: the session as it was, once removed.
pub fn delete_session(root: &Path, user_id: &str, session_id: &str) -> Result<Value, Refusal> {
    let dir = locate(root, user_id, session_id)?;
    let uploaded = uploaded_at_of(&dir).floor() as i64;
    let summary = summary(
        &Uploaded {
            user_id: user_id.to_owned(),
            session_id: session_id.to_owned(),
            dir: dir.clone(),
            uploaded_at_ms: uploaded as f64,
        },
        uploaded,
    );
    fs::remove_dir_all(&dir).map_err(|_| refuse(404, "PLAYER_LOG_NOT_FOUND", "No such uploaded session."))?;
    Ok(summary)
}

fn refusal(r: &Refusal) -> Response {
    util::json(r.status, &json!({ "error": { "code": r.code, "message": r.message } }))
}

fn query_error(message: &str) -> Response {
    util::json(
        400,
        &json!({ "error": { "code": "PLAYER_LOGS_QUERY", "message": message } }),
    )
}

/// `QueryText`: `None` when absent, `Err` when not a single string.
fn query_text<'a>(ctx: &'a Ctx, name: &str) -> Result<Option<&'a str>, ()> {
    if !ctx.req.query.iter().any(|(k, _)| k == name) {
        return Ok(None);
    }
    ctx.req.query_string(name).map(Some).ok_or(())
}

async fn list(ctx: Ctx) -> Response {
    let user = match query_text(&ctx, "userId") {
        Ok(Some(id)) if !id.is_empty() && is_safe_segment(id) => Some(id),
        Ok(None | Some("")) => None,
        _ => return query_error("userId must be one account id."),
    };
    let mut limit = LIST_DEFAULT_LIMIT;
    match query_text(&ctx, "limit") {
        Ok(None) => {}
        Ok(Some(text)) => {
            let valid = (1..=3).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
            let value = text
                .parse::<usize>()
                .ok()
                .filter(|v| valid && (1..=LIST_MAX_LIMIT).contains(v));
            match value {
                Some(value) => limit = value,
                None => return query_error(&format!("limit must be a whole number from 1 to {LIST_MAX_LIMIT}.")),
            }
        }
        Err(()) => return query_error(&format!("limit must be a whole number from 1 to {LIST_MAX_LIMIT}.")),
    }
    let message = "before must be a UTC time such as 2026-10-15T12:00:00.000Z.";
    let before = match query_text(&ctx, "before") {
        Ok(None) => None,
        Ok(Some(text)) => {
            // /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,3})?Z$/
            let shape = text.len() >= 20
                && text.ends_with('Z')
                && text.as_bytes()[..19].iter().enumerate().all(|(i, b)| match i {
                    4 | 7 => *b == b'-',
                    10 => *b == b'T',
                    13 | 16 => *b == b':',
                    _ => b.is_ascii_digit(),
                })
                && (text.len() == 20
                    || (text.as_bytes()[19] == b'.'
                        && (1..=3).contains(&(text.len() - 21))
                        && text[20..text.len() - 1].bytes().all(|b| b.is_ascii_digit())));
            match date_parse(text).filter(|_| shape) {
                Some(ms) => Some(ms),
                None => return query_error(message),
            }
        }
        Err(()) => return query_error(message),
    };
    let root = log_storage_root(&ctx.shared);
    util::json(200, &list_sessions(&root, user, limit, before))
}

async fn read(ctx: Ctx) -> Response {
    let download = match query_text(&ctx, "download") {
        Ok(None) => false,
        Ok(Some("1")) => true,
        Ok(Some("0")) => false,
        _ => return query_error("download must be 1 or 0."),
    };
    let get = |name: &str| ctx.params.get(name).unwrap_or_default().to_owned();
    let root = log_storage_root(&ctx.shared);
    match read_file(&root, &get("userId"), &get("launchSessionId"), &get("fileName")) {
        Err(r) => refusal(&r),
        Ok((name, content_type, bytes)) => {
            let mut response = util::text(200, content_type, bytes.clone());
            util::set_header(&mut response, "content-length", &bytes.len().to_string());
            util::set_header(&mut response, "x-content-type-options", "nosniff");
            if download {
                util::set_header(
                    &mut response,
                    "content-disposition",
                    &format!("attachment; filename=\"{name}\""),
                );
            }
            response
        }
    }
}

async fn delete(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    let user_id = ctx.params.get("userId").unwrap_or_default().to_owned();
    let session_id = ctx.params.get("launchSessionId").unwrap_or_default().to_owned();
    let reason = ctx
        .req
        .fields()
        .get("reason")
        .and_then(Value::as_str)
        .map_or_else(String::new, |r| service_kit::js::trim(r).to_owned());
    if !(REASON_MIN..=REASON_MAX).contains(&reason.encode_utf16().count()) {
        return util::json(
            400,
            &json!({ "error": { "code": "PLAYER_LOG_REASON", "message":
                format!("Give a reason of {REASON_MIN} to {REASON_MAX} characters for the audit log.") } }),
        );
    }
    let root = log_storage_root(&ctx.shared);
    let removed = match delete_session(&root, &user_id, &session_id) {
        Ok(removed) => removed,
        Err(r) => return refusal(&r),
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let record = AdminAuditRecord {
        id: uuid::Uuid::new_v4().to_string(),
        actor_user_id: caller.user_id.clone(),
        target_user_id: Some(user_id.clone()),
        action: "player_logs.delete".into(),
        old_state: Some(json!({
            "launchSessionId": removed["launchSessionId"], "uploadedAt": removed["uploadedAt"],
            "files": removed["files"], "totalBytes": removed["totalBytes"],
        })),
        new_state: Some(json!({ "deleted": true })),
        reason: Some(reason),
        ip: ctx.req.ip(),
        request_id: request_id.clone(),
        created_at: util::now_iso(),
    };
    let actor = module_api::ActorContext::new(module_api::Actor::Admin { key: caller.user_id });
    if let Err(error) = ctx
        .store
        .write(&actor, move |tx, repos| repos.admin.append_audit(tx, &record))
        .await
    {
        tracing::error!(
            "[PlayerLogs] a session was removed but its audit record could not be written (request {request_id}): {error}"
        );
        return util::json(
            500,
            &json!({
                "error": { "code": "PLAYER_LOG_AUDIT_FAILED", "message":
                    "The session was removed, but its audit record could not be written. Check the audit log." },
                "requestId": request_id,
            }),
        );
    }
    util::json(
        200,
        &json!({
            "deleted": true, "requestId": request_id, "userId": user_id, "launchSessionId": session_id,
            "files": removed["files"].as_array().map_or(0, Vec::len), "totalBytes": removed["totalBytes"],
        }),
    )
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/admin/v1/player-logs", |call| {
        Box::pin(route(call, true, list))
    });
    handlers.on(
        b,
        Method::Get,
        "/admin/v1/player-logs/:userId/:launchSessionId/:fileName",
        |call| Box::pin(route(call, true, read)),
    );
    handlers.on(
        b,
        Method::Delete,
        "/admin/v1/player-logs/:userId/:launchSessionId",
        |call| Box::pin(route(call, true, delete)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "abcdef12-3456-7890";

    fn upload(root: &Path, user: &str, session: &str, at: &str, files: &[(&str, &str)]) {
        let dir = root.join(user).join(session);
        fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            fs::write(dir.join(name), text).unwrap();
        }
        fs::write(
            dir.join(".uploaded-at.json"),
            format!("{{\"lastUploadedAt\":\"{at}\"}}\n"),
        )
        .unwrap();
    }

    // services/playerLogs.test.ts
    #[test]
    fn sessions_are_listed_newest_first_and_paged_without_splitting_a_second() {
        let dir = tempfile::tempdir().unwrap();
        upload(
            dir.path(),
            "u1",
            "aaaaaaaa-1",
            "2026-10-03T00:00:00.000Z",
            &[("launcher.log", "1234"), ("metadata.json", "{}"), ("evil.exe", "x")],
        );
        upload(
            dir.path(),
            "u2",
            "bbbbbbbb-2",
            "2026-10-03T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        upload(
            dir.path(),
            "u1",
            "cccccccc-3",
            "2026-10-01T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        upload(
            dir.path(),
            "bad name",
            "dddddddd-4",
            "2026-10-05T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        let page = list_sessions(dir.path(), None, 50, None);
        let ids: Vec<&str> = page["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["launchSessionId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["aaaaaaaa-1", "bbbbbbbb-2", "cccccccc-3"]);
        assert_eq!(page["nextBefore"], Value::Null);
        assert_eq!(
            page["sessions"][0],
            json!({"userId": "u1", "launchSessionId": "aaaaaaaa-1", "uploadedAt": "2026-10-03T00:00:00.000Z",
                   "files": [{"name": "launcher.log", "size": 4}, {"name": "metadata.json", "size": 2}], "totalBytes": 6})
        );
        // The edge of a page takes every session of the same second, and the cursor starts the next page
        let first = list_sessions(dir.path(), None, 1, None);
        assert_eq!(first["sessions"].as_array().unwrap().len(), 2);
        assert_eq!(first["nextBefore"], "2026-10-03T00:00:00.000Z");
        let next = list_sessions(dir.path(), None, 1, date_parse("2026-10-03T00:00:00.000Z"));
        assert_eq!(next["sessions"][0]["launchSessionId"], "cccccccc-3");
        assert_eq!(
            list_sessions(dir.path(), Some("u2"), 50, None)["sessions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(list_sessions(dir.path(), Some("../x"), 50, None)["sessions"], json!([]));
        assert_eq!(
            list_sessions(&dir.path().join("none"), None, 50, None)["sessions"],
            json!([])
        );
    }

    #[test]
    fn a_file_is_read_only_by_a_launcher_name_inside_a_real_session() {
        let dir = tempfile::tempdir().unwrap();
        upload(
            dir.path(),
            "u1",
            SESSION,
            "2026-10-03T00:00:00.000Z",
            &[("launcher.log", "hello"), ("metadata.json", "{}"), ("other.txt", "x")],
        );
        let ok = read_file(dir.path(), "u1", SESSION, "launcher.log").unwrap();
        assert_eq!(
            (ok.0.as_str(), ok.1, ok.2.as_slice()),
            ("launcher.log", "text/plain; charset=utf-8", &b"hello"[..])
        );
        assert_eq!(
            read_file(dir.path(), "u1", SESSION, "metadata.json").unwrap().1,
            "application/json; charset=utf-8"
        );
        let code = |r: Result<(String, &'static str, Vec<u8>), Refusal>| r.unwrap_err().code;
        assert_eq!(
            code(read_file(dir.path(), "u1", SESSION, "other.txt")),
            "PLAYER_LOG_NAME_REFUSED"
        );
        assert_eq!(code(read_file(dir.path(), "u1", SESSION, "..")), "PLAYER_LOG_PATH");
        assert_eq!(
            code(read_file(dir.path(), "u1", "short", "launcher.log")),
            "PLAYER_LOG_PATH"
        );
        assert_eq!(
            code(read_file(dir.path(), "../u1", SESSION, "launcher.log")),
            "PLAYER_LOG_PATH"
        );
        assert_eq!(
            code(read_file(dir.path(), "u2", SESSION, "launcher.log")),
            "PLAYER_LOG_NOT_FOUND"
        );
        assert_eq!(
            code(read_file(
                dir.path(),
                "u1",
                SESSION,
                "runtime-rustic_isles_dll_port1.log"
            )),
            "PLAYER_LOG_NOT_FOUND"
        );
        assert_eq!(
            code(read_file(&dir.path().join("none"), "u1", SESSION, "launcher.log")),
            "PLAYER_LOG_NOT_FOUND"
        );
        let big = vec![b'x'; usize::try_from(FILE_MAX_BYTES).unwrap() + 1];
        fs::write(dir.path().join("u1").join(SESSION).join("launcher.log"), big).unwrap();
        let refusal = read_file(dir.path(), "u1", SESSION, "launcher.log").unwrap_err();
        assert_eq!((refusal.status, refusal.code), (413, "PLAYER_LOG_TOO_LARGE"));
    }

    #[test]
    fn a_session_is_deleted_whole_and_its_summary_kept() {
        let dir = tempfile::tempdir().unwrap();
        upload(
            dir.path(),
            "u1",
            SESSION,
            "2026-10-03T00:00:00.000Z",
            &[("launcher.log", "hello"), ("metadata.json", "{}")],
        );
        upload(
            dir.path(),
            "u1",
            "ffffffff-1",
            "2026-10-03T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        let gone = delete_session(dir.path(), "u1", SESSION).unwrap();
        assert_eq!(gone["totalBytes"], 7);
        assert_eq!(gone["files"].as_array().unwrap().len(), 2);
        assert!(!dir.path().join("u1").join(SESSION).exists() && dir.path().join("u1/ffffffff-1").exists());
        assert_eq!(
            delete_session(dir.path(), "u1", SESSION).unwrap_err().code,
            "PLAYER_LOG_NOT_FOUND"
        );
        assert_eq!(
            delete_session(dir.path(), "u1", "..").unwrap_err().code,
            "PLAYER_LOG_PATH"
        );
    }

    // services/launcherLogRetention.test.ts
    #[test]
    fn expired_sessions_and_closed_accounts_leave_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        upload(
            dir.path(),
            "u1",
            "aaaaaaaa-1",
            "2026-06-01T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        upload(
            dir.path(),
            "u1",
            "bbbbbbbb-2",
            "2026-10-20T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        upload(
            dir.path(),
            "u2",
            "cccccccc-3",
            "2026-06-02T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        fs::create_dir_all(dir.path().join("not an account/dddddddd-4")).unwrap();
        let now = date_parse("2026-10-26T00:00:00Z").unwrap();
        let pass = purge_expired(dir.path(), now, RETENTION_DAYS);
        assert_eq!((pass.checked, pass.expired, pass.removed, pass.failed), (3, 2, 2, 0));
        assert!(dir.path().join("u1/bbbbbbbb-2").exists() && !dir.path().join("u1/aaaaaaaa-1").exists());
        assert_eq!(purge_expired(dir.path(), now, RETENTION_DAYS).expired, 0);
        assert_eq!(remove_account_logs(dir.path(), "u1"), Ok(1));
        assert!(!dir.path().join("u1").exists());
        assert_eq!(remove_account_logs(dir.path(), "u1"), Ok(0));
        assert!(remove_account_logs(dir.path(), "../u1").is_err());
        assert!(remove_account_logs(dir.path(), "").is_err());
        assert!(dir.path().join("not an account").exists());
    }
}
