//! The legacy routes (DR `src/app.ts` at 24b7624), in place of the runtime's on a 1.4.4 server:
//!
//! - /content/v1/manifest: the game's file list, no key needed (DR's launcher has the same list)
//! - /content/v1/files/<path>: one game file; needs `x-reforged-user-api-key` (DR's
//!   `x-undaunted-user-api-key`, translated as the ported metagame has it); one HTTP range
//!
//! DR's CORS headers (`Reply::public`) are not carried: Reforged's launcher reads these from Rust.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use serde_json::json;
use tokio::io::AsyncSeekExt;

use super::{App, Info, KEY_HEADER, Reply, file_body};
use crate::auth::LookupResult;
use crate::input::{header_text, if_none_match_hits};
use crate::legacy::LegacyGame;
use crate::legacy::gamedir::FileState;
use crate::legacy::manifest::on_disk_path;
use crate::legacy::paths::parse_file_request_path;
use crate::legacy::range::{Range, if_range_allows_partial, parse_range};
use crate::limits::{Refusal, Slot};
use crate::log;

impl Reply {
    fn remove(&mut self, names: &[&str]) {
        for name in names {
            self.headers.remove(*name);
        }
    }
}

pub(super) fn game_manifest(game: &LegacyGame, headers: &HeaderMap, head: bool, reply: &mut Reply) {
    if if_none_match_hits(header_text(headers, "if-none-match").as_deref(), &game.manifest_etag) {
        reply.status = StatusCode::NOT_MODIFIED;
        reply.set("etag", &game.manifest_etag);
        return;
    }
    reply.text(head, 200, game.manifest_text.clone(), &[("etag", &game.manifest_etag)]);
}

fn set_state(game: &LegacyGame, path: &str, state: FileState) {
    game.states
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(path.to_owned(), state);
}

/// One game file; the download slot it holds, if it got one.
pub(super) async fn game_file(
    app: &App,
    game: &LegacyGame,
    headers: &HeaderMap,
    head: bool,
    target: &str,
    reply: &mut Reply,
    info: &mut Info,
) -> Option<Slot> {
    let deps = &app.deps;
    let path = match parse_file_request_path(target) {
        Ok(path) => path,
        Err(reason) => {
            info.reason = Some(reason.into());
            reply.error(head, 400, "bad_path", &[]);
            return None;
        }
    };
    let Some(entry) = game.index.get(&path) else {
        info.reason = Some("not_in_manifest".into());
        reply.error(head, 404, "not_found", &[]);
        return None;
    };
    info.path = Some(entry.path.clone());
    let Some(key) = header_text(headers, KEY_HEADER).filter(|key| !key.is_empty()) else {
        info.reason = Some("no_key".into());
        reply.error(head, 401, "unauthorized", &[]);
        return None;
    };
    let account = match deps.auth.check(&key).await {
        LookupResult::Unavailable(reason) => {
            info.reason = Some(reason);
            reply.error(head, 503, "auth_unavailable", &[("retry-after", "5")]);
            return None;
        }
        LookupResult::Invalid => {
            info.reason = Some("bad_key".into());
            reply.error(head, 401, "unauthorized", &[]);
            return None;
        }
        LookupResult::Ok(account) => account,
    };
    info.account = Some(account.user_id.clone());
    info.user = Some(account.username.clone());
    let state = game
        .states
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&entry.path)
        .cloned();
    let Some(FileState::Ok {
        modified: seen_modified,
        ..
    }) = state
    else {
        info.reason = Some(format!("file_{}", state.as_ref().map_or("unknown", FileState::status)));
        reply.error(head, 503, "file_unavailable", &[]);
        return None;
    };
    let etag = format!("\"{}\"", entry.sha256);
    reply.set("etag", &etag);
    reply.set("accept-ranges", "bytes");
    reply.set("cache-control", "private, no-transform");
    if if_none_match_hits(header_text(headers, "if-none-match").as_deref(), &etag) {
        reply.status = StatusCode::NOT_MODIFIED;
        return None;
    }
    // A stale If-Range means the client's partial copy is of something else: send it all
    let if_range = header_text(headers, "if-range");
    let range_header = if if_range_allows_partial(if_range.as_deref(), &etag) {
        header_text(headers, "range")
    } else {
        None
    };
    let range = parse_range(range_header.as_deref(), entry.size);
    if let Some(range_header) = &range_header {
        info.range = Some(crate::input::slice_utf16(range_header, 64));
    }
    let (start, end) = match range {
        Range::Unsatisfiable => {
            info.reason = Some("bad_range".into());
            let content_range = format!("bytes */{}", entry.size);
            reply.error(head, 416, "range_not_satisfiable", &[("content-range", &content_range)]);
            return None;
        }
        Range::Partial { start, end } => (start, end as i64),
        Range::Full => (0, entry.size as i64 - 1),
    };
    let length = (end - start as i64 + 1).max(0) as u64;
    reply.status = if matches!(range, Range::Partial { .. }) {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    reply.set("content-type", "application/octet-stream");
    reply.set("content-length", length);
    if matches!(range, Range::Partial { .. }) {
        reply.set("content-range", format!("bytes {start}-{end}/{}", entry.size));
    }
    if head {
        return None;
    }
    let slot = match deps.limiter.try_acquire(&account.user_id) {
        Ok(slot) => slot,
        Err(refusal) => {
            info.reason = Some(refusal.as_str().into());
            reply.remove(&["content-range", "etag", "accept-ranges"]);
            match refusal {
                Refusal::AccountLimit => reply.error(head, 429, "too_many_streams", &[("retry-after", "2")]),
                Refusal::TotalLimit => reply.error(head, 503, "server_busy", &[("retry-after", "10")]),
            }
            return None;
        }
    };
    let unavailable = |reply: &mut Reply| {
        reply.remove(&[
            "content-range",
            "etag",
            "accept-ranges",
            "content-type",
            "content-length",
        ]);
        reply.error(head, 503, "file_unavailable", &[]);
    };
    let mut file = match tokio::fs::File::open(on_disk_path(&game.game_dir, entry)).await {
        Ok(file) => file,
        Err(error) => {
            set_state(game, &entry.path, FileState::Missing);
            info.reason = Some("open_failed".into());
            log::warn(
                "game file could not be opened; no longer serving it",
                json!({"path": entry.path, "error": error.to_string()}),
            );
            unavailable(reply);
            return Some(slot);
        }
    };
    let Ok(meta) = file.metadata().await else {
        unavailable(reply);
        return Some(slot);
    };
    // Changed on disk since the startup check: no longer served until the host verifies and restarts
    if !meta.is_file() || meta.len() != entry.size || meta.modified().ok() != seen_modified {
        set_state(game, &entry.path, FileState::Changed);
        info.reason = Some("changed_on_disk".into());
        log::warn(
            "game file changed since startup; no longer serving it (run content verify, then restart)",
            json!({"path": entry.path}),
        );
        unavailable(reply);
        return Some(slot);
    }
    info.bytes = Some((Arc::new(AtomicU64::new(0)), length));
    if length == 0 {
        reply.body = Body::empty();
        return Some(slot);
    }
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        unavailable(reply);
        return Some(slot);
    }
    reply.body = file_body(file, length);
    Some(slot)
}
