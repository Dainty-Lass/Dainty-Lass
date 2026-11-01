//! `controllers/emailCodes.ts` and `controllers/launcherEmail.ts`: the one-time codes of email verification, password
//! reset and the Discord unlink, the send limits, and the registration sweep. Answers never say whether an address has
//! an account, except the registration's existing AUTH_EMAIL_TAKEN: resend and forgot always answer `{sent: true}`, and
//! the code for an existing account is mailed after the answer has gone out, so the two cases take the same time. The
//! code itself is never stored or logged: only its HMAC, and the mail carries it to the owner of the address.

use std::sync::Arc;

use serde_json::{Value, json};

use super::crypto::{
    EMAIL_CODE_MAX_TRIES, EMAIL_CODE_TTL_MINUTES, EMAIL_CODE_TTL_MS, generate_email_code, hash_email_code,
    hashes_equal, is_email_code_format, mask_email,
};
use super::data::{account_write, launcher_by_email, player_actor, store, system_actor};
use super::errors::LauncherApiError;
use super::launcher::{
    assert_account_not_blocked, hash_password_blocking, is_valid_email_address, sign_in_after_email_proof,
    verify_password_blocking,
};
use super::mail::{MailMessage, SmtpReading, read_smtp_settings, render_code_email};
use super::util::{iso_from_ms, iso_now, now_ms};
use crate::http::Shared;
use crate::repo::auth::{EmailCodePurpose, EmailCodeRecord, LauncherAccountRecord};

pub const CODE_REFUSED_MESSAGE: &str = "That code is wrong or has expired. Check the latest email or resend the code.";

const MINUTE_MS: i64 = 60 * 1000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const FIFTEEN_MINUTES_MS: i64 = 15 * MINUTE_MS;

/// A registration not verified this long after it was made is removed, which frees its address.
pub const UNVERIFIED_REGISTRATION_TTL_MS: i64 = 48 * HOUR_MS;
pub const UNVERIFIED_SWEEP_INTERVAL_MS: u64 = 60 * 60 * 1000;

/// `TakeAttempt`: one attempt against a window; over the budget it refuses with AUTH_RATE_LIMITED.
pub fn take_attempt(shared: &Shared, key: &str, max_attempts: usize, window_ms: i64) -> Result<(), LauncherApiError> {
    if shared.auth.limiter.is_limited(key, max_attempts, window_ms) {
        return Err(LauncherApiError::new(
            "AUTH_RATE_LIMITED",
            "Too many attempts. Try again later.",
        ));
    }
    Ok(())
}

/// `IsEmailSendingConfigured`: the code mail's SMTP settings are complete.
pub fn is_email_sending_configured(shared: &Shared) -> bool {
    matches!(read_smtp_settings(&shared.settings), SmtpReading::Configured(_))
}

/// `RequireEmailSending`: AUTH_EMAIL_UNAVAILABLE (503) while the code mail is not set up on this server. Checked before
/// anything else in the routes.
pub fn require_email_sending(shared: &Shared) -> Result<(), LauncherApiError> {
    if is_email_sending_configured(shared) {
        return Ok(());
    }
    Err(LauncherApiError::new(
        "AUTH_EMAIL_UNAVAILABLE",
        "Email codes aren't available on this server right now. Try again later.",
    ))
}

