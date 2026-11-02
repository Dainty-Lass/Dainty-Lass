//! History and revert in the server GUI (DD23, DH3b): `history_query`, `history_revert_dry_run` and
//! `history_revert_apply` against the databases of the server the launcher hosts, read from its workspace
//! (`admin::history`, the same engine as `reforged-host history` and `revert`). Whether the server runs
//! does not matter: the files are the same, and a stopped server's history is the thing to read after a
//! partner has been in it.
//!
//! A dry run keeps its plan here until the apply takes it by id, so the page applies exactly what it showed;
//! the engine plans again under the write lock and refuses a plan that no longer holds. The change log
//! names the launcher as `host:server-gui`.

use std::path::Path;
use std::sync::LazyLock;

use admin::history::{self, HistoryError, HistorySet, PlanCache, RowConflict, Selection};

use crate::bridge::types::{
    ErrorCode, HistoryEntryView, HistoryOutcome, HistoryPageView, LauncherError, RevertApplyOutcome,
    RevertConflictView, RevertDoneView, RevertPlanOutcome, RevertPlanView, RevertRowView, RevertedInView,
};

/// The actor of a revert made from the server GUI.
const ACTOR: &str = "server-gui";

/// What the page sent: a history filter, or a revert's selection (which has no cursor).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryArgs {
    pub account: Option<String>,
    pub actor: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub cursor: Option<String>,
}

impl HistoryArgs {
    fn selection(&self) -> Selection {
        Selection {
            account: self.account.clone(),
            actor: self.actor.clone(),
            since: self.since.clone(),
            until: self.until.clone(),
        }
    }
}

static PLANS: LazyLock<PlanCache> = LazyLock::new(PlanCache::default);

fn failure(error: HistoryError) -> LauncherError {
    match error {
        HistoryError::Bad(_) => LauncherError::new(ErrorCode::HistoryBadFilter),
        HistoryError::NoDatabase => LauncherError::new(ErrorCode::HistoryUnavailable),
        HistoryError::PlanChanged => LauncherError::new(ErrorCode::HistoryPlanChanged),
        other => failed(&other.to_string()),
    }
}

fn failed(why: &str) -> LauncherError {
    // One line, short: it is shown in a banner
    let line: String = why.lines().next().unwrap_or_default().chars().take(200).collect();
    LauncherError::with_detail(ErrorCode::HistoryFailed, &line)
}

async fn open(workspace: &Path) -> Result<HistorySet, LauncherError> {
    let env = host::paths::Paths::new(workspace).config.join("admin.env");
    if !env.is_file() {
        return Err(LauncherError::new(ErrorCode::HistoryUnavailable));
    }
    let node = history::node_database(Some(&env)).map_err(|why| failed(&why))?;
    HistorySet::open(&node).await.map_err(failure)
}

