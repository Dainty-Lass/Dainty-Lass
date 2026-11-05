// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Matchmaking's answers to rustic-rifts (spec §11, RF1 §2.2); the rest goes to the composer's core. Accounts cross
//! as their userId text.

use std::sync::Arc;

use module_api::{GlobalId, NodeId};
use rifts_seam::{
    AccountId, BoxFuture, Candidate, GuardMode, HostGrantCatalog, InventoryTransaction, IslesCore, Json, Party,
    PartyId, Refusal, SeamError, SessionId,
};
use rustic_isles_store::IslesStore;
use rustic_isles_store::repo::parties::PartyRecord;

use crate::candidate::MatchCandidate;
use crate::party::{Parties, PartyError};
use crate::state::MatchmakingState;

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

fn refusal(code: &str, message: impl Into<String>) -> SeamError {
    SeamError::Refused(Refusal {
        code: code.to_owned(),
        message: message.into(),
    })
}

/// Rifts' `<ULID>@<node>` as isles' `party:<ULID>@<node>`.
pub fn isles_party_id(party: &PartyId) -> Option<GlobalId> {
    format!("party:{party}").parse().ok()
}

fn seam_party(record: &PartyRecord) -> Result<Party, SeamError> {
    let text = format!("{}@{}", record.id.ulid(), record.id.owner());
    let id: PartyId = text
        .parse()
        .map_err(|_| refusal("INVALID_PARTY", "a party id is not a ULID and a node"))?;
    let user = |m: &rustic_isles_store::repo::parties::PartyMember| AccountId(m.user_id.as_str().to_owned());
    Ok(Party {
        id,
        leader: user(&record.leader),
        members: record.members.iter().map(user).collect(),
        revision: u64::try_from(record.revision).unwrap_or(0),
    })
}

pub struct MatchmakingCore {
    store: Option<IslesStore>,
    node: NodeId,
    state: Arc<MatchmakingState>,
    fallback: Arc<dyn IslesCore>,
}

impl MatchmakingCore {
    pub fn new(
        store: Option<IslesStore>,
        node: NodeId,
        state: Arc<MatchmakingState>,
        fallback: Arc<dyn IslesCore>,
    ) -> Self {
        Self {
            store,
            node,
            state,
            fallback,
        }
    }

    fn parties(&self) -> Parties<'_> {
        Parties {
            store: self.store.as_ref(),
            node: &self.node,
            request_id: None,
            now: self.state.now_ms(),
        }
    }
}

impl IslesCore for MatchmakingCore {
    fn display_name(&self, account: AccountId) -> Fut<'_, Option<String>> {
        self.fallback.display_name(account)
    }

    fn find_by_display_name(&self, name: String) -> Fut<'_, Option<AccountId>> {
        self.fallback.find_by_display_name(name)
    }

    fn launcher_account_eligible(&self, account: AccountId) -> Fut<'_, bool> {
        self.fallback.launcher_account_eligible(account)
    }

    fn party(&self, party: PartyId) -> Fut<'_, Option<Party>> {
        Box::pin(async move {
            let Some(id) = isles_party_id(&party) else {
                return Ok(None);
            };
            let Some(store) = &self.store else {
                return Err(refusal("NO_DATABASE", "no database is configured"));
            };
            let found = store
                .read(move |conn, repos| repos.parties.find_by_id(conn, &id))
                .await
                .map_err(|e| refusal("PARTY_READ_FAILED", e.to_string()))?;
            found.as_ref().map(seam_party).transpose()
        })
    }

    fn get_or_create_party(&self, account: AccountId) -> Fut<'_, Party> {
        Box::pin(async move {
            // The realms side does not know the client's build; the party keeps it blank until a client names one
            let record = self
                .parties()
                .get_or_create(&account.0, "")
                .await
                .map_err(|e: PartyError| refusal("PARTY_WRITE_FAILED", e.to_string()))?;
            seam_party(&record)
        })
    }

    fn dedicated_servers_configured(&self) -> bool {
        self.state
            .shared()
            .is_some_and(|shared| shared.dedicated_servers().configured())
    }

    fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError> {
        let value: serde_json::Value = serde_json::from_str(&candidate.0.0)
            .map_err(|e| refusal("INVALID_CANDIDATE", format!("a candidate is a JSON object: {e}")))?;
        let read = MatchCandidate::from_json(&value).ok_or_else(|| {
            refusal(
                "INVALID_CANDIDATE",
                "a candidate has HuntId, CandidateId and GameSessionId",
            )
        })?;
        self.state.set_candidate(&account.0, read);
        Ok(())
    }

    fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError> {
        let players: Vec<String> = accounts.iter().map(|a| a.0.clone()).collect();
        Ok(self.state.cancel_hunt(hunt_id, &players, reason))
    }

    fn touch_game_session(&self, account: AccountId) -> Fut<'_, ()> {
        let now = self.state.now_ms();
        self.state.presence().touch(&account.0, now);
        Box::pin(std::future::ready(Ok(())))
    }

    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> Fut<'_, ()> {
        self.fallback.record_hunt_activity(session, activity)
    }

    fn guard_mode(&self) -> GuardMode {
        self.fallback.guard_mode()
    }

    fn healthy_guard_session(&self, account: AccountId) -> Fut<'_, Option<Json>> {
        self.fallback.healthy_guard_session(account)
    }

    fn published_guard_artifacts(&self) -> Fut<'_, Json> {
        self.fallback.published_guard_artifacts()
    }

    fn operations_policy(&self) -> Fut<'_, Json> {
        self.fallback.operations_policy()
    }

    fn verify_operations_step_up(&self, key_id: String, step_up: String) -> Fut<'_, bool> {
        self.fallback.verify_operations_step_up(key_id, step_up)
    }

    // Forwarded too, so they answer correctly once economy implements them (spec §15 item 11)
    fn host_grant_catalog(&self) -> Option<Arc<dyn HostGrantCatalog>> {
        self.fallback.host_grant_catalog()
    }

    fn stacked_items(&self, character_id: String) -> Fut<'_, Option<Json>> {
        self.fallback.stacked_items(character_id)
    }

    fn accepted_inventory_replay(&self, transaction: InventoryTransaction) -> Fut<'_, bool> {
        self.fallback.accepted_inventory_replay(transaction)
    }
}
