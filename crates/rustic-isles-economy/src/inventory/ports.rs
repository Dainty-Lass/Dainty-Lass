// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What the inventory part takes from outside (spec §11.3), each with its production default.

use std::sync::Arc;

use axum::response::Response;
use http::{Extensions, HeaderMap};
use module_api::{ActorContext, BoxFuture, GlobalId};
use rifts_seam::RuntimeAuth;
use rustic_isles::Shared;
use rustic_isles_auth::AuthData;
use rustic_isles_characters::EnsureError;
use rustic_isles_store::repo::inventory::InventoryRecord;
use rustic_isles_store::{AccountId, IslesStore, RepoError, RepoResult, Repositories, Tx, UserId};
use serde_json::Value;

use crate::bounties::wallet::{self as bounty_wallet, WalletError as BountyWalletError};
use crate::inventory::currency;
use crate::inventory::prestige::ProgressionConfig;
use crate::inventory::transaction::CommittedTransaction;

// ------------------------------------------------------------------ authenticator

/// A player-host runtime, with rifts' capability when the realms seam handed one over.
#[derive(Debug, Clone)]
pub struct HostRuntime {
    /// Rifts' runtime capability, when the seam's handle is one.
    pub capability: Option<RuntimeAuth>,
}

/// The caller as the inventory routes see it.
#[derive(Debug, Clone, Default)]
pub struct Caller {
    /// The player's userId, when the caller acts for one.
    pub user_id: Option<String>,
    /// True for the dedicated game server.
    pub is_gameserver: bool,
    /// Never together with `is_gameserver`
    pub runtime: Option<HostRuntime>,
}

impl Caller {
    /// The caller the auth area put in the request.
    pub fn from_auth(data: &AuthData) -> Self {
        let runtime = if data.is_gameserver {
            None
        } else {
            data.runtime.as_ref().map(|r| HostRuntime {
                capability: crate::caller::capability(r),
            })
        };
        Self {
            user_id: data.user_id.clone(),
            is_gameserver: data.is_gameserver,
            runtime,
        }
    }
}

/// The parts of a request the authenticator reads.
pub struct AuthRequest<'a> {
    /// The HTTP method.
    pub method: &'a http::Method,
    /// The request path.
    pub path: &'a str,
    /// The request headers.
    pub headers: &'a HeaderMap,
    /// The request extensions, where the auth area leaves its `AuthData`.
    pub extensions: &'a Extensions,
    /// The request id, for the log.
    pub request_id: &'a str,
}

/// What authenticating an inventory request gave.
pub enum Authentication {
    /// A caller to serve.
    Authenticated(Caller),
    /// The answer to send instead, such as an empty 401.
    Answer(Response),
}

/// Who an inventory request is from.
pub trait Authenticator: Send + Sync {
    /// The caller, or the answer refusing the request.
    fn authenticate<'a>(&'a self, shared: &'a Shared, request: AuthRequest<'a>) -> BoxFuture<'a, Authentication>;
}

/// The `AuthData` in the extensions, else empty 401.
#[derive(Debug, Default, Clone, Copy)]
pub struct ExtensionAuthenticator;

impl Authenticator for ExtensionAuthenticator {
    fn authenticate<'a>(&'a self, _: &'a Shared, request: AuthRequest<'a>) -> BoxFuture<'a, Authentication> {
        let found = request.extensions.get::<AuthData>().map(Caller::from_auth);
        Box::pin(async move {
            match found {
                Some(caller) => Authentication::Authenticated(caller),
                None => Authentication::Answer(crate::util::empty(http::StatusCode::UNAUTHORIZED)),
            }
        })
    }
}

// ------------------------------------------------------------------ wallet

/// The currency half of a transaction's inputs (§9.1).
pub struct CurrencyChange<'a> {
    /// The module's shared state.
    pub shared: &'a Shared,
    /// The wallet's account.
    pub account: &'a AccountId,
    /// The owner's userId, for the log.
    pub user_id: &'a str,
    /// Normalized
    pub add_stacked: &'a [Value],
    /// As sent
    pub remove_stacked: &'a Value,
    /// The transaction id, for funding and the log.
    pub transaction_id: &'a str,
    /// Whether a seasonal coin add may fund a claimed challenge reward (W5).
    pub allow_challenge_reward_funding: bool,
    /// An operator's test grant, which skips the prestige check.
    pub operator_test_funding: bool,
    /// The version's progression config, for the prestige rules.
    pub prestige: &'a ProgressionConfig,
    /// The starter wallet a new account begins with.
    pub starter: &'a [(String, f64)],
}

