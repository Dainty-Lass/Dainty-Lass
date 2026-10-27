//! What this area takes from the others. Each is a trait the owning RU5 area (or the integration pass) implements;
//! until then its default answers `Unimplemented`, which the routes write as the 501 envelope, so a missing piece is
//! loud rather than a silent difference from Node.
//!
//! | port | Node | owner |
//! | --- | --- | --- |
//! | [`Authenticator`] | `middleware/HasParadoxBackendAuth.ts` | auth |
//! | [`Wallet`] | `controllers/wallet.ts` (`ApplyCurrencyDeltas`, `MergeWalletIntoStacked`), `challengeRewards.ts` | bounties-store |
//! | [`StarterBootstrap`] | `controllers/starterManifest.ts` (`EnsureStarterBootstrapRecordsInTransaction`) | characters-loadouts |
//! | [`Characters`] | `controllers/character.ts` (`GetCharactersForUid`) | characters-loadouts |
//! | [`Observer`] | `diagnostics/*`, `activity/activityMonitor.ts` (monitoring only, never on the wire) | reforged-api, bounties-store |

use std::sync::Arc;

use axum::http::{HeaderMap, Method};
use axum::response::Response;
use module_api::BoxFuture;
use rifts_seam::RuntimeAuth;
use serde_json::{Map, Value};

use crate::db::IslesStore;
use crate::http::Shared;
use crate::repo::{AccountId, AnyConnection, RepoError, RepoResult, Repositories, Tx};

use crate::repo::inventory::InventoryRecord;

/// The part of Node's `req.AuthData` this area reads.
#[derive(Debug, Clone, Default)]
pub struct AuthData {
    /// `IsGameserver`: the dedicated server's API key
    pub is_gameserver: bool,
    /// The JWT's `userId`, or (a bearer-less gameserver call, under the `ConfiguredAccount` policy) the configured one
    pub user_id: Option<String>,
    /// `IsPlayerHostRuntime`: a realm host's runtime, authenticated by rustic-rifts
    pub host_runtime: Option<RuntimeAuth>,
}

/// The request as the middleware sees it: the body is read after.
pub struct AuthRequest<'a> {
    pub method: &'a Method,
    pub path: &'a str,
    pub headers: &'a HeaderMap,
    pub request_id: &'a str,
}

pub enum Authentication {
    Authenticated(AuthData),
    /// The middleware answered (401, 403): send it
    Answer(Response),
}

/// `HasParadoxBackendAuth`. (Its `INVALID` path rewrite is invisible to this area's handlers: Express bound the route
/// parameters before the middleware ran, so they still read `INVALID`.)
pub trait Authenticator: Send + Sync {
    fn authenticate<'a>(&'a self, shared: &'a Shared, request: AuthRequest<'a>) -> BoxFuture<'a, Authentication>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnwiredAuth;

impl Authenticator for UnwiredAuth {
    fn authenticate<'a>(&'a self, _: &'a Shared, request: AuthRequest<'a>) -> BoxFuture<'a, Authentication> {
        let id = request.request_id.to_owned();
        Box::pin(async move {
            Authentication::Answer(super::respond::repo_error(
                &RepoError::unimplemented("auth", "has_paradox_backend_auth"),
                &id,
            ))
        })
    }
}

/// One `ApplyCurrencyDeltas` call.
pub struct CurrencyChange<'a> {
    pub account: &'a AccountId,
    pub user_id: &'a str,
    pub adds: &'a [Value],
    pub removes: &'a [Value],
    pub transaction_id: &'a str,
    /// A season challenge's claim may fund the coin grant (`allowChallengeRewardFunding`)
    pub allow_challenge_reward_funding: bool,
    /// The admin tester tool's grant (`operatorTestFunding`)
    pub operator_test_funding: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// `InsufficientBalanceError`
    #[error("{0}")]
    InsufficientBalance(String),
    /// `InsufficientPrestigeProgressError`
    #[error("{0}")]
    InsufficientPrestige(String),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// The wallet half of a transaction: currency lives in the wallet, not the stacked blob.
pub trait Wallet: Send + Sync {
    /// `ApplyCurrencyDeltas`, inside the transaction. Returns the post-transaction balance of every currency it
    /// touched, in the order it touched them.
    fn apply_currency_deltas(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        change: &CurrencyChange<'_>,
    ) -> Result<Map<String, Value>, WalletError>;

    /// `MergeWalletIntoStacked`: the wallet's balances over the stacked items, creating the starter wallet when the
    /// account has none yet.
    fn merge_into_stacked(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        account: &AccountId,
        user_id: &str,
        stacked: Vec<Value>,
    ) -> RepoResult<Vec<Value>>;

    /// `WaitForFundableSeasonClaim`: up to 1.5 s for the bounty save that claims a challenge to land.
    fn wait_for_fundable_season_claim<'a>(
        &'a self,
        store: &'a IslesStore,
        account: &'a AccountId,
    ) -> BoxFuture<'a, bool>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnwiredWallet;

