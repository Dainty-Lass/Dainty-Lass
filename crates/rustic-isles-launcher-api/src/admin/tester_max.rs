// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Tester max (admin spec §7): an operator funds a tester account from the host's kit file, idempotent per run id.

use std::sync::Arc;

use axum::response::Response;
use dainty_module_api::{Actor, ActorContext, BoxFuture};
use dainty_service_kit::js::safe_integer;
use http::StatusCode;
use rustic_isles::Shared;
use rustic_isles_store::repo::auth::LauncherAccountRecord;
use rustic_isles_store::repo::progression::ProgressionGrantRecord;
use rustic_isles_store::repo::reforged::AdminAuditRecord;
use rustic_isles_store::{AccountId, IslesStore, RepoError};
use serde_json::Value;

use super::{AdminContext, LauncherApiAdmin, admin_actor, admin_error, fresh_id, player_not_found};
use crate::util::{self, iso, json, now_ms, object};

/// The tester max kit's file in the backend game data folder.
pub const KIT_FILE: &str = "tester_max_kit.json";
const MAX_GRANT: i64 = 100_000;

/// What a confirm asks of a track's curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RankRequirement {
    /// The XP the rank needs.
    Xp(f64),
    /// The track is not known.
    UnknownTrack,
    /// The rank is past the track's curve.
    OutOfRange,
}

/// How the operator's test stacks went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackOutcome {
    /// The stacks were granted.
    Applied,
    /// The character is not the player's.
    NotOwned,
    /// The transaction failed, with the reason.
    Failed(String),
}

/// What tester max takes from the economy (§15.3).
pub trait TesterFunding: Send + Sync {
    /// Whether tester max may grant XP on this track.
    fn is_approved_xp_track(&self, shared: &Shared, id: &str) -> bool;
    /// The track's XP cap, none when uncapped or unknown.
    fn track_cap_xp(&self, shared: &Shared, id: &str) -> Option<i64>;
    /// The rank `progress` XP reaches on the track.
    fn rank_for_progress(&self, shared: &Shared, id: &str, progress: i64) -> Option<i64>;
    /// The XP the rank needs on the track.
    fn required_xp_for_rank(&self, shared: &Shared, id: &str, rank: i64) -> RankRequirement;
    /// Whether the catalog has this item id.
    fn is_known_catalog_id(&self, shared: &Shared, id: &str) -> bool;
    /// Most XP one run grants.
    fn max_xp_grant(&self) -> i64 {
        MAX_GRANT
    }
    /// One inventory transaction (idempotent per transaction id through the ledger).
    fn apply_operator_test_stacks<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        user_id: &'a str,
        character_id: &'a str,
        transaction_id: &'a str,
        stacks: &'a [(String, i64)],
    ) -> BoxFuture<'a, StackOutcome>;
}

/// The economy crate's tracks, storage policy and inventory transaction.
#[derive(Debug, Default, Clone, Copy)]
pub struct EconomyFunding;

impl TesterFunding for EconomyFunding {
    fn is_approved_xp_track(&self, shared: &Shared, id: &str) -> bool {
        rustic_isles_economy::progression::tracks::is_approved_xp_track(shared, id)
    }

