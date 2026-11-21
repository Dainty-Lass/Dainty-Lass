// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Gauntlet progress kept in the store: the hunt server's finished runs, and the progress, level access and
//! leaderboard answers read from them. Without a store every answer is the stubs' (nothing earned).

use std::sync::Arc;

use dainty_module_api::{Actor, ActorContext};
use rustic_isles::{AccountId, Shared, UserId, replay, season};
use rustic_isles_store::repo::gauntlet::{self, GauntletBoardEntry, GauntletProgress};
use rustic_isles_store::{IslesStore, RepoError};
use serde_json::{Value, json};

/// The leaderboard's length.
const BOARD_SIZE: i64 = 100;

fn store(shared: &Shared) -> Option<Arc<IslesStore>> {
    shared.slot::<IslesStore>()
}

async fn account_of(store: &IslesStore, user: &str) -> Option<AccountId> {
    let key = UserId::new(user);
    store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &key))
        .await
        .ok()
        .flatten()
        .map(|record| record.id)
}

/// One finished run as the hunt server reports it (`PUT /process_run`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedRun {
    /// The season, such as `ESC_SEASON_1`.
    pub season_id: String,
    /// The userIds that played it.
    pub players: Vec<String>,
    /// The level finished (plain, not combined with the season).
    pub level: i64,
    /// Seconds left on the level's timer.
    pub remaining_sec: i64,
}

impl FinishedRun {
    /// `{"gauntlet_id","players","level","remaining_sec","data"}`; none when a field is missing or the level is
    /// not a positive integer.
    pub fn parse(body: &Value) -> Option<Self> {
        let season_id = body.get("gauntlet_id")?.as_str()?.trim();
        let level = body
            .get("level")?
            .as_i64()
            .filter(|l| (1..=i64::from(i32::MAX)).contains(l))?;
        let remaining_sec = body.get("remaining_sec").and_then(Value::as_i64).unwrap_or(0);
        let players: Vec<String> = body
            .get("players")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .filter(|p| !p.trim().is_empty())
            .map(str::to_owned)
            .collect();
        (!season_id.is_empty() && !players.is_empty()).then(|| Self {
            season_id: season_id.to_owned(),
            players,
            level,
            remaining_sec,
        })
    }
}

/// Records the run for each player this node is home to.
pub async fn record(shared: &Shared, run: &FinishedRun) {
    let Some(store) = store(shared) else {
        tracing::warn!(
            "[Gauntlet] level {} finished but there is no store to record it",
            run.level
        );
        return;
    };
    let now = season::to_iso_string(replay::now_ms());
    for player in &run.players {
        let Some(account) = account_of(&store, player).await else {
            tracing::warn!("[Gauntlet] finished run for unknown user {player}");
            continue;
        };
        let (id, season_id, level, remaining, at) = (
            account.clone(),
            run.season_id.clone(),
            run.level,
            run.remaining_sec,
            now.clone(),
        );
        let written = store
            .account_write(&account, &ActorContext::new(Actor::System), move |tx, _| {
                let mut progress = gauntlet::find(tx.conn(), &id, &season_id)?
                    .unwrap_or_else(|| GauntletProgress::new(id, &season_id));
                progress.record(level, remaining);
                gauntlet::save(tx, &progress, &at)?;
                Ok(progress)
            })
            .await;
        match written {
            Ok(p) => tracing::info!(
                "[Gauntlet] {player} finished level {level} with {remaining}s left; best level {} ({}s), {} runs",
                p.best_level,
                p.best_remaining_sec,
                p.runs
            ),
            Err(RepoError::NotHome { .. }) => {}
            Err(e) => tracing::warn!("[Gauntlet] {player}: the run was not recorded: {e}"),
        }
    }
}

/// The player's best finished level in the season, 0 when none (or no store).
pub async fn best_level(shared: &Shared, season_id: &str, user: &str) -> i64 {
    let Some(store) = store(shared) else {
        return 0;
    };
    let Some(account) = account_of(&store, user).await else {
        return 0;
    };
    let season_id = season_id.to_owned();
    store
        .read(move |conn, _| gauntlet::find(conn, &account, &season_id))
        .await
        .ok()
        .flatten()
        .map_or(0, |p| p.best_level)
}

/// The season's board as the client parses it: `[{guild_name, guild_nameplate, level, remaining_sec}]`. There are
/// no guilds yet, so each line is one player under their own name.
pub async fn leaderboard(shared: &Shared, season_id: &str) -> Value {
    let Some(store) = store(shared) else {
        return json!([]);
    };
    let season_id = season_id.to_owned();
    let entries: Vec<GauntletBoardEntry> = store
        .read(move |conn, _| gauntlet::leaderboard(conn, &season_id, BOARD_SIZE))
        .await
        .unwrap_or_default();
    Value::Array(
        entries
            .into_iter()
            .map(|e| json!({"guild_name": e.name, "guild_nameplate": "", "level": e.level, "remaining_sec": e.remaining_sec}))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_run() {
        let body = json!({"gauntlet_id":"ESC_SEASON_1","players":["u1",""],"level":1,"remaining_sec":526,"data":{}});
        assert_eq!(
            FinishedRun::parse(&body),
            Some(FinishedRun {
                season_id: "ESC_SEASON_1".into(),
                players: vec!["u1".into()],
                level: 1,
                remaining_sec: 526
            })
        );
        for bad in [
            json!({"players":["u1"],"level":1}),
            json!({"gauntlet_id":"S","players":[],"level":1}),
            json!({"gauntlet_id":"S","players":["u1"],"level":0}),
            json!({"gauntlet_id":"S","players":["u1"],"level":"1"}),
        ] {
            assert_eq!(FinishedRun::parse(&bad), None, "{bad}");
        }
    }
}
