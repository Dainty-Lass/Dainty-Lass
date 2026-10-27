//! The SQL half of the progression repositories: `store`'s two engines through Diesel's DSL, one set of queries for
//! both. `Database::write` runs the writers one at a time, so a read-modify-write inside a [`Tx`] is as atomic as
//! Mongo's `$inc` and `$max` were.

use diesel::prelude::*;

use super::{
    BreadcrumbRepository, BreadcrumbsRecord, EncounteredContentRecord, EncounteredContentRepository,
    EscalationProgressRecord, EscalationProgressRepository, PlayerJourneyRecord, PlayerJourneyRepository,
    ProgressionGrantRecord, ProgressionGrantRepository, ProgressionObjectiveRecord, ProgressionTrackRecord,
    ProgressionTrackRepository, WeaponReforge,
};
use crate::repo::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx};

mod schema {
    diesel::table! {
        progression_tracks (account, progression_id) {
            account -> Text,
            progression_id -> Text,
            progress -> BigInt,
            confirmed_fremium_rank -> BigInt,
            confirmed_premium_rank -> BigInt,
            update_version -> BigInt,
            seq -> BigInt,
            created_at -> Text,
            updated_at -> Text,
            reforge -> Nullable<Text>,
            prestige_claimed -> Nullable<BigInt>,
            deleted_at -> Nullable<BigInt>,
        }
    }
    diesel::table! {
        progression_objectives (account, objective_id) {
            account -> Text,
            objective_id -> Text,
            value -> BigInt,
            completed_count -> BigInt,
            seq -> BigInt,
            created_at -> Text,
            updated_at -> Text,
            deleted_at -> Nullable<BigInt>,
        }
    }
    diesel::table! {
        progression_objective_events (id) {
            id -> Text,
            account -> Text,
            raw_body -> Text,
            received_at -> Text,
        }
    }
    diesel::table! {
        progression_grants (grant_id) {
            grant_id -> Text,
            account -> Text,
            progression_id -> Text,
            amount -> BigInt,
            request_hash -> Text,
            status -> Text,
            result -> Text,
            created_at -> Text,
        }
    }
    diesel::table! {
        escalation_progress (account, season_id) {
            account -> Text,
            season_id -> Text,
            escalation_level -> BigInt,
            next_level_xp -> BigInt,
            talents_progress -> Text,
            unlock_progress -> Text,
            update_version -> BigInt,
            created_at -> Text,
            updated_at -> Text,
            deleted_at -> Nullable<BigInt>,
        }
    }
    diesel::table! {
        player_journey (account) {
            account -> Text,
            nodes -> Text,
            update_version -> BigInt,
            revision -> BigInt,
            deleted_at -> Nullable<BigInt>,
        }
    }
    diesel::table! {
        breadcrumbs (character_id) {
            character_id -> Text,
            account -> Text,
            items -> Text,
            update_version -> BigInt,
            deleted_at -> Nullable<BigInt>,
        }
    }
    diesel::table! {
        encountered_content (character_id) {
            character_id -> Text,
            account -> Text,
            content -> Text,
            deleted_at -> Nullable<BigInt>,
        }
    }
}

use schema::{
    breadcrumbs, encountered_content, escalation_progress, player_journey, progression_grants,
    progression_objective_events, progression_objectives, progression_tracks,
};

/// `new Date().toISOString()`.
pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn data<T, E: std::fmt::Display>(what: &str, result: Result<T, E>) -> RepoResult<T> {
    result.map_err(|error| RepoError::Data(format!("{what}: {error}")))
}

fn account_of(text: &str) -> RepoResult<AccountId> {
    data("account id", text.parse::<AccountId>())
}

