// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Staff grants: items, currencies and progression XP or ranks given to one account from the dashboard. They go
//! through the same inventory transaction and progression grant ledger as the game's own grants and tester max,
//! so a granted item is stored exactly like one the game gave, and each grant id applies once.

use std::sync::Arc;

use axum::response::Response;
use dainty_module_api::ActorContext;
use dainty_service_kit::js::safe_integer;
use http::StatusCode;
use rustic_isles::Shared;
use rustic_isles_store::repo::reforged::AdminAuditRecord;
use rustic_isles_store::{AccountId, IslesStore};
use serde_json::Value;

use super::tester_max::{
    RankRequirement, StackOutcome, StepError, TesterFunding, confirm_rank, cut, derived_with, grant_xp_block, is_run_id,
};
use super::{AdminContext, LauncherApiAdmin, admin_actor, admin_error, admin_internal, fresh_id, player_not_found};
use crate::util::{self, iso, json, now_ms, object};

const PREFIX: &str = "agrant";
const MAX_ITEM_QUANTITY: i64 = 1000;
const MAX_CURRENCY: i64 = 10_000_000;
const MAX_XP: i64 = 10_000_000;
const MAX_ENTRIES: usize = 50;

/// What one progression entry asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XpGoal {
    /// Add this much XP.
    Add(i64),
    /// Raise the track to at least this rank.
    Rank(i64),
}

/// A grant the request asked for, validated.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrantRequest {
    /// Item stacks, merged by catalog id.
    pub stacks: Vec<(String, i64)>,
    /// Progression entries in the order given.
    pub progression: Vec<(String, XpGoal)>,
}

/// The body's problems, in order, or the grant it asks for.
pub fn validate(
    body: Option<&Value>,
    shared: &Shared,
    funding: &dyn TesterFunding,
) -> Result<GrantRequest, Vec<String>> {
    let Some(map) = body.and_then(Value::as_object) else {
        return Err(vec!["the body must be a JSON object".to_owned()]);
    };
    let mut problems = Vec::new();
    for key in map.keys() {
        if !matches!(
            key.as_str(),
            "grantId" | "reason" | "items" | "currencies" | "progression"
        ) {
            problems.push(format!("unknown field \"{key}\""));
        }
    }
    let section = |name: &str, problems: &mut Vec<String>| -> Vec<Value> {
        match map.get(name) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) if list.len() <= MAX_ENTRIES => list.clone(),
            Some(_) => {
                problems.push(format!("{name} must be an array of at most {MAX_ENTRIES} entries"));
                Vec::new()
            }
        }
    };
    let id_text = |v: Option<&Value>| dainty_service_kit::js::string(v);
    let mut request = GrantRequest::default();
    let mut add = |id: &str, q: i64| match request.stacks.iter_mut().find(|(c, _)| c == id) {
        Some((_, total)) => *total += q,
        None => request.stacks.push((id.to_owned(), q)),
    };
    for (i, entry) in section("items", &mut problems).iter().enumerate() {
        let at = format!("items[{i}]");
        let id = entry.get("catalogId").and_then(Value::as_str);
        let quantity = safe_integer(entry.get("quantity")).filter(|n| (1..=MAX_ITEM_QUANTITY).contains(n));
        match id {
            Some(id) if id.starts_with("CURRENCY_") => {
                problems.push(format!("{at}: {id} is a currency; list it under currencies"));
            }
            Some(id) if funding.is_known_catalog_id(shared, id) => {}
            _ => problems.push(format!(
                "{at}: \"{}\" is not in the server's catalog",
                id_text(entry.get("catalogId"))
            )),
        }
        if quantity.is_none() {
            problems.push(format!(
                "{at}: quantity must be an integer from 1 to {MAX_ITEM_QUANTITY}"
            ));
        }
        if let (Some(id), Some(q)) = (id, quantity) {
            add(id, q);
        }
    }
    for (i, entry) in section("currencies", &mut problems).iter().enumerate() {
        let at = format!("currencies[{i}]");
        let id = entry.get("catalogId").and_then(Value::as_str);
        let quantity = safe_integer(entry.get("quantity")).filter(|n| (1..=MAX_CURRENCY).contains(n));
        match id {
            // The aggregate is a spend-only view of the buckets (economy spec)
            Some(id) if id == rustic_isles_economy::platinum::PLATINUM_AGGREGATE_ID => problems.push(format!(
                "{at}: {id} is the Platinum aggregate; grant a storefront bucket (CURRENCY_PLATINUM_*) instead"
            )),
            Some(id) if id.starts_with("CURRENCY_") && funding.is_known_catalog_id(shared, id) => {}
            _ => problems.push(format!(
                "{at}: \"{}\" is not a known CURRENCY_* id",
                id_text(entry.get("catalogId"))
            )),
        }
        if quantity.is_none() {
            problems.push(format!("{at}: quantity must be an integer from 1 to {MAX_CURRENCY}"));
        }
        if let (Some(id), Some(q)) = (id, quantity) {
            add(id, q);
        }
    }
    for (i, entry) in section("progression", &mut problems).iter().enumerate() {
        let at = format!("progression[{i}]");
        let id = entry.get("progressionId").and_then(Value::as_str);
        let approved = id.filter(|id| funding.is_approved_xp_track(shared, id));
        if approved.is_none() {
            problems.push(format!(
                "{at}: \"{}\" is not an approved XP track",
                id_text(entry.get("progressionId"))
            ));
        }
        if let Some(id) = approved
            && request.progression.iter().any(|(seen, _)| seen == id)
        {
            problems.push(format!("{at}: {id} is listed twice"));
        }
        let goal = match (entry.get("xp"), entry.get("rank")) {
            (Some(xp), None) => match safe_integer(Some(xp)).filter(|n| (1..=MAX_XP).contains(n)) {
                Some(n) => Some(XpGoal::Add(n)),
                None => {
                    problems.push(format!("{at}: xp must be an integer from 1 to {MAX_XP}"));
                    None
                }
            },
            (None, Some(rank)) => {
                let rank = safe_integer(Some(rank)).filter(|n| *n >= 1);
                match (rank, approved) {
                    (Some(rank), Some(id)) => match funding.required_xp_for_rank(shared, id, rank) {
                        RankRequirement::Xp(_) => Some(XpGoal::Rank(rank)),
                        _ => {
                            problems.push(format!("{at}: rank {rank} is past {id}'s top rank"));
                            None
                        }
                    },
                    (None, _) => {
                        problems.push(format!("{at}: rank must be a whole number of at least 1"));
                        None
                    }
                    (Some(_), None) => None,
                }
            }
            _ => {
                problems.push(format!("{at}: give exactly one of xp or rank"));
                None
            }
        };
        if let (Some(id), Some(goal)) = (approved, goal) {
            request.progression.push((id.to_owned(), goal));
        }
    }
    if problems.is_empty() && request.stacks.is_empty() && request.progression.is_empty() {
        problems.push("nothing to grant".to_owned());
    }
    if problems.is_empty() {
        Ok(request)
    } else {
        Err(problems)
    }
}

