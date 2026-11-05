// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One trait per repository (spec §5). A read takes the closure's connection, a write its `Tx`, so the change log
//! sees every row changed. Every method has a default answering `Unimplemented`, which is what [`Stub`] is.

/// Declares a repository trait whose methods default to `Unimplemented { repository: $label, operation }`, and
/// implements it for [`Stub`].
macro_rules! repository {
    (
        $(#[$doc:meta])*
        pub trait $name:ident = $label:literal {
            $( $(#[$mdoc:meta])* fn $method:ident(&self $(, $arg:ident: $ty:ty)* $(,)?) -> $ret:ty; )*
        }
    ) => {
        $(#[$doc])*
        pub trait $name: Send + Sync {
            $(
                $(#[$mdoc])*
                fn $method(&self $(, $arg: $ty)*) -> $crate::RepoResult<$ret> {
                    $(let _ = $arg;)*
                    Err($crate::RepoError::unimplemented($label, stringify!($method)))
                }
            )*
        }

        impl $name for $crate::repo::Stub {}
    };
}

pub mod auth;
pub mod bounties;
pub mod characters;
pub mod chat;
pub mod inventory;
pub mod parties;
pub mod progression;
pub mod reforged;

use std::sync::Arc;

/// Answers `Unimplemented` for every method of every repository: tests and area fakes start from it.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stub;

/// The repositories a store hands its closures; the field names are what the areas are written against.
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
    /// Every repository on this crate's SQL.
    pub fn on_store() -> Self {
        Self {
            accounts: Arc::new(auth::SqlAccounts),
            launcher_accounts: Arc::new(auth::SqlLauncherAccounts),
            auth_identities: Arc::new(auth::SqlAuthIdentities),
            refresh_sessions: Arc::new(auth::SqlRefreshSessions),
            game_exchange_codes: Arc::new(auth::SqlGameExchangeCodes),
            discord_oauth_transactions: Arc::new(auth::SqlDiscordOAuthTransactions),
            email_codes: Arc::new(auth::SqlEmailCodes),
            api_keys: Arc::new(auth::SqlApiKeys),
            admin: Arc::new(reforged::SqlAdmin),
            invites: Arc::new(reforged::SqlInvites),
            launcher_notices: Arc::new(reforged::SqlLauncherNotices),
            activity: Arc::new(reforged::SqlActivity),
            launcher_guard: Arc::new(reforged::SqlLauncherGuard),
            characters: Arc::new(characters::SqlCharacters),
            loadouts: Arc::new(characters::SqlLoadouts),
            inventories: Arc::new(inventory::SqlInventories),
            inventory_transactions: Arc::new(inventory::SqlInventoryTransactions),
            progression_tracks: Arc::new(progression::SqlProgression),
            progression_grants: Arc::new(progression::SqlProgression),
            escalation_progress: Arc::new(progression::SqlProgression),
            player_journey: Arc::new(progression::SqlProgression),
            breadcrumbs: Arc::new(progression::SqlProgression),
            encountered_content: Arc::new(progression::SqlProgression),
            bounty_states: Arc::new(bounties::SqlBountyStates),
            cooldown_states: Arc::new(bounties::SqlCooldownStates),
            entitlements: Arc::new(bounties::SqlEntitlements),
            wallets: Arc::new(bounties::SqlWallets),
            economy_rollups: Arc::new(bounties::SqlEconomyRollups),
            parties: Arc::new(parties::SqlParties),
            friendships: Arc::new(chat::SqlFriendships),
        }
    }

    /// Every repository [`Stub`].
    pub fn stubs() -> Self {
        Self {
            accounts: Arc::new(Stub),
            launcher_accounts: Arc::new(Stub),
            auth_identities: Arc::new(Stub),
            refresh_sessions: Arc::new(Stub),
            game_exchange_codes: Arc::new(Stub),
            discord_oauth_transactions: Arc::new(Stub),
            email_codes: Arc::new(Stub),
            api_keys: Arc::new(Stub),
            admin: Arc::new(Stub),
            invites: Arc::new(Stub),
            launcher_notices: Arc::new(Stub),
            activity: Arc::new(Stub),
            launcher_guard: Arc::new(Stub),
            characters: Arc::new(Stub),
            loadouts: Arc::new(Stub),
            inventories: Arc::new(Stub),
            inventory_transactions: Arc::new(Stub),
            progression_tracks: Arc::new(Stub),
            progression_grants: Arc::new(Stub),
            escalation_progress: Arc::new(Stub),
            player_journey: Arc::new(Stub),
            breadcrumbs: Arc::new(Stub),
            encountered_content: Arc::new(Stub),
            bounty_states: Arc::new(Stub),
            cooldown_states: Arc::new(Stub),
            entitlements: Arc::new(Stub),
            wallets: Arc::new(Stub),
            economy_rollups: Arc::new(Stub),
            parties: Arc::new(Stub),
            friendships: Arc::new(Stub),
        }
    }
}
