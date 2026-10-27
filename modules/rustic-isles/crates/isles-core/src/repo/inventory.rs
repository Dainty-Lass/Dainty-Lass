//! RU5 inventory, storage policy and reforge: the inventories, POST /inventory's idempotency ledger, and the pure
//! rules of the reforge record that `progression_tracks` keeps (its SQL is the progression area's).

use diesel::prelude::*;
use serde_json::{Map, Value};

use super::progression::WeaponReforge;
use super::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx, repository};

/// `InventoryRecord`. The item lists are JSON text, as Node stored them (`persistence/mapping/domainTypes.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRecord {
    pub character_id: String,
    /// Node's `userId`, optional there too: rows migrated from before it was tracked have none
    pub account: Option<AccountId>,
    pub instanced_items: String,
    pub stacked_items: String,
    /// `None` reads as 0 (a row from before the counter existed)
    pub revision: Option<i64>,
    /// `STARTER_MANIFEST`'s bootstrap version this inventory was seeded with
    pub bootstrap_version: Option<String>,
}

repository! {
    pub trait InventoryRepository("inventories") {
        fn find_by_character_id(&self, conn: &mut AnyConnection, character_id: &str) -> Option<InventoryRecord>;
        fn create(&self, tx: &mut Tx<'_>, inventory: &InventoryRecord) -> ();
        /// The only mutation path: applies only if the row's revision is `expected_revision` (a missing one reads
        /// as 0) and stores `expected_revision + 1`. `None` when the row is gone or someone wrote first.
        fn update_both_if_revision_matches(&self, tx: &mut Tx<'_>, character_id: &str, instanced_items_json: &str,
            stacked_items_json: &str, expected_revision: i64) -> Option<InventoryRecord>;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerStatus {
    Pending,
    Completed,
}

impl LedgerStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
        }
    }
}

/// `InventoryTransactionRecord`: `POST /inventory`'s idempotency ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryTransactionRecord {
    pub transaction_id: String,
    pub account: AccountId,
    pub character_id: String,
    /// `None` on a legacy row written before the body binding existed
    pub request_hash: Option<String>,
    pub status: LedgerStatus,
    pub result: Option<Json>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub economy_source: Option<String>,
    pub economy_session_id: Option<String>,
    /// The other fields an `onGrant` annotator stored on the row (an object), if any
    pub extra: Option<Json>,
}

/// What `tryBegin` found. Node throws on a duplicate because a duplicate-key write aborts a Mongo transaction; here
/// nothing was written, so it is a value.
#[derive(Debug, Clone, PartialEq)]
pub enum TryBegin {
    Begun,
    Existing(Box<InventoryTransactionRecord>),
}

/// The ledger's key: Node hashed `(userId, characterId, transactionId)` into `_id`.
#[derive(Debug, Clone, Copy)]
pub struct LedgerKey<'a> {
    pub account: &'a AccountId,
    pub character_id: &'a str,
    pub transaction_id: &'a str,
}

repository! {
    pub trait InventoryTransactionRepository("inventoryTransactions") {
        /// Claims `(account, character, transaction)` with a pending row, or reports the row already there.
        fn try_begin(&self, tx: &mut Tx<'_>, key: LedgerKey<'_>, request_hash: &str, created_at: &str) -> TryBegin;
        /// Marks the pending row completed with the result to replay. `extra_fields` are stored on the row
        /// (`economySource` and `economySessionId` in their columns, the rest as JSON); they never replace the
        /// ledger's own fields.
        fn complete(&self, tx: &mut Tx<'_>, key: LedgerKey<'_>, result: &Json, extra_fields: Option<&Json>,
            completed_at: &str) -> ();
        fn find_record(&self, conn: &mut AnyConnection, key: LedgerKey<'_>) -> Option<InventoryTransactionRecord>;
        /// The account's newest rows, newest first, with the fields an admin timeline shows and never the results.
        fn list_recent_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32) -> Vec<Json>;
    }
}

// ------------------------------------------------------------------ the reforge record

fn status_text(open: bool) -> &'static str {
    if open { "open" } else { "closed" }
}

