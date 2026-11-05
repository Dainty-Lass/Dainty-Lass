//! RU5 characters and loadouts: `CharacterRepository` and `LoadoutRepository` on `store`, both dialects (DD15).
//! Per-account rows are keyed by the account's global id (DD18); every write is optimistic as Node's was
//! (`updateVersion` for a character, `revision` for a loadout).

pub mod slots;

use diesel::prelude::*;
use serde_json::Value;
use service_kit::jsjson;

use self::slots::{DEFAULT_ACCOUNT_LOADOUT_SLOTS, MAX_TOTAL_LOADOUT_SLOTS};
use super::{AccountId, AnyConnection, RepoResult, Tx, repository};

/// `CharacterRecord`. `data` is the character's JSON as the client sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRecord {
    pub character_id: String,
    pub account: AccountId,
    pub created_date: String,
    pub last_modified_date: String,
    pub name: String,
    pub update_version: i64,
    pub data: String,
}

repository! {
    pub trait CharacterRepository("characters") {
        /// In insertion order
        fn find_many_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Vec<CharacterRecord>;
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<CharacterRecord>;
        fn create(&self, tx: &mut Tx<'_>, character: &CharacterRecord) -> CharacterRecord;
        /// Writes only while the stored `update_version` is below `effective_version`; a refused write is not an
        /// error, as in Node (`updateDataConditional` returned void).
        fn update_data_conditional(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId, data: &str,
            effective_version: i64) -> ();
    }
}

/// `LoadoutRecord`. `loadouts` and `persistent` are JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutRecord {
    pub character_id: String,
    pub account: AccountId,
    pub loadouts: String,
    pub persistent: String,
    pub active_index: Option<u32>,
    /// `None` on a row that predates the entitlement column; `create` defaults it to 1
    pub unlocked_total_slots: Option<u32>,
    pub revision: Option<i64>,
    pub bootstrap_version: Option<String>,
}

repository! {
    pub trait LoadoutRepository("loadouts") {
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<LoadoutRecord>;
        fn create(&self, tx: &mut Tx<'_>, loadout: &LoadoutRecord) -> ();
        /// Only an existing, visible slot; `false` on a stale revision or any refusal
        fn replace_slot_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId,
            slot_index: u32, data_json: &str, expected_revision: i64) -> bool;
        fn replace_all_and_entitlement_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str,
            account: &AccountId, loadouts_json: &str, unlocked_total_slots: u32, expected_revision: i64) -> bool;
        fn update_persistent_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId,
            persistent_json: &str, expected_revision: i64) -> bool;
        /// Re-applying the current slot is a success with no revision churn
        fn update_active_index_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId,
            active_index: u32, expected_revision: i64) -> bool;
    }
}

