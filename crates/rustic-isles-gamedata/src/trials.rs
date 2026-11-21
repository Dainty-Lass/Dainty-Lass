// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Trials leaderboards kept in the store: the hunt server's finished runs, and the boards the client asks for. The
//! shapes are the 1.14.7 client's Phoenix leaderboard serializers: a solo line is flat, a group line lists its
//! players under `entries`, a page holds `entries`, and the world's solo boards are keyed `all` and per weapon.
//! Without a store every board is empty.

use std::collections::HashSet;
use std::sync::Arc;

use dainty_module_api::{Actor, ActorContext};
use rustic_isles::{AccountId, Shared, UserId, replay, season};
use rustic_isles_store::repo::trials::{self, TrialBoardEntry, TrialMode, TrialRun};
use rustic_isles_store::{IslesStore, RepoError};
use serde_json::{Map, Value, json};

/// The world's per-weapon solo boards, as the client keys them, and the `weapon` each one holds. The numbers follow
/// the client's `ELeaderboardWeaponType` order; pike's 5 is confirmed by a recorded run (2026-11-20), the rest are not.
pub const WEAPON_BOARDS: [(&str, i64); 7] = [
    ("sword", 1),
    ("axe", 2),
    ("hammer", 3),
    ("chainblades", 4),
    ("pike", 5),
    ("repeaters", 6),
    ("strikers", 7),
];

/// The most lines a page holds.
const MAX_PAGE_SIZE: i64 = 100;

/// Rows in the arena schedule; the Director's featured week cycles through them (hunt.rs).
const ARENA_WEEKS: i64 = 181;

/// `TRIALS_ROTATION_MINUTES` as the Director reads it: a whole number in [1, 525 600], else a week.
fn rotation_minutes(raw: Option<&str>) -> i64 {
    raw.and_then(|r| r.trim().parse::<f64>().ok())
        .filter(|n| n.fract() == 0.0 && (1.0..=525_600.0).contains(n))
        .map_or(10_080, |n| n as i64)
}

/// The featured arena week at `now_ms`, three digits: the week the Director launches (hunt.rs).
pub fn featured_week(now_ms: i64, rotation: Option<&str>) -> String {
    let span = rotation_minutes(rotation).saturating_mul(60_000);
    let week = now_ms.div_euclid(span).rem_euclid(ARENA_WEEKS) + 1;
    format!("{week:03}")
}

/// The key a Trial's runs are kept under. The game names a Trial `Arena_MatchmakerHunt_<tier>_<NNNN>`, NNNN being
/// weeks since its schedule row's start plus one; the runtime moves that start on the hunt server only, so the hunt
/// server sends 0001 while the client asks for another number. Both become the featured hunt,
/// `Arena_MatchmakerHunt_<tier>_<week>`; any other id is kept as sent.
pub fn board_key(trial_id: &str, week: &str) -> String {
    let tier = trial_id
        .strip_prefix("Arena_MatchmakerHunt_")
        .and_then(|rest| rest.split_once('_'))
        .filter(|(tier, n)| !tier.is_empty() && n.len() == 4 && n.bytes().all(|b| b.is_ascii_digit()))
        .map(|(tier, _)| tier);
    match tier {
        Some(tier) => format!("Arena_MatchmakerHunt_{tier}_{week}"),
        None => trial_id.to_owned(),
    }
}

fn shared_key(shared: &Shared, trial_id: &str) -> String {
    let week = featured_week(replay::now_ms(), shared.settings().raw("TRIALS_ROTATION_MINUTES"));
    board_key(trial_id, &week)
}

fn store(shared: &Shared) -> Option<Arc<IslesStore>> {
    shared.slot::<IslesStore>()
}

/// A JSON number or numeric string as an integer (truncated), as the client's fields may be either.
fn int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64)),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .map(|f| f.trunc() as i64),
        _ => None,
    }
}

fn float(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).map(str::trim).unwrap_or("").to_owned()
}

/// One player of a finished run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    /// The userId (`phx_account_id`).
    pub user_id: String,
    /// The weapon played.
    pub weapon: i64,
    /// The role played.
    pub player_role_id: String,
    /// The player's platform.
    pub platform: String,
}

