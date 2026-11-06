// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Bounty and cooldown state, entitlements, wallets and the economy rollups (spec §11.1).

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use serde_json::{Map, Value};

use crate::sql::{account_id, sql};
use crate::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx};

/// This file's soft-delete tables.
pub const SOFT_DELETE_TABLES: &[&str] = &[
    "bounty_states",
    "cooldown_states",
    "entitlements",
    "wallets",
    "wallet_balances",
];

/// Which draft a stored bounty belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BountyGroup {
    /// The base draft.
    Base,
    /// The daily draft.
    Daily,
    /// The weekly season challenges.
    Weekly,
    /// An imported row with no group; it is inferred from the bounty id.
    Unassigned,
}

impl BountyGroup {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Unassigned => "unassigned",
        }
    }

    /// The group a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "base" => Some(Self::Base),
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "unassigned" => Some(Self::Unassigned),
            _ => None,
        }
    }
}

/// The seasonal coin credited for a claimed challenge (one per claim).
#[derive(Debug, Clone, PartialEq)]
pub struct RewardCredit {
    /// The coin's catalog id.
    pub catalog_id: String,
    /// The amount credited.
    pub amount: f64,
    /// The inventory transaction that carried it.
    pub transaction_id: String,
    /// When it was credited, as ISO text.
    pub at: String,
}

/// One bounty in the state.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredBounty {
    /// The draft it belongs to.
    pub group: BountyGroup,
    /// The bounty object as the client saved it.
    pub bounty: Json,
    /// When it was first saved claimed, as ISO text.
    pub claimed_at: Option<String>,
    /// The reward credited for it, once claimed and paid.
    pub reward_credit: Option<RewardCredit>,
}

/// The client's draft objects, by group.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DraftData {
    /// `draft_data`.
    pub base: Option<Json>,
    /// `draft_data_daily`.
    pub daily: Option<Json>,
    /// `draft_data_weekly`.
    pub weekly: Option<Json>,
}

/// A `bounty_states` row: an account's bounties and drafts.
#[derive(Debug, Clone, PartialEq)]
pub struct BountyStateRecord {
    /// The account.
    pub account: AccountId,
    /// Its bounties.
    pub bounties: Vec<StoredBounty>,
    /// Its drafts.
    pub draft_data: DraftData,
    /// The game day (ISO text) whose daily reward was credited.
    pub daily_reward_credited_on: Option<String>,
    /// The game day (ISO text) whose premium draft token was granted.
    pub daily_premium_draft_granted_on: Option<String>,
    /// Bumped on every save; a save names the version it read.
    pub update_version: i64,
    /// When the row was created, as ISO text.
    pub created_at: String,
    /// When it was last saved, as ISO text.
    pub updated_at: String,
}

/// One saved cooldown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooldownEntry {
    /// The cooldown id.
    pub id: String,
    /// When it started, as the client wrote it.
    pub started_date: String,
    /// When it was last saved, as ISO text.
    pub updated_at: String,
}

/// A `cooldown_states` row: an account's cooldowns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooldownStateRecord {
    /// The account.
    pub account: AccountId,
    /// Its cooldowns.
    pub entries: Vec<CooldownEntry>,
    /// Bumped on every save; a save names the version it read.
    pub update_version: i64,
    /// When the row was created, as ISO text.
    pub created_at: String,
    /// When it was last saved, as ISO text.
    pub updated_at: String,
}

/// One granted entitlement.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitlementEntry {
    /// The entitlement's name.
    pub name: String,
    /// How long it lasts; 0 is permanent.
    pub duration: f64,
    /// When a timed entitlement started, as ISO text.
    pub activated_date: Option<String>,
    /// When it was granted, as ISO text.
    pub granted_at: Option<String>,
    /// The SKU whose purchase granted it.
    pub source_sku_id: Option<String>,
}

/// An account's live entitlements.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitlementRecord {
    /// The account.
    pub account: AccountId,
    /// Its entitlements, by name.
    pub entitlements: Vec<EntitlementEntry>,
}

/// A live wallet and its balances.
#[derive(Debug, Clone, PartialEq)]
pub struct WalletRecord {
    /// The account.
    pub account: AccountId,
    /// `(catalog id, balance)` in first-written order
    pub balances: Vec<(String, f64)>,
    /// The starter wallet version it was created from.
    pub bootstrap_version: Option<String>,
}

