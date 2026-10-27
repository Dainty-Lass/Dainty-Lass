//! The realm directory's records (`realms/realmTypes.ts`) and its rules as pure functions (`realms/realmRules.ts`):
//! names, invites, the join decision, online state, hosting and hub room ids. Callers pass the time.

use rifts_seam::{HubKind, UnixMillis};
use sha2::{Digest, Sha256};

use crate::time::iso;

const MS_PER_HOUR: i64 = 3_600_000;
const MS_PER_DAY: i64 = 24 * MS_PER_HOUR;

/// `RealmJoinPolicy`. SOLO: only the host enters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPolicy {
    InviteOnly,
    Public,
    Solo,
}

impl JoinPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InviteOnly => "INVITE_ONLY",
            Self::Public => "PUBLIC",
            Self::Solo => "SOLO",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "INVITE_ONLY" => Some(Self::InviteOnly),
            "PUBLIC" => Some(Self::Public),
            "SOLO" => Some(Self::Solo),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberRole {
    Host,
    Member,
}

impl MemberRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "HOST",
            Self::Member => "MEMBER",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "HOST" => Some(Self::Host),
            "MEMBER" => Some(Self::Member),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmMember {
    pub account_id: String,
    pub role: MemberRole,
    pub joined_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmBan {
    pub account_id: String,
    pub reason: Option<String>,
    pub at: UnixMillis,
}

/// A join request or a player invite: an account and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAt {
    pub account_id: String,
    pub at: UnixMillis,
}

/// `RealmHubState`: one hosted server kind on the host's PC, mirrored by the p2p side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubState {
    None,
    Starting,
    Running,
    Stopping,
}

impl HubState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Stopping => "STOPPING",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "NONE" => Some(Self::None),
            "STARTING" => Some(Self::Starting),
            "RUNNING" => Some(Self::Running),
            "STOPPING" => Some(Self::Stopping),
            _ => None,
        }
    }
}

/// `RealmHosting`: what the realm's host runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmHosting {
    pub max_servers: i64,
    pub max_players: i64,
    pub afk_seconds: Option<i64>,
    pub hub_session_id: Option<String>,
    pub hub_state: HubState,
    pub training_state: HubState,
    pub active_servers: i64,
    pub updated_at: UnixMillis,
}

/// One optional field of a patch: left alone, removed (Node's `null`) or set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Patch<T> {
    #[default]
    Keep,
    Remove,
    Set(T),
}

/// `RealmHostingPatch`; `updatedAt` is always the writer's clock.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostingPatch {
    pub max_servers: Option<i64>,
    pub max_players: Option<i64>,
    pub afk_seconds: Patch<i64>,
    pub hub_session_id: Patch<String>,
    pub hub_state: Option<HubState>,
    pub training_state: Option<HubState>,
    pub active_servers: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmRecord {
    pub realm_id: String,
    /// Canonical (lower-case); unique.
    pub name: String,
    pub host_account_id: String,
    /// The host launcher's public key; a name that comes back with another key has a new owner.
    pub host_public_key: String,
    pub join_policy: JoinPolicy,
    pub listed: bool,
    /// The host (role HOST) from creation on, then in joining order.
    pub members: Vec<RealmMember>,
    pub bans: Vec<RealmBan>,
    pub created_at: UnixMillis,
    pub last_online_at: Option<UnixMillis>,
    pub online_until: Option<UnixMillis>,
    /// The runtime version the host reported with its last heartbeat.
    pub version: Option<String>,
    pub hosting: Option<RealmHosting>,
    pub hub_room_ids: Vec<String>,
    /// The `online_until` for which HOST_OFFLINE was already emitted: one notice per lapse.
    pub offline_notified_for: Option<UnixMillis>,
    pub join_requests: Vec<AccountAt>,
    pub player_invites: Vec<AccountAt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmInviteRecord {
    pub code: String,
    pub realm_id: String,
    pub created_by: String,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub max_uses: i64,
    pub uses: i64,
    pub revoked_at: Option<UnixMillis>,
}

/// A member's current realm context, one row per account; active while `expires_at` is ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmPresenceRecord {
    pub account_id: String,
    pub realm_id: String,
    pub bound_at: UnixMillis,
    pub last_seen_at: UnixMillis,
    pub expires_at: UnixMillis,
}

