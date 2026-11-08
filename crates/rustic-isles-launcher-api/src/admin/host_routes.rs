// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The host kit's own `/dainty/api/*` calls (admin spec §9): loopback, no gateway stamp, the host key.

use axum::response::Response;
use dainty_module_api::{Actor, ActorContext};
use http::{HeaderMap, StatusCode};
use rustic_isles::Call;
use rustic_isles_store::repo::auth::{AccessChange, LauncherAccountRecord, OperationalStatus};
use rustic_isles_store::repo::reforged::InviteRecord;
use rustic_isles_store::{IslesStore, RepoError};
use serde_json::Value;

use super::auth::is_loopback_peer;
use super::user_id_of;
use crate::launcher_api::{Refusal, invite_note, is_invite_code, make_invite_code, normalize_username};
use crate::state::LauncherApiState;
use crate::util::{self, iso, json, now_ms, object};

/// The local admin check: a long enough key, a loopback peer, no gateway or forwarding stamp, the key's SHA-256.
pub fn local_admin(settings: &rustic_isles::Settings, peer: &str, headers: &HeaderMap) -> bool {
    let configured = settings.raw("DAINTY_ADMIN_KEY").unwrap_or("");
    if util::utf16_len(configured) < 32 || !is_loopback_peer(peer) {
        return false;
    }
    if headers.contains_key("x-dainty-gateway") || headers.contains_key("x-forwarded-for") {
        return false;
    }
    let sent = util::header_text(headers, "x-dainty-admin-key").unwrap_or_default();
    util::ct_eq(&util::sha256(sent.as_bytes()), &util::sha256(configured.as_bytes()))
}

fn console() -> ActorContext {
    ActorContext::new(Actor::Console)
}

fn forbidden() -> Response {
    json(
        StatusCode::FORBIDDEN,
        &object(vec![("error", Value::from("forbidden"))]),
    )
}

/// The body, the check, the store, in that order.
async fn enter(call: &mut Call) -> Result<(Option<Value>, std::sync::Arc<IslesStore>), Response> {
    let body = util::read_json(&mut call.request).await?;
    let peer = util::peer_text(&call.request);
    if !local_admin(call.shared.settings(), &peer, call.request.headers()) {
        return Err(forbidden());
    }
    let Some(store) = util::store_of(&call.shared) else {
        return Err(util::no_store(call.route, &call.request_id));
    };
    Ok((body, store))
}

fn bad_request(message: Option<&str>) -> Response {
    let body = match message {
        Some(m) => object(vec![("error", Value::from("bad_request")), ("message", Value::from(m))]),
        None => object(vec![("error", Value::from("bad_request"))]),
    };
    json(StatusCode::BAD_REQUEST, &body)
}

