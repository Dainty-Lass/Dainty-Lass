//! RU5 progression, mastery, escalation and session supplies; the reforge state is inventory-reforge's to write.

use super::{AccountId, AnyConnection, Json, Tx, repository};

/// `WeaponReforgeState`.
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

/// `ProgressionTrackRecord`.
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

/// `ProgressionObjectiveRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressionObjectiveRecord {
    pub account: AccountId,
    pub objective_id: String,
    pub value: i64,
    pub completed_count: i64,
    pub updated_at: String,
}

repository! {
    pub trait ProgressionTrackRepository("progressionTracks") {
        fn get_all_for_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Vec<ProgressionTrackRecord>;
        fn get(&self, conn: &mut AnyConnection, account: &AccountId, progression_id: &str)
            -> Option<ProgressionTrackRecord>;
        fn increment(&self, tx: &mut Tx<'_>, account: &AccountId, progression_id: &str, amount: i64)
            -> ProgressionTrackRecord;
        fn set_progress_if_greater(&self, tx: &mut Tx<'_>, account: &AccountId, progression_id: &str, value: i64)
            -> ProgressionTrackRecord;
        fn set_confirmed_fremium_rank(&self, tx: &mut Tx<'_>, account: &AccountId, progression_id: &str, rank: i64)
            -> ProgressionTrackRecord;
        /// The raw report of a game server, kept for diagnosis
        fn append_objective_event(&self, tx: &mut Tx<'_>, account: &AccountId, raw_body: &str, received_at: &str)
            -> ();
        fn get_all_objectives_for_account(&self, conn: &mut AnyConnection, account: &AccountId)
            -> Vec<ProgressionObjectiveRecord>;
        fn set_objective_if_greater(&self, tx: &mut Tx<'_>, account: &AccountId, objective_id: &str, value: i64,
            completed_count: i64) -> ProgressionObjectiveRecord;
        fn begin_reforge(&self, tx: &mut Tx<'_>, account: &AccountId, progression_id: &str, expected_progress: i64,
            expected_version: i64, reforge: &WeaponReforge) -> Option<ProgressionTrackRecord>;
        /// `expected` and `set` as JSON objects of `WeaponReforgeState` fields (Node's `Partial<...>`)
        fn update_reforge(&self, tx: &mut Tx<'_>, account: &AccountId, progression_id: &str, reforge_id: &str,
            expected: &Json, set: &Json) -> bool;
        fn claim_prestige_levels(&self, tx: &mut Tx<'_>, account: &AccountId, progression_id: &str,
            claimed_before: i64, levels: i64, min_progress: i64) -> bool;
        fn get_objectives(&self, conn: &mut AnyConnection, account: &AccountId, objective_ids: &[String])
            -> Vec<ProgressionObjectiveRecord>;
        /// `(objective_id, value, completed_count)` rows
        fn merge_objectives(&self, tx: &mut Tx<'_>, account: &AccountId, rows: &[(String, i64, i64)]) -> ();
        fn spend(&self, tx: &mut Tx<'_>, account: &AccountId, progression_id: &str, amount: i64)
            -> Option<ProgressionTrackRecord>;
    }
}

/// `ProgressionGrantRecord`: one applied XP grant, by the id that makes a replay idempotent.
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

repository! {
    pub trait ProgressionGrantRepository("progressionGrants") {
        fn find_by_grant_id(&self, conn: &mut AnyConnection, grant_id: &str) -> Option<ProgressionGrantRecord>;
        fn insert_applied(&self, tx: &mut Tx<'_>, record: &ProgressionGrantRecord) -> ();
        fn list_recent_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32)
            -> Vec<ProgressionGrantRecord>;
    }
}

/// `EscalationProgressRecord`.
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

repository! {
    pub trait EscalationProgressRepository("escalationProgress") {
        fn get(&self, conn: &mut AnyConnection, account: &AccountId, season_id: &str)
            -> Option<EscalationProgressRecord>;
        /// `None` expected version inserts only; a conflict answers `None` so the caller re-reads and merges
        fn save_if_version(&self, tx: &mut Tx<'_>, record: &EscalationProgressRecord, expected_version: Option<i64>)
            -> Option<EscalationProgressRecord>;
    }
}

/// `PlayerJourneyRecord`: the Slayer's Path nodes, JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerJourneyRecord {
    pub account: AccountId,
    pub nodes: String,
    pub update_version: i64,
    pub revision: i64,
}

repository! {
    pub trait PlayerJourneyRepository("playerJourney") {
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<PlayerJourneyRecord>;
        fn create_if_absent(&self, tx: &mut Tx<'_>, record: &PlayerJourneyRecord) -> bool;
        fn update_if_revision(&self, tx: &mut Tx<'_>, account: &AccountId, nodes_json: &str, update_version: i64,
            expected_revision: i64) -> bool;
    }
}

/// `BreadcrumbsRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbsRecord {
    pub character_id: String,
    pub account: AccountId,
    pub breadcrumbs: String,
    pub update_version: i64,
}

repository! {
    pub trait BreadcrumbRepository("breadcrumbs") {
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<BreadcrumbsRecord>;
        fn create(&self, tx: &mut Tx<'_>, record: &BreadcrumbsRecord) -> ();
        fn update(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId, breadcrumbs_json: &str,
            update_version: i64) -> ();
    }
}

/// `EncounteredContentRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncounteredContentRecord {
    pub character_id: String,
    pub account: AccountId,
    pub encountered_content: String,
}

repository! {
    pub trait EncounteredContentRepository("encounteredContent") {
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId)
            -> Option<EncounteredContentRecord>;
        fn create(&self, tx: &mut Tx<'_>, record: &EncounteredContentRecord) -> ();
        fn update_content(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId, content_json: &str) -> ();
    }
}