/// `RealmEvent`: what the directory tells the p2p side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmEvent {
    MemberRemoved {
        realm_id: String,
        account_id: String,
        reason: RemovalReason,
    },
    RealmReleased {
        realm_id: String,
        host_account_id: String,
    },
    HostOffline {
        realm_id: String,
        host_account_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalReason {
    Kick,
    Ban,
    Leave,
}

// ------------------------------------------------------------------ names

/// Names nobody may claim, matched with the hyphens removed.
pub const RESERVED_REALM_NAMES: &[&str] = &[
    "official",
    "paradox",
    "mysticparadox",
    "mystic",
    "mysticfox",
    "admin",
    "administrator",
    "support",
    "mod",
    "moderator",
    "staff",
    "system",
    "root",
    "dauntless",
    "phoenixlabs",
    "launcher",
    "backend",
    "directory",
    "realm",
    "realms",
    "api",
    "www",
    "help",
    "security",
    "owner",
    "team",
    "local",
    "dedicated",
    "default",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRefusal {
    Invalid,
    Reserved,
}

/// `REALM_NAME_PATTERN`: 3 to 24 of a-z, 0-9 and hyphen.
pub fn matches_name_pattern(name: &str) -> bool {
    (3..=24).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Trimmed and lower-cased; `None` past 256 UTF-16 units, as Node's length check.
pub fn normalize_realm_name(raw: &str) -> Option<String> {
    if raw.encode_utf16().count() > 256 {
        return None;
    }
    Some(js_trim(raw).to_lowercase())
}

pub fn is_reserved_realm_name(name: &str) -> bool {
    let bare: String = name.chars().filter(|c| *c != '-').collect();
    RESERVED_REALM_NAMES.contains(&bare.as_str())
}

/// Format first (no leading or trailing hyphen), then the reserved list.
pub fn validate_realm_name(raw: &str) -> Result<String, NameRefusal> {
    let name = normalize_realm_name(raw).ok_or(NameRefusal::Invalid)?;
    if !matches_name_pattern(&name) || name.starts_with('-') || name.ends_with('-') {
        return Err(NameRefusal::Invalid);
    }
    if is_reserved_realm_name(&name) {
        return Err(NameRefusal::Reserved);
    }
    Ok(name)
}

/// A realm is named after its host's username: lower case, other characters as one hyphen, none at either end.
pub fn realm_name_for_username(username: &str) -> Option<String> {
    if username.encode_utf16().count() > 256 {
        return None;
    }
    let mut name = String::new();
    let mut pending_hyphen = false;
    for c in js_trim(username).to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_hyphen && !name.is_empty() {
                name.push('-');
            }
            pending_hyphen = false;
            name.push(c);
        } else {
            pending_hyphen = true;
        }
    }
    matches_name_pattern(&name).then_some(name)
}

pub const NEVER_ONLINE_RELEASE_MS: i64 = 3 * MS_PER_DAY;
pub const LAST_ONLINE_RELEASE_MS: i64 = 30 * MS_PER_DAY;

/// When the realm's name becomes free for someone else.
pub fn realm_name_release_at(created_at: UnixMillis, last_online_at: Option<UnixMillis>) -> UnixMillis {
    match last_online_at {
        Some(at) => UnixMillis(at.0 + LAST_ONLINE_RELEASE_MS),
        None => UnixMillis(created_at.0 + NEVER_ONLINE_RELEASE_MS),
    }
}

pub fn is_realm_name_releasable(realm: &RealmRecord, now: UnixMillis) -> bool {
    now >= realm_name_release_at(realm.created_at, realm.last_online_at)
}

/// The name the launcher knew is now held under a different host key.
pub fn detect_ownership_change(known_name: &str, known_key: &str, realm: &RealmRecord) -> bool {
    known_name.to_lowercase() == realm.name.to_lowercase() && known_key != realm.host_public_key
}

// ------------------------------------------------------------------ invites

/// No I, O, 0 or 1; 32 divides 256, so a masked random byte is unbiased.
pub const INVITE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const INVITE_CODE_LENGTH: usize = 10;
pub const INVITE_DEFAULT_MAX_USES: i64 = 10;
pub const INVITE_MAX_USES_LIMIT: i64 = 100;
pub const INVITE_DEFAULT_TTL_HOURS: i64 = 168;
pub const INVITE_MAX_TTL_HOURS: i64 = 720;
/// Node's TTL index removes an invite a week after it expired (`INVITE_PURGE_GRACE_SECONDS`).
pub const INVITE_PURGE_GRACE_MS: i64 = 7 * MS_PER_DAY;

pub fn generate_invite_code() -> String {
    invite_code_from(rand::random())
}

/// The low 5 bits of each byte pick a symbol.
pub fn invite_code_from(bytes: [u8; INVITE_CODE_LENGTH]) -> String {
    bytes
        .iter()
        .map(|b| char::from(INVITE_ALPHABET[usize::from(b & 31)]))
        .collect()
}

/// `abcde-fghjk` (dashes, spaces) down to its stored form; `None` when it cannot be a code.
pub fn normalize_invite_code(raw: &str) -> Option<String> {
    if raw.encode_utf16().count() > 40 {
        return None;
    }
    let code: String = raw
        .chars()
        .filter(|c| *c != '-' && !is_js_whitespace(*c))
        .collect::<String>()
        .to_uppercase();
    (code.len() == INVITE_CODE_LENGTH && code.bytes().all(|b| INVITE_ALPHABET.contains(&b))).then_some(code)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteState {
    Usable,
    Revoked,
    Expired,
    Exhausted,
}

impl InviteState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usable => "USABLE",
            Self::Revoked => "REVOKED",
            Self::Expired => "EXPIRED",
            Self::Exhausted => "EXHAUSTED",
        }
    }
}

