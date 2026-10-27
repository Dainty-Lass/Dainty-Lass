//! `controllers/discordAuth.ts`: Discord sign-in and linking. Discord never creates an account: sign-in resolves a
//! linked Discord user to its account (AUTH_DISCORD_NOT_LINKED otherwise), and an account is linked from Settings
//! through the same OAuth, whose transaction carries the signed-in account. The Discord user is asked for `identify`
//! only, so no email comes from Discord.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::crypto::{generate_opaque_token, hash_opaque_token, random_base64url, random_hex};
use super::data::{account_write, launcher_by_user, store};
use super::discord::{DiscordConfig, DiscordUser};
use super::email::{CODE_REFUSED_MESSAGE, accept_email_code, issue_email_code, require_email_sending, take_attempt};
use super::errors::LauncherApiError;
use super::launcher::{create_session_for_user, note_sign_in, to_account_view};
use super::policy::assert_account_admitted;
use super::util::{iso_from_ms, now_ms};
use crate::http::Shared;
use crate::repo::auth::{AuthIdentityRecord, DiscordOAuthTransactionRecord, EmailCodePurpose, LauncherAccountRecord};
use crate::repo::{AccountId, UserId};

const STATE_TTL_MS: i64 = 5 * 60 * 1000;
const COMPLETION_CODE_TTL_MS: i64 = 60 * 1000;
const FIFTEEN_MINUTES_MS: i64 = 15 * 60 * 1000;
const HOUR_MS: i64 = 60 * 60 * 1000;

/// `LAUNCHER_DEEPLINK_SCHEME`.
pub fn deep_link_scheme(shared: &Shared) -> String {
    shared
        .settings
        .raw("LAUNCHER_DEEPLINK_SCHEME")
        .unwrap_or("mysticparadox")
        .to_owned()
}

/// `RequireDiscordConfig`.
pub fn require_discord_config(shared: &Shared) -> Result<DiscordConfig, LauncherApiError> {
    let get = |name: &str| shared.settings.get(name).map(str::to_owned);
    match (
        get("DISCORD_CLIENT_ID"),
        get("DISCORD_CLIENT_SECRET"),
        get("DISCORD_REDIRECT_URI"),
    ) {
        (Some(client_id), Some(client_secret), Some(redirect_uri)) => Ok(DiscordConfig {
            client_id,
            client_secret,
            redirect_uri,
        }),
        _ => Err(LauncherApiError::new(
            "AUTH_DISCORD_NOT_CONFIGURED",
            "Discord sign-in isn't set up on this server yet.",
        )),
    }
}

