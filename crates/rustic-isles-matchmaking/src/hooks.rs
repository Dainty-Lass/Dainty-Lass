// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the composer's `ModuleHooks` take from this crate (spec §12, §7's offline hook): the players of a placed
//! hunt, ring recording, and the leave of a player the chat plugin saw go.

use std::sync::Arc;

use dainty_module_api::{BoxFuture, GlobalId};
use rustic_isles::Shared;
use rustic_isles_store::{AccountId, IslesStore};

use crate::party::Parties;
use crate::state::MatchmakingState;

/// An account homed here and in the store becomes its userId; any other stays its global id text. Order kept.
pub fn place_hunt_players<'a>(shared: &'a Arc<Shared>, players: &'a [GlobalId]) -> BoxFuture<'a, Vec<String>> {
    Box::pin(async move {
        let store = shared.slot::<IslesStore>();
        let mut out = Vec::with_capacity(players.len());
        for player in players {
            out.push(
                user_of(store.as_deref(), shared, player)
                    .await
                    .unwrap_or_else(|| player.to_string()),
            );
        }
        out
    })
}

async fn user_of(store: Option<&IslesStore>, shared: &Shared, player: &GlobalId) -> Option<String> {
    let store = store?;
    if player.owner() != &shared.node().node {
        return None;
    }
    let account = AccountId::try_from(player.clone()).ok()?;
    let found = store
        .read(move |conn, repos| repos.accounts.find(conn, &account))
        .await
        .ok()??;
    Some(found.user_id.as_str().to_owned())
}

/// A placed hunt's players map to it; a placed hub (no players) becomes the last ring instance (spec §15 item 10).
pub fn record_ring_instance(shared: &Arc<Shared>, instance: &GlobalId, players: &[String]) {
    if let Some(state) = shared.slot::<MatchmakingState>() {
        state.record_ring_instance(instance, players);
    }
}

/// The player leaves their party; errors are ignored.
pub fn player_offline<'a>(shared: &'a Arc<Shared>, user: &'a str) -> BoxFuture<'a, ()> {
    Box::pin(async move {
        let store = shared.slot::<IslesStore>();
        let now = shared
            .slot::<MatchmakingState>()
            .map_or_else(rustic_isles::replay::now_ms, |s| s.now_ms());
        let parties = Parties {
            store: store.as_deref(),
            node: &shared.node().node,
            request_id: None,
            now,
        };
        if parties.leave(user).await.is_ok() {
            crate::routes::member_left(shared, user).await;
        }
    })
}