pub fn invite_state(invite: &RealmInviteRecord, now: UnixMillis) -> InviteState {
    if invite.revoked_at.is_some() {
        InviteState::Revoked
    } else if now >= invite.expires_at {
        InviteState::Expired
    } else if invite.uses >= invite.max_uses {
        InviteState::Exhausted
    } else {
        InviteState::Usable
    }
}

pub fn build_invite(
    code: String,
    realm_id: &str,
    created_by: &str,
    now: UnixMillis,
    max_uses: i64,
    ttl_hours: i64,
) -> RealmInviteRecord {
    RealmInviteRecord {
        code,
        realm_id: realm_id.to_owned(),
        created_by: created_by.to_owned(),
        created_at: now,
        expires_at: UnixMillis(now.0 + ttl_hours * MS_PER_HOUR),
        max_uses,
        uses: 0,
        revoked_at: None,
    }
}

// ------------------------------------------------------------------ joining

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinVia {
    Member,
    Public,
    Invite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinDenial {
    Banned,
    Solo,
    InviteRequired,
    InviteInvalid,
    InviteExpired,
}

impl JoinDenial {
    /// The launcher error code (`JOIN_DENIAL_CODES`).
    pub const fn code(self) -> &'static str {
        match self {
            Self::Banned => "REALM_BANNED",
            Self::Solo => "REALM_SOLO",
            Self::InviteRequired => "REALM_INVITE_REQUIRED",
            Self::InviteInvalid => "REALM_INVITE_INVALID",
            Self::InviteExpired => "REALM_INVITE_EXPIRED",
        }
    }
}

/// A ban beats everything (an old invite too); SOLO lets only the host in; members are in; PUBLIC lets anyone in; an
/// invite-only realm needs a usable invite of its own. `Invite` tells the caller to consume a use.
pub fn decide_join(
    realm: &RealmRecord,
    account_id: &str,
    invite: Option<&RealmInviteRecord>,
    now: UnixMillis,
) -> Result<JoinVia, JoinDenial> {
    if realm.bans.iter().any(|ban| ban.account_id == account_id) {
        return Err(JoinDenial::Banned);
    }
    if realm.join_policy == JoinPolicy::Solo && account_id != realm.host_account_id {
        return Err(JoinDenial::Solo);
    }
    if realm.members.iter().any(|member| member.account_id == account_id) {
        return Ok(JoinVia::Member);
    }
    if realm.join_policy == JoinPolicy::Public {
        return Ok(JoinVia::Public);
    }
    let invite = invite.ok_or(JoinDenial::InviteRequired)?;
    if invite.realm_id != realm.realm_id {
        return Err(JoinDenial::InviteInvalid);
    }
    if invite_state(invite, now) != InviteState::Usable {
        return Err(JoinDenial::InviteExpired);
    }
    Ok(JoinVia::Invite)
}

/// Newest first; equal times keep their stored order (a stable sort, as Node's).
pub fn sort_newest_first(entries: &[AccountAt]) -> Vec<AccountAt> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|entry| std::cmp::Reverse(entry.at));
    sorted
}

// ------------------------------------------------------------------ online state

/// Each host heartbeat keeps the realm online this long.
pub const HEARTBEAT_ONLINE_MS: i64 = 90_000;

pub fn is_realm_online(realm: &RealmRecord, now: UnixMillis) -> bool {
    realm.online_until.is_some_and(|until| until > now)
}

