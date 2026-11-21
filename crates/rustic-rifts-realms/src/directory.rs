// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The realm directory: every launcher realm operation, the entry check Play runs, realm presence and its cache, and
//! the calls the p2p side and the sweeper make.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rustic_rifts_seam::{AccountId, Actor, Refusal, UnixMillis};
use serde::Serialize;
use serde_json::Value;

use crate::RealmsError;
use crate::db::RealmsDb;
use crate::input::{CreateRealmInput, EntryRealmInput, HeartbeatInput, InviteInput, JoinInput, SettingsInput};
use crate::ports::{Accounts, Clock, IdSource, RateLimiter, RealmEventSink, RealmsLog};
use crate::realm::validate_hosting_patch;
use crate::realm::{AccountAt, HostingPatch, JoinDenial, JoinPolicy, JoinVia, MemberRole, Patch, RealmBan, RealmEvent};
use crate::realm::{InviteState, RealmInviteRecord, RealmLimits, RealmMember, RealmPresenceRecord, RealmRecord};
use crate::realm::{PRESENCE_CACHE_MS, PRESENCE_TTL_MS, RemovalReason, js_trim, normalize_hub_room_id};
use crate::realm::{
    apply_heartbeat, build_invite, decide_join, detect_ownership_change, hub_room_ids_of, invite_state,
};
use crate::realm::{is_realm_hub_ready, is_realm_name_releasable, is_realm_online, is_realm_serving};
use crate::realm::{presence_touch_needs_write, realm_name_for_username, realm_name_release_at, sort_newest_first};
use crate::repo::realms::{AddBanResult, AddEntryResult, AddMemberResult, InsertInviteResult, InsertRealmResult};
use crate::repo::realms::{RealmPresenceRepository, RealmsRepository};
use crate::time::{Later as _, iso};

/// What the realm directory runs on.
pub struct DirectoryParts {
    /// The realms database.
    pub db: RealmsDb,
    /// Display names and account lookups.
    pub accounts: Arc<dyn Accounts>,
    /// Where realm events go; none are sent when absent.
    pub events: Option<Arc<dyn RealmEventSink>>,
    /// The time now.
    pub clock: Arc<dyn Clock>,
    /// Per-account rate limits on directory calls.
    pub limiter: Arc<dyn RateLimiter>,
    /// Ids and random codes.
    pub ids: Arc<dyn IdSource>,
    /// How many of each thing a realm may hold.
    pub limits: RealmLimits,
    /// The directory's log.
    pub log: Arc<dyn RealmsLog>,
}

/// Past this many cached answers the stale ones go, and then all of them if that was not enough.
const PRESENCE_CACHE_LIMIT: usize = 10_000;
/// Past this many per-account counters they reset under a new clear epoch.
const EPOCH_LIMIT: usize = 50_000;
const LIST_LIMIT: u32 = 100;
/// Tries at an unused invite code.
const INVITE_ATTEMPTS: usize = 5;
/// How far back the lapsed-host sweep looks for realms that went offline, in milliseconds.
pub const SWEEP_LOOKBACK_MS: i64 = 600_000;
const SWEEP_LIMIT: u32 = 100;
const HEARTBEAT_TTL_SECONDS: i64 = 90;

const P2P_ACTOR: Actor = Actor::System("realms-p2p");
const SWEEPER_ACTOR: Actor = Actor::System("realms-sweeper");

/// A rate-limit bucket: (name, requests, per window in ms).
#[derive(Clone, Copy, Debug)]
struct Bucket(&'static str, u32, i64);

const MINUTE: i64 = 60_000;
const READ: Bucket = Bucket("read", 240, MINUTE);
const WRITE: Bucket = Bucket("write", 60, MINUTE);
const HEARTBEAT: Bucket = Bucket("heartbeat", 30, MINUTE);
const CREATE: Bucket = Bucket("create", 5, 60 * MINUTE);
const INVITE: Bucket = Bucket("invite", 30, 60 * MINUTE);
const JOIN: Bucket = Bucket("join", 30, 10 * MINUTE);
const ENTRY: Bucket = Bucket("entry", 30, 10 * MINUTE);

/// A refusal's (code, text).
type Says = (&'static str, &'static str);

const RATE_LIMITED: Says = (
    "AUTH_RATE_LIMITED",
    "Too many realm requests. Please wait a moment and try again.",
);
const NOT_HOSTED: Says = ("REALM_NOT_FOUND", "You do not host a realm.");
const INVITE_NOT_VALID: Says = ("REALM_INVITE_INVALID", "That invite code is not valid.");
const ALREADY_HOSTS: Says = (
    "REALM_ALREADY_EXISTS",
    "You already host a realm. Release it before claiming another name.",
);
const USERNAME_REQUIRED: Says = (
    "REALM_USERNAME_REQUIRED",
    "Set a username before you host a realm: your realm is named after it.",
);
const NAME_IS_USERNAME: Says = (
    "REALM_NAME_IS_USERNAME",
    "That name is another player's username. A realm is named after its host.",
);
const NAME_TAKEN: Says = ("REALM_NAME_TAKEN", "That realm name is taken.");
const HOST_CANNOT_LEAVE: Says = (
    "REALM_HOST_CANNOT_LEAVE",
    "You host this realm. Release it instead of leaving it.",
);
const NOT_MY_REALM: Says = ("REALM_NOT_MEMBER", "You are not a member of this realm.");
const NOT_YOUR_MEMBER: Says = ("REALM_NOT_MEMBER", "That account is not a member of your realm.");
const KICK_SELF: Says = (
    "REALM_INVALID_TARGET",
    "You cannot remove yourself from your own realm.",
);
const BAN_SELF: Says = ("REALM_INVALID_TARGET", "You cannot ban yourself from your own realm.");
const INVITE_SELF: Says = ("REALM_INVALID_TARGET", "You are the host of this realm.");
const BANS_FULL: Says = ("REALM_FULL", "The ban list is full; lift a ban first.");
const NO_REQUEST: Says = (
    "REALM_REQUEST_NOT_FOUND",
    "That player has not asked to join your realm.",
);
const TARGET_BANNED: Says = ("REALM_BANNED", "That player is banned from your realm.");
const TARGET_BANNED_LIFT: Says = (
    "REALM_BANNED",
    "That player is banned from your realm. Lift the ban first.",
);
const NO_PLAYER: Says = ("REALM_PLAYER_NOT_FOUND", "No player has that username.");
const PLAYER_INVITES_FULL: Says = (
    "REALM_PLAYER_INVITES_FULL",
    "Too many invites are waiting for an answer. Cancel one first.",
);
const NO_WAITING_INVITE: Says = ("REALM_PLAYER_INVITE_NOT_FOUND", "That player has no invite waiting.");
const NO_INVITE_HERE: Says = ("REALM_PLAYER_INVITE_NOT_FOUND", "You have no invite to this realm.");
const REALMS_DISABLED: Says = ("REALMS_DISABLED", "Realms are not available on this server.");

/// The messages of the refusals the join route and Play share.
const ENTRY_MESSAGES: [Says; 11] = [
    ("REALM_NOT_FOUND", "There is no realm with that name."),
    ("REALM_OWNER_CHANGED", "This realm now belongs to a different host."),
    ("REALM_BANNED", "You cannot join this realm."),
    (
        "REALM_INVITE_REQUIRED",
        "This realm is invite-only. You need an invite code.",
    ),
    ("REALM_INVITE_INVALID", "That invite code is not valid for this realm."),
    (
        "REALM_INVITE_EXPIRED",
        "That invite has expired, was revoked or has been used up.",
    ),
    ("REALM_FULL", "This realm cannot take more members."),
    (
        "REALM_OFFLINE",
        "This realm is offline: its host is not running it right now.",
    ),
    (
        "REALM_SOLO",
        "This realm is in solo mode: its host is playing alone right now.",
    ),
    (
        "REALM_JOIN_REQUESTED",
        "Request sent to the host. Wait for their approval, then press Play again.",
    ),
    (
        "REALM_REQUESTS_FULL",
        "This realm has too many join requests waiting. Try again later.",
    ),
];

/// The refusals the join route and Play share; empty for any other code.
pub fn entry_message(code: &str) -> &'static str {
    ENTRY_MESSAGES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or("", |&(_, text)| text)
}

