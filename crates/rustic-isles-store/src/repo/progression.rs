// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Progression (spec §10): tracks, objectives, the grant ledger, escalation, the journey, breadcrumbs and
//! encountered content. One SQL type implements all six traits.

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Text};
use serde_json::{Map, Value};

use crate::sql::{account_id, sql};
use crate::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx};

/// The store's own `now` (spec §2.1): the request's replay clock when one is pinned.
pub fn now_iso() -> String {
    rustic_isles::season::to_iso_string(rustic_isles::replay::now_ms())
}

// ------------------------------------------------------------------ records

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponReforge {
    pub reforge_id: String,
    pub open: bool,
    pub prior_progress: i64,
    pub cap: i64,
    pub banked: i64,
    pub banked_restored: i64,
    pub prestige_granted: bool,
    pub opened_at: String,
    pub closed_at: Option<String>,
}

impl WeaponReforge {
    pub fn to_json(&self) -> Json {
        let mut out = Map::new();
        out.insert("reforgeId".into(), self.reforge_id.clone().into());
        out.insert("status".into(), if self.open { "open" } else { "closed" }.into());
        out.insert("priorProgress".into(), self.prior_progress.into());
        out.insert("cap".into(), self.cap.into());
        out.insert("banked".into(), self.banked.into());
        out.insert("bankedRestored".into(), self.banked_restored.into());
        out.insert("prestigeGranted".into(), self.prestige_granted.into());
        out.insert("openedAt".into(), self.opened_at.clone().into());
        if let Some(closed) = &self.closed_at {
            out.insert("closedAt".into(), closed.clone().into());
        }
        Value::Object(out)
    }

