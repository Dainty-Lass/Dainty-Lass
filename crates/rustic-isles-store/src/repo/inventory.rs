// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Inventories and POST /inventory's idempotency ledger (spec §9).

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Text};
use serde_json::{Map, Value};
use service_kit::jsjson;

use crate::sql::{account_id, limit_clause, opt_account_id, sql};
use crate::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRecord {
    pub character_id: String,
    /// None on rows from before it was tracked
    pub account: Option<AccountId>,
    pub instanced_items: String,
    pub stacked_items: String,
    pub revision: Option<i64>,
    pub bootstrap_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerStatus {
    Pending,
    Completed,
}

impl LedgerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
        }
    }

    /// Anything but `completed` reads pending.
    pub fn parse(text: &str) -> Self {
        if text == "completed" {
            Self::Completed
        } else {
            Self::Pending
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InventoryTransactionRecord {
    pub transaction_id: String,
    pub account: AccountId,
    pub character_id: String,
    pub request_hash: Option<String>,
    pub status: LedgerStatus,
    pub result: Option<Json>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub economy_source: Option<String>,
    pub economy_session_id: Option<String>,
    pub extra: Option<Json>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TryBegin {
    Begun,
    Existing(Box<InventoryTransactionRecord>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerKey<'a> {
    pub account: &'a AccountId,
    pub character_id: &'a str,
    pub transaction_id: &'a str,
}

repository! {
    pub trait InventoryRepository = "inventories" {
        /// Any account's row: the caller checks ownership
        fn find_by_character_id(&self, conn: &mut AnyConnection, character_id: &str) -> Option<InventoryRecord>;
        fn create(&self, tx: &mut Tx<'_>, record: &InventoryRecord) -> ();
        fn update_both_if_revision_matches(
            &self,
            tx: &mut Tx<'_>,
            character_id: &str,
            instanced: &str,
            stacked: &str,
            expected: i64,
        ) -> Option<InventoryRecord>;
    }
}

repository! {
    pub trait InventoryTransactionRepository = "inventoryTransactions" {
        fn try_begin(&self, tx: &mut Tx<'_>, key: LedgerKey<'_>, request_hash: Option<&str>, created_at: &str) -> TryBegin;
        fn complete(
            &self,
            tx: &mut Tx<'_>,
            key: LedgerKey<'_>,
            result: &Json,
            extra_fields: Option<&Json>,
            completed_at: &str,
        ) -> ();
        fn find_record(&self, conn: &mut AnyConnection, key: LedgerKey<'_>) -> Option<InventoryTransactionRecord>;
        fn list_recent_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32) -> Vec<Json>;
    }
}

// ------------------------------------------------------------------ inventories

pub struct SqlInventories;

#[derive(QueryableByName)]
struct InventoryRow {
    #[diesel(sql_type = Text)]
    character_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    account_id: Option<String>,
    #[diesel(sql_type = Text)]
    instanced_items: String,
    #[diesel(sql_type = Text)]
    stacked_items: String,
    #[diesel(sql_type = BigInt)]
    revision: i64,
    #[diesel(sql_type = Nullable<Text>)]
    bootstrap_version: Option<String>,
}

impl InventoryRow {
    fn decode(self) -> RepoResult<InventoryRecord> {
        Ok(InventoryRecord {
            account: opt_account_id(self.account_id.as_deref(), "inventories.account_id")?,
            character_id: self.character_id,
            instanced_items: self.instanced_items,
            stacked_items: self.stacked_items,
            revision: Some(self.revision),
            bootstrap_version: self.bootstrap_version,
        })
    }
}

fn find_inventory(conn: &mut AnyConnection, character_id: &str) -> RepoResult<Option<InventoryRecord>> {
    sql("SELECT * FROM inventories WHERE character_id = ? AND deleted_at IS NULL")
        .t(character_id)
        .first::<InventoryRow>(conn)?
        .map(InventoryRow::decode)
        .transpose()
}

impl InventoryRepository for SqlInventories {
    fn find_by_character_id(
        &self,
        conn: &mut AnyConnection,
        character_id: &str,
    ) -> RepoResult<Option<InventoryRecord>> {
        find_inventory(conn, character_id)
    }

    fn create(&self, tx: &mut Tx<'_>, r: &InventoryRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO inventories (character_id, account_id, instanced_items, stacked_items, revision, \
             bootstrap_version) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.character_id)
        .ot(r.account.as_ref().map(ToString::to_string))
        .t(&*r.instanced_items)
        .t(&*r.stacked_items)
        .i(r.revision.unwrap_or(0))
        .ot(r.bootstrap_version.clone())
        .run(tx.conn())?;
        Ok(())
    }

    fn update_both_if_revision_matches(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        instanced: &str,
        stacked: &str,
        expected: i64,
    ) -> RepoResult<Option<InventoryRecord>> {
        let conn = tx.conn();
        let changed = sql(
            "UPDATE inventories SET instanced_items = ?, stacked_items = ?, revision = ? \
             WHERE character_id = ? AND deleted_at IS NULL AND revision = ?",
        )
        .t(instanced)
        .t(stacked)
        .i(expected.saturating_add(1))
        .t(character_id)
        .i(expected)
        .run(conn)?;
        if changed == 0 {
            return Ok(None);
        }
        find_inventory(conn, character_id)
    }
}

// ------------------------------------------------------------------ ledger

pub struct SqlInventoryTransactions;

#[derive(QueryableByName)]
struct LedgerRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    character_id: String,
    #[diesel(sql_type = Text)]
    transaction_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    request_hash: Option<String>,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Nullable<Text>)]
    result: Option<String>,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    completed_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    economy_source: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    economy_session_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    extra: Option<String>,
}

fn parse_column(text: Option<&str>, column: &str) -> RepoResult<Option<Json>> {
    text.map(|t| serde_json::from_str(t).map_err(|e| RepoError::data(format!("inventory_transactions.{column}: {e}"))))
        .transpose()
}

impl LedgerRow {
    fn decode(self) -> RepoResult<InventoryTransactionRecord> {
        Ok(InventoryTransactionRecord {
            account: account_id(&self.account_id, "inventory_transactions.account_id")?,
            result: parse_column(self.result.as_deref(), "result")?,
            extra: parse_column(self.extra.as_deref(), "extra")?,
            transaction_id: self.transaction_id,
            character_id: self.character_id,
            request_hash: self.request_hash,
            status: LedgerStatus::parse(&self.status),
            created_at: self.created_at,
            completed_at: self.completed_at,
            economy_source: self.economy_source,
            economy_session_id: self.economy_session_id,
        })
    }
}

fn find_ledger(conn: &mut AnyConnection, key: LedgerKey<'_>) -> RepoResult<Option<LedgerRow>> {
    sql("SELECT * FROM inventory_transactions WHERE account_id = ? AND character_id = ? AND transaction_id = ?")
        .t(key.account.to_string())
        .t(key.character_id)
        .t(key.transaction_id)
        .first::<LedgerRow>(conn)
}

impl InventoryTransactionRepository for SqlInventoryTransactions {
    fn try_begin(
        &self,
        tx: &mut Tx<'_>,
        key: LedgerKey<'_>,
        request_hash: Option<&str>,
        created_at: &str,
    ) -> RepoResult<TryBegin> {
        let conn = tx.conn();
        if let Some(row) = find_ledger(conn, key)? {
            return Ok(TryBegin::Existing(Box::new(row.decode()?)));
        }
        sql(
            "INSERT INTO inventory_transactions (account_id, character_id, transaction_id, request_hash, status, \
             created_at) VALUES (?, ?, ?, ?, 'pending', ?)",
        )
        .t(key.account.to_string())
        .t(key.character_id)
        .t(key.transaction_id)
        .ot(request_hash)
        .t(created_at)
        .run(conn)?;
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
        let mut source = None;
        let mut session = None;
        let mut rest = Map::new();
        if let Some(Value::Object(fields)) = extra_fields {
            for (name, value) in fields {
                match name.as_str() {
                    "economySource" => source = value.as_str().map(str::to_owned),
                    "economySessionId" => session = value.as_str().map(str::to_owned),
                    "status" | "result" | "completedAt" => {}
                    _ => {
                        rest.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        let extra = (!rest.is_empty()).then(|| jsjson::stringify(&Value::Object(rest)));
        sql(
            "UPDATE inventory_transactions SET status = 'completed', result = ?, completed_at = ?, \
             economy_source = ?, economy_session_id = ?, extra = ? \
             WHERE account_id = ? AND character_id = ? AND transaction_id = ?",
        )
        .t(jsjson::stringify(result))
        .t(completed_at)
        .ot(source)
        .ot(session)
        .ot(extra)
        .t(key.account.to_string())
        .t(key.character_id)
        .t(key.transaction_id)
        .run(tx.conn())?;
        Ok(())
    }

    fn find_record(
        &self,
        conn: &mut AnyConnection,
        key: LedgerKey<'_>,
    ) -> RepoResult<Option<InventoryTransactionRecord>> {
        find_ledger(conn, key)?.map(LedgerRow::decode).transpose()
    }

    fn list_recent_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<Json>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let rows = sql(format!(
            "SELECT * FROM inventory_transactions WHERE account_id = ? \
             ORDER BY created_at DESC, transaction_id DESC{}",
            limit_clause(limit.into())
        ))
        .t(account.to_string())
        .load::<LedgerRow>(conn)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let mut out = Map::new();
                out.insert("transactionId".into(), row.transaction_id.into());
                out.insert("characterId".into(), row.character_id.into());
                out.insert("status".into(), LedgerStatus::parse(&row.status).as_str().into());
                out.insert("createdAt".into(), row.created_at.into());
                if let Some(v) = row.economy_source {
                    out.insert("economySource".into(), v.into());
                }
                if let Some(v) = row.economy_session_id {
                    out.insert("economySessionId".into(), v.into());
                }
                if let Some(v) = row.completed_at {
                    out.insert("completedAt".into(), v.into());
                }
                Value::Object(out)
            })
            .collect())
    }
}
