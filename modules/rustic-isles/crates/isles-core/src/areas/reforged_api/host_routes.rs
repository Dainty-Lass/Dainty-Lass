//! The host kit's routes of `routes/reforgedApi.ts` (`HostOnly`): this machine only, never through the gateway, with
//! `x-reforged-admin-key`. The gateway refuses them from outside whatever this does (AGENTS.md).

use axum::response::Response;
use serde_json::{Value, json};

use super::launcher_api::{BODY_LIMIT, actor_host, forget_status, js_slice, refusal};
use super::presence;
use super::rules::{
    MAX_INVITE_USES, Refusal, invite_note, is_invite_code, is_local_admin_call, make_invite_code, normalize_username,
};
use super::util::{self, BodyKind, Req};
use crate::db::IslesStore;
use crate::http::{Call, Handlers, not_implemented};
use crate::repo::RepoResult;
use crate::repo::auth::{AccessChange, LauncherAccountRecord, OperationalStatus};
use crate::repo::reforged::InviteRecord;
use crate::routes::{Method, Service};

const MAX_INVITES_LISTED: u32 = 500;

struct Host {
    store: IslesStore,
    req: Req,
}

/// `HostOnly`: the 403 a request that isn't the host kit's own gets, or the request and the database. Express's
/// body parser has run by then, so a malformed body is its error page first.
async fn host_only(call: Call) -> Result<Host, Response> {
    let (shared, route, request_id) = (call.shared.clone(), call.route, call.request_id.clone());
    let req = Req::read(call.request, BodyKind::Json(BODY_LIMIT)).await?;
    if !is_local_admin_call(
        &req.headers,
        req.remote_address().as_deref(),
        shared.settings.raw("REFORGED_ADMIN_KEY"),
    ) {
        return Err(util::json(403, &json!({ "error": "forbidden" })));
    }
    match shared.store.clone() {
        Some(store) => Ok(Host { store, req }),
        None => Err(not_implemented(route, &request_id)),
    }
}

async fn create_invite(call: Call) -> Response {
    let Host { store, req } = match host_only(call).await {
        Ok(host) => host,
        Err(response) => return response,
    };
    let fields = req.fields();
    let uses = fields.get("uses").filter(|v| !v.is_null()).cloned().unwrap_or(json!(1));
    let infinite = fields.get("infinite") == Some(&Value::Bool(true));
    let uses_ok = service_kit::js::safe_integer(Some(&uses)).filter(|n| (1..=MAX_INVITE_USES).contains(n));
    let name_ok = fields.get("name").is_none_or(Value::is_string);
    let (true, Some(uses)) = (name_ok, uses_ok) else {
        return util::json(
            400,
            &json!({
                "error": "bad_request",
                "message": format!("uses is a whole number from 1 to {MAX_INVITE_USES}; name is text"),
            }),
        );
    };
    let note = invite_note(fields.get("name"));
    // A code is 60 random bits: a clash is a curiosity, but it is retried rather than reported.
    for _ in 0..5 {
        let record = InviteRecord {
            code: make_invite_code(),
            uses_remaining: uses,
            infinite_uses: infinite,
            note: note.clone(),
            created_at: util::now_iso(),
        };
        let code = record.code.clone();
        match store
            .write(&actor_host(), move |tx, repos| {
                repos.invites.insert_if_absent(tx, &record)
            })
            .await
        {
            Ok(true) => {
                tracing::info!(
                    "[launcher-api] invite made: {}{}",
                    if infinite {
                        "unlimited".to_owned()
                    } else {
                        format!("{uses} use(s)")
                    },
                    note.as_ref().map_or_else(String::new, |n| format!(" for {n}"))
                );
                return util::json(200, &json!({ "code": code }));
            }
            Ok(false) => {}
            Err(error) => {
                tracing::error!("[launcher-api] CreateInvite: {error}");
                return util::empty(500);
            }
        }
    }
    util::empty(500)
}

async fn invite_codes(call: Call) -> Response {
    let Host { store, .. } = match host_only(call).await {
        Ok(host) => host,
        Err(response) => return response,
    };
    match store
        .read(|conn, repos| repos.invites.list(conn, MAX_INVITES_LISTED))
        .await
    {
        Ok(records) => util::json(
            200,
            &json!({
                "InviteCodes": records.into_iter().map(|r| json!({
                    "code": r.code,
                    "usesRemaining": r.uses_remaining,
                    "infiniteUses": r.infinite_uses,
                    "note": r.note,
                    "createdAt": r.created_at,
                })).collect::<Vec<_>>()
            }),
        ),
        Err(error) => {
            tracing::error!("[launcher-api] InviteCodes: {error}");
            util::empty(500)
        }
    }
}

async fn delete_invite(call: Call) -> Response {
    let code = call.params.get("code").unwrap_or_default().to_owned();
    let Host { store, .. } = match host_only(call).await {
        Ok(host) => host,
        Err(response) => return response,
    };
    if !is_invite_code(&code) {
        return util::json(400, &json!({ "error": "bad_request" }));
    }
    match store
        .write(&actor_host(), move |tx, repos| repos.invites.delete(tx, &code))
        .await
    {
        Ok(gone) => util::empty(if gone { 200 } else { 404 }),
        Err(error) => {
            tracing::error!("[launcher-api] InviteCode: {error}");
            util::empty(500)
        }
    }
}

