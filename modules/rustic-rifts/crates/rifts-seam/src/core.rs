//! What realms take from isles (RF1, section 2.2), implemented by `isles-1-14-7` and handed to the rifts crates.
//!
//! Only the calls that cross today are here. Persistence (`GetRepositories`, `GetUnitOfWork`) is DF7's `store`; hunt
//! data and the economy (game data, the hunt manifest, inventory and wallet rules) come with RU4's `isles-core`
//! traits; metrics and logs are `tracing`; the Director is reached through the module framework (DF2). Each joins
//! this trait, or replaces a method in it, as those rows land.

use std::sync::Arc;

use crate::error::{Component, SeamError};
use crate::hooks::{Candidate, InventoryTransaction};
use crate::http::{BoxFuture, Json};
use crate::id::{AccountId, PartyId, SessionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    pub id: PartyId,
    pub leader: AccountId,
    pub members: Vec<AccountId>,
    pub revision: u64,
}

/// `MYSTICPARADOX_GUARD_MODE` (`services/guardMode.ts`): blank or `observe` observes, anything else enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuardMode {
    #[default]
    Observe,
    Enforce,
}

impl GuardMode {
    pub fn from_setting(value: Option<&str>) -> Self {
        let value = value.unwrap_or_default().trim();
        if value.is_empty() || value.eq_ignore_ascii_case("observe") {
            Self::Observe
        } else {
            Self::Enforce
        }
    }
}

/// What `hostGrantPolicy.ts`'s `HostGrantCatalog` reads from game data (`DefaultHostGrantCatalog`).
pub trait HostGrantCatalog: Send + Sync {
    /// The id's tags in `inventory_storage_policy.json`.
    fn tags_of(&self, catalog_id: &str) -> Option<Vec<String>>;
    /// A prestige reward currency of `progression_config.json` (free and premium rewards).
    fn is_prestige_reward(&self, catalog_id: &str) -> bool;
    /// `IsPrestigeGatedReward`: the wallet's own rule, never a copy.
    fn is_prestige_gated(&self, catalog_id: &str) -> bool;
    /// An entitlement name game data grants or gates on; `name` is lower case.
    fn is_entitlement_name(&self, name: &str) -> bool;
}

pub trait IslesCore: Send + Sync {
    // Accounts and auth
    fn display_name(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, SeamError>>;
    /// `launcherAccounts.findByDisplayNameNormalized`.
    fn find_by_display_name(&self, name: String) -> BoxFuture<'_, Result<Option<AccountId>, SeamError>>;
    /// `IsCurrentLauncherAccountEligible`.
    fn launcher_account_eligible(&self, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>>;

    // Sessions, parties, matchmaking
    fn party(&self, party: PartyId) -> BoxFuture<'_, Result<Option<Party>, SeamError>>;
    fn get_or_create_party(&self, account: AccountId) -> BoxFuture<'_, Result<Party, SeamError>>;
    /// `HuntRouteContext.setCandidate`: publishes one account's travel candidate.
    fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError>;
    /// `HuntRouteContext.cancel`: ends these accounts' pending travel with a reason code.
    fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError>;
    /// `DedicatedServersConfigured`.
    fn dedicated_servers_configured(&self) -> bool;
    /// `TouchGameSession`.
    fn touch_game_session(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>>;
    /// `RecordP2PHuntActivity`: a player-hosted hunt row in `activityHunts`.
    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> BoxFuture<'_, Result<(), SeamError>>;

    // Launcher feed and Guard (RF1, decision 1: isles keeps Launcher Guard until that is settled)
    /// `launcherGuard.findHealthyForAccount`.
    fn healthy_guard_session(&self, account: AccountId) -> BoxFuture<'_, Result<Option<Json>, SeamError>>;
    /// `ReadPublishedGuardArtifacts`: the signed manifest's artifact hashes.
    fn published_guard_artifacts(&self) -> BoxFuture<'_, Result<Json, SeamError>>;
    fn guard_mode(&self) -> GuardMode;

    // Operations
    /// `ReadOperationsPolicyRecord` and `ResolveOperationsPolicy`: the `operationsPolicy` document.
    fn operations_policy(&self) -> BoxFuture<'_, Result<Json, SeamError>>;
    /// `VerifyOperationsStepUp` for the admin operations routes.
    fn verify_operations_step_up(&self, key_id: String, step_up: String) -> BoxFuture<'_, Result<bool, SeamError>>;

    // Inventory, for Guard's host grant policy (RF6). The defaults are Node's without game data or the reads.
    /// `DefaultHostGrantCatalog`; `None` leaves only the id rules.
    fn host_grant_catalog(&self) -> Option<Arc<dyn HostGrantCatalog>> {
        None
    }
    /// `inventories.findByCharacterId(...).stackedItems` as stored; `Ok(None)` when the character has no inventory.
    fn stacked_items(&self, _character_id: String) -> BoxFuture<'_, Result<Option<Json>, SeamError>> {
        absent("stacked_items")
    }
    /// `IsAcceptedReplay`: a completed ledger row for this transaction id with the body's request hash.
    fn accepted_inventory_replay(&self, _transaction: InventoryTransaction) -> BoxFuture<'_, Result<bool, SeamError>> {
        absent("accepted_inventory_replay")
    }
}

/// An isles that answers nothing: lets the rifts crates' tests run without a core.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCore;

fn absent<T: Send + 'static>(operation: &'static str) -> BoxFuture<'static, Result<T, SeamError>> {
    crate::stub(Component::Isles, operation)
}

