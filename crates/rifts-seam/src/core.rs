// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the module provides back to rifts.

use std::sync::Arc;

use crate::{
    error::{Component, SeamError},
    hooks::{Candidate, InventoryTransaction},
    http::{BoxFuture, Json},
    id::{AccountId, PartyId, SessionId},
    stub,
};

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Party {
    pub id: PartyId,
    pub leader: AccountId,
    pub members: Vec<AccountId>,
    pub revision: u64,
}

/// Whether Guard only records what it would refuse, or refuses it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GuardMode {
    #[default]
    Observe,
    Enforce,
}

impl GuardMode {
    /// Only an empty value or `observe` observes, so a typo (even `off`) fails closed into enforcing.
    pub fn from_setting(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") => Self::Observe,
            Some(word) if word.eq_ignore_ascii_case("observe") => Self::Observe,
            Some(_) => Self::Enforce,
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
    // Accounts and parties.
    fn display_name(&self, account: AccountId) -> Fut<'_, Option<String>>;
    fn find_by_display_name(&self, name: String) -> Fut<'_, Option<AccountId>>;
    fn launcher_account_eligible(&self, account: AccountId) -> Fut<'_, bool>;
    fn party(&self, party: PartyId) -> Fut<'_, Option<Party>>;
    fn get_or_create_party(&self, account: AccountId) -> Fut<'_, Party>;

    // Matchmaking and game sessions.
    fn dedicated_servers_configured(&self) -> bool;
    fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError>;
    fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError>;
    fn touch_game_session(&self, account: AccountId) -> Fut<'_, ()>;
    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> Fut<'_, ()>;

    // Guard and operations.
    fn guard_mode(&self) -> GuardMode;
    fn healthy_guard_session(&self, account: AccountId) -> Fut<'_, Option<Json>>;
    fn published_guard_artifacts(&self) -> Fut<'_, Json>;
    fn operations_policy(&self) -> Fut<'_, Json>;
    fn verify_operations_step_up(&self, key_id: String, step_up: String) -> Fut<'_, bool>;

    /// Absent: Guard then refuses every catalog-dependent grant.
    fn host_grant_catalog(&self) -> Option<Arc<dyn HostGrantCatalog>> {
        None
    }

    fn stacked_items(&self, character_id: String) -> Fut<'_, Option<Json>> {
        drop(character_id);
        stub(Component::Isles, "stacked_items")
    }

    fn accepted_inventory_replay(&self, transaction: InventoryTransaction) -> Fut<'_, bool> {
        drop(transaction);
        stub(Component::Isles, "accepted_inventory_replay")
    }
}

/// An `IslesCore` with nothing behind it: every call is `Unimplemented` under its own method name.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCore;

// Each listed method answers `Unimplemented` named after itself, as a ready future or at once.
macro_rules! not_written {
    (later: $($name:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $name(&self, $(_: $arg),*) -> Fut<'_, $out> {
            stub(Component::Isles, stringify!($name))
        }
    )*};
    (now: $($name:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $name(&self, $(_: $arg),*) -> Result<$out, SeamError> {
            Err(SeamError::unimplemented(Component::Isles, stringify!($name)))
        }
    )*};
}

impl IslesCore for NoCore {
    not_written! {
        later:
        display_name(AccountId) -> Option<String>;
        find_by_display_name(String) -> Option<AccountId>;
        launcher_account_eligible(AccountId) -> bool;
        party(PartyId) -> Option<Party>;
        get_or_create_party(AccountId) -> Party;
        touch_game_session(AccountId) -> ();
        record_hunt_activity(SessionId, Json) -> ();
        healthy_guard_session(AccountId) -> Option<Json>;
        published_guard_artifacts() -> Json;
        operations_policy() -> Json;
        verify_operations_step_up(String, String) -> bool;
    }

    not_written! {
        now:
        set_candidate(AccountId, Candidate) -> ();
        cancel_travel(&str, &[AccountId], &str) -> bool;
    }

    fn dedicated_servers_configured(&self) -> bool {
        false
    }

    fn guard_mode(&self) -> GuardMode {
        GuardMode::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poll_once;

    #[test]
    fn guard_mode_from_setting() {
        let observing = [
            None,
            Some(""),
            Some("  "),
            Some(" observe "),
            Some("OBSERVE"),
            Some("Observe"),
        ];
        let enforcing = [Some("enforce"), Some("off"), Some("observed"), Some("x")];
        let cases = observing
            .map(|value| (value, GuardMode::Observe))
            .into_iter()
            .chain(enforcing.map(|value| (value, GuardMode::Enforce)));
        for (value, mode) in cases {
            assert_eq!(GuardMode::from_setting(value), mode, "{value:?}");
        }
        assert_eq!(GuardMode::default(), GuardMode::Observe);
    }

    /// The operation an `Unimplemented` answer names, after checking it is the module's.
    fn operation_of<T: std::fmt::Debug>(answer: Fut<'_, T>) -> &'static str {
        let Some(Err(SeamError::Unimplemented(missing))) = poll_once(answer) else {
            panic!("expected a ready Unimplemented");
        };
        assert_eq!(missing.component, Component::Isles);
        missing.operation
    }

    #[test]
    fn no_core_answers_unimplemented_by_method() {
        let core = NoCore;
        let who = || AccountId("a".into());
        let global = || "01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap();
        let transaction = InventoryTransaction {
            user: who(),
            character_id: "c".into(),
            transaction_id: "t".into(),
            body: None,
        };
        let answered = [
            ("display_name", operation_of(core.display_name(who()))),
            (
                "find_by_display_name",
                operation_of(core.find_by_display_name("x".into())),
            ),
            (
                "launcher_account_eligible",
                operation_of(core.launcher_account_eligible(who())),
            ),
            ("party", operation_of(core.party(PartyId(global())))),
            ("get_or_create_party", operation_of(core.get_or_create_party(who()))),
            ("touch_game_session", operation_of(core.touch_game_session(who()))),
            (
                "record_hunt_activity",
                operation_of(core.record_hunt_activity(SessionId(global()), Json::empty_object())),
            ),
            ("healthy_guard_session", operation_of(core.healthy_guard_session(who()))),
            (
                "published_guard_artifacts",
                operation_of(core.published_guard_artifacts()),
            ),
            ("operations_policy", operation_of(core.operations_policy())),
            (
                "verify_operations_step_up",
                operation_of(core.verify_operations_step_up("k".into(), "s".into())),
            ),
            ("stacked_items", operation_of(core.stacked_items("c".into()))),
            (
                "accepted_inventory_replay",
                operation_of(core.accepted_inventory_replay(transaction)),
            ),
        ];
        for (method, operation) in answered {
            assert_eq!(operation, method);
        }

        let refused = |operation| SeamError::unimplemented(Component::Isles, operation);
        assert_eq!(
            core.set_candidate(who(), Candidate(Json::empty_object())),
            Err(refused("set_candidate"))
        );
        assert_eq!(core.cancel_travel("h", &[who()], "r"), Err(refused("cancel_travel")));
        assert!(!core.dedicated_servers_configured());
        assert_eq!(core.guard_mode(), GuardMode::Observe);
        assert!(core.host_grant_catalog().is_none());
    }
}