impl WalletRecord {
    /// The balance of a currency, when the wallet holds it.
    pub fn balance(&self, catalog_id: &str) -> Option<f64> {
        self.balances.iter().find(|(id, _)| id == catalog_id).map(|(_, b)| *b)
    }
}

/// 1 to 128 bytes of ASCII letters, digits, `_`, `.` and `-`: a name is a value, never a path.
pub fn is_valid_entitlement_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// 1 to 128 bytes of ASCII letters, digits and `_`.
pub fn is_valid_balance_catalog_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

store_trait! {
    /// Bounty states (`bounty_states`).
    pub trait BountyStateRepository = "bountyStates" {
        /// The account's live state.
        fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<BountyStateRecord>;
        /// Writes `record` if the stored version is still `expected` (None when there is no row); returns the record, or None when another save won.
        fn save_if_version(
            &self,
            tx: &mut Tx<'_>,
            record: &BountyStateRecord,
            expected: Option<i64>,
        ) -> Option<BountyStateRecord>;
    }
}

store_trait! {
    /// Cooldown states (`cooldown_states`).
    pub trait CooldownStateRepository = "cooldownStates" {
        /// The account's live state.
        fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<CooldownStateRecord>;
        /// Writes `record` if the stored version is still `expected` (None when there is no row); returns the record, or None when another save won.
        fn save_if_version(
            &self,
            tx: &mut Tx<'_>,
            record: &CooldownStateRecord,
            expected: Option<i64>,
        ) -> Option<CooldownStateRecord>;
    }
}

store_trait! {
    /// Entitlements (`entitlements`).
    pub trait EntitlementRepository = "entitlements" {
        /// The account's live entitlements; None when it has none.
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<EntitlementRecord>;
        /// Grants the entitlement unless the account has it; returns whether it did. An unsafe name is an error.
        fn grant_if_missing(&self, tx: &mut Tx<'_>, account: &AccountId, entry: &EntitlementEntry) -> bool;
    }
}

store_trait! {
    /// Wallets (`wallets`, `wallet_balances`).
    pub trait WalletRepository = "wallets" {
        /// The account's live wallet.
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<WalletRecord>;
        /// Writes a new wallet and its balances.
        fn create(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> ();
        /// Writes the wallet unless the account has one; returns the stored wallet either way.
        fn create_if_missing(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> WalletRecord;
        /// Adds `delta` to a balance; None without a wallet, or when a debit would go below 0.
        fn increment_balance(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            catalog_id: &str,
            delta: f64,
        ) -> Option<WalletRecord>;
    }
}

store_trait! {
    /// The economy monitor's daily counters (`economy_rollup_counters`).
    pub trait EconomyRollupRepository = "economyRollups" {
        /// Adds each `(dotted path, amount)` to the day's counters for the currency; returns the day's counters as one document.
        fn add_to_rollup(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            day: &str,
            currency: &str,
            increments: &[(String, f64)],
            ttl_at: i64,
        ) -> Json;
    }
}

// ------------------------------------------------------------------ bounty and cooldown state

/// [`BountyStateRepository`] in SQL.
pub struct SqlBountyStates;
/// [`CooldownStateRepository`] in SQL.
pub struct SqlCooldownStates;

fn text_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The stored `bounties` list.
pub fn decode_bounties(text: &str) -> RepoResult<Vec<StoredBounty>> {
    let not_a_list = || RepoError::data("bountyStates.bounties is not a list");
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(text) else {
        return Err(not_a_list());
    };
    entries
        .into_iter()
        .map(|entry| {
            let group = entry
                .get("group")
                .and_then(Value::as_str)
                .and_then(BountyGroup::parse)
                .ok_or_else(|| RepoError::data("bountyStates: an entry has no group"))?;
            let bounty = entry
                .get("bounty")
                .cloned()
                .ok_or_else(|| RepoError::data("bountyStates: an entry has no bounty"))?;
            let reward_credit = entry
                .get("rewardCredit")
                .filter(|c| c.is_object())
                .map(|c| RewardCredit {
                    catalog_id: text_field(c, "catalogId").unwrap_or_default(),
                    amount: c.get("amount").and_then(Value::as_f64).unwrap_or(0.0),
                    transaction_id: text_field(c, "transactionId").unwrap_or_default(),
                    at: text_field(c, "at").unwrap_or_default(),
                });
            Ok(StoredBounty {
                group,
                bounty,
                claimed_at: text_field(&entry, "claimedAt"),
                reward_credit,
            })
        })
        .collect()
}

fn encode_bounties(bounties: &[StoredBounty]) -> String {
    let list: Vec<Value> = bounties
        .iter()
        .map(|b| {
            let mut out = Map::new();
            out.insert("group".into(), b.group.as_str().into());
            out.insert("bounty".into(), b.bounty.clone());
            if let Some(at) = &b.claimed_at {
                out.insert("claimedAt".into(), at.clone().into());
            }
            if let Some(credit) = &b.reward_credit {
                let mut c = Map::new();
                c.insert("catalogId".into(), credit.catalog_id.clone().into());
                c.insert("amount".into(), credit.amount.into());
                c.insert("transactionId".into(), credit.transaction_id.clone().into());
                c.insert("at".into(), credit.at.clone().into());
                out.insert("rewardCredit".into(), Value::Object(c));
            }
            Value::Object(out)
        })
        .collect();
    Value::Array(list).to_string()
}

/// The stored `draft_data` object: a value that is no object reads as no drafts.
pub fn decode_draft_data(text: &str) -> RepoResult<DraftData> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| RepoError::data(format!("bountyStates.draft_data: {e}")))?;
    Ok(DraftData {
        base: value.get("base").cloned(),
        daily: value.get("daily").cloned(),
        weekly: value.get("weekly").cloned(),
    })
}

fn encode_draft_data(draft: &DraftData) -> String {
    let mut out = Map::new();
    for (key, value) in [
        ("base", &draft.base),
        ("daily", &draft.daily),
        ("weekly", &draft.weekly),
    ] {
        if let Some(v) = value {
            out.insert(key.into(), v.clone());
        }
    }
    Value::Object(out).to_string()
}

/// The stored cooldown `entries` list.
pub fn decode_cooldowns(text: &str) -> RepoResult<Vec<CooldownEntry>> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(text) else {
        return Err(RepoError::data("cooldownStates.entries is not a list"));
    };
    entries
        .into_iter()
        .map(|entry| {
            Ok(CooldownEntry {
                id: text_field(&entry, "id").ok_or_else(|| RepoError::data("a cooldown has no id"))?,
                started_date: text_field(&entry, "startedDate").unwrap_or_default(),
                updated_at: text_field(&entry, "updatedAt").unwrap_or_default(),
            })
        })
        .collect()
}