impl WeaponReforge {
    /// Node's `WeaponReforgeState`: `status` is `open` or `closed`, `closedAt` absent while open.
    pub fn to_json(&self) -> Json {
        let mut object = serde_json::Map::new();
        object.insert("reforgeId".into(), self.reforge_id.clone().into());
        object.insert("status".into(), (if self.open { "open" } else { "closed" }).into());
        object.insert("priorProgress".into(), self.prior_progress.into());
        object.insert("cap".into(), self.cap.into());
        object.insert("banked".into(), self.banked.into());
        object.insert("bankedRestored".into(), self.banked_restored.into());
        object.insert("prestigeGranted".into(), self.prestige_granted.into());
        object.insert("openedAt".into(), self.opened_at.clone().into());
        if let Some(closed) = &self.closed_at {
            object.insert("closedAt".into(), closed.clone().into());
        }
        Json::Object(object)
    }

    pub fn from_json(value: &Json) -> Option<Self> {
        let text = |key: &str| value.get(key)?.as_str().map(str::to_owned);
        let int = |key: &str| value.get(key)?.as_i64();
        Some(Self {
            reforge_id: text("reforgeId")?,
            open: value.get("status")?.as_str()? == "open",
            prior_progress: int("priorProgress")?,
            cap: int("cap")?,
            banked: int("banked")?,
            banked_restored: int("bankedRestored")?,
            prestige_granted: value.get("prestigeGranted")?.as_bool()?,
            opened_at: text("openedAt")?,
            closed_at: text("closedAt"),
        })
    }
}

type TrackRow = (
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    String,
    String,
    Option<String>,
    Option<i64>,
);

macro_rules! track_columns {
    () => {
        (
            progression_tracks::account,
            progression_tracks::progression_id,
            progression_tracks::progress,
            progression_tracks::confirmed_fremium_rank,
            progression_tracks::confirmed_premium_rank,
            progression_tracks::update_version,
            progression_tracks::created_at,
            progression_tracks::updated_at,
            progression_tracks::reforge,
            progression_tracks::prestige_claimed,
        )
    };
}

fn track(row: TrackRow) -> RepoResult<ProgressionTrackRecord> {
    let (account, progression_id, progress, fremium, premium, version, created_at, updated_at, reforge, claimed) = row;
    let reforge = match reforge {
        None => None,
        Some(text) => WeaponReforge::from_json(&data("reforge", serde_json::from_str::<Json>(&text))?),
    };
    Ok(ProgressionTrackRecord {
        account: account_of(&account)?,
        progression_id,
        progress,
        confirmed_fremium_rank: fremium,
        confirmed_premium_rank: premium,
        update_version: version,
        created_at,
        updated_at,
        reforge,
        prestige_claimed: claimed,
    })
}

fn find_track(
    conn: &mut AnyConnection,
    account: &AccountId,
    progression_id: &str,
) -> RepoResult<Option<ProgressionTrackRecord>> {
    progression_tracks::table
        .filter(progression_tracks::account.eq(account.to_string()))
        .filter(progression_tracks::progression_id.eq(progression_id))
        .filter(progression_tracks::deleted_at.is_null())
        .select(track_columns!())
        .first::<TrackRow>(conn)
        .optional()?
        .map(track)
        .transpose()
}

/// The row, or a fresh one inserted with Node's upsert defaults; `progress` is the value a new row starts at.
fn upsert_base(
    tx: &mut Tx<'_>,
    account: &AccountId,
    progression_id: &str,
    progress: i64,
    now: &str,
) -> RepoResult<ProgressionTrackRecord> {
    if let Some(row) = find_track(tx.conn(), account, progression_id)? {
        return Ok(row);
    }
    let next_seq = progression_tracks::table
        .filter(progression_tracks::account.eq(account.to_string()))
        .select(diesel::dsl::max(progression_tracks::seq))
        .first::<Option<i64>>(tx.conn())?
        .unwrap_or(0)
        + 1;
    diesel::insert_into(progression_tracks::table)
        .values((
            progression_tracks::account.eq(account.to_string()),
            progression_tracks::progression_id.eq(progression_id),
            progression_tracks::progress.eq(progress),
            progression_tracks::confirmed_fremium_rank.eq(0_i64),
            progression_tracks::confirmed_premium_rank.eq(0_i64),
            progression_tracks::update_version.eq(0_i64),
            progression_tracks::seq.eq(next_seq),
            progression_tracks::created_at.eq(now),
            progression_tracks::updated_at.eq(now),
        ))
        .execute(tx.conn())?;
    Ok(ProgressionTrackRecord {
        account: account.clone(),
        progression_id: progression_id.to_owned(),
        progress,
        confirmed_fremium_rank: 0,
        confirmed_premium_rank: 0,
        update_version: 0,
        created_at: now.to_owned(),
        updated_at: now.to_owned(),
        reforge: None,
        prestige_claimed: None,
    })
}