    /// Every key but `closedAt` with its type, else none.
    pub fn from_json(value: &Json) -> Option<Self> {
        let open = match value.get("status")?.as_str()? {
            "open" => true,
            "closed" => false,
            _ => return None,
        };
        let int = |key: &str| value.get(key).and_then(Value::as_i64);
        Some(Self {
            reforge_id: value.get("reforgeId")?.as_str()?.to_owned(),
            open,
            prior_progress: int("priorProgress")?,
            cap: int("cap")?,
            banked: int("banked")?,
            banked_restored: int("bankedRestored")?,
            prestige_granted: value.get("prestigeGranted")?.as_bool()?,
            opened_at: value.get("openedAt")?.as_str()?.to_owned(),
            closed_at: value.get("closedAt").and_then(Value::as_str).map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressionTrackRecord {
    pub account: AccountId,
    pub progression_id: String,
    pub progress: i64,
    pub confirmed_fremium_rank: i64,
    pub confirmed_premium_rank: i64,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
    pub reforge: Option<WeaponReforge>,
    pub prestige_claimed: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressionObjectiveRecord {
    pub account: AccountId,
    pub objective_id: String,
    pub value: i64,
    pub completed_count: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressionGrantRecord {
    pub grant_id: String,
    pub account: AccountId,
    pub progression_id: String,
    pub amount: i64,
    pub request_hash: String,
    pub result: ProgressionTrackRecord,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EscalationProgressRecord {
    pub account: AccountId,
    pub season_id: String,
    pub escalation_level: i64,
    pub next_level_xp: i64,
    pub talents_progress: Vec<Json>,
    pub unlock_progress: Vec<Json>,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerJourneyRecord {
    pub account: AccountId,
    pub nodes: String,
    pub update_version: i64,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbsRecord {
    pub character_id: String,
    pub account: AccountId,
    /// Stored in the `items` column
    pub breadcrumbs: String,
    pub update_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncounteredContentRecord {
    pub character_id: String,
    pub account: AccountId,
    /// Stored in the `content` column
    pub encountered_content: String,
}

// ------------------------------------------------------------------ traits

repository! {
    pub trait ProgressionTrackRepository = "progressionTracks" {
        fn get_all_for_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Vec<ProgressionTrackRecord>;
        fn get(&self, conn: &mut AnyConnection, account: &AccountId, id: &str) -> Option<ProgressionTrackRecord>;
        fn increment(&self, tx: &mut Tx<'_>, account: &AccountId, id: &str, amount: i64) -> ProgressionTrackRecord;
        fn set_progress_if_greater(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            id: &str,
            value: i64,
        ) -> ProgressionTrackRecord;
        fn set_confirmed_fremium_rank(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            id: &str,
            rank: i64,
        ) -> ProgressionTrackRecord;
        fn append_objective_event(&self, tx: &mut Tx<'_>, account: &AccountId, raw_body: &str, received_at: &str) -> ();
        fn get_all_objectives_for_account(
            &self,
            conn: &mut AnyConnection,
            account: &AccountId,
        ) -> Vec<ProgressionObjectiveRecord>;
        fn set_objective_if_greater(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            objective_id: &str,
            value: i64,
            completed: i64,
        ) -> ProgressionObjectiveRecord;
        /// `(objective id, value, completed count)` merged in the order given, at one `now`
        fn merge_objectives(&self, tx: &mut Tx<'_>, account: &AccountId, rows: &[(String, i64, i64)]) -> ();
        fn get_objectives(
            &self,
            conn: &mut AnyConnection,
            account: &AccountId,
            ids: &[String],
        ) -> Vec<ProgressionObjectiveRecord>;
        fn begin_reforge(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            id: &str,
            expected_progress: i64,
            expected_version: i64,
            reforge: &WeaponReforge,
        ) -> Option<ProgressionTrackRecord>;
        /// A compare-and-set on the stored reforge object; leaves the version and `updated_at` alone
        fn update_reforge(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            id: &str,
            reforge_id: &str,
            expected: &Json,
            set: &Json,
        ) -> bool;
        fn claim_prestige_levels(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            id: &str,
            claimed_before: i64,
            levels: i64,
            min_progress: i64,
        ) -> bool;
        fn spend(&self, tx: &mut Tx<'_>, account: &AccountId, id: &str, amount: i64) -> Option<ProgressionTrackRecord>;
    }
}

repository! {
    pub trait ProgressionGrantRepository = "progressionGrants" {
        fn find_by_grant_id(&self, conn: &mut AnyConnection, grant_id: &str) -> Option<ProgressionGrantRecord>;
        /// A replayed grant id is a database error: the caller then serves the stored result
        fn insert_applied(&self, tx: &mut Tx<'_>, record: &ProgressionGrantRecord) -> ();
        fn list_recent_for_account(
            &self,
            conn: &mut AnyConnection,
            account: &AccountId,
            limit: u32,
        ) -> Vec<ProgressionGrantRecord>;
    }
}

repository! {
    pub trait EscalationProgressRepository = "escalationProgress" {
        fn get(&self, conn: &mut AnyConnection, account: &AccountId, season_id: &str) -> Option<EscalationProgressRecord>;
        fn save_if_version(
            &self,
            tx: &mut Tx<'_>,
            record: &EscalationProgressRecord,
            expected: Option<i64>,
        ) -> Option<EscalationProgressRecord>;
    }
}

repository! {
    pub trait PlayerJourneyRepository = "playerJourney" {
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<PlayerJourneyRecord>;
        /// `false` when any row of the account exists, a soft-deleted one included
        fn create_if_absent(&self, tx: &mut Tx<'_>, record: &PlayerJourneyRecord) -> bool;
        fn update_if_revision(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            nodes: &str,
            update_version: i64,
            expected: i64,
        ) -> bool;
    }
}

repository! {
    pub trait BreadcrumbRepository = "breadcrumbs" {
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<BreadcrumbsRecord>;
        fn create(&self, tx: &mut Tx<'_>, record: &BreadcrumbsRecord) -> ();
        fn update(
            &self,
            tx: &mut Tx<'_>,
            character_id: &str,
            account: &AccountId,
            json: &str,
            update_version: i64,
        ) -> ();
    }
}

repository! {
    pub trait EncounteredContentRepository = "encounteredContent" {
        fn find(
            &self,
            conn: &mut AnyConnection,
            character_id: &str,
            account: &AccountId,
        ) -> Option<EncounteredContentRecord>;
        fn create(&self, tx: &mut Tx<'_>, record: &EncounteredContentRecord) -> ();
        fn update_content(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId, json: &str) -> ();
    }
}

pub struct SqlProgression;

// ------------------------------------------------------------------ tracks

#[derive(QueryableByName)]
struct TrackRow {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    progression_id: String,
    #[diesel(sql_type = BigInt)]
    progress: i64,
    #[diesel(sql_type = BigInt)]
    confirmed_fremium_rank: i64,
    #[diesel(sql_type = BigInt)]
    confirmed_premium_rank: i64,
    #[diesel(sql_type = BigInt)]
    update_version: i64,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    reforge: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    prestige_claimed: Option<i64>,
}

impl TrackRow {
    fn decode(self) -> RepoResult<ProgressionTrackRecord> {
        let reforge = match &self.reforge {
            None => None,
            // A malformed object reads as no reforge (spec §16 item 10)
            Some(text) => WeaponReforge::from_json(
                &serde_json::from_str::<Value>(text).map_err(|e| RepoError::data(format!("reforge: {e}")))?,
            ),
        };
        Ok(ProgressionTrackRecord {
            account: account_id(&self.account, "progression_tracks.account")?,
            progression_id: self.progression_id,
            progress: self.progress,
            confirmed_fremium_rank: self.confirmed_fremium_rank,
            confirmed_premium_rank: self.confirmed_premium_rank,
            update_version: self.update_version,
            created_at: self.created_at,
            updated_at: self.updated_at,
            reforge,
            prestige_claimed: self.prestige_claimed,
        })
    }
}

const LIVE_TRACK: &str =
    "SELECT * FROM progression_tracks WHERE account = ? AND progression_id = ? AND deleted_at IS NULL";
const TRACK_KEY: &str = "WHERE account = ? AND progression_id = ? AND deleted_at IS NULL";

fn track_row(conn: &mut AnyConnection, account: &AccountId, id: &str) -> RepoResult<Option<TrackRow>> {
    sql(LIVE_TRACK).t(account.to_string()).t(id).first::<TrackRow>(conn)
}

fn track(conn: &mut AnyConnection, account: &AccountId, id: &str) -> RepoResult<Option<ProgressionTrackRecord>> {
    track_row(conn, account, id)?.map(TrackRow::decode).transpose()
}

fn reread(conn: &mut AnyConnection, account: &AccountId, id: &str) -> RepoResult<ProgressionTrackRecord> {
    track(conn, account, id)?
        .ok_or_else(|| RepoError::data(format!("progression track {id} vanished inside its write")))
}

/// The live track, or a new one at zero (a soft-deleted key makes the insert fail: spec §16 item 3).
fn upsert_base(conn: &mut AnyConnection, account: &AccountId, id: &str, now: &str) -> RepoResult<TrackRow> {
    if let Some(row) = track_row(conn, account, id)? {
        return Ok(row);
    }
    let seq = sql("SELECT MAX(seq) AS n FROM progression_tracks WHERE account = ?")
        .t(account.to_string())
        .int(conn)?
        + 1;
    sql(
        "INSERT INTO progression_tracks (account, progression_id, progress, confirmed_fremium_rank, \
         confirmed_premium_rank, update_version, seq, created_at, updated_at) VALUES (?, ?, 0, 0, 0, 0, ?, ?, ?)",
    )
    .t(account.to_string())
    .t(id)
    .i(seq)
    .t(now)
    .t(now)
    .run(conn)?;
    track_row(conn, account, id)?
        .ok_or_else(|| RepoError::data(format!("progression track {id} vanished inside its write")))
}

/// Sets `column` and bumps the version at `now`; the re-read track.
fn set_track_column(
    conn: &mut AnyConnection,
    account: &AccountId,
    id: &str,
    column: &str,
    value: i64,
    now: &str,
) -> RepoResult<ProgressionTrackRecord> {
    sql(format!(
        "UPDATE progression_tracks SET {column} = ?, update_version = update_version + 1, updated_at = ? {TRACK_KEY}"
    ))
    .i(value)
    .t(now)
    .t(account.to_string())
    .t(id)
    .run(conn)?;
    reread(conn, account, id)
}

#[derive(QueryableByName)]
struct ObjectiveRow {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    objective_id: String,
    #[diesel(sql_type = BigInt)]
    value: i64,
    #[diesel(sql_type = BigInt)]
    completed_count: i64,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

impl ObjectiveRow {
    fn decode(self) -> RepoResult<ProgressionObjectiveRecord> {
        Ok(ProgressionObjectiveRecord {
            account: account_id(&self.account, "progression_objectives.account")?,
            objective_id: self.objective_id,
            value: self.value,
            completed_count: self.completed_count,
            updated_at: self.updated_at,
        })
    }
}

fn merge_objective(
    conn: &mut AnyConnection,
    account: &AccountId,
    objective_id: &str,
    value: i64,
    completed: i64,
    now: &str,
) -> RepoResult<ProgressionObjectiveRecord> {
    let live =
        sql("SELECT * FROM progression_objectives WHERE account = ? AND objective_id = ? AND deleted_at IS NULL")
            .t(account.to_string())
            .t(objective_id)
            .first::<ObjectiveRow>(conn)?;
    match live {
        Some(row) => {
            let value = row.value.max(value);
            let completed = row.completed_count.max(completed);
            sql(
                "UPDATE progression_objectives SET value = ?, completed_count = ?, updated_at = ? \
                 WHERE account = ? AND objective_id = ? AND deleted_at IS NULL",
            )
            .i(value)
            .i(completed)
            .t(now)
            .t(account.to_string())
            .t(objective_id)
            .run(conn)?;
            Ok(ProgressionObjectiveRecord {
                account: account.clone(),
                objective_id: objective_id.to_owned(),
                value,
                completed_count: completed,
                updated_at: now.to_owned(),
            })
        }
        None => {
            let seq = sql("SELECT MAX(seq) AS n FROM progression_objectives WHERE account = ?")
                .t(account.to_string())
                .int(conn)?
                + 1;
            sql(
                "INSERT INTO progression_objectives (account, objective_id, value, completed_count, seq, created_at, \
                 updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .t(account.to_string())
            .t(objective_id)
            .i(value)
            .i(completed)
            .i(seq)
            .t(now)
            .t(now)
            .run(conn)?;
            Ok(ProgressionObjectiveRecord {
                account: account.clone(),
                objective_id: objective_id.to_owned(),
                value,
                completed_count: completed,
                updated_at: now.to_owned(),
            })
        }
    }
}

impl ProgressionTrackRepository for SqlProgression {
    fn get_all_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Vec<ProgressionTrackRecord>> {
        sql("SELECT * FROM progression_tracks WHERE account = ? AND deleted_at IS NULL ORDER BY seq ASC")
            .t(account.to_string())
            .load::<TrackRow>(conn)?
            .into_iter()
            .map(TrackRow::decode)
            .collect()
    }

    fn get(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        id: &str,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        track(conn, account, id)
    }

    fn increment(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        amount: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        let now = now_iso();
        let conn = tx.conn();
        let base = upsert_base(conn, account, id, &now)?;
        set_track_column(
            conn,
            account,
            id,
            "progress",
            base.progress.saturating_add(amount),
            &now,
        )
    }

    fn set_progress_if_greater(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        value: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        let now = now_iso();
        let conn = tx.conn();
        let base = upsert_base(conn, account, id, &now)?;
        set_track_column(conn, account, id, "progress", base.progress.max(value), &now)
    }

    fn set_confirmed_fremium_rank(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        rank: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        let now = now_iso();
        let conn = tx.conn();
        let base = upsert_base(conn, account, id, &now)?;
        let rank = base.confirmed_fremium_rank.max(rank);
        set_track_column(conn, account, id, "confirmed_fremium_rank", rank, &now)
    }

    fn append_objective_event(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        raw_body: &str,
        received_at: &str,
    ) -> RepoResult<()> {
        // System randomness: a seeded id here would shift a replayed request's later ids (spec §16 item 11)
        sql("INSERT INTO progression_objective_events (id, account, raw_body, received_at) VALUES (?, ?, ?, ?)")
            .t(rustic_isles::replay::random_uuid())
            .t(account.to_string())
            .t(raw_body)
            .t(received_at)
            .run(tx.conn())?;
        Ok(())
    }

    fn get_all_objectives_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Vec<ProgressionObjectiveRecord>> {
        sql("SELECT * FROM progression_objectives WHERE account = ? AND deleted_at IS NULL ORDER BY seq ASC")
            .t(account.to_string())
            .load::<ObjectiveRow>(conn)?
            .into_iter()
            .map(ObjectiveRow::decode)
            .collect()
    }

    fn set_objective_if_greater(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        objective_id: &str,
        value: i64,
        completed: i64,
    ) -> RepoResult<ProgressionObjectiveRecord> {
        merge_objective(tx.conn(), account, objective_id, value, completed, &now_iso())
    }

    fn merge_objectives(&self, tx: &mut Tx<'_>, account: &AccountId, rows: &[(String, i64, i64)]) -> RepoResult<()> {
        let now = now_iso();
        let conn = tx.conn();
        for (id, value, completed) in rows {
            merge_objective(conn, account, id, *value, *completed, &now)?;
        }
        Ok(())
    }

    fn get_objectives(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        ids: &[String],
    ) -> RepoResult<Vec<ProgressionObjectiveRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; ids.len()].join(", ");
        let mut query = sql(format!(
            "SELECT * FROM progression_objectives WHERE account = ? AND deleted_at IS NULL AND objective_id IN ({marks})"
        ))
        .t(account.to_string());
        for id in ids {
            query = query.t(&**id);
        }
        query
            .load::<ObjectiveRow>(conn)?
            .into_iter()
            .map(ObjectiveRow::decode)
            .collect()
    }

    fn begin_reforge(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        expected_progress: i64,
        expected_version: i64,
        reforge: &WeaponReforge,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        let conn = tx.conn();
        let Some(current) = track(conn, account, id)? else {
            return Ok(None);
        };
        if current.progress != expected_progress
            || current.update_version != expected_version
            || current.reforge.as_ref().is_some_and(|r| r.open)
        {
            return Ok(None);
        }
        sql(format!(
            "UPDATE progression_tracks SET progress = 0, confirmed_fremium_rank = 0, confirmed_premium_rank = 0, \
             reforge = ?, updated_at = ?, update_version = update_version + 1 {TRACK_KEY}"
        ))
        .t(reforge.to_json().to_string())
        .t(&*reforge.opened_at)
        .t(account.to_string())
        .t(id)
        .run(conn)?;
        reread(conn, account, id).map(Some)
    }

    fn update_reforge(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        reforge_id: &str,
        expected: &Json,
        set: &Json,
    ) -> RepoResult<bool> {
        let conn = tx.conn();
        let Some(row) = track_row(conn, account, id)? else {
            return Ok(false);
        };
        let Some(Ok(Value::Object(mut stored))) = row.reforge.as_deref().map(serde_json::from_str::<Value>) else {
            return Ok(false);
        };
        if stored.get("reforgeId").and_then(Value::as_str) != Some(reforge_id) {
            return Ok(false);
        }
        if let Value::Object(conditions) = expected {
            // An absent key equals null (Mongo's `$eq: undefined`)
            let met = conditions
                .iter()
                .all(|(key, want)| stored.get(key).unwrap_or(&Value::Null) == want);
            if !met {
                return Ok(false);
            }
        }
        let mut changed = false;
        if let Value::Object(values) = set {
            for (key, value) in values {
                if stored.get(key) != Some(value) {
                    stored.insert(key.clone(), value.clone());
                    changed = true;
                }
            }
        }
        // Mongo's modifiedCount 0
        if !changed {
            return Ok(false);
        }
        sql(format!("UPDATE progression_tracks SET reforge = ? {TRACK_KEY}"))
            .t(Value::Object(stored).to_string())
            .t(account.to_string())
            .t(id)
            .run(conn)?;
        Ok(true)
    }

    fn claim_prestige_levels(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        claimed_before: i64,
        levels: i64,
        min_progress: i64,
    ) -> RepoResult<bool> {
        let claimed = sql(format!(
            "UPDATE progression_tracks SET prestige_claimed = ?, updated_at = ? {TRACK_KEY} \
             AND progress >= ? AND COALESCE(prestige_claimed, 0) = ?"
        ))
        .i(claimed_before.saturating_add(levels))
        .t(now_iso())
        .t(account.to_string())
        .t(id)
        .i(min_progress)
        .i(claimed_before)
        .run(tx.conn())?;
        Ok(claimed == 1)
    }

    fn spend(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        amount: i64,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        let now = now_iso();
        let conn = tx.conn();
        match track_row(conn, account, id)? {
            Some(row) if row.progress >= amount => {
                set_track_column(conn, account, id, "progress", row.progress - amount, &now).map(Some)
            }
            _ => Ok(None),
        }
    }
}

// ------------------------------------------------------------------ grants

#[derive(QueryableByName)]
struct GrantRow {
    #[diesel(sql_type = Text)]
    grant_id: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    progression_id: String,
    #[diesel(sql_type = BigInt)]
    amount: i64,
    #[diesel(sql_type = Text)]
    request_hash: String,
    #[diesel(sql_type = Text)]
    result: String,
    #[diesel(sql_type = Text)]
    created_at: String,
}

/// The stored result: the track under Node's names, the account's global id as `userId`.
fn grant_result_json(track: &ProgressionTrackRecord) -> String {
    let mut out = Map::new();
    out.insert("userId".into(), track.account.to_string().into());
    out.insert("progressionId".into(), track.progression_id.clone().into());
    out.insert("progress".into(), track.progress.into());
    out.insert("confirmedFremiumRank".into(), track.confirmed_fremium_rank.into());
    out.insert("confirmedPremiumRank".into(), track.confirmed_premium_rank.into());
    out.insert("updateVersion".into(), track.update_version.into());
    out.insert("createdAt".into(), track.created_at.clone().into());
    out.insert("updatedAt".into(), track.updated_at.clone().into());
    Value::Object(out).to_string()
}

pub(crate) fn grant_result(text: &str) -> RepoResult<ProgressionTrackRecord> {
    let not_a_track = || RepoError::data("progression_grants.result is not a track");
    let value: Value = serde_json::from_str(text).map_err(|_| not_a_track())?;
    let text_of = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let int = |key: &str| value.get(key).and_then(Value::as_i64);
    (|| {
        Some(ProgressionTrackRecord {
            account: text_of("userId")?.parse().ok()?,
            progression_id: text_of("progressionId")?,
            progress: int("progress")?,
            confirmed_fremium_rank: int("confirmedFremiumRank")?,
            confirmed_premium_rank: int("confirmedPremiumRank")?,
            update_version: int("updateVersion")?,
            created_at: text_of("createdAt")?,
            updated_at: text_of("updatedAt")?,
            reforge: None,
            prestige_claimed: None,
        })
    })()
    .ok_or_else(not_a_track)
}

impl GrantRow {
    fn decode(self) -> RepoResult<ProgressionGrantRecord> {
        Ok(ProgressionGrantRecord {
            account: account_id(&self.account, "progression_grants.account")?,
            result: grant_result(&self.result)?,
            grant_id: self.grant_id,
            progression_id: self.progression_id,
            amount: self.amount,
            request_hash: self.request_hash,
            created_at: self.created_at,
        })
    }
}

impl ProgressionGrantRepository for SqlProgression {
    fn find_by_grant_id(&self, conn: &mut AnyConnection, grant_id: &str) -> RepoResult<Option<ProgressionGrantRecord>> {
        sql("SELECT * FROM progression_grants WHERE grant_id = ?")
            .t(grant_id)
            .first::<GrantRow>(conn)?
            .map(GrantRow::decode)
            .transpose()
    }

    fn insert_applied(&self, tx: &mut Tx<'_>, r: &ProgressionGrantRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO progression_grants (grant_id, account, progression_id, amount, request_hash, status, result, \
             created_at) VALUES (?, ?, ?, ?, ?, 'applied', ?, ?)",
        )
        .t(&*r.grant_id)
        .t(r.account.to_string())
        .t(&*r.progression_id)
        .i(r.amount)
        .t(&*r.request_hash)
        .t(grant_result_json(&r.result))
        .t(&*r.created_at)
        .run(tx.conn())?;
        Ok(())
    }

    fn list_recent_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<ProgressionGrantRecord>> {
        sql(format!(
            "SELECT * FROM progression_grants WHERE account = ? ORDER BY created_at DESC LIMIT {limit}"
        ))
        .t(account.to_string())
        .load::<GrantRow>(conn)?
        .into_iter()
        .map(GrantRow::decode)
        .collect()
    }
}

// ------------------------------------------------------------------ escalation

#[derive(QueryableByName)]
struct EscalationRow {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    season_id: String,
    #[diesel(sql_type = BigInt)]
    escalation_level: i64,
    #[diesel(sql_type = BigInt)]
    next_level_xp: i64,
    #[diesel(sql_type = Text)]
    talents_progress: String,
    #[diesel(sql_type = Text)]
    unlock_progress: String,
    #[diesel(sql_type = BigInt)]
    update_version: i64,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

fn json_list(text: &str, column: &str) -> RepoResult<Vec<Json>> {
    serde_json::from_str::<Vec<Json>>(text).map_err(|e| RepoError::data(format!("{column}: {e}")))
}

impl EscalationRow {
    fn decode(self) -> RepoResult<EscalationProgressRecord> {
        Ok(EscalationProgressRecord {
            account: account_id(&self.account, "escalation_progress.account")?,
            talents_progress: json_list(&self.talents_progress, "talents_progress")?,
            unlock_progress: json_list(&self.unlock_progress, "unlock_progress")?,
            season_id: self.season_id,
            escalation_level: self.escalation_level,
            next_level_xp: self.next_level_xp,
            update_version: self.update_version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn escalation_row(conn: &mut AnyConnection, account: &AccountId, season_id: &str) -> RepoResult<Option<EscalationRow>> {
    sql("SELECT * FROM escalation_progress WHERE account = ? AND season_id = ? AND deleted_at IS NULL")
        .t(account.to_string())
        .t(season_id)
        .first::<EscalationRow>(conn)
}

impl EscalationProgressRepository for SqlProgression {
    fn get(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        season_id: &str,
    ) -> RepoResult<Option<EscalationProgressRecord>> {
        escalation_row(conn, account, season_id)?
            .map(EscalationRow::decode)
            .transpose()
    }

    fn save_if_version(
        &self,
        tx: &mut Tx<'_>,
        r: &EscalationProgressRecord,
        expected: Option<i64>,
    ) -> RepoResult<Option<EscalationProgressRecord>> {
        let conn = tx.conn();
        let live = escalation_row(conn, &r.account, &r.season_id)?;
        let talents = Value::Array(r.talents_progress.clone()).to_string();
        let unlocks = Value::Array(r.unlock_progress.clone()).to_string();
        match (expected, live) {
            (None, None) => {
                sql(
                    "INSERT INTO escalation_progress (account, season_id, escalation_level, next_level_xp, \
                     talents_progress, unlock_progress, update_version, created_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .t(r.account.to_string())
                .t(&*r.season_id)
                .i(r.escalation_level)
                .i(r.next_level_xp)
                .t(talents)
                .t(unlocks)
                .i(r.update_version)
                .t(&*r.created_at)
                .t(&*r.updated_at)
                .run(conn)?;
                Ok(Some(r.clone()))
            }
            (Some(version), Some(row)) if row.update_version == version => {
                sql(
                    "UPDATE escalation_progress SET escalation_level = ?, next_level_xp = ?, talents_progress = ?, \
                     unlock_progress = ?, update_version = ?, created_at = ?, updated_at = ? \
                     WHERE account = ? AND season_id = ? AND deleted_at IS NULL AND update_version = ?",
                )
                .i(r.escalation_level)
                .i(r.next_level_xp)
                .t(talents)
                .t(unlocks)
                .i(r.update_version)
                .t(&*r.created_at)
                .t(&*r.updated_at)
                .t(r.account.to_string())
                .t(&*r.season_id)
                .i(version)
                .run(conn)?;
                Ok(Some(r.clone()))
            }
            _ => Ok(None),
        }
    }
}

// ------------------------------------------------------------------ journey

#[derive(QueryableByName)]
struct JourneyRow {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    nodes: String,
    #[diesel(sql_type = BigInt)]
    update_version: i64,
    #[diesel(sql_type = BigInt)]
    revision: i64,
}

impl PlayerJourneyRepository for SqlProgression {
    fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PlayerJourneyRecord>> {
        let Some(row) = sql("SELECT * FROM player_journey WHERE account = ? AND deleted_at IS NULL")
            .t(account.to_string())
            .first::<JourneyRow>(conn)?
        else {
            return Ok(None);
        };
        Ok(Some(PlayerJourneyRecord {
            account: account_id(&row.account, "player_journey.account")?,
            nodes: row.nodes,
            update_version: row.update_version,
            revision: row.revision,
        }))
    }

    fn create_if_absent(&self, tx: &mut Tx<'_>, r: &PlayerJourneyRecord) -> RepoResult<bool> {
        let conn = tx.conn();
        let any = sql("SELECT CAST(1 AS BIGINT) AS n FROM player_journey WHERE account = ?")
            .t(r.account.to_string())
            .exists(conn)?;
        if any {
            return Ok(false);
        }
        sql("INSERT INTO player_journey (account, nodes, update_version, revision) VALUES (?, ?, ?, ?)")
            .t(r.account.to_string())
            .t(&*r.nodes)
            .i(r.update_version)
            .i(r.revision)
            .run(conn)?;
        Ok(true)
    }

    fn update_if_revision(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        nodes: &str,
        update_version: i64,
        expected: i64,
    ) -> RepoResult<bool> {
        let changed = sql("UPDATE player_journey SET nodes = ?, update_version = ?, revision = ? \
             WHERE account = ? AND deleted_at IS NULL AND revision = ?")
        .t(nodes)
        .i(update_version)
        .i(expected.saturating_add(1))
        .t(account.to_string())
        .i(expected)
        .run(tx.conn())?;
        Ok(changed == 1)
    }
}

// ------------------------------------------------------------------ breadcrumbs, encountered content

#[derive(QueryableByName)]
struct BreadcrumbRow {
    #[diesel(sql_type = Text)]
    character_id: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    items: String,
    #[diesel(sql_type = BigInt)]
    update_version: i64,
}

impl BreadcrumbRepository for SqlProgression {
    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<BreadcrumbsRecord>> {
        let Some(row) = sql("SELECT * FROM breadcrumbs WHERE character_id = ? AND account = ? AND deleted_at IS NULL")
            .t(character_id)
            .t(account.to_string())
            .first::<BreadcrumbRow>(conn)?
        else {
            return Ok(None);
        };
        Ok(Some(BreadcrumbsRecord {
            account: account_id(&row.account, "breadcrumbs.account")?,
            character_id: row.character_id,
            breadcrumbs: row.items,
            update_version: row.update_version,
        }))
    }

    fn create(&self, tx: &mut Tx<'_>, r: &BreadcrumbsRecord) -> RepoResult<()> {
        sql("INSERT INTO breadcrumbs (character_id, account, items, update_version) VALUES (?, ?, ?, ?)")
            .t(&*r.character_id)
            .t(r.account.to_string())
            .t(&*r.breadcrumbs)
            .i(r.update_version)
            .run(tx.conn())?;
        Ok(())
    }

    fn update(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        json: &str,
        update_version: i64,
    ) -> RepoResult<()> {
        sql("UPDATE breadcrumbs SET items = ?, update_version = ? WHERE character_id = ? AND account = ?")
            .t(json)
            .i(update_version)
            .t(character_id)
            .t(account.to_string())
            .run(tx.conn())?;
        Ok(())
    }
}

#[derive(QueryableByName)]
struct EncounteredRow {
    #[diesel(sql_type = Text)]
    character_id: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    content: String,
}

impl EncounteredContentRepository for SqlProgression {
    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<EncounteredContentRecord>> {
        let Some(row) =
            sql("SELECT * FROM encountered_content WHERE character_id = ? AND account = ? AND deleted_at IS NULL")
                .t(character_id)
                .t(account.to_string())
                .first::<EncounteredRow>(conn)?
        else {
            return Ok(None);
        };
        Ok(Some(EncounteredContentRecord {
            account: account_id(&row.account, "encountered_content.account")?,
            character_id: row.character_id,
            encountered_content: row.content,
        }))
    }

    fn create(&self, tx: &mut Tx<'_>, r: &EncounteredContentRecord) -> RepoResult<()> {
        sql("INSERT INTO encountered_content (character_id, account, content) VALUES (?, ?, ?)")
            .t(&*r.character_id)
            .t(r.account.to_string())
            .t(&*r.encountered_content)
            .run(tx.conn())?;
        Ok(())
    }

    fn update_content(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId, json: &str) -> RepoResult<()> {
        sql("UPDATE encountered_content SET content = ? WHERE character_id = ? AND account = ?")
            .t(json)
            .t(character_id)
            .t(account.to_string())
            .run(tx.conn())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reforge_round_trips_and_refuses_bad_shapes() {
        let reforge = WeaponReforge {
            reforge_id: "r1".into(),
            open: true,
            prior_progress: 10,
            cap: 20,
            banked: 3,
            banked_restored: 0,
            prestige_granted: false,
            opened_at: "2026-10-27T00:00:00.000Z".into(),
            closed_at: None,
        };
        let value = reforge.to_json();
        assert_eq!(
            value.to_string(),
            r#"{"reforgeId":"r1","status":"open","priorProgress":10,"cap":20,"banked":3,"bankedRestored":0,"prestigeGranted":false,"openedAt":"2026-10-27T00:00:00.000Z"}"#
        );
        assert_eq!(WeaponReforge::from_json(&value), Some(reforge));
        assert_eq!(WeaponReforge::from_json(&json!({"reforgeId": "r1"})), None);
        let mut wrong = value.clone();
        wrong["cap"] = json!("20");
        assert_eq!(WeaponReforge::from_json(&wrong), None);
    }

    #[test]
    fn grant_results_need_every_track_field() {
        assert!(grant_result("{}").is_err());
        assert_eq!(
            grant_result("nope").unwrap_err().to_string(),
            "stored data: progression_grants.result is not a track"
        );
    }
}