/// Why the currency step refused a transaction.
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// A removal asked for more than the balance holds.
    #[error("{0}")]
    InsufficientBalance(String),
    /// An add needs more banked prestige than the account has.
    #[error("{0}")]
    InsufficientPrestige(String),
    /// The store failed.
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<BountyWalletError> for WalletError {
    fn from(error: BountyWalletError) -> Self {
        match error {
            BountyWalletError::Insufficient { .. } => Self::InsufficientBalance(error.to_string()),
            BountyWalletError::UnsafeCatalogId(_) => Self::Repo(RepoError::Data(error.to_string())),
            BountyWalletError::Repo(e) => Self::Repo(e),
        }
    }
}

/// The currency half of an inventory transaction.
pub trait Wallet: Send + Sync {
    /// The touched balances, in touch order.
    fn apply_currency_deltas(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        change: &CurrencyChange<'_>,
    ) -> Result<Vec<(String, f64)>, WalletError>;
    /// The stored stacked list with the wallet's balances merged in, for an answer.
    fn merge_into_stacked(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        shared: &Shared,
        account: &AccountId,
        user_id: &str,
        stacked: Vec<Value>,
    ) -> RepoResult<Vec<Value>>;
    /// Waits briefly for a claimed challenge the coin grant can fund (W7); false when none appears.
    fn wait_for_fundable_season_claim<'a>(
        &'a self,
        store: &'a IslesStore,
        account: &'a AccountId,
    ) -> BoxFuture<'a, bool>;
}

/// §9 over the bounties part's wallet.
#[derive(Debug, Default, Clone, Copy)]
pub struct BountiesWallet;

impl Wallet for BountiesWallet {
    fn apply_currency_deltas(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        change: &CurrencyChange<'_>,
    ) -> Result<Vec<(String, f64)>, WalletError> {
        currency::apply_currency_deltas(tx, repos, change)
    }

    fn merge_into_stacked(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        shared: &Shared,
        account: &AccountId,
        _user_id: &str,
        stacked: Vec<Value>,
    ) -> RepoResult<Vec<Value>> {
        let balances = bounty_wallet::get_wallet(tx, repos, account, &bounty_wallet::starter_balances(shared))?;
        Ok(bounty_wallet::merge_wallet_into_stacked(&balances, stacked))
    }

    fn wait_for_fundable_season_claim<'a>(
        &'a self,
        store: &'a IslesStore,
        account: &'a AccountId,
    ) -> BoxFuture<'a, bool> {
        Box::pin(crate::bounties::challenge_rewards::wait_for_fundable_season_claim(
            store, account,
        ))
    }
}

// ------------------------------------------------------------------ starter bootstrap and characters

/// `Character <characterId> does not belong to user <userId>`
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Character {character_id} does not belong to user {user_id}")]
pub struct CharacterOwnership {
    /// The character asked for.
    pub character_id: String,
    /// The user who asked.
    pub user_id: String,
}

/// The starter records an inventory transaction needs before it runs.
pub trait StarterBootstrap: Send + Sync {
    /// The character's inventory, created with the starter kit when missing; an error value when the character is not the user's.
    fn ensure(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        shared: &Shared,
        account: &AccountId,
        user_id: &str,
        character_id: &str,
    ) -> RepoResult<Result<InventoryRecord, CharacterOwnership>>;
}

/// The characters area's starter check, in the caller's transaction.
#[derive(Debug, Default, Clone, Copy)]
pub struct StarterRecordsCheck;

impl StarterBootstrap for StarterRecordsCheck {
    fn ensure(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        shared: &Shared,
        account: &AccountId,
        user_id: &str,
        character_id: &str,
    ) -> RepoResult<Result<InventoryRecord, CharacterOwnership>> {
        let not_owned = || CharacterOwnership {
            character_id: character_id.to_owned(),
            user_id: user_id.to_owned(),
        };
        let Some(area) = shared.slot::<rustic_isles_characters::Characters>() else {
            // Without the characters area nothing is created: the inventory must exist
            if repos.characters.find(tx.conn(), character_id, account)?.is_none() {
                return Ok(Err(not_owned()));
            }
            return repos
                .inventories
                .find_by_character_id(tx.conn(), character_id)?
                .map(Ok)
                .ok_or_else(|| RepoError::Data(format!("no inventory for characterId {character_id}")));
        };
        match area.ensure_starter_in(tx, repos, shared.version().tables, account, user_id, character_id) {
            Ok((inventory, _)) => Ok(Ok(inventory)),
            Err(EnsureError::NotOwned { .. }) => Ok(Err(not_owned())),
            Err(EnsureError::Repo(e)) => Err(e),
        }
    }
}

/// The characters area, as the inventory routes call it.
pub trait Characters: Send + Sync {
    /// The user's first character id; it may create the account and that character.
    fn first_character_id<'a>(
        &'a self,
        shared: &'a Shared,
        store: &'a IslesStore,
        actor: &'a ActorContext,
        user_id: &'a str,
    ) -> BoxFuture<'a, RepoResult<Option<String>>>;
}

/// The characters area's own lookup.
#[derive(Debug, Default, Clone, Copy)]
pub struct CharactersList;

