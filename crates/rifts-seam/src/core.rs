// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What the module provides back to rifts.

use std::sync::Arc;

use crate::error::{Component, SeamError};
use crate::hooks::{Candidate, InventoryTransaction};
use crate::http::{BoxFuture, Json};
use crate::id::{AccountId, PartyId, SessionId};
use crate::stub;

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    pub id: PartyId,
    pub leader: AccountId,
    pub members: Vec<AccountId>,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuardMode {
    #[default]
    Observe,
    Enforce,
}

impl GuardMode {
    /// Only an empty value or `observe` observes, so a typo (even `off`) fails closed into enforcing.
    pub fn from_setting(value: Option<&str>) -> Self {
        let value = value.unwrap_or("").trim();
        if value.is_empty() || value.eq_ignore_ascii_case("observe") {
            Self::Observe
        } else {
            Self::Enforce
        }
    }
}

/// What the module's item catalog says about the items a host may grant.
pub trait HostGrantCatalog: Send + Sync {
    fn tags_of(&self, catalog_id: &str) -> Option<Vec<String>>;
    fn is_prestige_reward(&self, catalog_id: &str) -> bool;
    fn is_prestige_gated(&self, catalog_id: &str) -> bool;
    /// `name` is lower case.
    fn is_entitlement_name(&self, name: &str) -> bool;
}

pub trait IslesCore: Send + Sync {
    fn display_name(&self, account: AccountId) -> Fut<'_, Option<String>>;
    fn find_by_display_name(&self, name: String) -> Fut<'_, Option<AccountId>>;
    fn launcher_account_eligible(&self, account: AccountId) -> Fut<'_, bool>;
    fn party(&self, party: PartyId) -> Fut<'_, Option<Party>>;
    fn get_or_create_party(&self, account: AccountId) -> Fut<'_, Party>;
    fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError>;
    fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError>;
    fn dedicated_servers_configured(&self) -> bool;
    fn touch_game_session(&self, account: AccountId) -> Fut<'_, ()>;
    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> Fut<'_, ()>;
    fn healthy_guard_session(&self, account: AccountId) -> Fut<'_, Option<Json>>;
    fn published_guard_artifacts(&self) -> Fut<'_, Json>;
    fn guard_mode(&self) -> GuardMode;
    fn operations_policy(&self) -> Fut<'_, Json>;
    fn verify_operations_step_up(&self, key_id: String, step_up: String) -> Fut<'_, bool>;

    fn host_grant_catalog(&self) -> Option<Arc<dyn HostGrantCatalog>> {
        None
    }

    fn stacked_items(&self, character_id: String) -> Fut<'_, Option<Json>> {
        let _ = character_id;
        stub(Component::Isles, "stacked_items")
    }

    fn accepted_inventory_replay(&self, transaction: InventoryTransaction) -> Fut<'_, bool> {
        let _ = transaction;
        stub(Component::Isles, "accepted_inventory_replay")
    }
}

/// An `IslesCore` with nothing behind it: every call is `Unimplemented` under its own method name.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCore;

