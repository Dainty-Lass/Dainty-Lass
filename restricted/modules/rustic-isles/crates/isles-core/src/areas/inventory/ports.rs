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
        let found = request
            .extensions
            .get::<crate::areas::auth::AuthData>()
            .map(AuthData::from)
            .or_else(|| request.extensions.get::<AuthData>().cloned());
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
    pub shared: &'a Shared,
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

const AETHERHEART_CATALOG_ID: &str = "CURRENCY_PJM_PRESTIGE_FILLED";
const AETHERSPARK_CATALOG_ID: &str = "CURRENCY_PJM_PRESTIGE_EMPTY";
const AETHERSPARKS_PER_AETHERHEART: i64 = 200;

fn sparkforge_funded_hearts(removes: &[Value]) -> i64 {
    let mut sparks = 0i64;
    for item in removes {
        if item.get("catalogId").and_then(Value::as_str) == Some(AETHERSPARK_CATALOG_ID)
            && let Some(qty) = service_kit::js::safe_integer(item.get("quantity")).filter(|q| *q > 0)
        {
            sparks = sparks.saturating_add(qty);
        }
    }
    sparks / AETHERSPARKS_PER_AETHERHEART
}

fn spend_banked_prestige_for_reward(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    shared: &Shared,
    account: &AccountId,
    user_id: &str,
    catalog_id: &str,
    quantity: f64,
) -> Result<(), WalletError> {
    if !quantity.is_finite() || quantity <= 0.0 {
        return Ok(());
    }
    let Some(path) = shared.game_data.backend_file("progression_config.json") else {
        return Ok(());
    };
    let Ok(config) = super::progression_config::ProgressionConfig::load(&path) else {
        return Ok(());
    };
    let sources = config.prestige_sources(catalog_id);
    if sources.is_empty() {
        return Ok(());
    }
    let mut remaining = quantity;
    let mut claims: Vec<(String, i64, i64, i64)> = Vec::new();
    let mut total_available = 0.0;
    for source in sources {
        let track = repos
            .progression_tracks
            .get(tx.conn(), account, &source.progression_id)?;
        let claimed_before = track.as_ref().and_then(|t| t.prestige_claimed).unwrap_or(0);
        let progress = track.as_ref().map_or(0, |t| t.progress) as f64;
        let prestige_levels = ((progress - source.cap_xp).max(0.0) / source.xp_per_level).floor() as i64;
        let banked_levels = (prestige_levels - claimed_before).max(0);
        let available = (banked_levels as f64) * source.reward_quantity_per_level;
        total_available += available;
        if remaining <= 0.0 || available <= 0.0 {
            continue;
        }
        let units_to_take = remaining.min(available);
        let levels_to_spend = units_to_take / source.reward_quantity_per_level;
        if !levels_to_spend.is_finite() || levels_to_spend.fract() != 0.0 {
            continue;
        }
        let levels = levels_to_spend as i64;
        let min_progress = (source.cap_xp + ((claimed_before + levels) as f64) * source.xp_per_level) as i64;
        claims.push((source.progression_id.clone(), claimed_before, levels, min_progress));
        remaining -= levels_to_spend * source.reward_quantity_per_level;
    }
    if remaining > 0.0 {
        return Err(WalletError::InsufficientPrestige(format!(
            "Insufficient banked prestige: user {user_id} requested {quantity} of {catalog_id} but only {total_available} banked across eligible tracks"
        )));
    }
    for (progression_id, claimed_before, levels, min_progress) in claims {
        if !repos.progression_tracks.claim_prestige_levels(
            tx,
            account,
            &progression_id,
            claimed_before,
            levels,
            min_progress,
        )? {
            return Err(WalletError::InsufficientPrestige(format!(
                "Insufficient banked prestige: user {user_id} requested {quantity} of {catalog_id} but only {total_available} banked across eligible tracks"
            )));
        }
    }
    Ok(())
}

fn map_wallet_error(error: crate::areas::bounties_store::wallet::WalletError) -> WalletError {
    match error {
        crate::areas::bounties_store::wallet::WalletError::Insufficient { .. } => {
            WalletError::InsufficientBalance(error.to_string())
        }
        crate::areas::bounties_store::wallet::WalletError::UnsafeCatalogId(msg) => {
            WalletError::Repo(RepoError::Data(msg))
        }
        crate::areas::bounties_store::wallet::WalletError::Repo(repo) => WalletError::Repo(repo),
    }
}

/// The production wallet implementation over `bounties_store::wallet`, `challenge_rewards`, and `progression_tracks`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoWallet;