/// Whether `reforge` has every field `expected` names (Node's `Partial<WeaponReforgeState>` filter on
/// `updateReforge`), by the Node field names.
pub fn reforge_matches(reforge: &WeaponReforge, expected: &Json) -> bool {
    let Some(fields) = expected.as_object() else {
        return false;
    };
    fields.iter().all(|(name, want)| match name.as_str() {
        "reforgeId" => want.as_str() == Some(reforge.reforge_id.as_str()),
        "status" => want.as_str() == Some(status_text(reforge.open)),
        "priorProgress" => want.as_i64() == Some(reforge.prior_progress),
        "cap" => want.as_i64() == Some(reforge.cap),
        "banked" => want.as_i64() == Some(reforge.banked),
        "bankedRestored" => want.as_i64() == Some(reforge.banked_restored),
        "prestigeGranted" => want.as_bool() == Some(reforge.prestige_granted),
        "openedAt" => want.as_str() == Some(reforge.opened_at.as_str()),
        "closedAt" => want.as_str() == reforge.closed_at.as_deref(),
        _ => false,
    })
}

/// `reforge` with the fields `set` names replaced (Node's `$set` of `reforge.<field>`).
pub fn reforge_with(reforge: &WeaponReforge, set: &Json) -> WeaponReforge {
    let mut next = reforge.clone();
    let Some(fields) = set.as_object() else {
        return next;
    };
    for (name, value) in fields {
        match name.as_str() {
            "status" => next.open = value.as_str() == Some("open"),
            "priorProgress" => next.prior_progress = value.as_i64().unwrap_or(next.prior_progress),
            "cap" => next.cap = value.as_i64().unwrap_or(next.cap),
            "banked" => next.banked = value.as_i64().unwrap_or(next.banked),
            "bankedRestored" => next.banked_restored = value.as_i64().unwrap_or(next.banked_restored),
            "prestigeGranted" => next.prestige_granted = value.as_bool().unwrap_or(next.prestige_granted),
            "closedAt" => next.closed_at = value.as_str().map(str::to_owned),
            _ => {}
        }
    }
    next
}

/// `updateReforge`'s whole rule, for the progression area's SQL to share: `Some(next)` when the stored reforge is
/// `reforge_id` and has every field `expected` names, and `set` changes something (a `$set` that changes nothing is
/// not a modification, and answers false); `None` otherwise.
pub fn apply_reforge_update(
    current: &WeaponReforge,
    reforge_id: &str,
    expected: &Json,
    set: &Json,
) -> Option<WeaponReforge> {
    if current.reforge_id != reforge_id || !reforge_matches(current, expected) {
        return None;
    }
    let next = reforge_with(current, set);
    (next != *current).then_some(next)
}

// ------------------------------------------------------------------ SQL