/// `TakeCodeSendSlot`: a slot for the account and purpose (one code a minute, five an hour) and one for the address
/// (twenty an hour). False when a limit is reached: nothing is then stored or mailed.
pub fn take_code_send_slot(shared: &Shared, user_id: &str, purpose: EmailCodePurpose, ip: &str) -> bool {
    let limiter = &shared.auth.limiter;
    let purpose = purpose.as_str();
    if limiter.is_limited(&format!("mail-minute:{purpose}:{user_id}"), 1, MINUTE_MS) {
        return false;
    }
    if limiter.is_limited(&format!("mail-hour:{purpose}:{user_id}"), 5, HOUR_MS) {
        return false;
    }
    !limiter.is_limited(&format!("mail-ip:{ip}"), 20, HOUR_MS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailCodeIssue {
    Sent,
    Limited,
    Failed,
}

/// `IssueEmailCode`: stores a new live code for the account and purpose (replacing the code issued before) and mails
/// it. `Limited`, storing and mailing nothing, when a send limit is reached; `Failed` when the mail could not be sent
/// (logged by its error code only; the stored code is kept, so the owner can ask for another).
pub async fn issue_email_code(
    shared: &Shared,
    account: &LauncherAccountRecord,
    email: &str,
    purpose: EmailCodePurpose,
    ip: &str,
) -> Result<EmailCodeIssue, LauncherApiError> {
    let user_id = account.user_id.as_str();
    if !take_code_send_slot(shared, user_id, purpose, ip) {
        return Ok(EmailCodeIssue::Limited);
    }
    let code = generate_email_code();
    let issued_at = now_ms();
    let hash = hash_email_code(&shared.settings, user_id, purpose, &code)
        .map_err(|e| LauncherApiError::from(crate::repo::RepoError::Data(e.to_string())))?;
    let record = EmailCodeRecord {
        account: account.account.clone(),
        purpose,
        code_hash: hash,
        attempts: 0,
        created_at: iso_from_ms(issued_at),
        expires_at: iso_from_ms(issued_at + EMAIL_CODE_TTL_MS),
    };
    let id = account.account.clone();
    account_write(shared, &id, move |tx, repos| repos.email_codes.replace(tx, &record)).await?;

    let sent = async {
        let mail = render_code_email(purpose, &code, EMAIL_CODE_TTL_MINUTES)
            .map_err(|_| super::mail::MailError("ERENDER".into()))?;
        let message = MailMessage {
            to: email.to_owned(),
            subject: mail.subject,
            text: mail.text,
            html: mail.html,
        };
        shared.auth.mail.send(&shared.settings, message).await
    }
    .await;
    if let Err(error) = sent {
        tracing::warn!(
            "[Email] the {} code for user {user_id} was not mailed ({})",
            purpose.as_str(),
            super::mail::describe_mail_error(&error)
        );
        return Ok(EmailCodeIssue::Failed);
    }
    tracing::info!(
        "[Email] {} code mailed to {} (user {user_id})",
        purpose.as_str(),
        mask_email(email)
    );
    Ok(EmailCodeIssue::Sent)
}

/// `AcceptEmailCode`: accepts or refuses a submitted code. A malformed code is refused without a try. A right code
/// (compared in constant time against the live code) is used by one conditional delete, which verifies and consumes it
/// in the same write: a second use, or a parallel one, finds nothing. A wrong code, or no live code, counts one try in
/// one atomic write, so parallel guesses cannot exceed the limit; the code that runs out of tries is removed at once.
pub async fn accept_email_code(
    shared: &Shared,
    account: &LauncherAccountRecord,
    purpose: EmailCodePurpose,
    code: Option<&Value>,
    now: &str,
) -> Result<bool, LauncherApiError> {
    let Some(code) = code.and_then(Value::as_str).filter(|c| is_email_code_format(c)) else {
        return Ok(false);
    };
    let user_id = account.user_id.as_str();
    let submitted = hash_email_code(&shared.settings, user_id, purpose, code)
        .map_err(|e| LauncherApiError::from(crate::repo::RepoError::Data(e.to_string())))?;
    let id = account.account.clone();
    let (read_id, read_now) = (id.clone(), now.to_owned());
    let live = store(shared)?
        .read(move |conn, repos| {
            repos
                .email_codes
                .peek_live(conn, &read_id, purpose, &read_now, EMAIL_CODE_MAX_TRIES)
        })
        .await?;
    let actor = player_actor(&id);
    let now = now.to_owned();
    if let Some(live) = live
        && hashes_equal(&live.code_hash, &submitted)
    {
        let (consume_id, hash) = (id.clone(), live.code_hash);
        return Ok(store(shared)?
            .account_write(&id, &actor, move |tx, repos| {
                repos
                    .email_codes
                    .consume(tx, &consume_id, purpose, &hash, &now, EMAIL_CODE_MAX_TRIES)
            })
            .await?);
    }
    // A wrong code (or none live): the try is counted atomically, and the row that ran out of tries goes now
    let try_id = id.clone();
    store(shared)?
        .account_write(&id, &actor, move |tx, repos| {
            let counted = repos
                .email_codes
                .reserve_try(tx, &try_id, purpose, &now, EMAIL_CODE_MAX_TRIES)?;
            if let Some(counted) = counted
                && counted.attempts >= EMAIL_CODE_MAX_TRIES
            {
                repos.email_codes.remove(tx, &try_id, purpose, &counted.code_hash)?;
            }
            Ok(())
        })
        .await?;
    Ok(false)
}

// ------------------------------------------------------------------ the routes' controllers

fn normalize_address(email: &str) -> String {
    service_kit::js::trim(email).to_lowercase()
}

/// `VerifyEmailWithPassword`: the password AND the emailed code must both be right. A right pair verifies the email and
/// gives the sign-in answer, or the approval-pending answer while the approval is pending.
pub async fn verify_email_with_password(
    shared: &Shared,
    email: Option<&Value>,
    password: Option<&Value>,
    code: Option<&Value>,
    device_id: &str,
    device_name: &str,
    ip: &str,
) -> Result<Value, LauncherApiError> {
    require_email_sending(shared)?;
    take_attempt(shared, &format!("email-verify-ip:{ip}"), 30, FIFTEEN_MINUTES_MS)?;
    let invalid = || LauncherApiError::new("AUTH_INVALID_CREDENTIALS", "The email or password is incorrect.");
    let (Some(email), Some(password)) = (email.and_then(Value::as_str), password.and_then(Value::as_str)) else {
        return Err(invalid());
    };
    let normalized = normalize_address(email);
    // Verify takes the password, so it is guarded like the password sign-in: per address as well as per IP
    take_attempt(shared, &format!("email-verify:{normalized}"), 10, FIFTEEN_MINUTES_MS)?;

    let account = launcher_by_email(shared, &normalized).await?;
    let password_ok = verify_password_blocking(
        account.as_ref().and_then(|a| a.password_hash.clone()),
        password.to_owned(),
    )
    .await;
    let Some(account) = account.filter(|_| password_ok) else {
        return Err(invalid());
    };
    assert_account_not_blocked(&account)?;
    if !accept_email_code(shared, &account, EmailCodePurpose::Verify, code, &iso_now()).await? {
        return Err(LauncherApiError::new("AUTH_CODE_INVALID", CODE_REFUSED_MESSAGE));
    }
    let verified_at = iso_now();
    let (id, when) = (account.account.clone(), verified_at.clone());
    account_write(shared, &id.clone(), move |tx, repos| {
        repos.launcher_accounts.set_email_verified(tx, &id, &when)
    })
    .await?;
    let mut verified = account;
    verified.email_verified_at = Some(verified_at);
    verified.email_verification_pending_since = None;
    sign_in_after_email_proof(shared, &verified, device_id, device_name).await
}

/// `DeliverCodeToAccount`: mails a code to the account of the address when it may receive one: an account that exists
/// and is active, and, for a verify code, whose email is not verified yet. Nothing here reaches the caller; a failure
/// is logged by its error code only.
pub async fn deliver_code_to_account(shared: &Shared, normalized_email: &str, purpose: EmailCodePurpose, ip: &str) {
    let result = async {
        let Some(account) = launcher_by_email(shared, normalized_email).await? else {
            return Ok::<(), LauncherApiError>(());
        };
        if account.status != crate::repo::auth::OperationalStatus::Active {
            return Ok(());
        }
        if purpose == EmailCodePurpose::Verify && account.email_verified_at.is_some() {
            return Ok(());
        }
        let address = account.email.clone().unwrap_or_else(|| normalized_email.to_owned());
        issue_email_code(shared, &account, &address, purpose, ip).await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(
            "[Email] a {} code request could not be completed ({})",
            purpose.as_str(),
            error.code
        );
    }
}

fn spawn_delivery(shared: &Arc<Shared>, normalized: String, purpose: EmailCodePurpose, ip: String) {
    let shared = shared.clone();
    tokio::spawn(async move { deliver_code_to_account(&shared, &normalized, purpose, &ip).await });
}

/// `ResendEmailCode`: a new verification code, if the address has an unverified account. Always `{sent: true}`.
pub fn resend_email_code(shared: &Arc<Shared>, email: Option<&Value>, ip: &str) -> Result<Value, LauncherApiError> {
    require_email_sending(shared)?;
    take_attempt(shared, &format!("email-resend-ip:{ip}"), 20, HOUR_MS)?;
    if !is_valid_email_address(email) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Enter a valid email address.",
        ));
    }
    let normalized = normalize_address(email.and_then(Value::as_str).unwrap_or_default());
    take_attempt(shared, &format!("email-resend:{normalized}"), 5, HOUR_MS)?;
    spawn_delivery(shared, normalized, EmailCodePurpose::Verify, ip.to_owned());
    Ok(json!({ "sent": true }))
}

