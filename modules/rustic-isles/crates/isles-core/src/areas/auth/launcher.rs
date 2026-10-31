//! `controllers/launcherAuth.ts`: the launcher's own accounts and sessions. Registration, password sign-in, rotating
//! refresh sessions, the one-time game exchange code, and the account view. Every function answers the JSON Node's
//! `Handle` sends, or the `LauncherApiError` it throws.

use rifts_seam::{AccountId as RiftsAccount, Json as RiftsJson};
use serde_json::{Value, json};

use super::crypto::{generate_opaque_token, hash_opaque_token, hash_password, mask_email, verify_password};
use super::data::{account_write, launcher_by_display_name, launcher_by_email, launcher_by_user, store, system_actor};
use super::email::{issue_email_code, require_email_sending};
use super::errors::LauncherApiError;
use super::mail::describe_mail_error;
use super::policy::{
    LauncherBuildVerdict, OFFICIAL_PLAY_UNAVAILABLE_MESSAGE, UNOFFICIAL_LAUNCHER_MESSAGE, assert_account_admitted,
    assert_account_eligible, assert_account_not_deactivated, assert_consent_current_when_enforced,
    assert_username_allowed, current_legal_requirements, effective_approval_status, email_verification_required,
    parse_launcher_build_report, read_official_play, read_unofficial_launcher_policy, resolve_registration_approval,
    unofficial_launcher_refuses, validate_registration_consent, validate_username,
};
use super::runtime_authorization::{
    assert_runtime_artifact_set_approved, assert_runtime_channel_matches_account, runtime_update_root,
};
use super::tokens::sign_launcher_access_token;
use super::util::{iso_from_ms, iso_now, now_ms, parse_ms};
use crate::http::Shared;
use crate::repo::auth::{
    ApprovalStatus, CreateOutcome, GameExchangeCodeRecord, LauncherAccountRecord, OperationalStatus,
    RefreshSessionRecord, RuntimeChannel,
};
use crate::repo::{AccountId, UserId};

const REFRESH_TOKEN_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const EXCHANGE_CODE_TTL_MS: i64 = 60 * 1000;

pub type Reply = Result<Value, LauncherApiError>;

fn normalize_email(email: &str) -> String {
    service_kit::js::trim(email).to_lowercase()
}

fn normalize_display_name(name: &str) -> String {
    service_kit::js::trim(name).to_lowercase()
}

/// `GetApprovedExecutableHashes`: `APPROVED_EXECUTABLE_SHA256`, comma-separated. An unset or empty list approves
/// nothing: the check fails closed.
fn approved_executable_hashes(shared: &Shared) -> Vec<String> {
    shared
        .settings
        .raw("APPROVED_EXECUTABLE_SHA256")
        .unwrap_or_default()
        .split(',')
        .map(|h| service_kit::js::trim(h).to_lowercase())
        .filter(|h| is_sha256(h))
        .collect()
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `IsValidEmailAddress`: linear time, so a 50 KB "a@...." body cannot stall the event loop.
pub fn is_valid_email_address(raw: Option<&Value>) -> bool {
    let Some(raw) = raw.and_then(Value::as_str) else {
        return false;
    };
    is_valid_email(raw)
}

pub fn is_valid_email(raw: &str) -> bool {
    let units = raw.encode_utf16().count();
    if units == 0 || units > 254 {
        return false;
    }
    let Some(at) = raw.find('@') else { return false };
    if at == 0 || Some(at) != raw.rfind('@') || raw[..at].encode_utf16().count() > 64 {
        return false;
    }
    let domain = &raw[at + 1..];
    if domain.encode_utf16().count() < 3 || domain.starts_with('.') || domain.ends_with('.') || !domain.contains('.') {
        return false;
    }
    !raw.chars().any(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
}

fn validate_registration_input(email: Option<&Value>, password: Option<&Value>) -> Result<(), LauncherApiError> {
    if !is_valid_email_address(email) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Enter a valid email address.",
        ));
    }
    let long_enough = password
        .and_then(Value::as_str)
        .is_some_and(|p| p.encode_utf16().count() >= 8);
    if !long_enough {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Password must be at least 8 characters.",
        ));
    }
    Ok(())
}

