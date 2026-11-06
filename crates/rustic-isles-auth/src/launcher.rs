// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher API (spec §3.4, §4.2, §4.3, §4.5, §7): accounts, sessions, email codes, Discord, consent,
//! deletion and game launches.

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use module_api::{BoxFuture, GlobalId, IdKind};
use serde_json::{Map, Value, json};

use crate::discord::DiscordConfig;
use crate::errors::LauncherError;
use crate::launcher_build::LauncherPolicy;
use crate::mail::SmtpConfig;
use crate::policy::{self, DELETION_HOLD_DAYS, MINIMUM_AGE, PRIVACY_VERSION, TERMS_VERSION};
use crate::seams::RealmError;
use crate::store::{
    Approval, Channel, Consent, CreateOutcome, Deletion, DiscordTransaction, EmailCode, ExchangeCode, Identity,
    LauncherAccount, NewLauncherAccount, Purpose, RefreshSession, Status, StoreError,
};
use crate::util::{self, field, ok};
use crate::{Auth, AuthStore, email, hashes, launcher_build, password, runtime};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const SESSION_TTL_MS: i64 = 30 * DAY;
const EXCHANGE_TTL_MS: i64 = 60_000;
const DISCORD_TTL_MS: i64 = 5 * MINUTE;
const SIGN_IN_NOTE_MS: i64 = 5 * MINUTE;
const UNVERIFIED_TTL_MS: i64 = 48 * HOUR;

const MSG_SIGN_IN: &str = "Sign in required.";
const MSG_EXPIRED: &str = "Your session has expired. Please sign in again.";
const MSG_ATTEMPTS: &str = "Too many attempts. Try again later.";
const MSG_CREDENTIALS: &str = "The email or password is incorrect.";
const MSG_EMAIL: &str = "Enter a valid email address.";
const MSG_PASSWORD: &str = "Password must be at least 8 characters.";
const MSG_CODE: &str = "That code is wrong or has expired. Check the latest email or resend the code.";
const MSG_NOT_FOUND: &str = "Account not found.";
const MSG_DISCORD_OFF: &str = "Discord sign-in isn't set up on this server yet.";

impl From<StoreError> for LauncherError {
    fn from(error: StoreError) -> Self {
        tracing::error!("auth store failure: {error}");
        Self::internal()
    }
}

impl From<crate::TokenError> for LauncherError {
    fn from(error: crate::TokenError) -> Self {
        tracing::error!("token failure: {error}");
        Self::internal()
    }
}

type Answer = Result<Response, LauncherError>;

fn unauthorized(message: &str) -> LauncherError {
    LauncherError::new("AUTH_UNAUTHORIZED", message)
}

fn rate_limited(message: &str) -> LauncherError {
    LauncherError::new("AUTH_RATE_LIMITED", message)
}

fn not_found() -> LauncherError {
    LauncherError::new("NOT_FOUND", MSG_NOT_FOUND)
}

fn code_invalid() -> LauncherError {
    LauncherError::new("AUTH_CODE_INVALID", MSG_CODE)
}

/// What the launcher CORS gate does with a request (§4.5).
pub enum Cors {
    /// Not a launcher path, or no `Origin`
    Pass,
    /// Answer this instead (a refusal, or a preflight)
    Answer(Response),
    /// Run the route, then set these on whatever answers
    Headers(Vec<(HeaderName, HeaderValue)>),
}

/// The signed-in launcher caller (§4.2).
#[derive(Debug, Clone)]
pub struct LauncherAuth {
    pub user_id: String,
    /// The refresh family, or `apikey` for an account-key caller
    pub sid: String,
    pub account: LauncherAccount,
}

/// Device fields of a sign-in body.
fn device(body: Option<&Value>) -> (Option<String>, String) {
    let id = util::non_empty_str(field(body, "deviceId")).map(str::to_owned);
    let name = util::non_empty_str(field(body, "deviceName"))
        .unwrap_or("Unknown device")
        .to_owned();
    (id, name)
}

enum Issued {
    Sent,
    Limited,
    Failed,
}

impl<S: AuthStore> Auth<S> {
    fn limited(&self, key: &str, max: usize, window_ms: i64) -> bool {
        self.limiter.is_limited_at(key, max, window_ms, self.now())
    }

    fn limit(&self, key: &str, max: usize, window_ms: i64, message: &str) -> Result<(), LauncherError> {
        if self.limited(key, max, window_ms) {
            return Err(rate_limited(message));
        }
        Ok(())
    }

    fn require_mail(&self) -> Result<SmtpConfig, LauncherError> {
        self.smtp().map_err(|_| {
            LauncherError::new(
                "AUTH_EMAIL_UNAVAILABLE",
                "Email codes aren't available on this server right now. Try again later.",
            )
        })
    }

