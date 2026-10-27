//! RU5 bounties, store, Lady Luck and platinum: bounty and cooldown state, entitlements, wallets, economy rollups
//! (`migrations/*/0050_bounties_store`).
//!
//! Node's one document per account becomes a row per account (bounty and cooldown state, the lists as JSON text, since
//! the game's objects are kept verbatim) or a row per entry (entitlements, balances). Optimistic writes keep Node's
//! `saveIfVersion`: an insert that finds the row, or an update that finds another version, answers `None`.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use serde_json::{Map, Value};

use super::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx, repository};

/// A bound value of [`insert_or`]'s statements.
enum Bind {
    Text(String),
    OptText(Option<String>),
    Big(i64),
    Double(f64),
}

/// `INSERT ... ON CONFLICT ..` is the same SQL on SQLite and Postgres, but Diesel's `MultiBackend` has no upsert DSL.
/// Placeholders are `$1`, `$2`.. as `store` writes them.
fn execute(conn: &mut AnyConnection, sql: &str, binds: Vec<Bind>) -> RepoResult<usize> {
    let mut query = diesel::sql_query(sql).into_boxed::<::store::MultiBackend>();
    for bind in binds {
        query = match bind {
            Bind::Text(v) => query.bind::<Text, _>(v),
            Bind::OptText(v) => query.bind::<Nullable<Text>, _>(v),
            Bind::Big(v) => query.bind::<BigInt, _>(v),
            Bind::Double(v) => query.bind::<Double, _>(v),
        };
    }
    Ok(query.execute(conn)?)
}

fn data_error(what: &str, error: impl std::fmt::Display) -> RepoError {
    RepoError::Data(format!("{what}: {error}"))
}

fn json_text(value: &Value) -> String {
    value.to_string()
}

fn parse_json(what: &str, text: &str) -> RepoResult<Value> {
    serde_json::from_str(text).map_err(|e| data_error(what, e))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BountyGroup {
    Base,
    Daily,
    Weekly,
    Unassigned,
}

impl BountyGroup {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Unassigned => "unassigned",
        }
    }

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

/// The one currency grant a claim funded.
#[derive(Debug, Clone, PartialEq)]
pub struct RewardCredit {
    pub catalog_id: String,
    pub amount: f64,
    pub transaction_id: String,
    pub at: String,
}

/// `StoredBounty`: the game's bounty object verbatim, the group it is filed under and the claim's metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredBounty {
    pub group: BountyGroup,
    pub bounty: Json,
    /// Backend time this instance was first seen claimed
    pub claimed_at: Option<String>,
    pub reward_credit: Option<RewardCredit>,
}

impl StoredBounty {
    fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("group".into(), Value::from(self.group.as_str()));
        map.insert("bounty".into(), self.bounty.clone());
        if let Some(at) = &self.claimed_at {
            map.insert("claimedAt".into(), Value::from(at.as_str()));
        }
        if let Some(credit) = &self.reward_credit {
            map.insert(
                "rewardCredit".into(),
                serde_json::json!({
                    "catalogId": credit.catalog_id,
                    "amount": credit.amount,
                    "transactionId": credit.transaction_id,
                    "at": credit.at,
                }),
            );
        }
        Value::Object(map)
    }

    fn from_json(value: &Value) -> RepoResult<Self> {
        let group = value
            .get("group")
            .and_then(Value::as_str)
            .and_then(BountyGroup::parse)
            .ok_or_else(|| RepoError::Data("bountyStates: an entry has no group".into()))?;
        let bounty = value
            .get("bounty")
            .cloned()
            .ok_or_else(|| RepoError::Data("bountyStates: an entry has no bounty".into()))?;
        let credit = value.get("rewardCredit").filter(|c| c.is_object()).map(|c| {
            let text = |key: &str| c.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            RewardCredit {
                catalog_id: text("catalogId"),
                amount: c.get("amount").and_then(Value::as_f64).unwrap_or_default(),
                transaction_id: text("transactionId"),
                at: text("at"),
            }
        });
        Ok(Self {
            group,
            bounty,
            claimed_at: value.get("claimedAt").and_then(Value::as_str).map(str::to_owned),
            reward_credit: credit,
        })
    }
}

/// The three components' draft data, each as the server last sent it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DraftData {
    pub base: Option<Json>,
    pub daily: Option<Json>,
    pub weekly: Option<Json>,
}