fn internal_error(error: impl std::fmt::Display) -> LauncherApiError {
    tracing::error!("auth: {error}");
    super::errors::internal()
}

/// Argon2 is memory-hard and takes tens of milliseconds: it goes to the blocking pool.
pub async fn hash_password_blocking(password: String) -> Result<String, LauncherApiError> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(internal_error)?
        .map_err(internal_error)
}

pub async fn verify_password_blocking(hash: Option<String>, password: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(hash.as_deref(), &password))
        .await
        .unwrap_or(false)
}

// ------------------------------------------------------------------ the account view

/// `ToAccountView`.
pub async fn to_account_view(shared: &Shared, account: &LauncherAccountRecord) -> Result<Value, LauncherApiError> {
    let id = account.account.clone();
    let identity = store(shared)?
        .read(move |conn, repos| repos.auth_identities.find_by_account(conn, &id))
        .await?;
    let discord = match &identity {
        None => json!({ "linked": false }),
        Some(identity) => json!({ "linked": true, "username": identity.provider_username }),
    };
    Ok(json!({
        "userId": account.user_id.as_str(),
        "displayName": account.display_name,
        "email": account.email.clone().unwrap_or_default(),
        "discordLinked": identity.is_some(),
        "discord": discord,
        "emailVerified": account.email_verified_at.is_some(),
        "status": account.status.as_str(),
        "approvalStatus": effective_approval_status(account).as_str(),
        "needsUsername": account.username_set == Some(false),
    }))
}

/// `GetAccountView`.
pub async fn get_account_view(shared: &Shared, user_id: &str) -> Reply {
    let account = launcher_by_user(shared, user_id)
        .await?
        .ok_or_else(|| LauncherApiError::new("NOT_FOUND", "Account not found."))?;
    to_account_view(shared, &account).await
}

// ------------------------------------------------------------------ sessions

/// `CreateSessionForUser`: a new refresh family and its first token.
pub async fn create_session_for_user(
    shared: &Shared,
    account: &LauncherAccountRecord,
    device_id: &str,
    device_name: &str,
) -> Result<(String, String), LauncherApiError> {
    let family_id = uuid::Uuid::new_v4().to_string();
    let refresh_token = generate_opaque_token();
    let now = now_ms();
    let session = RefreshSessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        token_hash: hash_opaque_token(&refresh_token),
        account: account.account.clone(),
        family_id: family_id.clone(),
        device_id: device_id.to_owned(),
        device_name: device_name.to_owned(),
        created_at: iso_from_ms(now),
        expires_at: iso_from_ms(now + REFRESH_TOKEN_TTL_MS),
        revoked_at: None,
    };
    let id = account.account.clone();
    account_write(shared, &id, move |tx, repos| {
        repos.refresh_sessions.create(tx, &session)
    })
    .await?;
    let access =
        sign_launcher_access_token(&shared.settings, account.user_id.as_str(), &family_id).map_err(internal_error)?;
    Ok((access, refresh_token))
}

fn session_result(access: String, refresh: String, account: Value) -> Value {
    json!({ "accessToken": access, "refreshToken": refresh, "account": account })
}

/// `AssertAccountNotBlocked`: a banned or disabled account is refused with its own error. The approval state is not
/// looked at: the sign-in checks it afterwards, so a banned account never gets a code.
pub fn assert_account_not_blocked(account: &LauncherAccountRecord) -> Result<(), LauncherApiError> {
    if matches!(account.status, OperationalStatus::Banned | OperationalStatus::Disabled) {
        assert_account_admitted(account)?;
    }
    Ok(())
}

/// `SignInAfterEmailProof`: the session a password sign-in gives when the account is admitted, or, while the approval
/// is pending, the registration's approval answer (no session). Banned, disabled and rejected accounts are refused.
pub async fn sign_in_after_email_proof(
    shared: &Shared,
    account: &LauncherAccountRecord,
    device_id: &str,
    device_name: &str,
) -> Reply {
    if let Err(error) = assert_account_admitted(account) {
        if error.code == "AUTH_APPROVAL_PENDING" {
            return Ok(json!({ "approvalRequired": true, "account": to_account_view(shared, account).await? }));
        }
        return Err(error);
    }
    let now = iso_now();
    let id = account.account.clone();
    let when = now.clone();
    account_write(shared, &account.account, move |tx, repos| {
        repos.launcher_accounts.update_last_login(tx, &id, &when)
    })
    .await?;
    let (access, refresh) = create_session_for_user(shared, account, device_id, device_name).await?;
    let mut signed_in = account.clone();
    signed_in.last_login_at = Some(now);
    Ok(session_result(
        access,
        refresh,
        to_account_view(shared, &signed_in).await?,
    ))
}

