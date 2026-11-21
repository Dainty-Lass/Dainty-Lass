// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A realm's records and its pure rules: names, invites, the join decision, online state, hosting and hub room ids.

use rand::RngExt as _;
use rustic_rifts_seam::{HubKind, Settings, UnixMillis};
use sha2::{Digest, Sha256};

use crate::time::Later as _;
use crate::wire::wire_text;

/// Who may join a realm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JoinPolicy {
    /// Members and holders of a valid invite.
    InviteOnly,
    /// Anyone not banned.
    Public,
    /// The host alone.
    Solo,
}

wire_text!(impl JoinPolicy { as_str, parse }
    InviteOnly => "INVITE_ONLY",
    Public => "PUBLIC",
    Solo => "SOLO",
);

/// A member's role in a realm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MemberRole {
    /// The realm's host.
    Host,
    /// Any other member.
    Member,
}

wire_text!(impl MemberRole { as_str, parse }
    Host => "HOST",
    Member => "MEMBER",
);

/// A realm hub's state as its realm mirrors it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HubState {
    /// No hub session.
    None,
    /// The hub is starting.
    Starting,
    /// The hub is up.
    Running,
    /// The hub is shutting down.
    Stopping,
}

wire_text!(impl HubState { as_str, parse }
    None => "NONE",
    Starting => "STARTING",
    Running => "RUNNING",
    Stopping => "STOPPING",
);

/// Whether a realm invite can still be used.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InviteState {
    /// It can be used.
    Usable,
    /// The realm revoked it.
    Revoked,
    /// Its time ran out.
    Expired,
    /// It reached its maximum uses.
    Exhausted,
}

wire_text!(impl InviteState { as_str, parse }
    Usable => "USABLE",
    Revoked => "REVOKED",
    Expired => "EXPIRED",
    Exhausted => "EXHAUSTED",
);

/// Why a member left a realm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RemovalReason {
    /// The host removed them.
    Kick,
    /// The host banned them.
    Ban,
    /// They left.
    Leave,
}

// The event payload spells these in title case.
wire_text!(impl RemovalReason { as_str }
    Kick => "Kick",
    Ban => "Ban",
    Leave => "Leave",
);

/// Why a realm name was refused.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NameRefusal {
    /// Not 3 to 24 of `a-z`, `0-9` and `-`, with no dash at either end.
    Invalid,
    /// On the reserved list.
    Reserved,
}

/// How an account gets into a realm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JoinVia {
    /// Already a member.
    Member,
    /// The realm is public.
    Public,
    /// With a valid invite.
    Invite,
}

/// Why an account may not join a realm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JoinDenial {
    /// The account is banned.
    Banned,
    /// The realm is solo and the account is not its host.
    Solo,
    /// The realm is invite-only and no invite was given.
    InviteRequired,
    /// The invite is for another realm.
    InviteInvalid,
    /// The invite was revoked, expired or used up.
    InviteExpired,
}

wire_text!(impl JoinDenial { code }
    Banned => "REALM_BANNED",
    Solo => "REALM_SOLO",
    InviteRequired => "REALM_INVITE_REQUIRED",
    InviteInvalid => "REALM_INVITE_INVALID",
    InviteExpired => "REALM_INVITE_EXPIRED",
);

/// One member of a realm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmMember {
    /// The member's account.
    pub account_id: String,
    /// Host or member.
    pub role: MemberRole,
    /// When they joined.
    pub joined_at: UnixMillis,
}

/// One ban from a realm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmBan {
    /// The banned account.
    pub account_id: String,
    /// The host's reason, if given.
    pub reason: Option<String>,
    /// When the ban was made.
    pub at: UnixMillis,
}

/// A join request or a player invite.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountAt {
    /// The account.
    pub account_id: String,
    /// When it was made.
    pub at: UnixMillis,
}

/// How a realm's host runs its servers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmHosting {
    /// The most hunt servers the realm runs at once, up to [`HOSTING_MAX_SERVERS_LIMIT`].
    pub max_servers: i64,
    /// The most players in the hub, up to [`HOSTING_MAX_PLAYERS_LIMIT`].
    pub max_players: i64,
    /// How long an idle player may stay in the hub, in seconds; no limit when unset.
    pub afk_seconds: Option<i64>,
    /// The hub's current session, if one runs.
    pub hub_session_id: Option<String>,
    /// The Ramsgate hub's state.
    pub hub_state: HubState,
    /// The training hub's state.
    pub training_state: HubState,
    /// How many hunt servers run now.
    pub active_servers: i64,
    /// When the settings last changed.
    pub updated_at: UnixMillis,
}

