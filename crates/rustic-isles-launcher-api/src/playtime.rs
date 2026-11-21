// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Online playtime: chat presence opens and closes an account's session, a tick credits the ones still online,
//! and `GET /dainty/api/Playtime` answers the key holder's total. No Node route: Dainty Lass's own.

use std::collections::BTreeSet;

use axum::response::Response;
use dainty_module_api::{Actor, ActorContext, GlobalId};
use http::StatusCode;
use rustic_isles::{AccountId, Call, Shared};
use rustic_isles_store::RepoError;
use rustic_isles_store::repo::playtime::{self, PlaytimeRecord};
use serde_json::Value;

use crate::launcher_api::key_holder;
use crate::util::{self, json, now_ms, object, store_of};

/// Opens or closes the account's session; an account homed on another node is that node's to count (DD17).
pub async fn record_presence(shared: &Shared, account: &GlobalId, online: bool) {
    let (Some(store), Ok(account)) = (store_of(shared), AccountId::try_from(account.clone())) else {
        return;
    };
    let now = now_ms();
    let id = account.clone();
    let written = store
        .account_write(&account, &ActorContext::new(Actor::System), move |tx, _| {
            let found = playtime::find(tx.conn(), &id)?;
            if found.is_none() && !online {
                return Ok(());
            }
            let mut record = found.unwrap_or_else(|| PlaytimeRecord::new(id));
            if online {
                record.open(now);
            } else {
                record.close(now);
            }
            playtime::save(tx, &record)
        })
        .await;
    match written {
        Ok(()) | Err(RepoError::NotHome { .. }) => {}
        Err(e) => tracing::warn!("[Playtime] {account}: {e}"),
    }
}

/// Credits every open session whose player chat still has online; the others close uncounted.
pub async fn run_tick(shared: &Shared) {
    let Some(store) = store_of(shared) else {
        return;
    };
    let online: BTreeSet<String> = shared.presence().online_user_ids().into_iter().collect();
    let now = now_ms();
    let ticked = store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            for mut record in playtime::find_open(tx.conn())? {
                let user = repos.accounts.find(tx.conn(), &record.account)?.map(|a| a.user_id);
                if user.is_some_and(|u| online.contains(u.as_str())) {
                    record.tick(now);
                } else {
                    record.last_seen_ms = None;
                }
                playtime::save(tx, &record)?;
            }
            Ok(())
        })
        .await;
    if let Err(e) = ticked {
        tracing::warn!("[Playtime] tick failed: {e}");
    }
}

/// Closes what a crash left open; run at start, before anyone is online.
pub async fn reset(shared: &Shared) {
    let Some(store) = store_of(shared) else {
        return;
    };
    match store
        .write(&ActorContext::new(Actor::System), |tx, _| playtime::close_all(tx))
        .await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!("[Playtime] closed {n} session(s) left open"),
        Err(e) => tracing::warn!("[Playtime] reset failed: {e}"),
    }
}

/// `GET /dainty/api/Playtime`: the key holder's online time in whole seconds.
pub async fn get_playtime(call: Call) -> Response {
    let Some(store) = store_of(&call.shared) else {
        return util::no_store(call.route, &call.request_id);
    };
    let Some((holder, _)) = key_holder(&call.shared, &store, call.request.headers()).await else {
        return json(
            StatusCode::UNAUTHORIZED,
            &object(vec![("error", Value::from("unauthorized"))]),
        );
    };
    if !holder.eligible() {
        return json(
            StatusCode::FORBIDDEN,
            &object(vec![("error", Value::from("account_disabled"))]),
        );
    }
    let account = holder.account.clone();
    let found = store.read(move |conn, _| playtime::find(conn, &account)).await;
    let total_ms = match found {
        Ok(record) => record.map_or(0, |r| r.total_ms(now_ms())),
        Err(e) => {
            tracing::error!("[Playtime] read failed: {e}");
            return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    util::ok(&object(vec![("PlaytimeSeconds", Value::from(total_ms.max(0) / 1000))]))
}
