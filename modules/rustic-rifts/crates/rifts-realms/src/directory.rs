//! The realm directory (`realms/realmsService.ts`): claiming a realm, membership, invites, join requests, player
//! invites, the host's heartbeat, presence and the hosting record the p2p side mirrors. The answers are the launcher's
//! JSON shapes, fields in Node's order. Nothing in them carries an address: the directory never holds one.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rifts_seam::{AccountId, Actor, Refusal, UnixMillis};
use serde::Serialize;

use crate::RealmsError;
use crate::db::RealmsDb;
use crate::input::{CreateRealmInput, EntryRealmInput, HeartbeatInput, InviteInput, JoinInput, SettingsInput};
use crate::ports::{Accounts, Clock, IdSource, RateLimiter, RealmEventSink, RealmsLog};
use crate::realm::{
    AccountAt, HEARTBEAT_ONLINE_MS, HostingPatch, InviteState, JoinDenial, JoinVia, MemberRole, PRESENCE_CACHE_MS,
    PRESENCE_TTL_MS, Patch, RealmBan, RealmEvent, RealmInviteRecord, RealmLimits, RealmMember, RealmPresenceRecord,
    RealmRecord, RemovalReason, apply_heartbeat, build_invite, decide_join, detect_ownership_change,
    hub_room_ids_of, invite_state, is_realm_hub_ready, is_realm_name_releasable, is_realm_online, is_realm_serving,
    normalize_hub_room_id, presence_touch_needs_write, realm_name_for_username, realm_name_release_at,
    sort_newest_first, validate_hosting_patch,
};
use crate::repo::realms::{
    AddBanResult, AddEntryResult, AddMemberResult, InsertInviteResult, InsertRealmResult, RealmPresenceRepository,
    RealmsRepository,
};
use crate::time::iso;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;

/// `RATE_BUDGETS`: per-account budgets of the launcher routes.
#[derive(Debug, Clone, Copy)]
enum Bucket {
    Read,
    Write,
    Heartbeat,
    Create,
    Invite,
    /// Joining by code is also the only way to guess one.
    Join,
    Entry,
}

impl Bucket {
    const fn budget(self) -> (&'static str, u32, i64) {
        match self {
            Self::Read => ("read", 240, MINUTE),
            Self::Write => ("write", 60, MINUTE),
            Self::Heartbeat => ("heartbeat", 30, MINUTE),
            Self::Create => ("create", 5, HOUR),
            Self::Invite => ("invite", 30, HOUR),
            Self::Join => ("join", 30, 10 * MINUTE),
            Self::Entry => ("entry", 30, 10 * MINUTE),
        }
    }
}

const MEMBER_LIST_LIMIT: u32 = 100;
const INVITE_LIST_LIMIT: u32 = 100;
/// A lapse older than this is not "just lapsed": the sweeper leaves it.
pub const SWEEP_LOOKBACK_MS: i64 = 10 * MINUTE;
const SWEEP_BATCH: u32 = 100;
const PRESENCE_CACHE_LIMIT: usize = 10_000;

fn refused(code: &str, message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

/// `REALM_ENTRY_MESSAGES`: the refusals the join route and Play share.
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

fn entry_error(code: &'static str) -> RealmsError {
    refused(code, entry_message(code))
}

fn not_hosted() -> RealmsError {
    refused("REALM_NOT_FOUND", "You do not host a realm.")
}

// ------------------------------------------------------------------ the answers

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostLabel {
    pub display_name: Option<String>,
}

/// `RealmPublicInfo`: what anyone signed in may learn about a realm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmPublicInfo {
    pub realm_id: String,
    pub name: String,
    pub host: HostLabel,
    pub host_public_key: String,
    pub join_policy: &'static str,
    pub listed: bool,
    pub online: bool,
    /// Online with its hub running: the realm can be entered.
    pub serving: bool,
    pub hub_ready: bool,
    pub player_count: i64,
    pub version: Option<String>,
    pub member_count: usize,
    pub membership: Option<&'static str>,
    pub requested: bool,
    pub invited: bool,
    /// Only when the caller sent the host key it knew.
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

/// A waiting player or an unanswered player invite, as the host sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountAtView {
    pub account_id: String,
    pub display_name: Option<String>,
    pub at: String,
}

/// `RealmHostView`: the public info, then the people and dates the host manages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmHostView {
    #[serde(flatten)]
    pub info: RealmPublicInfo,
    pub created_at: String,
    pub last_online_at: Option<String>,
    pub online_until: Option<String>,
    pub name_release_at: Option<String>,
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