/// A finished Trial as the hunt server reports it (`POST /trials/leaderboards`).
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedTrial {
    /// The Trial.
    pub trial_id: String,
    /// The difficulty (1 when absent).
    pub difficulty: i64,
    /// The run's time in milliseconds (a 2.9 s run sent 2893).
    pub completion_time: f64,
    /// Objectives completed.
    pub objectives_completed: i64,
    /// Who played it.
    pub players: Vec<Player>,
}

fn player(value: &Value, outer: &Value) -> Option<Player> {
    let (user_id, source) = match value {
        Value::String(s) => (s.trim().to_owned(), outer),
        Value::Object(map) => (text(map.get("phx_account_id")), value),
        _ => return None,
    };
    (!user_id.is_empty()).then(|| Player {
        user_id,
        weapon: int(source.get("weapon")).unwrap_or(0),
        player_role_id: text(source.get("player_role_id")),
        platform: text(source.get("platform")),
    })
}

impl FinishedTrial {
    /// The run's `trial_id`, `difficulty`, `completion_time` and `objectives_completed`, and its players: the objects
    /// (or userIds) under `party_members` (the hunt server's), `entries` or `players`, else the body itself as one solo line. None when the Trial, the time
    /// or every player is missing.
    pub fn parse(body: &Value) -> Option<Self> {
        let trial_id = text(body.get("trial_id"));
        let completion_time = float(body.get("completion_time"))?;
        let listed = ["party_members", "entries", "players"]
            .iter()
            .find_map(|k| body.get(*k).and_then(Value::as_array));
        let mut seen = HashSet::new();
        let found: Vec<Player> = match listed {
            Some(items) => items.iter().filter_map(|p| player(p, body)).collect(),
            None => player(body, body).into_iter().collect(),
        };
        let players: Vec<Player> = found.into_iter().filter(|p| seen.insert(p.user_id.clone())).collect();
        (!trial_id.is_empty() && !players.is_empty()).then(|| Self {
            trial_id,
            difficulty: int(body.get("difficulty")).unwrap_or(1),
            completion_time: completion_time.max(0.0),
            objectives_completed: int(body.get("objectives_completed")).unwrap_or(0),
            players,
        })
    }
}

/// What a board request asks for: `trial_id`, `difficulty` (1), `page` (0) and `page_size` (100, at most 100).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardQuery {
    /// The Trial.
    pub trial_id: String,
    /// The difficulty.
    pub difficulty: i64,
    /// The page, from 0.
    pub page: i64,
    /// Lines on a page.
    pub page_size: i64,
}

impl BoardQuery {
    /// From a request body; anything unreadable takes its default.
    pub fn from_body(body: &Map<String, Value>) -> Self {
        Self {
            trial_id: text(body.get("trial_id")),
            difficulty: int(body.get("difficulty")).unwrap_or(1),
            page: int(body.get("page")).unwrap_or(0).max(0),
            page_size: int(body.get("page_size"))
                .unwrap_or(MAX_PAGE_SIZE)
                .clamp(1, MAX_PAGE_SIZE),
        }
    }

    fn window<T>(&self, lines: Vec<T>) -> Vec<(usize, T)> {
        let skip = usize::try_from(self.page.saturating_mul(self.page_size)).unwrap_or(usize::MAX);
        let take = usize::try_from(self.page_size).unwrap_or(0);
        lines.into_iter().enumerate().skip(skip).take(take).collect()
    }
}

/// A solo board line.
pub fn solo_line(entry: &TrialBoardEntry, rank: usize) -> Value {
    json!({
        "phx_account_id": entry.user_id,
        "platform_name": entry.name,
        "rank": rank,
        "completion_time": entry.run.completion_time,
        "weapon": entry.run.weapon,
        "player_role_id": entry.run.player_role_id,
        "objectives_completed": entry.run.objectives_completed,
        "platform": entry.run.platform,
        "trial_id": entry.run.trial_id,
    })
}

/// A group board line: the run and its players under `entries`.
pub fn group_line(entry: &TrialBoardEntry, rank: usize) -> Value {
    let players = serde_json::from_str::<Value>(&entry.run.party)
        .ok()
        .filter(Value::is_array)
        .unwrap_or_else(|| json!([]));
    json!({
        "rank": rank,
        "completion_time": entry.run.completion_time,
        "entries": players,
        "objectives_completed": entry.run.objectives_completed,
        "trial_id": entry.run.trial_id,
    })
}