diesel::table! {
    characters (character_id) {
        character_id -> Text,
        account_id -> Text,
        ord -> BigInt,
        created_date -> Text,
        last_modified_date -> Text,
        name -> Text,
        update_version -> BigInt,
        data -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    loadouts (character_id) {
        character_id -> Text,
        account_id -> Text,
        #[sql_name = "loadouts"]
        slots_json -> Text,
        persistent -> Text,
        active_index -> BigInt,
        unlocked_total_slots -> Nullable<BigInt>,
        revision -> BigInt,
        bootstrap_version -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

type CharacterRow = (String, String, String, String, i64, String);

fn character(account: &AccountId, row: CharacterRow) -> CharacterRecord {
    let (character_id, created_date, last_modified_date, name, update_version, data) = row;
    CharacterRecord {
        character_id,
        account: account.clone(),
        created_date,
        last_modified_date,
        name,
        update_version,
        data,
    }
}

macro_rules! character_columns {
    () => {
        (
            characters::character_id,
            characters::created_date,
            characters::last_modified_date,
            characters::name,
            characters::update_version,
            characters::data,
        )
    };
}

/// [`CharacterRepository`] on `store`: live rows only.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlCharacters;

impl CharacterRepository for SqlCharacters {
    fn find_many_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Vec<CharacterRecord>> {
        let rows = characters::table
            .filter(characters::account_id.eq(account.to_string()))
            .filter(characters::deleted_at.is_null())
            .order(characters::ord.asc())
            .select(character_columns!())
            .load::<CharacterRow>(conn)?;
        Ok(rows.into_iter().map(|row| character(account, row)).collect())
    }

    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<CharacterRecord>> {
        Ok(characters::table
            .filter(characters::character_id.eq(character_id))
            .filter(characters::account_id.eq(account.to_string()))
            .filter(characters::deleted_at.is_null())
            .select(character_columns!())
            .first::<CharacterRow>(conn)
            .optional()?
            .map(|row| character(account, row)))
    }

    fn create(&self, tx: &mut Tx<'_>, record: &CharacterRecord) -> RepoResult<CharacterRecord> {
        let last = characters::table
            .select(diesel::dsl::max(characters::ord))
            .first::<Option<i64>>(tx.conn())?;
        diesel::insert_into(characters::table)
            .values((
                characters::character_id.eq(&record.character_id),
                characters::account_id.eq(record.account.to_string()),
                characters::ord.eq(last.unwrap_or(0) + 1),
                characters::created_date.eq(&record.created_date),
                characters::last_modified_date.eq(&record.last_modified_date),
                characters::name.eq(&record.name),
                characters::update_version.eq(record.update_version),
                characters::data.eq(&record.data),
            ))
            .execute(tx.conn())?;
        Ok(record.clone())
    }

    fn update_data_conditional(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        data: &str,
        effective_version: i64,
    ) -> RepoResult<()> {
        diesel::update(
            characters::table
                .filter(characters::character_id.eq(character_id))
                .filter(characters::account_id.eq(account.to_string()))
                .filter(characters::deleted_at.is_null())
                .filter(characters::update_version.lt(effective_version)),
        )
        .set((
            characters::data.eq(data),
            characters::update_version.eq(effective_version),
        ))
        .execute(tx.conn())?;
        Ok(())
    }
}

type LoadoutRow = (String, String, i64, Option<i64>, i64, Option<String>);

fn loadout(character_id: &str, account: &AccountId, row: LoadoutRow) -> LoadoutRecord {
    let (loadouts, persistent, active_index, unlocked, revision, bootstrap_version) = row;
    LoadoutRecord {
        character_id: character_id.to_owned(),
        account: account.clone(),
        loadouts,
        persistent,
        active_index: Some(u32::try_from(active_index).unwrap_or(0)),
        unlocked_total_slots: unlocked.and_then(|n| u32::try_from(n).ok()),
        revision: Some(revision),
        bootstrap_version,
    }
}

macro_rules! loadout_columns {
    () => {
        (
            loadouts::slots_json,
            loadouts::persistent,
            loadouts::active_index,
            loadouts::unlocked_total_slots,
            loadouts::revision,
            loadouts::bootstrap_version,
        )
    };
}

/// The live row of one character's loadouts for its account (a `loadouts::table` query).
macro_rules! live_loadout {
    ($character_id:expr, $account:expr) => {
        loadouts::table
            .filter(loadouts::character_id.eq($character_id))
            .filter(loadouts::account_id.eq($account.to_string()))
            .filter(loadouts::deleted_at.is_null())
    };
}

/// The stored slot array, `None` when it isn't a JSON array (Node's `catch { return false }`).
fn slot_array(text: &str) -> Option<Vec<Value>> {
    match jsjson::parse(text).ok()? {
        Value::Array(items) => Some(items),
        _ => None,
    }
}

/// [`LoadoutRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlLoadouts;

impl SqlLoadouts {
    fn row(tx: &mut Tx<'_>, character_id: &str, account: &AccountId) -> RepoResult<Option<LoadoutRow>> {
        Ok(live_loadout!(character_id, account)
            .select(loadout_columns!())
            .first::<LoadoutRow>(tx.conn())
            .optional()?)
    }
}

impl LoadoutRepository for SqlLoadouts {
    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<LoadoutRecord>> {
        Ok(live_loadout!(character_id, account)
            .select(loadout_columns!())
            .first::<LoadoutRow>(conn)
            .optional()?
            .map(|row| loadout(character_id, account, row)))
    }

    fn create(&self, tx: &mut Tx<'_>, record: &LoadoutRecord) -> RepoResult<()> {
        diesel::insert_into(loadouts::table)
            .values((
                loadouts::character_id.eq(&record.character_id),
                loadouts::account_id.eq(record.account.to_string()),
                loadouts::slots_json.eq(&record.loadouts),
                loadouts::persistent.eq(&record.persistent),
                loadouts::active_index.eq(i64::from(record.active_index.unwrap_or(0))),
                loadouts::unlocked_total_slots.eq(Some(i64::from(
                    record.unlocked_total_slots.unwrap_or(DEFAULT_ACCOUNT_LOADOUT_SLOTS),
                ))),
                loadouts::revision.eq(record.revision.unwrap_or(0)),
                loadouts::bootstrap_version.eq(&record.bootstrap_version),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn replace_slot_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        slot_index: u32,
        data_json: &str,
        expected_revision: i64,
    ) -> RepoResult<bool> {
        let Some((stored, _, _, unlocked, _, _)) = Self::row(tx, character_id, account)? else {
            return Ok(false);
        };
        let Some(mut array) = slot_array(&stored) else {
            return Ok(false);
        };
        let Ok(visible) = slots::resolve_visible_total(array.len() as i64, unlocked) else {
            return Ok(false);
        };
        if slot_index >= visible {
            return Ok(false);
        }
        let Ok(slot) = jsjson::parse(data_json) else {
            return Ok(false);
        };
        array[slot_index as usize] = slot;
        let text = jsjson::stringify(&Value::Array(array));
        let changed =
            diesel::update(live_loadout!(character_id, account).filter(loadouts::revision.eq(expected_revision)))
                .set((
                    loadouts::slots_json.eq(text),
                    loadouts::revision.eq(expected_revision + 1),
                ))
                .execute(tx.conn())?;
        Ok(changed > 0)
    }

    fn replace_all_and_entitlement_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        loadouts_json: &str,
        unlocked_total_slots: u32,
        expected_revision: i64,
    ) -> RepoResult<bool> {
        let Some(array) = slot_array(loadouts_json) else {
            return Ok(false);
        };
        let len = array.len() as u32;
        if !(DEFAULT_ACCOUNT_LOADOUT_SLOTS..=MAX_TOTAL_LOADOUT_SLOTS).contains(&len)
            || !(DEFAULT_ACCOUNT_LOADOUT_SLOTS..=MAX_TOTAL_LOADOUT_SLOTS).contains(&unlocked_total_slots)
            || unlocked_total_slots > len
        {
            return Ok(false);
        }
        let text = jsjson::stringify(&Value::Array(array));
        let changed =
            diesel::update(live_loadout!(character_id, account).filter(loadouts::revision.eq(expected_revision)))
                .set((
                    loadouts::slots_json.eq(text),
                    loadouts::unlocked_total_slots.eq(Some(i64::from(unlocked_total_slots))),
                    loadouts::revision.eq(expected_revision + 1),
                ))
                .execute(tx.conn())?;
        Ok(changed > 0)
    }

    fn update_persistent_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        persistent_json: &str,
        expected_revision: i64,
    ) -> RepoResult<bool> {
        let changed =
            diesel::update(live_loadout!(character_id, account).filter(loadouts::revision.eq(expected_revision)))
                .set((
                    loadouts::persistent.eq(persistent_json),
                    loadouts::revision.eq(expected_revision + 1),
                ))
                .execute(tx.conn())?;
        Ok(changed > 0)
    }

    fn update_active_index_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        active_index: u32,
        expected_revision: i64,
    ) -> RepoResult<bool> {
        let Some((stored, _, current, unlocked, _, _)) = Self::row(tx, character_id, account)? else {
            return Ok(false);
        };
        let Some(array) = slot_array(&stored) else {
            return Ok(false);
        };
        let Ok(visible) = slots::resolve_visible_total(array.len() as i64, unlocked) else {
            return Ok(false);
        };
        if active_index >= visible {
            return Ok(false);
        }
        if current == i64::from(active_index) {
            return Ok(true);
        }
        let changed =
            diesel::update(live_loadout!(character_id, account).filter(loadouts::revision.eq(expected_revision)))
                .set((
                    loadouts::active_index.eq(i64::from(active_index)),
                    loadouts::revision.eq(expected_revision + 1),
                ))
                .execute(tx.conn())?;
        Ok(changed > 0)
    }
}