diesel::table! {
    inventories (character_id) {
        character_id -> Text,
        account_id -> Nullable<Text>,
        instanced_items -> Text,
        stacked_items -> Text,
        revision -> BigInt,
        bootstrap_version -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    inventory_transactions (account_id, character_id, transaction_id) {
        account_id -> Text,
        character_id -> Text,
        transaction_id -> Text,
        request_hash -> Nullable<Text>,
        status -> Text,
        result -> Nullable<Text>,
        created_at -> Text,
        completed_at -> Nullable<Text>,
        economy_source -> Nullable<Text>,
        economy_session_id -> Nullable<Text>,
        extra -> Nullable<Text>,
    }
}

type InventoryRow = (String, Option<String>, String, String, i64, Option<String>);

fn inventory(row: InventoryRow) -> RepoResult<InventoryRecord> {
    let (character_id, account, instanced_items, stacked_items, revision, bootstrap_version) = row;
    let account = account
        .map(|text| {
            text.parse()
                .map_err(|_| RepoError::Data(format!("inventories.account_id {text:?} is not an account id")))
        })
        .transpose()?;
    Ok(InventoryRecord {
        character_id,
        account,
        instanced_items,
        stacked_items,
        revision: Some(revision),
        bootstrap_version,
    })
}

fn select_inventory(conn: &mut AnyConnection, character_id: &str) -> RepoResult<Option<InventoryRecord>> {
    inventories::table
        .filter(inventories::character_id.eq(character_id))
        .filter(inventories::deleted_at.is_null())
        .select((
            inventories::character_id,
            inventories::account_id,
            inventories::instanced_items,
            inventories::stacked_items,
            inventories::revision,
            inventories::bootstrap_version,
        ))
        .first::<InventoryRow>(conn)
        .optional()?
        .map(inventory)
        .transpose()
}

/// [`InventoryRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlInventories;

impl InventoryRepository for SqlInventories {
    fn find_by_character_id(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
    ) -> RepoResult<Option<InventoryRecord>> {
        select_inventory(conn, character_id)
    }

    fn create(&self, tx: &mut Tx<'_>, record: &InventoryRecord) -> RepoResult<()> {
        diesel::insert_into(inventories::table)
            .values((
                inventories::character_id.eq(&record.character_id),
                inventories::account_id.eq(record.account.as_ref().map(ToString::to_string)),
                inventories::instanced_items.eq(&record.instanced_items),
                inventories::stacked_items.eq(&record.stacked_items),
                inventories::revision.eq(record.revision.unwrap_or(0)),
                inventories::bootstrap_version.eq(&record.bootstrap_version),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn update_both_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        instanced_items_json: &str,
        stacked_items_json: &str,
        expected_revision: i64,
    ) -> RepoResult<Option<InventoryRecord>> {
        let changed = diesel::update(
            inventories::table
                .filter(inventories::character_id.eq(character_id))
                .filter(inventories::deleted_at.is_null())
                .filter(inventories::revision.eq(expected_revision)),
        )
        .set((
            inventories::instanced_items.eq(instanced_items_json),
            inventories::stacked_items.eq(stacked_items_json),
            inventories::revision.eq(expected_revision + 1),
        ))
        .execute(tx.conn())?;
        if changed == 0 {
            return Ok(None);
        }
        select_inventory(tx.conn(), character_id)
    }
}

type LedgerRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

const LEDGER_COLUMNS: (
    inventory_transactions::account_id,
    inventory_transactions::character_id,
    inventory_transactions::transaction_id,
    inventory_transactions::request_hash,
    inventory_transactions::status,
    inventory_transactions::result,
    inventory_transactions::created_at,
    inventory_transactions::completed_at,
    inventory_transactions::economy_source,
    inventory_transactions::economy_session_id,
    inventory_transactions::extra,
) = (
    inventory_transactions::account_id,
    inventory_transactions::character_id,
    inventory_transactions::transaction_id,
    inventory_transactions::request_hash,
    inventory_transactions::status,
    inventory_transactions::result,
    inventory_transactions::created_at,
    inventory_transactions::completed_at,
    inventory_transactions::economy_source,
    inventory_transactions::economy_session_id,
    inventory_transactions::extra,
);

fn parse_json(text: Option<String>, what: &str) -> RepoResult<Option<Json>> {
    text.map(|text| {
        serde_json::from_str(&text).map_err(|error| RepoError::Data(format!("inventory_transactions.{what}: {error}")))
    })
    .transpose()
}

fn ledger(row: LedgerRow) -> RepoResult<InventoryTransactionRecord> {
    let (
        account,
        character_id,
        transaction_id,
        request_hash,
        status,
        result,
        created_at,
        completed_at,
        source,
        session,
        extra,
    ) = row;
    Ok(InventoryTransactionRecord {
        transaction_id,
        account: account.parse().map_err(|_| {
            RepoError::Data(format!(
                "inventory_transactions.account_id {account:?} is not an account id"
            ))
        })?,
        character_id,
        request_hash,
        status: if status == "completed" {
            LedgerStatus::Completed
        } else {
            LedgerStatus::Pending
        },
        result: parse_json(result, "result")?,
        created_at,
        completed_at,
        economy_source: source,
        economy_session_id: session,
        extra: parse_json(extra, "extra")?,
    })
}

fn select_ledger(conn: &mut AnyConnection, key: LedgerKey<'_>) -> RepoResult<Option<InventoryTransactionRecord>> {
    inventory_transactions::table
        .filter(inventory_transactions::account_id.eq(key.account.to_string()))
        .filter(inventory_transactions::character_id.eq(key.character_id))
        .filter(inventory_transactions::transaction_id.eq(key.transaction_id))
        .select(LEDGER_COLUMNS)
        .first::<LedgerRow>(conn)
        .optional()?
        .map(ledger)
        .transpose()
}

/// [`InventoryTransactionRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlInventoryTransactions;

impl InventoryTransactionRepository for SqlInventoryTransactions {
    fn try_begin(
        &self,
        tx: &mut Tx<'_>,
        key: LedgerKey<'_>,
        request_hash: &str,
        created_at: &str,
    ) -> RepoResult<TryBegin> {
        if let Some(existing) = select_ledger(tx.conn(), key)? {
            return Ok(TryBegin::Existing(Box::new(existing)));
        }
        diesel::insert_into(inventory_transactions::table)
            .values((
                inventory_transactions::account_id.eq(key.account.to_string()),
                inventory_transactions::character_id.eq(key.character_id),
                inventory_transactions::transaction_id.eq(key.transaction_id),
                inventory_transactions::request_hash.eq(request_hash),
                inventory_transactions::status.eq(LedgerStatus::Pending.as_str()),
                inventory_transactions::created_at.eq(created_at),
            ))
            .execute(tx.conn())?;
        Ok(TryBegin::Begun)
    }

    fn complete(
        &self,
        tx: &mut Tx<'_>,
        key: LedgerKey<'_>,
        result: &Json,
        extra_fields: Option<&Json>,
        completed_at: &str,
    ) -> RepoResult<()> {
        let mut rest: Map<String, Value> = extra_fields.and_then(Value::as_object).cloned().unwrap_or_default();
        let source = rest.remove("economySource").and_then(|v| v.as_str().map(str::to_owned));
        let session = rest
            .remove("economySessionId")
            .and_then(|v| v.as_str().map(str::to_owned));
        for core in ["status", "result", "completedAt"] {
            rest.remove(core);
        }
        let extra = (!rest.is_empty()).then(|| service_kit::jsjson::stringify(&Value::Object(rest)));
        diesel::update(
            inventory_transactions::table
                .filter(inventory_transactions::account_id.eq(key.account.to_string()))
                .filter(inventory_transactions::character_id.eq(key.character_id))
                .filter(inventory_transactions::transaction_id.eq(key.transaction_id)),
        )
        .set((
            inventory_transactions::status.eq(LedgerStatus::Completed.as_str()),
            inventory_transactions::result.eq(Some(service_kit::jsjson::stringify(result))),
            inventory_transactions::completed_at.eq(Some(completed_at)),
            inventory_transactions::economy_source.eq(source),
            inventory_transactions::economy_session_id.eq(session),
            inventory_transactions::extra.eq(extra),
        ))
        .execute(tx.conn())?;
        Ok(())
    }

    fn find_record(
        &self,
        conn: &mut AnyConnection,
        key: LedgerKey<'_>,
    ) -> RepoResult<Option<InventoryTransactionRecord>> {
        select_ledger(conn, key)
    }

    fn list_recent_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<Json>> {
        let rows = inventory_transactions::table
            .filter(inventory_transactions::account_id.eq(account.to_string()))
            .order((
                inventory_transactions::created_at.desc(),
                inventory_transactions::transaction_id.desc(),
            ))
            .limit(i64::from(limit))
            .select(LEDGER_COLUMNS)
            .load::<LedgerRow>(conn)?;
        rows.into_iter()
            .map(|row| {
                let record = ledger(row)?;
                // The projection's order (Mongo returns the stored document's)
                let mut out = Map::new();
                out.insert("transactionId".into(), record.transaction_id.into());
                out.insert("characterId".into(), record.character_id.into());
                out.insert("status".into(), record.status.as_str().into());
                out.insert("createdAt".into(), record.created_at.into());
                if let Some(source) = record.economy_source {
                    out.insert("economySource".into(), source.into());
                }
                if let Some(session) = record.economy_session_id {
                    out.insert("economySessionId".into(), session.into());
                }
                if let Some(at) = record.completed_at {
                    out.insert("completedAt".into(), at.into());
                }
                Ok(Value::Object(out))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn open_reforge() -> WeaponReforge {
        WeaponReforge {
            reforge_id: "r1".into(),
            open: true,
            prior_progress: 31275,
            cap: 27775,
            banked: 3500,
            banked_restored: 0,
            prestige_granted: false,
            opened_at: "2026-10-11T07:00:00.000Z".into(),
            closed_at: None,
        }
    }

    #[test]
    fn the_reforge_filter_and_set_read_node_field_names() {
        let reforge = open_reforge();
        assert!(reforge_matches(
            &reforge,
            &json!({"status": "open", "prestigeGranted": false, "bankedRestored": 0})
        ));
        assert!(!reforge_matches(&reforge, &json!({"status": "closed"})));
        assert!(!reforge_matches(&reforge, &json!({"bankedRestored": 1})));
        assert!(!reforge_matches(&reforge, &json!({"unknown": 1})));
        let closed = reforge_with(
            &reforge,
            &json!({"status": "closed", "closedAt": "2026-10-11T07:00:01.000Z", "prestigeGranted": true, "bankedRestored": 3500}),
        );
        assert!(!closed.open && closed.prestige_granted);
        assert_eq!(
            (closed.banked_restored, closed.closed_at.as_deref()),
            (3500, Some("2026-10-11T07:00:01.000Z"))
        );
        assert_eq!(closed.reforge_id, "r1");
        // the compare-and-set: the id and the expected fields must hold, and the set must change something
        let set = json!({"prestigeGranted": true});
        assert!(apply_reforge_update(&reforge, "r1", &json!({"status": "open"}), &set).is_some());
        assert!(apply_reforge_update(&reforge, "other", &json!({"status": "open"}), &set).is_none());
        assert!(apply_reforge_update(&reforge, "r1", &json!({"status": "closed"}), &set).is_none());
        assert!(apply_reforge_update(&reforge, "r1", &json!({}), &json!({"prestigeGranted": false})).is_none());
    }
}
