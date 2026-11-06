// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Characters and loadouts (spec §7).

pub mod slots;

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Text};
use serde_json::Value;
use service_kit::jsjson;

use crate::sql::{account_id, sql};
use crate::{AccountId, AnyConnection, RepoResult, Tx};

/// A live `characters` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRecord {
    /// The character's id.
    pub character_id: String,
    /// The owning account.
    pub account: AccountId,
    /// When it was created, as the client's date text.
    pub created_date: String,
    /// When it was last changed, as the client's date text.
    pub last_modified_date: String,
    /// The character's name.
    pub name: String,
    /// The client's version; a save must be newer.
    pub update_version: i64,
    /// The character JSON as the client sent it
    pub data: String,
}

/// A live `loadouts` row: a character's loadout slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutRecord {
    /// The character.
    pub character_id: String,
    /// The owning account.
    pub account: AccountId,
    /// JSON text
    pub loadouts: String,
    /// JSON text
    pub persistent: String,
    /// The selected slot.
    pub active_index: Option<u32>,
    /// The slots unlocked; None on a legacy row.
    pub unlocked_total_slots: Option<u32>,
    /// Bumped on every write; a write names the revision it read; None reads as 0.
    pub revision: Option<i64>,
    /// The starter kit version the row was created from.
    pub bootstrap_version: Option<String>,
}

