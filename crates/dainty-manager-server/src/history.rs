// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! History and revert against the workspace's databases (spec-tauri-shell §7, DD23, DH3b), through the same
//! engine as `dainty-host history` and `revert`. Whether the server runs does not matter.

use std::path::Path;
use std::sync::LazyLock;

use dainty_admin::history::{
    self, DEFAULT_PAGE, HistoryError, HistorySet, MAX_LISTED, PlanCache, Selection, conflict_fields, host_actor,
    kind_name, redacted_json, redacted_text,
};
use dainty_admin::input::iso;
use dainty_host::paths::Paths;
use dainty_launcher_kit::bridge::checks::HistoryFilter;
use dainty_launcher_kit::bridge::modules::refusal;
use dainty_launcher_kit::bridge::types::{
    ErrorCode, HistoryEntryView, HistoryPageView, LauncherError, RevertConflictView, RevertDoneView, RevertPlanView,
    RevertRowView, RevertedInView,
};

/// The plans this process showed in a dry run: apply takes only one of these.
static PLANS: LazyLock<PlanCache> = LazyLock::new(PlanCache::default);

/// The most of a failure's text the page's banner shows.
const FAILED_MAX: usize = 200;

/// The actor the change log records for the manager's reverts (`host:server-gui`).
pub const ACTOR: &str = "server-gui";

fn failed(text: &str) -> LauncherError {
    let first = text.lines().next().unwrap_or_default();
    refusal(
        ErrorCode::HistoryFailed,
        first.chars().take(FAILED_MAX).collect::<String>(),
    )
}

fn refused(error: HistoryError) -> LauncherError {
    match error {
        HistoryError::Bad(_) => refusal(ErrorCode::HistoryBadFilter, None),
        HistoryError::NoDatabase => refusal(ErrorCode::HistoryUnavailable, None),
        HistoryError::PlanChanged => refusal(ErrorCode::HistoryPlanChanged, None),
        other => failed(&other.to_string()),
    }
}

async fn open(workspace: &Path) -> Result<HistorySet, LauncherError> {
    let env = Paths::new(workspace).config.join("admin.env");
    if !env.is_file() {
        return Err(refusal(ErrorCode::HistoryUnavailable, None));
    }
    let node = history::node_database(Some(&env)).map_err(|why| failed(&why))?;
    HistorySet::open(&node).await.map_err(refused)
}

fn filter(page: &HistoryFilter) -> Result<dainty_store::revert::Filter, LauncherError> {
    Selection {
        account: page.account.clone(),
        actor: page.actor.clone(),
        since: page.since.clone(),
        until: page.until.clone(),
    }
    .to_filter(dainty_admin::now_ms())
    .map_err(refused)
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn conflict_view(database: &str, conflict: &dainty_store::revert::RowConflict) -> RevertConflictView {
    let fields = conflict_fields(conflict);
    RevertConflictView {
        database: database.to_owned(),
        table: conflict.table.clone(),
        key: conflict.key.to_string(),
        reason: fields.reason.to_owned(),
        entry: fields.entry,
        by: fields.by,
        at: fields.at,
        detail: fields.detail,
    }
}

fn conflicts<'a>(
    parts: impl Iterator<Item = (&'a str, &'a dainty_store::revert::RevertPlan)>,
) -> Vec<RevertConflictView> {
    parts
        .flat_map(|(database, plan)| plan.conflicts.iter().map(move |c| conflict_view(database, c)))
        .take(MAX_LISTED)
        .collect()
}

/// A page of the change log, newest first.
pub async fn query(workspace: &Path, page: &HistoryFilter) -> Result<HistoryPageView, LauncherError> {
    let set = open(workspace).await?;
    let filter = filter(page)?;
    let found = set
        .query(&filter, page.cursor.as_deref(), DEFAULT_PAGE)
        .await
        .map_err(refused)?;
    Ok(HistoryPageView {
        entries: found
            .entries
            .into_iter()
            .map(|e| {
                let table = e.entry.table_name;
                HistoryEntryView {
                    id: e.entry.id.to_string(),
                    database: e.database,
                    at: iso(e.entry.at),
                    actor: e.entry.actor,
                    key: e.entry.row_key,
                    op: e.entry.op,
                    before: redacted_text(&table, e.entry.before.as_deref()),
                    after: redacted_text(&table, e.entry.after.as_deref()),
                    table,
                }
            })
            .collect(),
        next: found.next,
    })
}

/// What a revert of the selected changes would do; the plan is kept for [`apply`].
pub async fn dry_run(workspace: &Path, selection: &HistoryFilter) -> Result<RevertPlanView, LauncherError> {
    let set = open(workspace).await?;
    let plan = set.dry_run(&filter(selection)?).await.map_err(refused)?;
    let actions: Vec<RevertRowView> = plan
        .parts
        .iter()
        .flat_map(|part| {
            part.plan.actions.iter().map(|action| RevertRowView {
                database: part.database.clone(),
                table: action.table.clone(),
                key: action.key.to_string(),
                kind: kind_name(action.kind).to_owned(),
                current: redacted_json(&action.table, action.current.as_ref()),
                target: redacted_json(&action.table, action.target.as_ref()),
            })
        })
        .take(MAX_LISTED)
        .collect();
    let view = RevertPlanView {
        id: plan.id.clone(),
        entries: count(plan.entries()),
        action_count: count(plan.actions()),
        conflict_count: count(plan.conflicts()),
        unchanged_count: count(plan.unchanged()),
        actions,
        conflicts: conflicts(plan.parts.iter().map(|p| (p.database.as_str(), &p.plan))),
        truncated: [plan.actions(), plan.conflicts(), plan.unchanged()]
            .into_iter()
            .any(|n| n > MAX_LISTED),
    };
    PLANS.put(plan);
    Ok(view)
}

/// Applies a plan this process showed, only while it still holds (the engine plans again under its lock).
pub async fn apply(workspace: &Path, plan_id: &str) -> Result<RevertDoneView, LauncherError> {
    let Some(plan) = PLANS.get(plan_id) else {
        return Err(refusal(ErrorCode::HistoryPlanChanged, None));
    };
    let set = open(workspace).await?;
    let applied = set
        .apply(&host_actor(ACTOR), &plan.selection, &plan.id)
        .await
        .map_err(refused)?;
    PLANS.forget(plan_id);
    let outcomes = || {
        applied
            .done
            .iter()
            .map(|(database, outcome)| (database.as_str(), &outcome.plan))
    };
    let conflict_count: usize = outcomes().map(|(_, plan)| plan.conflicts.len()).sum();
    Ok(RevertDoneView {
        at: applied.done.first().map(|(_, outcome)| iso(outcome.at)),
        rows_changed: count(outcomes().map(|(_, plan)| plan.actions.len()).sum()),
        databases: outcomes()
            .map(|(database, plan)| RevertedInView {
                database: database.to_owned(),
                rows: count(plan.actions.len()),
            })
            .collect(),
        conflicts: conflicts(outcomes()),
        conflict_count: count(conflict_count),
        truncated: conflict_count > MAX_LISTED,
    })
}
