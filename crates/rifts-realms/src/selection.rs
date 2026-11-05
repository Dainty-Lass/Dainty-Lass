// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Host selection: a host's presence report, ranking the candidates, offering the session to the best one and its
//! answer.

use rifts_seam::{AccountId, Actor, UnixMillis};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::host_policy::{artifact_hashes_match, host_ineligibility_reason, score_host};
use crate::hub::{RealmCapacity, RealmCapacityInput, decide_realm_capacity, realm_host_ineligibility_reason};
use crate::hunt::{ArtifactHashes, HostOfferRecord, HostPresenceRecord, HuntSession, HuntState as H, OfferStatus};
use crate::hunt::{GuardRole as StoredGuardRole, ReliabilityIncrements, ScoreBreakdown, TransitionActor};
use crate::leases::{Hosting, as_account, integer_in, number_in, refused, short_text};
use crate::lifecycle::{GuardListError, GuardRole, bump, queue, reset_to_selection, transition};
use crate::lifecycle::{parse_guard_presence_sessions, parse_guard_required_roles};
use crate::ports::HealthyGuard;
use crate::repo::hunts::HuntTx;
use crate::time::{Later as _, iso, parse_iso};

/// A report's own clock may be this far from ours.
const PRESENCE_WINDOW_MS: i64 = 30_000;
/// The first individual account's SteamID64; individual ids span the next 2^32.
const STEAM_ID_BASE: u64 = 76_561_197_960_265_728;
const SELECTION_ACTOR: Actor = Actor::System("realms-selection");

const FENCED: &str = "P2P_GENERATION_FENCED";
const MALFORMED: &str = "P2P_TICKET_INVALID";

fn all_digits(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_digit())
}

/// The transport's peer id for an account, in decimal; admission's tickets use the same derivation.
pub fn derive_ice_peer_id(account: &str) -> String {
    let peer = rifts_admission::derive_ice_peer_id(&AccountId(account.into()));
    format!("{}", peer.0)
}

/// 1 to 20 ASCII digits, no leading `0`, fitting a u64.
pub fn is_ice_peer_report(text: &str) -> bool {
    let shaped = !text.is_empty() && text.len() <= 20 && all_digits(text) && !text.starts_with('0');
    shaped && text.parse::<u64>().is_ok()
}

/// A 17-digit SteamID64 of an individual account.
pub fn is_steam_transport_id(text: &str) -> bool {
    let individual = STEAM_ID_BASE..=STEAM_ID_BASE + u64::from(u32::MAX);
    text.len() == 17 && all_digits(text) && text.parse::<u64>().is_ok_and(|id| individual.contains(&id))
}

/// Each hash's report keys, the current name first, in `ArtifactHashes` field order.
const ARTIFACT_KEYS: [&[&str]; 6] = [
    &["gameExe"],
    &["internalServer"],
    &["winmmProxy"],
    &["transportModule", "steamBridge"],
    &["transportRuntime", "steamRuntime"],
    &["admissionPublicKey"],
];