    fn track_cap_xp(&self, shared: &Shared, id: &str) -> Option<i64> {
        rustic_isles_economy::progression::tracks::track_cap_xp(shared, id).map(|xp| {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a cumulative XP total from the host's curve"
            )]
            let whole = xp as i64;
            whole
        })
    }

    fn rank_for_progress(&self, shared: &Shared, id: &str, progress: i64) -> Option<i64> {
        #[allow(clippy::cast_precision_loss, reason = "track progress is far below 2^53")]
        let progress = progress as f64;
        rustic_isles_economy::progression::curves(shared).rank_for_progress(id, progress)
    }

    fn required_xp_for_rank(&self, shared: &Shared, id: &str, rank: i64) -> RankRequirement {
        let curves = rustic_isles_economy::progression::curves(shared);
        match curves.get(id) {
            None => RankRequirement::UnknownTrack,
            Some(curve) => curve
                .required_xp_for_rank(rank)
                .map_or(RankRequirement::OutOfRange, RankRequirement::Xp),
        }
    }

    fn is_known_catalog_id(&self, shared: &Shared, id: &str) -> bool {
        rustic_isles_economy::inventory::storage_policy::StoragePolicy::load(shared)
            .is_ok_and(|policy| policy.entry(id).is_some())
    }

    fn apply_operator_test_stacks<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        user_id: &'a str,
        character_id: &'a str,
        transaction_id: &'a str,
        stacks: &'a [(String, i64)],
    ) -> BoxFuture<'a, StackOutcome> {
        use rustic_isles_economy::inventory::{
            Economy, InventoryPorts, TxContext, TxOptions, TxOutcome, TxRequest, run_inventory_transaction,
        };
        Box::pin(async move {
            let Some(store) = util::store_of(shared) else {
                return StackOutcome::Failed("the module has no database".into());
            };
            let ports = InventoryPorts::default();
            let add: Vec<Value> = stacks
                .iter()
                .map(|(id, q)| {
                    object(vec![
                        ("catalogId", Value::from(id.clone())),
                        ("quantity", Value::from(*q)),
                    ])
                })
                .collect();
            let request = TxRequest {
                user_id: user_id.to_owned(),
                character_id: character_id.to_owned(),
                transaction_id: transaction_id.to_owned(),
                add_instanced: Value::Array(Vec::new()),
                add_stacked: Value::Array(add),
                remove_instanced: Value::Array(Vec::new()),
                remove_stacked: Value::Array(Vec::new()),
                save_instanced: Value::Array(Vec::new()),
            };
            let options = TxOptions {
                ownership_already_validated: true,
                diagnostic_label: Some("tester-max".into()),
                operator_test_funding: true,
                economy: Some(Economy {
                    source: "OPERATOR_TEST".into(),
                    p2p_session_id: None,
                    store_sku_id: None,
                }),
                ..TxOptions::default()
            };
            match run_inventory_transaction(TxContext::new(shared, &store, &ports), request, options, None).await {
                Ok(TxOutcome::Done(_)) => StackOutcome::Applied,
                Ok(TxOutcome::NotOwned) => StackOutcome::NotOwned,
                Err(e) => StackOutcome::Failed(e.to_string()),
            }
        })
    }
}

// ------------------------------------------------------------------ the kit (§7.2)

