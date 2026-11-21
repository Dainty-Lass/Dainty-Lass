// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher-facing launcher API (public spec §3.3, §5): registration, the account key's holder, the server
//! status and its instances, and the rules the host routes share.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::response::Response;
use http::{HeaderMap, StatusCode};
use rustic_isles::{AccountId, Call, Shared, UserId};
use rustic_isles_store::repo::auth::{
    AccountRecord, ApprovalStatus, CreateOutcome, LauncherAccountRecord, OperationalStatus,
};
use rustic_isles_store::{IslesStore, RepoError};
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::state::{LauncherApiState, StatusView};
use crate::util::{self, field, iso, json, now_ms, object, store_of, trim, utf16_len};

const CODE_ALPHABET: &[u8; 31] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const DIRECTOR_TIMEOUT: Duration = Duration::from_secs(2);
const DIRECTOR_MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_INSTANCES: usize = 100;
const MAX_PLAYERS: usize = 500;

// ------------------------------------------------------------------ rules (§5.1)

/// Who may register (`REGISTRATION_MODE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationMode {
    /// Anyone.
    Open,
    /// Only with an invite code; the default.
    InviteCode,
    /// Nobody.
    None,
}

impl RegistrationMode {
    /// Reads `REGISTRATION_MODE`, trimmed, any case; anything unknown is invite-only.
    pub fn from_settings(settings: &rustic_isles::Settings) -> Self {
        let raw = settings.raw("REGISTRATION_MODE").unwrap_or("");
        match raw.trim().to_uppercase().as_str() {
            "OPEN" => Self::Open,
            "NONE" => Self::None,
            _ => Self::InviteCode,
        }
    }

    /// The mode as `RegistrationStatus` answers it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::InviteCode => "INVITECODE",
            Self::None => "NONE",
        }
    }
}

/// A JSON string, JS-trimmed, 3 to 16 UTF-16 units of ASCII letters, digits and `_`.
pub fn normalize_username(value: Option<&Value>) -> Option<String> {
    let name = trim(value?.as_str()?);
    let ok = (3..=16).contains(&utf16_len(name)) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    ok.then(|| name.to_owned())
}

/// 4 to 64 bytes of ASCII letters, digits and `-`.
pub fn is_invite_code(text: &str) -> bool {
    (4..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Three groups of four from the alphabet; `pick(n)` answers an index below `n`.
pub fn make_invite_code_with(mut pick: impl FnMut(usize) -> usize) -> String {
    let mut groups = Vec::with_capacity(3);
    for _ in 0..3 {
        let group: String = (0..4)
            .map(|_| char::from(CODE_ALPHABET[pick(CODE_ALPHABET.len()) % CODE_ALPHABET.len()]))
            .collect();
        groups.push(group);
    }
    groups.join("-")
}

/// A uniform index below `n` by rejection (no modulo bias).
pub fn uniform_index(n: usize) -> usize {
    let n = u32::try_from(n).unwrap_or(u32::MAX).max(1);
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let value = u32::from_le_bytes(util::random_bytes::<4>());
        if value < zone {
            return (value % n) as usize;
        }
    }
}

/// A fresh invite code: three groups of four characters joined with `-`.
pub fn make_invite_code() -> String {
    make_invite_code_with(uniform_index)
}

/// A string with control characters as spaces, JS-trimmed, its first 64 characters; empty or not a string is none.
pub fn invite_note(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let cleaned = util::controls_to_spaces(text);
    let note = util::take_chars(trim(&cleaned), 64);
    (!note.is_empty()).then_some(note)
}

/// `DLK_` and 48 lower-case hex digits.
pub fn make_account_key() -> String {
    format!("DLK_{}", util::random_hex::<24>())
}

/// 8 to 128 bytes of ASCII letters, digits, `_`, `-`.
pub fn looks_like_account_key(text: &str) -> bool {
    (8..=128).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Active, approved (or unset) and not stored without a username; timeouts are not consulted.
pub fn is_account_eligible(account: &LauncherAccountRecord) -> bool {
    account.status == OperationalStatus::Active
        && account.approval_status.is_none_or(|a| a == ApprovalStatus::Approved)
        && account.username_set != Some(false)
}

/// Why a launcher API call was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The body is not JSON, or a field is missing or wrong.
    BadRequest,
    /// The invite code is unknown or used up.
    InviteInvalid,
    /// No account has that name.
    NotFound,
    /// Registration is off.
    RegistrationClosed,
    /// The username breaks the rules.
    UsernameInvalid,
    /// The username is taken.
    UsernameTaken,
    /// Accounts are made only through the server's OAuth sign-in.
    OAuthRequired,
}

impl Refusal {
    /// The error code in the body.
    pub fn code(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::InviteInvalid => "invite_invalid",
            Self::NotFound => "not_found",
            Self::RegistrationClosed => "registration_closed",
            Self::UsernameInvalid => "username_invalid",
            Self::UsernameTaken => "username_taken",
            Self::OAuthRequired => "oauth_required",
        }
    }

    /// The message in the body.
    pub fn message(self) -> &'static str {
        match self {
            Self::BadRequest => "The body is not JSON, or a field is missing or has the wrong type.",
            Self::InviteInvalid => "That invite code is not valid, or has been used up.",
            Self::NotFound => "No account has that name.",
            Self::RegistrationClosed => "This server is not taking registrations.",
            Self::UsernameInvalid => "A username is 3 to 16 characters: letters, digits and underscore.",
            Self::UsernameTaken => "That name is taken.",
            Self::OAuthRequired => "This server makes new accounts only through its sign-in.",
        }
    }

    /// `{"error","message"}`.
    pub fn body(self) -> Value {
        object(vec![
            ("error", Value::from(self.code())),
            ("message", Value::from(self.message())),
        ])
    }

    /// The body with the status.
    pub fn response(self, status: StatusCode) -> Response {
        json(status, &self.body())
    }
}