/// All six must be 64 hex characters.
pub fn parse_artifacts(value: Option<&Value>) -> Option<ArtifactHashes> {
    let object = value?.as_object()?;
    let mut read = ARTIFACT_KEYS.iter().map(|keys| {
        let present = keys.iter().find_map(|key| object.get(*key).filter(|v| !v.is_null()));
        let hash = short_text(present, 64).unwrap_or_default();
        (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_owned())
    });
    let mut next = || read.next().flatten();
    Some(ArtifactHashes {
        game_exe: next()?,
        internal_server: next()?,
        winmm_proxy: next()?,
        steam_bridge: next()?,
        steam_runtime: next()?,
        admission_public_key: next()?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceReply {
    pub accepted: bool,
    pub revision: i64,
    pub expires_at: String,
    pub artifacts_verified: bool,
    pub guard_verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferReply {
    pub accepted: bool,
    pub offer_id: String,
    pub session_id: String,
    pub generation: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RankedHost {
    pub account_id: String,
    pub steam_id: String,
    pub score: f64,
    pub breakdown: ScoreBreakdown,
}

impl From<&HealthyGuard> for StoredGuardRole {
    fn from(guard: &HealthyGuard) -> Self {
        Self {
            role: guard.role.as_str().into(),
            guard_session_id: guard.guard_session_id.clone(),
            manifest_sequence: guard.manifest_sequence,
            sequence: guard.last_sequence,
            risk_score: guard.risk_score,
        }
    }
}

/// What a presence report says before anything is looked up; every shape failure is one refusal.
struct Report<'a> {
    ready: bool,
    reported_peer: Option<&'a Value>,
    artifacts: ArtifactHashes,
    observed_at: i64,
    consent: bool,
    power_saver: bool,
    runtime_supported: bool,
    preflight: bool,
}

/// The host's measurements, each inside its bounds.
struct Metrics {
    free_mib: i64,
    commit_mib: i64,
    cpus: i64,
    load: f64,
    backend_rtt: f64,
    worst_rtt: f64,
    jitter: f64,
    loss: f64,
}

fn read_report<'a>(body: &'a Map<String, Value>, now: UnixMillis) -> Result<Report<'a>, RealmsError> {
    let malformed = || refused(MALFORMED, "The host presence report is malformed or stale.");
    let flag = |key: &str| body.get(key).and_then(Value::as_bool);
    let given = |key: &str| body.get(key).filter(|v| !v.is_null());
    let ready = flag("transportReady").or_else(|| flag("steamRuntimeReady"));
    let artifacts = parse_artifacts(body.get("artifacts"));
    let observed_at = short_text(body.get("observedAt"), 40)
        .and_then(parse_iso)
        .map(|at| at.0);
    let preflight = match given("preflightOnly") {
        None => Some(false),
        Some(value) => value.as_bool(),
    };
    let report = (|| {
        Some(Report {
            ready: ready?,
            reported_peer: given("peerId").or_else(|| given("steamId")),
            artifacts: artifacts?,
            observed_at: observed_at?,
            consent: flag("consentToHost")?,
            power_saver: flag("powerSaver")?,
            runtime_supported: flag("runtimeSupported")?,
            preflight: preflight?,
        })
    })();
    // A report from too far off our clock is as bad as a malformed one.
    report
        .filter(|r| (r.observed_at - now.0).abs() <= PRESENCE_WINDOW_MS)
        .ok_or_else(malformed)
}

fn read_metrics(body: &Map<String, Value>) -> Result<Metrics, RealmsError> {
    let whole = |key: &str, min: i64, max: i64| integer_in(body.get(key), min, max);
    let real = |key: &str, max: f64| number_in(body.get(key), 0.0, max);
    let metrics = (|| {
        Some(Metrics {
            free_mib: whole("freePhysicalMemoryMiB", 0, 1_048_576)?,
            commit_mib: whole("commitHeadroomMiB", 0, 1_048_576)?,
            cpus: whole("logicalCpuCount", 1, 1024)?,
            load: real("cpuLoadPercent", 100.0)?,
            backend_rtt: real("backendRttMs", 60_000.0)?,
            worst_rtt: real("worstPeerRttMs", 60_000.0)?,
            jitter: real("jitterMs", 60_000.0)?,
            loss: real("packetLossPercent", 100.0)?,
        })
    })();
    metrics.ok_or_else(|| refused(MALFORMED, "The host presence metrics are outside their bounds."))
}

/// The transport id a ready host is reached at: derived from the account under ICE, the reported SteamID otherwise.
fn transport_peer(
    account: &str,
    ready: bool,
    reported: Option<&Value>,
    ice: bool,
) -> Result<Option<String>, RealmsError> {
    let peer = reported.and_then(Value::as_str);
    match peer {
        Some(peer) if ready && ice && is_ice_peer_report(peer) => Ok(Some(derive_ice_peer_id(account))),
        Some(peer) if ready && !ice && is_steam_transport_id(peer) => Ok(Some(peer.to_owned())),
        _ if !ready && reported.is_none() => Ok(None),
        _ => Err(refused(MALFORMED, "The co-op transport observation is malformed.")),
    }
}

/// What the host is offered: one session, one generation, one candidate.
struct OfferPlan {
    session_id: String,
    chosen: RankedHost,
    offer_id: String,
    expires_at: UnixMillis,
    estimated_mib: f64,
    now: UnixMillis,
}

/// HOST_SELECTING to HOST_OFFERED with the offer, its transition and the host's control message; `None` when the
/// session moved on.
fn offer_host(tx: &mut HuntTx<'_>, plan: OfferPlan) -> Result<Option<HuntSession>, RealmsError> {
    let OfferPlan {
        session_id,
        chosen,
        offer_id,
        expires_at,
        estimated_mib,
        now,
    } = plan;
    let Some(current) = tx.find_hunt(&session_id)?.filter(|hunt| hunt.state == H::HostSelecting) else {
        return Ok(None);
    };
    let assign = |hunt: &mut HuntSession| {
        hunt.state = H::HostOffered;
        hunt.host_account_id.clone_from(&chosen.account_id);
        hunt.host_steam_id.clone_from(&chosen.steam_id);
        hunt.host_offer_id = Some(offer_id.clone());
        hunt.offer_expires_at = Some(expires_at);
        if !hunt.tried_host_account_ids.contains(&chosen.account_id) {
            hunt.tried_host_account_ids.push(chosen.account_id.clone());
        }
    };
    let selecting = [H::HostSelecting];
    let Some(offered) = tx.compare_and_set(
        &session_id,
        current.generation,
        &selecting,
        current.state_version,
        assign,
    )?
    else {
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
    let RankedHost {
        account_id,
        steam_id,
        score,
        breakdown: score_breakdown,
    } = chosen.clone();
    let record = HostOfferRecord {
        offer_id: offer_id.clone(),
        session_id: offered.session_id.clone(),
        generation: offered.generation,
        rank: 1,
        status: OfferStatus::Offered,
        offered_at: now,
        responded_at: None,
        response_reason: None,
        account_id,
        steam_id,
        score,
        score_breakdown,
        expires_at,
    };
    tx.create_offer(&record)?;
    let mut payload = Map::new();
    payload.insert("offerId".into(), offer_id.into());
    payload.insert("huntDefinitionId".into(), json!(offered.hunt_definition_id));
    payload.insert("partySize".into(), offered.roster.len().into());
    payload.insert("expiresAt".into(), iso(expires_at).into());
    payload.insert("estimatedMemoryMiB".into(), json!(estimated_mib));
    payload.insert("score".into(), json!(chosen.score));
    let life = expires_at.0 - now.0;
    queue(
        tx,
        &chosen.account_id,
        "HOST_OFFERED",
        &offered,
        Value::Object(payload),
        now,
        life,
    )?;
    Ok(Some(offered))
}

/// The host's answer to its offer, recorded with the hunt still owned by the offer.
struct OfferAnswer {
    account: String,
    offer: HostOfferRecord,
    accept: bool,
    reason: Option<String>,
    startup_ms: i64,
    now: UnixMillis,
}

fn answer_offer(tx: &mut HuntTx<'_>, answer: OfferAnswer) -> Result<(), RealmsError> {
    let OfferAnswer {
        account,
        offer,
        accept,
        reason,
        startup_ms,
        now,
    } = answer;
    let status = match accept {
        true => OfferStatus::Accepted,
        false => OfferStatus::Declined,
    };
    let updated = tx.update_offer(
        &offer.offer_id,
        &account,
        OfferStatus::Offered,
        status,
        now,
        reason.as_deref(),
    )?;
    if updated.is_none() {
        return Err(refused(FENCED, "The host offer was already answered."));
    }
    let owned = |hunt: &HuntSession| {
        hunt.state == H::HostOffered
            && hunt.generation == offer.generation
            && hunt.host_offer_id.as_deref() == Some(offer.offer_id.as_str())
    };
    let Some(session) = tx.find_hunt(&offer.session_id)?.filter(owned) else {
        return Err(refused(FENCED, "The host offer no longer owns this generation."));
    };
    if !accept {
        bump(
            tx,
            &account,
            ReliabilityIncrements {
                offers_declined: 1,
                ..Default::default()
            },
            now,
        )?;
        let Some(reset) = reset_to_selection(tx, &session)? else {
            return Err(refused(FENCED, "The host generation changed."));
        };
        let why = reason.as_deref().unwrap_or("HOST_OFFER_DECLINED");
        transition(
            tx,
            H::HostOffered,
            &reset,
            why,
            TransitionActor::Host,
            Some(&account),
            now,
        )?;
        return Ok(());
    }
    bump(
        tx,
        &account,
        ReliabilityIncrements {
            offers_accepted: 1,
            ..Default::default()
        },
        now,
    )?;
    let payload = json!({ "sessionId": session.session_id, "generation": session.generation, "role": "HOST" });
    queue(tx, &account, "HUNT_BOOTSTRAP", &session, payload, now, startup_ms)?;
    Ok(())
}

impl Hosting {
    /// Node's `SubmitHostPresence`.
    pub async fn submit_presence(
        &self,
        account: &str,
        body: &Map<String, Value>,
    ) -> Result<PresenceReply, RealmsError> {
        let now = self.now();
        let report = read_report(body, now)?;
        let metrics = read_metrics(body)?;
        let expires_at =
            UnixMillis(report.observed_at).saturating_add(self.settings().eligibility.presence_maximum_age_ms);
        let everyone = (UnixMillis(0), UnixMillis(i64::MIN));
        let stored = self
            .hunts
            .list_host_presences(&[account.to_owned()], everyone.0, everyone.1)
            .await?;
        let stored_revision = stored.first().map_or(0, |presence| presence.revision);
        let artifacts_verified = self.artifacts_verified(&report.artifacts);
        let ice = self.parts.operations.ice_transport();
        let steam_id = transport_peer(account, report.ready, report.reported_peer, ice)?;

        let whole_body = Value::Object(body.clone());
        let guard_refusal = |e: GuardListError| refused(e.code, e.message);
        let sessions = parse_guard_presence_sessions(&whole_body).map_err(guard_refusal)?;
        let realm_sessions = integer_in(body.get("realmHostSessions"), 0, 8).unwrap_or_default();
        let realm_host_only = realm_sessions > 0 && self.hosts_running_realm(account).await;
        let required = parse_guard_required_roles(&whole_body, &sessions, realm_host_only).map_err(guard_refusal)?;
        let healthy = self.healthy_guards(account, &sessions).await?;
        let guard_verified = !required.is_empty() && required.iter().all(|r| healthy.iter().any(|h| &h.role == r));
        // Enforce mode refuses an unverified host; observe mode only says it would have.
        let unverified = !(required.is_empty() || guard_verified);
        match (unverified, self.parts.operations.guard_refuses()) {
            (true, true) => {
                let text = "A current healthy Launcher Guard session is required to host.";
                return Err(refused("GUARD_REQUIRED", text));
            }
            (true, false) => self
                .parts
                .log
                .info(&format!("[GuardObserve] would refuse hosting for {account}")),
            (false, _) => {}
        }

        let reply = |revision| PresenceReply {
            accepted: true,
            revision,
            expires_at: iso(expires_at),
            artifacts_verified,
            guard_verified,
        };
        // A preflight that already matches stores nothing.
        if report.preflight && artifacts_verified {
            return Ok(reply(stored_revision));
        }
        let revision = stored_revision + 1;
        let by_role = |role| healthy.iter().find(|h| h.role == role);
        let primary = by_role(GuardRole::Client).or_else(|| by_role(GuardRole::Host));
        let presence = HostPresenceRecord {
            account_id: account.to_owned(),
            revision,
            consent_to_host: report.consent,
            observed_at: UnixMillis(report.observed_at),
            expires_at,
            free_physical_memory_mib: metrics.free_mib as f64,
            commit_headroom_mib: metrics.commit_mib as f64,
            logical_cpu_count: metrics.cpus as f64,
            cpu_load_percent: metrics.load,
            power_saver: report.power_saver,
            runtime_supported: report.runtime_supported,
            steam_runtime_ready: report.ready,
            steam_id: steam_id.clone(),
            backend_rtt_ms: metrics.backend_rtt,
            worst_peer_rtt_ms: metrics.worst_rtt,
            jitter_ms: metrics.jitter,
            packet_loss_percent: metrics.loss,
            artifacts: report.artifacts,
            artifacts_verified,
            launcher_version: short_text(body.get("launcherVersion"), 64)
                .unwrap_or("unknown")
                .to_owned(),
            guard_session_id: primary.map(|guard| guard.guard_session_id.clone()),
            guard_risk_score: primary.map(|guard| guard.risk_score),
            guard_verified: Some(guard_verified),
            guard_roles: healthy.iter().map(StoredGuardRole::from).collect(),
            realm_host_sessions: Some(realm_sessions),
        };
        self.hunts.upsert_host_presence(&as_account(account), &presence).await?;
        if let (true, Some(peer)) = (report.ready, steam_id.as_deref()) {
            let method = if ice { "ICE_ACCOUNT_DERIVED" } else { "HOST_PRESENCE" };
            self.ensure_verified_link(account, peer, u64::try_from(revision.max(1)).unwrap_or(1), method);
        }
        Ok(reply(revision))
    }

    /// Unverified when the expected hashes cannot be read.
    fn artifacts_verified(&self, reported: &ArtifactHashes) -> bool {
        match (self.parts.expected_artifacts)() {
            Ok(expected) => artifact_hashes_match(reported, &expected),
            Err(error) => {
                let line = format!(
                    "[P2PArtifacts] expected host artifact hashes unavailable ({error}); hosts stay unverified"
                );
                self.parts.log.warn(&line);
                false
            }
        }
    }

    /// The healthy Launcher Guard sessions among those the host reported, in report order.
    async fn healthy_guards(
        &self,
        account: &str,
        sessions: &[(GuardRole, String)],
    ) -> Result<Vec<HealthyGuard>, RealmsError> {
        let mut healthy = Vec::with_capacity(sessions.len());
        for (role, id) in sessions {
            let found = self
                .parts
                .guard
                .find_healthy(account.to_owned(), id.clone(), *role)
                .await;
            healthy.extend(found.map_err(RealmsError::Database)?);
        }
        Ok(healthy)
    }

    /// Realm hosting is on, the account hosts a realm, and a server of it runs.
    async fn hosts_running_realm(&self, account: &str) -> bool {
        if !self.settings().realm_hosting {
            return false;
        }
        let lookup = async {
            match self.realms.find_by_host(account).await? {
                Some(realm) => Ok::<_, RealmsError>(self.hunts.count_running_realm_servers(&realm.realm_id).await? > 0),
                None => Ok(false),
            }
        };
        lookup.await.unwrap_or_else(|error| {
            let line = format!("[P2P] realm host check for {account} failed; the client role stays required: {error}");
            self.parts.log.warn(&line);
            false
        })
    }

    async fn fresh_presences(&self, accounts: &[String]) -> Result<Vec<HostPresenceRecord>, RealmsError> {
        let now = self.now();
        let oldest = UnixMillis(now.0 - self.settings().eligibility.presence_maximum_age_ms);
        let found = self.hunts.list_host_presences(accounts, oldest, now).await?;
        Ok(found)
    }

    /// Node's `RankHosts`: lowest score first, then account id.
    pub async fn rank_hosts(&self, hunt: &HuntSession) -> Result<Vec<RankedHost>, RealmsError> {
        let roster: Vec<String> = hunt.roster.iter().map(|player| player.account_id.clone()).collect();
        let presences = self.fresh_presences(&roster).await?;
        let (policy, guard_required, now) = (
            &self.settings().eligibility,
            self.parts.operations.guard_refuses(),
            self.now(),
        );
        let untried = hunt
            .roster
            .iter()
            .filter(|player| !hunt.tried_host_account_ids.contains(&player.account_id));
        let mut ranked = Vec::new();
        for player in untried {
            let id = &player.account_id;
            let Some(presence) = presences.iter().find(|presence| &presence.account_id == id) else {
                continue;
            };
            let reliability = self.hunts.find_host_reliability(id).await?;
            let active = self.hunts.count_active_host_sessions(id).await?;
            let refusal =
                host_ineligibility_reason(presence, reliability.as_ref(), active, policy, guard_required, now);
            if let Some(reason) = refusal {
                self.parts
                    .log
                    .info(&format!("[P2P] host candidate {id} ineligible reason={reason}"));
                continue;
            }
            let (score, breakdown) = score_host(presence, reliability.as_ref(), policy, active);
            ranked.push(RankedHost {
                account_id: id.clone(),
                steam_id: player.steam_id.clone(),
                score,
                breakdown,
            });
        }
        ranked.sort_by(|a, b| a.score.total_cmp(&b.score).then(a.account_id.cmp(&b.account_id)));
        Ok(ranked)
    }

    /// Node's `RankRealmHost`: the realm's host is the only candidate; `Err(())` is REALM_FULL.
    #[allow(clippy::result_unit_err)]
    pub async fn rank_realm_host(&self, hunt: &HuntSession) -> Result<Result<Vec<RankedHost>, ()>, RealmsError> {
        let nobody = Ok(Ok(Vec::new()));
        let Some(realm_id) = hunt.realm_id.as_deref() else {
            return nobody;
        };
        let Some(realm) = self.realms.find_by_id(realm_id).await? else {
            return nobody;
        };
        let host = realm.host_account_id.clone();
        if hunt.tried_host_account_ids.contains(&host) {
            return nobody;
        }
        let mut fresh = self.fresh_presences(std::slice::from_ref(&host)).await?;
        if fresh.is_empty() {
            self.parts.log.info(&format!(
                "[P2P] realm host {host} for realm {realm_id} has no fresh presence"
            ));
            return nobody;
        }
        let presence = fresh.swap_remove(0);
        let settings = self.settings();
        let (guard_required, now) = (self.parts.operations.guard_refuses(), self.now());
        // The capacity check below stands in for the memory floors.
        if let Some(reason) =
            realm_host_ineligibility_reason(&presence, &settings.eligibility, 0.0, 0.0, guard_required, now)
        {
            self.parts.log.info(&format!(
                "[P2P] realm host {host} for realm {realm_id} ineligible reason={reason}"
            ));
            return nobody;
        }
        // This hunt already counts.
        let others = self.hunts.count_active_realm_servers(realm_id).await? - 1;
        let hub = &settings.realm_hub;
        let input = RealmCapacityInput {
            active_servers: others.max(0),
            max_servers: realm
                .hosting
                .as_ref()
                .map_or(hub.default_max_servers, |hosting| hosting.max_servers),
            free_physical_memory_mib: presence.free_physical_memory_mib,
            commit_headroom_mib: presence.commit_headroom_mib,
            minimum_free_memory_mib: hub.minimum_free_memory_mib,
            minimum_commit_headroom_mib: hub.minimum_commit_headroom_mib,
        };
        match decide_realm_capacity(&input) {
            RealmCapacity::Ok => {}
            RealmCapacity::OkLowFreeMemory => {
                let free = presence.free_physical_memory_mib;
                let line = format!(
                    "[RealmCapacity] realm {realm_id} hunt {} starts on a host with {free} MiB free physical memory",
                    hunt.session_id
                );
                self.parts.log.warn(&line);
            }
            RealmCapacity::ServerLimit | RealmCapacity::CommitLow => return Ok(Err(())),
        }
        let (score, breakdown) = score_host(&presence, None, &settings.eligibility, 0);
        let steam_id = derive_ice_peer_id(&host);
        Ok(Ok(vec![RankedHost {
            account_id: host,
            steam_id,
            score,
            breakdown,
        }]))
    }

    /// Node's `BeginHostSelection`. A disabled policy starts no selection, but recorded hunts still end.
    pub async fn begin_host_selection(&self, session_id: &str) -> Result<(), RealmsError> {
        let Some(hunt) = self
            .hunts
            .find_hunt(session_id)
            .await?
            .filter(|hunt| hunt.state == H::HostSelecting)
        else {
            return Ok(());
        };
        let prepare = |reason: &str| {
            self.parts
                .recovery
                .prepare(hunt.session_id.clone(), hunt.generation, reason.to_owned(), true)
        };
        if self.emergency_stop().await? {
            return prepare("OPERATOR_P2P_STOP").await;
        }
        if !self.settings().policy.enabled {
            return prepare("P2P_DISABLED").await;
        }
        let ranked = match hunt.realm_id {
            Some(_) => self.rank_realm_host(&hunt).await?,
            None => Ok(self.rank_hosts(&hunt).await?),
        };
        let candidate = match ranked.map(|hosts| hosts.into_iter().next()) {
            Ok(Some(best)) => best,
            Ok(None) => return prepare("NO_ELIGIBLE_HOST").await,
            Err(()) => return prepare("REALM_FULL").await,
        };
        let now = self.now();
        let eligibility = &self.settings().eligibility;
        let plan = OfferPlan {
            session_id: hunt.session_id.clone(),
            chosen: candidate.clone(),
            offer_id: crate::ids::random_uuid(),
            expires_at: now.saturating_add(eligibility.offer_lifetime_ms),
            estimated_mib: eligibility.minimum_commit_headroom_mib,
            now,
        };
        let offer_id = plan.offer_id.clone();
        let offered = self
            .hunts
            .transaction(&SELECTION_ACTOR, move |tx| offer_host(tx, plan))
            .await?;
        if offered.is_some() {
            let RankedHost { account_id, score, .. } = &candidate;
            let session = &hunt.session_id;
            self.parts.log.info(&format!(
                "[P2P] host offer {offer_id} session={session} account={account_id} score={score}"
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
        let uuid_shaped = offer_id.len() == 36 && offer_id.chars().all(|c| c == '-' || c.is_ascii_hexdigit());
        let Some(accept) = body.get("accept").and_then(Value::as_bool).filter(|_| uuid_shaped) else {
            return Err(refused(MALFORMED, "The host offer response is malformed."));
        };
        let Some(offer) = self
            .hunts
            .find_host_offer(offer_id)
            .await?
            .filter(|offer| offer.account_id == account)
        else {
            return Err(refused("P2P_SESSION_NOT_FOUND", "Host offer not found."));
        };
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
        let open = offer.status == OfferStatus::Offered && offer.expires_at > now;
        if !open {
            return Err(refused(FENCED, "The host offer has expired or was replaced."));
        }
        let prepare = |reason: &str| {
            self.parts
                .recovery
                .prepare(offer.session_id.clone(), offer.generation, reason.to_owned(), true)
        };
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
        let answer = OfferAnswer {
            account: account.to_owned(),
            offer: offer.clone(),
            accept,
            reason: short_text(body.get("reason"), 96).map(String::from),
            startup_ms: self.settings().eligibility.startup_lifetime_ms,
            now,
        };
        self.hunts
            .transaction(&as_account(account), move |tx| answer_offer(tx, answer))
            .await?;
        if !accept {
            self.begin_host_selection(&offer.session_id).await?;
        }
        Ok(reply)
    }
}