/// 8 to 64 characters: a letter or digit, then letters, digits, `.`, `_`, `-`.
pub fn is_run_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    (8..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn js_string(value: Option<&Value>) -> String {
    dainty_service_kit::js::string(value)
}

/// The kit's problems, in order; none is a kit that can run.
pub fn validate_kit(kit: &Value, shared: &Shared, funding: &dyn TesterFunding) -> Vec<String> {
    let Some(map) = kit.as_object() else {
        return vec!["the kit must be a JSON object".to_owned()];
    };
    let mut problems = Vec::new();
    for key in map.keys() {
        if key == "slayersPath" {
            problems.push("slayersPath was removed (16 Oct): a tester unlocks the Slayer's Path in game".to_owned());
        } else if !matches!(key.as_str(), "comment" | "progression" | "inventory" | "currencies") {
            problems.push(format!("unknown section \"{key}\""));
        }
    }
    if map.get("comment").is_some_and(|c| !c.is_null() && !c.is_string()) {
        problems.push("comment must be a string".to_owned());
    }
    let section = |name: &str, max: usize, problems: &mut Vec<String>| -> Option<Vec<Value>> {
        match map.get(name) {
            None | Some(Value::Null) => None,
            Some(Value::Array(list)) if list.len() <= max => Some(list.clone()),
            Some(_) => {
                problems.push(format!("{name} must be an array of at most {max} entries"));
                None
            }
        }
    };
    if let Some(list) = section("progression", 50, &mut problems) {
        let mut seen: Vec<String> = Vec::new();
        for (i, entry) in list.iter().enumerate() {
            let at = format!("progression[{i}]");
            let id_value = entry.get("progressionId");
            let id = id_value.and_then(Value::as_str);
            match id {
                Some(id) if funding.is_approved_xp_track(shared, id) => {
                    if seen.iter().any(|s| s == id) {
                        problems.push(format!("{at}: {id} is listed twice"));
                    }
                    seen.push(id.to_owned());
                }
                _ => problems.push(format!(
                    "{at}: \"{}\" is not an approved XP track (progression config)",
                    js_string(id_value)
                )),
            }
            match entry.get("targetProgress") {
                Some(Value::String(s)) if s == "max" => {
                    if let Some(id) = id
                        && funding.is_approved_xp_track(shared, id)
                        && funding.track_cap_xp(shared, id).is_none()
                    {
                        problems.push(format!(
                            "{at}: {id} has no top rank, so \"max\" is not defined for it; give a number"
                        ));
                    }
                }
                value if safe_integer(value).is_some_and(|n| (1..=10_000_000).contains(&n)) => {}
                _ => problems.push(format!(
                    "{at}: targetProgress must be \"max\" or an integer from 1 to 10000000"
                )),
            }
        }
    }
    if let Some(list) = section("inventory", 200, &mut problems) {
        for (i, entry) in list.iter().enumerate() {
            let at = format!("inventory[{i}]");
            let id_value = entry.get("catalogId");
            match id_value.and_then(Value::as_str) {
                Some(id) if funding.is_known_catalog_id(shared, id) => {
                    if id.starts_with("CURRENCY_") {
                        problems.push(format!("{at}: {id} is a currency; list it under currencies"));
                    }
                }
                _ => problems.push(format!(
                    "{at}: \"{}\" is not in the inventory storage policy (do not invent catalog ids)",
                    js_string(id_value)
                )),
            }
            if !safe_integer(entry.get("quantity")).is_some_and(|n| (1..=1000).contains(&n)) {
                problems.push(format!("{at}: quantity must be an integer from 1 to 1000"));
            }
        }
    }
    if let Some(list) = section("currencies", 50, &mut problems) {
        for (i, entry) in list.iter().enumerate() {
            let at = format!("currencies[{i}]");
            let id_value = entry.get("catalogId");
            match id_value.and_then(Value::as_str) {
                Some(id) if id == rustic_isles_economy::platinum::PLATINUM_AGGREGATE_ID => problems.push(format!(
                    "{at}: {id} is the Platinum aggregate; grant a storefront bucket (CURRENCY_PLATINUM_*) instead"
                )),
                Some(id) if id.starts_with("CURRENCY_") && funding.is_known_catalog_id(shared, id) => {}
                _ => problems.push(format!(
                    "{at}: \"{}\" is not a known CURRENCY_* id",
                    js_string(id_value)
                )),
            }
            if !safe_integer(entry.get("quantity")).is_some_and(|n| (1..=10_000_000).contains(&n)) {
                problems.push(format!("{at}: quantity must be an integer from 1 to 10000000"));
            }
        }
    }
    problems
}

// ------------------------------------------------------------------ the run (§7.3)

/// Blocks aligned to the grant ceiling from max(start, 0) up to target: `(index, amount)`.
pub fn plan_blocks(start: i64, target: i64, ceiling: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut s = start.max(0);
    while s < target && ceiling > 0 {
        let index = s / ceiling;
        let end = target.min((index + 1).saturating_mul(ceiling));
        out.push((index, end - s));
        s = end;
    }
    out
}

/// `tmax-` and the first 40 hex digits of SHA-256 of `<runId>` NUL `<part>`.
pub fn derived_id(run_id: &str, part: &str) -> String {
    let mut bytes = run_id.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend_from_slice(part.as_bytes());
    format!("tmax-{}", &util::sha256_hex(&bytes)[..40])
}

#[derive(Debug, Clone)]
enum Target {
    Max,
    Number(i64),
}

#[derive(Debug, Clone, Default)]
struct Row {
    progression_id: String,
    target: i64,
    start: i64,
    granted: i64,
    grants: i64,
    confirmed: Option<i64>,
    error: Option<String>,
}

struct Report {
    run_id: String,
    user_id: String,
    character_id: String,
    partial: bool,
    rows: Vec<Row>,
    items: Option<(bool, Option<String>, usize)>,
}

impl Report {
    fn status(&self) -> &'static str {
        if self.partial { "partial" } else { "done" }
    }

    fn row_status(row: &Row) -> &'static str {
        if row.error.is_some() { "error" } else { "done" }
    }

    fn to_json(&self) -> Value {
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|r| {
                let mut pairs = vec![
                    ("progressionId", Value::from(r.progression_id.clone())),
                    ("target", Value::from(r.target)),
                    ("startProgress", Value::from(r.start)),
                    ("granted", Value::from(r.granted)),
                    ("grants", Value::from(r.grants)),
                ];
                if let Some(rank) = r.confirmed {
                    pairs.push(("confirmedRank", Value::from(rank)));
                }
                pairs.push(("status", Value::from(Self::row_status(r))));
                if let Some(error) = &r.error {
                    pairs.push(("error", Value::from(error.clone())));
                }
                object(pairs)
            })
            .collect();
        let mut pairs = vec![
            ("runId", Value::from(self.run_id.clone())),
            ("userId", Value::from(self.user_id.clone())),
            ("characterId", Value::from(self.character_id.clone())),
            ("status", Value::from(self.status())),
            ("progression", Value::Array(rows)),
        ];
        if let Some((ok, error, stacks)) = &self.items {
            let mut items = vec![("status", Value::from(if *ok { "done" } else { "error" }))];
            if let Some(error) = error {
                items.push(("error", Value::from(error.clone())));
            }
            items.push(("stacks", Value::from(*stacks)));
            pairs.push(("items", object(items)));
        }
        object(pairs)
    }

    fn summary(&self) -> Value {
        let rows: Vec<Value> = self
            .rows
            .iter()
            .map(|r| {
                object(vec![
                    ("progressionId", Value::from(r.progression_id.clone())),
                    ("target", Value::from(r.target)),
                    ("startProgress", Value::from(r.start)),
                    ("granted", Value::from(r.granted)),
                    ("grants", Value::from(r.grants)),
                    ("confirmedRank", r.confirmed.map_or(Value::Null, Value::from)),
                    ("status", Value::from(Self::row_status(r))),
                ])
            })
            .collect();
        let mut pairs = vec![
            ("runId", Value::from(self.run_id.clone())),
            ("status", Value::from(self.status())),
            ("progression", Value::Array(rows)),
        ];
        if let Some((ok, _, stacks)) = &self.items {
            pairs.push((
                "items",
                object(vec![
                    ("status", Value::from(if *ok { "done" } else { "error" })),
                    ("stacks", Value::from(*stacks)),
                ]),
            ));
        }
        object(pairs)
    }
}

