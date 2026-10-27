//! Host selection (`SubmitHostPresence`, `RankHosts`, `RankRealmHost`, `BeginHostSelection`, `RespondToHostOffer`):
//! PCs report that they can host, a forming hunt offers itself to the best-ranked one, and the offer is accepted,
//! declined or lets time run out. The transport link a verified presence writes (`p2pSteamLinks`) is rifts-admission's.

use rifts_seam::{AccountId, Actor, UnixMillis};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::RealmsError;
use crate::host_policy::{artifact_hashes_match, host_ineligibility_reason, score_host};
use crate::hub::{RealmCapacity, RealmCapacityInput, decide_realm_capacity, realm_host_ineligibility_reason};
use crate::hunt::{
    ArtifactHashes, GuardRole as StoredGuardRole, HostOfferRecord, HostPresenceRecord, HuntSession, HuntState,
    HuntTransition, OfferStatus, ReliabilityIncrements, ScoreBreakdown, TransitionActor,
};
use crate::ids::random_uuid;
use crate::leases::{Hosting, integer_in, number_in, refused};
use crate::lifecycle::{
    GuardRole, parse_guard_presence_sessions, parse_guard_required_roles, reset_to_selection, string_in,
};
use crate::repo::hunts::NewControlMessage;
use crate::time::iso;

const HOST_PRESENCE_MAX_BODY_AGE_MS: i64 = 30_000;

/// `DeriveIcePeerId`: ICE has no platform id, so the transport's 64-bit peer id comes from the account (the same
/// derivation rifts-admission's tickets use).
pub fn derive_ice_peer_id(account: &str) -> String {
    let digest = Sha256::digest(format!("mysticparadox-ice-peer-v1\n{account}").as_bytes());
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[..8]);
    let value = u64::from_be_bytes(head) & 0x7fff_ffff_ffff_ffff;
    (if value == 0 { 1 } else { value }).to_string()
}

/// `ParseIcePeerReport`: 1 to 20 digits within u64, no leading zero.
fn parse_ice_peer_report(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    let shaped = (1..=20).contains(&text.len()) && !text.starts_with('0') && text.bytes().all(|b| b.is_ascii_digit());
    (shaped && text.parse::<u64>().is_ok()).then(|| text.to_owned())
}

/// `ParseSteamTransportId`: a SteamID64 of an individual account.
fn parse_steam_transport_id(value: &Value) -> Option<String> {
    const MIN: u64 = 76_561_197_960_265_728;
    let text = value.as_str()?;
    if text.len() != 17 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = text.parse().ok()?;
    (MIN..=MIN + 0xffff_ffff).contains(&n).then(|| text.to_owned())
}

