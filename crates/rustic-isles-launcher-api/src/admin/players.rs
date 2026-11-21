// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Players and the audit (admin spec §6).

use axum::response::Response;
use http::StatusCode;
use rustic_isles::UserId;
use rustic_isles_store::RepoError;
use rustic_isles_store::repo::auth::{
    AccessChange, AdminAccountFilter, ApprovalStatus, LauncherAccountRecord, OperationalStatus,
};
use rustic_isles_store::repo::reforged::{AdminAuditRecord, PlayerDeletion};
use serde_json::{Map, Value};

use super::{
    AdminContext, admin_actor, admin_error, admin_internal, fresh_id, limit_of, page_of, player_not_found,
    public_account, skip_of, user_id_of,
};
use crate::state::LauncherApiState;
use crate::util::{self, iso, now_ms, object};

const ASSIGNABLE: [&str; 1] = ["tester"];

fn reason(ctx: &AdminContext) -> String {
    ctx.request
        .field("reason")
        .and_then(Value::as_str)
        .map(|r| util::take_chars(util::trim(r), 500))
        .unwrap_or_default()
}

fn approval_text(account: &LauncherAccountRecord) -> &'static str {
    account.approval_status.map_or("approved", ApprovalStatus::as_str)
}

/// `GET /admin/v1/online-players`: the players online now.
pub async fn online(ctx: AdminContext) -> Response {
    let state = LauncherApiState::of(&ctx.shared);
    let ids = state.active_players(&ctx.shared, 60_000);
    let found = ctx
        .store
        .read(move |conn, repos| {
            let mut players = Vec::new();
            for id in ids {
                let Some(game) = repos.accounts.find_by_user_id(conn, &UserId::new(id.clone()))? else {
                    continue;
                };
                let Some(launcher) = repos.launcher_accounts.find_by_account(conn, &game.id)? else {
                    continue;
                };
                players.push((id, launcher.display_name));
            }
            Ok(players)
        })
        .await;
    let mut players = match found {
        Ok(players) => players,
        Err(e) => return admin_internal(e),
    };
    players.sort_by(|a, b| dainty_service_kit::collate::locale_compare(&a.1, &b.1));
    let list: Vec<Value> = players
        .into_iter()
        .map(|(id, name)| object(vec![("userId", Value::from(id)), ("displayName", Value::from(name))]))
        .collect();
    util::ok(&object(vec![
        ("count", Value::from(list.len())),
        ("players", Value::Array(list)),
        ("observedAt", Value::from(iso(now_ms()))),
        ("source", Value::from("xmpp+presence")),
    ]))
}

fn is_player_id(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `GET /admin/v1/players`: a page of launcher accounts, filtered and searched.
pub async fn list(ctx: AdminContext) -> Response {
    let now = iso(now_ms());
    if let Some(ids) = ctx.request.joined("ids").filter(|t| !t.trim().is_empty()) {
        let mut kept: Vec<String> = Vec::new();
        for id in ids.split(',').map(str::trim).filter(|i| is_player_id(i)) {
            if !kept.iter().any(|k| k == id) {
                kept.push(id.to_owned());
            }
        }
        kept.truncate(100);
        let count = kept.len();
        let found = ctx
            .store
            .read(move |conn, repos| {
                let mut out = Vec::new();
                for id in kept {
                    if let Some(account) = super::find_target(conn, repos, &id)? {
                        let user_id = user_id_of(conn, repos, &account.account);
                        out.push(public_account(&account, &user_id, &now));
                    }
                }
                Ok(out)
            })
            .await;
        return match found {
            Ok(accounts) => {
                let total = accounts.len();
                util::ok(&object(vec![
                    ("accounts", Value::Array(accounts)),
                    ("total", Value::from(total)),
                    ("page", Value::from(1)),
                    ("limit", Value::from(count)),
                ]))
            }
            Err(e) => admin_internal(e),
        };
    }
    let approval = ctx
        .request
        .joined("approvalStatus")
        .and_then(|t| ApprovalStatus::parse(&t));
    let status_text = ctx.request.joined("status").unwrap_or_default();
    let (status, timed_out) = match status_text.as_str() {
        "timedOut" => (None, Some(now.clone())),
        other => (OperationalStatus::parse(other), None),
    };
    let search = ctx
        .request
        .single("search")
        .map(|s| util::take_chars(util::trim(s), 100))
        .filter(|s| !s.is_empty());
    let page = page_of(&ctx.request);
    let limit = limit_of(&ctx.request, 25, 100);
    let filter = AdminAccountFilter {
        approval_status: approval,
        status,
        timed_out_at: timed_out,
        search,
        skip: skip_of(page, limit),
        limit,
    };
    let found = ctx
        .store
        .read(move |conn, repos| {
            let (list, total) = repos.launcher_accounts.list_for_admin(conn, &filter)?;
            let shown: Vec<Value> = list
                .iter()
                .map(|a| public_account(a, &user_id_of(conn, repos, &a.account), &now))
                .collect();
            Ok((shown, total))
        })
        .await;
    match found {
        Ok((accounts, total)) => util::ok(&object(vec![
            ("accounts", Value::Array(accounts)),
            ("total", Value::from(total)),
            ("page", Value::from(page)),
            ("limit", Value::from(limit)),
        ])),
        Err(e) => admin_internal(e),
    }
}

fn validation(message: &str) -> Response {
    admin_error(StatusCode::BAD_REQUEST, "ADMIN_VALIDATION", message)
}

/// A request field that is absent or `null` is none; another value must parse.
fn optional<T>(value: Option<&Value>, parse: impl Fn(&str) -> Option<T>) -> Result<Option<T>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => parse(text).map(Some).ok_or(()),
        Some(_) => Err(()),
    }
}