fn require_track(tx: &mut Tx<'_>, account: &AccountId, progression_id: &str) -> RepoResult<ProgressionTrackRecord> {
    find_track(tx.conn(), account, progression_id)?
        .ok_or_else(|| RepoError::Data(format!("progression track {progression_id} vanished inside its write")))
}

macro_rules! track_row {
    ($account:expr, $progression_id:expr) => {
        progression_tracks::table
            .filter(progression_tracks::account.eq($account.to_string()))
            .filter(progression_tracks::progression_id.eq($progression_id))
    };
}

/// The progression, mastery, escalation, Slayer's Path, breadcrumb and encountered-content repositories on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlProgression;

impl ProgressionTrackRepository for SqlProgression {
    fn get_all_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Vec<ProgressionTrackRecord>> {
        progression_tracks::table
            .filter(progression_tracks::account.eq(account.to_string()))
            .filter(progression_tracks::deleted_at.is_null())
            // Mongo's natural order was the order rows were made in
            .order(progression_tracks::seq.asc())
            .select(track_columns!())
            .load::<TrackRow>(conn)?
            .into_iter()
            .map(track)
            .collect()
    }

    fn get(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        progression_id: &str,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        find_track(conn, account, progression_id)
    }

    fn increment(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        progression_id: &str,
        amount: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        let now = super::now_iso();
        let base = upsert_base(tx, account, progression_id, 0, &now)?;
        diesel::update(track_row!(account, progression_id))
            .set((
                progression_tracks::progress.eq(base.progress + amount),
                progression_tracks::update_version.eq(base.update_version + 1),
                progression_tracks::updated_at.eq(&now),
            ))
            .execute(tx.conn())?;
        require_track(tx, account, progression_id)
    }

    fn set_progress_if_greater(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        progression_id: &str,
        value: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        let now = super::now_iso();
        let base = upsert_base(tx, account, progression_id, 0, &now)?;
        diesel::update(track_row!(account, progression_id))
            .set((
                progression_tracks::progress.eq(base.progress.max(value)),
                progression_tracks::update_version.eq(base.update_version + 1),
                progression_tracks::updated_at.eq(&now),
            ))
            .execute(tx.conn())?;
        require_track(tx, account, progression_id)
    }

    fn set_confirmed_fremium_rank(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        progression_id: &str,
        rank: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        let now = super::now_iso();
        let base = upsert_base(tx, account, progression_id, 0, &now)?;
        diesel::update(track_row!(account, progression_id))
            .set((
                progression_tracks::confirmed_fremium_rank.eq(base.confirmed_fremium_rank.max(rank)),
                progression_tracks::update_version.eq(base.update_version + 1),
                progression_tracks::updated_at.eq(&now),
            ))
            .execute(tx.conn())?;
        require_track(tx, account, progression_id)
    }