/// `AccountNamed`: `None` is a name that is no username at all (Node's `null`), `Some(None)` a name nobody has
/// (`undefined`), else the launcher account and the client's `userId`.
async fn account_named(
    store: &IslesStore,
    raw: Option<&Value>,
) -> RepoResult<Option<Option<(LauncherAccountRecord, String)>>> {
    let Some(username) = normalize_username(raw) else {
        return Ok(None);
    };
    let normalized = username.to_lowercase();
    store
        .read(move |conn, repos| {
            let Some(account) = repos
                .launcher_accounts
                .find_by_display_name_normalized(conn, &normalized)?
            else {
                return Ok(Some(None));
            };
            let user_id = repos
                .accounts
                .find(conn, &account.account)?
                .map_or_else(|| account.account.to_string(), |game| game.user_id.to_string());
            Ok(Some(Some((account, user_id))))
        })
        .await
}

fn status_name(status: OperationalStatus) -> &'static str {
    match status {
        OperationalStatus::Active => "active",
        OperationalStatus::Banned => "banned",
        OperationalStatus::Disabled => "disabled",
    }
}

async fn set_account_status(call: Call) -> Response {
    let Host { store, req } = match host_only(call).await {
        Ok(host) => host,
        Err(response) => return response,
    };
    let fields = req.fields();
    let status = match fields.get("Status").and_then(Value::as_str) {
        Some("active") => Some(OperationalStatus::Active),
        Some("banned") => Some(OperationalStatus::Banned),
        Some("disabled") => Some(OperationalStatus::Disabled),
        _ => None,
    };
    let found = match status {
        None => None,
        Some(_) => match account_named(&store, fields.get("Username")).await {
            Ok(found) => found,
            Err(error) => {
                tracing::error!("[launcher-api] SetAccountStatus: {error}");
                return util::empty(500);
            }
        },
    };
    let (Some(status), Some(found)) = (status, found) else {
        return util::json(
            400,
            &json!({
                "error": "bad_request",
                "message": "Username and Status (active, banned or disabled) are required.",
            }),
        );
    };
    let Some((account, user_id)) = found else {
        return refusal(404, Refusal::NotFound);
    };
    let change = AccessChange {
        approval_status: None,
        status: Some(status),
        approval_updated_at: util::now_iso(),
        approval_updated_by: "host".to_owned(),
        approval_reason: fields.get("Reason").and_then(Value::as_str).map(|r| js_slice(r, 500)),
    };
    let target = account.account.clone();
    let changed = store
        .account_write(&target, &actor_host(), {
            let target = target.clone();
            move |tx, repos| repos.launcher_accounts.set_access_state(tx, &target, &change)
        })
        .await;
    match changed {
        Ok(changed) => {
            forget_status();
            tracing::info!(
                "[launcher-api] {} ({user_id}) set {}",
                account.display_name,
                status_name(status)
            );
            let shown = changed.map_or(status, |record| record.status);
            util::json(
                200,
                &json!({ "UserId": user_id, "Username": account.display_name, "Status": status_name(shown) }),
            )
        }
        Err(error) => {
            tracing::error!("[launcher-api] SetAccountStatus: {error}");
            util::empty(500)
        }
    }
}

/// The admin service's kick (RD25): the player is dropped, not banned, and logs in again with their key.
async fn kick(call: Call) -> Response {
    let Host { store, req } = match host_only(call).await {
        Ok(host) => host,
        Err(response) => return response,
    };
    let fields = req.fields();
    let found = match account_named(&store, fields.get("Username")).await {
        Ok(found) => found,
        Err(error) => {
            tracing::error!("[launcher-api] Kick: {error}");
            return util::empty(500);
        }
    };
    let Some(found) = found else {
        return util::json(
            400,
            &json!({ "error": "bad_request", "message": "Username is required." }),
        );
    };
    let Some((account, user_id)) = found else {
        return refusal(404, Refusal::NotFound);
    };
    // Tokens are revoked first, so the game cannot sign straight back in
    let target = account.account.clone();
    let revoked = store
        .account_write(&target, &actor_host(), {
            let target = target.clone();
            move |tx, repos| {
                repos
                    .refresh_sessions
                    .revoke_all_for_account(tx, &target, &util::now_iso())?;
                repos
                    .game_exchange_codes
                    .revoke_unused_for_account(tx, &target, &util::now_iso())
            }
        })
        .await;
    if let Err(error) = revoked {
        tracing::error!("[launcher-api] Kick: {error}");
        return util::empty(500);
    }
    let closed =
        presence::chat_sessions().map_or(0, |chat| chat.close_connections(&user_id, 1008, "kicked by the host"));
    forget_status();
    tracing::info!(
        "[launcher-api] {} ({user_id}) kicked, {closed} chat connection(s) closed",
        account.display_name
    );
    util::json(
        200,
        &json!({ "UserId": user_id, "Username": account.display_name, "Closed": closed }),
    )
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Post, "/reforged/api/CreateInvite", |call| {
        Box::pin(create_invite(call))
    });
    handlers.on(b, Method::Get, "/reforged/api/InviteCodes", |call| {
        Box::pin(invite_codes(call))
    });
    handlers.on(b, Method::Delete, "/reforged/api/InviteCode/:code", |call| {
        Box::pin(delete_invite(call))
    });
    handlers.on(b, Method::Post, "/reforged/api/SetAccountStatus", |call| {
        Box::pin(set_account_status(call))
    });
    handlers.on(b, Method::Post, "/reforged/api/Kick", |call| Box::pin(kick(call)));
}
