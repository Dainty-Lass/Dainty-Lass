//! RU5 inventory, storage policy and reforge.

use super::{AccountId, AnyConnection, Json, Tx, repository};

/// `InventoryRecord`. The item lists are JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRecord {
    pub character_id: String,
    pub account: Option<AccountId>,
    pub instanced_items: String,
    pub stacked_items: String,
    pub revision: Option<i64>,
    /// `STARTER_MANIFEST`'s bootstrap version this inventory was seeded with
    pub bootstrap_version: Option<String>,
}

repository! {
    pub trait InventoryRepository("inventories") {
        fn find_by_character_id(&self, conn: &mut AnyConnection, character_id: &str) -> Option<InventoryRecord>;
        fn create(&self, tx: &mut Tx<'_>, inventory: &InventoryRecord) -> ();
        fn update_both_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, instanced_items_json: &str,
            stacked_items_json: &str, expected_revision: i64) -> Option<InventoryRecord>;
    }
}

/// `InventoryTransactionRecord`: `POST /inventory`'s idempotency ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryTransactionRecord {
    pub transaction_id: String,
    pub account: AccountId,
    pub character_id: String,
    pub request_hash: String,
    pub completed: bool,
    pub result: Option<Json>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

repository! {
    pub trait InventoryTransactionRepository("inventoryTransactions") {
        /// `None` when the transaction was begun before (a replay)
        fn try_begin(&self, tx: &mut Tx<'_>, transaction_id: &str, account: &AccountId, character_id: &str,
            request_hash: &str) -> Option<InventoryTransactionRecord>;
        fn complete(&self, tx: &mut Tx<'_>, transaction_id: &str, account: &AccountId, character_id: &str,
            result: &Json, extra_fields: Option<&Json>) -> ();
        fn find_record(&self, conn: &mut AnyConnection, account: &AccountId, character_id: &str,
            transaction_id: &str) -> Option<InventoryTransactionRecord>;
        fn list_recent_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32) -> Vec<Json>;
    }
}