/// Each player's fastest line once (the solo `all` board, and the per-weapon boards).
pub fn per_player(runs: Vec<TrialBoardEntry>) -> Vec<TrialBoardEntry> {
    let mut seen = HashSet::new();
    runs.into_iter().filter(|e| seen.insert(e.user_id.clone())).collect()
}

/// Each group's fastest line once: its players' rows share a `party_key`.
pub fn per_group(runs: Vec<TrialBoardEntry>) -> Vec<TrialBoardEntry> {
    let mut seen = HashSet::new();
    runs.into_iter()
        .filter(|e| seen.insert(e.run.party_key.clone()))
        .collect()
}

/// The ranked lines of one board, before paging.
pub struct Boards {
    /// Solo, one line per player.
    pub solo: Vec<TrialBoardEntry>,
    /// Solo, each player's fastest run with each weapon.
    pub by_weapon: Vec<TrialBoardEntry>,
    /// Group, one line per group.
    pub group: Vec<TrialBoardEntry>,
}

impl Boards {
    /// The page's lines of the solo board, every weapon or one.
    pub fn solo_page(&self, query: &BoardQuery, weapon: Option<i64>) -> Vec<Value> {
        let lines: Vec<&TrialBoardEntry> = match weapon {
            None => self.solo.iter().collect(),
            Some(w) => self.by_weapon.iter().filter(|e| e.run.weapon == w).collect(),
        };
        query
            .window(lines)
            .into_iter()
            .map(|(i, e)| solo_line(e, i + 1))
            .collect()
    }

    /// The page's lines of the group board.
    pub fn group_page(&self, query: &BoardQuery) -> Vec<Value> {
        query
            .window(self.group.iter().collect())
            .into_iter()
            .map(|(i, e)| group_line(e, i + 1))
            .collect()
    }

    /// The player's solo line with their rank, any weapon.
    pub fn solo_of(&self, user: &str) -> Option<Value> {
        let i = self.solo.iter().position(|e| e.user_id == user)?;
        Some(solo_line(&self.solo[i], i + 1))
    }

    /// The fastest group line the player is in, with its rank.
    pub fn group_of(&self, user: &str) -> Option<Value> {
        let i = self
            .group
            .iter()
            .position(|e| e.run.party_key.split(',').any(|p| p == user))?;
        Some(group_line(&self.group[i], i + 1))
    }
}

/// The Trial's boards at the difficulty: the solo one has each player's fastest run with each weapon, fastest first.
/// Lines name the Trial as asked, whatever key its runs are kept under.
pub async fn boards(shared: &Shared, trial_id: &str, difficulty: i64) -> Boards {
    let empty = Boards {
        solo: Vec::new(),
        by_weapon: Vec::new(),
        group: Vec::new(),
    };
    let Some(store) = store(shared) else {
        return empty;
    };
    if trial_id.is_empty() {
        return empty;
    }
    let trial = shared_key(shared, trial_id);
    let read = store
        .read(move |conn, _| {
            Ok((
                trials::runs(conn, &trial, difficulty, TrialMode::Solo)?,
                trials::runs(conn, &trial, difficulty, TrialMode::Group)?,
            ))
        })
        .await;
    match read {
        Ok((mut solo, mut group)) => {
            for e in solo.iter_mut().chain(group.iter_mut()) {
                trial_id.clone_into(&mut e.run.trial_id);
            }
            Boards {
                solo: per_player(solo.clone()),
                by_weapon: solo,
                group: per_group(group),
            }
        }
        Err(e) => {
            tracing::warn!("[Trials] {trial_id}: the boards could not be read: {e}");
            empty
        }
    }
}

async fn account_of(store: &IslesStore, user: &str) -> Option<(AccountId, String)> {
    let key = UserId::new(user);
    store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &key))
        .await
        .ok()
        .flatten()
        .map(|record| (record.id, record.name))
}