enum Access {
    NotFound,
    Done(Box<LauncherAccountRecord>, String),
}

/// `PATCH /admin/v1/players/:userId/access`: sets a player's approval, status or timeout.
pub async fn access(ctx: AdminContext) -> Response {
    let Ok(approval) = optional(ctx.request.field("approvalStatus"), ApprovalStatus::parse) else {
        return validation("Invalid approval status.");
    };
    let Ok(status) = optional(ctx.request.field("status"), OperationalStatus::parse) else {
        return validation("Invalid account status.");
    };
    if approval.is_none() && status.is_none() {
        return validation("No access-state change was supplied.");
    }
    let reason = reason(&ctx);
    if util::utf16_len(&reason) < 3 {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_ACCESS_REASON",
            "Provide a reason of at least 3 characters for every server access change.",
        );
    }
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("the access change had no caller");
    };
    let target_id = ctx.param("userId").to_owned();
    let locks_out = approval.is_some_and(|a| a != ApprovalStatus::Approved)
        || status.is_some_and(|s| s != OperationalStatus::Active);
    if target_id == caller.user_id && locks_out {
        return admin_error(
            StatusCode::CONFLICT,
            "ADMIN_SELF_LOCKOUT",
            "You cannot reject, disable, or ban your own administrator account.",
        );
    }
    let lookup = target_id.clone();
    let target = match ctx
        .store
        .read(move |conn, repos| super::find_target(conn, repos, &lookup))
        .await
    {
        Ok(Some(target)) => target,
        Ok(None) => return player_not_found(),
        Err(e) => return admin_internal(e),
    };
    let request_id = fresh_id();
    let rid = request_id.clone();
    let ip = ctx.request.client_ip.clone();
    let by = caller.user_id.clone();
    let path_user = target_id.clone();
    let account = target.account.clone();
    let home = target.account.clone();
    let written = ctx
        .store
        .account_write(&home, &admin_actor(&caller), move |tx, repos| {
            let now = iso(now_ms());
            let change = AccessChange {
                approval_status: approval,
                status,
                approval_updated_at: now.clone(),
                approval_updated_by: by.clone(),
                approval_reason: Some(reason.clone()),
            };
            let Some(after) = repos.launcher_accounts.set_access_state(tx, &account, &change)? else {
                return Ok(Access::NotFound);
            };
            if !crate::launcher_api::is_account_eligible(&after) {
                repos.refresh_sessions.revoke_all_for_account(tx, &account, &now)?;
                repos
                    .game_exchange_codes
                    .revoke_unused_for_account(tx, &account, &now)?;
            }
            let state = |a: &LauncherAccountRecord| {
                object(vec![
                    ("approvalStatus", Value::from(approval_text(a))),
                    ("status", Value::from(a.status.as_str())),
                ])
            };
            repos.admin.append_audit(
                tx,
                &AdminAuditRecord {
                    id: fresh_id(),
                    actor_user_id: by,
                    target_user_id: Some(path_user),
                    action: "player.access.update".to_owned(),
                    old_state: Some(state(&target)),
                    new_state: Some(state(&after)),
                    reason: Some(reason),
                    ip,
                    request_id: rid,
                    created_at: now.clone(),
                },
            )?;
            Ok(Access::Done(Box::new(after), now))
        })
        .await;
    match written {
        Ok(Access::Done(after, now)) => util::ok(&object(vec![
            ("account", public_account(&after, &target_id, &now)),
            ("requestId", Value::from(request_id)),
        ])),
        Ok(Access::NotFound) => player_not_found(),
        Err(RepoError::NotHome { .. }) => admin_error(
            StatusCode::CONFLICT,
            "ADMIN_NOT_HOME",
            "That player's account lives on another server.",
        ),
        Err(e) => admin_internal(e),
    }
}

