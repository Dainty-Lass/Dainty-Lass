// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The realm directory: every launcher realm operation, the entry check Play runs, realm presence and its cache, and
//! the calls the p2p side and the sweeper make.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rifts_seam::{AccountId, Actor, Refusal, UnixMillis};
use serde::Serialize;
use serde_json::Value;

use crate::RealmsError;
use crate::db::RealmsDb;
use crate::input::{
    CreateRealmInput, EntryRealmInput, HeartbeatInput, InviteInput, JoinInput, SettingsInput, parse_entry_realm,
};
use crate::ports::{Accounts, Clock, IdSource, RateLimiter, RealmEventSink, RealmsLog};
use crate::realm::{
    AccountAt, HostingPatch, JoinDenial, JoinPolicy, JoinVia, MemberRole, PRESENCE_CACHE_MS, PRESENCE_TTL_MS, Patch,
    RealmBan, RealmEvent, RealmInviteRecord, RealmLimits, RealmMember, RealmPresenceRecord, RealmRecord, RemovalReason,
    apply_heartbeat, build_invite, decide_join, detect_ownership_change, hub_room_ids_of, invite_state,
    is_realm_hub_ready, is_realm_name_releasable, is_realm_online, is_realm_serving, js_trim, normalize_hub_room_id,
    presence_touch_needs_write, realm_name_for_username, realm_name_release_at, sort_newest_first,
    validate_hosting_patch,
};
use crate::repo::realms::{
    AddBanResult, AddEntryResult, AddMemberResult, InsertInviteResult, InsertRealmResult, RealmPresenceRepository,
    RealmsRepository,
};
use crate::time::{Later as _, iso};

pub struct DirectoryParts {
    pub db: RealmsDb,
    pub accounts: Arc<dyn Accounts>,
    pub events: Option<Arc<dyn RealmEventSink>>,
    pub clock: Arc<dyn Clock>,
    pub limiter: Arc<dyn RateLimiter>,
    pub ids: Arc<dyn IdSource>,
    pub limits: RealmLimits,
    pub log: Arc<dyn RealmsLog>,
}

const PRESENCE_CACHE_LIMIT: usize = 10_000;
const EPOCH_LIMIT: usize = 50_000;
const LIST_LIMIT: u32 = 100;
const INVITE_ATTEMPTS: usize = 5;
pub const SWEEP_LOOKBACK_MS: i64 = 600_000;
const SWEEP_LIMIT: u32 = 100;
const HEARTBEAT_TTL_SECONDS: i64 = 90;

const P2P_ACTOR: Actor = Actor::System("realms-p2p");
const SWEEPER_ACTOR: Actor = Actor::System("realms-sweeper");

#[derive(Debug, Clone, Copy)]
struct Bucket {
    name: &'static str,
    max: u32,
    window_ms: i64,
}

const READ: Bucket = Bucket {
    name: "read",
    max: 240,
    window_ms: 60_000,
};
const WRITE: Bucket = Bucket {
    name: "write",
    max: 60,
    window_ms: 60_000,
};
const HEARTBEAT: Bucket = Bucket {
    name: "heartbeat",
    max: 30,
    window_ms: 60_000,
};
const CREATE: Bucket = Bucket {
    name: "create",
    max: 5,
    window_ms: 3_600_000,
};
const INVITE: Bucket = Bucket {
    name: "invite",
    max: 30,
    window_ms: 3_600_000,
};
const JOIN: Bucket = Bucket {
    name: "join",
    max: 30,
    window_ms: 600_000,
};
const ENTRY: Bucket = Bucket {
    name: "entry",
    max: 30,
    window_ms: 600_000,
};

/// The refusals the join route and Play share; empty for any other code.
pub fn entry_message(code: &str) -> &'static str {
    match code {
        "REALM_NOT_FOUND" => "There is no realm with that name.",
        "REALM_OWNER_CHANGED" => "This realm now belongs to a different host.",
        "REALM_BANNED" => "You cannot join this realm.",
        "REALM_INVITE_REQUIRED" => "This realm is invite-only. You need an invite code.",
        "REALM_INVITE_INVALID" => "That invite code is not valid for this realm.",
        "REALM_INVITE_EXPIRED" => "That invite has expired, was revoked or has been used up.",
        "REALM_FULL" => "This realm cannot take more members.",
        "REALM_OFFLINE" => "This realm is offline: its host is not running it right now.",
        "REALM_SOLO" => "This realm is in solo mode: its host is playing alone right now.",
        "REALM_JOIN_REQUESTED" => "Request sent to the host. Wait for their approval, then press Play again.",
        "REALM_REQUESTS_FULL" => "This realm has too many join requests waiting. Try again later.",
        _ => "",
    }
}