/// `URLSearchParams#toString` for one component: unreserved `*-._` stay, a space is `+`, everything else `%XX`.
fn form_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(byte as char),
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// `CreateDiscordTransaction`: starts one OAuth transaction. With `link` the callback links the Discord user to that
/// account; without it it signs in.
async fn create_transaction(shared: &Shared, link: Option<&AccountId>) -> Result<Value, LauncherApiError> {
    let config = require_discord_config(shared)?;
    let state = random_hex(24);
    let code_verifier = random_base64url(32);
    let challenge = {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
    };
    let now = now_ms();
    let record = DiscordOAuthTransactionRecord {
        state: state.clone(),
        code_verifier,
        created_at: iso_from_ms(now),
        expires_at: iso_from_ms(now + STATE_TTL_MS),
        consumed_at: None,
        completion_code_hash: None,
        completion_expires_at: None,
        completion_consumed_at: None,
        account: None,
        link_account: link.cloned(),
    };
    store(shared)?
        .write(&super::data::system_actor(), move |tx, repos| {
            repos.discord_oauth_transactions.create(tx, &record)
        })
        .await?;
    // identify only: Discord-created accounts have no email, and this build never asks for it
    let params = [
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", "identify"),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]
    .iter()
    .map(|(k, v)| format!("{}={}", form_component(k), form_component(v)))
    .collect::<Vec<_>>()
    .join("&");
    Ok(json!({ "authorizeUrl": format!("https://discord.com/api/oauth2/authorize?{params}") }))
}

/// `StartDiscordAuth`.
pub async fn start_discord_auth(shared: &Shared) -> Result<Value, LauncherApiError> {
    create_transaction(shared, None).await
}

/// `StartDiscordLink`: the Discord OAuth bound to the signed-in account, to link Discord in Settings. An account that
/// already has a Discord link is refused here as well as at the callback.
pub async fn start_discord_link(shared: &Shared, user_id: &str) -> Result<Value, LauncherApiError> {
    require_discord_config(shared)?;
    let account = launcher_by_user(shared, user_id)
        .await?
        .ok_or_else(|| LauncherApiError::new("NOT_FOUND", "Account not found."))?;
    let id = account.account.clone();
    if store(shared)?
        .read(move |conn, repos| repos.auth_identities.find_by_account(conn, &id))
        .await?
        .is_some()
    {
        return Err(LauncherApiError::new(
            "AUTH_DISCORD_ACCOUNT_HAS_LINK",
            "This account already has a Discord account linked.",
        ));
    }
    create_transaction(shared, Some(&account.account)).await
}

fn cancelled() -> LauncherApiError {
    LauncherApiError::new(
        "AUTH_DISCORD_CANCELLED",
        "Discord sign-in didn't complete. Please try again.",
    )
}

/// `ExchangeDiscordCode`.
async fn exchange_discord_code(shared: &Shared, code: &str, verifier: &str) -> Result<DiscordUser, LauncherApiError> {
    let config = require_discord_config(shared)?;
    shared
        .auth
        .discord
        .exchange_code(&config, code, verifier)
        .await
        .map_err(|failure| {
            tracing::warn!("Discord exchange failed: {:?}", failure.status);
            cancelled()
        })
}

/// `ResolveOrCreateAccountForDiscordUser`: the account a Discord user is linked to; it never creates one.
async fn resolve_account_for_discord_user(
    shared: &Shared,
    discord: &DiscordUser,
) -> Result<AccountId, LauncherApiError> {
    let subject = discord.id.clone();
    let identity = store(shared)?
        .read(move |conn, repos| repos.auth_identities.find_by_provider_subject(conn, &subject))
        .await?;
    identity.map(|identity| identity.account).ok_or_else(|| {
        LauncherApiError::new(
            "AUTH_DISCORD_NOT_LINKED",
            "Your Discord isn't linked to a Mystic Paradox account. Register with your email, then link Discord on the Account page.",
        )
    })
}

/// `LinkDiscordIdentity`: the callback of a link transaction. An account has at most one Discord link, and a Discord
/// user belongs to at most one account.
async fn link_discord_identity(
    shared: &Shared,
    link: &AccountId,
    discord: &DiscordUser,
) -> Result<(), LauncherApiError> {
    let id = link.clone();
    let (existing, by_account, by_subject) = {
        let (a, b, subject) = (id.clone(), id.clone(), discord.id.clone());
        store(shared)?
            .read(move |conn, repos| {
                Ok((
                    repos.launcher_accounts.find_by_account(conn, &a)?,
                    repos.auth_identities.find_by_account(conn, &b)?,
                    repos.auth_identities.find_by_provider_subject(conn, &subject)?,
                ))
            })
            .await?
    };
    if existing.is_none() {
        return Err(cancelled());
    }
    if by_account.is_some() {
        return Err(LauncherApiError::new(
            "AUTH_DISCORD_ACCOUNT_HAS_LINK",
            "This account already has a Discord account linked.",
        ));
    }
    let already_linked = || {
        LauncherApiError::new(
            "AUTH_DISCORD_ALREADY_LINKED",
            "That Discord account is linked to another Mystic Paradox account.",
        )
    };
    if by_subject.is_some() {
        return Err(already_linked());
    }
    let identity = AuthIdentityRecord {
        provider_subject: discord.id.clone(),
        account: link.clone(),
        provider_username: discord.username.clone(),
        provider_avatar_url: discord
            .avatar
            .as_ref()
            .map(|avatar| format!("https://cdn.discordapp.com/avatars/{}/{avatar}.png", discord.id)),
        linked_at: super::util::iso_now(),
    };
    let created = account_write(shared, link, move |tx, repos| {
        repos.auth_identities.create(tx, &identity)
    })
    .await?;
    if !created {
        // The unique index refuses a parallel second link
        return Err(already_linked());
    }
    tracing::info!("Discord linked to account {id}");
    Ok(())
}

/// `HandleDiscordCallback`: called after Discord redirects back with `?code&state`; the route 302-redirects the
/// BROWSER to the URL it returns. It never fails: a sign-in answers `auth/complete` or `auth/error`, and a link answers
/// `link/done` or `link/error`.
pub async fn handle_discord_callback(shared: &Shared, code: Option<&str>, state: Option<&str>) -> String {
    let scheme = deep_link_scheme(shared);
    let mut channel = "auth";
    let result = async {
        let (Some(code), Some(state)) = (code, state) else {
            return Err(LauncherApiError::new(
                "AUTH_DISCORD_CANCELLED",
                "Discord sign-in was cancelled.",
            ));
        };
        let (state_owned, now) = (state.to_owned(), super::util::iso_now());
        let transaction = store(shared)?
            .write(&super::data::system_actor(), move |tx, repos| {
                repos
                    .discord_oauth_transactions
                    .consume_by_state(tx, &state_owned, &now)
            })
            .await?
            .ok_or_else(|| LauncherApiError::new("AUTH_DISCORD_CANCELLED", "That Discord sign-in link has expired."))?;

        if let Some(link) = &transaction.link_account {
            channel = "link";
            let user = exchange_discord_code(shared, code, &transaction.code_verifier).await?;
            link_discord_identity(shared, link, &user).await?;
            return Ok(format!("{scheme}://link/done"));
        }
        let user = exchange_discord_code(shared, code, &transaction.code_verifier).await?;
        let account = resolve_account_for_discord_user(shared, &user).await?;
        let completion = generate_opaque_token();
        let (hash, expires_at, state_owned, id) = (
            hash_opaque_token(&completion),
            iso_from_ms(now_ms() + COMPLETION_CODE_TTL_MS),
            state.to_owned(),
            account.clone(),
        );
        store(shared)?
            .write(&super::data::system_actor(), move |tx, repos| {
                repos
                    .discord_oauth_transactions
                    .attach_completion_code(tx, &state_owned, &hash, &expires_at, &id)
            })
            .await?;
        Ok(format!("{scheme}://auth/complete?code={completion}"))
    }
    .await;
    match result {
        Ok(url) => url,
        Err(error) => {
            if error.code == "INTERNAL" {
                tracing::error!("Unhandled error in Discord callback");
            }
            let scheme = deep_link_scheme(shared);
            let failure: String = url_encode_component(&error.code);
            format!("{scheme}://{channel}/error?code={failure}")
        }
    }
}

fn url_encode_component(text: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const KEEP: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'!')
        .remove(b'~')
        .remove(b'*');
    utf8_percent_encode(text, KEEP).to_string()
}