impl Wallet for RepoWallet {
    fn apply_currency_deltas(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        change: &CurrencyChange<'_>,
    ) -> Result<Map<String, Value>, WalletError> {
        use crate::areas::bounties_store::challenge_rewards::{
            fund_challenge_reward, is_daily_challenge_reward_limited,
        };
        use crate::areas::bounties_store::platinum::PLATINUM_AGGREGATE_ID;
        use crate::areas::bounties_store::wallet::{add_currency, spend_platinum};

        let mut touched = Map::new();
        if change.user_id.is_empty() || change.user_id == "INVALID" {
            return Ok(touched);
        }
        let mut sparkforge_hearts = sparkforge_funded_hearts(change.removes);
        let coin = change.shared.version.tables.seasonal_coin.as_ref();

        for item in change.adds {
            let Some(catalog_id) = item
                .get("catalogId")
                .and_then(Value::as_str)
                .filter(|id| id.starts_with("CURRENCY_"))
            else {
                continue;
            };
            let quantity = service_kit::js::number(item.get("quantity"));
            let funded = if change.allow_challenge_reward_funding {
                fund_challenge_reward(
                    repos,
                    tx,
                    coin,
                    change.account,
                    catalog_id,
                    quantity,
                    Some(change.transaction_id),
                )?
            } else {
                None
            };
            if funded.is_some() {
                // Funded by season challenge claim in the same transaction
            } else if change.allow_challenge_reward_funding
                && is_daily_challenge_reward_limited(repos, tx, coin, change.account, catalog_id, quantity)?
            {
                continue;
            } else if change.operator_test_funding {
                // Funded by operator tester tool
            } else if catalog_id == AETHERHEART_CATALOG_ID
                && sparkforge_hearts > 0
                && service_kit::js::safe_integer(item.get("quantity")).is_some_and(|q| q > 0)
            {
                let qty_i64 = quantity as i64;
                let converted = qty_i64.min(sparkforge_hearts);
                sparkforge_hearts -= converted;
                if qty_i64 > converted {
                    spend_banked_prestige_for_reward(
                        tx,
                        repos,
                        change.shared,
                        change.account,
                        change.user_id,
                        catalog_id,
                        (qty_i64 - converted) as f64,
                    )?;
                }
            } else {
                spend_banked_prestige_for_reward(
                    tx,
                    repos,
                    change.shared,
                    change.account,
                    change.user_id,
                    catalog_id,
                    quantity,
                )?;
            }
            let next = add_currency(repos, tx, change.account, catalog_id, quantity).map_err(map_wallet_error)?;
            touched.insert(catalog_id.to_owned(), super::items::number_value(next));
        }

        for item in change.removes {
            let Some(catalog_id) = item
                .get("catalogId")
                .and_then(Value::as_str)
                .filter(|id| id.starts_with("CURRENCY_"))
            else {
                continue;
            };
            let quantity = service_kit::js::number(item.get("quantity"));
            let raw_qty = if quantity.is_nan() { 0.0 } else { quantity };
            if catalog_id == PLATINUM_AGGREGATE_ID {
                let spent = spend_platinum(repos, tx, change.account, raw_qty).map_err(map_wallet_error)?;
                for (id, next) in spent {
                    touched.insert(id, super::items::number_value(next));
                }
                continue;
            }
            let next = add_currency(repos, tx, change.account, catalog_id, -raw_qty).map_err(map_wallet_error)?;
            touched.insert(catalog_id.to_owned(), super::items::number_value(next));
        }

        Ok(touched)
    }

    fn merge_into_stacked(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        account: &AccountId,
        _user_id: &str,
        stacked: Vec<Value>,
    ) -> RepoResult<Vec<Value>> {
        let balances = crate::areas::bounties_store::wallet::get_wallet(repos, tx, account)?;
        Ok(crate::areas::bounties_store::wallet::merge_wallet_into_stacked(
            &balances, &stacked,
        ))
    }

