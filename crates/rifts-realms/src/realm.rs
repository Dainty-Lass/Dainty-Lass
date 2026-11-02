// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A realm's records and its pure rules: names, invites, the join decision, online state, hosting and hub room ids.

use rand::RngExt as _;
use rifts_seam::{HubKind, Settings, UnixMillis};
use sha2::{Digest, Sha256};

use crate::time::Later as _;

/// Who may join a realm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        [Self::InviteOnly, Self::Public, Self::Solo]
            .into_iter()
            .find(|policy| policy.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        [Self::Host, Self::Member]
            .into_iter()
            .find(|role| role.as_str() == text)
    }
}

/// A realm hub's state as its realm mirrors it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        [Self::None, Self::Starting, Self::Running, Self::Stopping]
            .into_iter()
            .find(|state| state.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Usable, Self::Revoked, Self::Expired, Self::Exhausted]
            .into_iter()
            .find(|state| state.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemovalReason {
    Kick,
    Ban,
    Leave,
}

impl RemovalReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::Ban => "Ban",
            Self::Leave => "Leave",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameRefusal {
    Invalid,
    Reserved,
}

/// How an account gets into a realm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinVia {
    Member,
    Public,
    Invite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinDenial {
    Banned,
    Solo,
    InviteRequired,
    InviteInvalid,
    InviteExpired,
}

impl JoinDenial {
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

/// A join request or a player invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAt {
    pub account_id: String,
    pub at: UnixMillis,
}

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

/// One field of a patch: `Remove` is Node's `null`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Patch<T> {
    #[default]
    Keep,
    Remove,
    Set(T),
}

impl<T> Patch<T> {
    fn apply(self, current: Option<T>) -> Option<T> {
        match self {
            Self::Keep => current,
            Self::Remove => None,
            Self::Set(value) => Some(value),
        }
    }
}

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
    /// Canonical lower case; unique.
    pub name: String,
    pub host_account_id: String,
    pub host_public_key: String,
    pub join_policy: JoinPolicy,
    pub listed: bool,
    /// The host first, then in joining order.
    pub members: Vec<RealmMember>,
    pub bans: Vec<RealmBan>,
    pub created_at: UnixMillis,
    pub last_online_at: Option<UnixMillis>,
    pub online_until: Option<UnixMillis>,
    pub version: Option<String>,
    pub hosting: Option<RealmHosting>,
    pub hub_room_ids: Vec<String>,
    /// The `online_until` a host-offline notice went out for: one notice per lapse.
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