    pub fn launcher_cors(&self, request: &Request) -> Cors {
        if !request.uri().path().to_ascii_lowercase().starts_with("/launcher/") {
            return Cors::Pass;
        }
        let origin = util::header(request.headers(), "origin").filter(|o| !o.is_empty());
        let preflight = request.method() == Method::OPTIONS;
        let Some(origin) = origin else {
            return if preflight {
                Cors::Answer(StatusCode::NO_CONTENT.into_response())
            } else {
                Cors::Pass
            };
        };
        let configured = self.raw("LAUNCHER_ALLOWED_ORIGINS").unwrap_or("");
        let allowed = ["http://localhost:1420", "https://tauri.localhost"].contains(&origin.as_str())
            || configured
                .split(',')
                .map(service_kit::js::trim)
                .any(|o| !o.is_empty() && o == origin);
        if !allowed {
            tracing::warn!("[CORS] Rejected launcher origin");
            return Cors::Answer(StatusCode::FORBIDDEN.into_response());
        }
        let Ok(origin_value) = HeaderValue::from_str(&origin) else {
            return Cors::Answer(StatusCode::FORBIDDEN.into_response());
        };
        let headers = vec![
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, origin_value),
            (header::VARY, HeaderValue::from_static("Origin")),
            (
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
            ),
            (
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("Content-Type, Authorization"),
            ),
        ];
        if preflight {
            let mut response = StatusCode::NO_CONTENT.into_response();
            for (name, value) in headers {
                response.headers_mut().insert(name, value);
            }
            return Cors::Answer(response);
        }
        Cors::Headers(headers)
    }

    async fn bearer_inner(&self, authorization: &str) -> Result<LauncherAuth, LauncherError> {
        if !["bearer ", "Bearer ", "BEARER "]
            .iter()
            .any(|p| authorization.starts_with(p))
        {
            return Err(unauthorized(MSG_SIGN_IN));
        }
        let expired = || unauthorized(MSG_EXPIRED);
        let (user_id, sid) = self
            .validate_launcher_token(&authorization[7..])
            .map_err(|_| expired())?;
        let store = self.store().map_err(|_| expired())?;
        let account = store
            .launcher_account_by_user_id(&user_id)
            .await
            .map_err(|_| expired())?
            .ok_or_else(expired)?;
        let active = store
            .is_family_active(&sid, &account.account_id, &self.now_iso())
            .await
            .map_err(|_| expired())?;
        if !active {
            return Err(expired());
        }
        policy::assert_account_admitted(&account)?;
        Ok(LauncherAuth { user_id, sid, account })
    }

    /// The bearer half of §4.2, with the account it found.
    pub fn launcher_auth_bearer<'a>(
        &'a self,
        authorization: Option<&'a str>,
    ) -> BoxFuture<'a, Result<LauncherAuth, LauncherError>> {
        Box::pin(async move {
            match authorization {
                Some(authorization) => self.bearer_inner(authorization).await,
                None => Err(unauthorized(MSG_SIGN_IN)),
            }
        })
    }

    /// §4.2: a bearer, else an account key, else `Sign in required.`
    pub fn launcher_auth<'a>(&'a self, headers: &'a HeaderMap) -> BoxFuture<'a, Result<LauncherAuth, LauncherError>> {
        Box::pin(async move {
            if let Some(authorization) = util::header(headers, "authorization") {
                return self.bearer_inner(&authorization).await;
            }
            let key = util::header(headers, "x-reforged-user-api-key")
                .map(|k| service_kit::js::trim(&k).to_owned())
                .filter(|k| !k.is_empty());
            let Some(key) = key else {
                return Err(unauthorized(MSG_SIGN_IN));
            };
            let owner = self.user_key_owner(&key).await.map_err(|_| unauthorized(MSG_EXPIRED))?;
            let Some((user_id, _)) = owner else {
                return Err(unauthorized(MSG_SIGN_IN));
            };
            let account = self
                .store()
                .map_err(|_| unauthorized(MSG_EXPIRED))?
                .launcher_account_by_user_id(&user_id)
                .await
                .map_err(|_| unauthorized(MSG_EXPIRED))?
                .ok_or_else(|| unauthorized(MSG_EXPIRED))?;
            policy::assert_account_admitted(&account)?;
            Ok(LauncherAuth {
                user_id,
                sid: "apikey".into(),
                account,
            })
        })
    }

    /// §4.3.
    pub fn require_active_account<'a>(
        &'a self,
        user_id: &'a str,
        consent: bool,
    ) -> BoxFuture<'a, Result<LauncherAccount, LauncherError>> {
        Box::pin(async move {
            let account = self
                .store()?
                .launcher_account_by_user_id(user_id)
                .await?
                .ok_or_else(|| unauthorized(MSG_SIGN_IN))?;
            policy::assert_not_deactivated(&account)?;
            if consent {
                policy::assert_consent_if_enforced(&account, self.consent_enforced())?;
            }
            Ok(account)
        })
    }

    fn consent_enforced(&self) -> bool {
        policy::consent_enforced(self.raw("RUSTIC_ISLES_CONSENT_ENFORCE"))
    }

    async fn account_of(&self, user_id: &str) -> Result<LauncherAccount, LauncherError> {
        self.store()?
            .launcher_account_by_user_id(user_id)
            .await?
            .ok_or_else(not_found)
    }

    /// The account view; its field order is the contract.
    async fn view(&self, account: &LauncherAccount) -> Result<Value, LauncherError> {
        let identity = self.store()?.identity_by_account(&account.account_id).await?;
        let discord = match &identity {
            Some(identity) => json!({"linked": true, "username": identity.provider_username}),
            None => json!({"linked": false}),
        };
        Ok(util::object(vec![
            ("userId", Value::String(account.user_id.clone())),
            ("displayName", Value::String(account.display_name.clone())),
            ("email", Value::String(account.email.clone().unwrap_or_default())),
            ("discordLinked", Value::Bool(identity.is_some())),
            ("discord", discord),
            ("emailVerified", Value::Bool(account.email_verified_at.is_some())),
            ("status", json!(account.status.as_str())),
            ("approvalStatus", json!(account.effective_approval().as_str())),
            ("needsUsername", Value::Bool(account.username_set == Some(false))),
        ]))
    }

    async fn view_answer(&self, account: &LauncherAccount) -> Answer {
        Ok(ok(self.view(account).await?))
    }

    /// A new refresh family (§5.2): the session answer.
    async fn new_session(&self, account: &LauncherAccount, body: Option<&Value>) -> Answer {
        let (device_id, device_name) = device(body);
        let token = util::random_hex::<32>();
        let now = self.now();
        let session = RefreshSession {
            id: self.uuid(),
            token_hash: util::sha256_hex(&token),
            account_id: account.account_id.clone(),
            family_id: self.uuid(),
            device_id: device_id.unwrap_or_else(|| self.uuid()),
            device_name,
            created_at: util::iso(now),
            expires_at: util::iso(now + SESSION_TTL_MS),
            revoked_at: None,
        };
        let actor = self.player_actor(&account.account_id)?;
        self.store()?.create_session(&actor, &session).await?;
        self.session_answer(account, &session.family_id, &token).await
    }

    async fn session_answer(&self, account: &LauncherAccount, family: &str, token: &str) -> Answer {
        let access = self.sign_launcher_token(&account.user_id, family)?;
        Ok(ok(util::object(vec![
            ("accessToken", Value::String(access)),
            ("refreshToken", Value::String(token.to_owned())),
            ("account", self.view(account).await?),
        ])))
    }

    /// Password, verify and reset sign-ins: `last_login_at` now, then a session.
    async fn sign_in(&self, mut account: LauncherAccount, body: Option<&Value>) -> Answer {
        let now = self.now_iso();
        let actor = self.player_actor(&account.account_id)?;
        self.store()?.set_last_login(&actor, &account.account_id, &now).await?;
        account.last_login_at = Some(now);
        self.new_session(&account, body).await
    }

    /// §3.4.11: skipped while the stored sign-in is under five minutes old.
    async fn note_sign_in(&self, account: &mut LauncherAccount) -> Result<(), LauncherError> {
        let now = self.now();
        let recent = account
            .last_login_at
            .as_deref()
            .and_then(service_kit::js::date_parse)
            .is_some_and(|at| (0..SIGN_IN_NOTE_MS).contains(&(now - at)));
        if recent {
            return Ok(());
        }
        let now = util::iso(now);
        let actor = self.player_actor(&account.account_id)?;
        self.store()?.set_last_login(&actor, &account.account_id, &now).await?;
        account.last_login_at = Some(now);
        Ok(())
    }

    // Email codes (§7.3)

    async fn issue_code(
        &self,
        account: &LauncherAccount,
        to: &str,
        purpose: Purpose,
        ip: &str,
    ) -> Result<Issued, LauncherError> {
        let p = purpose.as_str();
        let user = &account.user_id;
        if self.limited(&format!("mail-minute:{p}:{user}"), 1, MINUTE)
            || self.limited(&format!("mail-hour:{p}:{user}"), 5, HOUR)
            || self.limited(&format!("mail-ip:{ip}"), 20, HOUR)
        {
            return Ok(Issued::Limited);
        }
        let secret = self.secret().map_err(|e| StoreError::new(e.to_string()))?;
        let code = util::random_code();
        let now = self.now();
        let row = EmailCode {
            account_id: account.account_id.clone(),
            purpose,
            code_hash: hashes::email_code_hash(secret, p, user, &code),
            attempts: 0,
            created_at: util::iso(now),
            expires_at: util::iso(now + email::CODE_TTL_MS),
        };
        let actor = self.player_actor(&account.account_id)?;
        self.store()?.replace_email_code(&actor, &row).await?;
        let sent = async {
            let config = self.smtp().map_err(|_| crate::MailError::new("NOT_CONFIGURED"))?;
            let message = crate::mail::render_code_mail(to, purpose, &code, 5)?;
            self.seams.mail.send(&config, message).await
        }
        .await;
        match sent {
            Ok(()) => Ok(Issued::Sent),
            Err(error) => {
                tracing::warn!("code mail to {} failed: {}", email::mask_email(to), error.code);
                Ok(Issued::Failed)
            }
        }
    }

    async fn accept_code(
        &self,
        account: &LauncherAccount,
        purpose: Purpose,
        submitted: Option<&Value>,
    ) -> Result<bool, LauncherError> {
        let Some(code) = submitted.and_then(Value::as_str).filter(|c| email::is_code_shape(c)) else {
            return Ok(false);
        };
        let secret = self.secret().map_err(|e| StoreError::new(e.to_string()))?;
        let hash = hashes::email_code_hash(secret, purpose.as_str(), &account.user_id, code);
        let store = self.store()?;
        let now = self.now_iso();
        let actor = self.player_actor(&account.account_id)?;
        let live = store.live_email_code(&account.account_id, purpose, &now).await?;
        if let Some(live) = live
            && hashes::hashes_equal(&live.code_hash, &hash)
        {
            return Ok(store
                .consume_email_code(&actor, &account.account_id, purpose, &hash, &now)
                .await?);
        }
        store
            .count_email_code_try(&actor, &account.account_id, purpose, &now, email::CODE_MAX_ATTEMPTS)
            .await?;
        Ok(false)
    }

    /// Resend and forgot: mailed after the answer, to an active account only.
    fn mail_in_background(&self, normalized: String, purpose: Purpose, ip: String) {
        let Some(this) = self.this.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            let found = match this.store() {
                Ok(store) => store.launcher_account_by_email(&normalized).await,
                Err(error) => Err(error),
            };
            let account = match found {
                Ok(Some(account)) if account.status == Status::Active => account,
                Ok(_) => return,
                Err(error) => {
                    tracing::warn!("background code mail: {error}");
                    return;
                }
            };
            if purpose == Purpose::Verify && account.email_verified_at.is_some() {
                return;
            }
            let to = account.email.clone().unwrap_or_default();
            if let Err(error) = this.issue_code(&account, &to, purpose, &ip).await {
                tracing::warn!("background code mail failed: {}", error.code);
            }
        });
    }

    async fn credentials(&self, body: Option<&Value>) -> Result<Option<(String, String)>, LauncherError> {
        match (field(body, "email"), field(body, "password")) {
            (Some(Value::String(e)), Some(Value::String(p))) => Ok(Some((util::normalize_lower(e), p.clone()))),
            _ => Ok(None),
        }
    }

    /// The account of an address with its password verified; equal work on a miss.
    async fn check_password(&self, normalized: &str, password: &str) -> Result<Option<LauncherAccount>, LauncherError> {
        let account = self.store()?.launcher_account_by_email(normalized).await?;
        let hash = account.as_ref().and_then(|a| a.password_hash.clone());
        let matched = password::verify_password_blocking(hash, password.to_owned()).await;
        Ok(account.filter(|_| matched))
    }

    // Routes

    pub(crate) async fn register(&self, ip: &str, body: Option<Value>) -> Answer {
        let body = body.as_ref();
        self.limit(
            &format!("register:{ip}"),
            5,
            HOUR,
            "Too many registration attempts. Try again later.",
        )?;
        let name = match field(body, "displayName") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if service_kit::js::trim(s).is_empty() => None,
            Some(value) => {
                let name = policy::validate_username_format(Some(value))?;
                let blocklist = policy::parse_blocklist(self.raw("RUSTIC_ISLES_USERNAME_BLOCKLIST"));
                policy::check_username_allowed(&name, &blocklist)?;
                Some(name)
            }
        };
        let email_text = match field(body, "email") {
            Some(Value::String(e)) if email::is_valid_email(e) => e.clone(),
            _ => return Err(LauncherError::validation(MSG_EMAIL)),
        };
        let password = match field(body, "password") {
            Some(Value::String(p)) if util::utf16_len(p) >= 8 => p.clone(),
            _ => return Err(LauncherError::validation(MSG_PASSWORD)),
        };
        let now = self.now();
        let now_iso = util::iso(now);
        let required = policy::consent_required_at_signup(
            self.raw("RUSTIC_ISLES_CONSENT_REQUIRED"),
            self.config.engine_is_postgres,
        );
        let consent = policy::registration_consent(field(body, "consent"), required, &now_iso)?;
        let normalized = util::normalize_lower(&email_text);
        let verification = self.verification_required();
        if verification {
            self.require_mail()?;
        }
        let store = self.store()?;
        let email_taken = || LauncherError::new("AUTH_EMAIL_TAKEN", "That email is already registered.");
        let name_taken = || LauncherError::new("AUTH_DISPLAY_NAME_TAKEN", "That display name is already taken.");
        if store.launcher_account_by_email(&normalized).await?.is_some() {
            return Err(email_taken());
        }
        if let Some(name) = &name
            && store.launcher_account_by_name(&name.to_lowercase()).await?.is_some()
        {
            return Err(name_taken());
        }
        let hash = password::hash_password_blocking(password)
            .await
            .map_err(|_| LauncherError::internal())?;
        let user_id = self.uuid();
        let display_name = name.clone().unwrap_or_else(|| user_id.clone());
        let (approval_required, _) = policy::registration_approval(self.raw("RUSTIC_ISLES_REGISTRATION_APPROVAL"));
        let account = LauncherAccount {
            account_id: GlobalId::new(IdKind::Account, &self.config.node.node).to_string(),
            user_id: user_id.clone(),
            email: Some(normalized.clone()),
            display_name: display_name.clone(),
            display_name_normalized: display_name.to_lowercase(),
            password_hash: Some(hash),
            status: Status::Active,
            approval: Some(if approval_required {
                Approval::Pending
            } else {
                Approval::Approved
            }),
            roles: vec!["player".into()],
            roles_updated_at: None,
            created_at: now_iso.clone(),
            last_login_at: None,
            username_set: Some(name.is_some()),
            consent,
            deletion: None,
            email_verified_at: None,
            email_verification_pending_since: verification.then(|| now_iso.clone()),
            timeout: None,
        };
        let created = store
            .create_launcher_account(
                &self.system_actor(),
                &NewLauncherAccount {
                    account,
                    game_name: display_name,
                },
            )
            .await?;
        let account = match created {
            CreateOutcome::Created(account) => account,
            CreateOutcome::EmailTaken => return Err(email_taken()),
            CreateOutcome::NameTaken => return Err(name_taken()),
        };
        let pending = account.effective_approval() == Approval::Pending;
        let view = self.view(&account).await?;
        let answer = if verification {
            if let Err(error) = self.issue_code(&account, &normalized, Purpose::Verify, ip).await {
                tracing::warn!("first verify code not stored: {}", error.code);
            }
            util::object(vec![
                ("approvalRequired", Value::Bool(pending)),
                ("verificationRequired", Value::Bool(true)),
                ("email", Value::String(normalized)),
                ("account", view),
            ])
        } else {
            util::object(vec![("approvalRequired", Value::Bool(pending)), ("account", view)])
        };
        Ok(util::json(StatusCode::ACCEPTED, answer))
    }

    pub(crate) async fn login_route(&self, ip: &str, body: Option<Value>) -> Answer {
        let body = body.as_ref();
        let message = "Too many login attempts. Try again later.";
        self.limit(&format!("login:{ip}"), 10, 5 * MINUTE, message)?;
        let email_value = field(body, "email");
        if !matches!(email_value, None | Some(Value::Null | Value::String(_))) {
            return Err(LauncherError::internal());
        }
        let normalized = email_value
            .and_then(Value::as_str)
            .map(util::normalize_lower)
            .unwrap_or_default();
        self.limit(&format!("login:{normalized}"), 10, 5 * MINUTE, message)?;
        let invalid = || LauncherError::new("AUTH_INVALID_CREDENTIALS", MSG_CREDENTIALS);
        let Some((normalized, password)) = self.credentials(body).await? else {
            return Err(invalid());
        };
        let account = self.check_password(&normalized, &password).await?.ok_or_else(invalid)?;
        policy::assert_not_blocked(&account)?;
        if self.verification_required() && account.email_verified_at.is_none() {
            self.require_mail()?;
            let to = account.email.clone().unwrap_or_default();
            self.issue_code(&account, &to, Purpose::Verify, ip).await?;
            return Err(LauncherError::new(
                "AUTH_EMAIL_UNVERIFIED",
                format!(
                    "Enter the 6-digit code we sent to {}. Older launchers: update the launcher to verify your email.",
                    email::mask_email(&to)
                ),
            ));
        }
        policy::assert_account_admitted(&account)?;
        self.sign_in(account, body).await
    }

    pub(crate) async fn refresh(&self, body: Option<Value>) -> Answer {
        let body = body.as_ref();
        let refuse = || LauncherError::new("AUTH_REFRESH_INVALID", "Session expired. Please sign in again.");
        let Some(token) = util::non_empty_str(field(body, "refreshToken")) else {
            return Err(refuse());
        };
        let store = self.store()?;
        let session = store
            .session_by_token_hash(&util::sha256_hex(token))
            .await?
            .ok_or_else(refuse)?;
        let now = self.now();
        let now_iso = util::iso(now);
        let actor = self.player_actor(&session.account_id)?;
        if session.revoked_at.is_some() {
            store.revoke_family(&actor, &session.family_id, &now_iso).await?;
            return Err(refuse());
        }
        if session.expires_at < now_iso {
            return Err(refuse());
        }
        let Some(mut account) = store.launcher_account_by_id(&session.account_id).await? else {
            store.revoke_family(&actor, &session.family_id, &now_iso).await?;
            return Err(refuse());
        };
        if let Err(error) = policy::assert_account_admitted(&account) {
            store.revoke_family(&actor, &session.family_id, &now_iso).await?;
            return Err(error);
        }
        self.note_sign_in(&mut account).await?;
        let (device_id, device_name) = device(body);
        let next_token = util::random_hex::<32>();
        let next = RefreshSession {
            id: self.uuid(),
            token_hash: util::sha256_hex(&next_token),
            account_id: account.account_id.clone(),
            family_id: session.family_id.clone(),
            device_id: device_id.unwrap_or_else(|| self.uuid()),
            device_name,
            created_at: now_iso.clone(),
            expires_at: util::iso(now + SESSION_TTL_MS),
            revoked_at: None,
        };
        store.rotate_session(&actor, &session.id, &now_iso, &next).await?;
        self.session_answer(&account, &session.family_id, &next_token).await
    }

    pub(crate) async fn logout(&self, caller: &LauncherAuth, all: bool) -> Answer {
        let store = self.store()?;
        let actor = self.player_actor(&caller.account.account_id)?;
        let now = self.now_iso();
        if all {
            store
                .revoke_all_sessions(&actor, &caller.account.account_id, &now)
                .await?;
        } else {
            store.revoke_family(&actor, &caller.sid, &now).await?;
        }
        Ok(ok(json!({})))
    }

    /// The pending answer, the rejection, or a sign-in (verify and reset).
    async fn after_code(&self, account: LauncherAccount, body: Option<&Value>) -> Answer {
        match account.effective_approval() {
            Approval::Pending => {
                let view = self.view(&account).await?;
                Ok(ok(util::object(vec![
                    ("approvalRequired", Value::Bool(true)),
                    ("account", view),
                ])))
            }
            Approval::Rejected => Err(LauncherError::new("AUTH_APPROVAL_REJECTED", policy::MSG_REJECTED)),
            Approval::Approved => self.sign_in(account, body).await,
        }
    }

    pub(crate) async fn email_verify(&self, ip: &str, body: Option<Value>) -> Answer {
        let body = body.as_ref();
        self.require_mail()?;
        self.limit(&format!("email-verify-ip:{ip}"), 30, 15 * MINUTE, MSG_ATTEMPTS)?;
        let invalid = || LauncherError::new("AUTH_INVALID_CREDENTIALS", MSG_CREDENTIALS);
        let Some((normalized, password)) = self.credentials(body).await? else {
            return Err(invalid());
        };
        self.limit(&format!("email-verify:{normalized}"), 10, 15 * MINUTE, MSG_ATTEMPTS)?;
        let mut account = self.check_password(&normalized, &password).await?.ok_or_else(invalid)?;
        policy::assert_not_blocked(&account)?;
        if !self.accept_code(&account, Purpose::Verify, field(body, "code")).await? {
            return Err(code_invalid());
        }
        let now = self.now_iso();
        let actor = self.player_actor(&account.account_id)?;
        self.store()?
            .set_email_verified(&actor, &account.account_id, &now)
            .await?;
        account.email_verified_at = Some(now);
        account.email_verification_pending_since = None;
        self.after_code(account, body).await
    }

    /// Resend and forgot: the same answer whether the address has an account.
    pub(crate) async fn code_request(&self, ip: &str, body: Option<Value>, purpose: Purpose) -> Answer {
        let (ip_key, email_key) = match purpose {
            Purpose::Reset => ("password-forgot-ip", "password-forgot"),
            _ => ("email-resend-ip", "email-resend"),
        };
        self.require_mail()?;
        self.limit(&format!("{ip_key}:{ip}"), 20, HOUR, MSG_ATTEMPTS)?;
        let normalized = match field(body.as_ref(), "email") {
            Some(Value::String(e)) if email::is_valid_email(e) => util::normalize_lower(e),
            _ => return Err(LauncherError::validation(MSG_EMAIL)),
        };
        self.limit(&format!("{email_key}:{normalized}"), 5, HOUR, MSG_ATTEMPTS)?;
        self.mail_in_background(normalized, purpose, ip.to_owned());
        Ok(ok(json!({"sent": true})))
    }

    pub(crate) async fn password_reset(&self, ip: &str, body: Option<Value>) -> Answer {
        let body = body.as_ref();
        self.require_mail()?;
        self.limit(&format!("password-reset-ip:{ip}"), 30, 15 * MINUTE, MSG_ATTEMPTS)?;
        let normalized = match field(body, "email") {
            Some(Value::String(e)) if email::is_valid_email(e) => util::normalize_lower(e),
            _ => return Err(LauncherError::validation(MSG_EMAIL)),
        };
        self.limit(&format!("password-reset:{normalized}"), 10, 15 * MINUTE, MSG_ATTEMPTS)?;
        let password = match field(body, "newPassword") {
            Some(Value::String(p)) if util::utf16_len(p) >= 8 => p.clone(),
            _ => return Err(LauncherError::validation(MSG_PASSWORD)),
        };
        let store = self.store()?;
        let mut account = store
            .launcher_account_by_email(&normalized)
            .await?
            .ok_or_else(code_invalid)?;
        if !self.accept_code(&account, Purpose::Reset, field(body, "code")).await? {
            return Err(code_invalid());
        }
        // After the code, so only the address's owner learns of a ban
        policy::assert_not_blocked(&account)?;
        let hash = password::hash_password_blocking(password)
            .await
            .map_err(|_| LauncherError::internal())?;
        let now = self.now_iso();
        let actor = self.player_actor(&account.account_id)?;
        store.reset_password(&actor, &account.account_id, &hash, &now).await?;
        account.password_hash = Some(hash);
        account.email_verified_at = Some(now);
        account.email_verification_pending_since = None;
        self.after_code(account, body).await
    }

    // Discord (§7.5)

    fn discord(&self) -> Result<DiscordConfig, LauncherError> {
        DiscordConfig::from_settings(self.settings.as_ref())
            .ok_or_else(|| LauncherError::new("AUTH_DISCORD_NOT_CONFIGURED", MSG_DISCORD_OFF))
    }

    async fn start_transaction(&self, config: &DiscordConfig, link: Option<&str>) -> Answer {
        let state = util::random_hex::<24>();
        let verifier = util::random_base64url::<32>();
        let now = self.now();
        let transaction = DiscordTransaction {
            state: state.clone(),
            code_verifier: verifier.clone(),
            created_at: util::iso(now),
            expires_at: util::iso(now + DISCORD_TTL_MS),
            consumed_at: None,
            completion_code_hash: None,
            completion_expires_at: None,
            completion_consumed_at: None,
            account_id: None,
            link_account_id: link.map(str::to_owned),
        };
        self.store()?
            .create_discord_transaction(&self.system_actor(), &transaction)
            .await?;
        let url = crate::discord::authorize_url(config, &state, &verifier);
        Ok(ok(json!({"authorizeUrl": url})))
    }

    pub(crate) async fn discord_start(&self) -> Answer {
        let config = self.discord()?;
        self.start_transaction(&config, None).await
    }

    pub(crate) async fn discord_link_start(&self, caller: &LauncherAuth) -> Answer {
        let config = self.discord()?;
        let account = self.account_of(&caller.user_id).await?;
        if self.store()?.identity_by_account(&account.account_id).await?.is_some() {
            return Err(LauncherError::new(
                "AUTH_DISCORD_ACCOUNT_HAS_LINK",
                "This account already has a Discord account linked.",
            ));
        }
        self.start_transaction(&config, Some(&account.account_id)).await
    }

    fn scheme(&self) -> &str {
        self.raw("LAUNCHER_DEEPLINK_SCHEME")
            .filter(|s| !s.is_empty())
            .unwrap_or("rusticisles")
    }

    pub(crate) async fn discord_callback(&self, query: Option<&str>, accept: Option<&str>) -> Response {
        let pairs = util::query_pairs(query);
        let once = |name: &str| match util::query_value(&pairs, name) {
            util::QueryValue::One(v) => Some(v),
            _ => None,
        };
        let mut channel = "auth";
        let outcome = self.callback_outcome(once("code"), once("state"), &mut channel).await;
        let scheme = self.scheme();
        let url = match outcome {
            Ok(path) => format!("{scheme}://{path}"),
            Err(code) => {
                let code = util::percent_encode(&code, |b| b.is_ascii_alphanumeric() || b"-_.!~*".contains(&b), false);
                format!("{scheme}://{channel}/error?code={code}")
            }
        };
        redirect(&url, accept)
    }

    /// The deep link's path, or the refusal's code.
    async fn callback_outcome(
        &self,
        code: Option<String>,
        state: Option<String>,
        channel: &mut &'static str,
    ) -> Result<String, String> {
        let cancelled = || "AUTH_DISCORD_CANCELLED".to_owned();
        let internal = |error: StoreError| {
            tracing::error!("discord callback store failure: {error}");
            "INTERNAL".to_owned()
        };
        let (Some(code), Some(state)) = (code, state) else {
            return Err(cancelled());
        };
        let store = self.store().map_err(internal)?;
        let transaction = store
            .consume_discord_state(&self.system_actor(), &state, &self.now_iso())
            .await
            .map_err(internal)?
            .ok_or_else(cancelled)?;
        if transaction.link_account_id.is_some() {
            *channel = "link";
        }
        let config = self.discord().map_err(|_| "AUTH_DISCORD_NOT_CONFIGURED".to_owned())?;
        let user = match self
            .seams
            .discord
            .exchange_code(&config, &code, &transaction.code_verifier)
            .await
        {
            Ok(user) => user,
            Err(failure) => {
                tracing::warn!("discord exchange failed (status {:?})", failure.status);
                return Err(cancelled());
            }
        };
        if let Some(link) = &transaction.link_account_id {
            let account = store
                .launcher_account_by_id(link)
                .await
                .map_err(internal)?
                .ok_or_else(cancelled)?;
            if store
                .identity_by_account(&account.account_id)
                .await
                .map_err(internal)?
                .is_some()
            {
                return Err("AUTH_DISCORD_ACCOUNT_HAS_LINK".into());
            }
            if store.identity_by_subject(&user.id).await.map_err(internal)?.is_some() {
                return Err("AUTH_DISCORD_ALREADY_LINKED".into());
            }
            let identity = Identity {
                provider_subject: user.id.clone(),
                account_id: account.account_id.clone(),
                provider_username: user.username.clone(),
                provider_avatar_url: user
                    .avatar
                    .as_ref()
                    .map(|avatar| format!("https://cdn.discordapp.com/avatars/{}/{avatar}.png", user.id)),
                linked_at: self.now_iso(),
            };
            let actor = self.player_actor(&account.account_id).map_err(internal)?;
            if !store.create_identity(&actor, &identity).await.map_err(internal)? {
                return Err("AUTH_DISCORD_ALREADY_LINKED".into());
            }
            return Ok("link/done".into());
        }
        let identity = store
            .identity_by_subject(&user.id)
            .await
            .map_err(internal)?
            .ok_or_else(|| "AUTH_DISCORD_NOT_LINKED".to_owned())?;
        let completion = util::random_hex::<32>();
        let expires = util::iso(self.now() + EXCHANGE_TTL_MS);
        store
            .attach_discord_completion(
                &self.system_actor(),
                &state,
                &util::sha256_hex(&completion),
                &expires,
                &identity.account_id,
            )
            .await
            .map_err(internal)?;
        Ok(format!("auth/complete?code={completion}"))
    }

    pub(crate) async fn discord_complete(&self, body: Option<Value>) -> Answer {
        let body = body.as_ref();
        let incomplete = || {
            LauncherError::new(
                "AUTH_DISCORD_CANCELLED",
                "Discord sign-in didn't complete. Please try again.",
            )
        };
        let Some(code) = util::non_empty_str(field(body, "code")) else {
            return Err(incomplete());
        };
        let store = self.store()?;
        let transaction = store
            .consume_discord_completion(&self.system_actor(), &util::sha256_hex(code), &self.now_iso())
            .await?;
        let Some(account_id) = transaction.and_then(|t| t.account_id) else {
            return Err(LauncherError::new(
                "AUTH_DISCORD_CANCELLED",
                "That sign-in link has expired. Please try again.",
            ));
        };
        let mut account = store
            .launcher_account_by_id(&account_id)
            .await?
            .ok_or_else(incomplete)?;
        policy::assert_account_admitted(&account)?;
        self.note_sign_in(&mut account).await?;
        self.new_session(&account, body).await
    }

    /// The checks both unlink routes run after their limits.
    async fn unlink_checks(&self, caller: &LauncherAuth) -> Result<(LauncherAccount, String), LauncherError> {
        let account = self.account_of(&caller.user_id).await?;
        if self.store()?.identity_by_account(&account.account_id).await?.is_none() {
            return Err(LauncherError::new(
                "AUTH_DISCORD_NOT_LINKED",
                "This account has no Discord account linked.",
            ));
        }
        if account.password_hash.is_none() {
            return Err(LauncherError::new(
                "AUTH_DISCORD_ONLY_LOGIN",
                "Discord is this account's only sign-in, so it can't be unlinked.",
            ));
        }
        let Some(to) = account.email.clone().filter(|e| !e.is_empty()) else {
            return Err(LauncherError::new(
                "AUTH_DISCORD_ONLY_LOGIN",
                "This account has no email address to send the unlink code to, so Discord can't be unlinked.",
            ));
        };
        self.require_mail()?;
        Ok((account, to))
    }

    pub(crate) async fn unlink_start(&self, ip: &str, caller: &LauncherAuth) -> Answer {
        self.limit(&format!("unlink-start-ip:{ip}"), 20, HOUR, MSG_ATTEMPTS)?;
        self.limit(&format!("unlink-start:{}", caller.user_id), 5, HOUR, MSG_ATTEMPTS)?;
        let (account, to) = self.unlink_checks(caller).await?;
        match self.issue_code(&account, &to, Purpose::Unlink, ip).await? {
            Issued::Failed => Err(LauncherError::new(
                "AUTH_EMAIL_UNAVAILABLE",
                "The code email couldn't be sent. Try again in a minute.",
            )),
            Issued::Sent | Issued::Limited => Ok(ok(json!({"sent": true, "email": email::mask_email(&to)}))),
        }
    }

    pub(crate) async fn unlink_confirm(&self, ip: &str, caller: &LauncherAuth, body: Option<Value>) -> Answer {
        self.limit(&format!("unlink-confirm-ip:{ip}"), 30, 15 * MINUTE, MSG_ATTEMPTS)?;
        self.limit(
            &format!("unlink-confirm:{}", caller.user_id),
            10,
            15 * MINUTE,
            MSG_ATTEMPTS,
        )?;
        let (account, _) = self.unlink_checks(caller).await?;
        if !self
            .accept_code(&account, Purpose::Unlink, field(body.as_ref(), "code"))
            .await?
        {
            return Err(code_invalid());
        }
        let actor = self.player_actor(&account.account_id)?;
        self.store()?
            .delete_identities_for_account(&actor, &account.account_id)
            .await?;
        let view = self.view(&account).await?;
        Ok(ok(util::object(vec![("account", view)])))
    }

    // Account and usernames (§3.4.7)

    pub(crate) async fn me(&self, caller: &LauncherAuth) -> Answer {
        let account = self.account_of(&caller.user_id).await?;
        self.view_answer(&account).await
    }

    pub(crate) async fn username_check(&self, query: Option<&str>) -> Answer {
        let pairs = util::query_pairs(query);
        let name: Vec<String> = pairs.into_iter().filter(|(k, _)| k == "name").map(|(_, v)| v).collect();
        let name = Value::String(name.join(","));
        let refused = |reason: &str| Ok(ok(json!({"available": false, "reason": reason})));
        let name = match policy::validate_username_format(Some(&name)) {
            Ok(name) => name,
            Err(error) => return refused(&error.message),
        };
        let blocklist = policy::parse_blocklist(self.raw("RUSTIC_ISLES_USERNAME_BLOCKLIST"));
        if let Err(error) = policy::check_username_allowed(&name, &blocklist) {
            return refused(&error.message);
        }
        if self
            .store()?
            .launcher_account_by_name(&name.to_lowercase())
            .await?
            .is_some()
        {
            return refused("That username is already taken.");
        }
        Ok(ok(json!({"available": true})))
    }

    pub(crate) async fn username_set(&self, caller: &LauncherAuth, body: Option<Value>) -> Answer {
        let name = policy::validate_username_format(field(body.as_ref(), "username"))?;
        let mut account = self.account_of(&caller.user_id).await?;
        let normalized = name.to_lowercase();
        let keeping = account.username_set != Some(false) && normalized == account.display_name_normalized;
        if !keeping {
            let blocklist = policy::parse_blocklist(self.raw("RUSTIC_ISLES_USERNAME_BLOCKLIST"));
            policy::check_username_allowed(&name, &blocklist)?;
        }
        let taken = || LauncherError::new("AUTH_DISPLAY_NAME_TAKEN", "That username is already taken.");
        let store = self.store()?;
        if let Some(holder) = store.launcher_account_by_name(&normalized).await?
            && holder.account_id != account.account_id
        {
            return Err(taken());
        }
        let actor = self.player_actor(&account.account_id)?;
        if !store
            .set_username(&actor, &account.account_id, &name, &normalized)
            .await?
        {
            return Err(taken());
        }
        account.display_name = name;
        account.display_name_normalized = normalized;
        account.username_set = Some(true);
        self.view_answer(&account).await
    }

    // Consent and deletion (§3.4.8)

    fn consent_state(&self, account: &LauncherAccount) -> Value {
        let accepted = match &account.consent {
            Some(c) => json!({
                "termsVersion": c.terms_version,
                "privacyVersion": c.privacy_version,
                "ageConfirmed": true,
                "acceptedAt": c.accepted_at
            }),
            None => Value::Null,
        };
        util::object(vec![
            (
                "required",
                json!({"termsVersion": TERMS_VERSION, "privacyVersion": PRIVACY_VERSION, "minimumAge": MINIMUM_AGE}),
            ),
            ("accepted", accepted),
            ("upToDate", Value::Bool(policy::has_current_consent(account))),
        ])
    }

    pub(crate) async fn consent_get(&self, caller: &LauncherAuth) -> Answer {
        let account = self.account_of(&caller.user_id).await?;
        Ok(ok(self.consent_state(&account)))
    }

    pub(crate) async fn consent_post(&self, caller: &LauncherAuth, body: Option<Value>) -> Answer {
        let body = match body {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        if body.get("ageConfirmed") != Some(&Value::Bool(true)) {
            return Err(LauncherError::new(
                "CONSENT_AGE_REQUIRED",
                "You must be at least 16 years old to play Rustic Isles.",
            ));
        }
        let (Some(Value::String(terms)), Some(Value::String(privacy))) =
            (body.get("termsVersion"), body.get("privacyVersion"))
        else {
            return Err(LauncherError::validation(
                "termsVersion and privacyVersion are required.",
            ));
        };
        if terms != TERMS_VERSION || !policy::is_privacy_accepted(privacy) {
            return Err(LauncherError::new(
                "LEGAL_VERSION_OUTDATED",
                "These Terms of Use or this Privacy Policy have changed. Update the launcher, read them again and accept.",
            )
            .with_detail("termsVersion", json!(TERMS_VERSION))
            .with_detail("privacyVersion", json!(PRIVACY_VERSION)));
        }
        let mut account = self.account_of(&caller.user_id).await?;
        let consent = Consent {
            terms_version: TERMS_VERSION.into(),
            privacy_version: privacy.clone(),
            accepted_at: self.now_iso(),
        };
        let actor = self.player_actor(&account.account_id)?;
        self.store()?.set_consent(&actor, &account.account_id, &consent).await?;
        account.consent = Some(consent);
        Ok(ok(self.consent_state(&account)))
    }

    fn deletion_state(deletion: Option<&Deletion>) -> Value {
        let request = match deletion {
            Some(d) => json!({"requestedAt": d.requested_at, "scheduledFor": d.scheduled_for}),
            None => Value::Null,
        };
        util::object(vec![("request", request), ("holdDays", json!(DELETION_HOLD_DAYS))])
    }

    pub(crate) async fn deletion_get(&self, caller: &LauncherAuth) -> Answer {
        let account = self.account_of(&caller.user_id).await?;
        Ok(ok(Self::deletion_state(account.deletion.as_ref())))
    }

    pub(crate) async fn deletion_post(&self, caller: &LauncherAuth, body: Option<Value>) -> Answer {
        let account = self.account_of(&caller.user_id).await?;
        let confirm = match field(body.as_ref(), "confirmUsername") {
            Some(Value::String(c)) => service_kit::js::trim(c).to_owned(),
            _ => String::new(),
        };
        if confirm.is_empty() || confirm.to_lowercase() != account.display_name.to_lowercase() {
            return Err(LauncherError::new(
                "DELETION_CONFIRMATION_MISMATCH",
                "Type your username exactly as it is shown to confirm the deletion.",
            ));
        }
        if let Some(existing) = &account.deletion {
            return Ok(ok(Self::deletion_state(Some(existing))));
        }
        let now = self.now();
        let deletion = Deletion {
            requested_at: util::iso(now),
            scheduled_for: util::iso(now + DELETION_HOLD_DAYS * DAY),
        };
        let actor = self.player_actor(&account.account_id)?;
        self.store()?
            .set_deletion(&actor, &account.account_id, Some(&deletion))
            .await?;
        Ok(ok(Self::deletion_state(Some(&deletion))))
    }

    pub(crate) async fn deletion_delete(&self, caller: &LauncherAuth) -> Answer {
        let account = self.account_of(&caller.user_id).await?;
        let Some(deletion) = &account.deletion else {
            return Ok(ok(Self::deletion_state(None)));
        };
        let now = self.now();
        let ended = match service_kit::js::date_parse(&deletion.scheduled_for) {
            Some(at) => at <= now,
            None => deletion.scheduled_for <= util::iso(now),
        };
        if ended {
            return Err(LauncherError::new(
                "DELETION_HOLD_EXPIRED",
                "The 30-day hold has ended, so this account can no longer be restored.",
            ));
        }
        let actor = self.player_actor(&account.account_id)?;
        self.store()?.set_deletion(&actor, &account.account_id, None).await?;
        Ok(ok(Self::deletion_state(None)))
    }

    // Reads (§3.4.9)

    pub(crate) async fn policy_route(&self, caller: &LauncherAuth) -> Answer {
        let account = self.account_of(&caller.user_id).await?;
        let ops = self.seams.operations.policy().await;
        let development = ops.diagnostics_profile == "DEVELOPMENT" && account.roles.iter().any(|r| r == "admin");
        let since = account
            .roles_updated_at
            .clone()
            .unwrap_or_else(|| account.created_at.clone());
        let mut answer = util::object(vec![
            ("policyVersion", Value::String(format!("{since}:ops-{}", ops.version))),
            ("roles", json!(account.roles)),
            ("channel", json!("stable")),
            (
                "managedFeatureIds",
                if development {
                    json!(["diagnostics.verbose"])
                } else {
                    json!([])
                },
            ),
            ("logUpload", json!({"auto": development})),
            ("guardEnforcement", Value::String(ops.guard_enforcement.clone())),
            (
                "diagnosticsProfile",
                json!(if development { "DEVELOPMENT" } else { "PRODUCTION" }),
            ),
        ]);
        let fields = self.seams.realms.launcher_policy_fields(&account).await?;
        if let Value::Object(map) = &mut answer {
            for (name, text) in fields {
                let value: Value = serde_json::from_str(&text).map_err(|_| LauncherError::internal())?;
                map.insert(name, value);
            }
        }
        Ok(ok(answer))
    }

    pub(crate) async fn status_route(&self) -> Answer {
        let online = match &self.store {
            Some(store) => store.health().await.is_ok(),
            None => false,
        };
        let official = if self.official_play() {
            json!({"available": true})
        } else {
            json!({"available": false, "message": policy::MSG_OFFICIAL_UNAVAILABLE})
        };
        Ok(ok(util::object(vec![
            ("online", Value::Bool(online)),
            ("supportedBuildChangelist", json!(self.config.target_changelist)),
            ("officialPlay", official),
        ])))
    }

    pub(crate) fn legal() -> Response {
        ok(json!({"termsVersion": TERMS_VERSION, "privacyVersion": PRIVACY_VERSION, "minimumAge": MINIMUM_AGE}))
    }

    pub(crate) async fn notices(&self) -> Response {
        use std::sync::atomic::Ordering;
        let now = self.now();
        let view = match self.seams.notices.public_view(now).await {
            Ok(view) => {
                self.notices_failing.store(false, Ordering::Relaxed);
                view
            }
            Err(error) => {
                if !self.notices_failing.swap(true, Ordering::Relaxed) {
                    tracing::error!("launcher notices unavailable: {error}");
                }
                crate::seams::empty_notices(now)
            }
        };
        let mut response = ok(view);
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=30"));
        response
    }

    // Game sessions (§3.4.10)

    pub(crate) async fn game_sessions(&self, caller: &LauncherAuth, body: Option<Value>) -> Answer {
        let body = body.as_ref();
        let channel = field(body, "runtimeChannel")
            .and_then(Value::as_str)
            .and_then(Channel::parse);
        let changelist = service_kit::js::number(field(body, "buildChangelist"));
        let executable = util::non_empty_str(field(body, "executableSha256"));
        let Some(channel) = channel else {
            return Err(LauncherError::validation(
                "runtimeChannel must be stable, beta, or dev.",
            ));
        };
        if !changelist.is_finite() {
            return Err(LauncherError::validation("buildChangelist is required."));
        }
        let Some(executable) = executable else {
            return Err(LauncherError::validation("executableSha256 is required."));
        };
        self.limit(
            &format!("game-session:{}", caller.user_id),
            10,
            5 * MINUTE,
            "Too many launch attempts. Try again in a few minutes.",
        )?;
        let store = self.store()?;
        let account = store
            .launcher_account_by_user_id(&caller.user_id)
            .await?
            .ok_or_else(|| unauthorized(MSG_SIGN_IN))?;
        let now = self.now();
        policy::assert_account_eligible(&account, &util::iso(now))?;
        policy::assert_not_deactivated(&account)?;
        policy::assert_consent_if_enforced(&account, self.consent_enforced())?;
        let realm = field(body, "realm").filter(|r| !r.is_null());
        if realm.is_none() && !self.official_play() {
            return Err(LauncherError::new(
                "OFFICIAL_PLAY_UNAVAILABLE",
                policy::MSG_OFFICIAL_UNAVAILABLE,
            ));
        }
        runtime::assert_channel(channel)?;
        let manifest = runtime::load_manifest(self.raw("LAUNCHER_UPDATE_ROOT"), channel).await?;
        let target = self.config.target_changelist;
        let approved = runtime::assert_artifact_set(
            &manifest,
            target,
            channel,
            field(body, "runtimeManifestVersion"),
            field(body, "runtimeArtifacts"),
        )?;
        let unsupported = |message: &str| LauncherError::new("GAME_BUILD_UNSUPPORTED", message);
        if changelist != f64::from(target) {
            return Err(unsupported(
                "This Dauntless build isn't supported. Verify or repair your installation.",
            ));
        }
        let executable = service_kit::js::trim(executable).to_lowercase();
        let approved_executables: Vec<String> = self
            .raw("APPROVED_EXECUTABLE_SHA256")
            .unwrap_or("")
            .split(',')
            .map(util::normalize_lower)
            .filter(|h| util::is_hex64(h))
            .collect();
        if !util::is_hex64(&executable) || !approved_executables.contains(&executable) {
            tracing::warn!("unrecognised executable for {} (target {target})", caller.user_id);
            return Err(unsupported(
                "This Dauntless installation isn't recognized. Verify or repair your installation.",
            ));
        }
        let report = launcher_build::parse_launcher_report(field(body, "launcher"));
        let verdict = self.seams.activity.classify_launcher_build(report.as_ref()).await;
        let launcher_policy = LauncherPolicy::from_settings(self.settings.as_ref());
        for warning in &launcher_policy.warnings {
            tracing::warn!("{warning}");
        }
        if launcher_policy.refuses(verdict, report.as_ref(), &account.roles) {
            tracing::warn!("refused an {} launcher for {}", verdict.as_str(), caller.user_id);
            return Err(LauncherError::new(
                "LAUNCHER_UNOFFICIAL",
                "This launcher isn't an official Rustic Isles build. Install the official launcher to play on Rustic Isles.",
            ));
        }
        let realm_text = realm.map(service_kit::jsjson::stringify);
        let realm_id = match self
            .seams
            .realms
            .authorize_game_session(&account, realm_text.as_deref())
            .await
        {
            Ok(realm_id) => realm_id,
            Err(RealmError::Refused { code, message }) => return Err(LauncherError::new(&code, message)),
            Err(RealmError::Failed(error)) => {
                tracing::error!("realm authorization failed: {error}");
                return Err(LauncherError::new(
                    "NOT_IMPLEMENTED",
                    "Something went wrong. Please try again.",
                ));
            }
        };
        let code = util::random_hex::<32>();
        let exchange = ExchangeCode {
            code_hash: util::sha256_hex(&code),
            account_id: account.account_id.clone(),
            launcher_session_id: caller.sid.clone(),
            build_changelist: i64::from(target),
            executable_sha256: executable,
            runtime_channel: channel,
            runtime_sha256: approved.runtime_sha256,
            runtime_manifest_version: approved.manifest_version,
            runtime_artifact_set_sha256: approved.artifact_set_sha256,
            realm_id,
            created_at: util::iso(now),
            expires_at: util::iso(now + EXCHANGE_TTL_MS),
            consumed_at: None,
        };
        let actor = self.player_actor(&account.account_id)?;
        store.create_exchange_code(&actor, &exchange).await?;
        self.seams.activity.touch_game_session(&caller.user_id, now).await;
        self.seams
            .activity
            .touch_matchmaking_presence(&caller.user_id, now)
            .await;
        self.seams
            .activity
            .record_launcher_build(&caller.user_id, report.as_ref(), verdict)
            .await;
        Ok(ok(json!({"exchangeCode": code, "expiresInSeconds": 60})))
    }

    /// Accounts the unverified sweep may remove (§8.1).
    pub(crate) fn unverified_cutoff(&self, now: i64) -> String {
        util::iso(now - UNVERIFIED_TTL_MS)
    }
}