impl DraftData {
    fn to_json(&self) -> Value {
        let mut map = Map::new();
        for (key, value) in [("base", &self.base), ("daily", &self.daily), ("weekly", &self.weekly)] {
            if let Some(value) = value {
                map.insert(key.into(), value.clone());
            }
        }
        Value::Object(map)
    }

    fn from_json(value: &Value) -> Self {
        let get = |key: &str| value.get(key).cloned();
        Self {
            base: get("base"),
            daily: get("daily"),
            weekly: get("weekly"),
        }
    }
}

/// `BountyStateRecord`.
#[derive(Debug, Clone, PartialEq)]
pub struct BountyStateRecord {
    pub account: AccountId,
    pub bounties: Vec<StoredBounty>,
    pub draft_data: DraftData,
    /// The game day (its 17:00 UTC start) a daily challenge's coins were last paid
    pub daily_reward_credited_on: Option<String>,
    pub daily_premium_draft_granted_on: Option<String>,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

repository! {
    pub trait BountyStateRepository("bountyStates") {
        fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<BountyStateRecord>;
        /// `None` expected version inserts only; a conflict answers `None`. Returns the record as saved.
        fn save_if_version(&self, tx: &mut Tx<'_>, record: &BountyStateRecord, expected_version: Option<i64>)
            -> Option<BountyStateRecord>;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooldownEntry {
    pub id: String,
    pub started_date: String,
    pub updated_at: String,
}

/// `CooldownStateRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooldownStateRecord {
    pub account: AccountId,
    pub entries: Vec<CooldownEntry>,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

repository! {
    pub trait CooldownStateRepository("cooldownStates") {
        fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<CooldownStateRecord>;
        fn save_if_version(&self, tx: &mut Tx<'_>, record: &CooldownStateRecord, expected_version: Option<i64>)
            -> Option<CooldownStateRecord>;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntitlementEntry {
    pub name: String,
    pub duration: f64,
    pub activated_date: Option<String>,
    pub granted_at: Option<String>,
    pub source_sku_id: Option<String>,
}

/// `EntitlementRecord`: the account's entitlements, by name.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitlementRecord {
    pub account: AccountId,
    pub entitlements: Vec<EntitlementEntry>,
}

/// `IsValidEntitlementName`: a name is a value, never a path, so this is data hygiene.
pub fn is_valid_entitlement_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

repository! {
    pub trait EntitlementRepository("entitlements") {
        /// `None` when the account holds none (Node's document may exist and be empty; no answer tells them apart)
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<EntitlementRecord>;
        /// Whether it was granted now; a held name is left as it is
        fn grant_if_missing(&self, tx: &mut Tx<'_>, account: &AccountId, entry: &EntitlementEntry) -> bool;
    }
}

/// `IsValidBalanceCatalogId`: a strict identifier.
pub fn is_valid_balance_catalog_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The tables `store` soft-deletes with their account (DD23); `deleted_at` is on each, and reads filter on it.
pub const SOFT_DELETE_TABLES: [&str; 5] = [
    "bounty_states",
    "cooldown_states",
    "entitlements",
    "wallets",
    "wallet_balances",
];

/// `WalletRecord`: balances by catalog id, in the order they were first written.
#[derive(Debug, Clone, PartialEq)]
pub struct WalletRecord {
    pub account: AccountId,
    pub balances: Vec<(String, f64)>,
    pub bootstrap_version: Option<String>,
}

impl WalletRecord {
    pub fn balance(&self, catalog_id: &str) -> Option<f64> {
        self.balances.iter().find(|(id, _)| id == catalog_id).map(|(_, v)| *v)
    }
}

repository! {
    pub trait WalletRepository("wallets") {
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<WalletRecord>;
        /// A plain insert; an existing wallet is an error
        fn create(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> ();
        /// Creates the wallet only when missing; an existing one comes back unchanged (never a starter reset)
        fn create_if_missing(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> WalletRecord;
        /// Atomic: `None` when the wallet is missing or a debit would take the balance below zero
        fn increment_balance(&self, tx: &mut Tx<'_>, account: &AccountId, catalog_id: &str, delta: f64)
            -> Option<WalletRecord>;
    }
}

repository! {
    pub trait EconomyRollupRepository("economyRollups") {
        /// Adds `increments` (dotted paths such as `bySource.STORE.debited`) to the day's document of one currency and
        /// returns the document after the write; read by the monitors only, never by a grant
        fn add_to_rollup(&self, tx: &mut Tx<'_>, account: &AccountId, day: &str, currency: &str,
            increments: &[(String, f64)], ttl_at: i64) -> Json;
    }
}

diesel::table! {
    bounty_states (account_id) {
        account_id -> Text,
        bounties -> Text,
        draft_data -> Text,
        daily_reward_credited_on -> Nullable<Text>,
        daily_premium_draft_granted_on -> Nullable<Text>,
        update_version -> BigInt,
        created_at -> Text,
        updated_at -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    cooldown_states (account_id) {
        account_id -> Text,
        entries -> Text,
        update_version -> BigInt,
        created_at -> Text,
        updated_at -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    entitlements (account_id, name) {
        account_id -> Text,
        name -> Text,
        duration -> Double,
        activated_date -> Nullable<Text>,
        granted_at -> Nullable<Text>,
        source_sku_id -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    wallets (account_id) {
        account_id -> Text,
        bootstrap_version -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    wallet_balances (account_id, catalog_id) {
        account_id -> Text,
        catalog_id -> Text,
        balance -> Double,
        ordinal -> BigInt,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    economy_rollup_counters (account_id, day, currency, path) {
        account_id -> Text,
        day -> Text,
        currency -> Text,
        path -> Text,
        value -> Double,
        ttl_at -> BigInt,
    }
}

type BountyRow = (String, String, Option<String>, Option<String>, i64, String, String);

/// [`BountyStateRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlBountyStates;

fn bounty_record(account: &AccountId, row: BountyRow) -> RepoResult<BountyStateRecord> {
    let (bounties, draft_data, credited_on, premium_on, update_version, created_at, updated_at) = row;
    let list = parse_json("bountyStates.bounties", &bounties)?;
    let bounties = list
        .as_array()
        .ok_or_else(|| RepoError::Data("bountyStates.bounties is not a list".into()))?
        .iter()
        .map(StoredBounty::from_json)
        .collect::<RepoResult<Vec<_>>>()?;
    Ok(BountyStateRecord {
        account: account.clone(),
        bounties,
        draft_data: DraftData::from_json(&parse_json("bountyStates.draft_data", &draft_data)?),
        daily_reward_credited_on: credited_on,
        daily_premium_draft_granted_on: premium_on,
        update_version,
        created_at,
        updated_at,
    })
}

impl BountyStateRepository for SqlBountyStates {
    fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<BountyStateRecord>> {
        bounty_states::table
            .filter(bounty_states::account_id.eq(account.to_string()))
            .filter(bounty_states::deleted_at.is_null())
            .select((
                bounty_states::bounties,
                bounty_states::draft_data,
                bounty_states::daily_reward_credited_on,
                bounty_states::daily_premium_draft_granted_on,
                bounty_states::update_version,
                bounty_states::created_at,
                bounty_states::updated_at,
            ))
            .first::<BountyRow>(conn)
            .optional()?
            .map(|row| bounty_record(account, row))
            .transpose()
    }

    fn save_if_version(
        &self,
        tx: &mut Tx<'_>,
        record: &BountyStateRecord,
        expected_version: Option<i64>,
    ) -> RepoResult<Option<BountyStateRecord>> {
        let key = record.account.to_string();
        let list = Value::Array(record.bounties.iter().map(StoredBounty::to_json).collect());
        let (bounties, draft_data) = (json_text(&list), json_text(&record.draft_data.to_json()));
        let written = match expected_version {
            None => execute(
                tx.conn(),
                "INSERT INTO bounty_states (account_id, bounties, draft_data, daily_reward_credited_on,                  daily_premium_draft_granted_on, update_version, created_at, updated_at)                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING",
                vec![
                    Bind::Text(key.clone()),
                    Bind::Text(bounties.clone()),
                    Bind::Text(draft_data.clone()),
                    Bind::OptText(record.daily_reward_credited_on.clone()),
                    Bind::OptText(record.daily_premium_draft_granted_on.clone()),
                    Bind::Big(record.update_version),
                    Bind::Text(record.created_at.clone()),
                    Bind::Text(record.updated_at.clone()),
                ],
            )?,
            Some(expected) => diesel::update(
                bounty_states::table
                    .filter(bounty_states::account_id.eq(&key))
                    .filter(bounty_states::deleted_at.is_null())
                    .filter(bounty_states::update_version.eq(expected)),
            )
            .set((
                bounty_states::bounties.eq(&bounties),
                bounty_states::draft_data.eq(&draft_data),
                bounty_states::daily_reward_credited_on.eq(&record.daily_reward_credited_on),
                bounty_states::daily_premium_draft_granted_on.eq(&record.daily_premium_draft_granted_on),
                bounty_states::update_version.eq(record.update_version),
                bounty_states::updated_at.eq(&record.updated_at),
            ))
            .execute(tx.conn())?,
        };
        Ok((written > 0).then(|| record.clone()))
    }
}

/// [`CooldownStateRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlCooldownStates;

impl CooldownStateRepository for SqlCooldownStates {
    fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<CooldownStateRecord>> {
        let row = cooldown_states::table
            .filter(cooldown_states::account_id.eq(account.to_string()))
            .filter(cooldown_states::deleted_at.is_null())
            .select((
                cooldown_states::entries,
                cooldown_states::update_version,
                cooldown_states::created_at,
                cooldown_states::updated_at,
            ))
            .first::<(String, i64, String, String)>(conn)
            .optional()?;
        let Some((entries, update_version, created_at, updated_at)) = row else {
            return Ok(None);
        };
        let list = parse_json("cooldownStates.entries", &entries)?;
        let entries = list
            .as_array()
            .ok_or_else(|| RepoError::Data("cooldownStates.entries is not a list".into()))?
            .iter()
            .map(|entry| {
                let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
                Ok(CooldownEntry {
                    id: text("id").ok_or_else(|| RepoError::Data("a cooldown has no id".into()))?,
                    started_date: text("startedDate").unwrap_or_default(),
                    updated_at: text("updatedAt").unwrap_or_default(),
                })
            })
            .collect::<RepoResult<Vec<_>>>()?;
        Ok(Some(CooldownStateRecord {
            account: account.clone(),
            entries,
            update_version,
            created_at,
            updated_at,
        }))
    }

    fn save_if_version(
        &self,
        tx: &mut Tx<'_>,
        record: &CooldownStateRecord,
        expected_version: Option<i64>,
    ) -> RepoResult<Option<CooldownStateRecord>> {
        let key = record.account.to_string();
        let entries = json_text(&Value::Array(
            record
                .entries
                .iter()
                .map(|e| serde_json::json!({ "id": e.id, "startedDate": e.started_date, "updatedAt": e.updated_at }))
                .collect(),
        ));
        let written = match expected_version {
            None => execute(
                tx.conn(),
                "INSERT INTO cooldown_states (account_id, entries, update_version, created_at, updated_at)                  VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
                vec![
                    Bind::Text(key.clone()),
                    Bind::Text(entries.clone()),
                    Bind::Big(record.update_version),
                    Bind::Text(record.created_at.clone()),
                    Bind::Text(record.updated_at.clone()),
                ],
            )?,
            Some(expected) => diesel::update(
                cooldown_states::table
                    .filter(cooldown_states::account_id.eq(&key))
                    .filter(cooldown_states::deleted_at.is_null())
                    .filter(cooldown_states::update_version.eq(expected)),
            )
            .set((
                cooldown_states::entries.eq(&entries),
                cooldown_states::update_version.eq(record.update_version),
                cooldown_states::updated_at.eq(&record.updated_at),
            ))
            .execute(tx.conn())?,
        };
        Ok((written > 0).then(|| record.clone()))
    }
}

/// [`EntitlementRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlEntitlements;

impl EntitlementRepository for SqlEntitlements {
    fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<EntitlementRecord>> {
        let rows = entitlements::table
            .filter(entitlements::account_id.eq(account.to_string()))
            .filter(entitlements::deleted_at.is_null())
            .select((
                entitlements::name,
                entitlements::duration,
                entitlements::activated_date,
                entitlements::granted_at,
                entitlements::source_sku_id,
            ))
            .order(entitlements::name.asc())
            .load::<(String, f64, Option<String>, Option<String>, Option<String>)>(conn)?;
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(EntitlementRecord {
            account: account.clone(),
            entitlements: rows
                .into_iter()
                .map(
                    |(name, duration, activated_date, granted_at, source_sku_id)| EntitlementEntry {
                        name,
                        duration,
                        activated_date,
                        granted_at,
                        source_sku_id,
                    },
                )
                .collect(),
        }))
    }

    fn grant_if_missing(&self, tx: &mut Tx<'_>, account: &AccountId, entry: &EntitlementEntry) -> RepoResult<bool> {
        if !is_valid_entitlement_name(&entry.name) {
            return Err(RepoError::Data(format!("unsafe entitlement name {:?}", entry.name)));
        }
        let written = execute(
            tx.conn(),
            "INSERT INTO entitlements (account_id, name, duration, activated_date, granted_at, source_sku_id)              VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
            vec![
                Bind::Text(account.to_string()),
                Bind::Text(entry.name.clone()),
                Bind::Double(entry.duration),
                Bind::OptText(entry.activated_date.clone()),
                Bind::OptText(entry.granted_at.clone()),
                Bind::OptText(entry.source_sku_id.clone()),
            ],
        )?;
        Ok(written > 0)
    }
}

/// [`WalletRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlWallets;

fn load_wallet(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<WalletRecord>> {
    let key = account.to_string();
    let wallet = wallets::table
        .filter(wallets::account_id.eq(&key))
        .filter(wallets::deleted_at.is_null())
        .select(wallets::bootstrap_version)
        .first::<Option<String>>(conn)
        .optional()?;
    let Some(bootstrap_version) = wallet else {
        return Ok(None);
    };
    let balances = wallet_balances::table
        .filter(wallet_balances::account_id.eq(&key))
        .filter(wallet_balances::deleted_at.is_null())
        .select((wallet_balances::catalog_id, wallet_balances::balance))
        .order(wallet_balances::ordinal.asc())
        .load::<(String, f64)>(conn)?;
    Ok(Some(WalletRecord {
        account: account.clone(),
        balances,
        bootstrap_version,
    }))
}

fn insert_balances(tx: &mut Tx<'_>, wallet: &WalletRecord) -> RepoResult<()> {
    let key = wallet.account.to_string();
    for (ordinal, (catalog_id, balance)) in (1_i64..).zip(&wallet.balances) {
        if !is_valid_balance_catalog_id(catalog_id) {
            return Err(RepoError::Data(format!("unsafe balance catalogId {catalog_id:?}")));
        }
        diesel::insert_into(wallet_balances::table)
            .values((
                wallet_balances::account_id.eq(&key),
                wallet_balances::catalog_id.eq(catalog_id),
                wallet_balances::balance.eq(*balance),
                wallet_balances::ordinal.eq(ordinal),
            ))
            .execute(tx.conn())?;
    }
    Ok(())
}

impl WalletRepository for SqlWallets {
    fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<WalletRecord>> {
        load_wallet(conn, account)
    }

    fn create(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> RepoResult<()> {
        diesel::insert_into(wallets::table)
            .values((
                wallets::account_id.eq(wallet.account.to_string()),
                wallets::bootstrap_version.eq(&wallet.bootstrap_version),
            ))
            .execute(tx.conn())?;
        insert_balances(tx, wallet)
    }

    fn create_if_missing(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> RepoResult<WalletRecord> {
        let inserted = execute(
            tx.conn(),
            "INSERT INTO wallets (account_id, bootstrap_version) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            vec![
                Bind::Text(wallet.account.to_string()),
                Bind::OptText(wallet.bootstrap_version.clone()),
            ],
        )?;
        if inserted > 0 {
            insert_balances(tx, wallet)?;
        }
        load_wallet(tx.conn(), &wallet.account)?.ok_or_else(|| RepoError::Data("a wallet vanished".into()))
    }

    fn increment_balance(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        catalog_id: &str,
        delta: f64,
    ) -> RepoResult<Option<WalletRecord>> {
        if !is_valid_balance_catalog_id(catalog_id) {
            return Err(RepoError::Data(format!("unsafe balance catalogId {catalog_id:?}")));
        }
        let key = account.to_string();
        let exists = wallets::table
            .filter(wallets::account_id.eq(&key))
            .filter(wallets::deleted_at.is_null())
            .count()
            .get_result::<i64>(tx.conn())?
            > 0;
        if !exists {
            return Ok(None);
        }
        let row = wallet_balances::table
            .filter(wallet_balances::account_id.eq(&key))
            .filter(wallet_balances::deleted_at.is_null())
            .filter(wallet_balances::catalog_id.eq(catalog_id));
        // Guarded in the statement itself, never read-modify-write: a debit that would go below zero matches nothing
        let updated = if delta < 0.0 {
            diesel::update(row.filter(wallet_balances::balance.ge(-delta)))
                .set(wallet_balances::balance.eq(wallet_balances::balance + delta))
                .execute(tx.conn())?
        } else {
            diesel::update(row)
                .set(wallet_balances::balance.eq(wallet_balances::balance + delta))
                .execute(tx.conn())?
        };
        if updated == 0 {
            // `$inc` creates a missing field on an existing wallet; a debit of a field it lacks matches nothing
            if delta < 0.0 {
                return Ok(None);
            }
            let next = wallet_balances::table
                .filter(wallet_balances::account_id.eq(&key))
                .select(diesel::dsl::max(wallet_balances::ordinal))
                .first::<Option<i64>>(tx.conn())?
                .unwrap_or(0)
                + 1;
            execute(
                tx.conn(),
                "INSERT INTO wallet_balances (account_id, catalog_id, balance, ordinal) VALUES ($1, $2, $3, $4)                  ON CONFLICT (account_id, catalog_id) DO UPDATE SET balance = wallet_balances.balance + $5",
                vec![
                    Bind::Text(key.clone()),
                    Bind::Text(catalog_id.to_owned()),
                    Bind::Double(delta),
                    Bind::Big(next),
                    Bind::Double(delta),
                ],
            )?;
        }
        load_wallet(tx.conn(), account)
    }
}

/// [`EconomyRollupRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlEconomyRollups;

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
        let key = account.to_string();
        for (path, amount) in increments {
            execute(
                tx.conn(),
                "INSERT INTO economy_rollup_counters (account_id, day, currency, path, value, ttl_at)                  VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (account_id, day, currency, path)                  DO UPDATE SET value = economy_rollup_counters.value + $7, ttl_at = $8",
                vec![
                    Bind::Text(key.clone()),
                    Bind::Text(day.to_owned()),
                    Bind::Text(currency.to_owned()),
                    Bind::Text(path.clone()),
                    Bind::Double(*amount),
                    Bind::Big(ttl_at),
                    Bind::Double(*amount),
                    Bind::Big(ttl_at),
                ],
            )?;
        }
        let rows = economy_rollup_counters::table
            .filter(economy_rollup_counters::account_id.eq(&key))
            .filter(economy_rollup_counters::day.eq(day))
            .filter(economy_rollup_counters::currency.eq(currency))
            .select((economy_rollup_counters::path, economy_rollup_counters::value))
            .order(economy_rollup_counters::path.asc())
            .load::<(String, f64)>(tx.conn())?;
        Ok(rollup_document(&rows))
    }
}

/// The nested document the dotted paths describe (`bySource.STORE.debited` is `{bySource: {STORE: {debited}}}`).
pub fn rollup_document(rows: &[(String, f64)]) -> Json {
    let mut root = Map::new();
    for (path, value) in rows {
        let mut level = &mut root;
        let mut parts = path.split('.').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                level.insert(part.to_owned(), Value::from(*value));
                break;
            }
            let next = level
                .entry(part.to_owned())
                .or_insert_with(|| Value::Object(Map::new()));
            if !next.is_object() {
                *next = Value::Object(Map::new());
            }
            let Some(object) = next.as_object_mut() else { break };
            level = object;
        }
    }
    Value::Object(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollup_paths_nest() {
        let doc = rollup_document(&[("bySource.STORE.debited".into(), 5.0), ("credited".into(), 2.0)]);
        assert_eq!(doc["bySource"]["STORE"]["debited"], 5.0);
        assert_eq!(doc["credited"], 2.0);
    }

    #[test]
    fn names_are_validated_like_node() {
        assert!(is_valid_entitlement_name("gauntlet-season01_top_100"));
        assert!(!is_valid_entitlement_name("a b"));
        assert!(!is_valid_entitlement_name(""));
        assert!(is_valid_balance_catalog_id("CURRENCY_NOTES"));
        assert!(!is_valid_balance_catalog_id("a.b"));
        assert!(!is_valid_balance_catalog_id("$x"));
    }
}