type KitEntries = (Vec<(String, Target)>, Vec<(String, i64)>);

fn parse_entries(kit: &Value) -> KitEntries {
    let list = |name: &str| kit.get(name).and_then(Value::as_array).cloned().unwrap_or_default();
    let progression = list("progression")
        .iter()
        .filter_map(|e| {
            let id = e.get("progressionId")?.as_str()?.to_owned();
            let target = match e.get("targetProgress") {
                Some(Value::String(s)) if s == "max" => Target::Max,
                value => Target::Number(safe_integer(value)?),
            };
            Some((id, target))
        })
        .collect();
    let mut stacks: Vec<(String, i64)> = Vec::new();
    for entry in list("inventory").iter().chain(list("currencies").iter()) {
        let (Some(id), Some(q)) = (
            entry.get("catalogId").and_then(Value::as_str),
            safe_integer(entry.get("quantity")),
        ) else {
            continue;
        };
        match stacks.iter_mut().find(|(c, _)| c == id) {
            Some((_, total)) => *total += q,
            None => stacks.push((id.to_owned(), q)),
        }
    }
    stacks.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    (progression, stacks)
}

fn cut(text: &str) -> String {
    util::take_chars(text, 200)
}

#[derive(Debug)]
enum StepError {
    Plain(String),
    Repo(RepoError),
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plain(text) => f.write_str(text),
            Self::Repo(e) => write!(f, "{e}"),
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "the run's context and the one track it drives")]
async fn run_track(
    shared: &Arc<Shared>,
    store: &IslesStore,
    funding: &dyn TesterFunding,
    account: &AccountId,
    user_id: &str,
    run_id: &str,
    id: &str,
    target: &Target,
    row: &mut Row,
) -> Result<(), StepError> {
    let (a, track) = (account.clone(), id.to_owned());
    let start = store
        .read(move |conn, repos| repos.progression_tracks.get(conn, &a, &track))
        .await
        .map_err(StepError::Repo)?
        .map_or(0, |t| t.progress);
    row.start = start;
    let goal = match target {
        Target::Max => funding
            .track_cap_xp(shared, id)
            .ok_or_else(|| StepError::Plain("the track has no top rank".into()))?,
        Target::Number(n) => *n,
    };
    row.target = goal;
    let actor = ActorContext::new(Actor::Player {
        account: account.global().clone(),
    });
    for (index, amount) in plan_blocks(start, goal, funding.max_xp_grant()) {
        if !funding.is_approved_xp_track(shared, id) {
            return Err(StepError::Plain(format!("Unsupported progression track '{id}'.")));
        }
        let grant_id = derived_id(run_id, &format!("xp:{id}:{index}"));
        #[allow(clippy::cast_precision_loss, reason = "an XP amount, far below 2^53")]
        let hash = rustic_isles_economy::progression::tracks::grant_request_hash(user_id, id, amount as f64);
        let (a, track) = (account.clone(), id.to_owned());
        store
            .account_write(account, &actor, move |tx, repos| {
                if let Some(existing) = repos.progression_grants.find_by_grant_id(tx.conn(), &grant_id)? {
                    let same = existing.account == a
                        && existing.progression_id == track
                        && existing.amount == amount
                        && existing.request_hash == hash;
                    if !same {
                        return Err(RepoError::Data(format!(
                            "Progression grant {grant_id} was already used with a different request."
                        )));
                    }
                    return Ok(());
                }
                let result = repos.progression_tracks.increment(tx, &a, &track, amount)?;
                repos.progression_grants.insert_applied(
                    tx,
                    &ProgressionGrantRecord {
                        grant_id,
                        account: a.clone(),
                        progression_id: track,
                        amount,
                        request_hash: hash,
                        result,
                        created_at: iso(now_ms()),
                    },
                )
            })
            .await
            .map_err(StepError::Repo)?;
        row.granted += amount;
        row.grants += 1;
    }
    let rank = funding.rank_for_progress(shared, id, start.max(goal)).unwrap_or(0);
    if rank > 0 {
        let required = match funding.required_xp_for_rank(shared, id, rank) {
            RankRequirement::UnknownTrack => {
                return Err(StepError::Plain(format!("rank {rank} confirm refused: unknown_track")));
            }
            RankRequirement::OutOfRange => {
                return Err(StepError::Plain(format!(
                    "rank {rank} confirm refused: rank_out_of_range"
                )));
            }
            RankRequirement::Xp(xp) => xp,
        };
        let (a, track) = (account.clone(), id.to_owned());
        store
            .account_write(account, &actor, move |tx, repos| {
                let progress = repos
                    .progression_tracks
                    .get(tx.conn(), &a, &track)?
                    .map_or(0, |t| t.progress);
                #[allow(clippy::cast_precision_loss, reason = "track progress is far below 2^53")]
                if required > 0.0 && (progress as f64) < required {
                    return Err(RepoError::Data(format!("rank {rank} confirm refused: insufficient_xp")));
                }
                repos
                    .progression_tracks
                    .set_confirmed_fremium_rank(tx, &a, &track, rank)?;
                Ok(())
            })
            .await
            .map_err(StepError::Repo)?;
        row.confirmed = Some(rank);
    }
    Ok(())
}