#[derive(Debug, Clone, Default)]
struct TrackRow {
    id: String,
    start: i64,
    granted: i64,
    progress: i64,
    confirmed: Option<i64>,
    error: Option<String>,
}

impl TrackRow {
    fn to_json(&self) -> Value {
        let mut pairs = vec![
            ("progressionId", Value::from(self.id.clone())),
            ("startProgress", Value::from(self.start)),
            ("granted", Value::from(self.granted)),
            ("progress", Value::from(self.progress)),
            ("confirmedRank", self.confirmed.map_or(Value::Null, Value::from)),
            (
                "status",
                Value::from(if self.error.is_some() { "error" } else { "done" }),
            ),
        ];
        if let Some(error) = &self.error {
            pairs.push(("error", Value::from(error.clone())));
        }
        object(pairs)
    }
}

async fn progress_of(store: &IslesStore, account: &AccountId, id: &str) -> Result<i64, StepError> {
    let (a, track) = (account.clone(), id.to_owned());
    Ok(store
        .read(move |conn, repos| repos.progression_tracks.get(conn, &a, &track))
        .await
        .map_err(StepError::Repo)?
        .map_or(0, |t| t.progress))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the grant's context and the one track it drives"
)]
async fn grant_track(
    shared: &Arc<Shared>,
    store: &IslesStore,
    funding: &dyn TesterFunding,
    account: &AccountId,
    actor: &ActorContext,
    user_id: &str,
    grant_id: &str,
    id: &str,
    goal: &XpGoal,
    row: &mut TrackRow,
) -> Result<(), StepError> {
    row.start = progress_of(store, account, id).await?;
    let ceiling = funding.max_xp_grant().max(1);
    // Blocks are named from the amount (Add) or the target (Rank), never from the progress read now, so a retried
    // grant id replays the same ledger rows instead of granting twice.
    let blocks: Vec<(String, i64)> = match goal {
        XpGoal::Add(amount) => {
            let mut out = Vec::new();
            let (mut left, mut index) = (*amount, 0);
            while left > 0 {
                let piece = left.min(ceiling);
                out.push((format!("xp:{id}:add:{index}"), piece));
                left -= piece;
                index += 1;
            }
            out
        }
        XpGoal::Rank(rank) => {
            let RankRequirement::Xp(required) = funding.required_xp_for_rank(shared, id, *rank) else {
                return Err(StepError::Plain(format!("rank {rank} is past the track's top rank")));
            };
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a cumulative XP total from the host's curve"
            )]
            let target = required.ceil() as i64;
            super::tester_max::plan_blocks(row.start, target, ceiling)
                .into_iter()
                .map(|(index, amount)| (format!("xp:{id}:rank:{rank}:{index}"), amount))
                .collect()
        }
    };
    for (part, amount) in blocks {
        let block_id = derived_with(PREFIX, grant_id, &part);
        if grant_xp_block(store, account, actor, block_id, user_id, id, amount)
            .await
            .map_err(StepError::Repo)?
        {
            row.granted += amount;
        }
    }
    row.progress = progress_of(store, account, id).await?;
    let rank = funding.rank_for_progress(shared, id, row.progress).unwrap_or(0);
    if rank > 0 {
        confirm_rank(shared, store, funding, account, actor, id, rank).await?;
        row.confirmed = Some(rank);
    }
    Ok(())
}