fn encode_cooldowns(entries: &[CooldownEntry]) -> String {
    let list: Vec<Value> = entries
        .iter()
        .map(|e| {
            let mut out = Map::new();
            out.insert("id".into(), e.id.clone().into());
            out.insert("startedDate".into(), e.started_date.clone().into());
            out.insert("updatedAt".into(), e.updated_at.clone().into());
            Value::Object(out)
        })
        .collect();
    Value::Array(list).to_string()
}

#[derive(QueryableByName)]
struct BountyRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    bounties: String,
    #[diesel(sql_type = Text)]
    draft_data: String,
    #[diesel(sql_type = Nullable<Text>)]
    daily_reward_credited_on: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    daily_premium_draft_granted_on: Option<String>,
    #[diesel(sql_type = BigInt)]
    update_version: i64,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

impl BountyStateRepository for SqlBountyStates {
    fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<BountyStateRecord>> {
        let Some(row) = sql("SELECT * FROM bounty_states WHERE account_id = ? AND deleted_at IS NULL")
            .t(account.to_string())
            .first::<BountyRow>(conn)?
        else {
            return Ok(None);
        };
        Ok(Some(BountyStateRecord {
            account: account_id(&row.account_id, "bounty_states.account_id")?,
            bounties: decode_bounties(&row.bounties)?,
            draft_data: decode_draft_data(&row.draft_data)?,
            daily_reward_credited_on: row.daily_reward_credited_on,
            daily_premium_draft_granted_on: row.daily_premium_draft_granted_on,
            update_version: row.update_version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }

    fn save_if_version(
        &self,
        tx: &mut Tx<'_>,
        r: &BountyStateRecord,
        expected: Option<i64>,
    ) -> RepoResult<Option<BountyStateRecord>> {
        let written = match expected {
            // A conflict with any row, a soft-deleted one included, writes nothing (spec §16 item 4)
            None => sql(
                "INSERT INTO bounty_states (account_id, bounties, draft_data, daily_reward_credited_on, \
                 daily_premium_draft_granted_on, update_version, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
            )
            .t(r.account.to_string())
            .t(encode_bounties(&r.bounties))
            .t(encode_draft_data(&r.draft_data))
            .ot(r.daily_reward_credited_on.clone())
            .ot(r.daily_premium_draft_granted_on.clone())
            .i(r.update_version)
            .t(&*r.created_at)
            .t(&*r.updated_at)
            .run(tx.conn())?,
            Some(version) => sql(
                "UPDATE bounty_states SET bounties = ?, draft_data = ?, daily_reward_credited_on = ?, \
                 daily_premium_draft_granted_on = ?, update_version = ?, updated_at = ? \
                 WHERE account_id = ? AND deleted_at IS NULL AND update_version = ?",
            )
            .t(encode_bounties(&r.bounties))
            .t(encode_draft_data(&r.draft_data))
            .ot(r.daily_reward_credited_on.clone())
            .ot(r.daily_premium_draft_granted_on.clone())
            .i(r.update_version)
            .t(&*r.updated_at)
            .t(r.account.to_string())
            .i(version)
            .run(tx.conn())?,
        };
        Ok((written == 1).then(|| r.clone()))
    }
}

#[derive(QueryableByName)]
struct CooldownRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    entries: String,
    #[diesel(sql_type = BigInt)]
    update_version: i64,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

impl CooldownStateRepository for SqlCooldownStates {
    fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<CooldownStateRecord>> {
        let Some(row) = sql("SELECT * FROM cooldown_states WHERE account_id = ? AND deleted_at IS NULL")
            .t(account.to_string())
            .first::<CooldownRow>(conn)?
        else {
            return Ok(None);
        };
        Ok(Some(CooldownStateRecord {
            account: account_id(&row.account_id, "cooldown_states.account_id")?,
            entries: decode_cooldowns(&row.entries)?,
            update_version: row.update_version,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }

    fn save_if_version(
        &self,
        tx: &mut Tx<'_>,
        r: &CooldownStateRecord,
        expected: Option<i64>,
    ) -> RepoResult<Option<CooldownStateRecord>> {
        let written = match expected {
            None => sql(
                "INSERT INTO cooldown_states (account_id, entries, update_version, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
            )
            .t(r.account.to_string())
            .t(encode_cooldowns(&r.entries))
            .i(r.update_version)
            .t(&*r.created_at)
            .t(&*r.updated_at)
            .run(tx.conn())?,
            Some(version) => sql(
                "UPDATE cooldown_states SET entries = ?, update_version = ?, updated_at = ? \
                 WHERE account_id = ? AND deleted_at IS NULL AND update_version = ?",
            )
            .t(encode_cooldowns(&r.entries))
            .i(r.update_version)
            .t(&*r.updated_at)
            .t(r.account.to_string())
            .i(version)
            .run(tx.conn())?,
        };
        Ok((written == 1).then(|| r.clone()))
    }
}

// ------------------------------------------------------------------ entitlements

/// [`EntitlementRepository`] in SQL.
pub struct SqlEntitlements;

#[derive(QueryableByName)]
struct EntitlementRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Double)]
    duration: f64,
    #[diesel(sql_type = Nullable<Text>)]
    activated_date: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    granted_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source_sku_id: Option<String>,
}

impl EntitlementRepository for SqlEntitlements {
    fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<EntitlementRecord>> {
        let entitlements: Vec<EntitlementEntry> =
            sql("SELECT * FROM entitlements WHERE account_id = ? AND deleted_at IS NULL ORDER BY name ASC")
                .t(account.to_string())
                .load::<EntitlementRow>(conn)?
                .into_iter()
                .map(|r| EntitlementEntry {
                    name: r.name,
                    duration: r.duration,
                    activated_date: r.activated_date,
                    granted_at: r.granted_at,
                    source_sku_id: r.source_sku_id,
                })
                .collect();
        Ok((!entitlements.is_empty()).then(|| EntitlementRecord {
            account: account.clone(),
            entitlements,
        }))
    }

    fn grant_if_missing(&self, tx: &mut Tx<'_>, account: &AccountId, e: &EntitlementEntry) -> RepoResult<bool> {
        if !is_valid_entitlement_name(&e.name) {
            return Err(RepoError::data(format!("unsafe entitlement name \"{}\"", e.name)));
        }
        let inserted = sql(
            "INSERT INTO entitlements (account_id, name, duration, activated_date, granted_at, \
             source_sku_id) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .t(account.to_string())
        .t(&*e.name)
        .f(e.duration)
        .ot(e.activated_date.clone())
        .ot(e.granted_at.clone())
        .ot(e.source_sku_id.clone())
        .run(tx.conn())?;
        Ok(inserted == 1)
    }
}

// ------------------------------------------------------------------ wallets

/// [`WalletRepository`] in SQL.
pub struct SqlWallets;

#[derive(QueryableByName)]
struct WalletRow {
    #[diesel(sql_type = Nullable<Text>)]
    bootstrap_version: Option<String>,
}

#[derive(QueryableByName)]
struct BalanceRow {
    #[diesel(sql_type = Text)]
    catalog_id: String,
    #[diesel(sql_type = Double)]
    balance: f64,
}

fn check_catalog_id(id: &str) -> RepoResult<()> {
    if is_valid_balance_catalog_id(id) {
        Ok(())
    } else {
        Err(RepoError::data(format!("unsafe balance catalogId \"{id}\"")))
    }
}

fn find_wallet(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<WalletRecord>> {
    let Some(wallet) = sql("SELECT bootstrap_version FROM wallets WHERE account_id = ? AND deleted_at IS NULL")
        .t(account.to_string())
        .first::<WalletRow>(conn)?
    else {
        return Ok(None);
    };
    let balances = sql(
        "SELECT catalog_id, balance FROM wallet_balances WHERE account_id = ? AND deleted_at IS NULL \
         ORDER BY ordinal ASC",
    )
    .t(account.to_string())
    .load::<BalanceRow>(conn)?
    .into_iter()
    .map(|b| (b.catalog_id, b.balance))
    .collect();
    Ok(Some(WalletRecord {
        account: account.clone(),
        balances,
        bootstrap_version: wallet.bootstrap_version,
    }))
}

fn insert_balances(conn: &mut AnyConnection, wallet: &WalletRecord) -> RepoResult<()> {
    for (ordinal, (catalog_id, balance)) in (1i64..).zip(&wallet.balances) {
        check_catalog_id(catalog_id)?;
        sql("INSERT INTO wallet_balances (account_id, catalog_id, balance, ordinal) VALUES (?, ?, ?, ?)")
            .t(wallet.account.to_string())
            .t(&**catalog_id)
            .f(*balance)
            .i(ordinal)
            .run(conn)?;
    }
    Ok(())
}

impl WalletRepository for SqlWallets {
    fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<WalletRecord>> {
        find_wallet(conn, account)
    }

    fn create(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> RepoResult<()> {
        let conn = tx.conn();
        sql("INSERT INTO wallets (account_id, bootstrap_version) VALUES (?, ?)")
            .t(wallet.account.to_string())
            .ot(wallet.bootstrap_version.clone())
            .run(conn)?;
        insert_balances(conn, wallet)
    }

    fn create_if_missing(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> RepoResult<WalletRecord> {
        let conn = tx.conn();
        let inserted = sql("INSERT INTO wallets (account_id, bootstrap_version) VALUES (?, ?) ON CONFLICT DO NOTHING")
            .t(wallet.account.to_string())
            .ot(wallet.bootstrap_version.clone())
            .run(conn)?;
        if inserted == 1 {
            insert_balances(conn, wallet)?;
        }
        // A soft-deleted wallet holds the key (spec §16 item 5)
        find_wallet(conn, &wallet.account)?.ok_or_else(|| RepoError::data("a wallet vanished"))
    }

    fn increment_balance(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        catalog_id: &str,
        delta: f64,
    ) -> RepoResult<Option<WalletRecord>> {
        check_catalog_id(catalog_id)?;
        let conn = tx.conn();
        if find_wallet(conn, account)?.is_none() {
            return Ok(None);
        }
        let update = "UPDATE wallet_balances SET balance = balance + ? \
                      WHERE account_id = ? AND catalog_id = ? AND deleted_at IS NULL";
        let applied = if delta < 0.0 {
            sql(format!("{update} AND balance >= ?"))
                .f(delta)
                .t(account.to_string())
                .t(catalog_id)
                .f(-delta)
                .run(conn)?
        } else {
            sql(update).f(delta).t(account.to_string()).t(catalog_id).run(conn)?
        };
        if applied == 0 {
            if delta < 0.0 {
                return Ok(None);
            }
            let ordinal = sql("SELECT MAX(ordinal) AS n FROM wallet_balances WHERE account_id = ?")
                .t(account.to_string())
                .int(conn)?
                + 1;
            // A soft-deleted row of that id takes the credit and stays deleted (spec §16 item 6)
            sql("INSERT INTO wallet_balances (account_id, catalog_id, balance, ordinal) VALUES (?, ?, ?, ?) \
                 ON CONFLICT (account_id, catalog_id) DO UPDATE SET balance = wallet_balances.balance + excluded.balance")
            .t(account.to_string())
            .t(catalog_id)
            .f(delta)
            .i(ordinal)
            .run(conn)?;
        }
        find_wallet(conn, account)
    }
}

// ------------------------------------------------------------------ rollups

/// [`EconomyRollupRepository`] in SQL.
pub struct SqlEconomyRollups;

#[derive(QueryableByName)]
struct CounterRow {
    #[diesel(sql_type = Text)]
    path: String,
    #[diesel(sql_type = Double)]
    value: f64,
}

/// Counters nested by `.` (`bySource.STORE.debited` is `{"bySource":{"STORE":{"debited":v}}}`); a non-object met
/// on the way becomes an object. Always an object.
pub fn rollup_document(counters: &[(String, f64)]) -> Json {
    let mut root = Map::new();
    for (path, value) in counters {
        set_path(&mut root, path, Value::from(*value));
    }
    Value::Object(root)
}

/// Puts `value` at the dotted `path` inside `root`, replacing any non-object on the way.
pub(crate) fn set_path(root: &mut Map<String, Value>, path: &str, value: Value) {
    let mut parts = path.split('.').peekable();
    let mut node = root;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            node.insert(part.to_owned(), value);
            return;
        }
        let entry = node.entry(part.to_owned()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let Value::Object(next) = entry else { return };
        node = next;
    }
}

impl EconomyRollupRepository for SqlEconomyRollups {
    fn add_to_rollup(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        day: &str,
        currency: &str,
        increments: &[(String, f64)],
        ttl_at: i64,
    ) -> RepoResult<Json> {
        let conn = tx.conn();
        for (path, amount) in increments {
            sql(
                "INSERT INTO economy_rollup_counters (account_id, day, currency, path, value, ttl_at) \
                 VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (account_id, day, currency, path) DO UPDATE \
                 SET value = economy_rollup_counters.value + excluded.value, ttl_at = excluded.ttl_at",
            )
            .t(account.to_string())
            .t(day)
            .t(currency)
            .t(&**path)
            .f(*amount)
            .i(ttl_at)
            .run(conn)?;
        }
        let counters: Vec<(String, f64)> = sql(
            "SELECT path, value FROM economy_rollup_counters WHERE account_id = ? AND day = ? AND currency = ? \
             ORDER BY path ASC",
        )
        .t(account.to_string())
        .t(day)
        .t(currency)
        .load::<CounterRow>(conn)?
        .into_iter()
        .map(|c| (c.path, c.value))
        .collect();
        Ok(rollup_document(&counters))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_catalog_ids() {
        assert!(is_valid_entitlement_name("exchange_slot_2"));
        assert!(is_valid_entitlement_name("a.b-c"));
        assert!(!is_valid_entitlement_name("bad name"));
        assert!(!is_valid_entitlement_name(""));
        assert!(!is_valid_entitlement_name(&"x".repeat(129)));
        assert!(is_valid_balance_catalog_id("CURRENCY_RAMS"));
        assert!(!is_valid_balance_catalog_id("a.b"));
    }

    #[test]
    fn rollups_nest_by_dots() {
        let doc = rollup_document(&[
            ("a".into(), 1.0),
            ("a.b".into(), 2.0),
            ("bySource.STORE.debited".into(), 42.0),
        ]);
        assert_eq!(
            doc.to_string(),
            r#"{"a":{"b":2.0},"bySource":{"STORE":{"debited":42.0}}}"#
        );
    }

    #[test]
    fn stored_lists_decode() {
        assert_eq!(
            decode_bounties("{}").unwrap_err().to_string(),
            "stored data: bountyStates.bounties is not a list"
        );
        assert_eq!(
            decode_bounties(r#"[{"bounty":{}}]"#).unwrap_err().to_string(),
            "stored data: bountyStates: an entry has no group"
        );
        assert_eq!(
            decode_bounties(r#"[{"group":"base"}]"#).unwrap_err().to_string(),
            "stored data: bountyStates: an entry has no bounty"
        );
        let read = decode_bounties(r#"[{"group":"daily","bounty":1,"rewardCredit":{}}]"#).unwrap();
        assert_eq!(read[0].reward_credit.as_ref().map(|c| c.amount), Some(0.0));
        assert_eq!(
            decode_cooldowns(r#"[{"startedDate":"x"}]"#).unwrap_err().to_string(),
            "stored data: a cooldown has no id"
        );
        assert_eq!(decode_cooldowns(r#"[{"id":"c"}]"#).unwrap()[0].updated_at, "");
    }
}