fn refused((code, message): Says) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.into(),
        message: message.into(),
    })
}

fn entry_error(code: &str) -> RealmsError {
    let message = entry_message(code);
    RealmsError::Refused(Refusal {
        code: code.into(),
        message: message.into(),
    })
}

/// `Err` with `says` unless `done`.
fn or_refuse(done: bool, says: Says) -> Result<(), RealmsError> {
    if done { Ok(()) } else { Err(refused(says)) }
}

fn as_account(account: &str) -> Actor {
    Actor::Account(AccountId(account.into()))
}

fn joined_now(account: &str, role: MemberRole, at: UnixMillis) -> RealmMember {
    RealmMember {
        account_id: account.into(),
        role,
        joined_at: at,
    }
}

/// The write the join paths share failed; anything else is an entry refusal.
fn internal(code: &'static str) -> RealmsError {
    match code {
        "INTERNAL" => RealmsError::Database(String::from("realm directory write failed")),
        refusal => entry_error(refusal),
    }
}

fn status(target: &str, status: &'static str) -> StatusReply {
    StatusReply {
        account_id: target.into(),
        status,
    }
}

fn changed(target: &str, status: &'static str, changed: bool) -> ChangedReply {
    ChangedReply {
        account_id: target.into(),
        status,
        changed,
    }
}

/// A realm host's name as a viewer sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostName {
    /// The host's display name, if it has one.
    pub display_name: Option<String>,
}

/// A realm as any player sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmPublicInfo {
    /// The realm's id.
    pub realm_id: String,
    /// The realm's name.
    pub name: String,
    /// The host.
    pub host: HostName,
    /// The host's public key.
    pub host_public_key: String,
    /// Who may join, as its wire name.
    pub join_policy: &'static str,
    /// Whether the realm is listed publicly.
    pub listed: bool,
    /// Whether the host is online.
    pub online: bool,
    /// Whether the realm serves this viewer: hub running, host online, and not another host's solo realm.
    pub serving: bool,
    /// Whether the realm's Ramsgate hub is running.
    pub hub_ready: bool,
    /// Players present in the realm now.
    pub player_count: i64,
    /// The host's reported version.
    pub version: Option<String>,
    /// Members, the host included.
    pub member_count: usize,
    /// The viewer's role in the realm, if a member.
    pub membership: Option<&'static str>,
    /// Whether the viewer asked to join.
    pub requested: bool,
    /// Whether the host invited the viewer.
    pub invited: bool,
    /// Whether someone else now holds the name, when the viewer gave the host key it knew.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ownership_changed: Option<bool>,
}

/// A realm as its host sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmHostView {
    /// What any player sees.
    #[serde(flatten)]
    pub info: RealmPublicInfo,
    /// When the realm was made, as ISO 8601.
    pub created_at: String,
    /// When the host was last online, as ISO 8601.
    pub last_online_at: Option<String>,
    /// Until when the realm counts as online, as ISO 8601.
    pub online_until: Option<String>,
    /// When the realm's name may be released, as ISO 8601.
    pub name_release_at: String,
    /// The members.
    pub members: Vec<MemberView>,
    /// The bans.
    pub bans: Vec<BanView>,
    /// Pending join requests.
    pub join_request_count: usize,
    /// Players the host invited, newest first.
    pub player_invites: Vec<AccountAtView>,
}

/// A realm member as the host sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    /// The member's account.
    pub account_id: String,
    /// The member's display name, if it has one.
    pub display_name: Option<String>,
    /// Host or member, as its wire name.
    pub role: &'static str,
    /// When they joined, as ISO 8601.
    pub joined_at: String,
}

/// A join request or a player invite as the host sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountAtView {
    /// The account.
    pub account_id: String,
    /// Its display name, if it has one.
    pub display_name: Option<String>,
    /// When it was made, as ISO 8601.
    pub at: String,
}

/// A ban as the host sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BanView {
    /// The banned account.
    pub account_id: String,
    /// Its display name, if it has one.
    pub display_name: Option<String>,
    /// The host's reason, if given.
    pub reason: Option<String>,
    /// When the ban was made, as ISO 8601.
    pub at: String,
}

/// An invite code as the host sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteView {
    /// The code.
    pub code: String,
    /// When it was made, as ISO 8601.
    pub created_at: String,
    /// When it expires, as ISO 8601.
    pub expires_at: String,
    /// How many joins it allows.
    pub max_uses: i64,
    /// How many joins it gave.
    pub uses: i64,
    /// When it was revoked, as ISO 8601.
    pub revoked_at: Option<String>,
    /// Whether it can still be used, as an [`InviteState`] name.
    pub state: &'static str,
}

/// The realm an account's login is bound to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceView {
    /// The realm, or `None` for no realm.
    pub realm: Option<PresenceRealm>,
    /// When the binding lapses, as ISO 8601.
    pub expires_at: Option<String>,
}

/// The realm a presence is bound to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceRealm {
    /// The realm's name.
    pub name: String,
    /// Whether the realm serves the account.
    pub serving: bool,
    /// Whether the realm's Ramsgate hub is running.
    pub hub_ready: bool,
    /// Whether the host is online.
    pub host_online: bool,
}

/// The realms an account hosts and belongs to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MineReply {
    /// The realm it hosts, if any.
    pub hosted: Option<RealmHostView>,
    /// The realms it is a member of, its own excluded.
    pub member_of: Vec<RealmPublicInfo>,
}

/// A host's own realm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HostedReply {
    /// The realm.
    pub realm: RealmHostView,
}

/// One realm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RealmReply {
    /// The realm.
    pub realm: RealmPublicInfo,
}

/// The answer to a join.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JoinReply {
    /// The realm.
    pub realm: RealmPublicInfo,
    /// Whether the account was added; false when it was already a member or only asked to join.
    pub joined: bool,
}

/// A list of realms.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RealmsReply {
    /// The realms.
    pub realms: Vec<RealmPublicInfo>,
}

/// One invite code.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InviteReply {
    /// The invite.
    pub invite: InviteView,
}

/// A realm's invite codes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InvitesReply {
    /// The invites, newest first.
    pub invites: Vec<InviteView>,
}

/// A realm that invited the player.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlayerInviteEntry {
    /// The realm.
    pub realm: RealmPublicInfo,
    /// When the invite came, as ISO 8601.
    pub at: String,
}

/// The realms that invited the player.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlayerInvitesReply {
    /// The invites, newest first.
    pub invites: Vec<PlayerInviteEntry>,
}

/// A realm's pending join requests.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RequestsReply {
    /// The requests, newest first.
    pub requests: Vec<AccountAtView>,
}

/// What a host's action did to an account.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusReply {
    /// The account acted on.
    pub account_id: String,
    /// What it did, such as `KICKED` or `CANCELLED`.
    pub status: &'static str,
}

/// What a host's action did to an account, and whether anything changed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedReply {
    /// The account acted on.
    pub account_id: String,
    /// What it did, such as `BANNED` or `UNBANNED`.
    pub status: &'static str,
    /// Whether anything changed.
    pub changed: bool,
}

/// A player added to a realm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedReply {
    /// The account.
    pub account_id: String,
    /// Its display name.
    pub display_name: String,
    /// Whether it was added; false when it was already a member.
    pub added: bool,
}

/// A player the host invited.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitedReply {
    /// The account.
    pub account_id: String,
    /// Its display name.
    pub display_name: String,
    /// `INVITED`, or `MEMBER` when the player is in the realm already.
    pub status: &'static str,
}

/// The answer to leaving.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LeftReply {
    /// Always true.
    pub left: bool,
}

/// The answer to declining an invite.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeclinedReply {
    /// Always true.
    pub declined: bool,
}

/// The answer to releasing a realm.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReleasedReply {
    /// Always true.
    pub released: bool,
}

/// The answer to a host heartbeat.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatReply {
    /// Whether the realm is online after it.
    pub online: bool,
    /// When the host was last online, as ISO 8601.
    pub last_online_at: String,
    /// Until when the realm counts as online, as ISO 8601.
    pub online_until: String,
    /// How long one heartbeat keeps the realm online, in seconds.
    pub ttl_seconds: i64,
    /// The host's reported version.
    pub version: Option<String>,
}