/// `PATCH /admin/v1/players/:userId/roles`: sets a player's roles.
pub async fn roles(ctx: AdminContext) -> Response {
    let requested: Option<Vec<String>> = ctx.request.field("roles").and_then(Value::as_array).and_then(|list| {
        list.iter()
            .map(|r| r.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
    });
    let Some(requested) = requested else {
        return validation("roles must be an array of strings.");
    };
    if requested.iter().any(|r| !ASSIGNABLE.contains(&r.as_str())) {
        return validation("Only these roles can be toggled here: tester.");
    }
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("the role change had no caller");
    };
    let target_id = ctx.param("userId").to_owned();
    let lookup = target_id.clone();
    let target = match ctx
        .store
        .read(move |conn, repos| super::find_target(conn, repos, &lookup))
        .await
    {
        Ok(Some(target)) => target,
        Ok(None) => return player_not_found(),
        Err(e) => return admin_internal(e),
    };
    let mut roles: Vec<String> = target
        .roles
        .iter()
        .filter(|r| !ASSIGNABLE.contains(&r.as_str()))
        .cloned()
        .collect();
    roles.extend(requested);
    let reason = reason(&ctx);
    let request_id = fresh_id();
    let rid = request_id.clone();
    let ip = ctx.request.client_ip.clone();
    let by = caller.user_id.clone();
    let path_user = target_id.clone();
    let account = target.account.clone();
    let home = target.account.clone();
    let written = ctx
        .store
        .account_write(&home, &admin_actor(&caller), move |tx, repos| {
            let now = iso(now_ms());
            let Some(after) = repos.launcher_accounts.update_roles(tx, &account, &roles, &now, &by)? else {
                return Ok(None);
            };
            let had = target.roles.iter().any(|r| r == "tester");
            let has = after.roles.iter().any(|r| r == "tester");
            if had && !has {
                repos
                    .game_exchange_codes
                    .revoke_unused_for_account(tx, &account, &now)?;
            }
            let list = |r: &[String]| {
                object(vec![(
                    "roles",
                    Value::Array(r.iter().cloned().map(Value::from).collect()),
                )])
            };
            repos.admin.append_audit(
                tx,
                &AdminAuditRecord {
                    id: fresh_id(),
                    actor_user_id: by,
                    target_user_id: Some(path_user),
                    action: "player.roles.update".to_owned(),
                    old_state: Some(list(&target.roles)),
                    new_state: Some(list(&after.roles)),
                    reason: (!reason.is_empty()).then_some(reason),
                    ip,
                    request_id: rid,
                    created_at: now.clone(),
                },
            )?;
            Ok(Some((after, now)))
        })
        .await;
    match written {
        Ok(Some((after, now))) => util::ok(&object(vec![
            ("account", public_account(&after, &target_id, &now)),
            ("requestId", Value::from(request_id)),
        ])),
        Ok(None) => player_not_found(),
        Err(e) => admin_internal(e),
    }
}