/// `GET /admin/v1/grant-catalog`: what a grant may name on this server.
pub async fn catalog(ctx: AdminContext) -> Response {
    let admin = LauncherApiAdmin::of(&ctx.shared);
    let funding = admin.funding();
    let (currencies, items): (Vec<_>, Vec<_>) = funding
        .catalog(&ctx.shared)
        .into_iter()
        .filter(|e| e.id != rustic_isles_economy::platinum::PLATINUM_AGGREGATE_ID)
        .partition(|e| e.id.starts_with("CURRENCY_"));
    let items = items
        .into_iter()
        .map(|e| {
            object(vec![
                ("catalogId", Value::from(e.id)),
                ("storage", Value::from(e.storage)),
                ("tags", Value::Array(e.tags.into_iter().map(Value::from).collect())),
            ])
        })
        .collect();
    let currencies = currencies.into_iter().map(|e| Value::from(e.id)).collect();
    let tracks = funding
        .xp_tracks(&ctx.shared)
        .into_iter()
        .map(|id| {
            let top = funding
                .track_cap_xp(&ctx.shared, &id)
                .and_then(|cap| funding.rank_for_progress(&ctx.shared, &id, cap));
            object(vec![
                ("progressionId", Value::from(id)),
                ("maxRank", top.map_or(Value::Null, Value::from)),
            ])
        })
        .collect();
    util::ok(&object(vec![
        ("items", Value::Array(items)),
        ("currencies", Value::Array(currencies)),
        ("progression", Value::Array(tracks)),
    ]))
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

fn stacks_json(stacks: &[(String, i64)]) -> Value {
    Value::Array(
        stacks
            .iter()
            .map(|(id, q)| {
                object(vec![
                    ("catalogId", Value::from(id.clone())),
                    ("quantity", Value::from(*q)),
                ])
            })
            .collect(),
    )
}

/// `POST /admin/v1/players/:userId/grants`: grants items, currencies and progression to one account.
pub async fn grant(ctx: AdminContext) -> Response {
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("the grant had no caller");
    };
    let Some(grant_id) = ctx
        .request
        .field("grantId")
        .and_then(Value::as_str)
        .filter(|r| is_run_id(r))
        .map(str::to_owned)
    else {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_GRANT_ID",
            "grantId must be 8 to 64 characters: letters, digits, dot, underscore or hyphen.",
        );
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
            "ADMIN_GRANT_REASON",
            "A reason of at least 3 characters is required.",
        );
    }
    let user_id = ctx.param("userId").to_owned();
    let lookup = user_id.clone();
    let target = match ctx
        .store
        .read(move |conn, repos| super::find_target(conn, repos, &lookup))
        .await
    {
        Ok(Some(target)) => target,
        Ok(None) => return player_not_found(),
        Err(e) => return admin_internal(e),
    };
    let admin = LauncherApiAdmin::of(&ctx.shared);
    let funding = Arc::clone(admin.funding());
    let request = match validate(ctx.request.body.as_ref(), &ctx.shared, funding.as_ref()) {
        Ok(request) => request,
        Err(problems) => {
            return json(
                StatusCode::UNPROCESSABLE_ENTITY,
                &object(vec![(
                    "error",
                    object(vec![
                        ("code", Value::from("ADMIN_GRANT_INVALID")),
                        ("message", Value::from(problems.join("; "))),
                        (
                            "problems",
                            Value::Array(problems.into_iter().map(Value::from).collect()),
                        ),
                    ]),
                )]),
            );
        }
    };
    let account = target.account.clone();
    let character = if request.stacks.is_empty() {
        None
    } else {
        let a = account.clone();
        let characters = ctx
            .store
            .read(move |conn, repos| repos.characters.find_many_by_account(conn, &a))
            .await
            .unwrap_or_default();
        match characters
            .into_iter()
            .map(|c| c.character_id)
            .find(|id| !util::trim(id).is_empty())
        {
            Some(character) => Some(character),
            None => {
                return admin_error(
                    StatusCode::CONFLICT,
                    "ADMIN_GRANT_NO_CHARACTER",
                    "The player has no character yet; they must sign in to the game once first.",
                );
            }
        }
    };
    if !admin.begin_run(&user_id) {
        return admin_error(
            StatusCode::CONFLICT,
            "ADMIN_GRANT_BUSY",
            "Another grant or max run for this account is in progress.",
        );
    }
    let busy = Busy {
        admin: &admin,
        user: user_id.clone(),
    };
    let actor = admin_actor(&caller);
    let mut partial = false;
    let mut rows = Vec::new();
    for (id, goal) in &request.progression {
        let mut row = TrackRow {
            id: id.clone(),
            ..TrackRow::default()
        };
        if let Err(e) = grant_track(
            &ctx.shared,
            &ctx.store,
            funding.as_ref(),
            &account,
            &actor,
            &user_id,
            &grant_id,
            id,
            goal,
            &mut row,
        )
        .await
        {
            row.error = Some(cut(&e.to_string()));
            partial = true;
        }
        rows.push(row);
    }
    let mut items = None;
    if let Some(character) = &character {
        let transaction = derived_with(PREFIX, &grant_id, "items");
        let outcome = funding
            .apply_operator_test_stacks(
                &ctx.shared,
                &user_id,
                character,
                &transaction,
                &request.stacks,
                "admin-grant",
            )
            .await;
        let error = match outcome {
            StackOutcome::Applied => None,
            StackOutcome::NotOwned => Some("the inventory transaction did not apply".to_owned()),
            StackOutcome::Failed(text) => Some(cut(&text)),
        };
        partial |= error.is_some();
        let mut pairs = vec![("status", Value::from(if error.is_some() { "error" } else { "done" }))];
        if let Some(error) = error {
            pairs.push(("error", Value::from(error)));
        }
        pairs.push(("stacks", stacks_json(&request.stacks)));
        items = Some(object(pairs));
    }
    drop(busy);
    let status = if partial { "partial" } else { "done" };
    let mut report = vec![
        ("grantId", Value::from(grant_id.clone())),
        ("userId", Value::from(user_id.clone())),
        ("status", Value::from(status)),
        (
            "progression",
            Value::Array(rows.iter().map(TrackRow::to_json).collect()),
        ),
    ];
    if let Some(character) = &character {
        report.push(("characterId", Value::from(character.clone())));
    }
    if let Some(items) = items {
        report.push(("items", items));
    }
    let report = object(report);
    let request_id = fresh_id();
    let audit = AdminAuditRecord {
        id: fresh_id(),
        actor_user_id: caller.user_id.clone(),
        target_user_id: Some(user_id.clone()),
        action: "player.grant".to_owned(),
        old_state: None,
        new_state: Some(report.clone()),
        reason: Some(reason),
        ip: ctx.request.client_ip.clone(),
        request_id: request_id.clone(),
        created_at: iso(now_ms()),
    };
    if let Err(e) = ctx
        .store
        .write(&actor, move |tx, repos| repos.admin.append_audit(tx, &audit))
        .await
    {
        tracing::error!("[AdminGrant] grant={grant_id} user={user_id} audit failed: {e}");
    }
    tracing::info!(
        "[AdminGrant] grant={grant_id} user={user_id} stacks={} tracks={} status={status} actor={}",
        request.stacks.len(),
        request.progression.len(),
        caller.user_id
    );
    util::ok(&object(vec![
        ("requestId", Value::from(request_id)),
        ("report", report),
    ]))
}