/// One field of a patch: `Remove` is Node's `null`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Patch<T> {
    /// Leave the field as it is.
    #[default]
    Keep,
    /// Clear the field.
    Remove,
    /// Set the field to this value.
    Set(T),
}

impl<T> Patch<T> {
    fn apply(self, current: Option<T>) -> Option<T> {
        match (self, current) {
            (Self::Set(value), _) => Some(value),
            (Self::Keep, kept) => kept,
            (Self::Remove, _) => None,
        }
    }
}

/// A change to a realm's hosting settings; `None` and [`Patch::Keep`] leave a field as it is.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostingPatch {
    /// New maximum hunt servers.
    pub max_servers: Option<i64>,
    /// New maximum hub players.
    pub max_players: Option<i64>,
    /// New idle limit, in seconds.
    pub afk_seconds: Patch<i64>,
    /// New hub session.
    pub hub_session_id: Patch<String>,
    /// New Ramsgate hub state.
    pub hub_state: Option<HubState>,
    /// New training hub state.
    pub training_state: Option<HubState>,
    /// New running hunt server count.
    pub active_servers: Option<i64>,
}

/// A realm as the directory stores it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmRecord {
    /// The realm's id.
    pub realm_id: String,
    /// Canonical lower case; unique.
    pub name: String,
    /// The host's account.
    pub host_account_id: String,
    /// The host's public key, which proves who holds the name.
    pub host_public_key: String,
    /// Who may join.
    pub join_policy: JoinPolicy,
    /// Whether the realm is listed publicly.
    pub listed: bool,
    /// The host first, then in joining order.
    pub members: Vec<RealmMember>,
    /// The bans.
    pub bans: Vec<RealmBan>,
    /// When the realm was made.
    pub created_at: UnixMillis,
    /// When the host was last online, if ever.
    pub last_online_at: Option<UnixMillis>,
    /// Until when the realm counts as online, renewed by the host's heartbeat.
    pub online_until: Option<UnixMillis>,
    /// The host's reported version.
    pub version: Option<String>,
    /// The hosting settings, once the host set them.
    pub hosting: Option<RealmHosting>,
    /// The game room ids of the realm's hubs.
    pub hub_room_ids: Vec<String>,
    /// The `online_until` a host-offline notice went out for: one notice per lapse.
    pub offline_notified_for: Option<UnixMillis>,
    /// Pending requests to join.
    pub join_requests: Vec<AccountAt>,
    /// Players the host invited.
    pub player_invites: Vec<AccountAt>,
}

/// A realm invite code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmInviteRecord {
    /// The code, ten symbols of [`INVITE_ALPHABET`].
    pub code: String,
    /// The realm it admits to.
    pub realm_id: String,
    /// The account that made it.
    pub created_by: String,
    /// When it was made.
    pub created_at: UnixMillis,
    /// When it expires.
    pub expires_at: UnixMillis,
    /// How many joins it allows.
    pub max_uses: i64,
    /// How many joins it gave.
    pub uses: i64,
    /// When it was revoked, if it was.
    pub revoked_at: Option<UnixMillis>,
}

/// Which realm an account's current game login belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealmPresenceRecord {
    /// The account.
    pub account_id: String,
    /// The realm it is in.
    pub realm_id: String,
    /// When the login was bound to the realm.
    pub bound_at: UnixMillis,
    /// When the account was last seen.
    pub last_seen_at: UnixMillis,
    /// When the row lapses unless touched.
    pub expires_at: UnixMillis,
}

/// Something that happened to a realm, sent to the players it concerns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RealmEvent {
    /// A member was removed.
    MemberRemoved {
        /// The realm.
        realm_id: String,
        /// The member removed.
        account_id: String,
        /// Why.
        reason: RemovalReason,
    },
    /// The realm's name was released.
    RealmReleased {
        /// The realm.
        realm_id: String,
        /// The host who held it.
        host_account_id: String,
    },
    /// The host went offline.
    HostOffline {
        /// The realm.
        realm_id: String,
        /// The host.
        host_account_id: String,
    },
}