/// `RequestPasswordResetCode`: a password reset code, if the address has an account. Always `{sent: true}`.
pub fn request_password_reset_code(
    shared: &Arc<Shared>,
    email: Option<&Value>,
    ip: &str,
) -> Result<Value, LauncherApiError> {
    require_email_sending(shared)?;
    take_attempt(shared, &format!("password-forgot-ip:{ip}"), 20, HOUR_MS)?;
    if !is_valid_email_address(email) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Enter a valid email address.",
        ));
    }
    let normalized = normalize_address(email.and_then(Value::as_str).unwrap_or_default());
    take_attempt(shared, &format!("password-forgot:{normalized}"), 5, HOUR_MS)?;
    spawn_delivery(shared, normalized, EmailCodePurpose::Reset, ip.to_owned());
    Ok(json!({ "sent": true }))
}

/// `ResetPasswordWithCode`: the emailed reset code and a new password. A right code sets the password, verifies the
/// email (this is how the owner of an address recovers one someone else registered), ends every session of the
/// account, and gives the sign-in answer (or the approval-pending answer).
pub async fn reset_password_with_code(
    shared: &Shared,
    email: Option<&Value>,
    code: Option<&Value>,
    new_password: Option<&Value>,
    device_id: &str,
    device_name: &str,
    ip: &str,
) -> Result<Value, LauncherApiError> {
    require_email_sending(shared)?;
    take_attempt(shared, &format!("password-reset-ip:{ip}"), 30, FIFTEEN_MINUTES_MS)?;
    if !is_valid_email_address(email) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Enter a valid email address.",
        ));
    }
    let normalized = normalize_address(email.and_then(Value::as_str).unwrap_or_default());
    take_attempt(shared, &format!("password-reset:{normalized}"), 10, FIFTEEN_MINUTES_MS)?;
    // The password rules of registration, checked before a try is counted
    let Some(new_password) = new_password
        .and_then(Value::as_str)
        .filter(|p| p.encode_utf16().count() >= 8)
    else {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Password must be at least 8 characters.",
        ));
    };
    let refused = || LauncherApiError::new("AUTH_CODE_INVALID", CODE_REFUSED_MESSAGE);
    let Some(account) = launcher_by_email(shared, &normalized).await? else {
        return Err(refused());
    };
    if !accept_email_code(shared, &account, EmailCodePurpose::Reset, code, &iso_now()).await? {
        return Err(refused());
    }
    // Checked after the code: only the owner of the address, who has the code, learns the account is banned or disabled
    assert_account_not_blocked(&account)?;
    let verified_at = iso_now();
    let hash = hash_password_blocking(new_password.to_owned()).await?;
    let (id, when, now) = (account.account.clone(), verified_at.clone(), verified_at.clone());
    account_write(shared, &id.clone(), move |tx, repos| {
        repos.launcher_accounts.set_password_hash(tx, &id, &hash)?;
        repos.launcher_accounts.set_email_verified(tx, &id, &when)?;
        // Every session of the account ends: whoever held the old password loses them
        repos.refresh_sessions.revoke_all_for_account(tx, &id, &now)
    })
    .await?;
    let mut reset = account;
    reset.email_verified_at = Some(verified_at);
    reset.email_verification_pending_since = None;
    sign_in_after_email_proof(shared, &reset, device_id, device_name).await
}