/// Serving: the host is online and its Ramsgate hub runs.
pub fn is_realm_serving(realm: &RealmRecord, now: UnixMillis) -> bool {
    is_realm_online(realm, now) && is_realm_hub_ready(realm)
}

pub fn is_realm_hub_ready(realm: &RealmRecord) -> bool {
    realm
        .hosting
        .as_ref()
        .is_some_and(|hosting| hosting.hub_state == HubState::Running)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatUpdate {
    pub last_online_at: UnixMillis,
    pub online_until: UnixMillis,
    pub version: Option<String>,
}

/// One host heartbeat: online for 90 s, or `online: false` ends it now (still online-at-least-once for the name).
pub fn apply_heartbeat(now: UnixMillis, online: Option<bool>, version: Option<String>) -> HeartbeatUpdate {
    HeartbeatUpdate {
        last_online_at: now,
        online_until: if online == Some(false) {
            now
        } else {
            UnixMillis(now.0 + HEARTBEAT_ONLINE_MS)
        },
        version,
    }
}

// ------------------------------------------------------------------ hosting

pub const HOSTING_MAX_SERVERS_LIMIT: i64 = 8;
pub const HOSTING_MAX_PLAYERS_LIMIT: i64 = 64;
pub const HOSTING_MAX_AFK_SECONDS: i64 = 7200;
pub const HOSTING_ACTIVE_SERVERS_LIMIT: i64 = 64;
const HOSTING_SESSION_ID_MAX_LENGTH: usize = 128;

/// `DEFAULT_HOSTING`: one server, the hub's default player count, nothing running.
pub fn default_hosting(updated_at: UnixMillis) -> RealmHosting {
    RealmHosting {
        max_servers: 1,
        max_players: 16,
        afk_seconds: None,
        hub_session_id: None,
        hub_state: HubState::None,
        training_state: HubState::None,
        active_servers: 0,
        updated_at,
    }
}

/// `ApplyHostingPatch`: defaults fill a realm that had none, named fields change, `Remove` drops an optional one,
/// and `updated_at` is stamped.
pub fn apply_hosting_patch(current: Option<RealmHosting>, patch: &HostingPatch, at: UnixMillis) -> RealmHosting {
    let mut next = current.unwrap_or_else(|| default_hosting(at));
    if let Some(value) = patch.max_servers {
        next.max_servers = value;
    }
    if let Some(value) = patch.max_players {
        next.max_players = value;
    }
    match &patch.afk_seconds {
        Patch::Keep => {}
        Patch::Remove => next.afk_seconds = None,
        Patch::Set(value) => next.afk_seconds = Some(*value),
    }
    match &patch.hub_session_id {
        Patch::Keep => {}
        Patch::Remove => next.hub_session_id = None,
        Patch::Set(value) => next.hub_session_id = Some(value.clone()),
    }
    if let Some(value) = patch.hub_state {
        next.hub_state = value;
    }
    if let Some(value) = patch.training_state {
        next.training_state = value;
    }
    if let Some(value) = patch.active_servers {
        next.active_servers = value;
    }
    next.updated_at = at;
    next
}

/// The first bad field of a patch from the p2p side (trusted code, but a typo must not corrupt a realm).
pub fn validate_hosting_patch(patch: &HostingPatch) -> Option<&'static str> {
    let outside = |value: Option<i64>, min: i64, max: i64| value.is_some_and(|v| !(min..=max).contains(&v));
    if outside(patch.max_servers, 1, HOSTING_MAX_SERVERS_LIMIT) {
        return Some("maxServers");
    }
    if outside(patch.max_players, 1, HOSTING_MAX_PLAYERS_LIMIT) {
        return Some("maxPlayers");
    }
    if let Patch::Set(value) = patch.afk_seconds
        && !(0..=HOSTING_MAX_AFK_SECONDS).contains(&value)
    {
        return Some("afkSeconds");
    }
    if outside(patch.active_servers, 0, HOSTING_ACTIVE_SERVERS_LIMIT) {
        return Some("activeServers");
    }
    if let Patch::Set(id) = &patch.hub_session_id
        && (id.is_empty() || id.encode_utf16().count() > HOSTING_SESSION_ID_MAX_LENGTH)
    {
        return Some("hubSessionId");
    }
    None
}

// ------------------------------------------------------------------ hub room ids

pub const fn hub_kind_name(kind: HubKind) -> &'static str {
    match kind {
        HubKind::Ramsgate => "RAMSGATE",
        HubKind::Training => "TRAINING",
    }
}