impl Wallet for UnwiredWallet {
    fn apply_currency_deltas(
        &self,
        _: &mut Tx<'_>,
        _: &Repositories,
        _: &CurrencyChange<'_>,
    ) -> Result<Map<String, Value>, WalletError> {
        Err(RepoError::unimplemented("wallets", "apply_currency_deltas").into())
    }

    fn merge_into_stacked(
        &self,
        _: &mut Tx<'_>,
        _: &Repositories,
        _: &AccountId,
        _: &str,
        _: Vec<Value>,
    ) -> RepoResult<Vec<Value>> {
        Err(RepoError::unimplemented("wallets", "merge_into_stacked"))
    }

    fn wait_for_fundable_season_claim<'a>(&'a self, _: &'a IslesStore, _: &'a AccountId) -> BoxFuture<'a, bool> {
        Box::pin(async { true })
    }
}

/// The character's account does not own it (`CharacterOwnershipError`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Character {character_id} does not belong to user {user_id}")]
pub struct CharacterOwnership {
    pub character_id: String,
    pub user_id: String,
}

/// `EnsureStarterBootstrapRecordsInTransaction`: creates an absent inventory and loadout from the version's starter
/// manifest, and appends the starter parts and stacks a newer manifest added. Returns the inventory.
pub trait StarterBootstrap: Send + Sync {
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

#[derive(Debug, Clone, Copy, Default)]
pub struct UnwiredStarter;

impl StarterBootstrap for UnwiredStarter {
    fn ensure(
        &self,
        _: &mut Tx<'_>,
        _: &Repositories,
        _: &Shared,
        _: &AccountId,
        _: &str,
        _: &str,
    ) -> RepoResult<Result<InventoryRecord, CharacterOwnership>> {
        Err(RepoError::unimplemented(
            "starterManifest",
            "ensure_starter_bootstrap_records",
        ))
    }
}

/// The account's first character, as `GetCharactersForUid(userId)[0]`.
pub trait Characters: Send + Sync {
    fn first_character_id<'a>(
        &'a self,
        store: &'a IslesStore,
        repos: &'a Repositories,
        user_id: &'a str,
    ) -> BoxFuture<'a, RepoResult<Option<String>>>;
}

/// Reads the account's characters. `GetCharactersForUid` also creates one when the account has none; that write is
/// the characters area's, so an account without a character answers `None` here.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadOnlyCharacters;

impl Characters for ReadOnlyCharacters {
    fn first_character_id<'a>(
        &'a self,
        store: &'a IslesStore,
        repos: &'a Repositories,
        user_id: &'a str,
    ) -> BoxFuture<'a, RepoResult<Option<String>>> {
        let repos = repos.clone();
        let user = crate::UserId(user_id.to_owned());
        Box::pin(async move {
            store
                .database()
                .read(move |conn: &mut AnyConnection| -> RepoResult<Option<String>> {
                    let Some(account) = repos.accounts.find_by_user_id(conn, &user)? else {
                        return Ok(None);
                    };
                    let characters = repos.characters.find_many_by_account(conn, &account.id)?;
                    Ok(characters.into_iter().next().map(|character| character.character_id))
                })
                .await
        })
    }
}

/// A committed `POST /inventory`, for the monitors Node feeds after the commit (the economy ledger's lines and daily
/// rollup, the activity monitor). None of it is on the wire.
#[derive(Debug, Clone)]
pub struct CommittedTransaction {
    pub account: AccountId,
    pub user_id: String,
    pub transaction_id: String,
    pub source: String,
    pub granted: Vec<Value>,
    pub currency_credits: Vec<Value>,
    pub currency_debits: Vec<Value>,
    pub result: Value,
}

pub trait Observer: Send + Sync {
    fn committed(&self, _transaction: &CommittedTransaction) {}
    /// `RecordReforgeActivity`
    fn reforge_reset(&self, _account: &AccountId, _at_ms: i64) {}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoObserver;

impl Observer for NoObserver {}

/// Everything the handlers take from outside the area.
#[derive(Clone)]
pub struct Ports {
    pub auth: Arc<dyn Authenticator>,
    pub wallet: Arc<dyn Wallet>,
    pub starter: Arc<dyn StarterBootstrap>,
    pub characters: Arc<dyn Characters>,
    pub observer: Arc<dyn Observer>,
    /// Repositories to use in place of the store's own (tests; the integration pass leaves it `None`)
    pub repos: Option<Repositories>,
}

impl Default for Ports {
    fn default() -> Self {
        Self {
            auth: Arc::new(UnwiredAuth),
            wallet: Arc::new(UnwiredWallet),
            starter: Arc::new(UnwiredStarter),
            characters: Arc::new(ReadOnlyCharacters),
            observer: Arc::new(NoObserver),
            repos: None,
        }
    }
}