/// `SweepUnverifiedRegistrations`: removes the launcher accounts registered under
/// `RUSTIC_ISLES_EMAIL_VERIFICATION=required` that were not verified within 48 h, with their codes and sessions. It
/// removes nothing else: existing accounts carry no pending mark, and an account verified meanwhile is kept by the
/// conditional delete. The rows are soft-deleted (DD23). Returns how many accounts were removed.
pub async fn sweep_unverified_registrations(shared: &Shared, now: i64, limit: u32) -> Result<usize, LauncherApiError> {
    let cutoff = iso_from_ms(now - UNVERIFIED_REGISTRATION_TTL_MS);
    let list_cutoff = cutoff.clone();
    let due = store(shared)?
        .read(move |conn, repos| {
            repos
                .launcher_accounts
                .list_unverified_registrations(conn, &list_cutoff, limit)
        })
        .await?;
    let mut removed = 0;
    for account in due {
        let (id, cutoff) = (account.account.clone(), cutoff.clone());
        let gone = store(shared)?
            .write(&system_actor(), move |tx, repos| {
                if !repos
                    .launcher_accounts
                    .delete_unverified_registration(tx, &id, &cutoff)?
                {
                    return Ok(false);
                }
                // The account is gone, so nothing can sign in as it. Its codes and sessions go too (a registration that
                // was never verified never got a session).
                repos.email_codes.delete_for_account(tx, &id)?;
                repos.refresh_sessions.delete_all_for_account(tx, &id)?;
                Ok(true)
            })
            .await?;
        if gone {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!("[EmailSweep] removed {removed} unverified registration(s) older than 48 h");
    }
    Ok(removed)
}

/// `StartUnverifiedRegistrationSweeper`: the hourly sweep, started only while verification is required (under off no
/// registration is ever pending). A slow sweep is not overlapped.
pub fn start_unverified_registration_sweeper(shared: Arc<Shared>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(UNVERIFIED_SWEEP_INTERVAL_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(error) = sweep_unverified_registrations(&shared, now_ms(), 500).await {
                tracing::warn!("[EmailSweep] sweep failed ({})", error.code);
            }
        }
    })
}