fn refused(code: &str, message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

fn entry_error(code: &str) -> RealmsError {
    refused(code, entry_message(code))
}

fn not_hosted() -> RealmsError {
    refused("REALM_NOT_FOUND", "You do not host a realm.")
}

fn no_such_realm() -> RealmsError {
    refused("REALM_NOT_FOUND", "There is no realm with that name.")
}

fn invite_not_valid() -> RealmsError {
    refused("REALM_INVITE_INVALID", "That invite code is not valid.")
}

fn as_account(account: &str) -> Actor {
    Actor::Account(AccountId(account.to_owned()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostName {
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmPublicInfo {
    pub realm_id: String,
    pub name: String,
    pub host: HostName,
    pub host_public_key: String,
    pub join_policy: &'static str,
    pub listed: bool,
    pub online: bool,
    pub serving: bool,
    pub hub_ready: bool,
    pub player_count: i64,
    pub version: Option<String>,
    pub member_count: usize,
    pub membership: Option<&'static str>,
    pub requested: bool,
    pub invited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ownership_changed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub account_id: String,
    pub display_name: Option<String>,
    pub role: &'static str,
    pub joined_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BanView {
    pub account_id: String,
    pub display_name: Option<String>,
    pub reason: Option<String>,
    pub at: String,
}

/// A join request or a player invite as the host sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountAtView {
    pub account_id: String,
    pub display_name: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmHostView {
    #[serde(flatten)]
    pub info: RealmPublicInfo,
    pub created_at: String,
    pub last_online_at: Option<String>,
    pub online_until: Option<String>,
    pub name_release_at: String,
    pub members: Vec<MemberView>,
    pub bans: Vec<BanView>,
    pub join_request_count: usize,
    pub player_invites: Vec<AccountAtView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteView {
    pub code: String,
    pub created_at: String,
    pub expires_at: String,
    pub max_uses: i64,
    pub uses: i64,
    pub revoked_at: Option<String>,
    pub state: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceRealm {
    pub name: String,
    pub serving: bool,
    pub hub_ready: bool,
    pub host_online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceView {
    pub realm: Option<PresenceRealm>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostedReply {
    pub realm: RealmHostView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RealmReply {
    pub realm: RealmPublicInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RealmsReply {
    pub realms: Vec<RealmPublicInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MineReply {
    pub hosted: Option<RealmHostView>,
    pub member_of: Vec<RealmPublicInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InviteReply {
    pub invite: InviteView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvitesReply {
    pub invites: Vec<InviteView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JoinReply {
    pub realm: RealmPublicInfo,
    pub joined: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeftReply {
    pub left: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusReply {
    pub account_id: String,
    pub status: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedReply {
    pub account_id: String,
    pub status: &'static str,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestsReply {
    pub requests: Vec<AccountAtView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedReply {
    pub account_id: String,
    pub display_name: String,
    pub added: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitedReply {
    pub account_id: String,
    pub display_name: String,
    pub status: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerInviteEntry {
    pub realm: RealmPublicInfo,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerInvitesReply {
    pub invites: Vec<PlayerInviteEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeclinedReply {
    pub declined: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleasedReply {
    pub released: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatReply {
    pub online: bool,
    pub last_online_at: String,
    pub online_until: String,
    pub ttl_seconds: i64,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryAuthorization {
    Ok { realm_id: String, is_host: bool },
    Refused(&'static str),
}

/// (clear epoch, the account's counter).
type Epoch = (u64, u64);

#[derive(Default)]
struct PresenceCache {
    entries: HashMap<String, (UnixMillis, Option<RealmPresenceRecord>)>,
    counters: HashMap<String, u64>,
    clear_epoch: u64,
}

impl PresenceCache {
    fn epoch(&self, account: &str) -> Epoch {
        (self.clear_epoch, self.counters.get(account).copied().unwrap_or(0))
    }

    fn store(&mut self, account: &str, now: UnixMillis, row: Option<RealmPresenceRecord>) {
        if self.entries.len() >= PRESENCE_CACHE_LIMIT {
            self.entries
                .retain(|_, (read_at, _)| now.0 - read_at.0 < PRESENCE_CACHE_MS);
            if self.entries.len() >= PRESENCE_CACHE_LIMIT {
                self.entries.clear();
            }
        }
        self.entries.insert(account.to_owned(), (now, row));
    }

    fn invalidate(&mut self, account: &str) {
        *self.counters.entry(account.to_owned()).or_insert(0) += 1;
        if self.counters.len() > EPOCH_LIMIT {
            self.counters.clear();
            self.clear_epoch += 1;
        }
        self.entries.remove(account);
    }

    fn clear(&mut self) {
        self.clear_epoch += 1;
        self.counters.clear();
        self.entries.clear();
    }
}

enum Admitted {
    Added,
    AlreadyMember,
}

pub struct RealmDirectory {
    parts: DirectoryParts,
    realms: RealmsRepository,
    presence: RealmPresenceRepository,
    cache: Mutex<PresenceCache>,
    presence_reads: AtomicU64,
    presence_touches: AtomicU64,
}

impl RealmDirectory {
    pub fn new(parts: DirectoryParts) -> Self {
        Self {
            realms: RealmsRepository::new(parts.db.clone()),
            presence: RealmPresenceRepository::new(parts.db.clone()),
            parts,
            cache: Mutex::new(PresenceCache::default()),
            presence_reads: AtomicU64::new(0),
            presence_touches: AtomicU64::new(0),
        }
    }

    fn now(&self) -> UnixMillis {
        self.parts.clock.now()
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, PresenceCache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// (presence rows read from the database, touches written).
    pub fn presence_io(&self) -> (u64, u64) {
        (
            self.presence_reads.load(Ordering::SeqCst),
            self.presence_touches.load(Ordering::SeqCst),
        )
    }

    fn throttle(&self, bucket: Bucket, account: &str) -> Result<(), RealmsError> {
        let key = format!("realms:{}:{account}", bucket.name);
        if self.parts.limiter.is_limited(&key, bucket.max, bucket.window_ms) {
            return Err(refused(
                "AUTH_RATE_LIMITED",
                "Too many realm requests. Please wait a moment and try again.",
            ));
        }
        Ok(())
    }

    async fn emit(&self, event: RealmEvent) {
        if let Some(sink) = &self.parts.events
            && let Err(error) = sink.emit(event).await
        {
            self.parts.log.warn(&format!("[Realms] event sink failed: {error}"));
        }
    }

    async fn names(&self, ids: Vec<String>) -> BTreeMap<String, String> {
        match self.parts.accounts.display_names(ids).await {
            Ok(names) => names,
            Err(error) => {
                self.parts
                    .log
                    .warn(&format!("[Realms] display-name lookup failed: {error}"));
                BTreeMap::new()
            }
        }
    }

    async fn player_counts(&self, realm_ids: &[String], now: UnixMillis) -> BTreeMap<String, i64> {
        match self.presence.count_active_by_realm(realm_ids, now).await {
            Ok(counts) => counts,
            Err(error) => {
                self.parts
                    .log
                    .warn(&format!("[Realms] player-count lookup failed: {error}"));
                BTreeMap::new()
            }
        }
    }

    fn info_of(
        realm: &RealmRecord,
        viewer: &str,
        names: &BTreeMap<String, String>,
        counts: &BTreeMap<String, i64>,
        known_key: Option<&str>,
        now: UnixMillis,
    ) -> RealmPublicInfo {
        let serving =
            is_realm_serving(realm, now) && (realm.join_policy != JoinPolicy::Solo || realm.host_account_id == viewer);
        RealmPublicInfo {
            realm_id: realm.realm_id.clone(),
            name: realm.name.clone(),
            host: HostName {
                display_name: names.get(&realm.host_account_id).cloned(),
            },
            host_public_key: realm.host_public_key.clone(),
            join_policy: realm.join_policy.as_str(),
            listed: realm.listed,
            online: is_realm_online(realm, now),
            serving,
            hub_ready: is_realm_hub_ready(realm),
            player_count: counts.get(&realm.realm_id).copied().unwrap_or(0),
            version: realm.version.clone(),
            member_count: realm.members.len(),
            membership: realm
                .members
                .iter()
                .find(|m| m.account_id == viewer)
                .map(|m| m.role.as_str()),
            requested: realm.join_requests.iter().any(|r| r.account_id == viewer),
            invited: realm.player_invites.iter().any(|i| i.account_id == viewer),
            ownership_changed: known_key.map(|key| detect_ownership_change(&realm.name, key, realm)),
        }
    }

    async fn public_infos(&self, realms: &[RealmRecord], viewer: &str) -> Vec<RealmPublicInfo> {
        if realms.is_empty() {
            return Vec::new();
        }
        let now = self.now();
        let names = self
            .names(realms.iter().map(|r| r.host_account_id.clone()).collect())
            .await;
        let ids: Vec<String> = realms.iter().map(|r| r.realm_id.clone()).collect();
        let counts = self.player_counts(&ids, now).await;
        realms
            .iter()
            .map(|realm| Self::info_of(realm, viewer, &names, &counts, None, now))
            .collect()
    }

    async fn public_info(&self, realm: &RealmRecord, viewer: &str, known_key: Option<&str>) -> RealmPublicInfo {
        let now = self.now();
        let names = self.names(vec![realm.host_account_id.clone()]).await;
        let counts = self.player_counts(std::slice::from_ref(&realm.realm_id), now).await;
        Self::info_of(realm, viewer, &names, &counts, known_key, now)
    }

    async fn host_view(&self, realm: &RealmRecord) -> RealmHostView {
        let now = self.now();
        let mut ids = vec![realm.host_account_id.clone()];
        ids.extend(realm.members.iter().map(|m| m.account_id.clone()));
        ids.extend(realm.bans.iter().map(|b| b.account_id.clone()));
        ids.extend(realm.player_invites.iter().map(|i| i.account_id.clone()));
        ids.sort();
        ids.dedup();
        let names = self.names(ids).await;
        let counts = self.player_counts(std::slice::from_ref(&realm.realm_id), now).await;
        let host = realm.host_account_id.as_str();
        let name_of = |id: &str| names.get(id).cloned();
        RealmHostView {
            info: Self::info_of(realm, host, &names, &counts, None, now),
            created_at: iso(realm.created_at),
            last_online_at: realm.last_online_at.map(iso),
            online_until: realm.online_until.map(iso),
            name_release_at: iso(realm_name_release_at(realm.created_at, realm.last_online_at)),
            members: realm
                .members
                .iter()
                .map(|m| MemberView {
                    account_id: m.account_id.clone(),
                    display_name: name_of(&m.account_id),
                    role: m.role.as_str(),
                    joined_at: iso(m.joined_at),
                })
                .collect(),
            bans: realm
                .bans
                .iter()
                .map(|b| BanView {
                    account_id: b.account_id.clone(),
                    display_name: name_of(&b.account_id),
                    reason: b.reason.clone(),
                    at: iso(b.at),
                })
                .collect(),
            join_request_count: realm.join_requests.len(),
            player_invites: sort_newest_first(&realm.player_invites)
                .into_iter()
                .map(|i| AccountAtView {
                    display_name: name_of(&i.account_id),
                    account_id: i.account_id,
                    at: iso(i.at),
                })
                .collect(),
        }
    }

    fn invite_view(invite: &RealmInviteRecord, now: UnixMillis) -> InviteView {
        InviteView {
            code: invite.code.clone(),
            created_at: iso(invite.created_at),
            expires_at: iso(invite.expires_at),
            max_uses: invite.max_uses,
            uses: invite.uses,
            revoked_at: invite.revoked_at.map(iso),
            state: invite_state(invite, now).as_str(),
        }
    }

    async fn hosted(&self, account: &str) -> Result<RealmRecord, RealmsError> {
        self.realms.find_by_host(account).await?.ok_or_else(not_hosted)
    }

    async fn reload(&self, realm_id: &str) -> Result<RealmRecord, RealmsError> {
        self.realms
            .find_by_id(realm_id)
            .await?
            .ok_or_else(|| entry_error("REALM_NOT_FOUND"))
    }

    async fn drop_presence_in_realm(&self, account: &str, realm_id: &str) {
        if let Err(error) = self
            .presence
            .remove(&as_account(account), account, Some(realm_id))
            .await
        {
            self.parts.log.warn(&format!(
                "[Realms] could not remove presence realmId={realm_id}: {error}"
            ));
        }
        self.cache().invalidate(account);
    }

    async fn drop_realm_presence(&self, actor: &Actor, realm_id: &str) {
        if let Err(error) = self.presence.remove_for_realm(actor, realm_id).await {
            self.parts.log.warn(&format!(
                "[Realms] could not remove presence of realmId={realm_id}: {error}"
            ));
        }
        self.cache().clear();
    }

    /// Consumes an invite's use first and gives it back if the member was not added.
    async fn admit(
        &self,
        account: &str,
        realm: &RealmRecord,
        via: JoinVia,
        invite: Option<&RealmInviteRecord>,
    ) -> Result<Admitted, &'static str> {
        let actor = as_account(account);
        let now = self.now();
        let invite_code = match (via, invite) {
            (JoinVia::Invite, Some(invite)) => {
                let consumed = self
                    .realms
                    .consume_invite(&actor, &invite.code, now)
                    .await
                    .map_err(|_| "INTERNAL")?;
                if consumed.is_none() {
                    return Err("REALM_INVITE_EXPIRED");
                }
                Some(invite.code.clone())
            }
            _ => None,
        };
        let member = RealmMember {
            account_id: account.to_owned(),
            role: MemberRole::Member,
            joined_at: now,
        };
        let added = self
            .realms
            .add_member(&actor, &realm.realm_id, &member, self.parts.limits.max_members)
            .await;
        if !matches!(added, Ok(AddMemberResult::Added))
            && let Some(code) = &invite_code
        {
            let _ = self.realms.refund_invite(&actor, code).await;
        }
        match added.map_err(|_| "INTERNAL")? {
            AddMemberResult::Banned => Err("REALM_BANNED"),
            AddMemberResult::Full => Err("REALM_FULL"),
            AddMemberResult::NotFound => Err("REALM_NOT_FOUND"),
            AddMemberResult::AlreadyMember => Ok(Admitted::AlreadyMember),
            AddMemberResult::Added => {
                let via = if invite_code.is_some() { "INVITE" } else { "PUBLIC" };
                self.parts
                    .log
                    .info(&format!("[Realms] member joined realmId={} via={via}", realm.realm_id));
                Ok(Admitted::Added)
            }
        }
    }

    /// `Ok(true)` when the account turned out to be a member already.
    async fn request(&self, account: &str, realm: &RealmRecord) -> Result<bool, &'static str> {
        let request = AccountAt {
            account_id: account.to_owned(),
            at: self.now(),
        };
        let result = self
            .realms
            .add_join_request(
                &as_account(account),
                &realm.realm_id,
                &request,
                self.parts.limits.max_join_requests,
            )
            .await
            .map_err(|_| "INTERNAL")?;
        match result {
            AddEntryResult::Added => {
                self.parts
                    .log
                    .info(&format!("[Realms] join requested realmId={}", realm.realm_id));
                Err("REALM_JOIN_REQUESTED")
            }
            AddEntryResult::Already => Err("REALM_JOIN_REQUESTED"),
            AddEntryResult::AlreadyMember => Ok(true),
            AddEntryResult::Banned => Err("REALM_BANNED"),
            AddEntryResult::Full => Err("REALM_REQUESTS_FULL"),
            AddEntryResult::NotFound => Err("REALM_NOT_FOUND"),
        }
    }

    async fn accept_invite_for(&self, account: &str, realm_id: &str) -> Result<Admitted, &'static str> {
        let actor = as_account(account);
        let member = RealmMember {
            account_id: account.to_owned(),
            role: MemberRole::Member,
            joined_at: self.now(),
        };
        let result = self
            .realms
            .add_member(&actor, realm_id, &member, self.parts.limits.max_members)
            .await
            .map_err(|_| "INTERNAL")?;
        let admitted = match result {
            AddMemberResult::Banned => return Err("REALM_BANNED"),
            AddMemberResult::Full => return Err("REALM_FULL"),
            AddMemberResult::NotFound => return Err("REALM_NOT_FOUND"),
            // An `Added` drops the player invite in the same write.
            AddMemberResult::AlreadyMember => {
                let _ = self.realms.remove_player_invite(&actor, realm_id, account).await;
                Admitted::AlreadyMember
            }
            AddMemberResult::Added => Admitted::Added,
        };
        let _ = self.realms.remove_join_request(&actor, realm_id, account).await;
        if matches!(admitted, Admitted::Added) {
            self.parts
                .log
                .info(&format!("[Realms] player invite accepted realmId={realm_id}"));
        }
        Ok(admitted)
    }

    fn internal(code: &'static str) -> RealmsError {
        if code == "INTERNAL" {
            RealmsError::Database("realm directory write failed".to_owned())
        } else {
            entry_error(code)
        }
    }

    pub async fn create_realm(&self, account: &str, input: &CreateRealmInput) -> Result<HostedReply, RealmsError> {
        self.throttle(CREATE, account)?;
        const ALREADY: &str = "You already host a realm. Release it before claiming another name.";
        if self.realms.find_by_host(account).await?.is_some() {
            return Err(refused("REALM_ALREADY_EXISTS", ALREADY));
        }
        let own_name = self.names(vec![account.to_owned()]).await.remove(account);
        let own = own_name.as_deref().and_then(realm_name_for_username);
        let name = match &input.name {
            None => own.ok_or_else(|| {
                refused(
                    "REALM_USERNAME_REQUIRED",
                    "Set a username before you host a realm: your realm is named after it.",
                )
            })?,
            Some(name) => {
                if own.as_deref() != Some(name.as_str()) {
                    let holder = self
                        .parts
                        .accounts
                        .resolve_username(name.clone())
                        .await
                        .map_err(RealmsError::Database)?;
                    if holder.is_some_and(|holder| holder != account) {
                        return Err(refused(
                            "REALM_NAME_IS_USERNAME",
                            "That name is another player's username. A realm is named after its host.",
                        ));
                    }
                }
                name.clone()
            }
        };
        let now = self.now();
        let realm_id = self.parts.ids.realm_id();
        let record = RealmRecord {
            hub_room_ids: hub_room_ids_of(&realm_id),
            realm_id,
            name: name.clone(),
            host_account_id: account.to_owned(),
            host_public_key: input.host_public_key.clone(),
            join_policy: input.join_policy,
            listed: input.listed,
            members: vec![RealmMember {
                account_id: account.to_owned(),
                role: MemberRole::Host,
                joined_at: now,
            }],
            bans: Vec::new(),
            created_at: now,
            last_online_at: None,
            online_until: None,
            version: input.version.clone(),
            hosting: None,
            offline_notified_for: None,
            join_requests: Vec::new(),
            player_invites: Vec::new(),
        };
        let actor = as_account(account);
        let mut result = self.realms.insert_realm(&actor, &record).await?;
        if result == InsertRealmResult::NameTaken
            && let Some(holder) = self.realms.find_by_name(&name).await?
            && is_realm_name_releasable(&holder, now)
            && self
                .realms
                .delete_realm_if_last_online(&actor, &holder.realm_id, holder.last_online_at)
                .await?
        {
            self.parts.log.info(&format!(
                "[Realms] name released realmId={} name={name}",
                holder.realm_id
            ));
            self.drop_realm_presence(&actor, &holder.realm_id).await;
            result = self.realms.insert_realm(&actor, &record).await?;
        }
        match result {
            InsertRealmResult::HostHasRealm => Err(refused("REALM_ALREADY_EXISTS", ALREADY)),
            InsertRealmResult::NameTaken => Err(refused("REALM_NAME_TAKEN", "That realm name is taken.")),
            InsertRealmResult::Inserted => {
                self.parts.log.info(&format!(
                    "[Realms] realm created realmId={} name={name} policy={} listed={}",
                    record.realm_id,
                    record.join_policy.as_str(),
                    record.listed
                ));
                Ok(HostedReply {
                    realm: self.host_view(&record).await,
                })
            }
        }
    }

    pub async fn get_mine(&self, account: &str) -> Result<MineReply, RealmsError> {
        self.throttle(READ, account)?;
        let hosted = self.realms.find_by_host(account).await?;
        let member_of: Vec<RealmRecord> = self
            .realms
            .find_by_member(account, LIST_LIMIT)
            .await?
            .into_iter()
            .filter(|r| r.host_account_id != account)
            .collect();
        let hosted = match &hosted {
            Some(realm) => Some(self.host_view(realm).await),
            None => None,
        };
        Ok(MineReply {
            hosted,
            member_of: self.public_infos(&member_of, account).await,
        })
    }

    pub async fn get_by_name(
        &self,
        account: &str,
        name: &str,
        known_host_key: Option<&str>,
    ) -> Result<RealmReply, RealmsError> {
        self.throttle(READ, account)?;
        let realm = self.realms.find_by_name(name).await?.ok_or_else(no_such_realm)?;
        Ok(RealmReply {
            realm: self.public_info(&realm, account, known_host_key).await,
        })
    }

    /// Uses nothing.
    pub async fn preview_invite(&self, account: &str, code: &str) -> Result<RealmReply, RealmsError> {
        self.throttle(JOIN, account)?;
        let invite = self.realms.find_invite(code).await?.ok_or_else(invite_not_valid)?;
        if invite_state(&invite, self.now()) != crate::realm::InviteState::Usable {
            return Err(refused(
                "REALM_INVITE_EXPIRED",
                "That invite has expired, was revoked or has been used up.",
            ));
        }
        let realm = self
            .realms
            .find_by_id(&invite.realm_id)
            .await?
            .ok_or_else(invite_not_valid)?;
        if realm.bans.iter().any(|b| b.account_id == account) {
            return Err(refused("REALM_BANNED", "You cannot join this realm."));
        }
        Ok(RealmReply {
            realm: self.public_info(&realm, account, None).await,
        })
    }

    /// Online first, then by name; a SOLO realm only to its host.
    pub async fn list_listed(&self, account: &str, limit: u32, offset: u32) -> Result<RealmsReply, RealmsError> {
        self.throttle(READ, account)?;
        let realms: Vec<RealmRecord> = self
            .realms
            .list_listed(limit, offset)
            .await?
            .into_iter()
            .filter(|r| r.join_policy != JoinPolicy::Solo || r.host_account_id == account)
            .collect();
        Ok(RealmsReply {
            realms: self.public_infos(&realms, account).await,
        })
    }

    pub async fn update_settings(&self, account: &str, input: &SettingsInput) -> Result<HostedReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        let changed = self
            .realms
            .set_settings(
                &as_account(account),
                &realm.realm_id,
                account,
                input.join_policy,
                input.listed,
            )
            .await?;
        if !changed {
            return Err(not_hosted());
        }
        let realm = self.realms.find_by_id(&realm.realm_id).await?.ok_or_else(not_hosted)?;
        self.parts.log.info(&format!(
            "[Realms] settings changed realmId={} policy={} listed={}",
            realm.realm_id,
            realm.join_policy.as_str(),
            realm.listed
        ));
        Ok(HostedReply {
            realm: self.host_view(&realm).await,
        })
    }

    pub async fn create_invite(&self, account: &str, input: &InviteInput) -> Result<InviteReply, RealmsError> {
        self.throttle(INVITE, account)?;
        let realm = self.hosted(account).await?;
        let now = self.now();
        let limit = self.parts.limits.max_active_invites;
        if self.realms.count_active_invites(&realm.realm_id, now).await? >= i64::from(limit) {
            return Err(refused(
                "REALM_INVITE_LIMIT",
                &format!("You can have at most {limit} open invites; revoke one first."),
            ));
        }
        let actor = as_account(account);
        for _ in 0..INVITE_ATTEMPTS {
            let invite = build_invite(
                &self.parts.ids.invite_code(),
                &realm.realm_id,
                account,
                now,
                input.max_uses,
                input.ttl_hours,
            );
            if self.realms.insert_invite(&actor, &invite).await? == InsertInviteResult::Inserted {
                self.parts.log.info(&format!(
                    "[Realms] invite created realmId={} maxUses={} expiresAt={}",
                    realm.realm_id,
                    invite.max_uses,
                    iso(invite.expires_at)
                ));
                return Ok(InviteReply {
                    invite: Self::invite_view(&invite, now),
                });
            }
        }
        Err(RealmsError::Database(
            "Could not generate a unique invite code.".to_owned(),
        ))
    }

    pub async fn list_invites(&self, account: &str) -> Result<InvitesReply, RealmsError> {
        self.throttle(READ, account)?;
        let realm = self.hosted(account).await?;
        let now = self.now();
        let invites = self.realms.list_invites(&realm.realm_id, LIST_LIMIT).await?;
        Ok(InvitesReply {
            invites: invites.iter().map(|i| Self::invite_view(i, now)).collect(),
        })
    }

    /// Revoked once: a second revoke keeps the first time.
    pub async fn revoke_invite(&self, account: &str, code: &str) -> Result<InviteReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        let now = self.now();
        let invite = self
            .realms
            .revoke_invite(&as_account(account), &realm.realm_id, code, now)
            .await?
            .ok_or_else(invite_not_valid)?;
        self.parts
            .log
            .info(&format!("[Realms] invite revoked realmId={}", realm.realm_id));
        Ok(InviteReply {
            invite: Self::invite_view(&invite, now),
        })
    }

    pub async fn join(&self, account: &str, input: &JoinInput) -> Result<JoinReply, RealmsError> {
        self.throttle(JOIN, account)?;
        let invite = match &input.invite_code {
            Some(code) => Some(self.realms.find_invite(code).await?.ok_or_else(invite_not_valid)?),
            None => None,
        };
        let realm = match (&input.name, &invite) {
            (Some(name), _) => self.realms.find_by_name(name).await?,
            (None, Some(invite)) => self.realms.find_by_id(&invite.realm_id).await?,
            (None, None) => None,
        };
        let realm = realm.ok_or_else(|| entry_error("REALM_NOT_FOUND"))?;
        if invite.as_ref().is_some_and(|i| i.realm_id != realm.realm_id) {
            return Err(entry_error("REALM_INVITE_INVALID"));
        }
        if let Some(key) = &input.known_host_key
            && detect_ownership_change(&realm.name, key, &realm)
        {
            return Err(entry_error("REALM_OWNER_CHANGED"));
        }
        let decision = decide_join(&realm, account, invite.as_ref(), self.now());
        let via = match decision {
            Err(JoinDenial::InviteRequired) if realm.player_invites.iter().any(|i| i.account_id == account) => {
                self.accept_invite_for(account, &realm.realm_id)
                    .await
                    .map_err(Self::internal)?;
                let realm = self.reload(&realm.realm_id).await?;
                return Ok(JoinReply {
                    realm: self.public_info(&realm, account, None).await,
                    joined: true,
                });
            }
            Err(JoinDenial::InviteRequired) => {
                self.request(account, &realm).await.map_err(Self::internal)?;
                JoinVia::Member
            }
            Err(denial) => return Err(entry_error(denial.code())),
            Ok(via) => via,
        };
        let joined = match via {
            JoinVia::Member => false,
            JoinVia::Public | JoinVia::Invite => matches!(
                self.admit(account, &realm, via, invite.as_ref())
                    .await
                    .map_err(Self::internal)?,
                Admitted::Added
            ),
        };
        let realm = self.reload(&realm.realm_id).await?;
        Ok(JoinReply {
            realm: self.public_info(&realm, account, None).await,
            joined,
        })
    }

    /// Play's realm check: refusals are returned, not thrown. The host enters its own realm even when nothing serves.
    pub async fn authorize_entry(
        &self,
        account: &str,
        input: &EntryRealmInput,
    ) -> Result<EntryAuthorization, RealmsError> {
        use EntryAuthorization::Refused;
        self.throttle(ENTRY, account)?;
        let invite = match &input.invite_code {
            Some(code) => match self.realms.find_invite(code).await? {
                Some(invite) => Some(invite),
                None => return Ok(Refused("REALM_INVITE_INVALID")),
            },
            None => None,
        };
        let Some(realm) = self.realms.find_by_name(&input.name).await? else {
            return Ok(Refused("REALM_NOT_FOUND"));
        };
        if invite.as_ref().is_some_and(|i| i.realm_id != realm.realm_id) {
            return Ok(Refused("REALM_INVITE_INVALID"));
        }
        if let Some(key) = &input.known_host_key
            && detect_ownership_change(&realm.name, key, &realm)
        {
            return Ok(Refused("REALM_OWNER_CHANGED"));
        }
        let via = match decide_join(&realm, account, invite.as_ref(), self.now()) {
            Err(JoinDenial::InviteRequired) if realm.player_invites.iter().any(|i| i.account_id == account) => {
                if let Err(code) = self.accept_invite_for(account, &realm.realm_id).await {
                    return Ok(Refused(code));
                }
                JoinVia::Member
            }
            Err(JoinDenial::InviteRequired) => match self.request(account, &realm).await {
                Ok(_) => JoinVia::Member,
                Err(code) => return Ok(Refused(code)),
            },
            Err(denial) => return Ok(Refused(denial.code())),
            Ok(via) => via,
        };
        let is_host = realm.host_account_id == account;
        // An offline refusal uses no invite and adds no member.
        if !is_host && !is_realm_serving(&realm, self.now()) {
            return Ok(Refused("REALM_OFFLINE"));
        }
        if matches!(via, JoinVia::Public | JoinVia::Invite)
            && let Err(code) = self.admit(account, &realm, via, invite.as_ref()).await
        {
            return Ok(Refused(code));
        }
        Ok(EntryAuthorization::Ok {
            realm_id: realm.realm_id,
            is_host,
        })
    }

    pub async fn leave(&self, account: &str, name: &str) -> Result<LeftReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.realms.find_by_name(name).await?.ok_or_else(no_such_realm)?;
        if realm.host_account_id == account {
            return Err(refused(
                "REALM_HOST_CANNOT_LEAVE",
                "You host this realm. Release it instead of leaving it.",
            ));
        }
        if !self
            .realms
            .remove_member(&as_account(account), &realm.realm_id, account)
            .await?
        {
            return Err(refused("REALM_NOT_MEMBER", "You are not a member of this realm."));
        }
        self.parts
            .log
            .info(&format!("[Realms] member left realmId={}", realm.realm_id));
        self.drop_presence_in_realm(account, &realm.realm_id).await;
        self.emit(RealmEvent::MemberRemoved {
            realm_id: realm.realm_id.clone(),
            account_id: account.to_owned(),
            reason: RemovalReason::Leave,
        })
        .await;
        Ok(LeftReply { left: true })
    }

    /// A kicked member may come back.
    pub async fn kick(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        if target == account {
            return Err(refused(
                "REALM_INVALID_TARGET",
                "You cannot remove yourself from your own realm.",
            ));
        }
        if !self
            .realms
            .remove_member(&as_account(account), &realm.realm_id, target)
            .await?
        {
            return Err(refused(
                "REALM_NOT_MEMBER",
                "That account is not a member of your realm.",
            ));
        }
        self.parts
            .log
            .info(&format!("[Realms] member kicked realmId={}", realm.realm_id));
        self.drop_presence_in_realm(target, &realm.realm_id).await;
        self.emit(RealmEvent::MemberRemoved {
            realm_id: realm.realm_id.clone(),
            account_id: target.to_owned(),
            reason: RemovalReason::Kick,
        })
        .await;
        Ok(StatusReply {
            account_id: target.to_owned(),
            status: "KICKED",
        })
    }

    pub async fn ban(&self, account: &str, target: &str, reason: Option<String>) -> Result<ChangedReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        if target == account {
            return Err(refused(
                "REALM_INVALID_TARGET",
                "You cannot ban yourself from your own realm.",
            ));
        }
        let ban = RealmBan {
            account_id: target.to_owned(),
            reason,
            at: self.now(),
        };
        let result = self
            .realms
            .add_ban(&as_account(account), &realm.realm_id, &ban, self.parts.limits.max_bans)
            .await?;
        match result {
            AddBanResult::NotFound => return Err(not_hosted()),
            AddBanResult::Full => return Err(refused("REALM_FULL", "The ban list is full; lift a ban first.")),
            AddBanResult::Banned => {
                self.parts
                    .log
                    .info(&format!("[Realms] account banned realmId={}", realm.realm_id));
                self.drop_presence_in_realm(target, &realm.realm_id).await;
                self.emit(RealmEvent::MemberRemoved {
                    realm_id: realm.realm_id.clone(),
                    account_id: target.to_owned(),
                    reason: RemovalReason::Ban,
                })
                .await;
            }
            AddBanResult::AlreadyBanned => {}
        }
        Ok(ChangedReply {
            account_id: target.to_owned(),
            status: "BANNED",
            changed: result == AddBanResult::Banned,
        })
    }

    /// Never an error.
    pub async fn unban(&self, account: &str, target: &str) -> Result<ChangedReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        let changed = self
            .realms
            .remove_ban(&as_account(account), &realm.realm_id, target)
            .await?;
        if changed {
            self.parts
                .log
                .info(&format!("[Realms] ban lifted realmId={}", realm.realm_id));
        }
        Ok(ChangedReply {
            account_id: target.to_owned(),
            status: "UNBANNED",
            changed,
        })
    }

    pub async fn list_join_requests(&self, account: &str) -> Result<RequestsReply, RealmsError> {
        self.throttle(READ, account)?;
        let realm = self.hosted(account).await?;
        let sorted = sort_newest_first(&realm.join_requests);
        let names = self.names(sorted.iter().map(|r| r.account_id.clone()).collect()).await;
        Ok(RequestsReply {
            requests: sorted
                .into_iter()
                .map(|r| AccountAtView {
                    display_name: names.get(&r.account_id).cloned(),
                    account_id: r.account_id,
                    at: iso(r.at),
                })
                .collect(),
        })
    }

    pub async fn approve_join_request(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        if !realm.join_requests.iter().any(|r| r.account_id == target) {
            return Err(refused(
                "REALM_REQUEST_NOT_FOUND",
                "That player has not asked to join your realm.",
            ));
        }
        let actor = as_account(account);
        let member = RealmMember {
            account_id: target.to_owned(),
            role: MemberRole::Member,
            joined_at: self.now(),
        };
        let result = self
            .realms
            .add_member(&actor, &realm.realm_id, &member, self.parts.limits.max_members)
            .await?;
        match result {
            AddMemberResult::NotFound => return Err(not_hosted()),
            AddMemberResult::Banned => {
                self.realms.remove_join_request(&actor, &realm.realm_id, target).await?;
                return Err(refused("REALM_BANNED", "That player is banned from your realm."));
            }
            AddMemberResult::Full => return Err(refused("REALM_FULL", "This realm cannot take more members.")),
            AddMemberResult::Added | AddMemberResult::AlreadyMember => {}
        }
        self.realms.remove_join_request(&actor, &realm.realm_id, target).await?;
        self.parts
            .log
            .info(&format!("[Realms] join request approved realmId={}", realm.realm_id));
        Ok(StatusReply {
            account_id: target.to_owned(),
            status: "APPROVED",
        })
    }

    /// The player may ask again.
    pub async fn deny_join_request(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        if !self
            .realms
            .remove_join_request(&as_account(account), &realm.realm_id, target)
            .await?
        {
            return Err(refused(
                "REALM_REQUEST_NOT_FOUND",
                "That player has not asked to join your realm.",
            ));
        }
        self.parts
            .log
            .info(&format!("[Realms] join request denied realmId={}", realm.realm_id));
        Ok(StatusReply {
            account_id: target.to_owned(),
            status: "DENIED",
        })
    }

    async fn resolve_player(&self, username: &str) -> Result<(String, String), RealmsError> {
        let username = js_trim(username).to_owned();
        let found = self
            .parts
            .accounts
            .resolve_username(username.clone())
            .await
            .map_err(RealmsError::Database)?;
        let target = found.ok_or_else(|| refused("REALM_PLAYER_NOT_FOUND", "No player has that username."))?;
        let display = self
            .names(vec![target.clone()])
            .await
            .remove(&target)
            .unwrap_or(username);
        Ok((target, display))
    }

    /// Launchers to 0.1.68 add members directly.
    pub async fn add_member_by_username(&self, account: &str, username: &str) -> Result<AddedReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        let (target, display_name) = self.resolve_player(username).await?;
        let actor = as_account(account);
        let member = RealmMember {
            account_id: target.clone(),
            role: MemberRole::Member,
            joined_at: self.now(),
        };
        let result = self
            .realms
            .add_member(&actor, &realm.realm_id, &member, self.parts.limits.max_members)
            .await?;
        match result {
            AddMemberResult::NotFound => return Err(not_hosted()),
            AddMemberResult::Banned => {
                return Err(refused(
                    "REALM_BANNED",
                    "That player is banned from your realm. Lift the ban first.",
                ));
            }
            AddMemberResult::Full => return Err(refused("REALM_FULL", "This realm cannot take more members.")),
            AddMemberResult::Added | AddMemberResult::AlreadyMember => {}
        }
        self.realms
            .remove_join_request(&actor, &realm.realm_id, &target)
            .await?;
        let added = result == AddMemberResult::Added;
        if added {
            self.parts
                .log
                .info(&format!("[Realms] player added by the host realmId={}", realm.realm_id));
        }
        Ok(AddedReply {
            account_id: target,
            display_name,
            added,
        })
    }

    pub async fn invite_player_by_username(&self, account: &str, username: &str) -> Result<InvitedReply, RealmsError> {
        self.throttle(INVITE, account)?;
        let realm = self.hosted(account).await?;
        let (target, display_name) = self.resolve_player(username).await?;
        if target == account {
            return Err(refused("REALM_INVALID_TARGET", "You are the host of this realm."));
        }
        let banned = || {
            refused(
                "REALM_BANNED",
                "That player is banned from your realm. Lift the ban first.",
            )
        };
        let reply = |status| InvitedReply {
            account_id: target.clone(),
            display_name: display_name.clone(),
            status,
        };
        if realm.join_requests.iter().any(|r| r.account_id == target) {
            return match self.accept_invite_for(&target, &realm.realm_id).await {
                Ok(_) => {
                    self.parts.log.info(&format!(
                        "[Realms] invited player had asked to join; let in realmId={}",
                        realm.realm_id
                    ));
                    Ok(reply("MEMBER"))
                }
                Err("REALM_BANNED") => Err(banned()),
                Err("REALM_FULL") => Err(refused("REALM_FULL", "This realm cannot take more members.")),
                Err(_) => Err(not_hosted()),
            };
        }
        let invite = AccountAt {
            account_id: target.clone(),
            at: self.now(),
        };
        let result = self
            .realms
            .add_player_invite(
                &as_account(account),
                &realm.realm_id,
                &invite,
                self.parts.limits.max_player_invites,
            )
            .await?;
        match result {
            AddEntryResult::NotFound => Err(not_hosted()),
            AddEntryResult::Banned => Err(banned()),
            AddEntryResult::Full => Err(refused(
                "REALM_PLAYER_INVITES_FULL",
                "Too many invites are waiting for an answer. Cancel one first.",
            )),
            AddEntryResult::AlreadyMember => Ok(reply("MEMBER")),
            AddEntryResult::Added => {
                self.parts
                    .log
                    .info(&format!("[Realms] player invited realmId={}", realm.realm_id));
                Ok(reply("INVITED"))
            }
            AddEntryResult::Already => Ok(reply("INVITED")),
        }
    }

    pub async fn cancel_player_invite(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        if !self
            .realms
            .remove_player_invite(&as_account(account), &realm.realm_id, target)
            .await?
        {
            return Err(refused(
                "REALM_PLAYER_INVITE_NOT_FOUND",
                "That player has no invite waiting.",
            ));
        }
        self.parts
            .log
            .info(&format!("[Realms] player invite cancelled realmId={}", realm.realm_id));
        Ok(StatusReply {
            account_id: target.to_owned(),
            status: "CANCELLED",
        })
    }

    pub async fn list_my_player_invites(&self, account: &str) -> Result<PlayerInvitesReply, RealmsError> {
        self.throttle(READ, account)?;
        let realms = self.realms.find_by_player_invite(account, LIST_LIMIT).await?;
        let infos = self.public_infos(&realms, account).await;
        let mut entries: Vec<(UnixMillis, RealmPublicInfo)> = realms
            .iter()
            .zip(infos)
            .map(|(realm, info)| {
                let at = realm
                    .player_invites
                    .iter()
                    .find(|i| i.account_id == account)
                    .map_or(realm.created_at, |i| i.at);
                (at, info)
            })
            .collect();
        entries.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        Ok(PlayerInvitesReply {
            invites: entries
                .into_iter()
                .map(|(at, realm)| PlayerInviteEntry { realm, at: iso(at) })
                .collect(),
        })
    }

    pub async fn accept_player_invite(&self, account: &str, name: &str) -> Result<JoinReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.realms.find_by_name(name).await?.ok_or_else(no_such_realm)?;
        if !realm.player_invites.iter().any(|i| i.account_id == account) {
            return Err(refused(
                "REALM_PLAYER_INVITE_NOT_FOUND",
                "You have no invite to this realm.",
            ));
        }
        if let Err(code) = self.accept_invite_for(account, &realm.realm_id).await {
            if code == "REALM_BANNED" {
                let _ = self
                    .realms
                    .remove_player_invite(&as_account(account), &realm.realm_id, account)
                    .await;
            }
            return Err(Self::internal(code));
        }
        let realm = self.reload(&realm.realm_id).await?;
        Ok(JoinReply {
            realm: self.public_info(&realm, account, None).await,
            joined: true,
        })
    }

    pub async fn decline_player_invite(&self, account: &str, name: &str) -> Result<DeclinedReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.realms.find_by_name(name).await?.ok_or_else(no_such_realm)?;
        if !self
            .realms
            .remove_player_invite(&as_account(account), &realm.realm_id, account)
            .await?
        {
            return Err(refused(
                "REALM_PLAYER_INVITE_NOT_FOUND",
                "You have no invite to this realm.",
            ));
        }
        self.parts
            .log
            .info(&format!("[Realms] player invite declined realmId={}", realm.realm_id));
        Ok(DeclinedReply { declined: true })
    }

    /// The hub fields are never self-reported.
    pub async fn heartbeat(&self, account: &str, input: &HeartbeatInput) -> Result<HeartbeatReply, RealmsError> {
        self.throttle(HEARTBEAT, account)?;
        let realm = self.hosted(account).await?;
        let now = self.now();
        let update = apply_heartbeat(now, input.online, input.version.clone());
        let actor = as_account(account);
        if !self
            .realms
            .record_heartbeat(&actor, &realm.realm_id, account, &update)
            .await?
        {
            return Err(not_hosted());
        }
        if let Some(hosting) = &input.hosting {
            let patch = HostingPatch {
                max_servers: Some(hosting.max_servers),
                max_players: Some(hosting.max_players),
                afk_seconds: hosting.afk_seconds.map_or(Patch::Remove, Patch::Set),
                ..HostingPatch::default()
            };
            self.realms.patch_hosting(&actor, &realm.realm_id, &patch, now).await?;
        }
        Ok(HeartbeatReply {
            online: input.online != Some(false),
            last_online_at: iso(update.last_online_at),
            online_until: iso(update.online_until),
            ttl_seconds: HEARTBEAT_TTL_SECONDS,
            version: input.version.clone().or(realm.version),
        })
    }

    /// The name is free at once; members and invites go with the realm.
    pub async fn release(&self, account: &str) -> Result<ReleasedReply, RealmsError> {
        self.throttle(WRITE, account)?;
        let realm = self.hosted(account).await?;
        let actor = as_account(account);
        if !self
            .realms
            .delete_realm_of_host(&actor, &realm.realm_id, account)
            .await?
        {
            return Err(not_hosted());
        }
        self.parts.log.info(&format!(
            "[Realms] realm released realmId={} name={}",
            realm.realm_id, realm.name
        ));
        self.drop_realm_presence(&actor, &realm.realm_id).await;
        self.emit(RealmEvent::RealmReleased {
            realm_id: realm.realm_id.clone(),
            host_account_id: account.to_owned(),
        })
        .await;
        Ok(ReleasedReply { released: true })
    }

    async fn presence_view(&self, row: Option<RealmPresenceRecord>) -> Result<PresenceView, RealmsError> {
        let Some(row) = row else {
            return Ok(PresenceView {
                realm: None,
                expires_at: None,
            });
        };
        let Some(realm) = self.realms.find_by_id(&row.realm_id).await? else {
            return Ok(PresenceView {
                realm: None,
                expires_at: None,
            });
        };
        let now = self.now();
        Ok(PresenceView {
            realm: Some(PresenceRealm {
                name: realm.name.clone(),
                serving: is_realm_serving(&realm, now),
                hub_ready: is_realm_hub_ready(&realm),
                host_online: is_realm_online(&realm, now),
            }),
            expires_at: Some(iso(row.expires_at)),
        })
    }

    pub async fn presence_heartbeat(&self, account: &str) -> Result<PresenceView, RealmsError> {
        self.throttle(HEARTBEAT, account)?;
        let row = self.touch_presence(account).await?;
        self.presence_view(row).await
    }

    /// Idempotent.
    pub async fn presence_leave(&self, account: &str) -> Result<LeftReply, RealmsError> {
        self.throttle(HEARTBEAT, account)?;
        self.leave_presence(account).await?;
        Ok(LeftReply { left: true })
    }

    pub async fn presence_status(&self, account: &str) -> Result<PresenceView, RealmsError> {
        self.throttle(READ, account)?;
        let row = self.presence_of(account).await?;
        self.presence_view(row).await
    }

    /// On the matchmaking hot path: each answer, "none" included, is trusted for two seconds.
    pub async fn presence_of(&self, account: &str) -> Result<Option<RealmPresenceRecord>, RealmsError> {
        let now = self.now();
        let epoch = {
            let cache = self.cache();
            if let Some((read_at, row)) = cache.entries.get(account)
                && (0..PRESENCE_CACHE_MS).contains(&(now.0 - read_at.0))
            {
                // A lapsed row is never active.
                return Ok(row.clone().filter(|r| r.expires_at > now));
            }
            cache.epoch(account)
        };
        self.presence_reads.fetch_add(1, Ordering::SeqCst);
        let row = self.presence.find_active(account, now).await?;
        let mut cache = self.cache();
        // A write that raced the read must not be overwritten by the stale answer.
        if cache.epoch(account) == epoch {
            cache.store(account, now, row.clone());
        }
        Ok(row)
    }

    pub async fn bind_presence(&self, account: &str, realm_id: Option<&str>) -> Result<(), RealmsError> {
        let actor = as_account(account);
        match realm_id {
            None => {
                self.presence.remove(&actor, account, None).await?;
            }
            Some(realm_id) => {
                let now = self.now();
                let row = RealmPresenceRecord {
                    account_id: account.to_owned(),
                    realm_id: realm_id.to_owned(),
                    bound_at: now,
                    last_seen_at: now,
                    expires_at: now.saturating_add(PRESENCE_TTL_MS),
                };
                self.presence.bind(&actor, &row).await?;
            }
        }
        self.cache().invalidate(account);
        Ok(())
    }

    /// An absent or lapsed row is never written back.
    pub async fn touch_presence(&self, account: &str) -> Result<Option<RealmPresenceRecord>, RealmsError> {
        let Some(current) = self.presence_of(account).await? else {
            return Ok(None);
        };
        let now = self.now();
        if !presence_touch_needs_write(current.last_seen_at, current.expires_at, now) {
            return Ok(Some(current));
        }
        let epoch = self.cache().epoch(account);
        self.presence_touches.fetch_add(1, Ordering::SeqCst);
        let touched = self
            .presence
            .touch(&as_account(account), account, now, now.saturating_add(PRESENCE_TTL_MS))
            .await?;
        let mut cache = self.cache();
        let unmoved = cache.epoch(account) == epoch;
        cache.invalidate(account);
        if unmoved {
            cache.store(account, now, touched.clone());
        }
        Ok(touched)
    }

    pub async fn leave_presence(&self, account: &str) -> Result<(), RealmsError> {
        self.presence.remove(&as_account(account), account, None).await?;
        self.cache().invalidate(account);
        Ok(())
    }

    /// Play's realm: `None` leaves Play unchanged; a realm is never played as official when the module is off.
    pub async fn authorize_game_session_realm(
        &self,
        account: &str,
        realm: Option<&Value>,
        module_enabled: bool,
    ) -> Result<Option<(String, bool)>, RealmsError> {
        let Some(realm) = realm.filter(|r| !r.is_null()) else {
            return Ok(None);
        };
        if !module_enabled {
            return Err(refused("REALMS_DISABLED", "Realms are not available on this server."));
        }
        let input = parse_entry_realm(realm).map_err(|f| refused(f.code, &f.message))?;
        match self.authorize_entry(account, &input).await? {
            EntryAuthorization::Refused(code) => Err(entry_error(code)),
            EntryAuthorization::Ok { realm_id, is_host } => {
                self.parts.log.info(&format!(
                    "[Realms] game session authorized realmId={realm_id} host={is_host}"
                ));
                Ok(Some((realm_id, is_host)))
            }
        }
    }

    pub async fn bind_presence_at_login(
        &self,
        account: &str,
        realm_id: Option<&str>,
        module_enabled: bool,
    ) -> Result<(), RealmsError> {
        if !module_enabled {
            return Ok(());
        }
        match realm_id {
            None => {
                if let Err(error) = self.bind_presence(account, None).await {
                    self.parts
                        .log
                        .warn(&format!("[Realms] could not reset realm presence at login: {error}"));
                }
                Ok(())
            }
            Some(realm_id) => self.bind_presence(account, Some(realm_id)).await,
        }
    }

    pub async fn set_hosting(&self, realm_id: &str, patch: &HostingPatch) -> Result<bool, RealmsError> {
        if let Some(field) = validate_hosting_patch(patch) {
            return Err(RealmsError::Database(format!("setHosting: invalid {field}")));
        }
        Ok(self
            .realms
            .patch_hosting(&P2P_ACTOR, realm_id, patch, self.now())
            .await?)
    }

    /// Ends the online state now; emits nothing.
    pub async fn mark_offline(&self, realm_id: &str) -> Result<bool, RealmsError> {
        Ok(self.realms.mark_offline(&P2P_ACTOR, realm_id, self.now()).await?)
    }

    pub async fn find_by_hub_room_id(&self, room: &str) -> Result<Option<RealmRecord>, RealmsError> {
        let Some(room) = normalize_hub_room_id(room) else {
            return Ok(None);
        };
        Ok(self.realms.find_by_hub_room_id(&room).await?)
    }

    pub async fn get_realm(&self, realm_id: &str) -> Result<Option<RealmRecord>, RealmsError> {
        Ok(self.realms.find_by_id(realm_id).await?)
    }

    pub async fn get_realm_by_host(&self, host: &str) -> Result<Option<RealmRecord>, RealmsError> {
        Ok(self.realms.find_by_host(host).await?)
    }

    pub async fn is_member(&self, realm_id: &str, account: &str) -> Result<bool, RealmsError> {
        Ok(self
            .get_realm(realm_id)
            .await?
            .is_some_and(|r| r.members.iter().any(|m| m.account_id == account)))
    }

    pub async fn is_banned(&self, realm_id: &str, account: &str) -> Result<bool, RealmsError> {
        Ok(self
            .get_realm(realm_id)
            .await?
            .is_some_and(|r| r.bans.iter().any(|b| b.account_id == account)))
    }

    /// Each lapse is reported once; nothing schedules this, the caller drives it.
    pub async fn sweep_lapsed_hosts(&self) -> Result<usize, RealmsError> {
        let now = self.now();
        let lapsed = self
            .realms
            .claim_lapsed_hosts(&SWEEPER_ACTOR, now, UnixMillis(now.0 - SWEEP_LOOKBACK_MS), SWEEP_LIMIT)
            .await?;
        for realm in &lapsed {
            self.parts
                .log
                .info(&format!("[Realms] host offline realmId={}", realm.realm_id));
            self.emit(RealmEvent::HostOffline {
                realm_id: realm.realm_id.clone(),
                host_account_id: realm.host_account_id.clone(),
            })
            .await;
        }
        Ok(lapsed.len())
    }
}