/// `CompleteDiscordLogin`: trades the deep link's completion code for a launcher session.
pub async fn complete_discord_login(
    shared: &Shared,
    raw_code: Option<&Value>,
    device_id: &str,
    device_name: &str,
) -> Result<Value, LauncherApiError> {
    let Some(raw) = raw_code.and_then(Value::as_str).filter(|c| !c.is_empty()) else {
        return Err(cancelled());
    };
    let (hash, now) = (hash_opaque_token(raw), super::util::iso_now());
    let transaction = store(shared)?
        .write(&super::data::system_actor(), move |tx, repos| {
            repos
                .discord_oauth_transactions
                .consume_by_completion_code_hash(tx, &hash, &now)
        })
        .await?;
    let Some(account_id) = transaction.and_then(|t| t.account) else {
        return Err(LauncherApiError::new(
            "AUTH_DISCORD_CANCELLED",
            "That sign-in link has expired. Please try again.",
        ));
    };
    let id = account_id.clone();
    let account = store(shared)?
        .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &id))
        .await?
        .ok_or_else(cancelled)?;
    assert_account_admitted(&account)?;
    // A Discord sign-in counts for "Last sign-in"
    let signed_in = note_sign_in(shared, &account, now_ms()).await?;
    let (access, refresh) = create_session_for_user(shared, &account, device_id, device_name).await?;
    Ok(json!({
        "accessToken": access,
        "refreshToken": refresh,
        "account": to_account_view(shared, &signed_in).await?,
    }))
}