/// The callback's 302 (§7.5.2): HTML when the client prefers it, else text.
fn redirect(url: &str, accept: Option<&str>) -> Response {
    let accept = accept.unwrap_or("");
    let html_at = accept.find("text/html");
    let plain_at = accept.find("text/plain").or_else(|| accept.find("*/*"));
    let html = match (html_at, plain_at) {
        (Some(_), None) => true,
        (Some(h), Some(p)) => h < p,
        _ => false,
    };
    let (content_type, body) = if html {
        let escaped = url
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        (
            "text/html; charset=utf-8",
            format!("<p>Found. Redirecting to {escaped}</p>"),
        )
    } else {
        ("text/plain; charset=utf-8", format!("Found. Redirecting to {url}"))
    };
    let mut response = (StatusCode::FOUND, body).into_response();
    let headers = response.headers_mut();
    if let Ok(location) = HeaderValue::from_str(url) {
        headers.insert(header::LOCATION, location);
    }
    headers.insert(header::VARY, HeaderValue::from_static("Accept"));
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_pick_html_only_when_preferred() {
        let html = redirect("x://a?b=1&c=2", Some("text/html,application/xhtml+xml,*/*;q=0.8"));
        assert_eq!(html.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
        let text = redirect("x://a", Some("*/*"));
        assert_eq!(text.headers()[header::CONTENT_TYPE], "text/plain; charset=utf-8");
        let text = redirect("x://a", None);
        assert_eq!(text.headers()[header::VARY], "Accept");
        assert_eq!(text.status(), StatusCode::FOUND);
    }
}
