// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Host selection: a host's presence report, ranking the candidates, offering the session to the best one and its
//! answer.

use rifts_seam::{AccountId, UnixMillis};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::host_policy::{artifact_hashes_match, host_ineligibility_reason, score_host};
use crate::hub::{RealmCapacity, RealmCapacityInput, decide_realm_capacity, realm_host_ineligibility_reason};
use crate::hunt::{
    ArtifactHashes, GuardRole as StoredGuardRole, HostOfferRecord, HostPresenceRecord, HuntSession, HuntState,
    OfferStatus, ReliabilityIncrements, ScoreBreakdown, TransitionActor,
};
use crate::ids::random_uuid;
use crate::leases::{Hosting, as_account, integer_in, number_in, refused, short_text};
use crate::lifecycle::{
    GuardRole, parse_guard_presence_sessions, parse_guard_required_roles, queue, reset_to_selection, transition,
};
use crate::ports::HealthyGuard;
use crate::time::{Later as _, iso, parse_iso};

use HuntState as H;

const PRESENCE_WINDOW_MS: i64 = 30_000;
const STEAM_ID_BASE: u64 = 76_561_197_960_265_728;
const SELECTION_ACTOR: rifts_seam::Actor = rifts_seam::Actor::System("realms-selection");

/// The transport's peer id for an account, in decimal; admission's tickets use the same derivation.
pub fn derive_ice_peer_id(account: &str) -> String {
    rifts_admission::derive_ice_peer_id(&AccountId(account.to_owned()))
        .0
        .to_string()
}

/// 1 to 20 ASCII digits, no leading `0`, fitting a u64.
pub fn is_ice_peer_report(text: &str) -> bool {
    (1..=20).contains(&text.len())
        && text.bytes().all(|b| b.is_ascii_digit())
        && !text.starts_with('0')
        && text.parse::<u64>().is_ok()
}

/// A 17-digit SteamID64 of an individual account.
pub fn is_steam_transport_id(text: &str) -> bool {
    text.len() == 17
        && text.bytes().all(|b| b.is_ascii_digit())
        && text
            .parse::<u64>()
            .is_ok_and(|id| (STEAM_ID_BASE..=STEAM_ID_BASE + 0xFFFF_FFFF).contains(&id))
}

fn artifact_field(object: &Map<String, Value>, names: &[&str]) -> String {
    let value = names.iter().find_map(|name| object.get(*name).filter(|v| !v.is_null()));
    short_text(value, 64).unwrap_or("").to_owned()
}