    fn wait_for_fundable_season_claim<'a>(
        &'a self,
        store: &'a IslesStore,
        account: &'a AccountId,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1500);
            loop {
                let owner = account.clone();
                let found = store
                    .read(move |conn, repos| {
                        let record = repos.bounty_states.get(conn, &owner)?;
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        Ok(crate::areas::bounties_store::bounty_state::find_fundable_season_claim(
                            record.as_ref(),
                            now_ms,
                        )
                        .is_some())
                    })
                    .await
                    .unwrap_or(false);
                if found {
                    return true;
                }
                if tokio::time::Instant::now() >= deadline {
                    return false;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
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
    fn committed(&self, _shared: &Shared, _transaction: &CommittedTransaction) {}
    /// `RecordReforgeActivity`
    fn reforge_reset(&self, _shared: &Shared, _account: &AccountId, _at_ms: i64) {}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoObserver;

impl Observer for NoObserver {}

#[derive(Debug, Clone, Copy, Default)]
pub struct ActivityObserver;

impl Observer for ActivityObserver {
    fn committed(&self, shared: &Shared, transaction: &CommittedTransaction) {
        crate::areas::reforged_api::activity::record_inventory_transaction(shared, transaction);
        let Some(store) = shared.store.clone() else {
            return;
        };
        use crate::areas::bounties_store::economy_ledger::{
            EconomyMonitorSettings, EconomyScope, EconomySource, default_hunt_pass_reward_ids, is_platinum_movement,
            new_economy_ledger,
        };
        let source = match transaction.source.as_str() {
            "STORE" => EconomySource::Store,
            "OPERATOR" => EconomySource::Operator,
            "OPERATOR_TEST" => EconomySource::OperatorTest,
            "QUEST" => EconomySource::Quest,
            "HUNT_PASS" => EconomySource::HuntPass,
            "INVENTORY_GAMESERVER" => EconomySource::InventoryGameserver,
            "INVENTORY_HOST" => EconomySource::InventoryHost,
            _ => EconomySource::Other,
        };
        let granted: Vec<(String, f64)> = transaction
            .granted
            .iter()
            .filter_map(|item| {
                let id = item.get("catalogId")?.as_str()?.to_owned();
                let qty = service_kit::js::number(item.get("quantity"));
                (qty.is_finite() && qty > 0.0).then_some((id, qty))
            })
            .collect();
        let settings = EconomyMonitorSettings::read(&shared.settings);
        let reward_ids = default_hunt_pass_reward_ids(&shared.game_data);
        let Some(mut ledger) = new_economy_ledger(
            &transaction.user_id,
            Some(&transaction.transaction_id),
            EconomyScope::new(source),
            granted,
            settings,
            reward_ids,
        ) else {
            return;
        };
        let updated_stacked = transaction
            .result
            .get("updatedStackedItems")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let balance_after_of = |catalog_id: &str| -> f64 {
            updated_stacked
                .iter()
                .find(|item| item.get("catalogId").and_then(Value::as_str) == Some(catalog_id))
                .map_or(0.0, |item| service_kit::js::number(item.get("quantity")))
        };
        let plat_balance_after: f64 = updated_stacked
            .iter()
            .filter(|item| {
                item.get("catalogId")
                    .and_then(Value::as_str)
                    .is_some_and(is_platinum_movement)
            })
            .map(|item| service_kit::js::number(item.get("quantity")).max(0.0))
            .sum();
        for entry in &transaction.currency_credits {
            let Some(catalog_id) = entry.get("catalogId").and_then(Value::as_str) else {
                continue;
            };
            let amount = service_kit::js::number(entry.get("amount"));
            if !amount.is_finite() || amount <= 0.0 {
                continue;
            }
            if is_platinum_movement(catalog_id) {
                let bal = if plat_balance_after > 0.0 {
                    plat_balance_after
                } else {
                    balance_after_of(catalog_id)
                };
                ledger.record_platinum_credit(catalog_id, amount, bal, source == EconomySource::Quest);
            } else {
                ledger.record_reward(catalog_id, amount, source == EconomySource::Quest);
            }
        }
        for entry in &transaction.currency_debits {
            let Some(catalog_id) = entry.get("catalogId").and_then(Value::as_str) else {
                continue;
            };
            let amount = service_kit::js::number(entry.get("amount"));
            if amount.is_finite() && amount > 0.0 && is_platinum_movement(catalog_id) {
                ledger.record_platinum_spend(amount, plat_balance_after);
            }
        }
        if !ledger.is_empty()
            && let Ok(handle) = tokio::runtime::Handle::try_current()
        {
            let account = transaction.account.clone();
            let now_ms = chrono::Utc::now().timestamp_millis();
            handle.spawn(async move {
                ledger.commit_to_store(&store, &account, now_ms).await;
            });
        }
    }

    fn reforge_reset(&self, shared: &Shared, account: &AccountId, _at_ms: i64) {
        crate::areas::reforged_api::activity::record_reforge(shared, &account.to_string());
    }
}

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
            wallet: Arc::new(RepoWallet),
            starter: Arc::new(CharactersStarter),
            characters: Arc::new(CharactersArea),
            observer: Arc::new(ActivityObserver),
            repos: None,
        }
    }
}