impl IslesCore for NoCore {
    fn display_name(&self, _account: AccountId) -> Fut<'_, Option<String>> {
        stub(Component::Isles, "display_name")
    }

    fn find_by_display_name(&self, _name: String) -> Fut<'_, Option<AccountId>> {
        stub(Component::Isles, "find_by_display_name")
    }

    fn launcher_account_eligible(&self, _account: AccountId) -> Fut<'_, bool> {
        stub(Component::Isles, "launcher_account_eligible")
    }

    fn party(&self, _party: PartyId) -> Fut<'_, Option<Party>> {
        stub(Component::Isles, "party")
    }

    fn get_or_create_party(&self, _account: AccountId) -> Fut<'_, Party> {
        stub(Component::Isles, "get_or_create_party")
    }

    fn set_candidate(&self, _account: AccountId, _candidate: Candidate) -> Result<(), SeamError> {
        Err(SeamError::unimplemented(Component::Isles, "set_candidate"))
    }

    fn cancel_travel(&self, _hunt_id: &str, _accounts: &[AccountId], _reason: &str) -> Result<bool, SeamError> {
        Err(SeamError::unimplemented(Component::Isles, "cancel_travel"))
    }

    fn dedicated_servers_configured(&self) -> bool {
        false
    }

    fn touch_game_session(&self, _account: AccountId) -> Fut<'_, ()> {
        stub(Component::Isles, "touch_game_session")
    }

    fn record_hunt_activity(&self, _session: SessionId, _activity: Json) -> Fut<'_, ()> {
        stub(Component::Isles, "record_hunt_activity")
    }

    fn healthy_guard_session(&self, _account: AccountId) -> Fut<'_, Option<Json>> {
        stub(Component::Isles, "healthy_guard_session")
    }

    fn published_guard_artifacts(&self) -> Fut<'_, Json> {
        stub(Component::Isles, "published_guard_artifacts")
    }

    fn guard_mode(&self) -> GuardMode {
        GuardMode::Observe
    }

    fn operations_policy(&self) -> Fut<'_, Json> {
        stub(Component::Isles, "operations_policy")
    }

    fn verify_operations_step_up(&self, _key_id: String, _step_up: String) -> Fut<'_, bool> {
        stub(Component::Isles, "verify_operations_step_up")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::GlobalId;
    use crate::poll_once;

    #[test]
    fn guard_mode_from_setting() {
        for observe in [
            None,
            Some(""),
            Some("  "),
            Some(" observe "),
            Some("OBSERVE"),
            Some("Observe"),
        ] {
            assert_eq!(GuardMode::from_setting(observe), GuardMode::Observe, "{observe:?}");
        }
        for enforce in [Some("enforce"), Some("off"), Some("observed"), Some("x")] {
            assert_eq!(GuardMode::from_setting(enforce), GuardMode::Enforce, "{enforce:?}");
        }
        assert_eq!(GuardMode::default(), GuardMode::Observe);
    }

    fn unimplemented<T: std::fmt::Debug>(future: Fut<'_, T>) -> String {
        match poll_once(future) {
            Some(Err(SeamError::Unimplemented(missing))) => {
                assert_eq!(missing.component, Component::Isles);
                missing.operation.to_owned()
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn no_core_answers_unimplemented_by_method() {
        let core = NoCore;
        let account = || AccountId("a".into());
        let id = || -> GlobalId { "01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap() };
        let names = [
            unimplemented(core.display_name(account())),
            unimplemented(core.find_by_display_name("x".into())),
            unimplemented(core.launcher_account_eligible(account())),
            unimplemented(core.party(PartyId(id()))),
            unimplemented(core.get_or_create_party(account())),
            unimplemented(core.touch_game_session(account())),
            unimplemented(core.record_hunt_activity(SessionId(id()), Json::empty_object())),
            unimplemented(core.healthy_guard_session(account())),
            unimplemented(core.published_guard_artifacts()),
            unimplemented(core.operations_policy()),
            unimplemented(core.verify_operations_step_up("k".into(), "s".into())),
            unimplemented(core.stacked_items("c".into())),
            unimplemented(core.accepted_inventory_replay(InventoryTransaction {
                user: account(),
                character_id: "c".into(),
                transaction_id: "t".into(),
                body: None,
            })),
        ];
        assert_eq!(
            names,
            [
                "display_name",
                "find_by_display_name",
                "launcher_account_eligible",
                "party",
                "get_or_create_party",
                "touch_game_session",
                "record_hunt_activity",
                "healthy_guard_session",
                "published_guard_artifacts",
                "operations_policy",
                "verify_operations_step_up",
                "stacked_items",
                "accepted_inventory_replay",
            ]
        );
        assert_eq!(
            core.set_candidate(account(), Candidate(Json::empty_object())),
            Err(SeamError::unimplemented(Component::Isles, "set_candidate"))
        );
        assert_eq!(
            core.cancel_travel("h", &[account()], "r"),
            Err(SeamError::unimplemented(Component::Isles, "cancel_travel"))
        );
        assert!(!core.dedicated_servers_configured());
        assert_eq!(core.guard_mode(), GuardMode::Observe);
        assert!(core.host_grant_catalog().is_none());
    }
}