/// `POST /dainty/api/CreateInvite`: mints a registration invite code.
pub async fn create_invite(mut call: Call) -> Response {
    let (body, store) = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    let body = body.as_ref();
    let uses = match util::field(body, "uses") {
        None | Some(Value::Null) => Some(1),
        value => dainty_service_kit::js::safe_integer(value).filter(|n| (1..=1000).contains(n)),
    };
    let name = util::field(body, "name");
    let (Some(uses), true) = (uses, name.is_none_or(Value::is_string)) else {
        return bad_request(Some("uses is a whole number from 1 to 1000; name is text"));
    };
    let infinite = util::field(body, "infinite") == Some(&Value::Bool(true));
    let note = invite_note(name);
    for _ in 0..5 {
        let record = InviteRecord {
            code: make_invite_code(),
            uses_remaining: uses,
            infinite_uses: infinite,
            note: note.clone(),
            created_at: iso(now_ms()),
        };
        let code = record.code.clone();
        match store
            .write(&console(), move |tx, repos| repos.invites.insert_if_absent(tx, &record))
            .await
        {
            Ok(true) => {
                let count = if infinite {
                    "unlimited".to_owned()
                } else {
                    format!("{uses} use(s)")
                };
                let named = note.as_ref().map(|n| format!(" for {n}")).unwrap_or_default();
                tracing::info!("[launcher-api] invite made: {count}{named}");
                return util::ok(&object(vec![("code", Value::from(code))]));
            }
            Ok(false) => {}
            Err(e) => {
                tracing::error!("[launcher-api] CreateInvite: {e}");
                return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }
    util::empty(StatusCode::INTERNAL_SERVER_ERROR)
}

/// `GET /dainty/api/InviteCodes`: lists the invite codes.
pub async fn invite_codes(mut call: Call) -> Response {
    let (_, store) = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    match store.read(|conn, repos| repos.invites.list(conn, 500)).await {
        Ok(list) => {
            let codes: Vec<Value> = list
                .into_iter()
                .map(|i| {
                    object(vec![
                        ("code", Value::from(i.code)),
                        ("usesRemaining", Value::from(i.uses_remaining)),
                        ("infiniteUses", Value::Bool(i.infinite_uses)),
                        ("note", i.note.map_or(Value::Null, Value::from)),
                        ("createdAt", Value::from(i.created_at)),
                    ])
                })
                .collect();
            util::ok(&object(vec![("InviteCodes", Value::Array(codes))]))
        }
        Err(e) => {
            tracing::error!("[launcher-api] InviteCodes: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// `DELETE /dainty/api/InviteCode/:code`: deletes an invite code.
pub async fn delete_invite(mut call: Call) -> Response {
    let (_, store) = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    let code = call.params.get("code").unwrap_or("").to_owned();
    if !is_invite_code(&code) {
        return bad_request(None);
    }
    match store
        .write(&console(), move |tx, repos| repos.invites.delete(tx, &code))
        .await
    {
        Ok(true) => util::empty(StatusCode::OK),
        Ok(false) => util::empty(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!("[launcher-api] InviteCode: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn by_name(store: &IslesStore, name: &str) -> Result<Option<(LauncherAccountRecord, String)>, RepoError> {
    let normalized = name.to_lowercase();
    store
        .read(move |conn, repos| {
            let Some(account) = repos
                .launcher_accounts
                .find_by_display_name_normalized(conn, &normalized)?
            else {
                return Ok(None);
            };
            let user_id = user_id_of(conn, repos, &account.account);
            Ok(Some((account, user_id)))
        })
        .await
}

/// `POST /dainty/api/SetAccountStatus`: sets an account's operational status.
pub async fn set_account_status(mut call: Call) -> Response {
    let (body, store) = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    let body = body.as_ref();
    let status = util::str_field(body, "Status")
        .filter(|s| matches!(*s, "active" | "banned" | "disabled"))
        .and_then(OperationalStatus::parse);
    let name = status.and_then(|_| normalize_username(util::field(body, "Username")));
    let (Some(status), Some(name)) = (status, name) else {
        return bad_request(Some("Username and Status (active, banned or disabled) are required."));
    };
    let reason = util::str_field(body, "Reason").map(|r| util::take_chars(r, 500));
    let (account, user_id) = match by_name(&store, &name).await {
        Ok(Some(found)) => found,
        Ok(None) => return Refusal::NotFound.response(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!("[launcher-api] SetAccountStatus: {e}");
            return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let id = account.account.clone();
    let written = store
        .account_write(&account.account, &console(), move |tx, repos| {
            let change = AccessChange {
                approval_status: None,
                status: Some(status),
                approval_updated_at: iso(now_ms()),
                approval_updated_by: "host".to_owned(),
                approval_reason: reason,
            };
            repos.launcher_accounts.set_access_state(tx, &id, &change)
        })
        .await;
    match written {
        Ok(after) => {
            LauncherApiState::of(&call.shared).forget_status();
            let stored = after.map_or(status, |a| a.status);
            tracing::info!(
                "[launcher-api] {} ({user_id}) set {}",
                account.display_name,
                status.as_str()
            );
            util::ok(&object(vec![
                ("UserId", Value::from(user_id)),
                ("Username", Value::from(account.display_name)),
                ("Status", Value::from(stored.as_str())),
            ]))
        }
        Err(e) => {
            tracing::error!("[launcher-api] SetAccountStatus: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// `POST /dainty/api/Kick`: disconnects a player by username.
pub async fn kick(mut call: Call) -> Response {
    let (body, store) = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    let Some(name) = normalize_username(util::field(body.as_ref(), "Username")) else {
        return bad_request(Some("Username is required."));
    };
    let (account, user_id) = match by_name(&store, &name).await {
        Ok(Some(found)) => found,
        Ok(None) => return Refusal::NotFound.response(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!("[launcher-api] Kick: {e}");
            return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let id = account.account.clone();
    let written = store
        .account_write(&account.account, &console(), move |tx, repos| {
            let now = iso(now_ms());
            repos.refresh_sessions.revoke_all_for_account(tx, &id, &now)?;
            repos.game_exchange_codes.revoke_unused_for_account(tx, &id, &now)
        })
        .await;
    if let Err(e) = written {
        tracing::error!("[launcher-api] Kick: {e}");
        return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let closed = call
        .shared
        .presence()
        .close_connections(&user_id, 1008, "kicked by the host");
    LauncherApiState::of(&call.shared).forget_status();
    tracing::info!(
        "[launcher-api] {} ({user_id}) kicked, {closed} chat connection(s) closed",
        account.display_name
    );
    util::ok(&object(vec![
        ("UserId", Value::from(user_id)),
        ("Username", Value::from(account.display_name)),
        ("Closed", Value::from(closed)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_admin_rules() {
        let key = "k".repeat(32);
        let settings = rustic_isles::Settings::from_pairs([("DAINTY_ADMIN_KEY", key.as_str())]);
        let mut headers = HeaderMap::new();
        headers.insert("x-dainty-admin-key", key.parse().unwrap());
        assert!(local_admin(&settings, "127.0.0.1", &headers));
        assert!(local_admin(&settings, "::ffff:127.0.0.2", &headers));
        assert!(!local_admin(&settings, "10.0.0.1", &headers));
        let mut gateway = headers.clone();
        gateway.insert("x-dainty-gateway", "1".parse().unwrap());
        assert!(!local_admin(&settings, "127.0.0.1", &gateway));
        let short = rustic_isles::Settings::from_pairs([("DAINTY_ADMIN_KEY", "k".repeat(31).as_str())]);
        assert!(!local_admin(&short, "127.0.0.1", &headers));
        assert!(!local_admin(&settings, "127.0.0.1", &HeaderMap::new()));
    }
}