/// The launcher's presence routes' answer.
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
pub struct PlayerInviteView {
    pub realm: RealmPublicInfo,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerInvitesReply {
    pub invites: Vec<PlayerInviteView>,
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

/// `EntryAuthorization`: Play with a realm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryAuthorization {
    Ok { realm_id: String, is_host: bool },
    Refused(&'static str),
}

// ------------------------------------------------------------------ presence cache

/// `presenceOf` is on the matchmaking hot path, so answers ("none" included) are kept 2 s in this process. Every write
/// moves the account's epoch, so a read that began before it can't put its old answer back.
#[derive(Debug, Default)]
struct PresenceCache {
    entries: HashMap<String, (i64, Option<RealmPresenceRecord>)>,
    epochs: HashMap<String, u64>,
    clear_epoch: u64,
}

impl PresenceCache {
    fn epoch(&self, account: &str) -> (u64, u64) {
        (self.clear_epoch, self.epochs.get(account).copied().unwrap_or(0))
    }

    fn put(&mut self, account: &str, value: Option<RealmPresenceRecord>, now: i64) {
        if self.entries.len() >= PRESENCE_CACHE_LIMIT {
            self.entries.retain(|_, (at, _)| now - *at < PRESENCE_CACHE_MS);
            if self.entries.len() >= PRESENCE_CACHE_LIMIT {
                self.entries.clear();
            }
        }
        self.entries.insert(account.to_owned(), (now, value));
    }

    fn invalidate(&mut self, account: &str) {
        *self.epochs.entry(account.to_owned()).or_insert(0) += 1;
        if self.epochs.len() > PRESENCE_CACHE_LIMIT * 5 {
            self.epochs.clear();
            self.clear_epoch += 1;
        }
        self.entries.remove(account);
    }

    fn clear(&mut self) {
        self.clear_epoch += 1;
        self.epochs.clear();
        self.entries.clear();
    }
}

// ------------------------------------------------------------------ the service

/// What the directory runs on.
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

/// `CreateRealmsService`.
pub struct RealmDirectory {
    repo: RealmsRepository,
    presence: RealmPresenceRepository,
    accounts: Arc<dyn Accounts>,
    events: Option<Arc<dyn RealmEventSink>>,
    clock: Arc<dyn Clock>,
    limiter: Arc<dyn RateLimiter>,
    ids: Arc<dyn IdSource>,
    limits: RealmLimits,
    log: Arc<dyn RealmsLog>,
    cache: Mutex<PresenceCache>,
    presence_reads: AtomicU64,
    presence_touches: AtomicU64,
}

fn player(account: &str) -> Actor {
    Actor::Account(AccountId(account.to_owned()))
}

const P2P: Actor = Actor::System("realms-p2p");

enum JoinRequestOutcome {
    Member,
    Refused(&'static str),
}

impl RealmDirectory {
    pub fn new(parts: DirectoryParts) -> Self {
        Self {
            repo: RealmsRepository::new(parts.db.clone()),
            presence: RealmPresenceRepository::new(parts.db),
            accounts: parts.accounts,
            events: parts.events,
            clock: parts.clock,
            limiter: parts.limiter,
            ids: parts.ids,
            limits: parts.limits,
            log: parts.log,
            cache: Mutex::new(PresenceCache::default()),
            presence_reads: AtomicU64::new(0),
            presence_touches: AtomicU64::new(0),
        }
    }

    /// Presence rows read from the database, and touches written: what the 2 s cache and the touch rule save.
    pub fn presence_io(&self) -> (u64, u64) {
        (
            self.presence_reads.load(Ordering::Relaxed),
            self.presence_touches.load(Ordering::Relaxed),
        )
    }

    fn now(&self) -> UnixMillis {
        self.clock.now()
    }

    fn throttle(&self, bucket: Bucket, account: &str) -> Result<(), RealmsError> {
        let (name, max, window) = bucket.budget();
        if self.limiter.is_limited(&format!("realms:{name}:{account}"), max, window) {
            return Err(refused(
                "AUTH_RATE_LIMITED",
                "Too many realm requests. Please wait a moment and try again.",
            ));
        }
        Ok(())
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, PresenceCache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn require_hosted(&self, account: &str) -> Result<RealmRecord, RealmsError> {
        self.repo.find_by_host(account).await?.ok_or_else(not_hosted)
    }

    /// Labels are best effort: a failed lookup never fails the directory.
    async fn names_for(&self, ids: Vec<String>) -> BTreeMap<String, String> {
        match self.accounts.display_names(ids).await {
            Ok(names) => names,
            Err(error) => {
                self.log.warn(&format!("[Realms] display-name lookup failed: {error}"));
                BTreeMap::new()
            }
        }
    }

    async fn player_counts(&self, realm_ids: Vec<String>) -> BTreeMap<String, i64> {
        match self.presence.count_active_by_realm(&realm_ids, self.now()).await {
            Ok(counts) => counts,
            Err(error) => {
                self.log.warn(&format!("[Realms] player-count lookup failed: {error}"));
                BTreeMap::new()
            }
        }
    }

    fn public_info(
        &self,
        realm: &RealmRecord,
        viewer: &str,
        names: &BTreeMap<String, String>,
        counts: &BTreeMap<String, i64>,
        known_host_key: Option<&str>,
    ) -> RealmPublicInfo {
        let now = self.now();
        RealmPublicInfo {
            realm_id: realm.realm_id.clone(),
            name: realm.name.clone(),
            host: HostLabel {
                display_name: names.get(&realm.host_account_id).cloned(),
            },
            host_public_key: realm.host_public_key.clone(),
            join_policy: realm.join_policy.as_str(),
            listed: realm.listed,
            online: is_realm_online(realm, now),
            // A SOLO realm can be entered by its host only, so to everyone else it is not serving
            serving: is_realm_serving(realm, now)
                && (realm.join_policy != crate::realm::JoinPolicy::Solo || viewer == realm.host_account_id),
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
            ownership_changed: known_host_key.map(|key| detect_ownership_change(&realm.name, key, realm)),
        }
    }

    async fn public_infos(
        &self,
        realms: &[RealmRecord],
        viewer: &str,
        known_host_key: Option<&str>,
    ) -> Vec<RealmPublicInfo> {
        let names = self.names_for(realms.iter().map(|r| r.host_account_id.clone()).collect()).await;
        let counts = self.player_counts(realms.iter().map(|r| r.realm_id.clone()).collect()).await;
        realms
            .iter()
            .map(|realm| self.public_info(realm, viewer, &names, &counts, known_host_key))
            .collect()
    }

    async fn public_info_one(&self, realm: &RealmRecord, viewer: &str, known: Option<&str>) -> RealmPublicInfo {
        let mut infos = self.public_infos(std::slice::from_ref(realm), viewer, known).await;
        infos.remove(0)
    }

    async fn host_view(&self, realm: &RealmRecord, viewer: &str) -> RealmHostView {
        let invites = sort_newest_first(&realm.player_invites);
        let ids: Vec<String> = std::iter::once(realm.host_account_id.clone())
            .chain(realm.members.iter().map(|m| m.account_id.clone()))
            .chain(realm.bans.iter().map(|b| b.account_id.clone()))
            .chain(invites.iter().map(|i| i.account_id.clone()))
            .collect();
        let names = self.names_for(ids).await;
        let counts = self.player_counts(vec![realm.realm_id.clone()]).await;
        let name = |id: &str| names.get(id).cloned();
        RealmHostView {
            info: self.public_info(realm, viewer, &names, &counts, None),
            created_at: iso(realm.created_at),
            last_online_at: realm.last_online_at.map(iso),
            online_until: realm.online_until.map(iso),
            name_release_at: Some(iso(realm_name_release_at(realm.created_at, realm.last_online_at))),
            members: realm
                .members
                .iter()
                .map(|m| MemberView {
                    account_id: m.account_id.clone(),
                    display_name: name(&m.account_id),
                    role: m.role.as_str(),
                    joined_at: iso(m.joined_at),
                })
                .collect(),
            bans: realm
                .bans
                .iter()
                .map(|b| BanView {
                    account_id: b.account_id.clone(),
                    display_name: name(&b.account_id),
                    reason: b.reason.clone(),
                    at: iso(b.at),
                })
                .collect(),
            join_request_count: realm.join_requests.len(),
            player_invites: invites
                .iter()
                .map(|i| AccountAtView {
                    account_id: i.account_id.clone(),
                    display_name: name(&i.account_id),
                    at: iso(i.at),
                })
                .collect(),
        }
    }

    fn invite_view(&self, invite: &RealmInviteRecord) -> InviteView {
        InviteView {
            code: invite.code.clone(),
            created_at: iso(invite.created_at),
            expires_at: iso(invite.expires_at),
            max_uses: invite.max_uses,
            uses: invite.uses,
            revoked_at: invite.revoked_at.map(iso),
            state: invite_state(invite, self.now()).as_str(),
        }
    }

    async fn emit(&self, event: RealmEvent) {
        let Some(sink) = &self.events else {
            return;
        };
        if let Err(error) = sink.emit(event).await {
            self.log.warn(&format!("[Realms] event sink failed: {error}"));
        }
    }

    // ------------------------------------------------------------------ presence

    /// The account's active presence, cached 2 s in this process.
    pub async fn presence_of(&self, account: &str) -> Result<Option<RealmPresenceRecord>, RealmsError> {
        let now = self.now();
        let epoch = {
            let cache = self.cache();
            if let Some((at, value)) = cache.entries.get(account)
                && now.0 - at >= 0
                && now.0 - at < PRESENCE_CACHE_MS
            {
                // A cached row that has lapsed since is no longer active
                return Ok(value.clone().filter(|row| row.expires_at > now));
            }
            cache.epoch(account)
        };
        self.presence_reads.fetch_add(1, Ordering::Relaxed);
        let row = self.presence.find_active(account, now).await?;
        let mut cache = self.cache();
        if cache.epoch(account) == epoch {
            cache.put(account, row.clone(), now.0);
        }
        Ok(row)
    }

    /// A login's realm (or none) replaces whatever context the account had.
    pub async fn bind_presence(&self, account: &str, realm_id: Option<&str>) -> Result<(), RealmsError> {
        match realm_id {
            None => {
                self.presence.remove(&player(account), account, None).await?;
            }
            Some(realm_id) => {
                let now = self.now();
                let row = RealmPresenceRecord {
                    account_id: account.to_owned(),
                    realm_id: realm_id.to_owned(),
                    bound_at: now,
                    last_seen_at: now,
                    expires_at: UnixMillis(now.0 + PRESENCE_TTL_MS),
                };
                self.presence.bind(&player(account), &row).await?;
            }
        }
        self.cache().invalidate(account);
        Ok(())
    }

    /// Another 120 s for an active presence, written only when the row needs it; never resurrects one.
    pub async fn touch_presence(&self, account: &str) -> Result<Option<RealmPresenceRecord>, RealmsError> {
        let now = self.now();
        let Some(current) = self.presence_of(account).await? else {
            return Ok(None);
        };
        if !presence_touch_needs_write(current.last_seen_at, current.expires_at, now) {
            return Ok(Some(current));
        }
        let epoch = self.cache().epoch(account);
        self.presence_touches.fetch_add(1, Ordering::Relaxed);
        let row = self
            .presence
            .touch(&player(account), account, now, UnixMillis(now.0 + PRESENCE_TTL_MS))
            .await?;
        let mut cache = self.cache();
        let unchanged = cache.epoch(account) == epoch;
        cache.invalidate(account);
        if unchanged {
            cache.put(account, row.clone(), now.0);
        }
        Ok(row)
    }

    pub async fn leave_presence(&self, account: &str) -> Result<(), RealmsError> {
        self.presence.remove(&player(account), account, None).await?;
        self.cache().invalidate(account);
        Ok(())
    }

    /// After a kick, ban or leave; a failure does not undo the removal.
    async fn drop_presence_in_realm(&self, actor: &Actor, account: &str, realm_id: &str) {
        if let Err(error) = self.presence.remove(actor, account, Some(realm_id)).await {
            self.log.warn(&format!("[Realms] could not remove presence realmId={realm_id}: {error}"));
        }
        self.cache().invalidate(account);
    }

    /// After a release: the accounts are unknown, so the whole cache goes.
    async fn drop_presence_of_realm(&self, actor: &Actor, realm_id: &str) {
        if let Err(error) = self.presence.remove_for_realm(actor, realm_id).await {
            self.log.warn(&format!("[Realms] could not remove presence of realmId={realm_id}: {error}"));
        }
        self.cache().clear();
    }

    async fn presence_view(&self, presence: Option<RealmPresenceRecord>) -> Result<PresenceView, RealmsError> {
        let empty = PresenceView {
            realm: None,
            expires_at: None,
        };
        let Some(presence) = presence else {
            return Ok(empty);
        };
        let Some(realm) = self.repo.find_by_id(&presence.realm_id).await? else {
            return Ok(empty);
        };
        let now = self.now();
        Ok(PresenceView {
            realm: Some(PresenceRealm {
                name: realm.name.clone(),
                serving: is_realm_serving(&realm, now),
                hub_ready: is_realm_hub_ready(&realm),
                host_online: is_realm_online(&realm, now),
            }),
            expires_at: Some(iso(presence.expires_at)),
        })
    }

    // ------------------------------------------------------------------ joining

    /// An invite is consumed first (the step that stops two joins sharing its last use), then given back if the
    /// membership can't be added. `Ok(added)`, or the refusal code.
    async fn admit_member(
        &self,
        realm: &RealmRecord,
        account: &str,
        via: JoinVia,
        invite: Option<&RealmInviteRecord>,
    ) -> Result<Result<bool, &'static str>, RealmsError> {
        let actor = player(account);
        let mut consumed = None;
        if via == JoinVia::Invite {
            let Some(invite) = invite else {
                return Ok(Err("REALM_INVITE_EXPIRED"));
            };
            match self.repo.consume_invite(&actor, &invite.code, self.now()).await? {
                Some(used) => consumed = Some(used.code),
                None => return Ok(Err("REALM_INVITE_EXPIRED")),
            }
        }
        let member = RealmMember {
            account_id: account.to_owned(),
            role: MemberRole::Member,
            joined_at: self.now(),
        };
        let added = self.repo.add_member(&actor, &realm.realm_id, &member, self.limits.max_members).await?;
        if added != AddMemberResult::Added
            && let Some(code) = &consumed
        {
            self.repo.refund_invite(&actor, code).await?;
        }
        Ok(match added {
            AddMemberResult::Banned => Err("REALM_BANNED"),
            AddMemberResult::Full => Err("REALM_FULL"),
            AddMemberResult::NotFound => Err("REALM_NOT_FOUND"),
            AddMemberResult::Added => {
                let via = if via == JoinVia::Invite { "INVITE" } else { "PUBLIC" };
                self.log.info(&format!("[Realms] member joined realmId={} via={via}", realm.realm_id));
                Ok(true)
            }
            AddMemberResult::AlreadyMember => Ok(false),
        })
    }

    /// An invite-only realm refused a player without an invite: the request is recorded (once) and they wait.
    async fn request_join(&self, realm_id: &str, account: &str) -> Result<JoinRequestOutcome, RealmsError> {
        let request = AccountAt {
            account_id: account.to_owned(),
            at: self.now(),
        };
        let result = self
            .repo
            .add_join_request(&player(account), realm_id, &request, self.limits.max_join_requests)
            .await?;
        Ok(match result {
            AddEntryResult::Added => {
                self.log.info(&format!("[Realms] join requested realmId={realm_id}"));
                JoinRequestOutcome::Refused("REALM_JOIN_REQUESTED")
            }
            AddEntryResult::Already => JoinRequestOutcome::Refused("REALM_JOIN_REQUESTED"),
            AddEntryResult::AlreadyMember => JoinRequestOutcome::Member,
            AddEntryResult::Banned => JoinRequestOutcome::Refused("REALM_BANNED"),
            AddEntryResult::Full => JoinRequestOutcome::Refused("REALM_REQUESTS_FULL"),
            AddEntryResult::NotFound => JoinRequestOutcome::Refused("REALM_NOT_FOUND"),
        })
    }

    /// The account holds a player invite: joining answers it as Accept would (a ban and the cap still apply).
    async fn accept_player_invite_inner(
        &self,
        actor: &Actor,
        realm: &RealmRecord,
        account: &str,
    ) -> Result<Result<(), &'static str>, RealmsError> {
        let member = RealmMember {
            account_id: account.to_owned(),
            role: MemberRole::Member,
            joined_at: self.now(),
        };
        let added = self.repo.add_member(actor, &realm.realm_id, &member, self.limits.max_members).await?;
        match added {
            AddMemberResult::Banned => return Ok(Err("REALM_BANNED")),
            AddMemberResult::Full => return Ok(Err("REALM_FULL")),
            AddMemberResult::NotFound => return Ok(Err("REALM_NOT_FOUND")),
            AddMemberResult::Added | AddMemberResult::AlreadyMember => {}
        }
        // Already a member: add_member changed nothing, so its invite goes here
        if added == AddMemberResult::AlreadyMember {
            self.repo.remove_player_invite(actor, &realm.realm_id, account).await?;
        }
        self.repo.remove_join_request(actor, &realm.realm_id, account).await?;
        if added == AddMemberResult::Added {
            self.log.info(&format!("[Realms] player invite accepted realmId={}", realm.realm_id));
        }
        Ok(Ok(()))
    }

    fn has_player_invite(realm: &RealmRecord, account: &str) -> bool {
        realm.player_invites.iter().any(|i| i.account_id == account)
    }

    /// Without a name, the host's username; a name that is sent must not be another player's username.
    async fn realm_name_to_claim(&self, account: &str, requested: Option<&str>) -> Result<String, RealmsError> {
        let names = self.names_for(vec![account.to_owned()]).await;
        let own = names.get(account).and_then(|name| realm_name_for_username(name));
        let Some(requested) = requested else {
            return own.ok_or_else(|| {
                refused(
                    "REALM_USERNAME_REQUIRED",
                    "Set a username before you host a realm: your realm is named after it.",
                )
            });
        };
        if own.as_deref() != Some(requested) {
            let holder = self
                .accounts
                .resolve_username(requested.to_owned())
                .await
                .map_err(RealmsError::Database)?;
            if holder.is_some_and(|holder| holder != account) {
                return Err(refused(
                    "REALM_NAME_IS_USERNAME",
                    "That name is another player's username. A realm is named after its host.",
                ));
            }
        }
        Ok(requested.to_owned())
    }

    async fn resolve_username(&self, username: &str) -> Result<Option<String>, RealmsError> {
        self.accounts
            .resolve_username(username.to_owned())
            .await
            .map_err(RealmsError::Database)
    }

    // ------------------------------------------------------------------ the launcher's calls

    /// Claims a realm name for the caller (one realm per host account).
    pub async fn create_realm(&self, account: &str, input: &CreateRealmInput) -> Result<HostedReply, RealmsError> {
        self.throttle(Bucket::Create, account)?;
        let already = || refused("REALM_ALREADY_EXISTS", "You already host a realm. Release it before claiming another name.");
        // The host first: releasing a dormant name only to find the host can't use it would cost someone their name
        if self.repo.find_by_host(account).await?.is_some() {
            return Err(already());
        }
        let name = self.realm_name_to_claim(account, input.name.as_deref()).await?;
        let now = self.now();
        let realm_id = self.ids.realm_id();
        let realm = RealmRecord {
            hub_room_ids: hub_room_ids_of(&realm_id),
            realm_id,
            name,
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
        let actor = player(account);
        let mut result = self.repo.insert_realm(&actor, &realm).await?;
        if result == InsertRealmResult::NameTaken
            && let Some(holder) = self.repo.find_by_name(&realm.name).await?
            && is_realm_name_releasable(&holder, now)
            && self
                .repo
                .delete_realm_if_last_online(&actor, &holder.realm_id, holder.last_online_at)
                .await?
        {
            // A dormant realm frees its name: 3 days never online, 30 days after it was last online
            self.log.info(&format!("[Realms] name released realmId={} name={}", holder.realm_id, holder.name));
            self.drop_presence_of_realm(&actor, &holder.realm_id).await;
            result = self.repo.insert_realm(&actor, &realm).await?;
        }
        match result {
            InsertRealmResult::HostHasRealm => Err(already()),
            InsertRealmResult::NameTaken => Err(refused("REALM_NAME_TAKEN", "That realm name is taken.")),
            InsertRealmResult::Inserted => {
                self.log.info(&format!(
                    "[Realms] realm created realmId={} name={} policy={} listed={}",
                    realm.realm_id,
                    realm.name,
                    realm.join_policy.as_str(),
                    realm.listed
                ));
                Ok(HostedReply {
                    realm: self.host_view(&realm, account).await,
                })
            }
        }
    }

    /// The realm the caller hosts and the realms they have joined.
    pub async fn get_mine(&self, account: &str) -> Result<MineReply, RealmsError> {
        self.throttle(Bucket::Read, account)?;
        let hosted = self.repo.find_by_host(account).await?;
        let memberships = self.repo.find_by_member(account, MEMBER_LIST_LIMIT).await?;
        let others: Vec<RealmRecord> = memberships
            .into_iter()
            .filter(|realm| realm.host_account_id != account)
            .collect();
        Ok(MineReply {
            hosted: match hosted {
                Some(realm) => Some(self.host_view(&realm, account).await),
                None => None,
            },
            member_of: self.public_infos(&others, account, None).await,
        })
    }

    pub async fn get_by_name(
        &self,
        account: &str,
        name: &str,
        known_host_key: Option<&str>,
    ) -> Result<RealmReply, RealmsError> {
        self.throttle(Bucket::Read, account)?;
        let realm = self
            .repo
            .find_by_name(name)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "There is no realm with that name."))?;
        Ok(RealmReply {
            realm: self.public_info_one(&realm, account, known_host_key).await,
        })
    }

    /// What an invite code opens, without using it.
    pub async fn preview_invite(&self, account: &str, code: &str) -> Result<RealmReply, RealmsError> {
        self.throttle(Bucket::Join, account)?;
        let invalid = || refused("REALM_INVITE_INVALID", "That invite code is not valid.");
        let invite = self.repo.find_invite(code).await?.ok_or_else(invalid)?;
        if invite_state(&invite, self.now()) != InviteState::Usable {
            return Err(refused(
                "REALM_INVITE_EXPIRED",
                "That invite has expired, was revoked or has been used up.",
            ));
        }
        let realm = self.repo.find_by_id(&invite.realm_id).await?.ok_or_else(invalid)?;
        if realm.bans.iter().any(|ban| ban.account_id == account) {
            return Err(refused("REALM_BANNED", "You cannot join this realm."));
        }
        Ok(RealmReply {
            realm: self.public_info_one(&realm, account, None).await,
        })
    }

    /// A SOLO realm is not offered to others; its host still sees it.
    pub async fn list_listed(&self, account: &str, limit: u32, offset: u32) -> Result<RealmsReply, RealmsError> {
        self.throttle(Bucket::Read, account)?;
        let listed: Vec<RealmRecord> = self
            .repo
            .list_listed(limit, offset)
            .await?
            .into_iter()
            .filter(|realm| realm.join_policy != crate::realm::JoinPolicy::Solo || realm.host_account_id == account)
            .collect();
        Ok(RealmsReply {
            realms: self.public_infos(&listed, account, None).await,
        })
    }

    pub async fn update_settings(&self, account: &str, settings: &SettingsInput) -> Result<HostedReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        let actor = player(account);
        if !self
            .repo
            .set_settings(&actor, &realm.realm_id, account, settings.join_policy, settings.listed)
            .await?
        {
            return Err(not_hosted());
        }
        let updated = self.repo.find_by_id(&realm.realm_id).await?.ok_or_else(not_hosted)?;
        self.log.info(&format!(
            "[Realms] settings changed realmId={} policy={} listed={}",
            realm.realm_id,
            updated.join_policy.as_str(),
            updated.listed
        ));
        Ok(HostedReply {
            realm: self.host_view(&updated, account).await,
        })
    }