// ------------------------------------------------------------------ the account key's holder (§3.3)

/// The holder of an account key.
#[derive(Debug, Clone)]
pub struct Holder {
    /// The game account.
    pub account: AccountId,
    /// The game account's userId, else the account id text.
    pub user_id: String,
    /// The display name, else the game name, else the userId.
    pub username: String,
    /// Whether the launcher account has the `admin` role.
    pub admin: bool,
    /// The launcher account, if any.
    pub launcher: Option<LauncherAccountRecord>,
}

impl Holder {
    /// Whether the holder may use the key: no launcher account, or an eligible one.
    pub fn eligible(&self) -> bool {
        self.launcher.as_ref().is_none_or(is_account_eligible)
    }
}

/// Who holds the presented account key; a store failure is no holder.
pub async fn key_holder(shared: &Shared, store: &IslesStore, headers: &HeaderMap) -> Option<(Holder, String)> {
    let key = util::header_lossy(headers, "x-dainty-user-api-key")?;
    let holder = holder_of_key(shared, store, &key).await.ok().flatten()?;
    Some((holder, key))
}

/// Who holds an account key, by its stored HMAC; `Err` is a store or secret failure, not an unknown key.
pub async fn holder_of_key(shared: &Shared, store: &IslesStore, key: &str) -> Result<Option<Holder>, String> {
    if !looks_like_account_key(key) {
        return Ok(None);
    }
    let secret = rustic_isles_auth::hashes::read_secret(shared.settings().raw("API_KEY_HASH_SECRET"))
        .map_err(|e| e.to_string())?;
    let hash = rustic_isles_auth::hashes::user_key_hash(secret, key);
    store
        .read(move |conn, repos| {
            let wanted = util::hex_lenient(&hash);
            let mut found = None;
            for row in repos.api_keys.find_all_user_key_hashes(conn)? {
                let matched = util::ct_eq(&wanted, &util::hex_lenient(&row.key_hash));
                if matched && found.is_none() {
                    found = Some(row.account);
                }
            }
            let Some(account) = found else {
                return Ok(None);
            };
            let game = repos.accounts.find(conn, &account)?;
            let launcher = repos.launcher_accounts.find_by_account(conn, &account)?;
            let user_id = game
                .as_ref()
                .map_or_else(|| account.to_string(), |g| g.user_id.as_str().to_owned());
            let username = launcher
                .as_ref()
                .map(|l| l.display_name.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| game.as_ref().map(|g| g.name.clone()).filter(|n| !n.is_empty()))
                .unwrap_or_else(|| user_id.clone());
            let admin = launcher.as_ref().is_some_and(|l| l.roles.iter().any(|r| r == "admin"));
            Ok(Some(Holder {
                account,
                user_id,
                username,
                admin,
                launcher,
            }))
        })
        .await
        .map_err(|e| e.to_string())
}

/// The home API hears about the key and the directory entry; failures are ignored.
pub async fn announce(shared: &Shared, account: &AccountId, key: &str, username: &str) {
    let home = shared.home();
    let digest = util::sha256_hex(key.as_bytes());
    let _ = home.register_account_key_hash(&digest, account.global()).await;
    let _ = home.register_directory_account(account.global(), username).await;
}

// ------------------------------------------------------------------ routes

/// `GET /dainty/api/RegistrationStatus`: the registration mode.
pub async fn registration_status(call: Call) -> Response {
    let mode = RegistrationMode::from_settings(call.shared.settings());
    util::ok(&object(vec![("mode", Value::from(mode.as_str()))]))
}

enum Registered {
    Done,
    Taken,
    InviteInvalid,
}

const TAKEN: &str = "launcher_accounts: the display name was taken";

fn is_unique_violation(error: &RepoError) -> bool {
    use diesel::result::{DatabaseErrorKind, Error};
    matches!(
        error,
        RepoError::Query(Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _))
    ) || matches!(error, RepoError::Data(text) if text == TAKEN)
}