// ------------------------------------------------------------------ register and sign in

/// `UsernameTaken`: another account (not `except`) holds the name in any letter case.
async fn username_taken(shared: &Shared, username: &str, except: Option<&UserId>) -> Result<bool, LauncherApiError> {
    let existing = launcher_by_display_name(shared, &username.to_lowercase()).await?;
    Ok(existing.is_some_and(|account| Some(&account.user_id) != except))
}

pub struct RegisterInput<'a> {
    pub display_name: Option<&'a Value>,
    pub email: Option<&'a Value>,
    pub password: Option<&'a Value>,
    pub consent: Option<&'a Value>,
    pub ip: &'a str,
}

/// `RegisterAccount`.
pub async fn register_account(shared: &Shared, input: RegisterInput<'_>) -> Reply {
    if shared
        .auth
        .limiter
        .is_limited(&format!("register:{}", input.ip), 5, 60 * 60 * 1000)
    {
        return Err(LauncherApiError::new(
            "AUTH_RATE_LIMITED",
            "Too many registration attempts. Try again later.",
        ));
    }
    // The name is optional: without one (absent, null or blank) the account carries its userId as its placeholder name
    // and usernameSet false, as Discord accounts do; the launcher then asks for it.
    let blank = match input.display_name {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => service_kit::js::trim(text).is_empty(),
        Some(_) => false,
    };
    let given_name = if blank {
        None
    } else {
        Some(validate_username(input.display_name)?)
    };
    if let Some(name) = &given_name {
        assert_username_allowed(name, &shared.settings)?;
    }
    validate_registration_input(input.email, input.password)?;
    let engine = shared.store.as_ref().map(|s| s.database().engine());
    let consent = validate_registration_consent(
        input.consent,
        &iso_now(),
        &shared.settings,
        engine,
        &current_legal_requirements(),
    )?;
    let email = normalize_email(input.email.and_then(Value::as_str).unwrap_or_default());
    let normalized_name = given_name.as_deref().map(normalize_display_name);

    // Under MYSTICPARADOX_EMAIL_VERIFICATION=required the code mail must be set up before an account is created: one
    // that can never be verified would only wait for the 48 h sweep
    let verification_required = email_verification_required(&shared.settings);
    if verification_required {
        require_email_sending(shared)?;
    }
    if launcher_by_email(shared, &email).await?.is_some() {
        return Err(LauncherApiError::new(
            "AUTH_EMAIL_TAKEN",
            "That email is already registered.",
        ));
    }
    if let Some(name) = &given_name
        && username_taken(shared, name, None).await?
    {
        return Err(LauncherApiError::new(
            "AUTH_DISPLAY_NAME_TAKEN",
            "That display name is already taken.",
        ));
    }

    let user_id = UserId::random();
    let password_hash =
        hash_password_blocking(input.password.and_then(Value::as_str).unwrap_or_default().to_owned()).await?;
    let approval_status = resolve_registration_approval(&shared.settings).approval_status;
    let created_at = iso_now();
    let record = LauncherAccountRecord {
        account: AccountId::new(&shared.node.node),
        user_id: user_id.clone(),
        email: Some(email.clone()),
        // The placeholder is the userId, lower-cased for the unique index, as Discord accounts use it
        display_name_normalized: normalized_name.unwrap_or_else(|| user_id.as_str().to_lowercase()),
        display_name: given_name.clone().unwrap_or_else(|| user_id.as_str().to_owned()),
        password_hash: Some(password_hash),
        status: OperationalStatus::Active,
        approval_status: Some(approval_status),
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["player".to_owned()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: created_at.clone(),
        last_login_at: None,
        username_set: Some(given_name.is_some()),
        consent,
        deletion: None,
        email_verified_at: None,
        // The sweep removes this registration if it is not verified within 48 h
        email_verification_pending_since: verification_required.then(|| created_at.clone()),
    };
    let to_store = record.clone();
    let outcome = store(shared)?
        .write(&system_actor(), move |tx, repos| {
            repos.launcher_accounts.create(tx, &to_store)
        })
        .await?;
    match outcome {
        CreateOutcome::Created => {}
        CreateOutcome::EmailTaken => {
            return Err(LauncherApiError::new(
                "AUTH_EMAIL_TAKEN",
                "That email is already registered.",
            ));
        }
        CreateOutcome::DisplayNameTaken => {
            return Err(LauncherApiError::new(
                "AUTH_DISPLAY_NAME_TAKEN",
                "That display name is already taken.",
            ));
        }
    }
    tracing::info!("Launcher account registered: {user_id}");

    // Pending accounts receive no access or refresh token: possession of the open-source launcher cannot bypass
    // server-side admission
    let approval_required = approval_status == ApprovalStatus::Pending;
    let view = to_account_view(shared, &record).await?;
    if !verification_required {
        return Ok(json!({ "approvalRequired": approval_required, "account": view }));
    }
    // The first code is mailed now (no send limit applies yet). A failure leaves the account in place: the launcher can
    // ask for a code again, and the sweep removes the account if it never verifies. Still no token is issued here.
    if let Err(error) = issue_email_code(
        shared,
        &record,
        &email,
        crate::repo::auth::EmailCodePurpose::Verify,
        input.ip,
    )
    .await
    {
        tracing::warn!("[Email] the first code for user {user_id} could not be stored ({error})");
    }
    Ok(json!({
        "approvalRequired": approval_required,
        "verificationRequired": true,
        "email": email,
        "account": view,
    }))
}

/// `LoginWithPassword`.
pub async fn login_with_password(
    shared: &Shared,
    email: Option<&Value>,
    password: Option<&Value>,
    device_id: &str,
    device_name: &str,
    ip: &str,
) -> Reply {
    const WINDOW: i64 = 5 * 60 * 1000;
    // `NormalizeEmail(email ?? "")` throws on anything but a string: Node answers INTERNAL
    let key_email = match email {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => normalize_email(text),
        Some(_) => String::new(),
    };
    let limited = shared.auth.limiter.is_limited(&format!("login:{ip}"), 10, WINDOW) || {
        if !matches!(email, None | Some(Value::Null) | Some(Value::String(_))) {
            return Err(internal_error("login: the email is not a string"));
        }
        shared
            .auth
            .limiter
            .is_limited(&format!("login:{key_email}"), 10, WINDOW)
    };
    if limited {
        return Err(LauncherApiError::new(
            "AUTH_RATE_LIMITED",
            "Too many login attempts. Try again later.",
        ));
    }
    let invalid = || LauncherApiError::new("AUTH_INVALID_CREDENTIALS", "The email or password is incorrect.");
    let (Some(email), Some(password)) = (email.and_then(Value::as_str), password.and_then(Value::as_str)) else {
        return Err(invalid());
    };
    let account = launcher_by_email(shared, &normalize_email(email)).await?;
    let password_ok = verify_password_blocking(
        account.as_ref().and_then(|a| a.password_hash.clone()),
        password.to_owned(),
    )
    .await;
    let Some(account) = account.filter(|_| password_ok) else {
        return Err(invalid());
    };
    // A banned or disabled account is refused first: no code is mailed to it
    assert_account_not_blocked(&account)?;
    // The right password is not enough while the email is unverified: a code is mailed within the send limits and the
    // sign-in waits for it. A wrong password never gets here.
    if email_verification_required(&shared.settings) && account.email_verified_at.is_none() {
        require_email_sending(shared)?;
        let address = account.email.clone().unwrap_or_default();
        issue_email_code(
            shared,
            &account,
            &address,
            crate::repo::auth::EmailCodePurpose::Verify,
            ip,
        )
        .await?;
        return Err(LauncherApiError::new(
            "AUTH_EMAIL_UNVERIFIED",
            format!(
                "Enter the 6-digit code we sent to {}. Older launchers: update the launcher to verify your email.",
                mask_email(&address)
            ),
        ));
    }
    assert_account_admitted(&account)?;
    let now = iso_now();
    let id = account.account.clone();
    let when = now.clone();
    account_write(shared, &account.account, move |tx, repos| {
        repos.launcher_accounts.update_last_login(tx, &id, &when)
    })
    .await?;
    let (access, refresh) = create_session_for_user(shared, &account, device_id, device_name).await?;
    let mut signed_in = account;
    signed_in.last_login_at = Some(now);
    Ok(session_result(
        access,
        refresh,
        to_account_view(shared, &signed_in).await?,
    ))
}

/// How old `lastLoginAt` may get before a session restore or a Discord sign-in writes it again.
pub const LAST_SIGN_IN_WRITE_INTERVAL_MS: i64 = 5 * 60 * 1000;

/// `NoteSignIn`: records a sign-in that does not go through the password form, at most once every
/// [`LAST_SIGN_IN_WRITE_INTERVAL_MS`] per account. Returns the account as it now reads.
pub async fn note_sign_in(
    shared: &Shared,
    account: &LauncherAccountRecord,
    now_ms_value: i64,
) -> Result<LauncherAccountRecord, LauncherApiError> {
    if let Some(last) = account.last_login_at.as_deref().and_then(parse_ms) {
        let age = now_ms_value - last;
        if (0..LAST_SIGN_IN_WRITE_INTERVAL_MS).contains(&age) {
            return Ok(account.clone());
        }
    }
    let when = iso_from_ms(now_ms_value);
    let id = account.account.clone();
    let to_write = when.clone();
    account_write(shared, &account.account, move |tx, repos| {
        repos.launcher_accounts.update_last_login(tx, &id, &to_write)
    })
    .await?;
    let mut noted = account.clone();
    noted.last_login_at = Some(when);
    Ok(noted)
}

async fn revoke_session_family(shared: &Shared, session: &RefreshSessionRecord) -> Result<(), LauncherApiError> {
    let (family, now) = (session.family_id.clone(), iso_now());
    account_write(shared, &session.account, move |tx, repos| {
        repos.refresh_sessions.revoke_family(tx, &family, &now)
    })
    .await?;
    Ok(())
}

/// `RefreshSession`: a rotating refresh token. Reuse of an already-rotated token revokes the whole family.
pub async fn refresh_session(shared: &Shared, raw_token: Option<&Value>, device_id: &str, device_name: &str) -> Reply {
    let invalid = || LauncherApiError::new("AUTH_REFRESH_INVALID", "Session expired. Please sign in again.");
    let Some(raw) = raw_token.and_then(Value::as_str).filter(|t| !t.is_empty()) else {
        return Err(invalid());
    };
    let hash = hash_opaque_token(raw);
    let found = store(shared)?
        .read(move |conn, repos| repos.refresh_sessions.find_by_token_hash(conn, &hash))
        .await?;
    let Some(session) = found else { return Err(invalid()) };

    if session.revoked_at.is_some() {
        // Someone replayed an already-rotated token: revoke the whole lineage
        tracing::warn!(
            "Refresh token reuse detected for family {} - revoking.",
            session.family_id
        );
        revoke_session_family(shared, &session).await?;
        return Err(invalid());
    }
    if parse_ms(&session.expires_at).is_some_and(|expires| expires < now_ms()) {
        return Err(invalid());
    }
    let id = session.account.clone();
    let account = store(shared)?
        .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &id))
        .await?;
    let Some(account) = account else {
        revoke_session_family(shared, &session).await?;
        return Err(invalid());
    };
    if let Err(error) = assert_account_admitted(&account) {
        revoke_session_family(shared, &session).await?;
        return Err(error);
    }
    // A restored session is a sign-in for "Last sign-in"
    let signed_in = note_sign_in(shared, &account, now_ms()).await?;

    let new_refresh = generate_opaque_token();
    let now = now_ms();
    let next = RefreshSessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        token_hash: hash_opaque_token(&new_refresh),
        account: session.account.clone(),
        family_id: session.family_id.clone(),
        device_id: if device_id.is_empty() {
            session.device_id.clone()
        } else {
            device_id.to_owned()
        },
        device_name: if device_name.is_empty() {
            session.device_name.clone()
        } else {
            device_name.to_owned()
        },
        created_at: iso_from_ms(now),
        expires_at: iso_from_ms(now + REFRESH_TOKEN_TTL_MS),
        revoked_at: None,
    };
    let (old_id, revoked_at, account_id) = (session.id.clone(), iso_from_ms(now), session.account.clone());
    account_write(shared, &account_id, move |tx, repos| {
        repos.refresh_sessions.revoke_by_id(tx, &old_id, &revoked_at)?;
        repos.refresh_sessions.create(tx, &next)
    })
    .await?;
    let access = sign_launcher_access_token(&shared.settings, session_user(&signed_in), &session.family_id)
        .map_err(internal_error)?;
    Ok(session_result(
        access,
        new_refresh,
        to_account_view(shared, &signed_in).await?,
    ))
}

