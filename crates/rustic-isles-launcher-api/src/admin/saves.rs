// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The save snapshots a player's `/reset` and staff restores leave (`rustic_isles_store::repo::resets`): listed, and
//! put back with an audit row. A restore snapshots the save it replaces first, so it can be undone the same way.

use axum::response::Response;
use http::StatusCode;
use rustic_isles_store::repo::reforged::AdminAuditRecord;
use rustic_isles_store::repo::resets::{self, SaveSnapshot};
use serde_json::Value;

use super::{AdminContext, admin_actor, admin_error, admin_internal, fresh_id, player_not_found};
use crate::util::{self, iso, now_ms, object};

fn snapshot_json(s: &SaveSnapshot) -> Value {
    object(vec![
        ("id", Value::from(s.id.clone())),
        ("source", Value::from(s.source.clone())),
        ("actorUserId", s.actor_user_id.clone().map_or(Value::Null, Value::from)),
        ("createdAt", Value::from(iso(s.created_at))),
        ("expiresAt", Value::from(iso(s.expires_at))),
        (
            "restoredAt",
            s.restored_at.map_or(Value::Null, |at| Value::from(iso(at))),
        ),
        ("rows", Value::from(s.rows)),
    ])
}

/// `GET /admin/v1/players/:userId/save-snapshots`: the player's snapshots, newest first.
pub async fn list(ctx: AdminContext) -> Response {
    let lookup = ctx.param("userId").to_owned();
    let listed = ctx
        .store
        .read(move |conn, repos| {
            let Some(target) = super::find_target(conn, repos, &lookup)? else {
                return Ok(None);
            };
            resets::list(conn, &target.account).map(Some)
        })
        .await;
    match listed {
        Ok(Some(snapshots)) => util::ok(&object(vec![(
            "snapshots",
            Value::Array(snapshots.iter().map(snapshot_json).collect()),
        )])),
        Ok(None) => player_not_found(),
        Err(e) => admin_internal(e),
    }
}

/// `POST /admin/v1/players/:userId/save-snapshots/:snapshotId/restore`: puts the snapshot back as the save.
pub async fn restore(ctx: AdminContext) -> Response {
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("the restore had no caller");
    };
    let reason = ctx
        .request
        .field("reason")
        .and_then(Value::as_str)
        .map(|r| util::take_chars(util::trim(r), 500))
        .unwrap_or_default();
    if reason.chars().count() < 3 {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_RESTORE_REASON",
            "A reason of at least 3 characters is required.",
        );
    }
    let user_id = ctx.param("userId").to_owned();
    let snapshot_id = ctx.param("snapshotId").to_owned();
    let (lookup, wanted) = (user_id.clone(), snapshot_id.clone());
    let found = ctx
        .store
        .read(move |conn, repos| {
            let Some(target) = super::find_target(conn, repos, &lookup)? else {
                return Ok(None);
            };
            let snapshot = resets::find(conn, &target.account, &wanted)?;
            Ok(Some((target.account, snapshot)))
        })
        .await;
    let (account, snapshot) = match found {
        Ok(Some((account, Some(snapshot)))) => (account, snapshot),
        Ok(Some((_, None))) => {
            return admin_error(
                StatusCode::NOT_FOUND,
                "ADMIN_SNAPSHOT_NOT_FOUND",
                "No such save snapshot.",
            );
        }
        Ok(None) => return player_not_found(),
        Err(e) => return admin_internal(e),
    };
    if snapshot.restored_at.is_some() {
        return admin_error(
            StatusCode::CONFLICT,
            "ADMIN_SNAPSHOT_RESTORED",
            "This snapshot was already restored.",
        );
    }
    let now = now_ms();
    let replaced = fresh_id();
    let request_id = fresh_id();
    let audit = AdminAuditRecord {
        id: fresh_id(),
        actor_user_id: caller.user_id.clone(),
        target_user_id: Some(user_id.clone()),
        action: "player.save.restore".to_owned(),
        old_state: Some(object(vec![("replacedBySnapshot", Value::from(replaced.clone()))])),
        new_state: Some(object(vec![("restoredSnapshot", Value::from(snapshot_id.clone()))])),
        reason: Some(reason),
        ip: ctx.request.client_ip.clone(),
        request_id: request_id.clone(),
        created_at: iso(now),
    };
    let (key, actor_id, replaced_id) = (account.clone(), caller.user_id.clone(), replaced.clone());
    let written = ctx
        .store
        .account_write(&account, &admin_actor(&caller), move |tx, repos| {
            resets::prune(tx, now)?;
            let moved = resets::restore(tx, &key, &snapshot_id, &replaced_id, &actor_id, now)?;
            repos.admin.append_audit(tx, &audit)?;
            Ok(moved)
        })
        .await;
    match written {
        Ok(moved) => {
            tracing::info!(
                "[AdminSave] restore user={user_id} rows={moved} actor={}",
                caller.user_id
            );
            util::ok(&object(vec![
                ("requestId", Value::from(request_id)),
                ("replacedSnapshotId", Value::from(replaced)),
                ("rows", Value::from(moved)),
            ]))
        }
        Err(e) => admin_internal(e),
    }
}