impl IslesCore for NoCore {
    fn display_name(&self, _: AccountId) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        absent("display_name")
    }
    fn find_by_display_name(&self, _: String) -> BoxFuture<'_, Result<Option<AccountId>, SeamError>> {
        absent("find_by_display_name")
    }
    fn launcher_account_eligible(&self, _: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        absent("launcher_account_eligible")
    }
    fn party(&self, _: PartyId) -> BoxFuture<'_, Result<Option<Party>, SeamError>> {
        absent("party")
    }
    fn get_or_create_party(&self, _: AccountId) -> BoxFuture<'_, Result<Party, SeamError>> {
        absent("get_or_create_party")
    }
    fn set_candidate(&self, _: AccountId, _: Candidate) -> Result<(), SeamError> {
        Err(SeamError::unimplemented(Component::Isles, "set_candidate"))
    }
    fn cancel_travel(&self, _: &str, _: &[AccountId], _: &str) -> Result<bool, SeamError> {
        Err(SeamError::unimplemented(Component::Isles, "cancel_travel"))
    }
    fn dedicated_servers_configured(&self) -> bool {
        false
    }
    fn touch_game_session(&self, _: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        absent("touch_game_session")
    }
    fn record_hunt_activity(&self, _: SessionId, _: Json) -> BoxFuture<'_, Result<(), SeamError>> {
        absent("record_hunt_activity")
    }
    fn healthy_guard_session(&self, _: AccountId) -> BoxFuture<'_, Result<Option<Json>, SeamError>> {
        absent("healthy_guard_session")
    }
    fn published_guard_artifacts(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
        absent("published_guard_artifacts")
    }
    fn guard_mode(&self) -> GuardMode {
        GuardMode::Observe
    }
    fn operations_policy(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
        absent("operations_policy")
    }
    fn verify_operations_step_up(&self, _: String, _: String) -> BoxFuture<'_, Result<bool, SeamError>> {
        absent("verify_operations_step_up")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poll_once;

    #[test]
    fn guard_mode_enforces_anything_but_observe() {
        assert_eq!(GuardMode::from_setting(None), GuardMode::Observe);
        assert_eq!(GuardMode::from_setting(Some(" observe ")), GuardMode::Observe);
        assert_eq!(GuardMode::from_setting(Some("OBSERVE")), GuardMode::Observe);
        assert_eq!(GuardMode::from_setting(Some("enforce")), GuardMode::Enforce);
        assert_eq!(GuardMode::from_setting(Some("off")), GuardMode::Enforce);
    }

    #[test]
    fn no_core_answers_unimplemented() {
        let core = NoCore;
        let error = poll_once(core.display_name(AccountId("a".to_owned())))
            .unwrap()
            .unwrap_err();
        assert_eq!(error, SeamError::unimplemented(Component::Isles, "display_name"));
        assert!(
            core.set_candidate(AccountId("a".to_owned()), Candidate(Json::empty_object()))
                .is_err()
        );
    }
}