/// Whether an account may enter the realm it asked for when it plays.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryAuthorization {
    /// Allowed.
    Ok {
        /// The realm entered.
        realm_id: String,
        /// Whether the account hosts it.
        is_host: bool,
    },
    /// Refused with this code.
    Refused(&'static str),
}

/// No realm presence to show.
const NOWHERE: PresenceView = PresenceView {
    realm: None,
    expires_at: None,
};

/// (clear epoch, the account's counter): a cached answer is written back only if neither moved since the read.
type Epoch = (u64, u64);

/// Presence answers, `None` included, keyed by account with the time they were read.
#[derive(Default)]
struct PresenceCache {
    clear_epoch: u64,
    counters: HashMap<String, u64>,
    entries: HashMap<String, (UnixMillis, Option<RealmPresenceRecord>)>,
}

impl PresenceCache {
    fn epoch(&self, account: &str) -> Epoch {
        let counter = self.counters.get(account).map_or(0, |n| *n);
        (self.clear_epoch, counter)
    }

    fn fresh(&self, account: &str, now: UnixMillis) -> Option<&Option<RealmPresenceRecord>> {
        let (read_at, row) = self.entries.get(account)?;
        (0..PRESENCE_CACHE_MS).contains(&(now.0 - read_at.0)).then_some(row)
    }

    fn store(&mut self, account: &str, now: UnixMillis, row: Option<RealmPresenceRecord>) {
        if self.entries.len() >= PRESENCE_CACHE_LIMIT {
            self.entries.retain(|_, entry| now.0 - entry.0.0 < PRESENCE_CACHE_MS);
        }
        if self.entries.len() >= PRESENCE_CACHE_LIMIT {
            self.entries.clear();
        }
        self.entries.insert(account.into(), (now, row));
    }

    fn invalidate(&mut self, account: &str) {
        self.entries.remove(account);
        *self.counters.entry(account.into()).or_default() += 1;
        if self.counters.len() > EPOCH_LIMIT {
            self.clear_epoch += 1;
            self.counters.clear();
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.counters.clear();
        self.clear_epoch += 1;
    }
}

enum Admitted {
    Added,
    AlreadyMember,
}

/// Realms, their members and invites, and where each player's login is bound.
pub struct RealmDirectory {
    parts: DirectoryParts,
    realms: RealmsRepository,
    presence: RealmPresenceRepository,
    cache: Mutex<PresenceCache>,
    presence_reads: AtomicU64,
    presence_touches: AtomicU64,
}

/// What every public view of a realm is built from.
struct ViewContext<'a> {
    viewer: &'a str,
    names: &'a BTreeMap<String, String>,
    counts: &'a BTreeMap<String, i64>,
    known_key: Option<&'a str>,
    now: UnixMillis,
}

impl ViewContext<'_> {
    fn name(&self, account: &str) -> Option<String> {
        self.names.get(account).cloned()
    }

    fn info(&self, realm: &RealmRecord) -> RealmPublicInfo {
        let viewer = self.viewer;
        // A SOLO realm serves only its host.
        let serves_viewer = realm.join_policy != JoinPolicy::Solo || realm.host_account_id == viewer;
        let membership = realm
            .members
            .iter()
            .find(|m| m.account_id == viewer)
            .map(|m| m.role.as_str());
        RealmPublicInfo {
            host: HostName {
                display_name: self.name(&realm.host_account_id),
            },
            serving: serves_viewer && is_realm_serving(realm, self.now),
            online: is_realm_online(realm, self.now),
            hub_ready: is_realm_hub_ready(realm),
            player_count: self.counts.get(&realm.realm_id).map_or(0, |n| *n),
            member_count: realm.members.len(),
            membership,
            requested: realm.join_requests.iter().any(|request| request.account_id == viewer),
            invited: realm.player_invites.iter().any(|invite| invite.account_id == viewer),
            ownership_changed: self
                .known_key
                .map(|key| detect_ownership_change(&realm.name, key, realm)),
            join_policy: realm.join_policy.as_str(),
            realm_id: realm.realm_id.clone(),
            name: realm.name.clone(),
            host_public_key: realm.host_public_key.clone(),
            listed: realm.listed,
            version: realm.version.clone(),
        }
    }

    fn account_at(&self, entry: AccountAt) -> AccountAtView {
        AccountAtView {
            display_name: self.name(&entry.account_id),
            at: iso(entry.at),
            account_id: entry.account_id,
        }
    }
}

fn invite_view(invite: &RealmInviteRecord, now: UnixMillis) -> InviteView {
    InviteView {
        state: invite_state(invite, now).as_str(),
        revoked_at: invite.revoked_at.map(iso),
        expires_at: iso(invite.expires_at),
        created_at: iso(invite.created_at),
        code: invite.code.clone(),
        max_uses: invite.max_uses,
        uses: invite.uses,
    }
}

impl RealmDirectory {
    /// A directory over these parts.
    pub fn new(parts: DirectoryParts) -> Self {
        let realms = RealmsRepository::new(parts.db.clone());
        let presence = RealmPresenceRepository::new(parts.db.clone());
        Self {
            cache: Mutex::default(),
            presence_reads: AtomicU64::default(),
            presence_touches: AtomicU64::default(),
            realms,
            presence,
            parts,
        }
    }

    fn now(&self) -> UnixMillis {
        self.parts.clock.now()
    }