/// Records the run for each player this node is home to: solo with the weapon played, or a group run with its players.
pub async fn record(shared: &Shared, run: &FinishedTrial) {
    let Some(store) = store(shared) else {
        tracing::warn!("[Trials] {} finished but there is no store to record it", run.trial_id);
        return;
    };
    let key = shared_key(shared, &run.trial_id);
    let mut known = Vec::new();
    let mut party = Vec::new();
    for p in &run.players {
        let account = account_of(&store, &p.user_id).await;
        let name = account.as_ref().map_or_else(|| p.user_id.clone(), |(_, n)| n.clone());
        party.push(json!({
            "phx_account_id": p.user_id,
            "platform_name": name,
            "weapon": p.weapon,
            "player_role_id": p.player_role_id,
            "platform": p.platform,
        }));
        match account {
            Some((id, _)) => known.push((p, id)),
            None => tracing::warn!("[Trials] finished run for unknown user {}", p.user_id),
        }
    }
    let mode = if run.players.len() > 1 {
        TrialMode::Group
    } else {
        TrialMode::Solo
    };
    let mut keys: Vec<&str> = run.players.iter().map(|p| p.user_id.as_str()).collect();
    keys.sort_unstable();
    let party_key = keys.join(",");
    let party = dainty_service_kit::jsjson::stringify(&Value::Array(party));
    let now = season::to_iso_string(replay::now_ms());
    for (p, account) in known {
        let fresh = TrialRun {
            account: account.clone(),
            trial_id: key.clone(),
            difficulty: run.difficulty,
            mode,
            weapon: if mode == TrialMode::Solo { p.weapon } else { 0 },
            completion_time: run.completion_time,
            objectives_completed: run.objectives_completed,
            player_role_id: p.player_role_id.clone(),
            platform: p.platform.clone(),
            party_key: party_key.clone(),
            party: party.clone(),
            runs: 1,
        };
        let at = now.clone();
        let written = store
            .account_write(&account, &ActorContext::new(Actor::System), move |tx, _| {
                let best = match trials::find(
                    tx.conn(),
                    &fresh.account,
                    &fresh.trial_id,
                    fresh.difficulty,
                    fresh.mode,
                    fresh.weapon,
                )? {
                    Some(mut best) => {
                        best.record(&fresh);
                        best
                    }
                    None => fresh,
                };
                trials::save(tx, &best, &at)?;
                Ok(best)
            })
            .await;
        match written {
            Ok(best) => tracing::info!(
                "[Trials] {} finished {} as {} (difficulty {}, {:?}, weapon {}) in {}; best {}, {} runs",
                p.user_id,
                run.trial_id,
                key,
                run.difficulty,
                mode,
                p.weapon,
                run.completion_time,
                best.completion_time,
                best.runs
            ),
            Err(RepoError::NotHome { .. }) => {}
            Err(e) => tracing::warn!("[Trials] {}: the run was not recorded: {e}", p.user_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(user: &str, weapon: i64, time: f64, party_key: &str) -> TrialBoardEntry {
        TrialBoardEntry {
            user_id: user.into(),
            name: format!("{user}-name"),
            run: TrialRun {
                account: AccountId::new(&dainty_module_api::NodeId::new("n1").unwrap()),
                trial_id: "T".into(),
                difficulty: 1,
                mode: TrialMode::Solo,
                weapon,
                completion_time: time,
                objectives_completed: 2,
                player_role_id: "r".into(),
                platform: "WIN".into(),
                party_key: party_key.into(),
                party: r#"[{"phx_account_id":"a"},{"phx_account_id":"b"}]"#.into(),
                runs: 1,
            },
        }
    }

    #[test]
    fn the_hunt_servers_body() {
        // Recorded from a 1.14.7 hunt server, 2026-11-20
        let body = json!({"difficulty":1,"completion_time":5414,"objectives_completed":3,
            "trial_id":"Arena_MatchmakerHunt_Elite_0001","party_members":[{"phx_account_id":"u1",
            "platform_name":"","weapon":5,"player_role_id":"PR_FRANK","platform":""}]});
        let run = FinishedTrial::parse(&body).unwrap();
        assert_eq!((run.completion_time, run.objectives_completed), (5414.0, 3));
        assert_eq!(
            run.players,
            [Player {
                user_id: "u1".into(),
                weapon: 5,
                player_role_id: "PR_FRANK".into(),
                platform: String::new(),
            }]
        );
    }

    #[test]
    fn a_trial_is_kept_under_the_featured_hunt() {
        let week = featured_week(0, None);
        assert_eq!(week, "001");
        assert_eq!(featured_week(7 * 86_400_000, None), "002");
        assert_eq!(featured_week(181 * 7 * 86_400_000, None), "001");
        assert_eq!(featured_week(60_000, Some("1")), "002");
        assert_eq!(featured_week(60_000, Some("0")), "001");
        assert_eq!(
            board_key("Arena_MatchmakerHunt_Elite_0001", "073"),
            "Arena_MatchmakerHunt_Elite_073"
        );
        assert_eq!(
            board_key("Arena_MatchmakerHunt_Elite_0312", "073"),
            "Arena_MatchmakerHunt_Elite_073"
        );
        for kept in ["Arena_MatchmakerHunt_Elite_073", "Arena_MatchmakerHunt__0001", "T", ""] {
            assert_eq!(board_key(kept, "073"), kept);
        }
    }

    #[test]
    fn a_finished_trial() {
        let group = json!({"trial_id":"T","difficulty":2,"completion_time":312.5,"objectives_completed":3,
            "entries":[{"phx_account_id":"u1","weapon":4,"player_role_id":"x","platform":"WIN","session_id":"s"},
                       {"phx_account_id":"u2"},{"phx_account_id":"u1"}]});
        let run = FinishedTrial::parse(&group).unwrap();
        assert_eq!(
            (run.difficulty, run.completion_time, run.objectives_completed),
            (2, 312.5, 3)
        );
        assert_eq!(run.players.len(), 2);
        assert_eq!((run.players[0].weapon, run.players[0].platform.as_str()), (4, "WIN"));
        let solo = json!({"trial_id":"T","completion_time":"90","phx_account_id":"u1","weapon":2});
        let run = FinishedTrial::parse(&solo).unwrap();
        assert_eq!(
            (run.difficulty, run.players[0].weapon, run.completion_time),
            (1, 2, 90.0)
        );
        let ids = json!({"trial_id":"T","completion_time":1,"players":["u1","u2"],"weapon":3});
        assert_eq!(FinishedTrial::parse(&ids).unwrap().players[1].weapon, 3);
        for bad in [
            json!({"completion_time":1,"phx_account_id":"u1"}),
            json!({"trial_id":"T","phx_account_id":"u1"}),
            json!({"trial_id":"T","completion_time":1,"entries":[]}),
        ] {
            assert_eq!(FinishedTrial::parse(&bad), None, "{bad}");
        }
    }

    #[test]
    fn queries_take_their_defaults() {
        let q = BoardQuery::from_body(&Map::new());
        assert_eq!(
            (q.trial_id.as_str(), q.difficulty, q.page, q.page_size),
            ("", 1, 0, 100)
        );
        let body = json!({"trial_id":"T","difficulty":"3","page":1.0,"page_size":500});
        let q = BoardQuery::from_body(body.as_object().unwrap());
        assert_eq!((q.difficulty, q.page, q.page_size), (3, 1, 100));
    }

    #[test]
    fn boards_rank_page_and_find_a_player() {
        let runs = vec![
            entry("a", 1, 10.0, "a"),
            entry("b", 3, 20.0, "b"),
            entry("a", 3, 30.0, "a"),
        ];
        let boards = Boards {
            solo: per_player(runs.clone()),
            by_weapon: runs,
            group: per_group(vec![
                entry("a", 0, 5.0, "a,b"),
                entry("b", 0, 5.0, "a,b"),
                entry("c", 0, 6.0, "c,d"),
            ]),
        };
        assert_eq!(boards.solo.len(), 2);
        let q = BoardQuery::from_body(json!({"trial_id":"T","page_size":1,"page":1}).as_object().unwrap());
        let page = boards.solo_page(&q, None);
        assert_eq!(
            (page.len(), &page[0]["rank"], &page[0]["phx_account_id"]),
            (1, &json!(2), &json!("b"))
        );
        let all = BoardQuery::from_body(&Map::new());
        let hammer = boards.solo_page(&all, Some(3));
        assert_eq!(hammer.len(), 2);
        assert_eq!(
            (&hammer[0]["phx_account_id"], &hammer[0]["rank"]),
            (&json!("b"), &json!(1))
        );
        assert_eq!(boards.group_page(&all).len(), 2);
        assert_eq!(
            boards.group_of("b").unwrap()["entries"][1]["phx_account_id"],
            json!("b")
        );
        assert_eq!(boards.group_of("d").unwrap()["rank"], json!(2));
        assert_eq!(boards.solo_of("b").unwrap()["platform_name"], json!("b-name"));
        assert!(boards.solo_of("z").is_none());
        let line = solo_line(&boards.solo[0], 1);
        let keys: Vec<&str> = line.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "phx_account_id",
                "platform_name",
                "rank",
                "completion_time",
                "weapon",
                "player_role_id",
                "objectives_completed",
                "platform",
                "trial_id"
            ]
        );
    }
}
