//! RU5 characters and loadouts.

use super::{AccountId, AnyConnection, Tx, repository};

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
        fn find_many_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Vec<CharacterRecord>;
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<CharacterRecord>;
        fn create(&self, tx: &mut Tx<'_>, character: &CharacterRecord) -> CharacterRecord;
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
    pub unlocked_total_slots: Option<u32>,
    pub revision: Option<i64>,
    pub bootstrap_version: Option<String>,
}

repository! {
    pub trait LoadoutRepository("loadouts") {
        fn find(&self, conn: &mut AnyConnection, character_id: &str, account: &AccountId) -> Option<LoadoutRecord>;
        fn create(&self, tx: &mut Tx<'_>, loadout: &LoadoutRecord) -> ();
        fn replace_slot_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId,
            slot_index: u32, data_json: &str, expected_revision: i64) -> bool;
        fn replace_all_and_entitlement_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str,
            account: &AccountId, loadouts_json: &str, unlocked_total_slots: u32, expected_revision: i64) -> bool;
        fn update_persistent_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId,
            persistent_json: &str, expected_revision: i64) -> bool;
        fn update_active_index_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, account: &AccountId,
            active_index: u32, expected_revision: i64) -> bool;
    }
}