/// `RequireDiscordUnlink`: the refusals of both unlink routes, in this order: no Discord link, no password (Discord is
/// the only sign-in), no email to mail the code to, the code mail not set up on this server.
async fn require_discord_unlink(
    shared: &Shared,
    user_id: &str,
) -> Result<(LauncherAccountRecord, String), LauncherApiError> {
    let account = launcher_by_user(shared, user_id)
        .await?
        .ok_or_else(|| LauncherApiError::new("NOT_FOUND", "Account not found."))?;
    let id = account.account.clone();
    if store(shared)?
        .read(move |conn, repos| repos.auth_identities.find_by_account(conn, &id))
        .await?
        .is_none()
    {
        return Err(LauncherApiError::new(
            "AUTH_DISCORD_NOT_LINKED",
            "This account has no Discord account linked.",
        ));
    }
    if account.password_hash.is_none() {
        return Err(LauncherApiError::new(
            "AUTH_DISCORD_ONLY_LOGIN",
            "Discord is this account's only sign-in, so it can't be unlinked.",
        ));
    }
    let Some(email) = account.email.clone().filter(|e| !e.is_empty()) else {
        return Err(LauncherApiError::new(
            "AUTH_DISCORD_ONLY_LOGIN",
            "This account has no email address to send the unlink code to, so Discord can't be unlinked.",
        ));
    };
    require_email_sending(shared)?;
    Ok((account, email))
}

/// `StartDiscordUnlink`: mails the code that confirms the unlink. Answers `{sent: true, email}` also when a send limit
/// held the mail back (the code mailed before is still live). A mail that could not be sent is AUTH_EMAIL_UNAVAILABLE:
/// the player is signed in, so saying so reveals nothing, and a silent "sent" would leave them waiting.
pub async fn start_discord_unlink(shared: &Shared, user_id: &str, ip: &str) -> Result<Value, LauncherApiError> {
    take_attempt(shared, &format!("unlink-start-ip:{ip}"), 20, HOUR_MS)?;
    take_attempt(shared, &format!("unlink-start:{user_id}"), 5, HOUR_MS)?;
    let (account, email) = require_discord_unlink(shared, user_id).await?;
    if issue_email_code(shared, &account, &email, EmailCodePurpose::Unlink, ip).await?
        == super::email::EmailCodeIssue::Failed
    {
        return Err(LauncherApiError::new(
            "AUTH_EMAIL_UNAVAILABLE",
            "The code email couldn't be sent. Try again in a minute.",
        ));
    }
    Ok(json!({ "sent": true, "email": super::crypto::mask_email(&email) }))
}

/// `ConfirmDiscordUnlink`: checks the emailed code and unlinks the Discord user. A wrong, expired or used code is
/// AUTH_CODE_INVALID (400) and the link stays.
pub async fn confirm_discord_unlink(
    shared: &Shared,
    user_id: &str,
    code: Option<&Value>,
    ip: &str,
) -> Result<Value, LauncherApiError> {
    take_attempt(shared, &format!("unlink-confirm-ip:{ip}"), 30, FIFTEEN_MINUTES_MS)?;
    take_attempt(shared, &format!("unlink-confirm:{user_id}"), 10, FIFTEEN_MINUTES_MS)?;
    let (account, _) = require_discord_unlink(shared, user_id).await?;
    if !accept_email_code(
        shared,
        &account,
        EmailCodePurpose::Unlink,
        code,
        &super::util::iso_now(),
    )
    .await?
    {
        return Err(LauncherApiError::new("AUTH_CODE_INVALID", CODE_REFUSED_MESSAGE));
    }
    let id = account.account.clone();
    let gone = account_write(shared, &id.clone(), move |tx, repos| {
        repos.auth_identities.delete_for_account(tx, &id)
    })
    .await?;
    if gone > 0 {
        tracing::info!("Discord unlinked from account {user_id}");
    }
    Ok(json!({ "account": to_account_view(shared, &account).await? }))
}

/// A `UserId` for tests and callers that hold only the text.
pub fn user_id(text: &str) -> UserId {
    UserId(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_components_encode_like_url_search_params() {
        assert_eq!(form_component("a b*c-d.e_f"), "a+b*c-d.e_f");
        assert_eq!(form_component("https://x/y?z=1"), "https%3A%2F%2Fx%2Fy%3Fz%3D1");
        assert_eq!(
            url_encode_component("AUTH_DISCORD_NOT_LINKED"),
            "AUTH_DISCORD_NOT_LINKED"
        );
    }
}