/// `ParseArtifacts`: launchers since plan step 4.2 send the neutral co-op names, older ones the transport's own.
pub fn parse_artifacts(value: Option<&Value>) -> Option<ArtifactHashes> {
    let input = value?.as_object()?;
    let field = |names: &[&str]| -> String {
        names
            .iter()
            .find_map(|name| input.get(*name).filter(|v| !v.is_null()))
            .and_then(|v| string_in(Some(v), 64))
            .unwrap_or_default()
    };
    let hashes = ArtifactHashes {
        game_exe: field(&["gameExe"]),
        internal_server: field(&["internalServer"]),
        winmm_proxy: field(&["winmmProxy"]),
        steam_bridge: field(&["transportModule", "steamBridge"]),
        steam_runtime: field(&["transportRuntime", "steamRuntime"]),
        admission_public_key: field(&["admissionPublicKey"]),
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

#[derive(Debug, Clone, PartialEq, Serialize)]
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

/// A host that may take the session, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedHost {
    pub account_id: String,
    pub steam_id: String,
    pub score: f64,
    pub breakdown: ScoreBreakdown,
}

fn ticket_invalid(message: &str) -> RealmsError {
    refused("P2P_TICKET_INVALID", message)
}

fn player(account: &str) -> Actor {
    Actor::Account(AccountId(account.to_owned()))
}

const SYSTEM: Actor = Actor::System("realms-selection");

impl Hosting {
    /// Whether the account hosts a realm with a live server now (the server's record, never the launcher's report).
    async fn confirms_realm_host(&self, account: &str) -> bool {
        if !self.settings.realm_hosting {
            return false;
        }
        match self.realms.find_by_host(account).await {
            Ok(Some(realm)) => {
                self.hunts
                    .count_running_realm_servers(&realm.realm_id)
                    .await
                    .unwrap_or(0)
                    > 0
            }
            Ok(None) => false,
            Err(error) => {
                self.log.warn(&format!(
                    "[P2P] realm host check for {account} failed; the client role stays required: {error}"
                ));
                false
            }
        }
    }

    /// `SubmitHostPresence`: a PC's capacity and integrity report. In observe mode a Guard that does not verify
    /// refuses nothing.
    pub async fn submit_presence(&self, account: &str, body: &Value) -> Result<PresenceReply, RealmsError> {
        let transport_ready = body
            .get("transportReady")
            .or_else(|| body.get("steamRuntimeReady"))
            .and_then(Value::as_bool);
        let peer = body
            .get("peerId")
            .filter(|v| !v.is_null())
            .or_else(|| body.get("steamId"))
            .filter(|v| !v.is_null());
        let artifacts = parse_artifacts(body.get("artifacts"));
        let observed = string_in(body.get("observedAt"), 40)
            .as_deref()
            .and_then(crate::time::parse_iso);
        let now = self.now();
        let flag = |key: &str| body.get(key).and_then(Value::as_bool);
        let preflight = body.get("preflightOnly").filter(|v| !v.is_null());
        let (
            Some(artifacts),
            Some(observed),
            Some(consent),
            Some(power_saver),
            Some(runtime_supported),
            Some(transport_ready),
        ) = (
            artifacts,
            observed,
            flag("consentToHost"),
            flag("powerSaver"),
            flag("runtimeSupported"),
            transport_ready,
        )
        else {
            return Err(ticket_invalid("The host presence report is malformed or stale."));
        };
        if (now.0 - observed.0).abs() > HOST_PRESENCE_MAX_BODY_AGE_MS || preflight.is_some_and(|v| !v.is_boolean()) {
            return Err(ticket_invalid("The host presence report is malformed or stale."));
        }
        #[allow(clippy::cast_precision_loss)]
        let metrics = (
            integer_in(body.get("freePhysicalMemoryMiB"), 0.0, 1_048_576.0).map(|n| n as f64),
            integer_in(body.get("commitHeadroomMiB"), 0.0, 1_048_576.0).map(|n| n as f64),
            integer_in(body.get("logicalCpuCount"), 1.0, 1024.0).map(|n| n as f64),
            number_in(body.get("cpuLoadPercent"), 0.0, 100.0),
            number_in(body.get("backendRttMs"), 0.0, 60_000.0),
            number_in(body.get("worstPeerRttMs"), 0.0, 60_000.0),
            number_in(body.get("jitterMs"), 0.0, 60_000.0),
            number_in(body.get("packetLossPercent"), 0.0, 100.0),
        );
        let (Some(free), Some(commit), Some(cpus), Some(load), Some(backend), Some(worst), Some(jitter), Some(loss)) =
            metrics
        else {
            return Err(ticket_invalid("The host presence metrics are outside their bounds."));
        };
        let expires_at = UnixMillis(observed.0 + self.settings.eligibility.presence_maximum_age_ms);
        let current = self
            .hunts
            .list_host_presences(&[account.to_owned()], UnixMillis(0), UnixMillis(i64::MIN))
            .await?;
        let revision = current.first().map_or(0, |p| p.revision) + 1;
        let artifacts_verified = match (self.expected_artifacts)() {
            Ok(expected) => artifact_hashes_match(&artifacts, &expected),
            Err(error) => {
                self.log.warn(&format!(
                    "[P2PArtifacts] expected host artifact hashes unavailable ({error}); hosts stay unverified"
                ));
                false
            }
        };
        // With ICE the reported id is only the module instance; the link carries the account-derived peer id
        let ice = self.operations.ice_transport();
        let reported = peer.and_then(|p| {
            if ice {
                parse_ice_peer_report(p)
            } else {
                parse_steam_transport_id(p)
            }
        });
        let steam_id = reported.map(|id| if ice { derive_ice_peer_id(account) } else { id });
        if (transport_ready && steam_id.is_none()) || (!transport_ready && peer.is_some()) {
            return Err(ticket_invalid("The co-op transport observation is malformed."));
        }
        let to_refusal = |e: crate::lifecycle::GuardListError| refused(e.code, e.message);
        let sessions = parse_guard_presence_sessions(body).map_err(to_refusal)?;
        let realm_sessions = integer_in(body.get("realmHostSessions"), 0.0, 8.0).unwrap_or(0);
        let realm_host_only = realm_sessions > 0 && self.confirms_realm_host(account).await;
        let required = parse_guard_required_roles(body, &sessions, realm_host_only).map_err(to_refusal)?;
        let mut healthy = Vec::new();
        for (role, id) in &sessions {
            if let Some(guard) = self
                .guard
                .find_healthy(account.to_owned(), id.clone(), *role)
                .await
                .map_err(RealmsError::Database)?
            {
                healthy.push(guard);
            }
        }
        let guard_verified = !required.is_empty() && required.iter().all(|r| healthy.iter().any(|g| g.role == *r));
        if !required.is_empty() && !guard_verified {
            if self.operations.guard_refuses() {
                return Err(refused(
                    "GUARD_REQUIRED",
                    "A current healthy Launcher Guard session is required to host.",
                ));
            }
            self.log
                .info(&format!("[GuardObserve] would refuse hosting for {account}"));
        }
        if preflight.and_then(Value::as_bool) == Some(true) && artifacts_verified {
            return Ok(PresenceReply {
                accepted: true,
                revision: current.first().map_or(0, |p| p.revision),
                expires_at: iso(expires_at),
                artifacts_verified,
                guard_verified,
            });
        }
        let preferred = healthy
            .iter()
            .find(|g| g.role == GuardRole::Client)
            .or_else(|| healthy.iter().find(|g| g.role == GuardRole::Host));
        let role_name = |role: GuardRole| if role == GuardRole::Client { "client" } else { "host" };
        let record = HostPresenceRecord {
            account_id: account.to_owned(),
            revision,
            consent_to_host: consent,
            observed_at: observed,
            expires_at,
            free_physical_memory_mib: free,
            commit_headroom_mib: commit,
            logical_cpu_count: cpus,
            cpu_load_percent: load,
            power_saver,
            runtime_supported,
            steam_runtime_ready: transport_ready,
            steam_id,
            backend_rtt_ms: backend,
            worst_peer_rtt_ms: worst,
            jitter_ms: jitter,
            packet_loss_percent: loss,
            artifacts,
            artifacts_verified,
            launcher_version: string_in(body.get("launcherVersion"), 64).unwrap_or_else(|| "unknown".to_owned()),
            guard_session_id: preferred.map(|g| g.guard_session_id.clone()),
            guard_risk_score: preferred.map(|g| g.risk_score),
            guard_verified: Some(guard_verified),
            guard_roles: healthy
                .iter()
                .map(|g| StoredGuardRole {
                    role: role_name(g.role).to_owned(),
                    guard_session_id: g.guard_session_id.clone(),
                    manifest_sequence: g.manifest_sequence,
                    sequence: g.last_sequence,
                    risk_score: g.risk_score,
                })
                .collect(),
            realm_host_sessions: Some(realm_sessions),
        };
        self.hunts.upsert_host_presence(&player(account), &record).await?;
        Ok(PresenceReply {
            accepted: true,
            revision,
            expires_at: iso(expires_at),
            artifacts_verified,
            guard_verified,
        })
    }

    /// `RankHosts`: the roster's fresh, eligible presences, by score then account.
    pub async fn rank_hosts(&self, hunt: &HuntSession) -> Result<Vec<RankedHost>, RealmsError> {
        let policy = &self.settings.eligibility;
        let now = self.now();
        let guard_required = self.operations.guard_refuses();
        let accounts: Vec<String> = hunt.roster.iter().map(|p| p.account_id.clone()).collect();
        let presences = self
            .hunts
            .list_host_presences(&accounts, UnixMillis(now.0 - policy.presence_maximum_age_ms), now)
            .await?;
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
                self.log.info(&format!(
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

    /// `RankRealmHost`: a realm hunt's one candidate is its realm's host, within the realm's server and commit limits.
    /// `Err(())` is REALM_FULL.
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
        let now = self.now();
        let policy = &self.settings.eligibility;
        let realm_policy = &self.settings.realm_hub;
        let presences = self
            .hunts
            .list_host_presences(
                std::slice::from_ref(&host),
                UnixMillis(now.0 - policy.presence_maximum_age_ms),
                now,
            )
            .await?;
        let Some(presence) = presences.first() else {
            self.log.info(&format!(
                "[P2P] realm host {host} for realm {realm_id} has no fresh presence"
            ));
            return Ok(Ok(Vec::new()));
        };
        // The memory floors are the capacity's below: a short host is REALM_FULL, not ineligible
        if let Some(reason) =
            realm_host_ineligibility_reason(presence, policy, 0.0, 0.0, self.operations.guard_refuses(), now)
        {
            self.log.info(&format!(
                "[P2P] realm host {host} for realm {realm_id} ineligible reason={reason}"
            ));
            return Ok(Ok(Vec::new()));
        }
        // This hunt is one of the realm's live sessions already
        let others = (self.hunts.count_active_realm_servers(realm_id).await? - 1).max(0);
        let capacity = decide_realm_capacity(&RealmCapacityInput {
            active_servers: others,
            max_servers: realm
                .hosting
                .as_ref()
                .map_or(realm_policy.default_max_servers, |h| h.max_servers),
            free_physical_memory_mib: presence.free_physical_memory_mib,
            commit_headroom_mib: presence.commit_headroom_mib,
            minimum_free_memory_mib: realm_policy.minimum_free_memory_mib,
            minimum_commit_headroom_mib: realm_policy.minimum_commit_headroom_mib,
        });
        match capacity {
            RealmCapacity::ServerLimit | RealmCapacity::CommitLow => return Ok(Err(())),
            RealmCapacity::OkLowFreeMemory => self.log.warn(&format!(
                "[RealmCapacity] realm {realm_id} hunt {} starts on a host with {} MiB free physical memory",
                hunt.session_id, presence.free_physical_memory_mib
            )),
            RealmCapacity::Ok => {}
        }
        let (score, breakdown) = score_host(presence, None, policy, 0);
        Ok(Ok(vec![RankedHost {
            steam_id: derive_ice_peer_id(&host),
            account_id: host,
            score,
            breakdown,
        }]))
    }

    /// `BeginHostSelection`: offers the session to the best host, or hands it to recovery when none can take it.
    pub async fn begin_host_selection(&self, session_id: &str) -> Result<(), RealmsError> {
        let Some(hunt) = self.hunts.find_hunt(session_id).await? else {
            return Ok(());
        };
        if hunt.state != HuntState::HostSelecting {
            return Ok(());
        }
        if self.emergency_stop().await? {
            self.hand_to_recovery(&hunt, "OPERATOR_P2P_STOP", true).await?;
            return Ok(());
        }
        // The monitor still runs with routing off so recorded hunts can end, but starts no selection
        if !self.settings.policy.enabled {
            self.hand_to_recovery(&hunt, "P2P_DISABLED", true).await?;
            return Ok(());
        }
        let (ranked, refusal) = if hunt.realm_id.is_some() {
            match self.rank_realm_host(&hunt).await? {
                Ok(ranked) => (ranked, None),
                Err(()) => (Vec::new(), Some("REALM_FULL")),
            }
        } else {
            (self.rank_hosts(&hunt).await?, None)
        };
        let Some(candidate) = ranked.into_iter().next() else {
            self.hand_to_recovery(&hunt, refusal.unwrap_or("NO_ELIGIBLE_HOST"), true)
                .await?;
            return Ok(());
        };
        let now = self.now();
        let expires_at = UnixMillis(now.0 + self.settings.eligibility.offer_lifetime_ms);
        let memory = self.settings.eligibility.minimum_commit_headroom_mib;
        let offer_id = random_uuid();
        let (id, chosen, offer) = (session_id.to_owned(), candidate.clone(), offer_id.clone());
        let offered = self
            .hunts
            .transaction(&SYSTEM, move |tx| {
                let Some(current) = tx.find_hunt(&id)? else {
                    return Ok::<_, RealmsError>(None);
                };
                if current.state != HuntState::HostSelecting {
                    return Ok(None);
                }
                let (account, steam, offer_set) = (chosen.account_id.clone(), chosen.steam_id.clone(), offer.clone());
                let Some(next) = tx.compare_and_set(
                    &id,
                    current.generation,
                    &[HuntState::HostSelecting],
                    current.state_version,
                    move |h| {
                        h.state = HuntState::HostOffered;
                        h.host_account_id = account.clone();
                        h.host_steam_id = steam;
                        h.host_offer_id = Some(offer_set);
                        h.offer_expires_at = Some(expires_at);
                        if !h.tried_host_account_ids.contains(&account) {
                            h.tried_host_account_ids.push(account);
                        }
                    },
                )?
                else {
                    return Ok(None);
                };
                tx.append_transition(&HuntTransition {
                    transition_id: String::new(),
                    session_id: id.clone(),
                    generation: next.generation,
                    from_state: current.state,
                    to_state: next.state,
                    state_version: next.state_version,
                    reason: "HOST_CANDIDATE_SELECTED".to_owned(),
                    actor_type: TransitionActor::System.as_str().to_owned(),
                    actor_account_id: None,
                    at: now,
                })?;
                tx.create_offer(&HostOfferRecord {
                    offer_id: offer.clone(),
                    session_id: id.clone(),
                    generation: next.generation,
                    account_id: chosen.account_id.clone(),
                    steam_id: chosen.steam_id.clone(),
                    rank: 1,
                    score: chosen.score,
                    score_breakdown: chosen.breakdown,
                    status: OfferStatus::Offered,
                    offered_at: now,
                    expires_at,
                    responded_at: None,
                    response_reason: None,
                })?;
                tx.enqueue_control(&NewControlMessage {
                    account_id: chosen.account_id.clone(),
                    message_type: "HOST_OFFERED".to_owned(),
                    session_id: Some(id.clone()),
                    generation: Some(next.generation),
                    payload: json!({
                        "offerId": offer,
                        "huntDefinitionId": next.hunt_definition_id,
                        "partySize": next.roster.len(),
                        "expiresAt": iso(expires_at),
                        "estimatedMemoryMiB": memory,
                        "score": chosen.score,
                    }),
                    created_at: now,
                    expires_at,
                })?;
                Ok(Some(next))
            })
            .await?;
        if offered.is_some() {
            self.log.info(&format!(
                "[P2P] host offer {offer_id} session={session_id} account={} score={}",
                candidate.account_id, candidate.score
            ));
        }
        Ok(())
    }

    /// `RespondToHostOffer`: an accepted offer asks the host to start (HUNT_BOOTSTRAP); a declined one goes back to
    /// selection. The same account accepting twice is answered as success (a lost ACK is retried).
    pub async fn respond_to_offer(
        &self,
        account: &str,
        offer_id: &str,
        body: &Value,
    ) -> Result<OfferReply, RealmsError> {
        let shaped = offer_id.len() == 36 && offer_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        let Some(accept) = body.get("accept").and_then(Value::as_bool).filter(|_| shaped) else {
            return Err(ticket_invalid("The host offer response is malformed."));
        };
        let now = self.now();
        let offer = self
            .hunts
            .find_host_offer(offer_id)
            .await?
            .filter(|o| o.account_id == account)
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Host offer not found."))?;
        let reply = |accepted| OfferReply {
            accepted,
            offer_id: offer_id.to_owned(),
            session_id: offer.session_id.clone(),
            generation: offer.generation,
        };
        if accept && offer.status == OfferStatus::Accepted {
            return Ok(reply(true));
        }
        if offer.status != OfferStatus::Offered || offer.expires_at <= now {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The host offer has expired or was replaced.",
            ));
        }
        if accept && !self.settings.policy.enabled {
            self.recovery
                .prepare(
                    offer.session_id.clone(),
                    offer.generation,
                    "P2P_DISABLED".to_owned(),
                    true,
                )
                .await?;
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        if self.emergency_stop().await? {
            self.recovery
                .prepare(
                    offer.session_id.clone(),
                    offer.generation,
                    "OPERATOR_P2P_STOP".to_owned(),
                    true,
                )
                .await?;
            return Err(refused(
                "P2P_DISABLED",
                "Player-hosted hunts are stopped by operations policy.",
            ));
        }
        let reason = string_in(body.get("reason"), 96);
        let status = if accept {
            OfferStatus::Accepted
        } else {
            OfferStatus::Declined
        };
        let startup_ms = self.settings.eligibility.startup_lifetime_ms;
        let (offer_key, who, session, generation) = (
            offer_id.to_owned(),
            account.to_owned(),
            offer.session_id.clone(),
            offer.generation,
        );
        self.hunts
            .transaction(&player(account), move |tx| {
                let fenced = |m: &str| refused("P2P_GENERATION_FENCED", m);
                if tx
                    .update_offer(&offer_key, &who, OfferStatus::Offered, status, now, reason.as_deref())?
                    .is_none()
                {
                    return Err(fenced("The host offer was already answered."));
                }
                let hunt = tx
                    .find_hunt(&session)?
                    .filter(|h| {
                        h.generation == generation
                            && h.host_offer_id.as_deref() == Some(offer_key.as_str())
                            && h.state == HuntState::HostOffered
                    })
                    .ok_or_else(|| fenced("The host offer no longer owns this generation."))?;
                if accept {
                    let add = ReliabilityIncrements {
                        offers_accepted: 1,
                        ..ReliabilityIncrements::default()
                    };
                    tx.increment_reliability(&who, add, now, None)?;
                    tx.enqueue_control(&NewControlMessage {
                        account_id: who.clone(),
                        message_type: "HUNT_BOOTSTRAP".to_owned(),
                        session_id: Some(hunt.session_id.clone()),
                        generation: Some(hunt.generation),
                        payload: json!({ "sessionId": hunt.session_id, "generation": hunt.generation, "role": "HOST" }),
                        created_at: now,
                        expires_at: UnixMillis(now.0 + startup_ms),
                    })?;
                } else {
                    let add = ReliabilityIncrements {
                        offers_declined: 1,
                        ..ReliabilityIncrements::default()
                    };
                    tx.increment_reliability(&who, add, now, None)?;
                    let reset = reset_to_selection(tx, &hunt)?.ok_or_else(|| fenced("The host generation changed."))?;
                    tx.append_transition(&HuntTransition {
                        transition_id: String::new(),
                        session_id: hunt.session_id.clone(),
                        generation: reset.generation,
                        from_state: hunt.state,
                        to_state: reset.state,
                        state_version: reset.state_version,
                        reason: reason.clone().unwrap_or_else(|| "HOST_OFFER_DECLINED".to_owned()),
                        actor_type: TransitionActor::Host.as_str().to_owned(),
                        actor_account_id: Some(who.clone()),
                        at: now,
                    })?;
                }
                Ok(())
            })
            .await?;
        if !accept {
            self.begin_host_selection(&offer.session_id).await?;
        }
        Ok(reply(accept))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ice_peer_ids_and_reports() {
        let id = derive_ice_peer_id("account-1");
        assert_eq!(id, derive_ice_peer_id("account-1"));
        assert_ne!(id, derive_ice_peer_id("account-2"));
        assert!(id.parse::<u64>().is_ok_and(|n| n > 0 && n <= i64::MAX as u64));
        assert_eq!(parse_ice_peer_report(&json!("12345")).as_deref(), Some("12345"));
        assert_eq!(parse_ice_peer_report(&json!("012")), None);
        assert_eq!(parse_ice_peer_report(&json!("99999999999999999999")), None);
        assert_eq!(
            parse_steam_transport_id(&json!("76561198000000101")).as_deref(),
            Some("76561198000000101")
        );
        assert_eq!(parse_steam_transport_id(&json!("12345678901234567")), None);
    }
}