    fn cache(&self) -> MutexGuard<'_, PresenceCache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn note(&self, line: &str) {
        self.parts.log.info(line);
    }

    fn caution(&self, line: &str) {
        self.parts.log.warn(line);
    }

    /// (presence rows read from the database, touches written).
    pub fn presence_io(&self) -> (u64, u64) {
        let (reads, touches) = (&self.presence_reads, &self.presence_touches);
        (reads.load(SeqCst), touches.load(SeqCst))
    }

    /// One request from the account's budget in `bucket`.
    fn spend(&self, account: &str, Bucket(name, max, window_ms): Bucket) -> Result<(), RealmsError> {
        let limited = self
            .parts
            .limiter
            .is_limited(&format!("realms:{name}:{account}"), max, window_ms);
        or_refuse(!limited, RATE_LIMITED)
    }

    /// The account's own realm, after its rate limit.
    async fn own_realm(&self, bucket: Bucket, account: &str) -> Result<RealmRecord, RealmsError> {
        self.spend(account, bucket)?;
        self.hosted(account).await
    }

    async fn named_realm(&self, name: &str) -> Result<RealmRecord, RealmsError> {
        self.realms
            .find_by_name(name)
            .await?
            .ok_or_else(|| entry_error("REALM_NOT_FOUND"))
    }

    /// A failure is the sink's to log, never the request's.
    async fn emit(&self, event: RealmEvent) {
        let Some(sink) = self.parts.events.as_ref() else {
            return;
        };
        if let Err(error) = sink.emit(event).await {
            self.caution(&format!("[Realms] event sink failed: {error}"));
        }
    }

    async fn member_removed(&self, realm_id: &str, account: &str, reason: RemovalReason) {
        self.drop_presence_in_realm(account, realm_id).await;
        let event = RealmEvent::MemberRemoved {
            realm_id: realm_id.into(),
            account_id: account.into(),
            reason,
        };
        self.emit(event).await;
    }

    /// Best effort: an account the lookup cannot name has no display name.
    async fn names(&self, ids: Vec<String>) -> BTreeMap<String, String> {
        self.parts.accounts.display_names(ids).await.unwrap_or_else(|error| {
            self.caution(&format!("[Realms] display-name lookup failed: {error}"));
            BTreeMap::new()
        })
    }

    async fn player_counts(&self, realm_ids: &[String], now: UnixMillis) -> BTreeMap<String, i64> {
        self.presence
            .count_active_by_realm(realm_ids, now)
            .await
            .unwrap_or_else(|error| {
                self.caution(&format!("[Realms] player-count lookup failed: {error}"));
                BTreeMap::new()
            })
    }

    async fn public_infos(&self, realms: &[RealmRecord], viewer: &str) -> Vec<RealmPublicInfo> {
        if realms.is_empty() {
            return Vec::new();
        }
        let now = self.now();
        let (hosts, ids): (Vec<String>, Vec<String>) = realms
            .iter()
            .map(|realm| (realm.host_account_id.clone(), realm.realm_id.clone()))
            .unzip();
        let names = self.names(hosts).await;
        let counts = self.player_counts(&ids, now).await;
        let context = ViewContext {
            viewer,
            names: &names,
            counts: &counts,
            known_key: None,
            now,
        };
        realms.iter().map(|realm| context.info(realm)).collect()
    }

    async fn public_info(&self, realm: &RealmRecord, viewer: &str, known_key: Option<&str>) -> RealmPublicInfo {
        let now = self.now();
        let names = self.names(vec![realm.host_account_id.clone()]).await;
        let counts = self.player_counts(std::slice::from_ref(&realm.realm_id), now).await;
        ViewContext {
            viewer,
            names: &names,
            counts: &counts,
            known_key,
            now,
        }
        .info(realm)
    }

    async fn host_view(&self, realm: &RealmRecord) -> RealmHostView {
        let now = self.now();
        let everyone: BTreeSet<String> = std::iter::once(&realm.host_account_id)
            .chain(realm.members.iter().map(|m| &m.account_id))
            .chain(realm.bans.iter().map(|b| &b.account_id))
            .chain(realm.player_invites.iter().map(|i| &i.account_id))
            .cloned()
            .collect();
        let names = self.names(everyone.into_iter().collect()).await;
        let counts = self.player_counts(std::slice::from_ref(&realm.realm_id), now).await;
        let context = ViewContext {
            viewer: &realm.host_account_id,
            names: &names,
            counts: &counts,
            known_key: None,
            now,
        };
        let members = realm.members.iter().map(|m| MemberView {
            display_name: context.name(&m.account_id),
            account_id: m.account_id.clone(),
            joined_at: iso(m.joined_at),
            role: m.role.as_str(),
        });
        let bans = realm.bans.iter().map(|b| BanView {
            display_name: context.name(&b.account_id),
            account_id: b.account_id.clone(),
            reason: b.reason.clone(),
            at: iso(b.at),
        });
        RealmHostView {
            info: context.info(realm),
            name_release_at: iso(realm_name_release_at(realm.created_at, realm.last_online_at)),
            created_at: iso(realm.created_at),
            last_online_at: realm.last_online_at.map(iso),
            online_until: realm.online_until.map(iso),
            join_request_count: realm.join_requests.len(),
            player_invites: sort_newest_first(&realm.player_invites)
                .into_iter()
                .map(|i| context.account_at(i))
                .collect(),
            members: members.collect(),
            bans: bans.collect(),
        }
    }

    async fn hosted(&self, account: &str) -> Result<RealmRecord, RealmsError> {
        let found = self.realms.find_by_host(account).await?;
        found.ok_or_else(|| refused(NOT_HOSTED))
    }

    async fn reload(&self, realm_id: &str) -> Result<RealmRecord, RealmsError> {
        let found = self.realms.find_by_id(realm_id).await?;
        found.ok_or_else(|| entry_error("REALM_NOT_FOUND"))
    }

    async fn drop_presence_in_realm(&self, account: &str, realm_id: &str) {
        let removed = self
            .presence
            .remove(&as_account(account), account, Some(realm_id))
            .await;
        if let Err(error) = removed {
            self.caution(&format!(
                "[Realms] could not remove presence realmId={realm_id}: {error}"
            ));
        }
        self.cache().invalidate(account);
    }

    async fn drop_realm_presence(&self, actor: &Actor, realm_id: &str) {
        let removed = self.presence.remove_for_realm(actor, realm_id).await;
        if let Err(error) = removed {
            self.caution(&format!(
                "[Realms] could not remove presence of realmId={realm_id}: {error}"
            ));
        }
        self.cache().clear();
    }

    async fn add_as_member(
        &self,
        actor: &Actor,
        realm_id: &str,
        account: &str,
    ) -> Result<AddMemberResult, RealmsError> {
        let member = joined_now(account, MemberRole::Member, self.now());
        Ok(self
            .realms
            .add_member(actor, realm_id, &member, self.parts.limits.max_members)
            .await?)
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
        let used = match invite.filter(|_| via == JoinVia::Invite) {
            None => None,
            Some(invite) => {
                let consumed = self.realms.consume_invite(&actor, &invite.code, self.now()).await;
                if consumed.map_err(|_| "INTERNAL")?.is_none() {
                    return Err("REALM_INVITE_EXPIRED");
                }
                Some(invite.code.as_str())
            }
        };
        let outcome = self.add_as_member(&actor, &realm.realm_id, account).await;
        if let Some(code) = used
            && !matches!(outcome, Ok(AddMemberResult::Added))
        {
            let _ = self.realms.refund_invite(&actor, code).await;
        }
        match outcome.map_err(|_| "INTERNAL")? {
            AddMemberResult::Added => {
                let source = if used.is_some() { "INVITE" } else { "PUBLIC" };
                self.note(&format!(
                    "[Realms] member joined realmId={} via={source}",
                    realm.realm_id
                ));
                Ok(Admitted::Added)
            }
            AddMemberResult::AlreadyMember => Ok(Admitted::AlreadyMember),
            AddMemberResult::NotFound => Err("REALM_NOT_FOUND"),
            AddMemberResult::Full => Err("REALM_FULL"),
            AddMemberResult::Banned => Err("REALM_BANNED"),
        }
    }

    /// `Ok(true)` when the account turned out to be a member already.
    async fn request(&self, account: &str, realm: &RealmRecord) -> Result<bool, &'static str> {
        let entry = AccountAt {
            account_id: account.into(),
            at: self.now(),
        };
        let cap = self.parts.limits.max_join_requests;
        let added = self
            .realms
            .add_join_request(&as_account(account), &realm.realm_id, &entry, cap)
            .await;
        let code = match added.map_err(|_| "INTERNAL")? {
            AddEntryResult::AlreadyMember => return Ok(true),
            AddEntryResult::Added => {
                self.note(&format!("[Realms] join requested realmId={}", realm.realm_id));
                "REALM_JOIN_REQUESTED"
            }
            AddEntryResult::Already => "REALM_JOIN_REQUESTED",
            AddEntryResult::Full => "REALM_REQUESTS_FULL",
            AddEntryResult::Banned => "REALM_BANNED",
            AddEntryResult::NotFound => "REALM_NOT_FOUND",
        };
        Err(code)
    }

    async fn accept_invite_for(&self, account: &str, realm_id: &str) -> Result<Admitted, &'static str> {
        let actor = as_account(account);
        let admitted = match self
            .add_as_member(&actor, realm_id, account)
            .await
            .map_err(|_| "INTERNAL")?
        {
            // An `Added` drops the player invite in the same write.
            AddMemberResult::Added => Admitted::Added,
            AddMemberResult::AlreadyMember => {
                let _ = self.realms.remove_player_invite(&actor, realm_id, account).await;
                Admitted::AlreadyMember
            }
            AddMemberResult::NotFound => return Err("REALM_NOT_FOUND"),
            AddMemberResult::Full => return Err("REALM_FULL"),
            AddMemberResult::Banned => return Err("REALM_BANNED"),
        };
        let _ = self.realms.remove_join_request(&actor, realm_id, account).await;
        if let Admitted::Added = admitted {
            self.note(&format!("[Realms] player invite accepted realmId={realm_id}"));
        }
        Ok(admitted)
    }

    /// The name the account may claim: its own username's realm name, or another that is nobody else's username.
    async fn claimable_name(&self, account: &str, asked: Option<&String>) -> Result<String, RealmsError> {
        let own_name = self.names(vec![account.to_owned()]).await.remove(account);
        let own = own_name.as_deref().and_then(realm_name_for_username);
        let Some(asked) = asked else {
            return own.ok_or_else(|| refused(USERNAME_REQUIRED));
        };
        if own.as_deref() != Some(asked.as_str()) {
            let holder = self
                .parts
                .accounts
                .resolve_username(asked.clone())
                .await
                .map_err(RealmsError::Database)?;
            if holder.is_some_and(|holder| holder != account) {
                return Err(refused(NAME_IS_USERNAME));
            }
        }
        Ok(asked.clone())
    }

    /// Makes the account's realm; refused when it already hosts one, the name is taken or invalid, or it is rate-limited.
    pub async fn create_realm(&self, account: &str, input: &CreateRealmInput) -> Result<HostedReply, RealmsError> {
        self.spend(account, CREATE)?;
        if self.realms.find_by_host(account).await?.is_some() {
            return Err(refused(ALREADY_HOSTS));
        }
        let name = self.claimable_name(account, input.name.as_ref()).await?;
        let now = self.now();
        let realm_id = self.parts.ids.realm_id();
        let record = RealmRecord {
            hub_room_ids: hub_room_ids_of(&realm_id),
            members: vec![joined_now(account, MemberRole::Host, now)],
            host_account_id: account.into(),
            host_public_key: input.host_public_key.clone(),
            join_policy: input.join_policy,
            listed: input.listed,
            version: input.version.clone(),
            name: name.clone(),
            created_at: now,
            realm_id,
            bans: Vec::new(),
            join_requests: Vec::new(),
            player_invites: Vec::new(),
            last_online_at: None,
            online_until: None,
            hosting: None,
            offline_notified_for: None,
        };
        let actor = as_account(account);
        let mut inserted = self.realms.insert_realm(&actor, &record).await?;
        // A name whose holder let it lapse is released to the new claim.
        if inserted == InsertRealmResult::NameTaken
            && let Some(holder) = self.realms.find_by_name(&name).await?
            && is_realm_name_releasable(&holder, now)
            && self
                .realms
                .delete_realm_if_last_online(&actor, &holder.realm_id, holder.last_online_at)
                .await?
        {
            self.note(&format!(
                "[Realms] name released realmId={} name={name}",
                holder.realm_id
            ));
            self.drop_realm_presence(&actor, &holder.realm_id).await;
            inserted = self.realms.insert_realm(&actor, &record).await?;
        }
        match inserted {
            InsertRealmResult::Inserted => {
                let (id, policy, listed) = (&record.realm_id, record.join_policy.as_str(), record.listed);
                self.note(&format!(
                    "[Realms] realm created realmId={id} name={name} policy={policy} listed={listed}"
                ));
                let view = self.host_view(&record).await;
                Ok(HostedReply { realm: view })
            }
            InsertRealmResult::NameTaken => Err(refused(NAME_TAKEN)),
            InsertRealmResult::HostHasRealm => Err(refused(ALREADY_HOSTS)),
        }
    }

    /// The realm the account hosts and the realms it belongs to.
    pub async fn get_mine(&self, account: &str) -> Result<MineReply, RealmsError> {
        self.spend(account, READ)?;
        let own = self.realms.find_by_host(account).await?;
        let mut joined = self.realms.find_by_member(account, LIST_LIMIT).await?;
        joined.retain(|realm| realm.host_account_id != account);
        let hosted = match own.as_ref() {
            None => None,
            Some(realm) => Some(self.host_view(realm).await),
        };
        let member_of = self.public_infos(&joined, account).await;
        Ok(MineReply { hosted, member_of })
    }

    /// The realm with this name; a known host key tells the viewer whether its owner changed.
    pub async fn get_by_name(
        &self,
        account: &str,
        name: &str,
        known_host_key: Option<&str>,
    ) -> Result<RealmReply, RealmsError> {
        self.spend(account, READ)?;
        let realm = self.named_realm(name).await?;
        let info = self.public_info(&realm, account, known_host_key).await;
        Ok(RealmReply { realm: info })
    }

    /// Uses nothing.
    pub async fn preview_invite(&self, account: &str, code: &str) -> Result<RealmReply, RealmsError> {
        self.spend(account, JOIN)?;
        let invalid = || refused(INVITE_NOT_VALID);
        let invite = self.realms.find_invite(code).await?.ok_or_else(invalid)?;
        if invite_state(&invite, self.now()) != InviteState::Usable {
            return Err(entry_error("REALM_INVITE_EXPIRED"));
        }
        let realm = self.realms.find_by_id(&invite.realm_id).await?.ok_or_else(invalid)?;
        if realm.bans.iter().any(|ban| ban.account_id == account) {
            return Err(entry_error("REALM_BANNED"));
        }
        let info = self.public_info(&realm, account, None).await;
        Ok(RealmReply { realm: info })
    }

    /// Online first, then by name; a SOLO realm only to its host.
    pub async fn list_listed(&self, account: &str, limit: u32, offset: u32) -> Result<RealmsReply, RealmsError> {
        self.spend(account, READ)?;
        let mut realms = self.realms.list_listed(limit, offset).await?;
        realms.retain(|realm| realm.host_account_id == account || realm.join_policy != JoinPolicy::Solo);
        let infos = self.public_infos(&realms, account).await;
        Ok(RealmsReply { realms: infos })
    }

    /// Changes the host's join policy and listing.
    pub async fn update_settings(&self, account: &str, input: &SettingsInput) -> Result<HostedReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        let (policy, listed) = (input.join_policy, input.listed);
        let set = self
            .realms
            .set_settings(&as_account(account), &realm.realm_id, account, policy, listed)
            .await?;
        or_refuse(set, NOT_HOSTED)?;
        let updated = self
            .realms
            .find_by_id(&realm.realm_id)
            .await?
            .ok_or_else(|| refused(NOT_HOSTED))?;
        let (id, policy, listed) = (&updated.realm_id, updated.join_policy.as_str(), updated.listed);
        self.note(&format!(
            "[Realms] settings changed realmId={id} policy={policy} listed={listed}"
        ));
        let view = self.host_view(&updated).await;
        Ok(HostedReply { realm: view })
    }

    /// Makes an invite code for the host's realm; refused past the active-invite limit.
    pub async fn create_invite(&self, account: &str, input: &InviteInput) -> Result<InviteReply, RealmsError> {
        let realm = self.own_realm(INVITE, account).await?;
        let now = self.now();
        let limit = self.parts.limits.max_active_invites;
        if self.realms.count_active_invites(&realm.realm_id, now).await? >= i64::from(limit) {
            let message = format!("You can have at most {limit} open invites; revoke one first.");
            return Err(RealmsError::Refused(Refusal {
                code: "REALM_INVITE_LIMIT".into(),
                message,
            }));
        }
        let actor = as_account(account);
        // A clash with an existing code is retried with a fresh one.
        for _ in 0..INVITE_ATTEMPTS {
            let code = self.parts.ids.invite_code();
            let invite = build_invite(&code, &realm.realm_id, account, now, input.max_uses, input.ttl_hours);
            if self.realms.insert_invite(&actor, &invite).await? != InsertInviteResult::Inserted {
                continue;
            }
            let (id, uses, expires) = (&realm.realm_id, invite.max_uses, iso(invite.expires_at));
            self.note(&format!(
                "[Realms] invite created realmId={id} maxUses={uses} expiresAt={expires}"
            ));
            return Ok(InviteReply {
                invite: invite_view(&invite, now),
            });
        }
        Err(RealmsError::Database(String::from(
            "Could not generate a unique invite code.",
        )))
    }

    /// The host realm's invite codes.
    pub async fn list_invites(&self, account: &str) -> Result<InvitesReply, RealmsError> {
        let realm = self.own_realm(READ, account).await?;
        let now = self.now();
        let stored = self.realms.list_invites(&realm.realm_id, LIST_LIMIT).await?;
        Ok(InvitesReply {
            invites: stored.iter().map(|invite| invite_view(invite, now)).collect(),
        })
    }

    /// Revoked once: a second revoke keeps the first time.
    pub async fn revoke_invite(&self, account: &str, code: &str) -> Result<InviteReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        let now = self.now();
        let revoked = self
            .realms
            .revoke_invite(&as_account(account), &realm.realm_id, code, now)
            .await?;
        let invite = revoked.ok_or_else(|| refused(INVITE_NOT_VALID))?;
        self.note(&format!("[Realms] invite revoked realmId={}", realm.realm_id));
        Ok(InviteReply {
            invite: invite_view(&invite, now),
        })
    }

    /// Joins a realm as a member, by invite or because it is public, or asks to join.
    pub async fn join(&self, account: &str, input: &JoinInput) -> Result<JoinReply, RealmsError> {
        self.spend(account, JOIN)?;
        let invite = match input.invite_code.as_deref() {
            None => None,
            Some(code) => Some(
                self.realms
                    .find_invite(code)
                    .await?
                    .ok_or_else(|| refused(INVITE_NOT_VALID))?,
            ),
        };
        // The name decides; an invite alone finds its realm.
        let found = match (input.name.as_deref(), invite.as_ref()) {
            (Some(name), _) => self.realms.find_by_name(name).await?,
            (None, Some(invite)) => self.realms.find_by_id(&invite.realm_id).await?,
            (None, None) => None,
        };
        let realm = found.ok_or_else(|| entry_error("REALM_NOT_FOUND"))?;
        if invite.as_ref().is_some_and(|invite| invite.realm_id != realm.realm_id) {
            return Err(entry_error("REALM_INVITE_INVALID"));
        }
        if input
            .known_host_key
            .as_deref()
            .is_some_and(|key| detect_ownership_change(&realm.name, key, &realm))
        {
            return Err(entry_error("REALM_OWNER_CHANGED"));
        }
        let invited = realm.player_invites.iter().any(|entry| entry.account_id == account);
        let via = match decide_join(&realm, account, invite.as_ref(), self.now()) {
            Ok(via) => via,
            // A player the host invited joins on the spot.
            Err(JoinDenial::InviteRequired) if invited => {
                self.accept_invite_for(account, &realm.realm_id)
                    .await
                    .map_err(internal)?;
                let realm = self.reload(&realm.realm_id).await?;
                let info = self.public_info(&realm, account, None).await;
                return Ok(JoinReply {
                    joined: true,
                    realm: info,
                });
            }
            Err(JoinDenial::InviteRequired) => {
                self.request(account, &realm).await.map_err(internal)?;
                JoinVia::Member
            }
            Err(denial) => return Err(entry_error(denial.code())),
        };
        let joined = via != JoinVia::Member
            && matches!(
                self.admit(account, &realm, via, invite.as_ref())
                    .await
                    .map_err(internal)?,
                Admitted::Added
            );
        let realm = self.reload(&realm.realm_id).await?;
        let info = self.public_info(&realm, account, None).await;
        Ok(JoinReply { joined, realm: info })
    }

    /// Play's realm check: refusals are returned, not thrown. The host enters its own realm even when nothing serves.
    pub async fn authorize_entry(
        &self,
        account: &str,
        input: &EntryRealmInput,
    ) -> Result<EntryAuthorization, RealmsError> {
        use EntryAuthorization::Refused;
        self.spend(account, ENTRY)?;
        let invite = match input.invite_code.as_deref() {
            None => None,
            Some(code) => match self.realms.find_invite(code).await? {
                None => return Ok(Refused("REALM_INVITE_INVALID")),
                found => found,
            },
        };
        let Some(realm) = self.realms.find_by_name(&input.name).await? else {
            return Ok(Refused("REALM_NOT_FOUND"));
        };
        if invite.as_ref().is_some_and(|invite| invite.realm_id != realm.realm_id) {
            return Ok(Refused("REALM_INVITE_INVALID"));
        }
        if input
            .known_host_key
            .as_deref()
            .is_some_and(|key| detect_ownership_change(&realm.name, key, &realm))
        {
            return Ok(Refused("REALM_OWNER_CHANGED"));
        }
        let invited = realm.player_invites.iter().any(|entry| entry.account_id == account);
        let via = match decide_join(&realm, account, invite.as_ref(), self.now()) {
            Ok(via) => via,
            Err(JoinDenial::InviteRequired) => {
                let asked = if invited {
                    self.accept_invite_for(account, &realm.realm_id).await.map(drop)
                } else {
                    self.request(account, &realm).await.map(drop)
                };
                if let Err(code) = asked {
                    return Ok(Refused(code));
                }
                JoinVia::Member
            }
            Err(denial) => return Ok(Refused(denial.code())),
        };
        let is_host = realm.host_account_id == account;
        // An offline refusal uses no invite and adds no member.
        if !is_host && !is_realm_serving(&realm, self.now()) {
            return Ok(Refused("REALM_OFFLINE"));
        }
        if via != JoinVia::Member
            && let Err(code) = self.admit(account, &realm, via, invite.as_ref()).await
        {
            return Ok(Refused(code));
        }
        let realm_id = realm.realm_id;
        Ok(EntryAuthorization::Ok { realm_id, is_host })
    }

    /// Leaves a realm; the host cannot.
    pub async fn leave(&self, account: &str, name: &str) -> Result<LeftReply, RealmsError> {
        self.spend(account, WRITE)?;
        let realm = self.named_realm(name).await?;
        or_refuse(realm.host_account_id != account, HOST_CANNOT_LEAVE)?;
        let removed = self
            .realms
            .remove_member(&as_account(account), &realm.realm_id, account)
            .await?;
        or_refuse(removed, NOT_MY_REALM)?;
        self.note(&format!("[Realms] member left realmId={}", realm.realm_id));
        self.member_removed(&realm.realm_id, account, RemovalReason::Leave)
            .await;
        Ok(LeftReply { left: true })
    }

    /// A kicked member may come back.
    pub async fn kick(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        or_refuse(target != account, KICK_SELF)?;
        let removed = self
            .realms
            .remove_member(&as_account(account), &realm.realm_id, target)
            .await?;
        or_refuse(removed, NOT_YOUR_MEMBER)?;
        self.note(&format!("[Realms] member kicked realmId={}", realm.realm_id));
        self.member_removed(&realm.realm_id, target, RemovalReason::Kick).await;
        Ok(status(target, "KICKED"))
    }

    /// Bans an account from the host's realm, removing it as a member.
    pub async fn ban(&self, account: &str, target: &str, reason: Option<String>) -> Result<ChangedReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        or_refuse(target != account, BAN_SELF)?;
        let ban = RealmBan {
            account_id: target.into(),
            at: self.now(),
            reason,
        };
        let cap = self.parts.limits.max_bans;
        let outcome = self
            .realms
            .add_ban(&as_account(account), &realm.realm_id, &ban, cap)
            .await?;
        match outcome {
            AddBanResult::Banned => {
                self.note(&format!("[Realms] account banned realmId={}", realm.realm_id));
                self.member_removed(&realm.realm_id, target, RemovalReason::Ban).await;
            }
            AddBanResult::AlreadyBanned => {}
            AddBanResult::Full => return Err(refused(BANS_FULL)),
            AddBanResult::NotFound => return Err(refused(NOT_HOSTED)),
        }
        Ok(changed(target, "BANNED", outcome == AddBanResult::Banned))
    }

    /// Never an error.
    pub async fn unban(&self, account: &str, target: &str) -> Result<ChangedReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        let lifted = self
            .realms
            .remove_ban(&as_account(account), &realm.realm_id, target)
            .await?;
        if lifted {
            self.note(&format!("[Realms] ban lifted realmId={}", realm.realm_id));
        }
        Ok(changed(target, "UNBANNED", lifted))
    }

    /// The host realm's pending join requests.
    pub async fn list_join_requests(&self, account: &str) -> Result<RequestsReply, RealmsError> {
        let realm = self.own_realm(READ, account).await?;
        let newest_first = sort_newest_first(&realm.join_requests);
        let names = self
            .names(newest_first.iter().map(|entry| entry.account_id.clone()).collect())
            .await;
        let none = BTreeMap::new();
        let context = ViewContext {
            viewer: account,
            names: &names,
            counts: &none,
            known_key: None,
            now: self.now(),
        };
        Ok(RequestsReply {
            requests: newest_first
                .into_iter()
                .map(|entry| context.account_at(entry))
                .collect(),
        })
    }

    /// Admits an account that asked to join.
    pub async fn approve_join_request(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        or_refuse(
            realm.join_requests.iter().any(|entry| entry.account_id == target),
            NO_REQUEST,
        )?;
        let actor = as_account(account);
        match self.add_as_member(&actor, &realm.realm_id, target).await? {
            AddMemberResult::Added | AddMemberResult::AlreadyMember => {}
            AddMemberResult::Banned => {
                self.realms.remove_join_request(&actor, &realm.realm_id, target).await?;
                return Err(refused(TARGET_BANNED));
            }
            AddMemberResult::Full => return Err(entry_error("REALM_FULL")),
            AddMemberResult::NotFound => return Err(refused(NOT_HOSTED)),
        }
        self.realms.remove_join_request(&actor, &realm.realm_id, target).await?;
        self.note(&format!("[Realms] join request approved realmId={}", realm.realm_id));
        Ok(status(target, "APPROVED"))
    }

    /// The player may ask again.
    pub async fn deny_join_request(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        let removed = self
            .realms
            .remove_join_request(&as_account(account), &realm.realm_id, target)
            .await?;
        or_refuse(removed, NO_REQUEST)?;
        self.note(&format!("[Realms] join request denied realmId={}", realm.realm_id));
        Ok(status(target, "DENIED"))
    }

    /// (account, display name) of the username's player; the name typed stands in for a display name it lacks.
    async fn resolve_player(&self, username: &str) -> Result<(String, String), RealmsError> {
        let typed = js_trim(username).to_owned();
        let found = self
            .parts
            .accounts
            .resolve_username(typed.clone())
            .await
            .map_err(RealmsError::Database)?;
        let Some(target) = found else {
            return Err(refused(NO_PLAYER));
        };
        let display = self.names(vec![target.clone()]).await.remove(&target).unwrap_or(typed);
        Ok((target, display))
    }

    /// Launchers to 0.1.68 add members directly.
    pub async fn add_member_by_username(&self, account: &str, username: &str) -> Result<AddedReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        let (account_id, display_name) = self.resolve_player(username).await?;
        let actor = as_account(account);
        let outcome = self.add_as_member(&actor, &realm.realm_id, &account_id).await?;
        match outcome {
            AddMemberResult::Added | AddMemberResult::AlreadyMember => {}
            AddMemberResult::Banned => return Err(refused(TARGET_BANNED_LIFT)),
            AddMemberResult::Full => return Err(entry_error("REALM_FULL")),
            AddMemberResult::NotFound => return Err(refused(NOT_HOSTED)),
        }
        self.realms
            .remove_join_request(&actor, &realm.realm_id, &account_id)
            .await?;
        let added = outcome == AddMemberResult::Added;
        if added {
            self.note(&format!("[Realms] player added by the host realmId={}", realm.realm_id));
        }
        Ok(AddedReply {
            account_id,
            display_name,
            added,
        })
    }

    /// Invites a player by username, or lets them in at once when they had asked to join.
    pub async fn invite_player_by_username(&self, account: &str, username: &str) -> Result<InvitedReply, RealmsError> {
        let realm = self.own_realm(INVITE, account).await?;
        let (target, display_name) = self.resolve_player(username).await?;
        or_refuse(target != account, INVITE_SELF)?;
        let reply = |status| InvitedReply {
            status,
            display_name: display_name.clone(),
            account_id: target.clone(),
        };
        let realm_id = realm.realm_id.as_str();
        // A player who already asked to join is let in instead.
        if realm.join_requests.iter().any(|entry| entry.account_id == target) {
            return match self.accept_invite_for(&target, realm_id).await {
                Ok(_) => {
                    self.note(&format!(
                        "[Realms] invited player had asked to join; let in realmId={realm_id}"
                    ));
                    Ok(reply("MEMBER"))
                }
                Err("REALM_BANNED") => Err(refused(TARGET_BANNED_LIFT)),
                Err("REALM_FULL") => Err(entry_error("REALM_FULL")),
                Err(_) => Err(refused(NOT_HOSTED)),
            };
        }
        let invite = AccountAt {
            at: self.now(),
            account_id: target.clone(),
        };
        let cap = self.parts.limits.max_player_invites;
        match self
            .realms
            .add_player_invite(&as_account(account), realm_id, &invite, cap)
            .await?
        {
            AddEntryResult::Added => {
                self.note(&format!("[Realms] player invited realmId={realm_id}"));
                Ok(reply("INVITED"))
            }
            AddEntryResult::Already => Ok(reply("INVITED")),
            AddEntryResult::AlreadyMember => Ok(reply("MEMBER")),
            AddEntryResult::Full => Err(refused(PLAYER_INVITES_FULL)),
            AddEntryResult::Banned => Err(refused(TARGET_BANNED_LIFT)),
            AddEntryResult::NotFound => Err(refused(NOT_HOSTED)),
        }
    }

    /// Withdraws an invite the host sent a player.
    pub async fn cancel_player_invite(&self, account: &str, target: &str) -> Result<StatusReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        let removed = self
            .realms
            .remove_player_invite(&as_account(account), &realm.realm_id, target)
            .await?;
        or_refuse(removed, NO_WAITING_INVITE)?;
        self.note(&format!("[Realms] player invite cancelled realmId={}", realm.realm_id));
        Ok(status(target, "CANCELLED"))
    }

    /// The realms that invited the account.
    pub async fn list_my_player_invites(&self, account: &str) -> Result<PlayerInvitesReply, RealmsError> {
        self.spend(account, READ)?;
        let realms = self.realms.find_by_player_invite(account, LIST_LIMIT).await?;
        let infos = self.public_infos(&realms, account).await;
        // When the invite arrived; a realm without one sorts by its creation.
        let invited_at = |realm: &RealmRecord| {
            let mine = realm.player_invites.iter().find(|invite| invite.account_id == account);
            mine.map_or(realm.created_at, |invite| invite.at)
        };
        let mut dated: Vec<(UnixMillis, RealmPublicInfo)> = realms.iter().map(invited_at).zip(infos).collect();
        dated.sort_by_key(|&(at, _)| std::cmp::Reverse(at));
        let invites = dated
            .into_iter()
            .map(|(at, realm)| PlayerInviteEntry { at: iso(at), realm })
            .collect();
        Ok(PlayerInvitesReply { invites })
    }

    /// Joins a realm that invited the account.
    pub async fn accept_player_invite(&self, account: &str, name: &str) -> Result<JoinReply, RealmsError> {
        self.spend(account, WRITE)?;
        let realm = self.named_realm(name).await?;
        or_refuse(
            realm.player_invites.iter().any(|invite| invite.account_id == account),
            NO_INVITE_HERE,
        )?;
        if let Err(code) = self.accept_invite_for(account, &realm.realm_id).await {
            // A banned player's invite is no use to them.
            if code == "REALM_BANNED" {
                let _ = self
                    .realms
                    .remove_player_invite(&as_account(account), &realm.realm_id, account)
                    .await;
            }
            return Err(internal(code));
        }
        let realm = self.reload(&realm.realm_id).await?;
        let info = self.public_info(&realm, account, None).await;
        Ok(JoinReply {
            joined: true,
            realm: info,
        })
    }

    /// Declines a realm's invite.
    pub async fn decline_player_invite(&self, account: &str, name: &str) -> Result<DeclinedReply, RealmsError> {
        self.spend(account, WRITE)?;
        let realm = self.named_realm(name).await?;
        let removed = self
            .realms
            .remove_player_invite(&as_account(account), &realm.realm_id, account)
            .await?;
        or_refuse(removed, NO_INVITE_HERE)?;
        self.note(&format!("[Realms] player invite declined realmId={}", realm.realm_id));
        Ok(DeclinedReply { declined: true })
    }

    /// The hub fields are never self-reported.
    pub async fn heartbeat(&self, account: &str, input: &HeartbeatInput) -> Result<HeartbeatReply, RealmsError> {
        let realm = self.own_realm(HEARTBEAT, account).await?;
        let now = self.now();
        let update = apply_heartbeat(now, input.online, input.version.clone());
        let actor = as_account(account);
        let recorded = self
            .realms
            .record_heartbeat(&actor, &realm.realm_id, account, &update)
            .await?;
        or_refuse(recorded, NOT_HOSTED)?;
        if let Some(hosting) = input.hosting.as_ref() {
            let patch = HostingPatch {
                afk_seconds: hosting.afk_seconds.map_or(Patch::Remove, Patch::Set),
                max_players: Some(hosting.max_players),
                max_servers: Some(hosting.max_servers),
                ..HostingPatch::default()
            };
            self.realms.patch_hosting(&actor, &realm.realm_id, &patch, now).await?;
        }
        Ok(HeartbeatReply {
            version: input.version.clone().or(realm.version),
            ttl_seconds: HEARTBEAT_TTL_SECONDS,
            online_until: iso(update.online_until),
            last_online_at: iso(update.last_online_at),
            online: input.online != Some(false),
        })
    }

    /// The name is free at once; members and invites go with the realm.
    pub async fn release(&self, account: &str) -> Result<ReleasedReply, RealmsError> {
        let realm = self.own_realm(WRITE, account).await?;
        let actor = as_account(account);
        or_refuse(
            self.realms
                .delete_realm_of_host(&actor, &realm.realm_id, account)
                .await?,
            NOT_HOSTED,
        )?;
        self.note(&format!(
            "[Realms] realm released realmId={} name={}",
            realm.realm_id, realm.name
        ));
        self.drop_realm_presence(&actor, &realm.realm_id).await;
        self.emit(RealmEvent::RealmReleased {
            realm_id: realm.realm_id,
            host_account_id: account.into(),
        })
        .await;
        Ok(ReleasedReply { released: true })
    }

    async fn presence_view(&self, row: Option<RealmPresenceRecord>) -> Result<PresenceView, RealmsError> {
        let realm = match row.as_ref() {
            Some(row) => self.realms.find_by_id(&row.realm_id).await?,
            None => None,
        };
        let (Some(row), Some(realm)) = (row, realm) else {
            return Ok(NOWHERE);
        };
        let now = self.now();
        let shown = PresenceRealm {
            host_online: is_realm_online(&realm, now),
            hub_ready: is_realm_hub_ready(&realm),
            serving: is_realm_serving(&realm, now),
            name: realm.name,
        };
        Ok(PresenceView {
            realm: Some(shown),
            expires_at: Some(iso(row.expires_at)),
        })
    }

    /// Keeps the account's realm presence alive and answers where it is bound.
    pub async fn presence_heartbeat(&self, account: &str) -> Result<PresenceView, RealmsError> {
        self.spend(account, HEARTBEAT)?;
        let touched = self.touch_presence(account).await?;
        self.presence_view(touched).await
    }

    /// Idempotent.
    pub async fn presence_leave(&self, account: &str) -> Result<LeftReply, RealmsError> {
        self.spend(account, HEARTBEAT)?;
        self.leave_presence(account).await.map(|()| LeftReply { left: true })
    }

    /// Where the account's login is bound.
    pub async fn presence_status(&self, account: &str) -> Result<PresenceView, RealmsError> {
        self.spend(account, READ)?;
        let current = self.presence_of(account).await?;
        self.presence_view(current).await
    }

    /// On the matchmaking hot path: each answer, "none" included, is trusted for two seconds.
    pub async fn presence_of(&self, account: &str) -> Result<Option<RealmPresenceRecord>, RealmsError> {
        let now = self.now();
        let epoch = {
            let cache = self.cache();
            if let Some(row) = cache.fresh(account, now) {
                // A lapsed row is never active.
                return Ok(row.as_ref().filter(|r| r.expires_at > now).cloned());
            }
            cache.epoch(account)
        };
        self.presence_reads.fetch_add(1, SeqCst);
        let read = self.presence.find_active(account, now).await?;
        // A write that raced the read must not be overwritten by the stale answer.
        let mut cache = self.cache();
        if epoch == cache.epoch(account) {
            cache.store(account, now, read.clone());
        }
        Ok(read)
    }

    /// Binds the account's login to a realm, or clears the binding for `None`.
    pub async fn bind_presence(&self, account: &str, realm_id: Option<&str>) -> Result<(), RealmsError> {
        let actor = as_account(account);
        if let Some(realm_id) = realm_id {
            let now = self.now();
            let row = RealmPresenceRecord {
                expires_at: now.saturating_add(PRESENCE_TTL_MS),
                account_id: account.into(),
                realm_id: realm_id.into(),
                last_seen_at: now,
                bound_at: now,
            };
            self.presence.bind(&actor, &row).await?;
        } else {
            self.presence.remove(&actor, account, None).await?;
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
        self.presence_touches.fetch_add(1, SeqCst);
        let until = now.saturating_add(PRESENCE_TTL_MS);
        let touched = self.presence.touch(&as_account(account), account, now, until).await?;
        let mut cache = self.cache();
        // Invalidated either way; the touched row is cached only when no other write raced this one.
        let raced = cache.epoch(account) != epoch;
        cache.invalidate(account);
        if !raced {
            cache.store(account, now, touched.clone());
        }
        Ok(touched)
    }

    /// Clears the account's realm binding.
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
        let Some(asked) = realm.filter(|value| !value.is_null()) else {
            return Ok(None);
        };
        or_refuse(module_enabled, REALMS_DISABLED)?;
        let input = crate::input::parse_entry_realm(asked).map_err(|failure| {
            RealmsError::Refused(Refusal {
                code: failure.code.into(),
                message: failure.message,
            })
        })?;
        match self.authorize_entry(account, &input).await? {
            EntryAuthorization::Refused(code) => Err(entry_error(code)),
            EntryAuthorization::Ok { realm_id, is_host } => {
                self.note(&format!(
                    "[Realms] game session authorized realmId={realm_id} host={is_host}"
                ));
                Ok(Some((realm_id, is_host)))
            }
        }
    }

    /// Binds or clears the account's realm at game login while the module is on; a failed clear is logged, not returned.
    pub async fn bind_presence_at_login(
        &self,
        account: &str,
        realm_id: Option<&str>,
        module_enabled: bool,
    ) -> Result<(), RealmsError> {
        if !module_enabled {
            return Ok(());
        }
        if realm_id.is_some() {
            return self.bind_presence(account, realm_id).await;
        }
        // A failed reset leaves the old row to lapse; login goes on.
        if let Err(error) = self.bind_presence(account, None).await {
            self.caution(&format!("[Realms] could not reset realm presence at login: {error}"));
        }
        Ok(())
    }

    /// Applies a hosting patch to the realm and returns whether a realm was written. An invalid patch is an error.
    pub async fn set_hosting(&self, realm_id: &str, patch: &HostingPatch) -> Result<bool, RealmsError> {
        if let Some(field) = validate_hosting_patch(patch) {
            return Err(RealmsError::Database(format!("setHosting: invalid {field}")));
        }
        let written = self
            .realms
            .patch_hosting(&P2P_ACTOR, realm_id, patch, self.now())
            .await?;
        Ok(written)
    }

    /// Ends the online state now; emits nothing.
    pub async fn mark_offline(&self, realm_id: &str) -> Result<bool, RealmsError> {
        let marked = self.realms.mark_offline(&P2P_ACTOR, realm_id, self.now()).await?;
        Ok(marked)
    }

    /// The realm whose hub has this room id.
    pub async fn find_by_hub_room_id(&self, room: &str) -> Result<Option<RealmRecord>, RealmsError> {
        match normalize_hub_room_id(room) {
            Some(room) => Ok(self.realms.find_by_hub_room_id(&room).await?),
            None => Ok(None),
        }
    }

    /// The realm with this id.
    pub async fn get_realm(&self, realm_id: &str) -> Result<Option<RealmRecord>, RealmsError> {
        let found = self.realms.find_by_id(realm_id).await?;
        Ok(found)
    }

    /// The realm this account hosts.
    pub async fn get_realm_by_host(&self, host: &str) -> Result<Option<RealmRecord>, RealmsError> {
        let found = self.realms.find_by_host(host).await?;
        Ok(found)
    }

    /// Whether the account is a member of the realm.
    pub async fn is_member(&self, realm_id: &str, account: &str) -> Result<bool, RealmsError> {
        let realm = self.get_realm(realm_id).await?;
        Ok(realm.is_some_and(|found| found.members.iter().any(|member| member.account_id == account)))
    }

    /// Whether the account is banned from the realm.
    pub async fn is_banned(&self, realm_id: &str, account: &str) -> Result<bool, RealmsError> {
        let realm = self.get_realm(realm_id).await?;
        Ok(realm.is_some_and(|found| found.bans.iter().any(|ban| ban.account_id == account)))
    }

    /// Each lapse is reported once; nothing schedules this, the caller drives it.
    pub async fn sweep_lapsed_hosts(&self) -> Result<usize, RealmsError> {
        let now = self.now();
        let since = UnixMillis(now.0 - SWEEP_LOOKBACK_MS);
        let lapsed = self
            .realms
            .claim_lapsed_hosts(&SWEEPER_ACTOR, now, since, SWEEP_LIMIT)
            .await?;
        for realm in &lapsed {
            self.note(&format!("[Realms] host offline realmId={}", realm.realm_id));
            let event = RealmEvent::HostOffline {
                realm_id: realm.realm_id.clone(),
                host_account_id: realm.host_account_id.clone(),
            };
            self.emit(event).await;
        }
        Ok(lapsed.len())
    }
}