/// `POST /dainty/api/Register`: creates an account and its key.
pub async fn register(call: Call) -> Response {
    let Call {
        shared,
        route,
        mut request,
        request_id,
        ..
    } = call;
    let Some(store) = store_of(&shared) else {
        return util::no_store(route, &request_id);
    };
    let mode = RegistrationMode::from_settings(shared.settings());
    if mode == RegistrationMode::None {
        return Refusal::RegistrationClosed.response(StatusCode::BAD_REQUEST);
    }
    if crate::oauth::provider(&shared).is_some_and(|p| p.required) {
        return Refusal::OAuthRequired.response(StatusCode::FORBIDDEN);
    }
    let body = match util::read_json(&mut request).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    let username = field(body.as_ref(), "Username");
    let code = field(body.as_ref(), "InviteCode");
    if !matches!(username, Some(Value::String(_))) || code.is_some_and(|c| !c.is_string()) {
        return Refusal::BadRequest.response(StatusCode::BAD_REQUEST);
    }
    let Some(username) = normalize_username(username) else {
        return Refusal::UsernameInvalid.response(StatusCode::BAD_REQUEST);
    };
    let code = code
        .and_then(Value::as_str)
        .map(|c| trim(c).to_owned())
        .unwrap_or_default();
    if mode == RegistrationMode::InviteCode && !is_invite_code(&code) {
        return Refusal::InviteInvalid.response(StatusCode::UNAUTHORIZED);
    }
    let made = match create_account(&shared, &store, &username, mode, code, None).await {
        Ok(made) => made,
        Err(page) => return page,
    };
    let response = util::ok(&object(vec![("UUK", Value::from(made.key))]));
    util::with_header(response, "x-dainty-account-id", &made.account.to_string())
}

/// An identity linked to a new account in the same write (`oauth.rs`).
#[derive(Debug, Clone)]
pub struct LinkIdentity {
    /// `auth_identities.provider`.
    pub provider: String,
    /// The provider's id for the user.
    pub subject: String,
    /// Their name there.
    pub username: String,
    /// Discord's avatar URL.
    pub avatar_url: Option<String>,
}

/// A new account and its key.
#[derive(Debug, Clone)]
pub struct Made {
    /// The game account.
    pub account: AccountId,
    /// Its userId.
    pub user_id: UserId,
    /// The account key, shown once.
    pub key: String,
}

const LINKED: &str = "auth_identities: the identity was linked";