/// All six must be 64 hex characters.
pub fn parse_artifacts(value: Option<&Value>) -> Option<ArtifactHashes> {
    let object = value?.as_object()?;
    let hashes = ArtifactHashes {
        game_exe: artifact_field(object, &["gameExe"]),
        internal_server: artifact_field(object, &["internalServer"]),
        winmm_proxy: artifact_field(object, &["winmmProxy"]),
        steam_bridge: artifact_field(object, &["transportModule", "steamBridge"]),
        steam_runtime: artifact_field(object, &["transportRuntime", "steamRuntime"]),
        admission_public_key: artifact_field(object, &["admissionPublicKey"]),
    };
    let all = [
        &hashes.game_exe,
        &hashes.internal_server,
        &hashes.winmm_proxy,
        &hashes.steam_bridge,
        &hashes.steam_runtime,
        &hashes.admission_public_key,
    ];
    all.iter()
        .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(hashes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReply {
    pub accepted: bool,
    pub revision: i64,
    pub expires_at: String,
    pub artifacts_verified: bool,
    pub guard_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferReply {
    pub accepted: bool,
    pub offer_id: String,
    pub session_id: String,
    pub generation: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RankedHost {
    pub account_id: String,
    pub steam_id: String,
    pub score: f64,
    pub breakdown: ScoreBreakdown,
}

fn stored_role(guard: &HealthyGuard) -> StoredGuardRole {
    StoredGuardRole {
        role: guard.role.as_str().to_owned(),
        guard_session_id: guard.guard_session_id.clone(),
        manifest_sequence: guard.manifest_sequence,
        sequence: guard.last_sequence,
        risk_score: guard.risk_score,
    }
}

impl Hosting {
    /// Node's `SubmitHostPresence`.
    pub async fn submit_presence(
        &self,
        account: &str,
        body: &Map<String, Value>,
    ) -> Result<PresenceReply, RealmsError> {
        let malformed = || refused("P2P_TICKET_INVALID", "The host presence report is malformed or stale.");
        let flag = |name: &str| body.get(name).and_then(Value::as_bool);
        let ready = flag("transportReady")
            .or_else(|| flag("steamRuntimeReady"))
            .ok_or_else(malformed)?;
        let reported = body
            .get("peerId")
            .filter(|v| !v.is_null())
            .or_else(|| body.get("steamId").filter(|v| !v.is_null()));
        let artifacts = parse_artifacts(body.get("artifacts")).ok_or_else(malformed)?;
        let observed_at = short_text(body.get("observedAt"), 40)
            .and_then(parse_iso)
            .ok_or_else(malformed)?
            .0;
        let (Some(consent), Some(power_saver), Some(runtime_supported)) =
            (flag("consentToHost"), flag("powerSaver"), flag("runtimeSupported"))
        else {
            return Err(malformed());
        };
        let now = self.now();
        let preflight = match body.get("preflightOnly") {
            None | Some(Value::Null) => false,
            Some(value) => value.as_bool().ok_or_else(malformed)?,
        };
        if (observed_at - now.0).abs() > PRESENCE_WINDOW_MS {
            return Err(malformed());
        }
        let bounds = || {
            refused(
                "P2P_TICKET_INVALID",
                "The host presence metrics are outside their bounds.",
            )
        };
        let free = integer_in(body.get("freePhysicalMemoryMiB"), 0, 1_048_576).ok_or_else(bounds)?;
        let commit = integer_in(body.get("commitHeadroomMiB"), 0, 1_048_576).ok_or_else(bounds)?;
        let cpus = integer_in(body.get("logicalCpuCount"), 1, 1024).ok_or_else(bounds)?;
        let load = number_in(body.get("cpuLoadPercent"), 0.0, 100.0).ok_or_else(bounds)?;
        let backend_rtt = number_in(body.get("backendRttMs"), 0.0, 60_000.0).ok_or_else(bounds)?;
        let worst_rtt = number_in(body.get("worstPeerRttMs"), 0.0, 60_000.0).ok_or_else(bounds)?;
        let jitter = number_in(body.get("jitterMs"), 0.0, 60_000.0).ok_or_else(bounds)?;
        let loss = number_in(body.get("packetLossPercent"), 0.0, 100.0).ok_or_else(bounds)?;
        let eligibility = &self.settings().eligibility;
        let expires_at = observed_at.saturating_add(eligibility.presence_maximum_age_ms);
        let stored = self
            .hunts
            .list_host_presences(&[account.to_owned()], UnixMillis(0), UnixMillis(i64::MIN))
            .await?
            .into_iter()
            .next();
        let stored_revision = stored.as_ref().map_or(0, |p| p.revision);
        let artifacts_verified = match (self.parts.expected_artifacts)() {
            Ok(expected) => artifact_hashes_match(&artifacts, &expected),
            Err(error) => {
                self.parts.log.warn(&format!(
                    "[P2PArtifacts] expected host artifact hashes unavailable ({error}); hosts stay unverified"
                ));
                false
            }
        };
        let ice = self.parts.operations.ice_transport();
        let transport_malformed = || refused("P2P_TICKET_INVALID", "The co-op transport observation is malformed.");
        let steam_id = match (ready, reported.and_then(Value::as_str)) {
            (true, Some(peer)) if ice && is_ice_peer_report(peer) => Some(derive_ice_peer_id(account)),
            (true, Some(peer)) if !ice && is_steam_transport_id(peer) => Some(peer.to_owned()),
            (true, _) => return Err(transport_malformed()),
            (false, _) if reported.is_some() => return Err(transport_malformed()),
            (false, _) => None,
        };
        let sessions =
            parse_guard_presence_sessions(&Value::Object(body.clone())).map_err(|e| refused(e.code, e.message))?;
        let realm_sessions = integer_in(body.get("realmHostSessions"), 0, 8).unwrap_or(0);
        let realm_host_only = realm_sessions > 0 && self.hosts_running_realm(account).await;
        let required = parse_guard_required_roles(&Value::Object(body.clone()), &sessions, realm_host_only)
            .map_err(|e| refused(e.code, e.message))?;
        let mut healthy: Vec<HealthyGuard> = Vec::new();
        for (role, id) in &sessions {
            if let Some(found) = self
                .parts
                .guard
                .find_healthy(account.to_owned(), id.clone(), *role)
                .await
                .map_err(RealmsError::Database)?
            {
                healthy.push(found);
            }
        }
        let guard_verified = !required.is_empty() && required.iter().all(|r| healthy.iter().any(|h| h.role == *r));
        if !required.is_empty() && !guard_verified {
            if self.parts.operations.guard_refuses() {
                return Err(refused(
                    "GUARD_REQUIRED",
                    "A current healthy Launcher Guard session is required to host.",
                ));
            }
            self.parts
                .log
                .info(&format!("[GuardObserve] would refuse hosting for {account}"));
        }
        let reply = |revision| PresenceReply {
            accepted: true,
            revision,
            expires_at: iso(UnixMillis(expires_at)),
            artifacts_verified,
            guard_verified,
        };
        if preflight && artifacts_verified {
            return Ok(reply(stored_revision));
        }
        let revision = stored_revision + 1;
        let primary = healthy
            .iter()
            .find(|h| h.role == GuardRole::Client)
            .or_else(|| healthy.iter().find(|h| h.role == GuardRole::Host));
        let launcher_version = short_text(body.get("launcherVersion"), 64)
            .unwrap_or("unknown")
            .to_owned();
        let presence = HostPresenceRecord {
            account_id: account.to_owned(),
            revision,
            consent_to_host: consent,
            observed_at: UnixMillis(observed_at),
            expires_at: UnixMillis(expires_at),
            free_physical_memory_mib: free as f64,
            commit_headroom_mib: commit as f64,
            logical_cpu_count: cpus as f64,
            cpu_load_percent: load,
            power_saver,
            runtime_supported,
            steam_runtime_ready: ready,
            steam_id: steam_id.clone(),
            backend_rtt_ms: backend_rtt,
            worst_peer_rtt_ms: worst_rtt,
            jitter_ms: jitter,
            packet_loss_percent: loss,
            artifacts,
            artifacts_verified,
            launcher_version,
            guard_session_id: primary.map(|g| g.guard_session_id.clone()),
            guard_risk_score: primary.map(|g| g.risk_score),
            guard_verified: Some(guard_verified),
            guard_roles: healthy.iter().map(stored_role).collect(),
            realm_host_sessions: Some(realm_sessions),
        };
        self.hunts.upsert_host_presence(&as_account(account), &presence).await?;
        if ready && let Some(peer) = &steam_id {
            let method = if ice { "ICE_ACCOUNT_DERIVED" } else { "HOST_PRESENCE" };
            self.ensure_verified_link(account, peer, u64::try_from(revision.max(1)).unwrap_or(1), method);
        }
        Ok(reply(revision))
    }

    /// Realm hosting is on, the account hosts a realm, and a server of it runs.
    async fn hosts_running_realm(&self, account: &str) -> bool {
        if !self.settings().realm_hosting {
            return false;
        }
        let check = async {
            let Some(realm) = self.realms.find_by_host(account).await? else {
                return Ok::<bool, RealmsError>(false);
            };
            Ok(self.hunts.count_running_realm_servers(&realm.realm_id).await? > 0)
        };
        match check.await {
            Ok(running) => running,
            Err(error) => {
                self.parts.log.warn(&format!(
                    "[P2P] realm host check for {account} failed; the client role stays required: {error}"
                ));
                false
            }
        }
    }

    async fn fresh_presences(&self, accounts: &[String]) -> Result<Vec<HostPresenceRecord>, RealmsError> {
        let now = self.now();
        let age = self.settings().eligibility.presence_maximum_age_ms;
        Ok(self
            .hunts
            .list_host_presences(accounts, UnixMillis(now.0 - age), now)
            .await?)
    }

    /// Node's `RankHosts`: lowest score first, then account id.
    pub async fn rank_hosts(&self, hunt: &HuntSession) -> Result<Vec<RankedHost>, RealmsError> {
        let accounts: Vec<String> = hunt.roster.iter().map(|p| p.account_id.clone()).collect();
        let presences = self.fresh_presences(&accounts).await?;
        let policy = &self.settings().eligibility;
        let guard_required = self.parts.operations.guard_refuses();
        let now = self.now();
        let mut ranked = Vec::new();
        for participant in &hunt.roster {
            if hunt.tried_host_account_ids.contains(&participant.account_id) {
                continue;
            }
            let Some(presence) = presences.iter().find(|p| p.account_id == participant.account_id) else {
                continue;
            };
            let reliability = self.hunts.find_host_reliability(&participant.account_id).await?;
            let active = self.hunts.count_active_host_sessions(&participant.account_id).await?;
            if let Some(reason) =
                host_ineligibility_reason(presence, reliability.as_ref(), active, policy, guard_required, now)
            {
                self.parts.log.info(&format!(
                    "[P2P] host candidate {} ineligible reason={reason}",
                    participant.account_id
                ));
                continue;
            }
            let (score, breakdown) = score_host(presence, reliability.as_ref(), policy, active);
            ranked.push(RankedHost {
                account_id: participant.account_id.clone(),
                steam_id: participant.steam_id.clone(),
                score,
                breakdown,
            });
        }
        ranked.sort_by(|a, b| {
            a.score
                .total_cmp(&b.score)
                .then_with(|| a.account_id.cmp(&b.account_id))
        });
        Ok(ranked)
    }

    /// Node's `RankRealmHost`: the realm's host is the only candidate; `Err(())` is REALM_FULL.
    #[allow(clippy::result_unit_err)]
    pub async fn rank_realm_host(&self, hunt: &HuntSession) -> Result<Result<Vec<RankedHost>, ()>, RealmsError> {
        let Some(realm_id) = &hunt.realm_id else {
            return Ok(Ok(Vec::new()));
        };
        let Some(realm) = self.realms.find_by_id(realm_id).await? else {
            return Ok(Ok(Vec::new()));
        };
        let host = realm.host_account_id.clone();
        if hunt.tried_host_account_ids.contains(&host) {
            return Ok(Ok(Vec::new()));
        }
        let Some(presence) = self
            .fresh_presences(std::slice::from_ref(&host))
            .await?
            .into_iter()
            .next()
        else {
            self.parts.log.info(&format!(
                "[P2P] realm host {host} for realm {realm_id} has no fresh presence"
            ));
            return Ok(Ok(Vec::new()));
        };
        let settings = self.settings();
        let guard_required = self.parts.operations.guard_refuses();
        let now = self.now();
        if let Some(reason) =
            realm_host_ineligibility_reason(&presence, &settings.eligibility, 0.0, 0.0, guard_required, now)
        {
            self.parts.log.info(&format!(
                "[P2P] realm host {host} for realm {realm_id} ineligible reason={reason}"
            ));
            return Ok(Ok(Vec::new()));
        }
        // This hunt already counts.
        let active = (self.hunts.count_active_realm_servers(realm_id).await? - 1).max(0);
        let max_servers = realm
            .hosting
            .as_ref()
            .map_or(settings.realm_hub.default_max_servers, |h| h.max_servers);
        let capacity = decide_realm_capacity(&RealmCapacityInput {
            active_servers: active,
            max_servers,
            free_physical_memory_mib: presence.free_physical_memory_mib,
            commit_headroom_mib: presence.commit_headroom_mib,
            minimum_free_memory_mib: settings.realm_hub.minimum_free_memory_mib,
            minimum_commit_headroom_mib: settings.realm_hub.minimum_commit_headroom_mib,
        });
        match capacity {
            RealmCapacity::ServerLimit | RealmCapacity::CommitLow => return Ok(Err(())),
            RealmCapacity::OkLowFreeMemory => self.parts.log.warn(&format!(
                "[RealmCapacity] realm {realm_id} hunt {} starts on a host with {} MiB free physical memory",
                hunt.session_id, presence.free_physical_memory_mib
            )),
            RealmCapacity::Ok => {}
        }
        let (score, breakdown) = score_host(&presence, None, &settings.eligibility, 0);
        Ok(Ok(vec![RankedHost {
            steam_id: derive_ice_peer_id(&host),
            account_id: host,
            score,
            breakdown,
        }]))
    }

    /// Node's `BeginHostSelection`. A disabled policy starts no selection, but recorded hunts still end.
    pub async fn begin_host_selection(&self, session_id: &str) -> Result<(), RealmsError> {
        let Some(hunt) = self.hunts.find_hunt(session_id).await? else {
            return Ok(());
        };
        if hunt.state != H::HostSelecting {
            return Ok(());
        }
        let recovery = &self.parts.recovery;
        let prepare =
            |reason: &str| recovery.prepare(hunt.session_id.clone(), hunt.generation, reason.to_owned(), true);
        if self.emergency_stop().await? {
            return prepare("OPERATOR_P2P_STOP").await;
        }
        if !self.settings().policy.enabled {
            return prepare("P2P_DISABLED").await;
        }
        let (ranked, realm_full) = if hunt.realm_id.is_some() {
            match self.rank_realm_host(&hunt).await? {
                Ok(ranked) => (ranked, false),
                Err(()) => (Vec::new(), true),
            }
        } else {
            (self.rank_hosts(&hunt).await?, false)
        };
        let Some(candidate) = ranked.into_iter().next() else {
            return prepare(if realm_full { "REALM_FULL" } else { "NO_ELIGIBLE_HOST" }).await;
        };
        let now = self.now();
        let offer_id = random_uuid();
        let offer_expires = now.saturating_add(self.settings().eligibility.offer_lifetime_ms);
        let estimated = self.settings().eligibility.minimum_commit_headroom_mib;
        let (id, chosen, offer) = (hunt.session_id.clone(), candidate.clone(), offer_id.clone());
        let offered = self
            .hunts
            .transaction(
                &SELECTION_ACTOR,
                move |tx| -> Result<Option<HuntSession>, RealmsError> {
                    let Some(current) = tx.find_hunt(&id)? else {
                        return Ok(None);
                    };
                    if current.state != H::HostSelecting {
                        return Ok(None);
                    }
                    let offered = tx.compare_and_set(
                        &id,
                        current.generation,
                        &[H::HostSelecting],
                        current.state_version,
                        |h| {
                            h.state = H::HostOffered;
                            h.host_account_id = chosen.account_id.clone();
                            h.host_steam_id = chosen.steam_id.clone();
                            h.host_offer_id = Some(offer.clone());
                            h.offer_expires_at = Some(offer_expires);
                            if !h.tried_host_account_ids.contains(&chosen.account_id) {
                                h.tried_host_account_ids.push(chosen.account_id.clone());
                            }
                        },
                    )?;
                    let Some(offered) = offered else {
                        return Ok(None);
                    };
                    transition(
                        tx,
                        H::HostSelecting,
                        &offered,
                        "HOST_CANDIDATE_SELECTED",
                        TransitionActor::System,
                        None,
                        now,
                    )?;
                    tx.create_offer(&HostOfferRecord {
                        offer_id: offer.clone(),
                        session_id: offered.session_id.clone(),
                        generation: offered.generation,
                        account_id: chosen.account_id.clone(),
                        steam_id: chosen.steam_id.clone(),
                        rank: 1,
                        score: chosen.score,
                        score_breakdown: chosen.breakdown.clone(),
                        status: OfferStatus::Offered,
                        offered_at: now,
                        expires_at: offer_expires,
                        responded_at: None,
                        response_reason: None,
                    })?;
                    let payload = json!({
                        "offerId": offer,
                        "huntDefinitionId": offered.hunt_definition_id,
                        "partySize": offered.roster.len(),
                        "expiresAt": iso(offer_expires),
                        "estimatedMemoryMiB": estimated,
                        "score": chosen.score,
                    });
                    queue(
                        tx,
                        &chosen.account_id,
                        "HOST_OFFERED",
                        &offered,
                        payload,
                        now,
                        offer_expires.0 - now.0,
                    )?;
                    Ok(Some(offered))
                },
            )
            .await?;
        if offered.is_some() {
            self.parts.log.info(&format!(
                "[P2P] host offer {offer_id} session={} account={} score={}",
                hunt.session_id, candidate.account_id, candidate.score
            ));
        }
        Ok(())
    }

    /// Node's `RespondToHostOffer`.
    pub async fn respond_to_offer(
        &self,
        account: &str,
        offer_id: &str,
        body: &Map<String, Value>,
    ) -> Result<OfferReply, RealmsError> {
        let shaped = offer_id.len() == 36 && offer_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        let accept = body.get("accept").and_then(Value::as_bool);
        let (true, Some(accept)) = (shaped, accept) else {
            return Err(refused("P2P_TICKET_INVALID", "The host offer response is malformed."));
        };
        let offer = self
            .hunts
            .find_host_offer(offer_id)
            .await?
            .filter(|o| o.account_id == account)
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Host offer not found."))?;
        let reply = OfferReply {
            accepted: accept,
            offer_id: offer.offer_id.clone(),
            session_id: offer.session_id.clone(),
            generation: offer.generation,
        };
        // A lost ACK is retried.
        if accept && offer.status == OfferStatus::Accepted {
            return Ok(reply);
        }
        let now = self.now();
        if offer.status != OfferStatus::Offered || offer.expires_at <= now {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The host offer has expired or was replaced.",
            ));
        }
        let recovery = &self.parts.recovery;
        let prepare =
            |reason: &str| recovery.prepare(offer.session_id.clone(), offer.generation, reason.to_owned(), true);
        if accept && !self.settings().policy.enabled {
            prepare("P2P_DISABLED").await?;
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        if self.emergency_stop().await? {
            prepare("OPERATOR_P2P_STOP").await?;
            return Err(refused(
                "P2P_DISABLED",
                "Player-hosted hunts are stopped by operations policy.",
            ));
        }
        let reason = short_text(body.get("reason"), 96).map(str::to_owned);
        let startup = self.settings().eligibility.startup_lifetime_ms;
        let (who, offer_owned) = (account.to_owned(), offer.clone());
        self.hunts
            .transaction(&as_account(account), move |tx| -> Result<(), RealmsError> {
                let to = if accept {
                    OfferStatus::Accepted
                } else {
                    OfferStatus::Declined
                };
                tx.update_offer(
                    &offer_owned.offer_id,
                    &who,
                    OfferStatus::Offered,
                    to,
                    now,
                    reason.as_deref(),
                )?
                .ok_or_else(|| refused("P2P_GENERATION_FENCED", "The host offer was already answered."))?;
                let session = tx
                    .find_hunt(&offer_owned.session_id)?
                    .filter(|s| {
                        s.generation == offer_owned.generation
                            && s.host_offer_id.as_deref() == Some(offer_owned.offer_id.as_str())
                            && s.state == H::HostOffered
                    })
                    .ok_or_else(|| {
                        refused(
                            "P2P_GENERATION_FENCED",
                            "The host offer no longer owns this generation.",
                        )
                    })?;
                if accept {
                    let add = ReliabilityIncrements {
                        offers_accepted: 1,
                        ..ReliabilityIncrements::default()
                    };
                    tx.increment_reliability(&who, add, now, None)?;
                    let payload = json!({
                        "sessionId": session.session_id,
                        "generation": session.generation,
                        "role": "HOST",
                    });
                    queue(tx, &who, "HUNT_BOOTSTRAP", &session, payload, now, startup)?;
                } else {
                    let add = ReliabilityIncrements {
                        offers_declined: 1,
                        ..ReliabilityIncrements::default()
                    };
                    tx.increment_reliability(&who, add, now, None)?;
                    let reset = reset_to_selection(tx, &session)?
                        .ok_or_else(|| refused("P2P_GENERATION_FENCED", "The host generation changed."))?;
                    let why = reason.as_deref().unwrap_or("HOST_OFFER_DECLINED");
                    transition(tx, H::HostOffered, &reset, why, TransitionActor::Host, Some(&who), now)?;
                }
                Ok(())
            })
            .await?;
        if !accept {
            self.begin_host_selection(&offer.session_id).await?;
        }
        Ok(reply)
    }
}