    pub async fn create_invite(&self, account: &str, input: &InviteInput) -> Result<InviteReply, RealmsError> {
        self.throttle(Bucket::Invite, account)?;
        let realm = self.require_hosted(account).await?;
        let now = self.now();
        if self.repo.count_active_invites(&realm.realm_id, now).await? >= i64::from(self.limits.max_active_invites) {
            return Err(refused(
                "REALM_INVITE_LIMIT",
                &format!(
                    "You can have at most {} open invites; revoke one first.",
                    self.limits.max_active_invites
                ),
            ));
        }
        // 50 random bits make a clash vanishingly rare; the retry is for the unique key
        for _ in 0..5 {
            let invite = build_invite(
                self.ids.invite_code(),
                &realm.realm_id,
                account,
                now,
                input.max_uses,
                input.ttl_hours,
            );
            if self.repo.insert_invite(&player(account), &invite).await? == InsertInviteResult::Inserted {
                self.log.info(&format!(
                    "[Realms] invite created realmId={} maxUses={} expiresAt={}",
                    realm.realm_id,
                    invite.max_uses,
                    iso(invite.expires_at)
                ));
                return Ok(InviteReply {
                    invite: self.invite_view(&invite),
                });
            }
        }
        Err(RealmsError::Database("Could not generate a unique invite code.".to_owned()))
    }