/// What a host heartbeat writes to its realm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeartbeatUpdate {
    /// When the host was last online.
    pub last_online_at: UnixMillis,
    /// Until when the realm counts as online.
    pub online_until: UnixMillis,
    /// The host's reported version.
    pub version: Option<String>,
}

/// How many of each thing a realm may hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RealmLimits {
    /// Members, the host included.
    pub max_members: u32,
    /// Bans.
    pub max_bans: u32,
    /// Usable invite codes.
    pub max_active_invites: u32,
    /// Pending join requests.
    pub max_join_requests: u32,
    /// Player invites.
    pub max_player_invites: u32,
}

/// The limits when no setting changes them.
pub const DEFAULT_REALM_LIMITS: RealmLimits = RealmLimits {
    max_player_invites: 50,
    max_join_requests: 50,
    max_active_invites: 25,
    max_bans: 1000,
    max_members: 1000,
};

/// An invite is purged a week after it expires.
pub const INVITE_PURGE_GRACE_MS: i64 = 7 * DAY_MS;

/// No `I`, `O`, `0` or `1`, which read alike.
pub const INVITE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Symbols in an invite code.
pub const INVITE_CODE_LENGTH: usize = 10;

/// The invite code these bytes give, each byte's low five bits picking a symbol.
pub fn invite_code_from(bytes: [u8; INVITE_CODE_LENGTH]) -> String {
    let symbol = |byte: u8| INVITE_ALPHABET[usize::from(byte % 32)] as char;
    bytes.into_iter().map(symbol).collect()
}

/// 32 divides 256, so the low five bits are unbiased.
pub fn generate_invite_code() -> String {
    let mut random = [0u8; INVITE_CODE_LENGTH];
    rand::rng().fill(random.as_mut_slice());
    invite_code_from(random)
}

/// Hosting settings with no hub running and nothing set.
pub fn default_hosting(at: UnixMillis) -> RealmHosting {
    RealmHosting {
        updated_at: at,
        hub_state: HubState::None,
        training_state: HubState::None,
        hub_session_id: None,
        afk_seconds: None,
        active_servers: 0,
        max_players: 16,
        max_servers: 1,
    }
}

/// `current` (or the defaults) with the patch applied and `updated_at` set to `at`.
pub fn apply_hosting_patch(current: Option<RealmHosting>, patch: &HostingPatch, at: UnixMillis) -> RealmHosting {
    let HostingPatch {
        max_servers,
        max_players,
        afk_seconds,
        hub_session_id,
        hub_state,
        training_state,
        active_servers,
    } = patch.clone();
    let base = current.unwrap_or_else(|| default_hosting(at));
    RealmHosting {
        updated_at: at,
        afk_seconds: afk_seconds.apply(base.afk_seconds),
        hub_session_id: hub_session_id.apply(base.hub_session_id),
        max_servers: max_servers.unwrap_or(base.max_servers),
        max_players: max_players.unwrap_or(base.max_players),
        active_servers: active_servers.unwrap_or(base.active_servers),
        hub_state: hub_state.unwrap_or(base.hub_state),
        training_state: training_state.unwrap_or(base.training_state),
    }
}

const MAX_MEMBERS_SETTING: &str = "RUSTIC_RIFTS_REALMS_MAX_MEMBERS";
const MAX_BANS_SETTING: &str = "RUSTIC_RIFTS_REALMS_MAX_BANS";

/// 1 to 6 ASCII digits within `range`, trimmed; anything else is the default.
fn bounded_setting(settings: &dyn Settings, name: &str, default: u32, range: std::ops::RangeInclusive<u32>) -> u32 {
    settings
        .get(name)
        .map(str::trim)
        .filter(|text| (1..=6).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse::<u32>().ok())
        .filter(|value| range.contains(value))
        .unwrap_or(default)
}

