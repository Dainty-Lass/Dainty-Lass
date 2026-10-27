//! One trait per Node repository (`persistence/contracts/*.ts`), on `store`'s two engines (DD15) and its change log
//! (DD23, DH1).
//!
//! A request's database work is one closure on `store`'s blocking pool, so the methods are synchronous: a read takes
//! the closure's connection, a write the [`Tx`] of [`crate::db::IslesStore::write`], whose triggers log every row
//! it changes under the actor. One closure is one transaction, which is what Node's `GetUnitOfWork` and the
//! `session?: ClientSession` parameters were for. Per-account data is keyed by [`AccountId`], the global id (DD18);
//! a handler resolves the client's `userId` first ([`auth::AccountRepository::find_by_user_id`]).
//!
//! Each file is one RU5 area's, so an area's agent changes only its own: a signature written here from the Mongo
//! contract is a starting point, not a contract. Every method of [`Stub`] answers [`RepoError::Unimplemented`]
//! (RD18) until its area ports it.

pub mod auth;
pub mod bounties;
pub mod characters;
pub mod chat;
pub mod inventory;
pub mod parties;
pub mod progression;
pub mod reforged;

use std::sync::Arc;

pub use ::store::{AnyConnection, Tx};

pub use crate::ids::{AccountId, UserId};

