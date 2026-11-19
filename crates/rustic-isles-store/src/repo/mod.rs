// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One trait per repository (spec §5). A read takes the closure's connection, a write its `Tx`, so the change log
//! sees every row changed. Every method has a default answering `Unimplemented`, which is what [`Stub`] is.

/// Declares a store trait whose methods all fall back to `Unimplemented { repository: <name>, operation }`, and
/// gives [`Stub`] that fallback.
macro_rules! store_trait {
    (
        $(#[$trait_attr:meta])*
        pub trait $trait_name:ident = $repo_name:literal {
            $( $(#[$fn_attr:meta])* fn $op:ident(&self $(, $param:ident: $param_ty:ty)* $(,)?) -> $answer:ty; )*
        }
    ) => {
        $(#[$trait_attr])*
        pub trait $trait_name: Send + Sync {
            $(
                $(#[$fn_attr])*
                fn $op(&self $(, $param: $param_ty)*) -> $crate::RepoResult<$answer> {
                    $(let _ = $param;)*
                    Err($crate::RepoError::unimplemented($repo_name, stringify!($op)))
                }
            )*
        }

        impl $trait_name for $crate::repo::Stub {}
    };
}

pub mod auth;
pub mod bounties;
pub mod characters;
pub mod chat;
pub mod gauntlet;
pub mod inventory;
pub mod parties;
pub mod playtime;
pub mod progression;
pub mod reforged;
pub mod resets;
pub mod trials;

use std::sync::Arc;

/// Answers `Unimplemented` for every method of every repository: tests and area fakes start from it.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stub;

/// The repositories a store hands its closures; the field names are what the areas are written against.
#[derive(Clone)]
pub struct Repositories {
    /// Game accounts.
    pub accounts: Arc<dyn auth::AccountRepository>,
    /// Launcher accounts.
    pub launcher_accounts: Arc<dyn auth::LauncherAccountRepository>,
    /// Linked Discord identities.
    pub auth_identities: Arc<dyn auth::AuthIdentityRepository>,
    /// Launcher refresh sessions.
    pub refresh_sessions: Arc<dyn auth::RefreshSessionRepository>,
    /// One-time codes from the launcher to the game.
    pub game_exchange_codes: Arc<dyn auth::GameExchangeCodeRepository>,
    /// Discord sign-in transactions.
    pub discord_oauth_transactions: Arc<dyn auth::DiscordOAuthTransactionRepository>,
    /// Emailed one-time codes.
    pub email_codes: Arc<dyn auth::EmailCodeRepository>,
    /// Game server and user API key hashes.
    pub api_keys: Arc<dyn auth::ApiKeyRepository>,
    /// The Reforged API's admin data.
    pub admin: Arc<dyn reforged::AdminRepository>,
    /// Reforged invites.
    pub invites: Arc<dyn reforged::InviteRepository>,
    /// The launcher's notices.
    pub launcher_notices: Arc<dyn reforged::LauncherNoticesRepository>,
    /// Activity records.
    pub activity: Arc<dyn reforged::ActivityRepository>,
    /// Launcher guard data.
    pub launcher_guard: Arc<dyn reforged::LauncherGuardRepository>,
    /// Characters.
    pub characters: Arc<dyn characters::CharacterRepository>,
    /// Loadouts.
    pub loadouts: Arc<dyn characters::LoadoutRepository>,
    /// Inventories.
    pub inventories: Arc<dyn inventory::InventoryRepository>,
    /// The inventory transaction ledger.
    pub inventory_transactions: Arc<dyn inventory::InventoryTransactionRepository>,
    /// Progression tracks.
    pub progression_tracks: Arc<dyn progression::ProgressionTrackRepository>,
    /// The progression grant ledger.
    pub progression_grants: Arc<dyn progression::ProgressionGrantRepository>,
    /// Escalation progress.
    pub escalation_progress: Arc<dyn progression::EscalationProgressRepository>,
    /// Slayer's Path journeys.
    pub player_journey: Arc<dyn progression::PlayerJourneyRepository>,
    /// Breadcrumbs.
    pub breadcrumbs: Arc<dyn progression::BreadcrumbRepository>,
    /// Encountered content.
    pub encountered_content: Arc<dyn progression::EncounteredContentRepository>,
    /// Bounty states.
    pub bounty_states: Arc<dyn bounties::BountyStateRepository>,
    /// Cooldown states.
    pub cooldown_states: Arc<dyn bounties::CooldownStateRepository>,
    /// Entitlements.
    pub entitlements: Arc<dyn bounties::EntitlementRepository>,
    /// Wallets.
    pub wallets: Arc<dyn bounties::WalletRepository>,
    /// Daily economy rollups.
    pub economy_rollups: Arc<dyn bounties::EconomyRollupRepository>,
    /// Parties.
    pub parties: Arc<dyn parties::PartyRepository>,
    /// Friendships.
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
