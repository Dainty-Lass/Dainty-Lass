// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Launcher logs (public spec §7): testers' uploads, the sync script's reads, the operator's view, the 90-day purge
//! and an erased account's removal. No path here follows a symbolic link (coordinator decisions, items 6 and 11).

use std::path::{Path, PathBuf};

use axum::response::{IntoResponse, Response};
use http::{StatusCode, header};
use module_api::{Actor, ActorContext};
use rustic_isles::{Call, Settings};
use rustic_isles_store::repo::reforged::AdminAuditRecord;
use serde_json::Value;

use crate::admin::{AdminContext, admin_error, fresh_id};
use crate::launcher_auth::launcher_sign_in;
use crate::util::{self, DAY_MS, HOUR_MS, iso, json, now_ms, object, status_text};

/// Days an uploaded session is kept.
pub const RETENTION_DAYS: i64 = 90;
/// The file in a session folder that records when it was uploaded.
pub const MARKER: &str = ".uploaded-at.json";
const MAX_FILE: usize = 10 * 1024 * 1024;
const MAX_SESSIONS: usize = 200;
const MAX_ACCOUNT_BYTES: u64 = 200 * 1024 * 1024;

// ------------------------------------------------------------------ names (§7.1)

/// 8 to 64 bytes of hex digits and `-`.
pub fn is_session_id(text: &str) -> bool {
    (8..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// `metadata.json`, `launcher.log`, or `runtime-rustic_isles_dll_port<digits>.log` (any case).
pub fn is_launcher_file_name(text: &str) -> bool {
    if text == "metadata.json" || text == "launcher.log" {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    lower
        .strip_prefix("runtime-rustic_isles_dll_port")
        .and_then(|rest| rest.strip_suffix(".log"))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// 1 to 64 bytes of ASCII letters, digits and `-`.
pub fn is_account_folder(text: &str) -> bool {
    (1..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `LAUNCHER_LOG_STORAGE_DIR` as set, else `../MysticLauncherLogs`, made absolute.
pub fn log_storage_root(settings: &Settings) -> PathBuf {
    crate::updates::absolute(
        settings
            .raw("LAUNCHER_LOG_STORAGE_DIR")
            .unwrap_or("../MysticLauncherLogs"),
    )
}

/// A real folder (a link is never followed).
fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

fn is_real_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

fn sorted_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

fn marker_text(session: &Path) -> Option<String> {
    let text = std::fs::read_to_string(session.join(MARKER)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("lastUploadedAt")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// The marker's time when it parses, else the folder's modification time.
fn upload_time(session: &Path) -> Option<i64> {
    marker_text(session)
        .and_then(|t| service_kit::js::date_parse(&t))
        .or_else(|| modified_ms(session))
}

// ------------------------------------------------------------------ the listing (§7.4)

/// One uploaded session's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    /// The account folder's userId.
    pub user_id: String,
    /// The launch session id.
    pub session_id: String,
    /// When it was last uploaded, Unix ms.
    pub uploaded_ms: i64,
    /// The folder.
    pub path: PathBuf,
}

/// Every session of the root (or of one account): account folders and session ids, real folders only.
pub fn list_uploaded(root: &Path, user: Option<&str>) -> Vec<Upload> {
    let mut out = Vec::new();
    if !is_real_dir(root) {
        return out;
    }
    let accounts = match user {
        Some(user) => vec![user.to_owned()],
        None => sorted_entries(root),
    };
    for account in accounts.into_iter().filter(|a| is_account_folder(a)) {
        let folder = root.join(&account);
        if !is_real_dir(&folder) {
            continue;
        }
        for session in sorted_entries(&folder).into_iter().filter(|s| is_session_id(s)) {
            let path = folder.join(&session);
            if !is_real_dir(&path) {
                continue;
            }
            let Some(uploaded_ms) = upload_time(&path) else {
                continue;
            };
            out.push(Upload {
                user_id: account.clone(),
                session_id: session,
                uploaded_ms,
                path,
            });
        }
    }
    out
}

fn summary_files(session: &Path) -> Vec<(String, u64)> {
    let mut names: Vec<String> = sorted_entries(session)
        .into_iter()
        .filter(|n| is_launcher_file_name(n) && is_real_file(&session.join(n)))
        .collect();
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    names
        .into_iter()
        .map(|n| {
            let size = std::fs::symlink_metadata(session.join(&n))
                .map(|m| m.len())
                .unwrap_or(0);
            (n, size)
        })
        .collect()
}

/// A session as the listings show it: ids, upload time and its files.
pub fn summary(upload: &Upload) -> Value {
    let files = summary_files(&upload.path);
    let total: u64 = files.iter().map(|(_, s)| s).sum();
    object(vec![
        ("userId", Value::from(upload.user_id.clone())),
        ("launchSessionId", Value::from(upload.session_id.clone())),
        ("uploadedAt", Value::from(iso(upload.uploaded_ms))),
        (
            "files",
            Value::Array(
                files
                    .into_iter()
                    .map(|(name, size)| object(vec![("name", Value::from(name)), ("size", Value::from(size))]))
                    .collect(),
            ),
        ),
        ("totalBytes", Value::from(total)),
    ])
}

// ------------------------------------------------------------------ retention (§7.5)

/// What a purge did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgeReport {
    /// Sessions looked at.
    pub checked: usize,
    /// Sessions past retention.
    pub expired: usize,
    /// Sessions removed.
    pub removed: usize,
    /// Sessions that could not be removed.
    pub failed: usize,
}

/// Removes every listed session uploaded before `now − days`.
pub fn purge_expired(root: &Path, now_ms: i64, days: i64) -> PurgeReport {
    let cutoff = now_ms - days * DAY_MS;
    let mut report = PurgeReport::default();
    for upload in list_uploaded(root, None) {
        report.checked += 1;
        if upload.uploaded_ms < cutoff {
            report.expired += 1;
            match std::fs::remove_dir_all(&upload.path) {
                Ok(()) => report.removed += 1,
                Err(_) => report.failed += 1,
            }
        }
    }
    report
}

/// An erased account's folder, whole; the count of its listed sessions.
pub fn remove_account_logs(root: &Path, user_id: &str) -> Result<usize, String> {
    if !is_account_folder(user_id) {
        return Err("Refusing to remove uploaded logs for an account id that is not a plain folder name".to_owned());
    }
    let folder = root.join(user_id);
    if std::fs::symlink_metadata(&folder).is_err() {
        return Ok(0);
    }
    let count = list_uploaded(root, Some(user_id)).len();
    std::fs::remove_dir_all(&folder).map_err(|_| "The folder could not be removed".to_owned())?;
    Ok(count)
}

// ------------------------------------------------------------------ uploads (§7.2)

fn quota_error(message: &str) -> Response {
    json(
        StatusCode::PAYLOAD_TOO_LARGE,
        &object(vec![(
            "error",
            object(vec![
                ("code", Value::from("LOG_UPLOAD_QUOTA")),
                ("message", Value::from(message)),
            ]),
        )]),
    )
}

fn usage(account: &Path) -> (usize, u64) {
    let mut sessions = 0;
    let mut bytes = 0;
    for name in sorted_entries(account) {
        let session = account.join(name);
        if !is_real_dir(&session) {
            continue;
        }
        sessions += 1;
        for file in sorted_entries(&session) {
            if let Ok(meta) = std::fs::symlink_metadata(session.join(file))
                && meta.is_file()
            {
                bytes += meta.len();
            }
        }
    }
    (sessions, bytes)
}

enum Saved {
    Done(String),
    Sessions,
    Storage,
}

/// `PUT /launcher/v1/logs/sessions/:launchSessionId/:fileName`: a tester's log upload.
pub async fn upload(mut call: Call) -> Response {
    let caller = match launcher_sign_in(&call.shared, call.request.headers()).await {
        Ok(caller) => caller,
        Err(refused) => return refused,
    };
    let client = util::client_ip(&call.request);
    let body = match util::read_octets(&mut call.request, MAX_FILE).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    if !caller.has_role("tester") {
        return util::launcher_error("AUTH_TESTER_REQUIRED", "Tester access is required to upload logs.");
    }
    let limiter = call.shared.auth().limiter();
    let now = now_ms();
    if limiter.is_limited_at(&format!("log-upload:{}", caller.user_id), 120, HOUR_MS, now)
        || limiter.is_limited_at(&format!("log-upload-ip:{client}"), 300, HOUR_MS, now)
    {
        return json(
            StatusCode::TOO_MANY_REQUESTS,
            &object(vec![(
                "error",
                object(vec![
                    ("code", Value::from("LOG_UPLOAD_RATE_LIMITED")),
                    ("message", Value::from("Too many uploads. Try again later.")),
                ]),
            )]),
        );
    }
    let session = call.params.get("launchSessionId").unwrap_or("").to_owned();
    let file = call.params.get("fileName").unwrap_or("").to_owned();
    // The userId becomes a folder: one that is not a plain folder name is refused (decisions, item 6)
    if !is_session_id(&session) || !is_launcher_file_name(&file) || !is_account_folder(&caller.user_id) {
        return util::launcher_error("AUTH_VALIDATION_FAILED", "Invalid session id or file name.");
    }
    let body = match body {
        Some(b) if !b.is_empty() => b,
        _ => return util::launcher_error("AUTH_VALIDATION_FAILED", "The uploaded file was empty."),
    };
    if body.len() > MAX_FILE {
        return util::launcher_error("AUTH_VALIDATION_FAILED", "The uploaded file is too large.");
    }
    let account = log_storage_root(call.shared.settings()).join(&caller.user_id);
    let saved = tokio::task::spawn_blocking(move || -> std::io::Result<Saved> {
        let (sessions, bytes) = usage(&account);
        let folder = account.join(&session);
        if !is_real_dir(&folder) && sessions >= MAX_SESSIONS {
            return Ok(Saved::Sessions);
        }
        if bytes + body.len() as u64 > MAX_ACCOUNT_BYTES {
            return Ok(Saved::Storage);
        }
        std::fs::create_dir_all(&folder)?;
        std::fs::write(folder.join(&file), &body)?;
        let at = iso(now);
        let marker = format!(
            "{}\n",
            service_kit::jsjson::stringify(&object(vec![("lastUploadedAt", Value::from(at.clone()))]))
        );
        std::fs::write(folder.join(MARKER), marker)?;
        Ok(Saved::Done(at))
    })
    .await;
    match saved {
        Ok(Ok(Saved::Done(at))) => json(StatusCode::CREATED, &object(vec![("uploadedAt", Value::from(at))])),
        Ok(Ok(Saved::Sessions)) => quota_error("Session limit reached for this account."),
        Ok(Ok(Saved::Storage)) => quota_error("Storage quota reached for this account."),
        Ok(Err(e)) => {
            tracing::error!("[logs] upload not saved: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(e) => {
            tracing::error!("[logs] upload not saved: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ------------------------------------------------------------------ the sync script (§7.3)

async fn sync_gate(call: &mut Call) -> Result<(), Response> {
    if util::read_json(&mut call.request).await.is_err() {
        return Err(util::empty(StatusCode::BAD_REQUEST));
    }
    let configured = util::trim(call.shared.settings().raw("LAUNCHER_LOG_SYNC_TOKEN").unwrap_or("")).to_owned();
    let sent = util::header_lossy(call.request.headers(), "x-sync-token").unwrap_or_default();
    let sent = util::trim(&sent);
    if configured.is_empty() || sent.is_empty() || !util::ct_eq(sent.as_bytes(), configured.as_bytes()) {
        return Err(json(
            StatusCode::UNAUTHORIZED,
            &object(vec![("error", Value::from("Invalid sync credentials."))]),
        ));
    }
    Ok(())
}

/// `GET /admin/v1/launcher-logs`: the sync script's listing.
pub async fn sync_list(mut call: Call) -> Response {
    if let Err(refused) = sync_gate(&mut call).await {
        return refused;
    }
    let pairs = util::query_pairs(call.request.uri().query());
    let since = util::single(&pairs, "since").and_then(service_kit::js::date_parse);
    let root = log_storage_root(call.shared.settings());
    let sessions = tokio::task::spawn_blocking(move || {
        let mut rows: Vec<(String, Value)> = Vec::new();
        if !is_real_dir(&root) {
            return rows;
        }
        for account in sorted_entries(&root) {
            let folder = root.join(&account);
            if !util::is_segment(&account) || !is_real_dir(&folder) {
                continue;
            }
            for session in sorted_entries(&folder) {
                let path = folder.join(&session);
                if !util::is_segment(&session) || !is_real_dir(&path) {
                    continue;
                }
                let uploaded = marker_text(&path)
                    .or_else(|| modified_ms(&path).map(iso))
                    .unwrap_or_default();
                if let (Some(at), Some(since)) = (service_kit::js::date_parse(&uploaded), since)
                    && at <= since
                {
                    continue;
                }
                let files: Vec<Value> = sorted_entries(&path)
                    .into_iter()
                    .filter(|n| n != MARKER && is_real_file(&path.join(n)))
                    .map(Value::from)
                    .collect();
                let row = object(vec![
                    ("userId", Value::from(account.clone())),
                    ("launchSessionId", Value::from(session.clone())),
                    ("uploadedAt", Value::from(uploaded.clone())),
                    ("files", Value::Array(files)),
                ]);
                rows.push((uploaded, row));
            }
        }
        rows.sort_by(|a, b| service_kit::collate::locale_compare(&a.0, &b.0));
        rows
    })
    .await
    .unwrap_or_default();
    util::ok(&object(vec![(
        "sessions",
        Value::Array(sessions.into_iter().map(|(_, v)| v).collect()),
    )]))
}

/// `GET /admin/v1/launcher-logs/:userId/:launchSessionId/:fileName`: the sync script's file read.
pub async fn sync_file(mut call: Call) -> Response {
    if let Err(refused) = sync_gate(&mut call).await {
        return refused;
    }
    let names: Vec<String> = ["userId", "launchSessionId", "fileName"]
        .iter()
        .map(|n| call.params.get(n).unwrap_or("").to_owned())
        .collect();
    if !names.iter().all(|n| util::is_segment(n)) {
        return json(
            StatusCode::BAD_REQUEST,
            &object(vec![("error", Value::from("Invalid path."))]),
        );
    }
    let root = log_storage_root(call.shared.settings());
    let session = root.join(&names[0]).join(&names[1]);
    let path = session.join(&names[2]);
    if !is_real_dir(&root.join(&names[0])) || !is_real_dir(&session) || !is_real_file(&path) {
        return status_text(StatusCode::NOT_FOUND);
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                (header::CONTENT_LENGTH, bytes.len().to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => status_text(StatusCode::NOT_FOUND),
    }
}

// ------------------------------------------------------------------ the operator's view (§7.4)

fn query_error(message: &str) -> Response {
    admin_error(StatusCode::BAD_REQUEST, "PLAYER_LOGS_QUERY", message)
}

fn not_found(message: &str) -> Response {
    admin_error(StatusCode::NOT_FOUND, "PLAYER_LOG_NOT_FOUND", message)
}

fn is_before(text: &str) -> bool {
    let b = text.as_bytes();
    let shape = b.len() >= 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .iter()
            .all(|i| b[*i].is_ascii_digit());
    if !shape || !text.ends_with('Z') {
        return false;
    }
    let tail = &text[19..text.len() - 1];
    let fraction_ok = tail.is_empty()
        || tail
            .strip_prefix('.')
            .is_some_and(|d| (1..=3).contains(&d.len()) && d.bytes().all(|c| c.is_ascii_digit()));
    fraction_ok && service_kit::js::date_parse(text).is_some()
}

/// `GET /admin/v1/player-logs`: the dashboard's listing.
pub async fn list_player_logs(ctx: AdminContext) -> Response {
    let query = &ctx.request.query;
    let user = if util::repeated(query, "userId") {
        return query_error("userId must be one account id.");
    } else {
        util::single(query, "userId")
            .filter(|u| !u.is_empty())
            .map(str::to_owned)
    };
    if user.as_deref().is_some_and(|u| !util::is_segment(u)) {
        return query_error("userId must be one account id.");
    }
    let limit_message = "limit must be a whole number from 1 to 200.";
    if util::repeated(query, "limit") {
        return query_error(limit_message);
    }
    let limit = match util::single(query, "limit") {
        None => 50,
        Some(text) => {
            let ok = (1..=3).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
            match text.parse::<usize>() {
                Ok(n) if ok && (1..=200).contains(&n) => n,
                _ => return query_error(limit_message),
            }
        }
    };
    let before_message = "before must be a UTC time such as 2026-10-15T12:00:00.000Z.";
    if util::repeated(query, "before") {
        return query_error(before_message);
    }
    let before = match util::single(query, "before") {
        None => None,
        Some(text) if is_before(text) => service_kit::js::date_parse(text),
        Some(_) => return query_error(before_message),
    };
    let root = log_storage_root(ctx.shared.settings());
    let (sessions, next) = tokio::task::spawn_blocking(move || {
        let mut uploads: Vec<Upload> = list_uploaded(&root, user.as_deref())
            .into_iter()
            .filter(|u| before.is_none_or(|b| u.uploaded_ms < b))
            .collect();
        uploads.sort_by(|a, b| {
            b.uploaded_ms
                .cmp(&a.uploaded_ms)
                .then_with(|| a.user_id.as_bytes().cmp(b.user_id.as_bytes()))
                .then_with(|| a.session_id.as_bytes().cmp(b.session_id.as_bytes()))
        });
        let mut taken = uploads.len().min(limit);
        if taken > 0 {
            let last = uploads[taken - 1].uploaded_ms;
            while taken < uploads.len() && uploads[taken].uploaded_ms == last {
                taken += 1;
            }
        }
        let next = (taken < uploads.len() && taken > 0).then(|| iso(uploads[taken - 1].uploaded_ms));
        let page: Vec<Value> = uploads[..taken].iter().map(summary).collect();
        (page, next)
    })
    .await
    .unwrap_or_default();
    util::ok(&object(vec![
        ("sessions", Value::Array(sessions)),
        ("nextBefore", next.map_or(Value::Null, Value::from)),
    ]))
}

/// The session folder, checked as §7.4 says.
fn locate_session(root: &Path, user: &str, session: &str) -> Result<PathBuf, Response> {
    if !util::is_segment(user) || !util::is_segment(session) || !is_session_id(session) {
        return Err(admin_error(
            StatusCode::BAD_REQUEST,
            "PLAYER_LOG_PATH",
            "The account or session id is not a valid folder name.",
        ));
    }
    let missing = || not_found("No such uploaded session.");
    let account = root.join(user);
    let folder = account.join(session);
    if !is_real_dir(root) || !is_real_dir(&account) || !is_real_dir(&folder) {
        return Err(missing());
    }
    let (Ok(root_real), Ok(folder_real)) = (std::fs::canonicalize(root), std::fs::canonicalize(&folder)) else {
        return Err(missing());
    };
    if folder_real == root_real || !folder_real.starts_with(&root_real) {
        return Err(missing());
    }
    Ok(folder)
}

/// `GET /admin/v1/player-logs/:userId/:launchSessionId/:fileName`: the dashboard's file read.
pub async fn read_player_log(ctx: AdminContext) -> Response {
    let query = &ctx.request.query;
    let download = match (util::repeated(query, "download"), util::single(query, "download")) {
        (true, _) => return query_error("download must be 1 or 0."),
        (false, None | Some("0")) => false,
        (false, Some("1")) => true,
        _ => return query_error("download must be 1 or 0."),
    };
    let file = ctx.param("fileName").to_owned();
    if !util::is_segment(&file) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "PLAYER_LOG_PATH",
            "The file name is not a valid file name.",
        );
    }
    if !is_launcher_file_name(&file) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "PLAYER_LOG_NAME_REFUSED",
            "Only the launcher's log files can be read (metadata.json, launcher.log, runtime logs).",
        );
    }
    let root = log_storage_root(ctx.shared.settings());
    let folder = match locate_session(&root, ctx.param("userId"), ctx.param("launchSessionId")) {
        Ok(folder) => folder,
        Err(refused) => return refused,
    };
    let path = folder.join(&file);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return not_found("No such file in that session.");
    };
    if !meta.is_file() {
        return not_found("No such file in that session.");
    }
    let too_large = || {
        admin_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PLAYER_LOG_TOO_LARGE",
            "That file is larger than the admin view serves.",
        )
    };
    if meta.len() > MAX_FILE as u64 {
        return too_large();
    }
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return not_found("No such file in that session.");
    };
    if bytes.len() > MAX_FILE {
        return too_large();
    }
    let content_type = if file == "metadata.json" {
        "application/json; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    };
    let mut response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (header::CONTENT_LENGTH, bytes.len().to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        bytes,
    )
        .into_response();
    if download {
        util::set_header(
            &mut response,
            "content-disposition",
            &format!("attachment; filename=\"{file}\""),
        );
    }
    response
}

/// `DELETE /admin/v1/player-logs/:userId/:launchSessionId`: deletes a session's logs, with a reason.
pub async fn delete_player_logs(ctx: AdminContext) -> Response {
    let reason = ctx
        .request
        .field("reason")
        .and_then(Value::as_str)
        .map(|r| util::trim(r).to_owned());
    let Some(reason) = reason.filter(|r| (8..=200).contains(&util::utf16_len(r))) else {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "PLAYER_LOG_REASON",
            "Give a reason of 8 to 200 characters for the audit log.",
        );
    };
    let root = log_storage_root(ctx.shared.settings());
    let user = ctx.param("userId").to_owned();
    let session = ctx.param("launchSessionId").to_owned();
    let folder = match locate_session(&root, &user, &session) {
        Ok(folder) => folder,
        Err(refused) => return refused,
    };
    let upload = Upload {
        user_id: user.clone(),
        session_id: session.clone(),
        uploaded_ms: upload_time(&folder).unwrap_or(0),
        path: folder.clone(),
    };
    let summary = summary(&upload);
    if std::fs::remove_dir_all(&folder).is_err() {
        return not_found("No such uploaded session.");
    }
    let operator = ctx.caller_user_id();
    let request_id = fresh_id();
    let files = summary["files"].as_array().map_or(0, Vec::len);
    let total = summary["totalBytes"].clone();
    let audit = AdminAuditRecord {
        id: fresh_id(),
        actor_user_id: operator.clone(),
        target_user_id: Some(user.clone()),
        action: "player_logs.delete".to_owned(),
        old_state: Some(object(vec![
            ("launchSessionId", summary["launchSessionId"].clone()),
            ("uploadedAt", summary["uploadedAt"].clone()),
            ("files", summary["files"].clone()),
            ("totalBytes", total.clone()),
        ])),
        new_state: Some(object(vec![("deleted", Value::Bool(true))])),
        reason: Some(reason),
        ip: ctx.request.client_ip.clone(),
        request_id: request_id.clone(),
        created_at: iso(now_ms()),
    };
    let actor = ActorContext::new(Actor::Admin { key: operator });
    if let Err(e) = ctx
        .store
        .write(&actor, move |tx, repos| repos.admin.append_audit(tx, &audit))
        .await
    {
        tracing::error!(
            "[PlayerLogs] a session was removed but its audit record could not be written (request {request_id}): {e}"
        );
        let mut body = util::error_object(
            "PLAYER_LOG_AUDIT_FAILED",
            "The session was removed, but its audit record could not be written. Check the audit log.",
        );
        if let Value::Object(map) = &mut body {
            map.insert("requestId".into(), Value::from(request_id));
        }
        return json(StatusCode::INTERNAL_SERVER_ERROR, &body);
    }
    util::ok(&object(vec![
        ("deleted", Value::Bool(true)),
        ("requestId", Value::from(request_id)),
        ("userId", Value::from(user)),
        ("launchSessionId", Value::from(session)),
        ("files", Value::from(files)),
        ("totalBytes", total),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in [
            "metadata.json",
            "launcher.log",
            "runtime-rustic_isles_dll_port7777.log",
            "RUNTIME-Rustic_Isles_DLL_PORT1.LOG",
        ] {
            assert!(is_launcher_file_name(ok), "{ok}");
        }
        for bad in [
            "",
            "other.log",
            "runtime-rustic_isles_dll_port.log",
            "runtime-rustic_isles_dll_portx.log",
            "../launcher.log",
            "Metadata.json",
            "launcher.log.exe",
            "runtime-mysticparadox_dll_port7777.log",
        ] {
            assert!(!is_launcher_file_name(bad), "{bad}");
        }
        assert!(is_session_id("abcdef12-3456") && is_session_id(&"a".repeat(64)));
        assert!(!is_session_id("short") && !is_session_id("xyzxyzxyz"));
        assert!(is_before("2026-10-15T12:00:00.000Z") && is_before("2026-10-15T12:00:00Z"));
        assert!(!is_before("2026-10-15T12:00:00") && !is_before("2026-10-15T12:00:00.0000Z"));
    }

    fn session(root: &Path, user: &str, id: &str, at: &str, files: &[(&str, &str)]) {
        let dir = root.join(user).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MARKER), format!("{{\"lastUploadedAt\":\"{at}\"}}\n")).unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
    }

    #[test]
    fn retention() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        session(
            root,
            "u1",
            "aaaaaaaa-1",
            "2026-06-01T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        session(
            root,
            "u1",
            "aaaaaaaa-2",
            "2026-10-20T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        session(
            root,
            "u2",
            "bbbbbbbb-1",
            "2026-06-02T00:00:00.000Z",
            &[("launcher.log", "x")],
        );
        std::fs::create_dir_all(root.join("not an account").join("cccccccc-1")).unwrap();
        let now = service_kit::js::date_parse("2026-10-26T00:00:00Z").unwrap();
        assert_eq!(
            purge_expired(root, now, RETENTION_DAYS),
            PurgeReport {
                checked: 3,
                expired: 2,
                removed: 2,
                failed: 0
            }
        );
        assert_eq!(purge_expired(root, now, RETENTION_DAYS).expired, 0);
        assert_eq!(remove_account_logs(root, "u1"), Ok(1));
        assert_eq!(remove_account_logs(root, "u1"), Ok(0));
        assert!(remove_account_logs(root, "../u1").is_err() && remove_account_logs(root, "").is_err());
        assert!(root.join("not an account").exists());
    }
}