/// Which realm an account's current game login belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmPresenceRecord {
    pub account_id: String,
    pub realm_id: String,
    pub bound_at: UnixMillis,
    pub last_seen_at: UnixMillis,
    pub expires_at: UnixMillis,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatUpdate {
    pub last_online_at: UnixMillis,
    pub online_until: UnixMillis,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealmLimits {
    pub max_members: u32,
    pub max_bans: u32,
    pub max_active_invites: u32,
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

/// An invite is purged a week after it expires.
pub const INVITE_PURGE_GRACE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// No `I`, `O`, `0` or `1`, which read alike.
pub const INVITE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const INVITE_CODE_LENGTH: usize = 10;

pub fn invite_code_from(bytes: [u8; INVITE_CODE_LENGTH]) -> String {
    bytes
        .iter()
        .map(|byte| char::from(INVITE_ALPHABET[usize::from(byte & 0x1f)]))
        .collect()
}

/// 32 divides 256, so the low five bits are unbiased.
pub fn generate_invite_code() -> String {
    let mut bytes = [0u8; INVITE_CODE_LENGTH];
    rand::rng().fill(&mut bytes[..]);
    invite_code_from(bytes)
}

pub fn default_hosting(at: UnixMillis) -> RealmHosting {
    RealmHosting {
        max_servers: 1,
        max_players: 16,
        afk_seconds: None,
        hub_session_id: None,
        hub_state: HubState::None,
        training_state: HubState::None,
        active_servers: 0,
        updated_at: at,
    }
}

pub fn apply_hosting_patch(current: Option<RealmHosting>, patch: &HostingPatch, at: UnixMillis) -> RealmHosting {
    let mut hosting = current.unwrap_or_else(|| default_hosting(at));
    let patch = patch.clone();
    hosting.max_servers = patch.max_servers.unwrap_or(hosting.max_servers);
    hosting.max_players = patch.max_players.unwrap_or(hosting.max_players);
    hosting.afk_seconds = patch.afk_seconds.apply(hosting.afk_seconds);
    hosting.hub_session_id = patch.hub_session_id.apply(hosting.hub_session_id);
    hosting.hub_state = patch.hub_state.unwrap_or(hosting.hub_state);
    hosting.training_state = patch.training_state.unwrap_or(hosting.training_state);
    hosting.active_servers = patch.active_servers.unwrap_or(hosting.active_servers);
    hosting.updated_at = at;
    hosting
}

const MAX_MEMBERS_SETTING: &str = "RUSTIC_RIFTS_REALMS_MAX_MEMBERS";
const MAX_BANS_SETTING: &str = "RUSTIC_RIFTS_REALMS_MAX_BANS";

/// 1 to 6 ASCII digits within the range, trimmed; anything else is the default.
fn bounded_setting(settings: &dyn Settings, name: &str, default: u32, min: u32, max: u32) -> u32 {
    let Some(text) = settings.get(name).map(str::trim) else {
        return default;
    };
    if text.is_empty() || text.len() > 6 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return default;
    }
    text.parse::<u32>()
        .ok()
        .filter(|value| (min..=max).contains(value))
        .unwrap_or(default)
}

impl RealmLimits {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        Self {
            max_members: bounded_setting(settings, MAX_MEMBERS_SETTING, 1000, 2, 5000),
            max_bans: bounded_setting(settings, MAX_BANS_SETTING, 1000, 1, 5000),
            ..DEFAULT_REALM_LIMITS
        }
    }
}

impl Default for RealmLimits {
    fn default() -> Self {
        DEFAULT_REALM_LIMITS
    }
}

/// Matched with every `-` removed.
pub const RESERVED_REALM_NAMES: [&str; 25] = [
    "official",
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

const NAME_RAW_MAX_UNITS: usize = 256;

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub fn matches_name_pattern(name: &str) -> bool {
    (3..=24).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn is_reserved_realm_name(name: &str) -> bool {
    let bare: String = name.chars().filter(|c| *c != '-').collect();
    RESERVED_REALM_NAMES.contains(&bare.as_str())
}

pub fn normalize_realm_name(raw: &str) -> Option<String> {
    (utf16_len(raw) <= NAME_RAW_MAX_UNITS).then(|| js_trim(raw).to_lowercase())
}

fn well_formed_name(name: &str) -> bool {
    matches_name_pattern(name) && !name.starts_with('-') && !name.ends_with('-')
}

/// Format first, then the reserved list.
pub fn validate_realm_name(raw: &str) -> Result<String, NameRefusal> {
    let name = normalize_realm_name(raw).ok_or(NameRefusal::Invalid)?;
    if !well_formed_name(&name) {
        return Err(NameRefusal::Invalid);
    }
    if is_reserved_realm_name(&name) {
        return Err(NameRefusal::Reserved);
    }
    Ok(name)
}

/// An existing realm's name: a reserved one passes, since it names a realm that already has it.
pub fn existing_realm_name(raw: &str) -> Option<String> {
    normalize_realm_name(raw).filter(|name| well_formed_name(name))
}

/// The realm name a username gives, or `None`; reserved words are not refused here.
pub fn realm_name_for_username(username: &str) -> Option<String> {
    if utf16_len(username) > NAME_RAW_MAX_UNITS {
        return None;
    }
    let lower = js_trim(username).to_lowercase();
    let mut name = String::new();
    let mut gap = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !name.is_empty() {
                name.push('-');
            }
            gap = false;
            name.push(c);
        } else {
            gap = true;
        }
    }
    matches_name_pattern(&name).then_some(name)
}