fn session_user(account: &LauncherAccountRecord) -> &str {
    account.user_id.as_str()
}

/// `Logout`: revokes the session family the access token names.
pub async fn logout(shared: &Shared, account: &AccountId, family_id: &str) -> Reply {
    let (family, now, id) = (family_id.to_owned(), iso_now(), account.clone());
    account_write(shared, account, move |tx, repos| {
        repos.refresh_sessions.revoke_family(tx, &family, &now)?;
        let _ = &id;
        Ok(())
    })
    .await?;
    Ok(json!({}))
}

/// `LogoutAll`.
pub async fn logout_all(shared: &Shared, account: &AccountId) -> Reply {
    let (now, id) = (iso_now(), account.clone());
    account_write(shared, account, move |tx, repos| {
        repos.refresh_sessions.revoke_all_for_account(tx, &id, &now)
    })
    .await?;
    Ok(json!({}))
}

// ------------------------------------------------------------------ the game session

/// What `POST /launcher/v1/game-sessions` asks of the controller, after the route's own checks.
pub struct GameSessionRequest<'a> {
    pub build_changelist: f64,
    pub executable_sha256: &'a str,
    pub runtime_channel: RuntimeChannel,
    pub runtime_manifest_version: Option<&'a Value>,
    pub runtime_artifacts: Option<&'a Value>,
    pub realm: Option<&'a Value>,
    pub launcher: Option<&'a Value>,
}