/// Makes the launcher and game accounts and the key, spending the invite under `INVITECODE` and linking `link`;
/// a refusal or failure is the page to answer.
pub async fn create_account(
    shared: &Shared,
    store: &IslesStore,
    username: &str,
    mode: RegistrationMode,
    code: String,
    link: Option<LinkIdentity>,
) -> Result<Made, Response> {
    let key = make_account_key();
    let Ok(secret) = rustic_isles_auth::hashes::read_secret(shared.settings().raw("API_KEY_HASH_SECRET")) else {
        tracing::error!("[launcher-api] registration failed: API_KEY_HASH_SECRET must contain at least 32 characters.");
        return Err(util::empty(StatusCode::INTERNAL_SERVER_ERROR));
    };
    let hash = rustic_isles_auth::hashes::user_key_hash(secret, &key);
    let account = AccountId::new(&shared.node().node);
    let user_id = UserId::random();
    let actor = dainty_module_api::ActorContext::new(dainty_module_api::Actor::Player {
        account: account.global().clone(),
    });
    let normalized = username.to_lowercase();
    let now = iso(now_ms());
    let spent_at = now.clone();
    let linked_at = now.clone();
    let record = LauncherAccountRecord {
        account: account.clone(),
        user_id: user_id.clone(),
        email: None,
        display_name_normalized: normalized.clone(),
        display_name: username.to_owned(),
        password_hash: None,
        status: OperationalStatus::Active,
        approval_status: Some(ApprovalStatus::Approved),
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["player".to_owned()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: now,
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    };
    let game = AccountRecord {
        id: account.clone(),
        user_id: user_id.clone(),
        name: username.to_owned(),
        notes: 0,
    };
    let invite = mode == RegistrationMode::InviteCode;
    let id = account.clone();
    let written = store
        .write(&actor, move |tx, repos| {
            if repos
                .launcher_accounts
                .find_by_display_name_normalized(tx.conn(), &normalized)?
                .is_some()
            {
                return Ok(Registered::Taken);
            }
            if invite && !repos.invites.spend(tx, &code, &spent_at)? {
                return Ok(Registered::InviteInvalid);
            }
            repos.accounts.create(tx, &game)?;
            if repos.launcher_accounts.create(tx, &record)? != CreateOutcome::Created {
                // Rolls the account and the spent use back
                return Err(RepoError::Data(TAKEN.to_owned()));
            }
            if let Some(link) = link {
                let identity = rustic_isles_store::repo::auth::AuthIdentityRecord {
                    provider_subject: link.subject,
                    account: id.clone(),
                    provider_username: link.username,
                    provider_avatar_url: link.avatar_url,
                    linked_at,
                };
                if !repos
                    .auth_identities
                    .create_for_provider(tx, &link.provider, &identity)?
                {
                    return Err(RepoError::Data(LINKED.to_owned()));
                }
            }
            repos.api_keys.insert_user_key_hash(tx, &id, &hash)?;
            Ok(Registered::Done)
        })
        .await;
    match written {
        Ok(Registered::Done) => {}
        Ok(Registered::Taken) => return Err(Refusal::UsernameTaken.response(StatusCode::CONFLICT)),
        Ok(Registered::InviteInvalid) => return Err(Refusal::InviteInvalid.response(StatusCode::UNAUTHORIZED)),
        // Signed in twice at once: the other request made the account
        Err(RepoError::Data(text)) if text == LINKED => {
            return Err(json(
                StatusCode::CONFLICT,
                &object(vec![("error", Value::from("oauth_linked"))]),
            ));
        }
        Err(e) if is_unique_violation(&e) => return Err(Refusal::UsernameTaken.response(StatusCode::CONFLICT)),
        Err(e) => {
            tracing::error!("[launcher-api] registration failed: {e}");
            return Err(util::empty(StatusCode::INTERNAL_SERVER_ERROR));
        }
    }
    tracing::info!("[launcher-api] {username} registered as {user_id}");
    announce(shared, &account, &key, username).await;
    LauncherApiState::of(shared).forget_status();
    Ok(Made { account, user_id, key })
}

/// `GET /dainty/api/GetUserInfo`: the key holder's account.
pub async fn get_user_info(call: Call) -> Response {
    let Some(store) = store_of(&call.shared) else {
        return util::no_store(call.route, &call.request_id);
    };
    let Some((holder, key)) = key_holder(&call.shared, &store, call.request.headers()).await else {
        return json(
            StatusCode::UNAUTHORIZED,
            &object(vec![("error", Value::from("unauthorized"))]),
        );
    };
    if !holder.eligible() {
        let launcher = holder.launcher.as_ref();
        let code = if launcher.is_some_and(|l| l.status == OperationalStatus::Banned) {
            "account_banned"
        } else {
            "account_disabled"
        };
        let reason = launcher
            .and_then(|l| l.approval_reason.as_deref())
            .filter(|r| !r.is_empty())
            .map_or_else(|| "Ask the server's host.".to_owned(), |r| util::take_chars(r, 500));
        return json(
            StatusCode::FORBIDDEN,
            &object(vec![("error", Value::from(code)), ("reason", Value::from(reason))]),
        );
    }
    announce(&call.shared, &holder.account, &key, &holder.username).await;
    let body = object(vec![
        ("UserId", Value::from(holder.user_id.clone())),
        ("Username", Value::from(holder.username.clone())),
        ("IsAdmin", Value::Bool(holder.admin)),
    ]);
    util::with_header(util::ok(&body), "x-dainty-account-id", &holder.account.to_string())
}

/// `POST /dainty/api/GameTicket`: a one-time login for the key holder's game, in place of the key on its command
/// line (`rustic_isles_auth::game_tickets`). Refused as GetUserInfo refuses.
pub async fn game_ticket(call: Call) -> Response {
    let Some(store) = store_of(&call.shared) else {
        return util::no_store(call.route, &call.request_id);
    };
    let Some((holder, _)) = key_holder(&call.shared, &store, call.request.headers()).await else {
        return json(
            StatusCode::UNAUTHORIZED,
            &object(vec![("error", Value::from("unauthorized"))]),
        );
    };
    if !holder.eligible() {
        let banned = holder
            .launcher
            .as_ref()
            .is_some_and(|l| l.status == OperationalStatus::Banned);
        let code = if banned { "account_banned" } else { "account_disabled" };
        return json(StatusCode::FORBIDDEN, &object(vec![("error", Value::from(code))]));
    }
    let ticket = call
        .shared
        .auth()
        .mint_game_ticket(rustic_isles_auth::game_tickets::TicketHolder {
            user_id: holder.user_id.clone(),
            account_id: holder.account.to_string(),
        });
    let seconds = rustic_isles_auth::game_tickets::TTL_MS / 1000;
    util::ok(&object(vec![
        ("ticket", Value::from(ticket)),
        ("expiresInSeconds", Value::from(seconds)),
    ]))
}

/// `GET /dainty/api/ServerStatus`: the server's status, the full view for an eligible key holder.
pub async fn server_status(call: Call) -> Response {
    let Some(store) = store_of(&call.shared) else {
        return util::no_store(call.route, &call.request_id);
    };
    let holder = key_holder(&call.shared, &store, call.request.headers()).await;
    let view = match &holder {
        Some((holder, key)) if holder.eligible() => {
            announce(&call.shared, &holder.account, key, &holder.username).await;
            StatusView::Full
        }
        _ => StatusView::Limited,
    };
    let state = LauncherApiState::of(&call.shared);
    let body = match state.cached_status(view) {
        Some(body) => body,
        None => {
            let body = status_body(&call.shared, &store, &state, view).await;
            state.store_status(view, &body);
            body
        }
    };
    let mut response = util::ok(&body);
    util::set_header(&mut response, "cache-control", "no-store");
    util::set_header(&mut response, "vary", "x-dainty-user-api-key");
    response
}

// ------------------------------------------------------------------ the status body (§5.4)

fn server_name(raw: Option<&str>) -> String {
    let cleaned = util::controls_to_spaces(raw.unwrap_or(""));
    let name = trim(&cleaned);
    if name.is_empty() {
        "Dainty Lass server".to_owned()
    } else {
        util::take_chars(name, 64)
    }
}

/// `https://` (or `HTTPS://`) with a non-empty authority holding no `@` or white space, its host lower-cased.
pub fn source_url(raw: Option<&str>) -> Option<String> {
    let text = trim(raw?);
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("HTTPS://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace) {
        return None;
    }
    let tail = if tail.is_empty() || tail.starts_with(['?', '#']) {
        format!("/{tail}")
    } else {
        tail.to_owned()
    };
    Some(format!("https://{}{tail}", authority.to_lowercase()))
}

/// What the backend game data folder holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataKind {
    /// A file or the folder is missing.
    Missing,
    /// A file is byte-equal to its `.example.json`.
    Synthetic,
    /// Every file is the host's own.
    Real,
}