const NO_CHARACTER: (&str, &str) = (
    "ADMIN_TESTER_MAX_NO_CHARACTER",
    "The tester has no character yet; the account must sign in to the game once first.",
);

async fn run(
    shared: &Arc<Shared>,
    store: &IslesStore,
    funding: &dyn TesterFunding,
    target: &LauncherAccountRecord,
    user_id: &str,
    run_id: &str,
    kit: &Value,
) -> Result<Report, (&'static str, &'static str)> {
    let account = target.account.clone();
    let a = account.clone();
    let characters = store
        .read(move |conn, repos| repos.characters.find_many_by_account(conn, &a))
        .await
        .unwrap_or_default();
    let Some(character) = characters
        .into_iter()
        .map(|c| c.character_id)
        .find(|id| !util::trim(id).is_empty())
    else {
        return Err(NO_CHARACTER);
    };
    let (progression, stacks) = parse_entries(kit);
    let mut report = Report {
        run_id: run_id.to_owned(),
        user_id: user_id.to_owned(),
        character_id: character.clone(),
        partial: false,
        rows: Vec::new(),
        items: None,
    };
    for (id, goal) in progression {
        let mut row = Row {
            progression_id: id.clone(),
            target: if let Target::Number(n) = goal { n } else { 0 },
            ..Row::default()
        };
        if let Err(e) = run_track(shared, store, funding, &account, user_id, run_id, &id, &goal, &mut row).await {
            row.error = Some(cut(&e.to_string()));
            report.partial = true;
        }
        report.rows.push(row);
    }
    if !stacks.is_empty() {
        let transaction = derived_id(run_id, "items");
        let outcome = funding
            .apply_operator_test_stacks(shared, user_id, &character, &transaction, &stacks)
            .await;
        report.items = Some(match outcome {
            StackOutcome::Applied => (true, None, stacks.len()),
            StackOutcome::NotOwned => {
                report.partial = true;
                (
                    false,
                    Some("the inventory transaction did not apply".into()),
                    stacks.len(),
                )
            }
            StackOutcome::Failed(text) => {
                report.partial = true;
                (false, Some(cut(&text)), stacks.len())
            }
        });
    }
    Ok(report)
}