    pub async fn list_invites(&self, account: &str) -> Result<InvitesReply, RealmsError> {
        self.throttle(Bucket::Read, account)?;
        let realm = self.require_hosted(account).await?;
        let invites = self.repo.list_invites(&realm.realm_id, INVITE_LIST_LIMIT).await?;
        Ok(InvitesReply {
            invites: invites.iter().map(|invite| self.invite_view(invite)).collect(),
        })
    }

    pub async fn revoke_invite(&self, account: &str, code: &str) -> Result<InviteReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        let invite = self
            .repo
            .revoke_invite(&player(account), &realm.realm_id, code, self.now())
            .await?
            .ok_or_else(|| refused("REALM_INVITE_INVALID", "That invite code is not valid."))?;
        self.log.info(&format!("[Realms] invite revoked realmId={}", realm.realm_id));
        Ok(InviteReply {
            invite: self.invite_view(&invite),
        })
    }

    /// By name, by invite code, or both. Ownership first, so a launcher with the old host key hears of the new owner
    /// before anything else.
    pub async fn join(&self, account: &str, input: &JoinInput) -> Result<JoinReply, RealmsError> {
        self.throttle(Bucket::Join, account)?;
        let mut invite = None;
        if let Some(code) = &input.invite_code {
            invite = Some(
                self.repo
                    .find_invite(code)
                    .await?
                    .ok_or_else(|| refused("REALM_INVITE_INVALID", "That invite code is not valid."))?,
            );
        }
        let realm = match (&input.name, &invite) {
            (Some(name), _) => self.repo.find_by_name(name).await?,
            (None, Some(invite)) => self.repo.find_by_id(&invite.realm_id).await?,
            (None, None) => None,
        };
        let Some(mut realm) = realm else {
            return Err(entry_error("REALM_NOT_FOUND"));
        };
        if invite.as_ref().is_some_and(|invite| invite.realm_id != realm.realm_id) {
            return Err(entry_error("REALM_INVITE_INVALID"));
        }
        if let Some(key) = &input.known_host_key
            && detect_ownership_change(&realm.name, key, &realm)
        {
            return Err(entry_error("REALM_OWNER_CHANGED"));
        }
        let mut decision = decide_join(&realm, account, invite.as_ref(), self.now());
        if decision == Err(JoinDenial::InviteRequired) && Self::has_player_invite(&realm, account) {
            // The host invited this player by name: joining accepts the invite
            if let Err(code) = self.accept_player_invite_inner(&player(account), &realm, account).await? {
                return Err(entry_error(code));
            }
            let fresh = self.repo.find_by_id(&realm.realm_id).await?.unwrap_or(realm);
            return Ok(JoinReply {
                realm: self.public_info_one(&fresh, account, None).await,
                joined: true,
            });
        }
        if decision == Err(JoinDenial::InviteRequired) {
            // No invite on an invite-only realm: ask the host; approved meanwhile counts as a member's join
            if let JoinRequestOutcome::Refused(code) = self.request_join(&realm.realm_id, account).await? {
                return Err(entry_error(code));
            }
            realm = self.repo.find_by_id(&realm.realm_id).await?.unwrap_or(realm);
            decision = Ok(JoinVia::Member);
        }
        let via = decision.map_err(|denial| entry_error(denial.code()))?;
        if via == JoinVia::Member {
            return Ok(JoinReply {
                realm: self.public_info_one(&realm, account, None).await,
                joined: false,
            });
        }
        let added = self
            .admit_member(&realm, account, via, invite.as_ref())
            .await?
            .map_err(entry_error)?;
        let fresh = self.repo.find_by_id(&realm.realm_id).await?.unwrap_or(realm);
        Ok(JoinReply {
            realm: self.public_info_one(&fresh, account, None).await,
            joined: added,
        })
    }

    /// The realm part of Play: may this account enter now? The host enters its own realm even when not serving
    /// (that is how it starts it); the offline check comes before any invite use, so a refused Play costs nothing.
    pub async fn authorize_entry(&self, account: &str, input: &EntryRealmInput) -> Result<EntryAuthorization, RealmsError> {
        use EntryAuthorization::Refused;
        self.throttle(Bucket::Entry, account)?;
        let mut invite = None;
        if let Some(code) = &input.invite_code {
            match self.repo.find_invite(code).await? {
                Some(found) => invite = Some(found),
                None => return Ok(Refused("REALM_INVITE_INVALID")),
            }
        }
        let Some(realm) = self.repo.find_by_name(&input.name).await? else {
            return Ok(Refused("REALM_NOT_FOUND"));
        };
        if invite.as_ref().is_some_and(|invite| invite.realm_id != realm.realm_id) {
            return Ok(Refused("REALM_INVITE_INVALID"));
        }
        if let Some(key) = &input.known_host_key
            && detect_ownership_change(&realm.name, key, &realm)
        {
            return Ok(Refused("REALM_OWNER_CHANGED"));
        }
        let mut decision = decide_join(&realm, account, invite.as_ref(), self.now());
        if decision == Err(JoinDenial::InviteRequired) && Self::has_player_invite(&realm, account) {
            // Play with a pending player invite accepts it (the offline check below still applies)
            if let Err(code) = self.accept_player_invite_inner(&player(account), &realm, account).await? {
                return Ok(Refused(code));
            }
            decision = Ok(JoinVia::Member);
        }
        if decision == Err(JoinDenial::InviteRequired) {
            if let JoinRequestOutcome::Refused(code) = self.request_join(&realm.realm_id, account).await? {
                return Ok(Refused(code));
            }
            decision = Ok(JoinVia::Member);
        }
        let via = match decision {
            Ok(via) => via,
            Err(denial) => return Ok(Refused(denial.code())),
        };
        let is_host = realm.host_account_id == account;
        if !is_host && !is_realm_serving(&realm, self.now()) {
            return Ok(Refused("REALM_OFFLINE"));
        }
        if via != JoinVia::Member
            && let Err(code) = self.admit_member(&realm, account, via, invite.as_ref()).await?
        {
            return Ok(Refused(code));
        }
        Ok(EntryAuthorization::Ok {
            realm_id: realm.realm_id,
            is_host,
        })
    }

    pub async fn leave(&self, account: &str, name: &str) -> Result<LeftReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self
            .repo
            .find_by_name(name)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "There is no realm with that name."))?;
        if realm.host_account_id == account {
            return Err(refused(
                "REALM_HOST_CANNOT_LEAVE",
                "You host this realm. Release it instead of leaving it.",
            ));
        }
        let actor = player(account);
        if !self.repo.remove_member(&actor, &realm.realm_id, account).await? {
            return Err(refused("REALM_NOT_MEMBER", "You are not a member of this realm."));
        }
        self.log.info(&format!("[Realms] member left realmId={}", realm.realm_id));
        self.drop_presence_in_realm(&actor, account, &realm.realm_id).await;
        self.emit(RealmEvent::MemberRemoved {
            realm_id: realm.realm_id.clone(),
            account_id: account.to_owned(),
            reason: RemovalReason::Leave,
        })
        .await;
        Ok(LeftReply { left: true })
    }

    pub async fn kick(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        if target == account {
            return Err(refused("REALM_INVALID_TARGET", "You cannot remove yourself from your own realm."));
        }
        let actor = player(account);
        if !self.repo.remove_member(&actor, &realm.realm_id, target).await? {
            return Err(refused("REALM_NOT_MEMBER", "That account is not a member of your realm."));
        }
        self.log.info(&format!("[Realms] member kicked realmId={}", realm.realm_id));
        self.drop_presence_in_realm(&actor, target, &realm.realm_id).await;
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

    /// Bans an account (a member or not); repeating a ban changes nothing.
    pub async fn ban(&self, account: &str, target: &str, reason: Option<&str>) -> Result<ChangedReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        if target == account {
            return Err(refused("REALM_INVALID_TARGET", "You cannot ban yourself from your own realm."));
        }
        let actor = player(account);
        let ban = RealmBan {
            account_id: target.to_owned(),
            reason: reason.map(str::to_owned),
            at: self.now(),
        };
        let result = self.repo.add_ban(&actor, &realm.realm_id, &ban, self.limits.max_bans).await?;
        match result {
            AddBanResult::NotFound => return Err(not_hosted()),
            AddBanResult::Full => return Err(refused("REALM_FULL", "The ban list is full; lift a ban first.")),
            AddBanResult::Banned => {
                self.log.info(&format!("[Realms] account banned realmId={}", realm.realm_id));
                self.drop_presence_in_realm(&actor, target, &realm.realm_id).await;
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

    pub async fn unban(&self, account: &str, target: &str) -> Result<ChangedReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        let changed = self.repo.remove_ban(&player(account), &realm.realm_id, target).await?;
        if changed {
            self.log.info(&format!("[Realms] ban lifted realmId={}", realm.realm_id));
        }
        Ok(ChangedReply {
            account_id: target.to_owned(),
            status: "UNBANNED",
            changed,
        })
    }

    /// The players waiting for the host, newest request first.
    pub async fn list_join_requests(&self, account: &str) -> Result<RequestsReply, RealmsError> {
        self.throttle(Bucket::Read, account)?;
        let realm = self.require_hosted(account).await?;
        let requests = sort_newest_first(&realm.join_requests);
        let names = self.names_for(requests.iter().map(|r| r.account_id.clone()).collect()).await;
        Ok(RequestsReply {
            requests: requests
                .iter()
                .map(|r| AccountAtView {
                    account_id: r.account_id.clone(),
                    display_name: names.get(&r.account_id).cloned(),
                    at: iso(r.at),
                })
                .collect(),
        })
    }

    /// The membership goes through the same `add_member` as an invite join; the request stays when the cap refuses.
    pub async fn approve_join_request(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        if !realm.join_requests.iter().any(|r| r.account_id == target) {
            return Err(refused(
                "REALM_REQUEST_NOT_FOUND",
                "That player has not asked to join your realm.",
            ));
        }
        let actor = player(account);
        let member = RealmMember {
            account_id: target.to_owned(),
            role: MemberRole::Member,
            joined_at: self.now(),
        };
        match self.repo.add_member(&actor, &realm.realm_id, &member, self.limits.max_members).await? {
            AddMemberResult::NotFound => return Err(not_hosted()),
            AddMemberResult::Banned => {
                self.repo.remove_join_request(&actor, &realm.realm_id, target).await?;
                return Err(refused("REALM_BANNED", "That player is banned from your realm."));
            }
            AddMemberResult::Full => return Err(refused("REALM_FULL", "This realm cannot take more members.")),
            AddMemberResult::Added | AddMemberResult::AlreadyMember => {}
        }
        self.repo.remove_join_request(&actor, &realm.realm_id, target).await?;
        self.log.info(&format!("[Realms] join request approved realmId={}", realm.realm_id));
        Ok(StatusReply {
            account_id: target.to_owned(),
            status: "APPROVED",
        })
    }

    pub async fn deny_join_request(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        if !self.repo.remove_join_request(&player(account), &realm.realm_id, target).await? {
            return Err(refused(
                "REALM_REQUEST_NOT_FOUND",
                "That player has not asked to join your realm.",
            ));
        }
        self.log.info(&format!("[Realms] join request denied realmId={}", realm.realm_id));
        Ok(StatusReply {
            account_id: target.to_owned(),
            status: "DENIED",
        })
    }

    /// Launchers up to 0.1.68: the host lets a player in by username at once.
    pub async fn add_member_by_username(&self, account: &str, username: &str) -> Result<AddedReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        let trimmed = crate::realm::js_trim(username);
        let target = self
            .resolve_username(trimmed)
            .await?
            .ok_or_else(|| refused("REALM_PLAYER_NOT_FOUND", "No player has that username."))?;
        let actor = player(account);
        let member = RealmMember {
            account_id: target.clone(),
            role: MemberRole::Member,
            joined_at: self.now(),
        };
        let added = self.repo.add_member(&actor, &realm.realm_id, &member, self.limits.max_members).await?;
        match added {
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
        self.repo.remove_join_request(&actor, &realm.realm_id, &target).await?;
        if added == AddMemberResult::Added {
            self.log.info(&format!("[Realms] player added by the host realmId={}", realm.realm_id));
        }
        let display_name = self
            .names_for(vec![target.clone()])
            .await
            .remove(&target)
            .unwrap_or_else(|| trimmed.to_owned());
        Ok(AddedReply {
            account_id: target,
            display_name,
            added: added == AddMemberResult::Added,
        })
    }

    /// The host invites by username; a player who already asked is let in at once.
    pub async fn invite_player_by_username(&self, account: &str, username: &str) -> Result<InvitedReply, RealmsError> {
        self.throttle(Bucket::Invite, account)?;
        let realm = self.require_hosted(account).await?;
        let trimmed = crate::realm::js_trim(username);
        let target = self
            .resolve_username(trimmed)
            .await?
            .ok_or_else(|| refused("REALM_PLAYER_NOT_FOUND", "No player has that username."))?;
        if target == account {
            return Err(refused("REALM_INVALID_TARGET", "You are the host of this realm."));
        }
        let display_name = self
            .names_for(vec![target.clone()])
            .await
            .remove(&target)
            .unwrap_or_else(|| trimmed.to_owned());
        let actor = player(account);
        let reply = |status| InvitedReply {
            account_id: target.clone(),
            display_name: display_name.clone(),
            status,
        };
        let banned = || refused("REALM_BANNED", "That player is banned from your realm. Lift the ban first.");
        if realm.join_requests.iter().any(|r| r.account_id == target) {
            return match self.accept_player_invite_inner(&actor, &realm, &target).await? {
                Ok(()) => {
                    self.log.info(&format!(
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
        let entry = AccountAt {
            account_id: target.clone(),
            at: self.now(),
        };
        let result = self
            .repo
            .add_player_invite(&actor, &realm.realm_id, &entry, self.limits.max_player_invites)
            .await?;
        match result {
            AddEntryResult::NotFound => Err(not_hosted()),
            AddEntryResult::Banned => Err(banned()),
            AddEntryResult::Full => Err(refused(
                "REALM_PLAYER_INVITES_FULL",
                "Too many invites are waiting for an answer. Cancel one first.",
            )),
            AddEntryResult::AlreadyMember => Ok(reply("MEMBER")),
            AddEntryResult::Added | AddEntryResult::Already => {
                if result == AddEntryResult::Added {
                    self.log.info(&format!("[Realms] player invited realmId={}", realm.realm_id));
                }
                Ok(reply("INVITED"))
            }
        }
    }

    pub async fn cancel_player_invite(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        if !self.repo.remove_player_invite(&player(account), &realm.realm_id, target).await? {
            return Err(refused("REALM_PLAYER_INVITE_NOT_FOUND", "That player has no invite waiting."));
        }
        self.log.info(&format!("[Realms] player invite cancelled realmId={}", realm.realm_id));
        Ok(StatusReply {
            account_id: target.to_owned(),
            status: "CANCELLED",
        })
    }

    /// The invites the caller received and has not answered, newest first.
    pub async fn list_my_player_invites(&self, account: &str) -> Result<PlayerInvitesReply, RealmsError> {
        self.throttle(Bucket::Read, account)?;
        let realms = self.repo.find_by_player_invite(account, MEMBER_LIST_LIMIT).await?;
        let infos = self.public_infos(&realms, account, None).await;
        let mut invites: Vec<(UnixMillis, PlayerInviteView)> = realms
            .iter()
            .zip(infos)
            .map(|(realm, info)| {
                let at = realm
                    .player_invites
                    .iter()
                    .find(|i| i.account_id == account)
                    .map_or(realm.created_at, |i| i.at);
                (at, PlayerInviteView { realm: info, at: iso(at) })
            })
            .collect();
        invites.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        Ok(PlayerInvitesReply {
            invites: invites.into_iter().map(|(_, view)| view).collect(),
        })
    }

    pub async fn accept_player_invite(&self, account: &str, name: &str) -> Result<JoinReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self
            .repo
            .find_by_name(name)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "There is no realm with that name."))?;
        if !Self::has_player_invite(&realm, account) {
            return Err(refused("REALM_PLAYER_INVITE_NOT_FOUND", "You have no invite to this realm."));
        }
        let actor = player(account);
        if let Err(code) = self.accept_player_invite_inner(&actor, &realm, account).await? {
            if code == "REALM_BANNED" {
                self.repo.remove_player_invite(&actor, &realm.realm_id, account).await?;
            }
            return Err(entry_error(code));
        }
        let fresh = self.repo.find_by_id(&realm.realm_id).await?.unwrap_or(realm);
        Ok(JoinReply {
            realm: self.public_info_one(&fresh, account, None).await,
            joined: true,
        })
    }

    pub async fn decline_player_invite(&self, account: &str, name: &str) -> Result<DeclinedReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self
            .repo
            .find_by_name(name)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "There is no realm with that name."))?;
        if !self.repo.remove_player_invite(&player(account), &realm.realm_id, account).await? {
            return Err(refused("REALM_PLAYER_INVITE_NOT_FOUND", "You have no invite to this realm."));
        }
        self.log.info(&format!("[Realms] player invite declined realmId={}", realm.realm_id));
        Ok(DeclinedReply { declined: true })
    }

    /// Online for the next 90 s, or `online: false` closes it now; `hosting` replaces the host-chosen limits only.
    pub async fn heartbeat(&self, account: &str, input: &HeartbeatInput) -> Result<HeartbeatReply, RealmsError> {
        self.throttle(Bucket::Heartbeat, account)?;
        let realm = self.require_hosted(account).await?;
        let update = apply_heartbeat(self.now(), input.online, input.version.clone());
        let actor = player(account);
        if !self.repo.record_heartbeat(&actor, &realm.realm_id, account, &update).await? {
            return Err(not_hosted());
        }
        if let Some(hosting) = input.hosting {
            // The hub fields are the p2p side's and never self-reported
            let patch = HostingPatch {
                max_servers: Some(hosting.max_servers),
                max_players: Some(hosting.max_players),
                afk_seconds: hosting.afk_seconds.map_or(Patch::Remove, Patch::Set),
                ..HostingPatch::default()
            };
            self.repo.patch_hosting(&actor, &realm.realm_id, &patch, self.now()).await?;
        }
        Ok(HeartbeatReply {
            online: input.online != Some(false),
            last_online_at: iso(update.last_online_at),
            online_until: iso(update.online_until),
            ttl_seconds: HEARTBEAT_ONLINE_MS / 1000,
            version: update.version.or(realm.version),
        })
    }

    /// The host gives the realm up: its name is free at once, its members and invites gone.
    pub async fn release(&self, account: &str) -> Result<ReleasedReply, RealmsError> {
        self.throttle(Bucket::Write, account)?;
        let realm = self.require_hosted(account).await?;
        let actor = player(account);
        if !self.repo.delete_realm_of_host(&actor, &realm.realm_id, account).await? {
            return Err(not_hosted());
        }
        self.log.info(&format!("[Realms] realm released realmId={} name={}", realm.realm_id, realm.name));
        self.drop_presence_of_realm(&actor, &realm.realm_id).await;
        self.emit(RealmEvent::RealmReleased {
            realm_id: realm.realm_id.clone(),
            host_account_id: realm.host_account_id.clone(),
        })
        .await;
        Ok(ReleasedReply { released: true })
    }

    pub async fn presence_heartbeat(&self, account: &str) -> Result<PresenceView, RealmsError> {
        self.throttle(Bucket::Heartbeat, account)?;
        let presence = self.touch_presence(account).await?;
        self.presence_view(presence).await
    }

    /// The game exited. Idempotent.
    pub async fn presence_leave(&self, account: &str) -> Result<LeftReply, RealmsError> {
        self.throttle(Bucket::Heartbeat, account)?;
        self.leave_presence(account).await?;
        Ok(LeftReply { left: true })
    }

    pub async fn presence_status(&self, account: &str) -> Result<PresenceView, RealmsError> {
        self.throttle(Bucket::Read, account)?;
        let presence = self.presence_of(account).await?;
        self.presence_view(presence).await
    }

    // ------------------------------------------------------------------ for the core (`realms/entry.ts`)

    /// `AuthorizeGameSessionRealm`: the `realm` of `POST /launcher/v1/game-sessions`. None leaves Play as it is; one
    /// with the module off is refused (never played as official), else parsed and authorized; any refusal is thrown.
    pub async fn authorize_game_session_realm(
        &self,
        account: &str,
        realm: Option<&serde_json::Value>,
        module_enabled: bool,
    ) -> Result<Option<(String, bool)>, RealmsError> {
        let Some(raw) = realm.filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        if !module_enabled {
            return Err(refused("REALMS_DISABLED", "Realms are not available on this server."));
        }
        let input = crate::input::parse_entry_realm(raw).map_err(|failure| refused(failure.code, &failure.message))?;
        match self.authorize_entry(account, &input).await? {
            EntryAuthorization::Refused(code) => Err(entry_error(code)),
            EntryAuthorization::Ok { realm_id, is_host } => {
                self.log.info(&format!("[Realms] game session authorized realmId={realm_id} host={is_host}"));
                Ok(Some((realm_id, is_host)))
            }
        }
    }

    /// `BindRealmPresenceAtLogin`: the consumed code's realm (or none) becomes the account's presence. A failed reset
    /// is logged and the login goes on; a failed realm bind is the login's error.
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
                    self.log.warn(&format!("[Realms] could not reset realm presence at login: {error}"));
                }
                Ok(())
            }
            Some(realm) => self.bind_presence(account, Some(realm)).await,
        }
    }

    // ------------------------------------------------------------------ for the p2p side: no throttle

    /// Merges the hub state the p2p side knows; a malformed patch is a bug and refused.
    pub async fn set_hosting(&self, realm_id: &str, patch: &HostingPatch) -> Result<bool, RealmsError> {
        if let Some(field) = validate_hosting_patch(patch) {
            return Err(RealmsError::Database(format!("setHosting: invalid {field}")));
        }
        Ok(self.repo.patch_hosting(&P2P, realm_id, patch, self.now()).await?)
    }

    /// Ends the online state now; emits nothing, since the caller is the one who knows.
    pub async fn mark_offline(&self, realm_id: &str) -> Result<bool, RealmsError> {
        Ok(self.repo.mark_offline(&P2P, realm_id, self.now()).await?)
    }

    pub async fn find_by_hub_room_id(&self, room_id: &str) -> Result<Option<RealmRecord>, RealmsError> {
        match normalize_hub_room_id(room_id) {
            Some(id) => Ok(self.repo.find_by_hub_room_id(&id).await?),
            None => Ok(None),
        }
    }

    pub async fn get_realm(&self, realm_id: &str) -> Result<Option<RealmRecord>, RealmsError> {
        Ok(self.repo.find_by_id(realm_id).await?)
    }

    pub async fn get_realm_by_host(&self, host: &str) -> Result<Option<RealmRecord>, RealmsError> {
        Ok(self.repo.find_by_host(host).await?)
    }

    pub async fn is_member(&self, realm_id: &str, account: &str) -> Result<bool, RealmsError> {
        Ok(self
            .repo
            .find_by_id(realm_id)
            .await?
            .is_some_and(|realm| realm.members.iter().any(|m| m.account_id == account)))
    }

    pub async fn is_banned(&self, realm_id: &str, account: &str) -> Result<bool, RealmsError> {
        Ok(self
            .repo
            .find_by_id(realm_id)
            .await?
            .is_some_and(|realm| realm.bans.iter().any(|b| b.account_id == account)))
    }

    /// One sweep: each realm whose online state lapsed in the last 10 minutes, claimed once, gets HOST_OFFLINE.
    pub async fn sweep_lapsed_hosts(&self) -> Result<usize, RealmsError> {
        let now = self.now();
        let lapsed = self
            .repo
            .claim_lapsed_hosts(&Actor::System("realms-sweeper"), now, UnixMillis(now.0 - SWEEP_LOOKBACK_MS), SWEEP_BATCH)
            .await?;
        for realm in &lapsed {
            self.log.info(&format!("[Realms] host offline realmId={}", realm.realm_id));
            self.emit(RealmEvent::HostOffline {
                realm_id: realm.realm_id.clone(),
                host_account_id: realm.host_account_id.clone(),
            })
            .await;
        }
        Ok(lapsed.len())
    }
}