/// JSON that Node stores as a free-form document (`Record<string, any>`, `unknown`).
pub type Json = serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("{repository}: {operation} is not implemented yet")]
    Unimplemented {
        repository: &'static str,
        operation: &'static str,
    },
    /// An account write reached a node that isn't the account's home (DD17); DN5 forwards it there
    #[error("{account} is homed on {home}, not here")]
    NotHome { account: AccountId, home: String },
    #[error("the account's home could not be resolved: {0}")]
    Home(String),
    #[error(transparent)]
    Store(#[from] ::store::Error),
    #[error("database: {0}")]
    Query(#[from] diesel::result::Error),
    #[error("stored data: {0}")]
    Data(String),
}

impl RepoError {
    pub const fn unimplemented(repository: &'static str, operation: &'static str) -> Self {
        Self::Unimplemented { repository, operation }
    }
}

pub type RepoResult<T> = Result<T, RepoError>;

/// The stand-in every area starts from.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

/// Declares a repository trait and its [`Stub`]: every method returns `RepoResult<..>`.
macro_rules! repository {
    (
        $(#[$doc:meta])*
        pub trait $name:ident($label:literal) {
            $(
                $(#[$mdoc:meta])*
                fn $method:ident(&self $(, $arg:ident: $ty:ty)* $(,)?) -> $ret:ty;
            )*
        }
    ) => {
        $(#[$doc])*
        pub trait $name: Send + Sync {
            $(
                $(#[$mdoc])*
                fn $method(&self $(, $arg: $ty)*) -> $crate::repo::RepoResult<$ret>;
            )*
        }

        impl $name for $crate::repo::Stub {
            $(
                fn $method(&self $(, _: $ty)*) -> $crate::repo::RepoResult<$ret> {
                    Err($crate::repo::RepoError::unimplemented($label, stringify!($method)))
                }
            )*
        }
    };
}
pub(crate) use repository;

/// Node's `RepositoryProvider`: every repository, as the areas reach them.
#[derive(Clone)]
pub struct Repositories {
    pub accounts: Arc<dyn auth::AccountRepository>,
    pub launcher_accounts: Arc<dyn auth::LauncherAccountRepository>,
    pub auth_identities: Arc<dyn auth::AuthIdentityRepository>,
    pub refresh_sessions: Arc<dyn auth::RefreshSessionRepository>,
    pub game_exchange_codes: Arc<dyn auth::GameExchangeCodeRepository>,
    pub discord_oauth_transactions: Arc<dyn auth::DiscordOAuthTransactionRepository>,
    pub email_codes: Arc<dyn auth::EmailCodeRepository>,
    pub api_keys: Arc<dyn auth::ApiKeyRepository>,
    pub admin: Arc<dyn reforged::AdminRepository>,
    pub invites: Arc<dyn reforged::InviteRepository>,
    pub launcher_notices: Arc<dyn reforged::LauncherNoticesRepository>,
    pub activity: Arc<dyn reforged::ActivityRepository>,
    pub launcher_guard: Arc<dyn reforged::LauncherGuardRepository>,
    pub characters: Arc<dyn characters::CharacterRepository>,
    pub loadouts: Arc<dyn characters::LoadoutRepository>,
    pub inventories: Arc<dyn inventory::InventoryRepository>,
    pub inventory_transactions: Arc<dyn inventory::InventoryTransactionRepository>,
    pub progression_tracks: Arc<dyn progression::ProgressionTrackRepository>,
    pub progression_grants: Arc<dyn progression::ProgressionGrantRepository>,
    pub escalation_progress: Arc<dyn progression::EscalationProgressRepository>,
    pub player_journey: Arc<dyn progression::PlayerJourneyRepository>,
    pub breadcrumbs: Arc<dyn progression::BreadcrumbRepository>,
    pub encountered_content: Arc<dyn progression::EncounteredContentRepository>,
    pub bounty_states: Arc<dyn bounties::BountyStateRepository>,
    pub cooldown_states: Arc<dyn bounties::CooldownStateRepository>,
    pub entitlements: Arc<dyn bounties::EntitlementRepository>,
    pub wallets: Arc<dyn bounties::WalletRepository>,
    pub economy_rollups: Arc<dyn bounties::EconomyRollupRepository>,
    pub parties: Arc<dyn parties::PartyRepository>,
    pub friendships: Arc<dyn chat::FriendshipRepository>,
}

impl Repositories {
    /// Every repository a stub.
    pub fn stubs() -> Self {
        let stub = Arc::new(Stub);
        Self {
            accounts: stub.clone(),
            launcher_accounts: stub.clone(),
            auth_identities: stub.clone(),
            refresh_sessions: stub.clone(),
            game_exchange_codes: stub.clone(),
            discord_oauth_transactions: stub.clone(),
            email_codes: stub.clone(),
            api_keys: stub.clone(),
            admin: stub.clone(),
            invites: stub.clone(),
            launcher_notices: stub.clone(),
            activity: stub.clone(),
            launcher_guard: stub.clone(),
            characters: stub.clone(),
            loadouts: stub.clone(),
            inventories: stub.clone(),
            inventory_transactions: stub.clone(),
            progression_tracks: stub.clone(),
            progression_grants: stub.clone(),
            escalation_progress: stub.clone(),
            player_journey: stub.clone(),
            breadcrumbs: stub.clone(),
            encountered_content: stub.clone(),
            bounty_states: stub.clone(),
            cooldown_states: stub.clone(),
            entitlements: stub.clone(),
            wallets: stub.clone(),
            economy_rollups: stub.clone(),
            parties: stub.clone(),
            friendships: stub,
        }
    }

    /// What runs on `store` today: the accounts table (RU4), the rest stubs until their areas port them.
    pub fn on_store() -> Self {
        let progression = Arc::new(progression::SqlProgression);
        Self {
            accounts: Arc::new(auth::SqlAccounts),
            friendships: Arc::new(chat::SqlFriendships),
            invites: Arc::new(reforged::SqlInvites),
            characters: Arc::new(characters::SqlCharacters),
            loadouts: Arc::new(characters::SqlLoadouts),
            progression_tracks: progression.clone(),
            progression_grants: progression.clone(),
            escalation_progress: progression.clone(),
            player_journey: progression.clone(),
            breadcrumbs: progression.clone(),
            encountered_content: progression,
            bounty_states: Arc::new(bounties::SqlBountyStates),
            cooldown_states: Arc::new(bounties::SqlCooldownStates),
            entitlements: Arc::new(bounties::SqlEntitlements),
            wallets: Arc::new(bounties::SqlWallets),
            economy_rollups: Arc::new(bounties::SqlEconomyRollups),
            parties: Arc::new(parties::SqlParties),
            inventories: Arc::new(inventory::SqlInventories),
            inventory_transactions: Arc::new(inventory::SqlInventoryTransactions),
            launcher_guard: Arc::new(reforged::SqlLauncherGuard),
            admin: Arc::new(reforged::SqlAdmin),
            launcher_notices: Arc::new(reforged::SqlLauncherNotices),
            launcher_accounts: Arc::new(auth::SqlLauncherAccounts),
            auth_identities: Arc::new(auth::SqlAuthIdentities),
            refresh_sessions: Arc::new(auth::SqlRefreshSessions),
            game_exchange_codes: Arc::new(auth::SqlGameExchangeCodes),
            discord_oauth_transactions: Arc::new(auth::SqlDiscordOAuthTransactions),
            email_codes: Arc::new(auth::SqlEmailCodes),
            api_keys: Arc::new(auth::SqlApiKeys),
            ..Self::stubs()
        }
    }
}