impl GameDataKind {
    /// The kind as `ServerStatus` answers it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Synthetic => "synthetic",
            Self::Real => "real",
        }
    }
}

const STATUS_FILES: [&str; 7] = [
    "progression_config.json",
    "slayers_path.json",
    "slayers_path_definitions.json",
    "ladyluck_store.json",
    "platinum_store.json",
    "reward_cache_store.json",
    "inventory_storage_policy.json",
];

/// The backend folder: any file missing is `missing`; any byte-equal to its `.example.json` is `synthetic`.
pub fn classify_game_data(folder: Option<&std::path::Path>) -> GameDataKind {
    let Some(folder) = folder else {
        return GameDataKind::Missing;
    };
    let mut example = false;
    for name in STATUS_FILES {
        let path = folder.join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            return GameDataKind::Missing;
        };
        let stem = name.strip_suffix(".json").unwrap_or(name);
        if let Ok(sample) = std::fs::read(folder.join(format!("{stem}.example.json")))
            && sample == bytes
        {
            example = true;
        }
    }
    if example {
        GameDataKind::Synthetic
    } else {
        GameDataKind::Real
    }
}

fn game_data_kind(shared: &Shared, state: &LauncherApiState) -> String {
    if let Some(kind) = state.cached_game_data() {
        return kind;
    }
    let kind = classify_game_data(shared.game_data().backend.as_deref())
        .as_str()
        .to_owned();
    state.store_game_data(&kind);
    kind
}

/// The text after the last `/`, up to its first `.`, its first 64 characters.
pub fn asset_leaf(text: &str) -> String {
    let leaf = text.rsplit('/').next().unwrap_or("");
    let leaf = leaf.split('.').next().unwrap_or("");
    util::take_chars(leaf, 64)
}

fn spaced(text: &str) -> String {
    util::take_chars(&text.replace('_', " "), 64)
}

/// The instance's title from its kind, map leaf and behemoth leaf.
pub fn instance_title(kind: &str, map: &str, behemoth: Option<&str>) -> String {
    match kind {
        "city" => return "Ramsgate".to_owned(),
        "dojo" => return "Training Grounds".to_owned(),
        "tutorial" => return "Tutorial".to_owned(),
        _ => {}
    }
    if let Some(behemoth) = behemoth.filter(|b| !b.is_empty()) {
        let name = behemoth
            .strip_prefix("BPC_")
            .or_else(|| behemoth.strip_prefix("BP_"))
            .unwrap_or(behemoth);
        let name = name.strip_suffix("_C").unwrap_or(name);
        return spaced(name);
    }
    let lower = map.to_ascii_lowercase();
    let name = if lower.ends_with("_persistent") {
        &map[..map.len() - "_persistent".len()]
    } else {
        map
    };
    let title = spaced(name);
    if title.is_empty() { "Hunt".to_owned() } else { title }
}

fn positive_floor(value: Option<&Value>) -> u64 {
    match value.and_then(Value::as_f64) {
        Some(n) if n.is_finite() && n > 0.0 => {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "floored and positive; a huge value saturates"
            )]
            let out = n.floor() as u64;
            out
        }
        _ => 0,
    }
}

/// The Director's instances read field by field, and which player is in which hunt.
pub fn status_instances(answer: &Value) -> (Vec<Value>, HashMap<String, String>) {
    let mut out = Vec::new();
    let mut players = HashMap::new();
    let Some(list) = answer.as_array() else {
        return (out, players);
    };
    for entry in list.iter().take(MAX_INSTANCES) {
        let Some(map) = entry.as_object() else {
            continue;
        };
        let Some(id) = map.get("id").and_then(Value::as_str).filter(|i| !i.is_empty()) else {
            continue;
        };
        let Some(kind) = map
            .get("kind")
            .and_then(Value::as_str)
            .filter(|k| matches!(*k, "city" | "hunt" | "dojo" | "tutorial"))
        else {
            continue;
        };
        let id = util::take_chars(id, 64);
        let map_leaf = map
            .get("map")
            .and_then(Value::as_str)
            .map(|m| asset_leaf(&util::take_chars(m, 160)))
            .unwrap_or_default();
        let behemoth = map
            .get("behemoth")
            .and_then(Value::as_str)
            .map(|b| asset_leaf(&util::take_chars(b, 160)));
        let started = map
            .get("startedAt")
            .and_then(Value::as_str)
            .and_then(dainty_service_kit::js::date_parse)
            .map(iso);
        let title = instance_title(kind, &map_leaf, behemoth.as_deref());
        if kind == "hunt"
            && let Some(ids) = map.get("playerIds").and_then(Value::as_array)
        {
            for player in ids.iter().take(16).filter_map(Value::as_str) {
                if utf16_len(player) <= 128 {
                    players.insert(player.to_owned(), id.clone());
                }
            }
        }
        out.push(object(vec![
            ("id", Value::from(id)),
            ("kind", Value::from(kind)),
            ("title", Value::from(title)),
            ("map", Value::from(map_leaf)),
            ("behemoth", behemoth.map_or(Value::Null, Value::from)),
            ("players", Value::from(positive_floor(map.get("players")))),
            ("maxPlayers", Value::from(positive_floor(map.get("maxPlayers")))),
            ("startedAt", started.map_or(Value::Null, Value::from)),
        ]));
    }
    (out, players)
}