pub fn parse_hub_kind(text: &str) -> Option<HubKind> {
    match text {
        "RAMSGATE" => Some(HubKind::Ramsgate),
        "TRAINING" => Some(HubKind::Training),
        _ => None,
    }
}

/// The game's and the chat's stable id of a realm's hub: SHA-256 over a fixed label, laid out as a version 5 UUID.
pub fn hub_room_id(realm_id: &str, kind: HubKind) -> String {
    let digest = Sha256::digest(format!("paradox-realm-hub:v1:{realm_id}:{}", hub_kind_name(kind)).as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    crate::ids::uuid_text(u128::from_be_bytes(bytes))
}

/// Both hub room ids, in the order the record stores them.
pub fn hub_room_ids_of(realm_id: &str) -> Vec<String> {
    vec![
        hub_room_id(realm_id, HubKind::Ramsgate),
        hub_room_id(realm_id, HubKind::Training),
    ]
}

/// The lower-case form of a UUID-shaped string; `None` when it is not one.
pub fn normalize_hub_room_id(raw: &str) -> Option<String> {
    let id = raw.to_ascii_lowercase();
    crate::ids::session_bits(&id).map(|_| id)
}

// ------------------------------------------------------------------ presence

/// Each bind and touch keeps a presence active this long.
pub const PRESENCE_TTL_MS: i64 = 120_000;
/// How long a process trusts its own answer to `presence_of`.
pub const PRESENCE_CACHE_MS: i64 = 2_000;
/// A touch writes once the row's `last_seen_at` is older than this,
pub const PRESENCE_TOUCH_WRITE_AFTER_MS: i64 = 30_000;
/// or when the row expires in less than this. Touches at most 60 s apart never let a row lapse.
pub const PRESENCE_TOUCH_MIN_LEFT_MS: i64 = 60_000;

pub fn presence_touch_needs_write(last_seen_at: UnixMillis, expires_at: UnixMillis, now: UnixMillis) -> bool {
    now.0 - last_seen_at.0 > PRESENCE_TOUCH_WRITE_AFTER_MS || expires_at.0 - now.0 < PRESENCE_TOUCH_MIN_LEFT_MS
}

// ------------------------------------------------------------------ JavaScript's string rules

/// `String.prototype.trim`'s whitespace.
pub fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

/// `toISOString()` of an optional time, for views.
pub fn iso_or_null(at: Option<UnixMillis>) -> Option<String> {
    at.map(iso)
}

// ------------------------------------------------------------------ module settings (`realms/realmsConfig.ts`)

/// `MYSTICPARADOX_REALMS_MODULE`: off unless `on`, `true` or `1`.
pub fn realms_module_enabled(settings: &dyn rifts_seam::Settings) -> bool {
    let value = settings.get("MYSTICPARADOX_REALMS_MODULE").unwrap_or_default().trim();
    ["on", "true", "1"].iter().any(|on| value.eq_ignore_ascii_case(on))
}

/// Technical backstops, not policy: Node embedded members and bans in one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealmLimits {
    pub max_members: u32,
    pub max_bans: u32,
    /// Invites a host may have open at once.
    pub max_active_invites: u32,
    /// A full list refuses a newcomer rather than push out someone who asked earlier.
    pub max_join_requests: u32,
    pub max_player_invites: u32,
}

pub const DEFAULT_REALM_LIMITS: RealmLimits = RealmLimits {
    max_members: 1000,
    max_bans: 1000,
    max_active_invites: 25,
    max_join_requests: 50,
    max_player_invites: 50,
};

/// 1 to 6 digits within the range, else the default (`ReadBoundedInt`).
fn bounded_int(raw: Option<&str>, fallback: u32, min: u32, max: u32) -> u32 {
    let Some(text) = raw.map(str::trim) else {
        return fallback;
    };
    if !(1..=6).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return fallback;
    }
    text.parse().ok().filter(|n| (min..=max).contains(n)).unwrap_or(fallback)
}

impl RealmLimits {
    /// `ReadRealmLimits`: members and bans are settings, the rest fixed.
    pub fn from_settings(settings: &dyn rifts_seam::Settings) -> Self {
        Self {
            max_members: bounded_int(settings.get("MYSTICPARADOX_REALMS_MAX_MEMBERS"), 1000, 2, 5000),
            max_bans: bounded_int(settings.get("MYSTICPARADOX_REALMS_MAX_BANS"), 1000, 1, 5000),
            ..DEFAULT_REALM_LIMITS
        }
    }
}