impl RealmLimits {
    /// Reads the member and ban limits from the settings; the rest stay at their defaults.
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let max_members = bounded_setting(settings, MAX_MEMBERS_SETTING, 1000, 2..=5000);
        let max_bans = bounded_setting(settings, MAX_BANS_SETTING, 1000, 1..=5000);
        Self {
            max_bans,
            max_members,
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

/// Longest raw name, in UTF-16 units as JavaScript counts `length`.
const NAME_RAW_MAX_UNITS: usize = 256;

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn name_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-')
}

/// Whether `name` is 3 to 24 of `a-z`, `0-9` and `-`.
pub fn matches_name_pattern(name: &str) -> bool {
    let length_ok = (3..=24).contains(&name.len());
    length_ok && name.bytes().all(name_byte)
}

/// Whether `name`, dashes removed, is on [`RESERVED_REALM_NAMES`].
pub fn is_reserved_realm_name(name: &str) -> bool {
    let undashed = name.replace('-', "");
    RESERVED_REALM_NAMES.iter().any(|reserved| *reserved == undashed)
}

/// The name trimmed and lower-cased; `None` when the raw text is longer than 256 UTF-16 units.
pub fn normalize_realm_name(raw: &str) -> Option<String> {
    if utf16_len(raw) > NAME_RAW_MAX_UNITS {
        return None;
    }
    Some(js_trim(raw).to_lowercase())
}

fn well_formed_name(name: &str) -> bool {
    let edged = name.starts_with('-') || name.ends_with('-');
    !edged && matches_name_pattern(name)
}

/// Format first, then the reserved list.
pub fn validate_realm_name(raw: &str) -> Result<String, NameRefusal> {
    match normalize_realm_name(raw) {
        Some(name) if well_formed_name(&name) => {
            if is_reserved_realm_name(&name) {
                Err(NameRefusal::Reserved)
            } else {
                Ok(name)
            }
        }
        _ => Err(NameRefusal::Invalid),
    }
}

/// An existing realm's name: a reserved one passes, since it names a realm that already has it.
pub fn existing_realm_name(raw: &str) -> Option<String> {
    let name = normalize_realm_name(raw)?;
    well_formed_name(&name).then_some(name)
}

/// The realm name a username gives, or `None`; reserved words are not refused here.
pub fn realm_name_for_username(username: &str) -> Option<String> {
    let lowered = normalize_realm_name(username)?;
    // Runs of anything else become one `-`, never leading or trailing.
    let words: Vec<String> = lowered
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let name = words.join("-");
    matches_name_pattern(&name).then_some(name)
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// How long a realm whose host never came online keeps its name.
pub const NEVER_ONLINE_RELEASE_MS: i64 = 3 * DAY_MS;
/// How long a realm keeps its name after its host was last online.
pub const LAST_ONLINE_RELEASE_MS: i64 = 30 * DAY_MS;

/// When the realm's name may be released.
pub fn realm_name_release_at(created_at: UnixMillis, last_online_at: Option<UnixMillis>) -> UnixMillis {
    last_online_at.map_or_else(
        || created_at.saturating_add(NEVER_ONLINE_RELEASE_MS),
        |online| online.saturating_add(LAST_ONLINE_RELEASE_MS),
    )
}

/// Whether the realm's name may be released now.
pub fn is_realm_name_releasable(realm: &RealmRecord, now: UnixMillis) -> bool {
    realm_name_release_at(realm.created_at, realm.last_online_at) <= now
}

/// A known name whose host key no longer matches: someone else holds the name now.
pub fn detect_ownership_change(known_name: &str, known_key: &str, realm: &RealmRecord) -> bool {
    let same_name = known_name.to_lowercase() == realm.name.to_lowercase();
    same_name && realm.host_public_key != known_key
}

/// Joins an invite allows when none is asked for.
pub const INVITE_DEFAULT_MAX_USES: i64 = 10;
/// The most joins an invite may allow.
pub const INVITE_MAX_USES_LIMIT: i64 = 100;
/// An invite's lifetime when none is asked for, in hours.
pub const INVITE_DEFAULT_TTL_HOURS: i64 = 168;
/// The longest an invite may last, in hours.
pub const INVITE_MAX_TTL_HOURS: i64 = 720;

const INVITE_RAW_MAX_UNITS: usize = 40;
const HOUR_MS: i64 = 3_600_000;

/// Dashes and whitespace dropped, upper-cased; then exactly ten symbols of the alphabet.
pub fn normalize_invite_code(raw: &str) -> Option<String> {
    if utf16_len(raw) > INVITE_RAW_MAX_UNITS {
        return None;
    }
    let kept: String = raw.chars().filter(|c| !(*c == '-' || js_whitespace(*c))).collect();
    let code = kept.to_uppercase();
    let in_alphabet = code.bytes().all(|b| INVITE_ALPHABET.contains(&b));
    (in_alphabet && code.len() == INVITE_CODE_LENGTH).then_some(code)
}

/// Whether the invite can still be used: revoked, then expired, then exhausted.
pub fn invite_state(invite: &RealmInviteRecord, now: UnixMillis) -> InviteState {
    let ended = [
        (invite.revoked_at.is_some(), InviteState::Revoked),
        (invite.expires_at <= now, InviteState::Expired),
        (invite.max_uses <= invite.uses, InviteState::Exhausted),
    ];
    ended
        .into_iter()
        .find_map(|(hit, state)| hit.then_some(state))
        .unwrap_or(InviteState::Usable)
}

/// A fresh, unused invite lasting `ttl_hours` from `now`.
pub fn build_invite(
    code: &str,
    realm_id: &str,
    created_by: &str,
    now: UnixMillis,
    max_uses: i64,
    ttl_hours: i64,
) -> RealmInviteRecord {
    let lifetime = ttl_hours.saturating_mul(HOUR_MS);
    RealmInviteRecord {
        expires_at: now.saturating_add(lifetime),
        created_at: now,
        revoked_at: None,
        uses: 0,
        max_uses,
        code: code.into(),
        realm_id: realm_id.into(),
        created_by: created_by.into(),
    }
}

/// First match wins: a ban beats membership, a public realm and a valid invite.
pub fn decide_join(
    realm: &RealmRecord,
    account: &str,
    invite: Option<&RealmInviteRecord>,
    now: UnixMillis,
) -> Result<JoinVia, JoinDenial> {
    let banned = realm.bans.iter().any(|ban| ban.account_id == account);
    let solo_guest = realm.join_policy == JoinPolicy::Solo && account != realm.host_account_id;
    let member = realm.members.iter().any(|m| m.account_id == account);
    match (banned, solo_guest, member, realm.join_policy, invite) {
        (true, ..) => Err(JoinDenial::Banned),
        (_, true, ..) => Err(JoinDenial::Solo),
        (_, _, true, ..) => Ok(JoinVia::Member),
        (.., JoinPolicy::Public, _) => Ok(JoinVia::Public),
        (.., None) => Err(JoinDenial::InviteRequired),
        (.., Some(code)) if code.realm_id != realm.realm_id => Err(JoinDenial::InviteInvalid),
        (.., Some(code)) if invite_state(code, now) != InviteState::Usable => Err(JoinDenial::InviteExpired),
        _ => Ok(JoinVia::Invite),
    }
}

/// Newest first; equal times keep their stored order.
pub fn sort_newest_first(entries: &[AccountAt]) -> Vec<AccountAt> {
    let mut newest = entries.to_vec();
    // Stable, so ties keep their place.
    newest.sort_by_key(|item| std::cmp::Reverse(item.at.0));
    newest
}

/// How long one heartbeat keeps a realm online, in milliseconds.
pub const HEARTBEAT_ONLINE_MS: i64 = 90_000;

/// Whether the realm is in its online window.
pub fn is_realm_online(realm: &RealmRecord, now: UnixMillis) -> bool {
    matches!(realm.online_until, Some(until) if now < until)
}

/// Training's state does not count.
pub fn is_realm_hub_ready(realm: &RealmRecord) -> bool {
    matches!(&realm.hosting, Some(hosting) if hosting.hub_state == HubState::Running)
}

/// Whether the realm's hub is running and its host online.
pub fn is_realm_serving(realm: &RealmRecord, now: UnixMillis) -> bool {
    is_realm_hub_ready(realm) && is_realm_online(realm, now)
}

/// The update a heartbeat at `now` writes; `online: false` ends the online window now.
pub fn apply_heartbeat(now: UnixMillis, online: Option<bool>, version: Option<String>) -> HeartbeatUpdate {
    // An explicit `online: false` ends the online window now.
    let window = if online == Some(false) { 0 } else { HEARTBEAT_ONLINE_MS };
    HeartbeatUpdate {
        version,
        last_online_at: now,
        online_until: now.saturating_add(window),
    }
}

/// The most hunt servers a realm may set.
pub const HOSTING_MAX_SERVERS_LIMIT: i64 = 8;
/// The most hub players a realm may set.
pub const HOSTING_MAX_PLAYERS_LIMIT: i64 = 64;
/// The longest idle limit a realm may set, in seconds.
pub const HOSTING_MAX_AFK_SECONDS: i64 = 7200;
/// The most running hunt servers a host may report.
pub const HOSTING_ACTIVE_SERVERS_LIMIT: i64 = 64;
/// The longest hub session id, in UTF-16 units.
pub const HUB_SESSION_ID_MAX_UNITS: usize = 128;

/// The first bad field by its wire name.
pub fn validate_hosting_patch(patch: &HostingPatch) -> Option<&'static str> {
    let out_of = |value: Option<i64>, low: i64, high: i64| matches!(value, Some(v) if v < low || v > high);
    let afk = match patch.afk_seconds {
        Patch::Set(seconds) => Some(seconds),
        Patch::Keep | Patch::Remove => None,
    };
    let bad_hub_id =
        matches!(&patch.hub_session_id, Patch::Set(id) if id.is_empty() || utf16_len(id) > HUB_SESSION_ID_MAX_UNITS);
    let checks = [
        ("maxServers", out_of(patch.max_servers, 1, HOSTING_MAX_SERVERS_LIMIT)),
        ("maxPlayers", out_of(patch.max_players, 1, HOSTING_MAX_PLAYERS_LIMIT)),
        ("afkSeconds", out_of(afk, 0, HOSTING_MAX_AFK_SECONDS)),
        (
            "activeServers",
            out_of(patch.active_servers, 0, HOSTING_ACTIVE_SERVERS_LIMIT),
        ),
        ("hubSessionId", bad_hub_id),
    ];
    checks.into_iter().find_map(|(field, bad)| bad.then_some(field))
}

/// How long a realm presence row lives without a touch, in milliseconds.
pub const PRESENCE_TTL_MS: i64 = 120_000;
/// A process trusts its own `presence_of` answer this long.
pub const PRESENCE_CACHE_MS: i64 = 2_000;
/// A touch writes when the row was last written longer ago than this, in milliseconds.
pub const PRESENCE_TOUCH_WRITE_AFTER_MS: i64 = 30_000;
/// A touch writes when the row has less than this left, in milliseconds.
pub const PRESENCE_TOUCH_MIN_LEFT_MS: i64 = 60_000;

/// Touches at most 60 s apart never let a row lapse; one every 10 s writes once per 40 s.
pub fn presence_touch_needs_write(last_seen_at: UnixMillis, expires_at: UnixMillis, now: UnixMillis) -> bool {
    let (since_seen, left) = (now.0 - last_seen_at.0, expires_at.0 - now.0);
    left < PRESENCE_TOUCH_MIN_LEFT_MS || since_seen > PRESENCE_TOUCH_WRITE_AFTER_MS
}

/// The room id lower-cased, or `None` when it is not an `8-4-4-4-12` hex UUID.
pub fn normalize_hub_room_id(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    crate::ids::session_bits(&lower)?;
    Some(lower)
}

const HUB_ROOM_LABEL: &str = "rustic-realm-hub:v1:";

/// What the game and chat know a realm's hub by: a name-based (version 5 shaped) UUID of the realm and kind.
pub fn hub_room_id(realm_id: &str, kind: HubKind) -> String {
    let seed = format!("{HUB_ROOM_LABEL}{realm_id}:{}", crate::hunt::hub_kind_name(kind));
    let digest = Sha256::digest(seed.as_bytes());
    let mut head = [0u8; 16];
    head.copy_from_slice(&digest[..16]);
    // RFC 4122 version 5 and variant bits.
    head[6] = 0x50 | (head[6] & 0x0f);
    head[8] = 0x80 | (head[8] & 0x3f);
    crate::ids::uuid_text(u128::from_be_bytes(head))
}

/// The room ids of the realm's two hubs, Ramsgate then training.
pub fn hub_room_ids_of(realm_id: &str) -> Vec<String> {
    [HubKind::Ramsgate, HubKind::Training]
        .into_iter()
        .map(|kind| hub_room_id(realm_id, kind))
        .collect()
}

/// Unicode `White_Space` plus the byte-order mark, as JavaScript's `trim` sees it.
pub fn js_whitespace(c: char) -> bool {
    c == '\u{feff}' || c.is_whitespace()
}

/// `String.prototype.trim`: strips JavaScript's whitespace, the BOM included.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(js_whitespace)
}