fn dechunk(body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&rest[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        if rest.len() < size {
            return None;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..)?;
    }
}

/// A plain HTTP/1.1 GET of `/api/instances` on the authority; anything but a 2xx JSON answer is none.
pub async fn director_get(authority: &str) -> Option<Value> {
    let work = async {
        let mut stream = tokio::net::TcpStream::connect(authority).await.ok()?;
        let head =
            format!("GET /api/instances HTTP/1.1\r\nHost: {authority}\r\nAccept: */*\r\nConnection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).await.ok()?;
        let mut raw = Vec::new();
        let mut limited = (&mut stream).take(DIRECTOR_MAX_BYTES as u64);
        limited.read_to_end(&mut raw).await.ok()?;
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let mut lines = head.split("\r\n");
        let status: u16 = lines.next()?.split(' ').nth(1)?.parse().ok()?;
        if !(200..300).contains(&status) {
            return None;
        }
        let chunked = lines.any(|line| {
            line.split_once(':').is_some_and(|(n, v)| {
                n.trim().eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
            })
        });
        let body = &raw[split + 4..];
        let body = if chunked { dechunk(body)? } else { body.to_vec() };
        serde_json::from_slice::<Value>(&body).ok()
    };
    tokio::time::timeout(DIRECTOR_TIMEOUT, work).await.ok().flatten()
}

async fn online_players(shared: &Shared, store: &IslesStore, map: &HashMap<String, String>) -> Vec<Value> {
    let ids: Vec<String> = shared
        .presence()
        .online_user_ids()
        .into_iter()
        .take(MAX_PLAYERS)
        .collect();
    let found = store
        .read(move |conn, repos| {
            let mut names = Vec::new();
            for id in ids {
                let user = UserId::new(id.clone());
                let Ok(Some(game)) = repos.accounts.find_by_user_id(conn, &user) else {
                    continue;
                };
                let launcher = match repos.launcher_accounts.find_by_account(conn, &game.id) {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                let name = launcher
                    .map(|l| l.display_name)
                    .filter(|n| !n.is_empty())
                    .unwrap_or(game.name);
                if !name.is_empty() {
                    names.push((id, name));
                }
            }
            Ok(names)
        })
        .await
        .unwrap_or_default();
    let mut players: Vec<(String, Value)> = found
        .into_iter()
        .map(|(id, name)| {
            let instance = map.get(&id).cloned();
            let body = object(vec![
                ("name", Value::from(util::take_chars(&name, 32))),
                (
                    "where",
                    Value::from(if instance.is_some() { "hunt" } else { "unknown" }),
                ),
                ("instance", instance.map_or(Value::Null, Value::from)),
            ]);
            (name, body)
        })
        .collect();
    players.sort_by(|a, b| dainty_service_kit::collate::locale_compare(&a.0, &b.0));
    players.into_iter().map(|(_, v)| v).collect()
}

async fn status_body(shared: &Arc<Shared>, store: &IslesStore, state: &LauncherApiState, view: StatusView) -> Value {
    let settings = shared.settings();
    let tables = shared.version().tables;
    let version = util::take_chars(trim(settings.raw("DAINTY_SERVER_VERSION").unwrap_or("")), 48);
    let sha = settings
        .raw("APPROVED_EXECUTABLE_SHA256")
        .filter(|v| util::is_hex(v, 64))
        .map(str::to_lowercase);
    let mut game = Map::new();
    game.insert(
        "build".into(),
        Value::from(format!("rel-{}-Archon", tables.game_version)),
    );
    game.insert("changelist".into(), Value::from(tables.target_changelist));
    game.insert("executableSha256".into(), sha.map_or(Value::Null, Value::from));
    let (players, instances) = match view {
        StatusView::Full => {
            let authority = trim(settings.raw("DEPLOYSERVER_URL").unwrap_or("")).to_owned();
            let answer = if authority.is_empty() {
                None
            } else {
                director_get(&authority).await
            };
            let (instances, map) = answer.as_ref().map(status_instances).unwrap_or_default();
            (online_players(shared, store, &map).await, instances)
        }
        StatusView::Limited => (Vec::new(), Vec::new()),
    };
    let mut body = object(vec![
        ("name", Value::from(server_name(settings.raw("DAINTY_SERVER_NAME")))),
        ("online", Value::Bool(true)),
        (
            "version",
            Value::from(if version.is_empty() {
                "0.1.0".to_owned()
            } else {
                version
            }),
        ),
        (
            "commit",
            Value::from(util::take_chars(
                trim(settings.raw("DAINTY_SOURCE_COMMIT").unwrap_or("")),
                64,
            )),
        ),
        (
            "sourceUrl",
            source_url(settings.raw("DAINTY_SOURCE_URL")).map_or(Value::Null, Value::from),
        ),
        (
            "registration",
            Value::from(RegistrationMode::from_settings(settings).as_str()),
        ),
        ("uptimeSeconds", Value::from(state.uptime_seconds())),
        ("contentPort", Value::Null),
        ("game", Value::Object(game)),
        ("gameData", Value::from(game_data_kind(shared, state))),
        (
            "chat",
            Value::Bool(settings.raw("REALTIME_XMPP_ENABLED") == Some("true")),
        ),
        ("playersOnline", Value::from(players.len())),
        ("players", Value::Array(players)),
        ("instances", Value::Array(instances)),
        ("limited", Value::Bool(view == StatusView::Limited)),
    ]);
    if let (Some(economy), Value::Object(body)) = (economy(settings), &mut body) {
        body.insert("economy".into(), economy);
    }
    if let (Some(minimum), Value::Object(body)) = (min_launcher_version(settings), &mut body) {
        body.insert("minLauncherVersion".into(), Value::from(minimum));
    }
    if let (Some(provider), Value::Object(body)) = (crate::oauth::provider(shared), &mut body) {
        body.insert("signIn".into(), crate::oauth::status_entry(&provider));
    }
    body
}

/// The oldest launcher the host lets play (`[backend] min_launcher_version`). The host kit checked it is a
/// version and the launcher parses it again; unset keeps the body Node's.
pub fn min_launcher_version(settings: &rustic_isles::Settings) -> Option<String> {
    settings
        .raw("DAINTY_MIN_LAUNCHER_VERSION")
        .map(trim)
        .filter(|text| !text.is_empty() && text.len() <= 64 && text.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_owned)
}

/// The game servers' supply caps and fusion time, which the launcher hands the player's game (spec-runtime-r15
/// Q-1). Only values the runtime would take; none set keeps the body Node's.
pub fn economy(settings: &rustic_isles::Settings) -> Option<Value> {
    let keep = match settings.raw("DAINTY_KEEP_SUPPLY_STACK_CAPS").map(trim) {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };
    // 10 080: a week, the host kit's and the runtime's ceiling
    let minutes = settings
        .raw("DAINTY_CELL_FUSION_MINUTES")
        .map(trim)
        .filter(|text| !text.is_empty() && text.len() <= 5 && text.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|text| text.parse::<u32>().ok())
        .filter(|&m| m <= 10_080);
    if keep.is_none() && minutes.is_none() {
        return None;
    }
    let mut out = Map::new();
    if let Some(keep) = keep {
        out.insert("keepSupplyStackCaps".into(), Value::Bool(keep));
    }
    if let Some(minutes) = minutes {
        out.insert("cellFusionMinutes".into(), Value::from(minutes));
    }
    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usernames_codes_notes_keys() {
        assert_eq!(normalize_username(Some(&json!("  Fox_9 "))).as_deref(), Some("Fox_9"));
        for bad in ["ab", "a b c", "abcdefghijklmnopq", "née_ab"] {
            assert_eq!(normalize_username(Some(&json!(bad))), None, "{bad}");
        }
        let mut n = 0;
        assert_eq!(
            make_invite_code_with(|_| {
                n += 1;
                n - 1
            }),
            "ABCD-EFGH-JKMN"
        );
        let code = make_invite_code();
        assert_eq!(code.len(), 14);
        assert!(code.bytes().all(|b| b == b'-' || CODE_ALPHABET.contains(&b)));
        assert!(is_invite_code("abc-1") && !is_invite_code("abc") && !is_invite_code("a b c d"));
        assert!(!is_invite_code(&"a".repeat(65)));
        assert_eq!(
            invite_note(Some(&json!("  for\u{0}Alex\n"))).as_deref(),
            Some("for Alex")
        );
        assert_eq!(invite_note(Some(&json!("   "))), None);
        assert_eq!(invite_note(Some(&json!("x".repeat(100)))).map(|n| n.len()), Some(64));
        assert_eq!(invite_note(Some(&json!(5))), None);
        let key = make_account_key();
        assert!(key.starts_with("DLK_") && key.len() == 52 && looks_like_account_key(&key));
        assert!(!looks_like_account_key("short") && !looks_like_account_key("a b c d e f g"));
    }

    #[test]
    fn instances() {
        let answer = json!([
            {"id":"h1","kind":"hunt","map":"/Game/Maps/islands/foo/foo_persistent",
             "behemoth":"/Game/Behemoths/BPC_Skarn_Prime.BPC_Skarn_Prime_C","players":3.7,"maxPlayers":5,
             "startedAt":"2026-10-20T00:00:00Z","playerIds":["a",7,"b"]},
            {"id":"c1","kind":"city","players":-2},
            {"id":"x","kind":"arena"}, "text", {"kind":"hunt"},
            {"id":"m","kind":"hunt","map":"/Game/Maps/Marsh_Of_Fog_persistent.Marsh"},
            {"id":"e","kind":"hunt"}
        ]);
        let (list, players) = status_instances(&answer);
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&list[0]),
            r#"{"id":"h1","kind":"hunt","title":"Skarn Prime","map":"foo_persistent","behemoth":"BPC_Skarn_Prime","players":3,"maxPlayers":5,"startedAt":"2026-10-20T00:00:00.000Z"}"#
        );
        assert_eq!(players.get("a").map(String::as_str), Some("h1"));
        assert_eq!(players.get("b").map(String::as_str), Some("h1"));
        assert_eq!(list[1]["title"], "Ramsgate");
        assert_eq!(list[1]["players"], 0);
        assert_eq!(list[1]["startedAt"], Value::Null);
        assert_eq!(list.len(), 4);
        assert_eq!(list[2]["title"], "Marsh Of Fog");
        assert_eq!(list[3]["title"], "Hunt");
        assert!(status_instances(&json!({"a":1})).0.is_empty());
        assert_eq!(asset_leaf("/Game/Behemoths/Skarn.Skarn_C"), "Skarn");
    }

    #[test]
    fn source_urls() {
        assert_eq!(
            source_url(Some("https://Example.com/src")).as_deref(),
            Some("https://example.com/src")
        );
        assert_eq!(source_url(Some("HTTPS://x.y?a")).as_deref(), Some("https://x.y/?a"));
        assert_eq!(source_url(Some("http://example.com/src")), None);
        assert_eq!(source_url(Some("Https://example.com")), None);
        assert_eq!(source_url(Some("https://u@x/")), None);
        assert_eq!(source_url(Some("https:///x")), None);
        assert_eq!(server_name(Some("")), "Dainty Lass server");
        assert_eq!(server_name(Some(" Fox\u{1}hold ")), "Fox hold");
    }

    #[test]
    fn economy_only_as_the_runtime_takes_it() {
        let of = |pairs: &[(&str, &str)]| economy(&rustic_isles::Settings::from_pairs(pairs.iter().copied()));
        assert_eq!(of(&[]), None);
        assert_eq!(
            of(&[
                ("DAINTY_KEEP_SUPPLY_STACK_CAPS", "true"),
                ("DAINTY_CELL_FUSION_MINUTES", "45")
            ]),
            Some(json!({ "keepSupplyStackCaps": true, "cellFusionMinutes": 45 }))
        );
        assert_eq!(
            of(&[("DAINTY_KEEP_SUPPLY_STACK_CAPS", "false")]),
            Some(json!({ "keepSupplyStackCaps": false }))
        );
        assert_eq!(
            of(&[("DAINTY_CELL_FUSION_MINUTES", "0")]),
            Some(json!({ "cellFusionMinutes": 0 }))
        );
        assert_eq!(
            of(&[("DAINTY_CELL_FUSION_MINUTES", "10080")]),
            Some(json!({ "cellFusionMinutes": 10080 }))
        );
        for bad in [
            ("DAINTY_KEEP_SUPPLY_STACK_CAPS", "TRUE"),
            ("DAINTY_KEEP_SUPPLY_STACK_CAPS", "1"),
        ] {
            assert_eq!(of(&[bad]), None, "{bad:?}");
        }
        for bad in ["10081", "-1", "4.5", "", "99999999999", "45min"] {
            assert_eq!(of(&[("DAINTY_CELL_FUSION_MINUTES", bad)]), None, "{bad}");
        }
    }

    #[test]
    fn min_launcher_version_only_when_set() {
        let of =
            |pairs: &[(&str, &str)]| min_launcher_version(&rustic_isles::Settings::from_pairs(pairs.iter().copied()));
        assert_eq!(of(&[]), None);
        assert_eq!(of(&[("DAINTY_MIN_LAUNCHER_VERSION", "")]), None);
        assert_eq!(
            of(&[("DAINTY_MIN_LAUNCHER_VERSION", " 2.1.0-rc.1 ")]).as_deref(),
            Some("2.1.0-rc.1")
        );
        assert_eq!(of(&[("DAINTY_MIN_LAUNCHER_VERSION", "2.1 0")]), None);
        assert_eq!(of(&[("DAINTY_MIN_LAUNCHER_VERSION", &"9".repeat(65))]), None);
    }

    #[test]
    fn game_data_classes() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(classify_game_data(None), GameDataKind::Missing);
        assert_eq!(classify_game_data(Some(dir.path())), GameDataKind::Missing);
        for name in STATUS_FILES {
            std::fs::write(dir.path().join(name), "{}").unwrap();
        }
        assert_eq!(classify_game_data(Some(dir.path())), GameDataKind::Real);
        std::fs::write(dir.path().join("platinum_store.example.json"), "{\"a\":1}").unwrap();
        assert_eq!(classify_game_data(Some(dir.path())), GameDataKind::Real);
        std::fs::write(dir.path().join("platinum_store.example.json"), "{}").unwrap();
        assert_eq!(classify_game_data(Some(dir.path())), GameDataKind::Synthetic);
    }

    #[tokio::test]
    async fn director_reads() {
        async fn serve(answer: &'static str) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap().to_string();
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                socket.write_all(answer.as_bytes()).await.unwrap();
            });
            address
        }
        let plain = serve("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n[1]").await;
        assert_eq!(director_get(&plain).await, Some(json!([1])));
        let chunked =
            serve("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n[2\r\n1\r\n]\r\n0\r\n\r\n").await;
        assert_eq!(director_get(&chunked).await, Some(json!([2])));
        let failed = serve("HTTP/1.1 500 Oops\r\n\r\n[]").await;
        assert_eq!(director_get(&failed).await, None);
        let text = serve("HTTP/1.1 200 OK\r\n\r\nnot json").await;
        assert_eq!(director_get(&text).await, None);
    }
}