fn conflict_view(database: &str, conflict: &RowConflict) -> RevertConflictView {
    let f = history::conflict_fields(conflict);
    RevertConflictView {
        database: database.to_owned(),
        table: conflict.table.clone(),
        key: conflict.key.to_string(),
        reason: f.reason.to_owned(),
        entry: f.entry,
        by: f.by,
        at: f.at,
        detail: f.detail,
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn conflicts_of(plan: &history::Plan) -> Vec<RevertConflictView> {
    plan.parts
        .iter()
        .flat_map(|part| part.plan.conflicts.iter().map(|c| conflict_view(&part.database, c)))
        .take(history::MAX_LISTED)
        .collect()
}

fn plan_view(plan: &history::Plan) -> RevertPlanView {
    let actions = plan
        .parts
        .iter()
        .flat_map(|part| {
            part.plan.actions.iter().map(|a| RevertRowView {
                database: part.database.clone(),
                table: a.table.clone(),
                key: a.key.to_string(),
                kind: history::kind_name(a.kind).to_owned(),
                current: history::redacted_json(&a.table, a.current.as_ref()),
                target: history::redacted_json(&a.table, a.target.as_ref()),
            })
        })
        .take(history::MAX_LISTED)
        .collect();
    RevertPlanView {
        id: plan.id.clone(),
        entries: count(plan.entries()),
        action_count: count(plan.actions()),
        conflict_count: count(plan.conflicts()),
        unchanged_count: count(plan.unchanged()),
        actions,
        conflicts: conflicts_of(plan),
        truncated: plan.actions() > history::MAX_LISTED
            || plan.conflicts() > history::MAX_LISTED
            || plan.unchanged() > history::MAX_LISTED,
    }
}

pub async fn query(workspace: &Path, args: HistoryArgs) -> HistoryOutcome {
    let run = async {
        let filter = args.selection().to_filter(admin::now_ms()).map_err(failure)?;
        let set = open(workspace).await?;
        set.query(&filter, args.cursor.as_deref(), history::DEFAULT_PAGE)
            .await
            .map_err(failure)
    };
    match run.await {
        Ok(page) => HistoryOutcome::Ok(HistoryPageView {
            entries: page
                .entries
                .into_iter()
                .map(|e| HistoryEntryView {
                    id: e.entry.id.to_string(),
                    database: e.database,
                    at: admin::input::iso(e.entry.at),
                    actor: e.entry.actor,
                    key: e.entry.row_key,
                    op: e.entry.op,
                    before: history::redacted_text(&e.entry.table_name, e.entry.before.as_deref()),
                    after: history::redacted_text(&e.entry.table_name, e.entry.after.as_deref()),
                    table: e.entry.table_name,
                })
                .collect(),
            next: page.next,
        }),
        Err(error) => HistoryOutcome::Err(error),
    }
}

pub async fn dry_run(workspace: &Path, args: HistoryArgs) -> RevertPlanOutcome {
    let run = async {
        let filter = args.selection().to_filter(admin::now_ms()).map_err(failure)?;
        let set = open(workspace).await?;
        set.dry_run(&filter).await.map_err(failure)
    };
    match run.await {
        Ok(plan) => {
            PLANS.put(plan.clone());
            RevertPlanOutcome::Ok(plan_view(&plan))
        }
        Err(error) => RevertPlanOutcome::Err(error),
    }
}

pub async fn apply(workspace: &Path, plan_id: &str) -> RevertApplyOutcome {
    let run = async {
        // A plan the page never got from a dry run here is not applied
        let plan = PLANS
            .get(plan_id)
            .ok_or_else(|| LauncherError::new(ErrorCode::HistoryPlanChanged))?;
        let set = open(workspace).await?;
        let actor = history::host_actor(ACTOR);
        set.apply(&actor, &plan.selection, plan_id).await.map_err(failure)
    };
    match run.await {
        Ok(applied) => {
            PLANS.forget(plan_id);
            let rows: usize = applied.done.iter().map(|(_, o)| o.plan.actions.len()).sum();
            RevertApplyOutcome::Ok(RevertDoneView {
                at: applied.done.first().map(|(_, o)| admin::input::iso(o.at)),
                rows_changed: count(rows),
                databases: applied
                    .done
                    .iter()
                    .map(|(database, o)| RevertedInView {
                        database: database.clone(),
                        rows: count(o.plan.actions.len()),
                    })
                    .collect(),
                conflicts: conflicts_of(&applied.plan),
                conflict_count: count(applied.plan.conflicts()),
                truncated: applied.plan.conflicts() > history::MAX_LISTED,
            })
        }
        Err(error) => RevertApplyOutcome::Err(error),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // tests

    use admin::history::Actor;
    use admin::keys::{Keys, Role};

    use super::*;

    /// A workspace with the admin API's database: two moderators, and a partner who revoked both.
    async fn workspace() -> (tempfile::TempDir, Vec<String>, String) {
        let dir = tempfile::tempdir().unwrap();
        let paths = host::paths::Paths::new(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let slashed = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
        let key = "0123456789abcdef0123456789abcdef";
        let env = format!(
            "REFORGED_ADMIN_API_KEY={key}\nREFORGED_ADMIN_KEY={key}\nRUSTIC_ISLES_MANAGER_OPERATOR_SECRET={key}\n\
             REFORGED_ADMIN_API_TIMEOUTS_FILE={}\nREFORGED_DB_ENGINE=sqlite\nREFORGED_DB_FILE={}\n",
            slashed(&paths.config.join("timeouts.json")),
            slashed(&paths.config.join("dainty-lass.db")),
        );
        std::fs::write(paths.config.join("admin.env"), env).unwrap();
        let node = history::node_database(Some(&paths.config.join("admin.env"))).unwrap();
        let keys = Keys::open(&Keys::database_for(&node), key).unwrap();
        let host = Actor::Host("test".into());
        let (maya, _) = keys.issue("Maya", Role::Moderator, host.clone(), 1).await.unwrap();
        let (lee, _) = keys.issue("Lee", Role::Moderator, host.clone(), 1).await.unwrap();
        let (partner, _) = keys.issue("Partner", Role::Owner, host, 1).await.unwrap();
        for id in [&maya.id, &lee.id] {
            keys.revoke(id, Actor::Admin(partner.id.clone()), 2).await.unwrap();
        }
        (dir, vec![maya.id, lee.id], partner.id)
    }

    fn by(actor: &str) -> HistoryArgs {
        HistoryArgs {
            actor: Some(actor.to_owned()),
            ..HistoryArgs::default()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_reverts_a_partners_changes_with_the_plan_it_was_shown() {
        let (dir, revoked, partner) = workspace().await;
        let HistoryOutcome::Ok(page) = query(dir.path(), by(&format!("admin:{partner}"))).await else {
            panic!("a page");
        };
        assert_eq!(page.entries.len(), 2);
        assert!(
            page.entries
                .iter()
                .all(|e| e.table == "admin_keys" && e.database == "admin")
        );
        assert!(
            page.entries
                .iter()
                .all(|e| !e.after.as_deref().unwrap_or("").contains("key_hash\":\"0"))
        );

        let RevertPlanOutcome::Ok(plan) = dry_run(dir.path(), by(&format!("admin:{partner}"))).await else {
            panic!("a plan");
        };
        assert_eq!((plan.action_count, plan.conflict_count), (2, 0));
        assert!(
            plan.actions
                .iter()
                .all(|a| a.kind == "update" && revoked.iter().any(|id| a.key.contains(id)))
        );

        // Only a plan this page was shown is applied
        let RevertApplyOutcome::Err(unknown) = apply(dir.path(), &"f".repeat(64)).await else {
            panic!("refused");
        };
        assert_eq!(unknown.code, ErrorCode::HistoryPlanChanged);
        let RevertApplyOutcome::Ok(done) = apply(dir.path(), &plan.id).await else {
            panic!("applied");
        };
        assert_eq!((done.rows_changed, done.conflict_count), (2, 0));
        assert_eq!(done.databases[0].database, "admin");
        // The plan is spent
        assert!(matches!(apply(dir.path(), &plan.id).await, RevertApplyOutcome::Err(_)));
        // The revert is in the history as the server GUI
        let HistoryOutcome::Ok(page) = query(dir.path(), by("host:server-gui")).await else {
            panic!("a page");
        };
        assert_eq!(page.entries.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bad_filter_and_a_workspace_without_a_server_are_refused_with_codes() {
        let (dir, _, _) = workspace().await;
        let HistoryOutcome::Err(bad) = query(dir.path(), by("nobody-knows")).await else {
            panic!("refused");
        };
        assert_eq!(bad.code, ErrorCode::HistoryBadFilter);
        let RevertPlanOutcome::Err(nothing) = dry_run(dir.path(), HistoryArgs::default()).await else {
            panic!("refused");
        };
        assert_eq!(nothing.code, ErrorCode::HistoryBadFilter);

        let empty = tempfile::tempdir().unwrap();
        let HistoryOutcome::Err(unset) = query(empty.path(), HistoryArgs::default()).await else {
            panic!("refused");
        };
        assert_eq!(unset.code, ErrorCode::HistoryUnavailable);
    }
}