/// The time as an ISO 8601 string, or `None`.
pub fn iso_or_null(at: Option<UnixMillis>) -> Option<String> {
    at.map(crate::time::iso)
}

pub(crate) const REALMS_MODULE_SETTING: &str = "RUSTIC_RIFTS_REALMS_MODULE";

/// Whether `RUSTIC_RIFTS_REALMS_MODULE` turns realms on; off unless `on`, `true` or `1`.
pub fn realms_module_enabled(settings: &dyn Settings) -> bool {
    let Some(value) = settings.get(REALMS_MODULE_SETTING) else {
        return false;
    };
    let value = value.trim().to_ascii_lowercase();
    matches!(value.as_str(), "on" | "true" | "1")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn enum_texts_round_trip() {
        let policies = [JoinPolicy::InviteOnly, JoinPolicy::Public, JoinPolicy::Solo];
        assert!(policies.iter().all(|p| JoinPolicy::parse(p.as_str()) == Some(*p)));
        let roles = [MemberRole::Host, MemberRole::Member];
        assert!(roles.iter().all(|r| MemberRole::parse(r.as_str()) == Some(*r)));
        let hub_states = [
            HubState::None,
            HubState::Starting,
            HubState::Running,
            HubState::Stopping,
        ];
        assert!(hub_states.iter().all(|s| HubState::parse(s.as_str()) == Some(*s)));
        assert!(JoinPolicy::parse("public").is_none());
        assert!(HubState::parse("BOGUS").is_none());
    }

    #[test]
    fn hosting_patch_starts_from_defaults_and_merges() {
        let first = HostingPatch {
            afk_seconds: Patch::Set(300),
            max_players: Some(8),
            ..Default::default()
        };
        let created = apply_hosting_patch(None, &first, UnixMillis(5));
        let mut expected = default_hosting(UnixMillis(5));
        expected.max_players = 8;
        expected.afk_seconds = Some(300);
        assert_eq!(created, expected);

        let second = HostingPatch {
            hub_state: Some(HubState::Running),
            hub_session_id: Patch::Set("hub-1".into()),
            afk_seconds: Patch::Remove,
            ..Default::default()
        };
        let merged = apply_hosting_patch(Some(created), &second, UnixMillis(9));
        assert_eq!(
            (merged.max_players, merged.afk_seconds, merged.hub_session_id.as_deref()),
            (8, None, Some("hub-1"))
        );
        assert_eq!(
            (merged.hub_state, merged.updated_at),
            (HubState::Running, UnixMillis(9))
        );
    }

    #[test]
    fn invite_codes_use_the_alphabet() {
        let fixed = invite_code_from([0, 1, 2, 3, 31, 32, 33, 255, 8, 9]);
        assert_eq!(fixed, "ABCD9AB9JK");
        let random = generate_invite_code();
        assert_eq!(random.len(), INVITE_CODE_LENGTH);
        assert!(random.bytes().all(|b| INVITE_ALPHABET.contains(&b)));
    }

    #[test]
    fn hub_room_ids_are_stable_v5_shaped_uuids() {
        let id = |realm: &str, kind| hub_room_id(realm, kind);
        let ramsgate = id("realm-1", HubKind::Ramsgate);
        assert_eq!(id("realm-1", HubKind::Ramsgate), ramsgate);
        for other in [id("realm-1", HubKind::Training), id("realm-2", HubKind::Ramsgate)] {
            assert_ne!(other, ramsgate);
        }
        assert!(crate::ids::session_bits(&ramsgate).is_some());
        assert_eq!(ramsgate.as_bytes()[14], b'5');
        assert_eq!(hub_room_ids_of("realm-1").first(), Some(&ramsgate));
    }

    #[test]
    fn realms_module_switch() {
        let settings = |value: &str| BTreeMap::from([(REALMS_MODULE_SETTING.to_owned(), value.to_owned())]);
        for (value, on) in [
            ("on", true),
            (" TRUE ", true),
            ("1", true),
            ("off", false),
            ("yes", false),
            ("", false),
            ("0", false),
        ] {
            assert_eq!(realms_module_enabled(&settings(value)), on, "{value:?}");
        }
        assert!(!realms_module_enabled(&BTreeMap::new()));
    }
}
