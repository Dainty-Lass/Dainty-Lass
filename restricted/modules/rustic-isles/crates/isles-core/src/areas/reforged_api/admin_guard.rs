//! The dashboard's read-only Launcher Guard views (`controllers/adminGuard.ts`): sessions, one session with its
//! history, and the module findings. A session keeps its challenge (the heartbeat chain's secret) and public key in
//! the store; the answers leave both out.

use axum::response::Response;
use serde_json::{Value, json};

use super::admin::{Ctx, limit_of, page_of, route, user_id_of};
use super::util;
use crate::http::Handlers;
use crate::repo::reforged::{GuardStatus, LauncherGuardSessionRecord};
use crate::routes::{Method, Service};

const STATUSES: [&str; 5] = ["CHALLENGED", "HEALTHY", "DEGRADED", "REVOKED", "EXPIRED"];

fn bad_query(message: &str) -> Response {
    util::json(
        400,
        &json!({ "error": { "code": "GUARD_QUERY_INVALID", "message": message } }),
    )
}

/// `PublicGuardSession`; `user_id` is the `accountId` the dashboard shows.
pub fn public_session(record: &LauncherGuardSessionRecord, user_id: &str) -> Value {
    json!({
        "guardSessionId": record.guard_session_id,
        "accountId": user_id,
        "role": record.role.as_str(),
        "status": record.status.as_str(),
        "riskScore": record.risk_score,
        "reasons": record.reasons,
        "channel": record.channel,
        "launcherVersion": record.launcher_version,
        "manifestId": record.manifest_id,
        "manifestSequence": record.manifest_sequence,
        "lastSequence": record.last_sequence,
        "createdAt": record.created_at,
        "lastHeartbeatAt": record.last_heartbeat_at,
        "expiresAt": record.expires_at,
        "lastSignals": record.last_signals,
    })
}

fn skip(page: i64, limit: i64) -> u32 {
    u32::try_from(((page - 1) * limit).max(0)).unwrap_or(u32::MAX)
}

async fn list_sessions(ctx: Ctx) -> Response {
    let status_text = ctx
        .req
        .query_string("status")
        .map_or("", service_kit::js::trim)
        .to_owned();
    if !status_text.is_empty() && !STATUSES.contains(&status_text.as_str()) {
        return bad_query(&format!("status must be one of {}", STATUSES.join(", ")));
    }
    let min_text = ctx
        .req
        .query_string("minRisk")
        .map_or("", service_kit::js::trim)
        .to_owned();
    let min_risk = if min_text.is_empty() {
        None
    } else {
        let n = service_kit::js::number(Some(&Value::String(min_text)));
        if n.fract() != 0.0 || !(0.0..=100.0).contains(&n) {
            return bad_query("minRisk must be a whole number from 0 to 100");
        }
        Some(n)
    };
    let (page, limit) = (page_of(&ctx.req), limit_of(&ctx.req, 25, 100));
    let status = GuardStatus::parse(&status_text);
    let (offset, take) = (skip(page, limit), u32::try_from(limit).unwrap_or(25));
    let result = ctx
        .store
        .read(move |conn, repos| {
            let (records, total) = repos
                .launcher_guard
                .list_sessions(conn, status, min_risk, offset, take)?;
            let mut sessions = Vec::new();
            for record in &records {
                let user_id = user_id_of(conn, repos, &record.account)?;
                sessions.push(public_session(record, &user_id));
            }
            Ok((sessions, total))
        })
        .await;
    match result {
        Ok((sessions, total)) => util::json(
            200,
            &json!({ "sessions": sessions, "total": total, "page": page, "limit": limit }),
        ),
        Err(error) => {
            tracing::error!("[ADMIN] guard sessions: {error}");
            util::empty(500)
        }
    }
}

async fn get_session(ctx: Ctx) -> Response {
    let id = ctx.params.get("guardSessionId").unwrap_or_default().to_owned();
    let lookup = id.clone();
    let result = ctx
        .store
        .read(move |conn, repos| {
            if lookup.is_empty() {
                return Ok(None);
            }
            let Some(record) = repos.launcher_guard.find(conn, &lookup)? else {
                return Ok(None);
            };
            let user_id = user_id_of(conn, repos, &record.account)?;
            Ok(Some((public_session(&record, &user_id), record.history)))
        })
        .await;
    match result {
        Ok(Some((session, history))) => util::json(200, &json!({ "session": session, "history": history })),
        Ok(None) => util::json(
            404,
            &json!({ "error": { "code": "GUARD_SESSION_NOT_FOUND", "message": "No Guard session has that id." } }),
        ),
        Err(error) => {
            tracing::error!("[ADMIN] guard session: {error}");
            util::empty(500)
        }
    }
}

async fn list_findings(ctx: Ctx) -> Response {
    let signer: String = ctx
        .req
        .query_string("signer")
        .map_or("", service_kit::js::trim)
        .chars()
        .take(100)
        .collect();
    let (page, limit) = (page_of(&ctx.req), limit_of(&ctx.req, 25, 100));
    let (offset, take) = (skip(page, limit), u32::try_from(limit).unwrap_or(25));
    let result = ctx
        .store
        .read(move |conn, repos| {
            repos
                .launcher_guard
                .list_findings(conn, (!signer.is_empty()).then_some(signer.as_str()), offset, take)
        })
        .await;
    match result {
        Ok((records, total)) => {
            let findings: Vec<Value> = records
                .iter()
                .map(|f| {
                    json!({
                        "fileName": f.file_name, "sha256": f.sha256, "signer": f.signer, "size": f.size,
                        "firstSeenAt": f.first_seen_at, "lastSeenAt": f.last_seen_at,
                        "sessionCount": f.session_count, "accountCount": f.account_count,
                    })
                })
                .collect();
            util::json(
                200,
                &json!({ "findings": findings, "total": total, "page": page, "limit": limit }),
            )
        }
        Err(error) => {
            tracing::error!("[ADMIN] guard findings: {error}");
            util::empty(500)
        }
    }
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/admin/v1/guard/sessions", |call| {
        Box::pin(route(call, true, list_sessions))
    });
    handlers.on(b, Method::Get, "/admin/v1/guard/sessions/:guardSessionId", |call| {
        Box::pin(route(call, true, get_session))
    });
    handlers.on(b, Method::Get, "/admin/v1/guard/findings", |call| {
        Box::pin(route(call, true, list_findings))
    });
}