pub const NEVER_ONLINE_RELEASE_MS: i64 = 3 * 24 * 60 * 60 * 1000;
pub const LAST_ONLINE_RELEASE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

pub fn realm_name_release_at(created_at: UnixMillis, last_online_at: Option<UnixMillis>) -> UnixMillis {
    match last_online_at {
        Some(at) => at.saturating_add(LAST_ONLINE_RELEASE_MS),
        None => created_at.saturating_add(NEVER_ONLINE_RELEASE_MS),
    }
}

pub fn is_realm_name_releasable(realm: &RealmRecord, now: UnixMillis) -> bool {
    now >= realm_name_release_at(realm.created_at, realm.last_online_at)
}

/// A known name whose host key no longer matches: someone else holds the name now.
pub fn detect_ownership_change(known_name: &str, known_key: &str, realm: &RealmRecord) -> bool {
    known_name.to_lowercase() == realm.name.to_lowercase() && known_key != realm.host_public_key
}

pub const INVITE_DEFAULT_MAX_USES: i64 = 10;
pub const INVITE_MAX_USES_LIMIT: i64 = 100;
pub const INVITE_DEFAULT_TTL_HOURS: i64 = 168;
pub const INVITE_MAX_TTL_HOURS: i64 = 720;

const INVITE_RAW_MAX_UNITS: usize = 40;

/// Dashes and whitespace dropped, upper-cased; then exactly ten symbols of the alphabet.
pub fn normalize_invite_code(raw: &str) -> Option<String> {
    if utf16_len(raw) > INVITE_RAW_MAX_UNITS {
        return None;
    }
    let code: String = raw
        .chars()
        .filter(|c| *c != '-' && !js_whitespace(*c))
        .collect::<String>()
        .to_uppercase();
    (code.len() == INVITE_CODE_LENGTH && code.bytes().all(|b| INVITE_ALPHABET.contains(&b))).then_some(code)
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
    code: &str,
    realm_id: &str,
    created_by: &str,
    now: UnixMillis,
    max_uses: i64,
    ttl_hours: i64,
) -> RealmInviteRecord {
    RealmInviteRecord {
        code: code.to_owned(),
        realm_id: realm_id.to_owned(),
        created_by: created_by.to_owned(),
        created_at: now,
        expires_at: now.saturating_add(ttl_hours.saturating_mul(3_600_000)),
        max_uses,
        uses: 0,
        revoked_at: None,
    }
}

/// First match wins: a ban beats membership, a public realm and a valid invite.
pub fn decide_join(
    realm: &RealmRecord,
    account: &str,
    invite: Option<&RealmInviteRecord>,
    now: UnixMillis,
) -> Result<JoinVia, JoinDenial> {
    if realm.bans.iter().any(|b| b.account_id == account) {
        return Err(JoinDenial::Banned);
    }
    if realm.join_policy == JoinPolicy::Solo && realm.host_account_id != account {
        return Err(JoinDenial::Solo);
    }
    if realm.members.iter().any(|m| m.account_id == account) {
        return Ok(JoinVia::Member);
    }
    if realm.join_policy == JoinPolicy::Public {
        return Ok(JoinVia::Public);
    }
    let Some(invite) = invite else {
        return Err(JoinDenial::InviteRequired);
    };
    if invite.realm_id != realm.realm_id {
        return Err(JoinDenial::InviteInvalid);
    }
    if invite_state(invite, now) != InviteState::Usable {
        return Err(JoinDenial::InviteExpired);
    }
    Ok(JoinVia::Invite)
}

/// Newest first; equal times keep their stored order.
pub fn sort_newest_first(entries: &[AccountAt]) -> Vec<AccountAt> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|entry| std::cmp::Reverse(entry.at));
    sorted
}

pub const HEARTBEAT_ONLINE_MS: i64 = 90_000;

pub fn is_realm_online(realm: &RealmRecord, now: UnixMillis) -> bool {
    realm.online_until.is_some_and(|until| until > now)
}

