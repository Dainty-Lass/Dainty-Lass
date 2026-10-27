//! RU5 bounties, store, Lady Luck and platinum: bounty and cooldown state, entitlements, wallets, economy rollups.

use super::{AccountId, AnyConnection, Json, Tx, repository};

/// `BountyStateRecord`. The bounties and drafts stay the client's JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct BountyStateRecord {
    pub account: AccountId,
    /// `StoredBounty[]`: `{group, bounty, claimedAt?, rewardCredit?}`
    pub bounties: Vec<Json>,
    pub draft_data: Json,
    pub daily_reward_credited_on: Option<String>,
    pub daily_premium_draft_granted_on: Option<String>,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

repository! {
    pub trait BountyStateRepository("bountyStates") {
        fn get(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<BountyStateRecord>;
        /// `None` expected version inserts only; a conflict answers `None`
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

/// `EntitlementRecord`.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitlementRecord {
    pub account: AccountId,
    pub entitlements: Vec<EntitlementEntry>,
}

repository! {
    pub trait EntitlementRepository("entitlements") {
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<EntitlementRecord>;
        fn create_if_missing(&self, tx: &mut Tx<'_>, account: &AccountId) -> EntitlementRecord;
        /// Whether it was granted now
        fn grant_if_missing(&self, tx: &mut Tx<'_>, account: &AccountId, entry: &EntitlementEntry) -> bool;
    }
}

/// `WalletRecord`: balances by catalog id.
#[derive(Debug, Clone, PartialEq)]
pub struct WalletRecord {
    pub account: AccountId,
    pub balances: Vec<(String, f64)>,
    pub bootstrap_version: Option<String>,
}

repository! {
    pub trait WalletRepository("wallets") {
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<WalletRecord>;
        fn create(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> ();
        fn create_if_missing(&self, tx: &mut Tx<'_>, wallet: &WalletRecord) -> WalletRecord;
        /// `None` when the balance would go below zero
        fn increment_balance(&self, tx: &mut Tx<'_>, account: &AccountId, catalog_id: &str, delta: f64)
            -> Option<WalletRecord>;
    }
}

repository! {
    pub trait EconomyRollupRepository("economyRollups") {
        /// The rollup after the increments; read by the monitors only, never by a grant
        fn add_to_rollup(&self, tx: &mut Tx<'_>, account: &AccountId, day: &str, currency: &str,
            increments: &[(String, f64)], ttl_at: i64) -> Json;
    }
}
