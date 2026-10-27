//! What this area takes from the others. Each is a trait the owning RU5 area (or the integration pass) implements;
//! until then its default answers `Unimplemented`, which the routes write as the 501 envelope, so a missing piece is
//! loud rather than a silent difference from Node.
//!
//! | port | Node | owner |
//! | --- | --- | --- |
//! | [`Authenticator`] | `middleware/HasParadoxBackendAuth.ts` | auth (until it lands, the `AuthData` it leaves in the request's extensions, as the characters area reads it) |
//! | [`Wallet`] | `controllers/wallet.ts` (`ApplyCurrencyDeltas`, `MergeWalletIntoStacked`), `challengeRewards.ts` | bounties-store |
//! | [`StarterBootstrap`] | `controllers/starterManifest.ts` (`EnsureStarterBootstrapRecordsInTransaction`) | characters-loadouts (wired) |
//! | [`Characters`] | `controllers/character.ts` (`GetCharactersForUid`) | characters-loadouts (wired) |
//! | [`Observer`] | `diagnostics/*`, `activity/activityMonitor.ts` (monitoring only, never on the wire) | reforged-api, bounties-store |

use std::sync::Arc;

use axum::http::{Extensions, HeaderMap, Method};
use axum::response::Response;
use module_api::BoxFuture;
use serde_json::{Map, Value};

use module_api::ActorContext;

use crate::areas::characters::controller::{CtlError, characters_for_user};
use crate::areas::characters::starter::{EnsureError, ensure_in_transaction};
use crate::db::IslesStore;
use crate::http::Shared;
use crate::repo::{AccountId, RepoError, RepoResult, Repositories, Tx};

use crate::repo::inventory::InventoryRecord;

/// The part of Node's `req.AuthData` this area reads: the characters area's, which the auth middleware fills.
pub use crate::areas::characters::AuthData;

/// The request as the middleware sees it: the body is read after.
pub struct AuthRequest<'a> {
    pub method: &'a Method,
    pub path: &'a str,
    pub headers: &'a HeaderMap,
    /// Where the auth middleware leaves the request's [`AuthData`]
    pub extensions: &'a Extensions,
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

/// The auth middleware's `AuthData` in the request's extensions; a request without it is refused 401, as
/// `HasParadoxBackendAuth` refuses one it couldn't authenticate.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtensionAuth;

impl Authenticator for ExtensionAuth {
    fn authenticate<'a>(&'a self, _: &'a Shared, request: AuthRequest<'a>) -> BoxFuture<'a, Authentication> {
        let found = request.extensions.get::<AuthData>().cloned();
        Box::pin(async move {
            match found {
                Some(auth) => Authentication::Authenticated(auth),
                None => Authentication::Answer(super::respond::empty(401)),
            }
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

/// The characters area's starter bootstrap (`characters::starter::ensure_in_transaction`).
#[derive(Debug, Clone, Copy, Default)]
pub struct CharactersStarter;

impl StarterBootstrap for CharactersStarter {
    fn ensure(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        shared: &Shared,
        account: &AccountId,
        user_id: &str,
        character_id: &str,
    ) -> RepoResult<Result<InventoryRecord, CharacterOwnership>> {
        match ensure_in_transaction(tx, repos, &shared.version.tables, account, user_id, character_id) {
            Ok((inventory, _loadout)) => Ok(Ok(inventory)),
            Err(EnsureError::NotOwned { character_id, user }) => Ok(Err(CharacterOwnership {
                character_id,
                user_id: user,
            })),
            Err(EnsureError::Repo(error)) => Err(error),
        }
    }
}

/// The account's first character, as `GetCharactersForUid(userId)[0]`.
pub trait Characters: Send + Sync {
    fn first_character_id<'a>(
        &'a self,
        shared: &'a Shared,
        store: &'a IslesStore,
        actor: &'a ActorContext,
        user_id: &'a str,
    ) -> BoxFuture<'a, RepoResult<Option<String>>>;
}

/// The characters area's `characters_for_user`, which also makes the first character when the account has none.
#[derive(Debug, Clone, Copy, Default)]
pub struct CharactersArea;

impl Characters for CharactersArea {
    fn first_character_id<'a>(
        &'a self,
        shared: &'a Shared,
        store: &'a IslesStore,
        actor: &'a ActorContext,
        user_id: &'a str,
    ) -> BoxFuture<'a, RepoResult<Option<String>>> {
        Box::pin(async move {
            match characters_for_user(store, &shared.version.tables, actor, user_id).await {
                Ok(list) => Ok(list
                    .first()
                    .and_then(|character| character.get("id"))
                    .and_then(|id| id.as_str().map(str::to_owned))),
                Err(CtlError::Repo(error)) => Err(error),
                Err(other) => Err(RepoError::Data(other.to_string())),
            }
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
            auth: Arc::new(ExtensionAuth),
            wallet: Arc::new(UnwiredWallet),
            starter: Arc::new(CharactersStarter),
            characters: Arc::new(CharactersArea),
            observer: Arc::new(NoObserver),
            repos: None,
        }
    }
}
