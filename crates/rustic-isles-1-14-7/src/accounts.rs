// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Rifts' fallback core over the module's accounts and the Reforged operations policy (spec §5.6.1).

use std::sync::Arc;

use dainty_module_api::GlobalId;
use rustic_isles::Settings;
use rustic_isles_auth::{AuthStore, LauncherAccount, StoreError};
use rustic_isles_store::{IslesAuthStore, IslesStore};
use rustic_rifts_seam::{
    AccountId, BoxFuture, Candidate, GuardMode, IslesCore, Json, NoCore, Party, PartyId, Refusal, SeamError, SessionId,
};
use serde_json::Value;

use crate::composer::SharedCell;

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

fn read_failed(error: StoreError) -> SeamError {
    SeamError::Refused(Refusal {
        code: "ACCOUNT_READ_FAILED".into(),
        message: error.message,
    })
}

/// Accounts cross as their userId text; operations read the module's `Shared` through the late cell.
pub struct AccountsCore {
    store: Option<IslesAuthStore>,
    settings: Settings,
    shared: Arc<SharedCell>,
}

// The store and settings may reach secrets: names only
impl std::fmt::Debug for AccountsCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountsCore")
            .field("store", &self.store.is_some())
            .finish_non_exhaustive()
    }
}

impl AccountsCore {
    /// Builds the core over the module's store (none without a database), its settings and the late `Shared` cell.
    pub fn new(store: Option<IslesStore>, settings: Settings, shared: Arc<SharedCell>) -> Self {
        Self {
            store: store.map(IslesAuthStore::new),
            settings,
            shared,
        }
    }

    /// By global id, else through the game account with that userId.
    async fn launcher_account(&self, store: &IslesAuthStore, text: &str) -> Result<Lookup, SeamError> {
        if text.parse::<GlobalId>().is_ok() {
            let found = store.launcher_account_by_id(text).await.map_err(read_failed)?;
            return Ok(Lookup {
                launcher: found,
                game_name: None,
            });
        }
        let Some(game) = store.game_account_by_user_id(text).await.map_err(read_failed)? else {
            return Ok(Lookup::default());
        };
        let launcher = store.launcher_account_by_id(&game.id).await.map_err(read_failed)?;
        Ok(Lookup {
            launcher,
            game_name: Some(game.name),
        })
    }
}

#[derive(Default)]
struct Lookup {
    launcher: Option<LauncherAccount>,
    game_name: Option<String>,
}

fn now_iso() -> String {
    rustic_isles::season::to_iso_string(rustic_isles::replay::now_ms())
}

impl IslesCore for AccountsCore {
    fn display_name(&self, account: AccountId) -> Fut<'_, Option<String>> {
        Box::pin(async move {
            let Some(store) = &self.store else {
                return Ok(None);
            };
            let found = self.launcher_account(store, &account.0).await?;
            Ok(found.launcher.map(|l| l.display_name).or(found.game_name))
        })
    }

    fn find_by_display_name(&self, name: String) -> Fut<'_, Option<AccountId>> {
        Box::pin(async move {
            let Some(store) = &self.store else {
                return Ok(None);
            };
            let normalized = name.trim().to_lowercase();
            let found = store.launcher_account_by_name(&normalized).await.map_err(read_failed)?;
            Ok(found.map(|l| AccountId(l.user_id)))
        })
    }

    /// An account rifts cannot find is eligible (§14 item 12).
    fn launcher_account_eligible(&self, account: AccountId) -> Fut<'_, bool> {
        Box::pin(async move {
            let Some(store) = &self.store else {
                return Ok(true);
            };
            let found = self.launcher_account(store, &account.0).await?;
            Ok(found
                .launcher
                .is_none_or(|l| rustic_isles_auth::is_account_eligible(&l, &now_iso())))
        })
    }

    fn party(&self, party: PartyId) -> Fut<'_, Option<Party>> {
        NoCore.party(party)
    }

    fn get_or_create_party(&self, account: AccountId) -> Fut<'_, Party> {
        NoCore.get_or_create_party(account)
    }

    fn dedicated_servers_configured(&self) -> bool {
        false
    }

    fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError> {
        NoCore.set_candidate(account, candidate)
    }

    fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError> {
        NoCore.cancel_travel(hunt_id, accounts, reason)
    }

    fn touch_game_session(&self, account: AccountId) -> Fut<'_, ()> {
        NoCore.touch_game_session(account)
    }

    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> Fut<'_, ()> {
        NoCore.record_hunt_activity(session, activity)
    }

    /// Only empty or `observe` observes.
    fn guard_mode(&self) -> GuardMode {
        GuardMode::from_setting(self.settings.raw("RUSTIC_ISLES_GUARD_MODE"))
    }

    fn healthy_guard_session(&self, account: AccountId) -> Fut<'_, Option<Json>> {
        NoCore.healthy_guard_session(account)
    }

    fn published_guard_artifacts(&self) -> Fut<'_, Json> {
        NoCore.published_guard_artifacts()
    }

    fn operations_policy(&self) -> Fut<'_, Json> {
        Box::pin(async move {
            let text = match self.shared.get() {
                Some(shared) => rustic_isles_launcher_api::operations::operations_policy_json(&shared).await,
                None => dainty_service_kit::jsjson::stringify(
                    &rustic_isles_launcher_api::operations::default_policy().to_json(),
                ),
            };
            Ok(Json(text))
        })
    }

    fn verify_operations_step_up(&self, key_id: String, step_up: String) -> Fut<'_, bool> {
        Box::pin(async move {
            let Some(shared) = self.shared.get() else {
                return Ok(false);
            };
            let code = Value::String(step_up);
            Ok(rustic_isles_launcher_api::operations::verify_operations_step_up(
                &shared,
                &key_id,
                Some(&code),
            ))
        })
    }
}