fn rifts_account(user_id: &str) -> RiftsAccount {
    RiftsAccount(user_id.to_owned())
}

/// `SeamError` from rustic-rifts as the launcher error Node's realms code would have thrown.
pub fn seam_error(error: rifts_seam::SeamError) -> LauncherApiError {
    match error {
        rifts_seam::SeamError::Refused(refusal) => LauncherApiError::new(&refusal.code, refusal.message),
        other => LauncherApiError::new("NOT_IMPLEMENTED", other.to_string()),
    }
}

/// `RequestGameExchangeCode`: the one-time code the game trades for its session at `/account/api/oauth/token`.
pub async fn request_game_exchange_code(
    shared: &Shared,
    account_id: &AccountId,
    user_id: &str,
    launcher_session_id: &str,
    request: GameSessionRequest<'_>,
) -> Reply {
    if shared
        .auth
        .limiter
        .is_limited(&format!("game-session:{user_id}"), 10, 5 * 60 * 1000)
    {
        return Err(LauncherApiError::new(
            "AUTH_RATE_LIMITED",
            "Too many launch attempts. Try again in a few minutes.",
        ));
    }
    let account = launcher_by_user(shared, user_id)
        .await?
        .ok_or_else(|| LauncherApiError::new("AUTH_UNAUTHORIZED", "Sign in required."))?;
    assert_account_eligible(&account)?;
    // A deletion request deactivates the account; MYSTICPARADOX_CONSENT_ENFORCE gates the consent
    assert_account_not_deactivated(&account)?;
    assert_consent_current_when_enforced(&account, &shared.settings)?;
    // Official play (no realm) is refused while MYSTICPARADOX_OFFICIAL_PLAY is off. A realm launch is not refused here:
    // the realm decides at the end of this function.
    let no_realm = matches!(request.realm, None | Some(Value::Null));
    if no_realm && !read_official_play(&shared.settings).available {
        return Err(LauncherApiError::new(
            "OFFICIAL_PLAY_UNAVAILABLE",
            OFFICIAL_PLAY_UNAVAILABLE_MESSAGE,
        ));
    }

    // The server derives the channel from the current roles and verifies the exact runtime files against that channel's
    // signed release manifest, so a launcher cannot keep a cached beta DLL and still receive a ticket
    assert_runtime_channel_matches_account(request.runtime_channel, &account.roles)?;
    let target = shared.version.tables.target_changelist;
    let attestation = assert_runtime_artifact_set_approved(
        request.runtime_channel,
        request.runtime_manifest_version,
        request.runtime_artifacts,
        &runtime_update_root(&shared.settings),
        target,
    )?;
    if request.build_changelist != f64::from(target) {
        return Err(LauncherApiError::new(
            "GAME_BUILD_UNSUPPORTED",
            "This Dauntless build isn't supported. Verify or repair your installation.",
        ));
    }
    let normalized = service_kit::js::trim(request.executable_sha256).to_lowercase();
    if !is_sha256(&normalized) || !approved_executable_hashes(shared).contains(&normalized) {
        tracing::warn!("Rejected game-session request: unapproved executable hash for user={user_id} build={target}");
        return Err(LauncherApiError::new(
            "GAME_BUILD_UNSUPPORTED",
            "This Dauntless installation isn't recognized. Verify or repair your installation.",
        ));
    }

    // An unofficial launcher build: refused only with MYSTICPARADOX_UNOFFICIAL_LAUNCHER=refuse, and never an outdated
    // official build, a report-less older launcher, an allowed test hash or (by default) an admin
    let report = parse_launcher_build_report(request.launcher);
    let verdict =
        shared
            .auth
            .activity
            .classify_launcher_build(shared, request.runtime_channel.as_str(), report.as_ref());
    if verdict == LauncherBuildVerdict::Unofficial {
        let policy = read_unofficial_launcher_policy(&shared.settings);
        for warning in &policy.warnings {
            tracing::warn!("[UnofficialLauncher] {warning}");
        }
        let refused = unofficial_launcher_refuses(verdict, report.as_ref(), &account.roles, &policy);
        let shown = report.as_ref().map_or("?", |r| r.version.as_str());
        tracing::info!(
            "[UnofficialLauncher] {} Play user={user_id} launcher={shown}",
            if refused {
                "refused"
            } else if policy.refuse {
                "exempt"
            } else {
                "observe, would refuse"
            }
        );
        if refused {
            return Err(LauncherApiError::new(
                "LAUNCHER_UNOFFICIAL",
                UNOFFICIAL_LAUNCHER_MESSAGE,
            ));
        }
    }

    // The last gate before the code exists: entering a realm may use an invite and add the member, so nothing above
    // may still refuse
    let realm_json = match request.realm {
        None | Some(Value::Null) => None,
        Some(value) => Some(RiftsJson(value.to_string())),
    };
    let entry = shared
        .rifts
        .entry()
        .authorize_game_session_realm(rifts_account(user_id), realm_json)
        .await
        .map_err(seam_error)?;

    let code = generate_opaque_token();
    let now = now_ms();
    let record = GameExchangeCodeRecord {
        code_hash: hash_opaque_token(&code),
        account: account_id.clone(),
        launcher_session_id: launcher_session_id.to_owned(),
        build_changelist: target,
        executable_sha256: normalized,
        runtime_channel: request.runtime_channel,
        runtime_sha256: attestation.runtime_sha256,
        runtime_manifest_version: attestation.version,
        runtime_artifact_set_sha256: attestation.artifact_set_sha256,
        realm_id: entry.map(|entry| entry.realm.to_string()),
        created_at: iso_from_ms(now),
        expires_at: iso_from_ms(now + EXCHANGE_CODE_TTL_MS),
        consumed_at: None,
    };
    account_write(shared, account_id, move |tx, repos| {
        repos.game_exchange_codes.create(tx, &record)
    })
    .await?;
    // A game session starts here (the activity monitor's playtime), and the launcher build is recorded
    shared.auth.activity.touch_game_session(shared, account_id);
    shared.matchmaking.presence.touch(user_id, now_ms());
    shared
        .auth
        .activity
        .record_launcher_build(shared, account_id, request.runtime_channel.as_str(), report.as_ref());
    Ok(json!({ "exchangeCode": code, "expiresInSeconds": EXCHANGE_CODE_TTL_MS / 1000 }))
}