/// `DELETE /admin/v1/players/:userId`: deletes a player's account; never the caller's own.
pub async fn delete(ctx: AdminContext) -> Response {
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("the deletion had no caller");
    };
    let target_id = ctx.param("userId").to_owned();
    if target_id == caller.user_id {
        return admin_error(
            StatusCode::CONFLICT,
            "ADMIN_SELF_DELETE",
            "You cannot delete your own administrator account.",
        );
    }
    let lookup = target_id.clone();
    let target = match ctx
        .store
        .read(move |conn, repos| super::find_target(conn, repos, &lookup))
        .await
    {
        Ok(Some(target)) => target,
        Ok(None) => return player_not_found(),
        Err(e) => return admin_internal(e),
    };
    let expected = format!("DELETE {}", target.display_name);
    if ctx.request.field("confirmation").and_then(Value::as_str) != Some(expected.as_str()) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_DELETE_CONFIRMATION",
            &format!("Type \"DELETE {}\" to confirm permanent deletion.", target.display_name),
        );
    }
    let reason = reason(&ctx);
    if util::utf16_len(&reason) < 3 {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_DELETE_REASON",
            "Provide a deletion reason for the audit log.",
        );
    }
    let request_id = fresh_id();
    let audit = AdminAuditRecord {
        id: fresh_id(),
        actor_user_id: caller.user_id.clone(),
        target_user_id: Some(target_id.clone()),
        action: "player.account.delete".to_owned(),
        old_state: Some(object(vec![
            ("approvalStatus", Value::from(approval_text(&target))),
            ("status", Value::from(target.status.as_str())),
            (
                "roles",
                Value::Array(target.roles.iter().cloned().map(Value::from).collect()),
            ),
        ])),
        new_state: Some(object(vec![("deleted", Value::Bool(true))])),
        reason: Some(reason),
        ip: ctx.request.client_ip.clone(),
        request_id: request_id.clone(),
        created_at: iso(now_ms()),
    };
    let account = target.account.clone();
    let user = target_id.clone();
    let written = ctx
        .store
        .account_write(&target.account, &admin_actor(&caller), move |tx, repos| {
            let deletion: PlayerDeletion = repos.admin.delete_player_data(tx, &account, &user, &audit)?;
            let now = iso(now_ms());
            repos.refresh_sessions.delete_all_for_account(tx, &account)?;
            repos
                .game_exchange_codes
                .revoke_unused_for_account(tx, &account, &now)?;
            Ok(deletion)
        })
        .await;
    match written {
        Ok(deletion) => {
            let mut counts = Map::new();
            for (name, n) in deletion.deleted_counts {
                counts.insert(name, Value::from(n));
            }
            util::ok(&object(vec![
                ("requestId", Value::from(request_id)),
                ("deletedCounts", Value::Object(counts)),
                ("characterCount", Value::from(deletion.character_count)),
            ]))
        }
        Err(e) => admin_internal(e),
    }
}

/// An audit row as the dashboard reads it.
pub fn audit_json(r: &AdminAuditRecord) -> Value {
    let mut pairs = vec![
        ("id", Value::from(r.id.clone())),
        ("actorUserId", Value::from(r.actor_user_id.clone())),
    ];
    if let Some(t) = &r.target_user_id {
        pairs.push(("targetUserId", Value::from(t.clone())));
    }
    pairs.push(("action", Value::from(r.action.clone())));
    if let Some(s) = &r.old_state {
        pairs.push(("oldState", s.clone()));
    }
    if let Some(s) = &r.new_state {
        pairs.push(("newState", s.clone()));
    }
    if let Some(s) = &r.reason {
        pairs.push(("reason", Value::from(s.clone())));
    }
    pairs.push(("ip", Value::from(r.ip.clone())));
    pairs.push(("requestId", Value::from(r.request_id.clone())));
    pairs.push(("createdAt", Value::from(r.created_at.clone())));
    object(pairs)
}

/// `GET /admin/v1/audit`: a page of the admin audit.
pub async fn audit(ctx: AdminContext) -> Response {
    let page = page_of(&ctx.request);
    let limit = limit_of(&ctx.request, 50, 100);
    let skip = skip_of(page, limit);
    let target = ctx
        .request
        .single("targetUserId")
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    match ctx
        .store
        .read(move |conn, repos| repos.admin.list_audit(conn, target.as_deref(), skip, limit))
        .await
    {
        Ok((records, total)) => util::ok(&object(vec![
            ("records", Value::Array(records.iter().map(audit_json).collect())),
            ("total", Value::from(total)),
            ("page", Value::from(page)),
            ("limit", Value::from(limit)),
        ])),
        Err(e) => admin_internal(e),
    }
}