store_trait! {
    /// Characters (`characters`); every read sees live rows only.
    pub trait CharacterRepository = "characters" {
        /// The account's characters, in creation order.
        fn find_many_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Vec<CharacterRecord>;
        /// The character, when it is the account's.
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<CharacterRecord>;
        /// Writes a new character, placed after every row the account ever had.
        fn create(&self, tx: &mut Tx<'_>, record: &CharacterRecord) -> CharacterRecord;
        /// Refused silently unless the stored version is below `effective_version` (Node's `void`)
        fn update_data_conditional(
            &self,
            tx: &mut Tx<'_>,
            character_id: &str,
            account: &AccountId,
            data: &str,
            effective_version: i64,
        ) -> ();
    }
}

store_trait! {
    /// Loadouts (`loadouts`); each write is refused (false) unless the revision is still `expected`, and bumps it.
    pub trait LoadoutRepository = "loadouts" {
        /// The character's loadouts, when it is the account's.
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<LoadoutRecord>;
        /// Writes a new loadout row.
        fn create(&self, tx: &mut Tx<'_>, record: &LoadoutRecord) -> ();
        /// Replaces one visible slot with `data_json`; false for a slot out of range or text that does not parse.
        fn replace_slot_if_revision_matches(
            &self,
            tx: &mut Tx<'_>,
            character_id: &str,
            account: &AccountId,
            slot_index: u32,
            data_json: &str,
            expected: i64,
        ) -> bool;
        /// Replaces every slot and the unlocked count; false unless 1 to 6 slots with at most that many unlocked.
        fn replace_all_and_entitlement_if_revision_matches(
            &self,
            tx: &mut Tx<'_>,
            character_id: &str,
            account: &AccountId,
            loadouts_json: &str,
            unlocked: u32,
            expected: i64,
        ) -> bool;
        /// Replaces `persistent`.
        fn update_persistent_if_revision_matches(
            &self,
            tx: &mut Tx<'_>,
            character_id: &str,
            account: &AccountId,
            persistent_json: &str,
            expected: i64,
        ) -> bool;
        /// `true` without a write when the index is already stored, whatever the revision (spec §16 item 8)
        fn update_active_index_if_revision_matches(
            &self,
            tx: &mut Tx<'_>,
            character_id: &str,
            account: &AccountId,
            index: u32,
            expected: i64,
        ) -> bool;
    }
}

// ------------------------------------------------------------------ characters

/// [`CharacterRepository`] in SQL.
pub struct SqlCharacters;

#[derive(QueryableByName)]
struct CharacterRow {
    #[diesel(sql_type = Text)]
    character_id: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    created_date: String,
    #[diesel(sql_type = Text)]
    last_modified_date: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    update_version: i64,
    #[diesel(sql_type = Text)]
    data: String,
}

impl CharacterRow {
    fn decode(self) -> RepoResult<CharacterRecord> {
        Ok(CharacterRecord {
            account: account_id(&self.account_id, "characters.account_id")?,
            character_id: self.character_id,
            created_date: self.created_date,
            last_modified_date: self.last_modified_date,
            name: self.name,
            update_version: self.update_version,
            data: self.data,
        })
    }
}

impl CharacterRepository for SqlCharacters {
    fn find_many_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Vec<CharacterRecord>> {
        sql("SELECT * FROM characters WHERE account_id = ? AND deleted_at IS NULL ORDER BY ord ASC")
            .t(account.to_string())
            .load::<CharacterRow>(conn)?
            .into_iter()
            .map(CharacterRow::decode)
            .collect()
    }

    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<CharacterRecord>> {
        sql("SELECT * FROM characters WHERE character_id = ? AND account_id = ? AND deleted_at IS NULL")
            .t(character_id)
            .t(account.to_string())
            .first::<CharacterRow>(conn)?
            .map(CharacterRow::decode)
            .transpose()
    }

    fn create(&self, tx: &mut Tx<'_>, r: &CharacterRecord) -> RepoResult<CharacterRecord> {
        let conn = tx.conn();
        // Every account's rows, deleted ones too: only the order matters (spec §16 item 7)
        let ord = sql("SELECT MAX(ord) AS n FROM characters").int(conn)? + 1;
        sql(
            "INSERT INTO characters (character_id, account_id, ord, created_date, last_modified_date, name, \
             update_version, data) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.character_id)
        .t(r.account.to_string())
        .i(ord)
        .t(&*r.created_date)
        .t(&*r.last_modified_date)
        .t(&*r.name)
        .i(r.update_version)
        .t(&*r.data)
        .run(conn)?;
        Ok(r.clone())
    }

    fn update_data_conditional(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        data: &str,
        effective_version: i64,
    ) -> RepoResult<()> {
        sql(
            "UPDATE characters SET data = ?, update_version = ? WHERE character_id = ? AND account_id = ? \
             AND deleted_at IS NULL AND update_version < ?",
        )
        .t(data)
        .i(effective_version)
        .t(character_id)
        .t(account.to_string())
        .i(effective_version)
        .run(tx.conn())?;
        Ok(())
    }
}

// ------------------------------------------------------------------ loadouts

/// [`LoadoutRepository`] in SQL.
pub struct SqlLoadouts;

#[derive(QueryableByName)]
struct LoadoutRow {
    #[diesel(sql_type = Text)]
    character_id: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    loadouts: String,
    #[diesel(sql_type = Text)]
    persistent: String,
    #[diesel(sql_type = BigInt)]
    active_index: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    unlocked_total_slots: Option<i64>,
    #[diesel(sql_type = BigInt)]
    revision: i64,
    #[diesel(sql_type = Nullable<Text>)]
    bootstrap_version: Option<String>,
}

impl LoadoutRow {
    fn decode(self) -> RepoResult<LoadoutRecord> {
        Ok(LoadoutRecord {
            account: account_id(&self.account_id, "loadouts.account_id")?,
            character_id: self.character_id,
            loadouts: self.loadouts,
            persistent: self.persistent,
            active_index: Some(u32::try_from(self.active_index).unwrap_or(0)),
            unlocked_total_slots: self.unlocked_total_slots.and_then(|n| u32::try_from(n).ok()),
            revision: Some(self.revision),
            bootstrap_version: self.bootstrap_version,
        })
    }

    /// The stored array and its visible total, when both resolve.
    fn slots(&self) -> Option<(Vec<Value>, u32)> {
        let Ok(Value::Array(items)) = jsjson::parse(&self.loadouts) else {
            return None;
        };
        let stored = i64::try_from(items.len()).ok()?;
        let visible = slots::resolve_visible_total(stored, self.unlocked_total_slots).ok()?;
        Some((items, visible))
    }
}

fn loadout_row(conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> RepoResult<Option<LoadoutRow>> {
    sql("SELECT * FROM loadouts WHERE character_id = ? AND account_id = ? AND deleted_at IS NULL")
        .t(character_id)
        .t(account.to_string())
        .first::<LoadoutRow>(conn)
}

/// Runs a [`bump_sql`] statement whose `set` values are bound; whether the row at `expected` changed.
fn bump(
    conn: &mut AnyConnection,
    value: crate::sql::Sql,
    character_id: &str,
    account: &AccountId,
    expected: i64,
) -> RepoResult<bool> {
    Ok(value
        .i(expected.saturating_add(1))
        .t(character_id)
        .t(account.to_string())
        .i(expected)
        .run(conn)?
        == 1)
}

/// `UPDATE loadouts SET <set>, revision = expected + 1` on the live row of the character and account at `expected`.
fn bump_sql(set: &str) -> String {
    format!(
        "UPDATE loadouts SET {set}, revision = ? WHERE character_id = ? AND account_id = ? \
         AND deleted_at IS NULL AND revision = ?"
    )
}

impl LoadoutRepository for SqlLoadouts {
    fn find(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<LoadoutRecord>> {
        loadout_row(conn, character_id, account)?
            .map(LoadoutRow::decode)
            .transpose()
    }

    fn create(&self, tx: &mut Tx<'_>, r: &LoadoutRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO loadouts (character_id, account_id, loadouts, persistent, active_index, \
             unlocked_total_slots, revision, bootstrap_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.character_id)
        .t(r.account.to_string())
        .t(&*r.loadouts)
        .t(&*r.persistent)
        .i(r.active_index.unwrap_or(0).into())
        .i(r.unlocked_total_slots.unwrap_or(1).into())
        .i(r.revision.unwrap_or(0))
        .ot(r.bootstrap_version.clone())
        .run(tx.conn())?;
        Ok(())
    }

    fn replace_slot_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        slot_index: u32,
        data_json: &str,
        expected: i64,
    ) -> RepoResult<bool> {
        let conn = tx.conn();
        let Some(row) = loadout_row(conn, character_id, account)? else {
            return Ok(false);
        };
        let Some((mut items, visible)) = row.slots() else {
            return Ok(false);
        };
        if slot_index >= visible {
            return Ok(false);
        }
        let Ok(data) = jsjson::parse(data_json) else {
            return Ok(false);
        };
        let Some(slot) = items.get_mut(slot_index as usize) else {
            return Ok(false);
        };
        *slot = data;
        let text = jsjson::stringify(&Value::Array(items));
        bump(
            conn,
            sql(bump_sql("loadouts = ?")).t(text),
            character_id,
            account,
            expected,
        )
    }

    fn replace_all_and_entitlement_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        loadouts_json: &str,
        unlocked: u32,
        expected: i64,
    ) -> RepoResult<bool> {
        let Ok(Value::Array(items)) = jsjson::parse(loadouts_json) else {
            return Ok(false);
        };
        let length = items.len();
        if !(1..=6).contains(&length) || !(1..=6).contains(&unlocked) || unlocked as usize > length {
            return Ok(false);
        }
        let text = jsjson::stringify(&Value::Array(items));
        let value = sql(bump_sql("loadouts = ?, unlocked_total_slots = ?"))
            .t(text)
            .i(unlocked.into());
        bump(tx.conn(), value, character_id, account, expected)
    }

    fn update_persistent_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        persistent_json: &str,
        expected: i64,
    ) -> RepoResult<bool> {
        let value = sql(bump_sql("persistent = ?")).t(persistent_json);
        bump(tx.conn(), value, character_id, account, expected)
    }

    fn update_active_index_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        index: u32,
        expected: i64,
    ) -> RepoResult<bool> {
        let conn = tx.conn();
        let Some(row) = loadout_row(conn, character_id, account)? else {
            return Ok(false);
        };
        let Some((_, visible)) = row.slots() else {
            return Ok(false);
        };
        if index >= visible {
            return Ok(false);
        }
        if row.active_index == i64::from(index) {
            return Ok(true);
        }
        let value = sql(bump_sql("active_index = ?")).i(index.into());
        bump(conn, value, character_id, account, expected)
    }
}