/// Training's state does not count.
pub fn is_realm_hub_ready(realm: &RealmRecord) -> bool {
    realm.hosting.as_ref().is_some_and(|h| h.hub_state == HubState::Running)
}

pub fn is_realm_serving(realm: &RealmRecord, now: UnixMillis) -> bool {
    is_realm_online(realm, now) && is_realm_hub_ready(realm)
}

pub fn apply_heartbeat(now: UnixMillis, online: Option<bool>, version: Option<String>) -> HeartbeatUpdate {
    HeartbeatUpdate {
        last_online_at: now,
        online_until: if online == Some(false) {
            now
        } else {
            now.saturating_add(HEARTBEAT_ONLINE_MS)
        },
        version,
    }
}

pub const HOSTING_MAX_SERVERS_LIMIT: i64 = 8;
pub const HOSTING_MAX_PLAYERS_LIMIT: i64 = 64;
pub const HOSTING_MAX_AFK_SECONDS: i64 = 7200;
pub const HOSTING_ACTIVE_SERVERS_LIMIT: i64 = 64;
pub const HUB_SESSION_ID_MAX_UNITS: usize = 128;

/// The first bad field by its wire name.
pub fn validate_hosting_patch(patch: &HostingPatch) -> Option<&'static str> {
    let outside = |value: Option<i64>, min: i64, max: i64| value.is_some_and(|v| !(min..=max).contains(&v));
    if outside(patch.max_servers, 1, HOSTING_MAX_SERVERS_LIMIT) {
        return Some("maxServers");
    }
    if outside(patch.max_players, 1, HOSTING_MAX_PLAYERS_LIMIT) {
        return Some("maxPlayers");
    }
    if let Patch::Set(afk) = patch.afk_seconds
        && !(0..=HOSTING_MAX_AFK_SECONDS).contains(&afk)
    {
        return Some("afkSeconds");
    }
    if outside(patch.active_servers, 0, HOSTING_ACTIVE_SERVERS_LIMIT) {
        return Some("activeServers");
    }
    if let Patch::Set(id) = &patch.hub_session_id
        && (id.is_empty() || utf16_len(id) > HUB_SESSION_ID_MAX_UNITS)
    {
        return Some("hubSessionId");
    }
    None
}

pub const PRESENCE_TTL_MS: i64 = 120_000;
/// A process trusts its own `presence_of` answer this long.
pub const PRESENCE_CACHE_MS: i64 = 2_000;
pub const PRESENCE_TOUCH_WRITE_AFTER_MS: i64 = 30_000;
pub const PRESENCE_TOUCH_MIN_LEFT_MS: i64 = 60_000;

/// Touches at most 60 s apart never let a row lapse; one every 10 s writes once per 40 s.
pub fn presence_touch_needs_write(last_seen_at: UnixMillis, expires_at: UnixMillis, now: UnixMillis) -> bool {
    now.0 - last_seen_at.0 > PRESENCE_TOUCH_WRITE_AFTER_MS || expires_at.0 - now.0 < PRESENCE_TOUCH_MIN_LEFT_MS
}

pub fn normalize_hub_room_id(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    crate::ids::session_bits(&lower).map(|_| lower)
}

const HUB_ROOM_LABEL: &str = "rustic-realm-hub:v1:";