// ------------------------------------------------------------------ usernames

/// `CheckUsernameAvailable`: is this username valid and free? Pre-auth and read-only.
pub async fn check_username_available(shared: &Shared, raw: &str) -> Reply {
    let value = Value::String(raw.to_owned());
    let username = validate_username(Some(&value))
        .and_then(|name| assert_username_allowed(&name, &shared.settings).map(|()| name));
    let username = match username {
        Ok(name) => name,
        Err(error) => return Ok(json!({ "available": false, "reason": error.message })),
    };
    if username_taken(shared, &username, None).await? {
        return Ok(json!({ "available": false, "reason": "That username is already taken." }));
    }
    Ok(json!({ "available": true }))
}

/// `SetUsername`: sets the authenticated account's unique username (the Discord set-username step).
pub async fn set_username(shared: &Shared, user_id: &str, raw: Option<&Value>) -> Reply {
    let username = validate_username(raw)?;
    let normalized = username.to_lowercase();
    let mut account = launcher_by_user(shared, user_id)
        .await?
        .ok_or_else(|| LauncherApiError::new("NOT_FOUND", "Account not found."))?;
    // Keeping your own name is allowed (even one the policy would refuse now, set before it existed); taking someone
    // else's is not
    if account.username_set == Some(false) || account.display_name_normalized != normalized {
        assert_username_allowed(&username, &shared.settings)?;
    }
    if username_taken(shared, &username, Some(&account.user_id)).await? {
        return Err(LauncherApiError::new(
            "AUTH_DISPLAY_NAME_TAKEN",
            "That username is already taken.",
        ));
    }
    let (id, name, norm) = (account.account.clone(), username.clone(), normalized.clone());
    let outcome = account_write(shared, &account.account.clone(), move |tx, repos| {
        repos.launcher_accounts.set_username(tx, &id, &name, &norm)
    })
    .await?;
    if outcome == CreateOutcome::DisplayNameTaken {
        return Err(LauncherApiError::new(
            "AUTH_DISPLAY_NAME_TAKEN",
            "That username is already taken.",
        ));
    }
    tracing::info!("Username set for {user_id}: {username}");
    account.display_name = username;
    account.display_name_normalized = normalized;
    account.username_set = Some(true);
    to_account_view(shared, &account).await
}

