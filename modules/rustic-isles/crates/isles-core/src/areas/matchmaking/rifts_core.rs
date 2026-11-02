//! `rifts_seam::IslesCore`: what rustic-rifts calls back into isles (RF1, section 2.2). The parts of it that are
//! matchmaking's are answered here: parties, publishing a travel candidate, ending pending travel, whether dedicated
//! servers exist and the game session touch. The rest (accounts, Launcher Guard, operations) belongs to other areas,
//! and is answered by `fallback` until the integration pass hands over theirs.
//!
//! Accounts cross as their `userId` text, the way the realms side holds them (`rifts_seam::AccountId` is "the
//! backend's account id text", DD17 not applying to it yet) and the way every in-memory table here is keyed.

use std::sync::Arc;

use module_api::{BoxFuture, GlobalId, NodeId};
pub use rifts_seam::IslesCore;
use rifts_seam::{AccountId, Candidate, GuardMode, Json, Party, PartyId, SeamError, SessionId};

use super::party::{Parties, Party as IslesParty};
use super::state::{MatchmakingResult, MatchmakingState};
use crate::db::IslesStore;

fn refused(code: &str, message: impl Into<String>) -> SeamError {
    SeamError::Refused(rifts_seam::Refusal {
        code: code.to_owned(),
        message: message.into(),
    })
}

/// A rifts party id (`<ULID>@<node>`) as isles keys parties (`party:<ULID>@<node>`).
fn isles_party_id(party: &PartyId) -> Option<GlobalId> {
    format!("party:{party}").parse().ok()
}

fn seam_party(party: &IslesParty) -> Result<Party, SeamError> {
    let id = format!("{}@{}", party.global_id.ulid(), party.global_id.owner())
        .parse()
        .map_err(|_| refused("INVALID_PARTY", "a party id is not a ULID and a node"))?;
    Ok(Party {
        id: PartyId(id),
        leader: AccountId(party.leader.clone()),
        members: party.members.iter().cloned().map(AccountId).collect(),
        revision: u64::try_from(party.revision).unwrap_or(0),
    })
}

pub struct MatchmakingCore {
    pub store: Option<IslesStore>,
    pub node: NodeId,
    pub state: Arc<MatchmakingState>,
    /// Accounts, Launcher Guard and operations, the other areas'
    pub fallback: Arc<dyn IslesCore>,
}

impl MatchmakingCore {
    fn parties(&self) -> Parties<'_> {
        Parties {
            store: self.store.as_ref(),
            node: &self.node,
            clock: &self.state.clock,
            rifts: None,
            request_id: None,
        }
    }
}

impl IslesCore for MatchmakingCore {
    fn display_name(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        self.fallback.display_name(account)
    }

    fn find_by_display_name(&self, name: String) -> BoxFuture<'_, Result<Option<AccountId>, SeamError>> {
        self.fallback.find_by_display_name(name)
    }

    fn launcher_account_eligible(&self, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        self.fallback.launcher_account_eligible(account)
    }

    fn party(&self, party: PartyId) -> BoxFuture<'_, Result<Option<Party>, SeamError>> {
        Box::pin(async move {
            let Some(id) = isles_party_id(&party) else {
                return Ok(None);
            };
            let Some(store) = &self.store else {
                return Err(refused("NO_DATABASE", "no database is configured"));
            };
            let record = store
                .read(move |conn, repos| repos.parties.find_by_id(conn, &id))
                .await
                .map_err(|e| refused("PARTY_READ_FAILED", e.to_string()))?;
            record.map(|record| seam_party(&record.into())).transpose()
        })
    }

    fn get_or_create_party(&self, account: AccountId) -> BoxFuture<'_, Result<Party, SeamError>> {
        Box::pin(async move {
            // The realms side doesn't know the client's build id; the party keeps it blank until a client names one
            let party = self
                .parties()
                .get_or_create(&account.0, "")
                .await
                .map_err(|e| refused("PARTY_WRITE_FAILED", e.to_string()))?;
            seam_party(&party)
        })
    }

    fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError> {
        let value: serde_json::Value = serde_json::from_str(&candidate.0.0)
            .map_err(|e| refused("INVALID_CANDIDATE", format!("a candidate is a JSON object: {e}")))?;
        let result = MatchmakingResult::from_json(&value).ok_or_else(|| {
            refused(
                "INVALID_CANDIDATE",
                "a candidate has HuntId, CandidateId and GameSessionId",
            )
        })?;
        self.state.set_candidate(&account.0, result);
        Ok(())
    }

    fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError> {
        let players: Vec<String> = accounts.iter().map(|a| a.0.clone()).collect();
        Ok(self.state.cancel_hunt(hunt_id, &players, reason))
    }

    fn dedicated_servers_configured(&self) -> bool {
        self.state.deploy.get().configured()
    }

    fn touch_game_session(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async move {
            self.state.presence.touch(&account.0, self.state.clock.now_ms());
            Ok(())
        })
    }

    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> BoxFuture<'_, Result<(), SeamError>> {
        self.fallback.record_hunt_activity(session, activity)
    }

    fn healthy_guard_session(&self, account: AccountId) -> BoxFuture<'_, Result<Option<Json>, SeamError>> {
        self.fallback.healthy_guard_session(account)
    }

    fn published_guard_artifacts(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
        self.fallback.published_guard_artifacts()
    }

    fn guard_mode(&self) -> GuardMode {
        self.fallback.guard_mode()
    }

    fn operations_policy(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
        self.fallback.operations_policy()
    }

    fn verify_operations_step_up(&self, key_id: String, step_up: String) -> BoxFuture<'_, Result<bool, SeamError>> {
        self.fallback.verify_operations_step_up(key_id, step_up)
    }
}