/// What the game and chat know a realm's hub by: a name-based (version 5 shaped) UUID of the realm and kind.
pub fn hub_room_id(realm_id: &str, kind: HubKind) -> String {
    let kind = match kind {
        HubKind::Ramsgate => "RAMSGATE",
        HubKind::Training => "TRAINING",
    };
    let digest = Sha256::digest(format!("{HUB_ROOM_LABEL}{realm_id}:{kind}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    crate::ids::uuid_text(u128::from_be_bytes(bytes))
}

pub fn hub_room_ids_of(realm_id: &str) -> Vec<String> {
    vec![
        hub_room_id(realm_id, HubKind::Ramsgate),
        hub_room_id(realm_id, HubKind::Training),
    ]
}

/// Unicode `White_Space` plus the byte-order mark, as JavaScript's `trim` sees it.
pub fn js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

pub fn js_trim(text: &str) -> &str {
    text.trim_matches(js_whitespace)
}

pub fn iso_or_null(at: Option<UnixMillis>) -> Option<String> {
    at.map(crate::time::iso)
}

pub(crate) const REALMS_MODULE_SETTING: &str = "RUSTIC_RIFTS_REALMS_MODULE";

pub fn realms_module_enabled(settings: &dyn Settings) -> bool {
    let value = settings.get(REALMS_MODULE_SETTING).unwrap_or("").trim();
    ["on", "true", "1"].iter().any(|on| value.eq_ignore_ascii_case(on))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn enum_texts_round_trip() {
        for policy in [JoinPolicy::InviteOnly, JoinPolicy::Public, JoinPolicy::Solo] {
            assert_eq!(JoinPolicy::parse(policy.as_str()), Some(policy));
        }
        for role in [MemberRole::Host, MemberRole::Member] {
            assert_eq!(MemberRole::parse(role.as_str()), Some(role));
        }
        for state in [
            HubState::None,
            HubState::Starting,
            HubState::Running,
            HubState::Stopping,
        ] {
            assert_eq!(HubState::parse(state.as_str()), Some(state));
        }
        assert_eq!(JoinPolicy::parse("public"), None);
        assert_eq!(HubState::parse("BOGUS"), None);
    }

    #[test]
    fn hosting_patch_starts_from_defaults_and_merges() {
        let created = apply_hosting_patch(
            None,
            &HostingPatch {
                max_players: Some(8),
                afk_seconds: Patch::Set(300),
                ..HostingPatch::default()
            },
            UnixMillis(5),
        );
        assert_eq!(
            created,
            RealmHosting {
                max_players: 8,
                afk_seconds: Some(300),
                ..default_hosting(UnixMillis(5))
            }
        );
        let merged = apply_hosting_patch(
            Some(created),
            &HostingPatch {
                afk_seconds: Patch::Remove,
                hub_session_id: Patch::Set("hub-1".into()),
                hub_state: Some(HubState::Running),
                ..HostingPatch::default()
            },
            UnixMillis(9),
        );
        assert_eq!(merged.max_players, 8);
        assert_eq!(merged.afk_seconds, None);
        assert_eq!(merged.hub_session_id.as_deref(), Some("hub-1"));
        assert_eq!(merged.hub_state, HubState::Running);
        assert_eq!(merged.updated_at, UnixMillis(9));
    }

    #[test]
    fn invite_codes_use_the_alphabet() {
        assert_eq!(invite_code_from([0, 1, 2, 3, 31, 32, 33, 255, 8, 9]), "ABCD9AB9JK");
        let code = generate_invite_code();
        assert_eq!(code.len(), INVITE_CODE_LENGTH);
        assert!(code.bytes().all(|b| INVITE_ALPHABET.contains(&b)));
    }

    #[test]
    fn hub_room_ids_are_stable_v5_shaped_uuids() {
        let ramsgate = hub_room_id("realm-1", HubKind::Ramsgate);
        assert_eq!(ramsgate, hub_room_id("realm-1", HubKind::Ramsgate));
        assert_ne!(ramsgate, hub_room_id("realm-1", HubKind::Training));
        assert_ne!(ramsgate, hub_room_id("realm-2", HubKind::Ramsgate));
        assert!(crate::ids::session_bits(&ramsgate).is_some());
        assert_eq!(&ramsgate[14..15], "5");
        assert_eq!(hub_room_ids_of("realm-1")[0], ramsgate);
    }

    #[test]
    fn realms_module_switch() {
        let with =
            |value: &str| -> BTreeMap<String, String> { [(REALMS_MODULE_SETTING.to_owned(), value.to_owned())].into() };
        for on in ["on", " TRUE ", "1"] {
            assert!(realms_module_enabled(&with(on)), "{on}");
        }
        for off in ["off", "yes", "", "0"] {
            assert!(!realms_module_enabled(&with(off)), "{off}");
        }
        assert!(!realms_module_enabled(&BTreeMap::new()));
    }
}