/// What may be logged about a failed mail: its code only.
pub fn log_mail_failure(error: &super::mail::MailError) -> &str {
    describe_mail_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_addresses_are_checked_in_linear_time_and_by_node_rules() {
        for value in [
            "slayer",
            "@example.com",
            "slayer@localhost",
            "slayer@@example.com",
            "slayer @example.com",
            &format!("slayer@{}.com", "a".repeat(250)),
        ] {
            assert!(!is_valid_email(value), "{value}");
        }
        for value in ["slayer@example.com", "a.b+c@sub.example.co"] {
            assert!(is_valid_email(value), "{value}");
        }
        let started = std::time::Instant::now();
        assert!(!is_valid_email(&format!("a@{}\t", ".".repeat(100_000))));
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert!(!is_valid_email_address(None));
        assert!(!is_valid_email_address(Some(&json!(5))));
        assert!(!is_valid_email(&format!("{}@example.com", "a".repeat(65))));
    }

    #[test]
    fn registration_input_asks_for_an_address_and_eight_characters() {
        let ok = json!("slayer@example.com");
        assert!(validate_registration_input(Some(&ok), Some(&json!("12345678"))).is_ok());
        assert_eq!(
            validate_registration_input(Some(&json!("x")), Some(&json!("12345678")))
                .unwrap_err()
                .message,
            "Enter a valid email address."
        );
        assert_eq!(
            validate_registration_input(Some(&ok), Some(&json!("1234567")))
                .unwrap_err()
                .message,
            "Password must be at least 8 characters."
        );
        assert!(validate_registration_input(Some(&ok), None).is_err());
        assert!(validate_registration_input(Some(&ok), Some(&json!(12345678))).is_err());
    }
}