fn kit_invalid(problems: Vec<String>) -> Response {
    json(
        StatusCode::UNPROCESSABLE_ENTITY,
        &object(vec![(
            "error",
            object(vec![
                ("code", Value::from("ADMIN_TESTER_MAX_KIT_INVALID")),
                ("message", Value::from("The tester kit cannot run.")),
                (
                    "problems",
                    Value::Array(problems.into_iter().map(Value::from).collect()),
                ),
            ]),
        )]),
    )
}

/// Releases the busy mark whatever happens.
struct Busy<'a> {
    admin: &'a LauncherApiAdmin,
    user: String,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.admin.end_run(&self.user);
    }
}

/// `POST /admin/v1/players/:userId/tester-max`: grants a tester's account the kit, ranks and XP.
pub async fn tester_max(ctx: AdminContext) -> Response {
    let user_id = ctx.param("userId").to_owned();
    let lookup = user_id.clone();
    let target = match ctx
        .store
        .read(move |conn, repos| super::find_target(conn, repos, &lookup))
        .await
    {
        Ok(Some(target)) => target,
        Ok(None) => return player_not_found(),
        Err(e) => return super::admin_internal(e),
    };
    if !target.roles.iter().any(|r| r == "tester") {
        return admin_error(
            StatusCode::CONFLICT,
            "ADMIN_TESTER_REQUIRED",
            "Only an account with the tester role can be maxed.",
        );
    }
    let confirm = format!("MAX {}", target.display_name);
    if ctx.request.field("confirm").and_then(Value::as_str) != Some(confirm.as_str()) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_TESTER_MAX_CONFIRMATION",
            &format!("Type \"MAX {}\" to confirm.", target.display_name),
        );
    }
    let Some(run_id) = ctx
        .request
        .field("runId")
        .and_then(Value::as_str)
        .filter(|r| is_run_id(r))
        .map(str::to_owned)
    else {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_TESTER_MAX_RUN_ID",
            "runId must be 8 to 64 characters: letters, digits, dot, underscore or hyphen.",
        );
    };
    let missing = || {
        admin_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "ADMIN_TESTER_MAX_KIT_MISSING",
            "Copy game-data/tester_max_kit.example.json to tester_max_kit.json and fill it in.",
        )
    };
    let Some(path) = ctx.shared.game_data().backend_file(KIT_FILE) else {
        return missing();
    };
    if !std::fs::metadata(&path).is_ok_and(|m| m.is_file()) {
        return missing();
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return missing();
    };
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let Ok(kit) = dainty_service_kit::jsjson::parse(text) else {
        return kit_invalid(vec!["tester_max_kit.json is not valid JSON".to_owned()]);
    };
    let admin = LauncherApiAdmin::of(&ctx.shared);
    let funding = Arc::clone(admin.funding());
    let problems = validate_kit(&kit, &ctx.shared, funding.as_ref());
    if !problems.is_empty() {
        return kit_invalid(problems);
    }
    if !admin.begin_run(&user_id) {
        return admin_error(
            StatusCode::CONFLICT,
            "ADMIN_TESTER_MAX_BUSY",
            "A max run for this account is already in progress.",
        );
    }
    let busy = Busy {
        admin: &admin,
        user: user_id.clone(),
    };
    let request_id = fresh_id();
    let outcome = run(
        &ctx.shared,
        &ctx.store,
        funding.as_ref(),
        &target,
        &user_id,
        &run_id,
        &kit,
    )
    .await;
    drop(busy);
    let caller = ctx.caller.clone();
    let caller_user = ctx.caller_user_id();
    let (action, new_state) = match &outcome {
        Ok(report) => ("player.tester_max", report.summary()),
        Err((code, _)) => (
            "player.tester_max.failed",
            object(vec![("refused", Value::from(*code))]),
        ),
    };
    let audit = AdminAuditRecord {
        id: fresh_id(),
        actor_user_id: caller_user.clone(),
        target_user_id: Some(user_id.clone()),
        action: action.to_owned(),
        old_state: Some(object(vec![(
            "roles",
            Value::Array(target.roles.iter().cloned().map(Value::from).collect()),
        )])),
        new_state: Some(new_state),
        reason: Some(format!("tester max run {run_id}")),
        ip: ctx.request.client_ip.clone(),
        request_id: request_id.clone(),
        created_at: iso(now_ms()),
    };
    let actor = caller
        .as_ref()
        .map_or_else(|| ActorContext::new(Actor::System), admin_actor);
    let _ = ctx
        .store
        .write(&actor, move |tx, repos| repos.admin.append_audit(tx, &audit))
        .await;
    match outcome {
        Ok(report) => {
            tracing::info!(
                "[TesterMax] run={run_id} user={user_id} status={} actor={caller_user}",
                report.status()
            );
            util::ok(&object(vec![
                ("requestId", Value::from(request_id)),
                ("report", report.to_json()),
            ]))
        }
        Err((code, message)) => {
            tracing::warn!("[TesterMax] run={run_id} user={user_id} refused: {code}");
            let mut body = util::error_object(code, message);
            if let Value::Object(map) = &mut body {
                map.insert("requestId".into(), Value::from(request_id));
            }
            json(StatusCode::CONFLICT, &body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_ids_and_run_ids() {
        assert_eq!(
            plan_blocks(0, 250_000, 100_000),
            [(0, 100_000), (1, 100_000), (2, 50_000)]
        );
        assert_eq!(plan_blocks(200_000, 250_000, 100_000), [(2, 50_000)]);
        assert!(plan_blocks(5, 5, 100_000).is_empty());
        assert_eq!(plan_blocks(1234, 100_000, 100_000), [(0, 98_766)]);
        let id = derived_id("run-0001", "items");
        assert_eq!(id, derived_id("run-0001", "items"));
        assert!(id.starts_with("tmax-") && id.len() == 45);
        assert!(is_run_id("abcdefgh") && !is_run_id("abcdefg") && !is_run_id("-leading-hyphen"));
    }
}