    fn append_objective_event(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        raw_body: &str,
        received_at: &str,
    ) -> RepoResult<()> {
        diesel::insert_into(progression_objective_events::table)
            .values((
                progression_objective_events::id.eq(uuid::Uuid::new_v4().to_string()),
                progression_objective_events::account.eq(account.to_string()),
                progression_objective_events::raw_body.eq(raw_body),
                progression_objective_events::received_at.eq(received_at),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn get_all_objectives_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Vec<ProgressionObjectiveRecord>> {
        let rows = progression_objectives::table
            .filter(progression_objectives::account.eq(account.to_string()))
            .filter(progression_objectives::deleted_at.is_null())
            .order(progression_objectives::seq.asc())
            .select((
                progression_objectives::objective_id,
                progression_objectives::value,
                progression_objectives::completed_count,
                progression_objectives::updated_at,
            ))
            .load::<(String, i64, i64, String)>(conn)?;
        Ok(objectives(account, rows))
    }

    fn set_objective_if_greater(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        objective_id: &str,
        value: i64,
        completed_count: i64,
    ) -> RepoResult<ProgressionObjectiveRecord> {
        let now = super::now_iso();
        merge_objective(tx, account, objective_id, value, completed_count, &now)
    }

    fn begin_reforge(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        progression_id: &str,
        expected_progress: i64,
        expected_version: i64,
        reforge: &WeaponReforge,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        let Some(row) = find_track(tx.conn(), account, progression_id)? else {
            return Ok(None);
        };
        if row.progress != expected_progress
            || row.update_version != expected_version
            || row.reforge.as_ref().is_some_and(|r| r.open)
        {
            return Ok(None);
        }
        diesel::update(track_row!(account, progression_id))
            .set((
                progression_tracks::progress.eq(0_i64),
                progression_tracks::confirmed_fremium_rank.eq(0_i64),
                progression_tracks::confirmed_premium_rank.eq(0_i64),
                progression_tracks::reforge.eq(Some(reforge.to_json().to_string())),
                progression_tracks::updated_at.eq(&reforge.opened_at),
                progression_tracks::update_version.eq(row.update_version + 1),
            ))
            .execute(tx.conn())?;
        find_track(tx.conn(), account, progression_id)
    }

    fn update_reforge(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        progression_id: &str,
        reforge_id: &str,
        expected: &Json,
        set: &Json,
    ) -> RepoResult<bool> {
        let stored: Option<Option<String>> = track_row!(account, progression_id)
            .filter(progression_tracks::deleted_at.is_null())
            .select(progression_tracks::reforge)
            .first(tx.conn())
            .optional()?;
        let Some(Some(text)) = stored else {
            return Ok(false);
        };
        let mut current = data("reforge", serde_json::from_str::<Json>(&text))?;
        if current.get("reforgeId").and_then(Json::as_str) != Some(reforge_id) {
            return Ok(false);
        }
        // `{ $eq: undefined }` matches an absent or null field
        for (key, wanted) in expected.as_object().into_iter().flatten() {
            let held = current.get(key).unwrap_or(&Json::Null);
            if held != wanted {
                return Ok(false);
            }
        }
        let mut modified = false;
        if let Some(object) = current.as_object_mut() {
            for (key, value) in set.as_object().into_iter().flatten() {
                if object.get(key) != Some(value) {
                    modified = true;
                }
                object.insert(key.clone(), value.clone());
            }
        }
        // A $set that changes nothing is not a modification (modifiedCount is 0 in MongoDB)
        if !modified {
            return Ok(false);
        }
        diesel::update(track_row!(account, progression_id))
            .set(progression_tracks::reforge.eq(Some(current.to_string())))
            .execute(tx.conn())?;
        Ok(true)
    }

    fn claim_prestige_levels(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        progression_id: &str,
        claimed_before: i64,
        levels: i64,
        min_progress: i64,
    ) -> RepoResult<bool> {
        let Some(row) = find_track(tx.conn(), account, progression_id)? else {
            return Ok(false);
        };
        if row.progress < min_progress || row.prestige_claimed.unwrap_or(0) != claimed_before {
            return Ok(false);
        }
        diesel::update(track_row!(account, progression_id))
            .set((
                progression_tracks::prestige_claimed.eq(Some(row.prestige_claimed.unwrap_or(0) + levels)),
                progression_tracks::updated_at.eq(super::now_iso()),
            ))
            .execute(tx.conn())?;
        Ok(true)
    }

    fn get_objectives(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        objective_ids: &[String],
    ) -> RepoResult<Vec<ProgressionObjectiveRecord>> {
        if objective_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = progression_objectives::table
            .filter(progression_objectives::account.eq(account.to_string()))
            .filter(progression_objectives::deleted_at.is_null())
            .filter(progression_objectives::objective_id.eq_any(objective_ids))
            .select((
                progression_objectives::objective_id,
                progression_objectives::value,
                progression_objectives::completed_count,
                progression_objectives::updated_at,
            ))
            .load::<(String, i64, i64, String)>(conn)?;
        Ok(objectives(account, rows))
    }

    fn merge_objectives(&self, tx: &mut Tx<'_>, account: &AccountId, rows: &[(String, i64, i64)]) -> RepoResult<()> {
        let now = super::now_iso();
        for (objective_id, value, completed_count) in rows {
            merge_objective(tx, account, objective_id, *value, *completed_count, &now)?;
        }
        Ok(())
    }

    fn spend(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        progression_id: &str,
        amount: i64,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        let Some(row) = find_track(tx.conn(), account, progression_id)? else {
            return Ok(None);
        };
        if row.progress < amount {
            return Ok(None);
        }
        diesel::update(track_row!(account, progression_id))
            .set((
                progression_tracks::progress.eq(row.progress - amount),
                progression_tracks::update_version.eq(row.update_version + 1),
                progression_tracks::updated_at.eq(super::now_iso()),
            ))
            .execute(tx.conn())?;
        find_track(tx.conn(), account, progression_id)
    }
}

fn objectives(account: &AccountId, rows: Vec<(String, i64, i64, String)>) -> Vec<ProgressionObjectiveRecord> {
    rows.into_iter()
        .map(
            |(objective_id, value, completed_count, updated_at)| ProgressionObjectiveRecord {
                account: account.clone(),
                objective_id,
                value,
                completed_count,
                updated_at,
            },
        )
        .collect()
}

fn merge_objective(
    tx: &mut Tx<'_>,
    account: &AccountId,
    objective_id: &str,
    value: i64,
    completed_count: i64,
    now: &str,
) -> RepoResult<ProgressionObjectiveRecord> {
    let existing: Option<(i64, i64)> = progression_objectives::table
        .filter(progression_objectives::account.eq(account.to_string()))
        .filter(progression_objectives::objective_id.eq(objective_id))
        .filter(progression_objectives::deleted_at.is_null())
        .select((progression_objectives::value, progression_objectives::completed_count))
        .first(tx.conn())
        .optional()?;
    let (next_value, next_count) = match existing {
        Some((held_value, held_count)) => (held_value.max(value), held_count.max(completed_count)),
        None => (value, completed_count),
    };
    if existing.is_some() {
        diesel::update(
            progression_objectives::table
                .filter(progression_objectives::account.eq(account.to_string()))
                .filter(progression_objectives::objective_id.eq(objective_id)),
        )
        .set((
            progression_objectives::value.eq(next_value),
            progression_objectives::completed_count.eq(next_count),
            progression_objectives::updated_at.eq(now),
        ))
        .execute(tx.conn())?;
    } else {
        let next_seq = progression_objectives::table
            .filter(progression_objectives::account.eq(account.to_string()))
            .select(diesel::dsl::max(progression_objectives::seq))
            .first::<Option<i64>>(tx.conn())?
            .unwrap_or(0)
            + 1;
        diesel::insert_into(progression_objectives::table)
            .values((
                progression_objectives::account.eq(account.to_string()),
                progression_objectives::objective_id.eq(objective_id),
                progression_objectives::value.eq(next_value),
                progression_objectives::completed_count.eq(next_count),
                progression_objectives::seq.eq(next_seq),
                progression_objectives::created_at.eq(now),
                progression_objectives::updated_at.eq(now),
            ))
            .execute(tx.conn())?;
    }
    Ok(ProgressionObjectiveRecord {
        account: account.clone(),
        objective_id: objective_id.to_owned(),
        value: next_value,
        completed_count: next_count,
        updated_at: now.to_owned(),
    })
}

fn ledger_json(track: &ProgressionTrackRecord) -> Json {
    serde_json::json!({
        "userId": track.account.to_string(),
        "progressionId": track.progression_id,
        "progress": track.progress,
        "confirmedFremiumRank": track.confirmed_fremium_rank,
        "confirmedPremiumRank": track.confirmed_premium_rank,
        "updateVersion": track.update_version,
        "createdAt": track.created_at,
        "updatedAt": track.updated_at,
    })
}

fn ledger_track(value: &Json) -> RepoResult<ProgressionTrackRecord> {
    let bad = || RepoError::Data("progression_grants.result is not a track".into());
    let text = |key: &str| value.get(key).and_then(Json::as_str).map(str::to_owned).ok_or_else(bad);
    let int = |key: &str| value.get(key).and_then(Json::as_i64).ok_or_else(bad);
    Ok(ProgressionTrackRecord {
        account: account_of(&text("userId")?)?,
        progression_id: text("progressionId")?,
        progress: int("progress")?,
        confirmed_fremium_rank: int("confirmedFremiumRank")?,
        confirmed_premium_rank: int("confirmedPremiumRank")?,
        update_version: int("updateVersion")?,
        created_at: text("createdAt")?,
        updated_at: text("updatedAt")?,
        reforge: None,
        prestige_claimed: None,
    })
}

type GrantRow = (String, String, String, i64, String, String, String);

fn grant(row: GrantRow) -> RepoResult<ProgressionGrantRecord> {
    let (grant_id, account, progression_id, amount, request_hash, result, created_at) = row;
    Ok(ProgressionGrantRecord {
        grant_id,
        account: account_of(&account)?,
        progression_id,
        amount,
        request_hash,
        result: ledger_track(&data("grant result", serde_json::from_str::<Json>(&result))?)?,
        created_at,
    })
}

macro_rules! grant_columns {
    () => {
        (
            progression_grants::grant_id,
            progression_grants::account,
            progression_grants::progression_id,
            progression_grants::amount,
            progression_grants::request_hash,
            progression_grants::result,
            progression_grants::created_at,
        )
    };
}

impl ProgressionGrantRepository for SqlProgression {
    fn find_by_grant_id(&self, conn: &mut AnyConnection, grant_id: &str) -> RepoResult<Option<ProgressionGrantRecord>> {
        progression_grants::table
            .filter(progression_grants::grant_id.eq(grant_id))
            .select(grant_columns!())
            .first::<GrantRow>(conn)
            .optional()?
            .map(grant)
            .transpose()
    }

    /// The grant id is the key: a replay's insert fails and the caller serves the stored result instead.
    fn insert_applied(&self, tx: &mut Tx<'_>, record: &ProgressionGrantRecord) -> RepoResult<()> {
        diesel::insert_into(progression_grants::table)
            .values((
                progression_grants::grant_id.eq(&record.grant_id),
                progression_grants::account.eq(record.account.to_string()),
                progression_grants::progression_id.eq(&record.progression_id),
                progression_grants::amount.eq(record.amount),
                progression_grants::request_hash.eq(&record.request_hash),
                progression_grants::status.eq("applied"),
                progression_grants::result.eq(ledger_json(&record.result).to_string()),
                progression_grants::created_at.eq(&record.created_at),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn list_recent_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<ProgressionGrantRecord>> {
        progression_grants::table
            .filter(progression_grants::account.eq(account.to_string()))
            .order(progression_grants::created_at.desc())
            .limit(i64::from(limit))
            .select(grant_columns!())
            .load::<GrantRow>(conn)?
            .into_iter()
            .map(grant)
            .collect()
    }
}

type EscalationRow = (String, String, i64, i64, String, String, i64, String, String);

macro_rules! escalation_columns {
    () => {
        (
            escalation_progress::account,
            escalation_progress::season_id,
            escalation_progress::escalation_level,
            escalation_progress::next_level_xp,
            escalation_progress::talents_progress,
            escalation_progress::unlock_progress,
            escalation_progress::update_version,
            escalation_progress::created_at,
            escalation_progress::updated_at,
        )
    };
}

fn json_list(what: &str, text: &str) -> RepoResult<Vec<Json>> {
    data(what, serde_json::from_str::<Vec<Json>>(text))
}

fn escalation(row: EscalationRow) -> RepoResult<EscalationProgressRecord> {
    let (account, season_id, level, xp, talents, unlocks, version, created_at, updated_at) = row;
    Ok(EscalationProgressRecord {
        account: account_of(&account)?,
        season_id,
        escalation_level: level,
        next_level_xp: xp,
        talents_progress: json_list("talents_progress", &talents)?,
        unlock_progress: json_list("unlock_progress", &unlocks)?,
        update_version: version,
        created_at,
        updated_at,
    })
}

fn find_escalation(
    conn: &mut AnyConnection,
    account: &AccountId,
    season_id: &str,
) -> RepoResult<Option<EscalationProgressRecord>> {
    escalation_progress::table
        .filter(escalation_progress::account.eq(account.to_string()))
        .filter(escalation_progress::season_id.eq(season_id))
        .filter(escalation_progress::deleted_at.is_null())
        .select(escalation_columns!())
        .first::<EscalationRow>(conn)
        .optional()?
        .map(escalation)
        .transpose()
}

impl EscalationProgressRepository for SqlProgression {
    fn get(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        season_id: &str,
    ) -> RepoResult<Option<EscalationProgressRecord>> {
        find_escalation(conn, account, season_id)
    }

    /// `expected_version` `None` inserts only; otherwise the row is replaced only if its version still is that.
    fn save_if_version(
        &self,
        tx: &mut Tx<'_>,
        record: &EscalationProgressRecord,
        expected_version: Option<i64>,
    ) -> RepoResult<Option<EscalationProgressRecord>> {
        let held = find_escalation(tx.conn(), &record.account, &record.season_id)?;
        let talents = Json::Array(record.talents_progress.clone()).to_string();
        let unlocks = Json::Array(record.unlock_progress.clone()).to_string();
        match (expected_version, held) {
            (None, Some(_)) => Ok(None),
            (None, None) => {
                diesel::insert_into(escalation_progress::table)
                    .values((
                        escalation_progress::account.eq(record.account.to_string()),
                        escalation_progress::season_id.eq(&record.season_id),
                        escalation_progress::escalation_level.eq(record.escalation_level),
                        escalation_progress::next_level_xp.eq(record.next_level_xp),
                        escalation_progress::talents_progress.eq(&talents),
                        escalation_progress::unlock_progress.eq(&unlocks),
                        escalation_progress::update_version.eq(record.update_version),
                        escalation_progress::created_at.eq(&record.created_at),
                        escalation_progress::updated_at.eq(&record.updated_at),
                    ))
                    .execute(tx.conn())?;
                Ok(Some(record.clone()))
            }
            (Some(_), None) => Ok(None),
            (Some(expected), Some(held)) => {
                if held.update_version != expected {
                    return Ok(None);
                }
                diesel::update(
                    escalation_progress::table
                        .filter(escalation_progress::account.eq(record.account.to_string()))
                        .filter(escalation_progress::season_id.eq(&record.season_id)),
                )
                .set((
                    escalation_progress::escalation_level.eq(record.escalation_level),
                    escalation_progress::next_level_xp.eq(record.next_level_xp),
                    escalation_progress::talents_progress.eq(&talents),
                    escalation_progress::unlock_progress.eq(&unlocks),
                    escalation_progress::update_version.eq(record.update_version),
                    escalation_progress::created_at.eq(&record.created_at),
                    escalation_progress::updated_at.eq(&record.updated_at),
                ))
                .execute(tx.conn())?;
                Ok(Some(record.clone()))
            }
        }
    }
}

impl PlayerJourneyRepository for SqlProgression {
    fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PlayerJourneyRecord>> {
        let row = player_journey::table
            .filter(player_journey::account.eq(account.to_string()))
            .filter(player_journey::deleted_at.is_null())
            .select((
                player_journey::nodes,
                player_journey::update_version,
                player_journey::revision,
            ))
            .first::<(String, i64, i64)>(conn)
            .optional()?;
        Ok(row.map(|(nodes, update_version, revision)| PlayerJourneyRecord {
            account: account.clone(),
            nodes,
            update_version,
            revision,
        }))
    }

    fn create_if_absent(&self, tx: &mut Tx<'_>, record: &PlayerJourneyRecord) -> RepoResult<bool> {
        let held: i64 = player_journey::table
            .filter(player_journey::account.eq(record.account.to_string()))
            .count()
            .get_result(tx.conn())?;
        if held > 0 {
            return Ok(false);
        }
        diesel::insert_into(player_journey::table)
            .values((
                player_journey::account.eq(record.account.to_string()),
                player_journey::nodes.eq(&record.nodes),
                player_journey::update_version.eq(record.update_version),
                player_journey::revision.eq(record.revision),
            ))
            .execute(tx.conn())?;
        Ok(true)
    }

    fn update_if_revision(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        nodes_json: &str,
        update_version: i64,
        expected_revision: i64,
    ) -> RepoResult<bool> {
        let changed = diesel::update(
            player_journey::table
                .filter(player_journey::account.eq(account.to_string()))
                .filter(player_journey::deleted_at.is_null())
                .filter(player_journey::revision.eq(expected_revision)),
        )
        .set((
            player_journey::nodes.eq(nodes_json),
            player_journey::update_version.eq(update_version),
            player_journey::revision.eq(expected_revision + 1),
        ))
        .execute(tx.conn())?;
        Ok(changed == 1)
    }
}

impl BreadcrumbRepository for SqlProgression {
    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<BreadcrumbsRecord>> {
        let row = breadcrumbs::table
            .filter(breadcrumbs::character_id.eq(character_id))
            .filter(breadcrumbs::account.eq(account.to_string()))
            .filter(breadcrumbs::deleted_at.is_null())
            .select((breadcrumbs::items, breadcrumbs::update_version))
            .first::<(String, i64)>(conn)
            .optional()?;
        Ok(row.map(|(held, update_version)| BreadcrumbsRecord {
            character_id: character_id.to_owned(),
            account: account.clone(),
            breadcrumbs: held,
            update_version,
        }))
    }

    fn create(&self, tx: &mut Tx<'_>, record: &BreadcrumbsRecord) -> RepoResult<()> {
        diesel::insert_into(breadcrumbs::table)
            .values((
                breadcrumbs::character_id.eq(&record.character_id),
                breadcrumbs::account.eq(record.account.to_string()),
                breadcrumbs::items.eq(&record.breadcrumbs),
                breadcrumbs::update_version.eq(record.update_version),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn update(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        breadcrumbs_json: &str,
        update_version: i64,
    ) -> RepoResult<()> {
        diesel::update(
            breadcrumbs::table
                .filter(breadcrumbs::character_id.eq(character_id))
                .filter(breadcrumbs::account.eq(account.to_string())),
        )
        .set((
            breadcrumbs::items.eq(breadcrumbs_json),
            breadcrumbs::update_version.eq(update_version),
        ))
        .execute(tx.conn())?;
        Ok(())
    }
}

impl EncounteredContentRepository for SqlProgression {
    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<EncounteredContentRecord>> {
        let row = encountered_content::table
            .filter(encountered_content::character_id.eq(character_id))
            .filter(encountered_content::account.eq(account.to_string()))
            .filter(encountered_content::deleted_at.is_null())
            .select(encountered_content::content)
            .first::<String>(conn)
            .optional()?;
        Ok(row.map(|content| EncounteredContentRecord {
            character_id: character_id.to_owned(),
            account: account.clone(),
            encountered_content: content,
        }))
    }

    fn create(&self, tx: &mut Tx<'_>, record: &EncounteredContentRecord) -> RepoResult<()> {
        diesel::insert_into(encountered_content::table)
            .values((
                encountered_content::character_id.eq(&record.character_id),
                encountered_content::account.eq(record.account.to_string()),
                encountered_content::content.eq(&record.encountered_content),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn update_content(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        content_json: &str,
    ) -> RepoResult<()> {
        diesel::update(
            encountered_content::table
                .filter(encountered_content::character_id.eq(character_id))
                .filter(encountered_content::account.eq(account.to_string())),
        )
        .set(encountered_content::content.eq(content_json))
        .execute(tx.conn())?;
        Ok(())
    }
}