impl Characters for CharactersList {
    fn first_character_id<'a>(
        &'a self,
        shared: &'a Shared,
        store: &'a IslesStore,
        actor: &'a ActorContext,
        user_id: &'a str,
    ) -> BoxFuture<'a, RepoResult<Option<String>>> {
        Box::pin(async move {
            let Some(area) = shared.slot::<rustic_isles_characters::Characters>() else {
                let user = UserId::new(user_id);
                return store
                    .read(move |conn, repos| {
                        let Some(account) = repos.accounts.find_by_user_id(conn, &user)? else {
                            return Ok(None);
                        };
                        let list = repos.characters.find_many_by_account(conn, &account.id)?;
                        Ok(list.into_iter().next().map(|c| c.character_id))
                    })
                    .await;
            };
            match area
                .characters_for_user(store, shared.version().tables, actor, user_id)
                .await
            {
                Ok(list) => Ok(list
                    .first()
                    .and_then(|c| c.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)),
                Err(rustic_isles_characters::CharactersError::Repo(e)) => Err(e),
                Err(other) => Err(RepoError::Data(other.to_string())),
            }
        })
    }
}

// ------------------------------------------------------------------ observer, activity and the ring

/// Monitoring only: never on the wire, never fails a request.
pub trait Observer: Send + Sync {
    /// A transaction was committed.
    fn committed(&self, shared: &Arc<Shared>, transaction: &CommittedTransaction);
    /// An account's reforge reset its progression at `at_ms`.
    fn reforge_reset(&self, shared: &Arc<Shared>, account: &AccountId, at_ms: i64);
}

/// The Reforged API's activity hooks; the composer puts an [`InventoryActivitySlot`] in the slots.
pub trait InventoryActivity: Send + Sync {
    /// Counts a committed inventory transaction.
    fn record_inventory_transaction(&self, _transaction: &CommittedTransaction) {}
    /// Counts a reforge.
    fn record_reforge(&self, _account: &AccountId, _at_ms: i64) {}
}

/// The slot the composer fills with the Reforged API's activity hooks.
pub struct InventoryActivitySlot(pub Arc<dyn InventoryActivity>);

/// The economy ledger (W9) and the activity slot.
#[derive(Debug, Default, Clone, Copy)]
pub struct LedgerObserver;

impl Observer for LedgerObserver {
    fn committed(&self, shared: &Arc<Shared>, transaction: &CommittedTransaction) {
        crate::bounties::economy_ledger::commit_transaction(shared, transaction);
        if let Some(slot) = shared.slot::<InventoryActivitySlot>() {
            slot.0.record_inventory_transaction(transaction);
        }
    }

    fn reforge_reset(&self, shared: &Arc<Shared>, account: &AccountId, at_ms: i64) {
        if let Some(slot) = shared.slot::<InventoryActivitySlot>() {
            slot.0.record_reforge(account, at_ms);
        }
    }
}

/// Matchmaking's knowledge of where a player is hunting (a slot trait it fills).
pub trait RingInstances: Send + Sync {
    /// The hunt instance the player was placed in, else the last ring instance.
    fn ring_instance_for_player(&self, player: &GlobalId) -> Option<GlobalId>;
}

/// The slot matchmaking fills with its ring instances.
pub struct RingInstancesSlot(pub Arc<dyn RingInstances>);

/// No ring instance (a new hunt id is minted).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRing;

impl RingInstances for NoRing {
    fn ring_instance_for_player(&self, _: &GlobalId) -> Option<GlobalId> {
        None
    }
}

/// The ring port, or the module's slot when the port is the default.
pub(crate) fn ring_instance(shared: &Shared, ports: &InventoryPorts, player: &GlobalId) -> Option<GlobalId> {
    ports
        .ring
        .ring_instance_for_player(player)
        .or_else(|| shared.slot::<RingInstancesSlot>()?.0.ring_instance_for_player(player))
}

/// The ports; `Default` is production's.
#[derive(Clone)]
pub struct InventoryPorts {
    /// Who a request is from.
    pub auth: Arc<dyn Authenticator>,
    /// The currency half of a transaction.
    pub wallet: Arc<dyn Wallet>,
    /// The starter records check.
    pub starter: Arc<dyn StarterBootstrap>,
    /// The characters lookup.
    pub characters: Arc<dyn Characters>,
    /// The monitoring hooks.
    pub observer: Arc<dyn Observer>,
    /// Where a player is hunting.
    pub ring: Arc<dyn RingInstances>,
    /// Tests; none is the store's own
    pub repos: Option<Repositories>,
}

impl Default for InventoryPorts {
    fn default() -> Self {
        Self {
            auth: Arc::new(ExtensionAuthenticator),
            wallet: Arc::new(BountiesWallet),
            starter: Arc::new(StarterRecordsCheck),
            characters: Arc::new(CharactersList),
            observer: Arc::new(LedgerObserver),
            ring: Arc::new(NoRing),
            repos: None,
        }
    }
}
