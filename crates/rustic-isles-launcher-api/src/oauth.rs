// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! OAuth sign-in for the Dainty launcher (`docs/oauth.md`). `OAuthStart` opens a transaction (`state`, the PKCE
//! verifier, five minutes) and answers the authorize URL and a ticket, an HMAC of the `state` that only the launcher
//! holds: another program on the player's PC that catches the loopback redirect has the code and `state` but not the
//! ticket. `OAuthFinish` spends the `state` once, trades the code with the provider, and answers the account key of the
//! account linked to that identity, making one (under `REGISTRATION_MODE`) when none is. With the account key in
//! `x-dainty-user-api-key` it links the identity to that account instead and answers no key.

use axum::response::Response;
use http::StatusCode;
use rustic_isles::{Call, Shared};
use rustic_isles_auth::oauth::{OAuthProvider, OAuthUser};
use rustic_isles_store::repo::auth::{DiscordOAuthTransactionRecord, OperationalStatus};
use serde_json::Value;

use crate::launcher_api::{
    Holder, LinkIdentity, Refusal, RegistrationMode, create_account, holder_of_key, is_account_eligible,
    is_invite_code, make_account_key, normalize_username,
};
use crate::state::LauncherApiState;
use crate::util::{self, field, iso, json, now_ms, object, store_of, trim};

const STATE_TTL_MS: i64 = 5 * 60 * 1000;
const TICKET_LABEL: &str = "dainty-oauth-ticket\n";

/// The server's provider, when `DAINTY_OAUTH_*` name a whole one.
pub fn provider(shared: &Shared) -> Option<OAuthProvider> {
    OAuthProvider::from_settings(shared.settings(), dainty_protocol::account::OAUTH_REDIRECT_URI)
        .ok()
        .flatten()
}

/// The status's `signIn`.
pub fn status_entry(provider: &OAuthProvider) -> Value {
    object(vec![
        ("provider", Value::from(provider.id.clone())),
        ("name", Value::from(provider.name.clone())),
        ("required", Value::Bool(provider.required)),
    ])
}

fn error(status: StatusCode, code: &str) -> Response {
    json(status, &object(vec![("error", Value::from(code))]))
}

fn ticket(secret: &str, state: &str) -> String {
    hex::encode(util::hmac_sha256(
        secret.as_bytes(),
        format!("{TICKET_LABEL}{state}").as_bytes(),
    ))
}

fn secret(shared: &Shared) -> Option<&str> {
    rustic_isles_auth::hashes::read_secret(shared.settings().raw("API_KEY_HASH_SECRET")).ok()
}

/// A name the game takes from the provider's: letters, digits and `_`, cut to 16; None under 3.
pub fn name_from_provider(text: &str) -> Option<String> {
    let name: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(dainty_protocol::account::USERNAME_MAX)
        .collect();
    (name.len() >= dainty_protocol::account::USERNAME_MIN).then_some(name)
}

/// `POST /dainty/api/OAuthStart`: a sign-in transaction and the URL the launcher opens.
pub async fn start(call: Call) -> Response {
    let Some(store) = store_of(&call.shared) else {
        return util::no_store(call.route, &call.request_id);
    };
    let provider =
        match OAuthProvider::from_settings(call.shared.settings(), dainty_protocol::account::OAUTH_REDIRECT_URI) {
            Ok(Some(provider)) => provider,
            Ok(None) => return error(StatusCode::NOT_FOUND, "oauth_off"),
            Err(e) => {
                tracing::error!("[launcher-api] OAuth sign-in is set up wrong: {e}");
                return error(StatusCode::NOT_FOUND, "oauth_off");
            }
        };
    let Some(secret) = secret(&call.shared) else {
        tracing::error!("[launcher-api] OAuth start failed: API_KEY_HASH_SECRET must contain at least 32 characters.");
        return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let state = util::random_hex::<24>();
    let verifier = util::random_base64url::<32>();
    let now = now_ms();
    let record = DiscordOAuthTransactionRecord {
        state: state.clone(),
        code_verifier: verifier.clone(),
        created_at: iso(now),
        expires_at: iso(now + STATE_TTL_MS),
        consumed_at: None,
        completion_code_hash: None,
        completion_expires_at: None,
        completion_consumed_at: None,
        account: None,
        link_account: None,
    };
    let actor = dainty_module_api::ActorContext::new(dainty_module_api::Actor::System);
    if let Err(e) = store
        .write(&actor, move |tx, repos| {
            repos.discord_oauth_transactions.create(tx, &record)
        })
        .await
    {
        tracing::error!("[launcher-api] OAuth start failed: {e}");
        return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    util::ok(&object(vec![
        ("provider", Value::from(provider.id.clone())),
        ("name", Value::from(provider.name.clone())),
        ("authorizeUrl", Value::from(provider.authorize_url(&state, &verifier))),
        ("redirectUri", Value::from(provider.redirect_uri.clone())),
        ("ticket", Value::from(ticket(secret, &state))),
        ("state", Value::from(state)),
        ("expiresInSeconds", Value::from(STATE_TTL_MS / 1000)),
    ]))
}

fn signed_in(key: String, username: &str, created: bool, account: &str) -> Response {
    let body = object(vec![
        ("UUK", Value::from(key)),
        ("Username", Value::from(username)),
        ("Created", Value::Bool(created)),
    ]);
    util::with_header(util::ok(&body), "x-dainty-account-id", account)
}

/// The key holder linking, when the request carries a key: checked before the `state` is spent.
async fn linking_holder(
    shared: &Shared,
    store: &rustic_isles_store::IslesStore,
    headers: &http::HeaderMap,
) -> Result<Option<Holder>, Response> {
    let Some(key) = util::header_lossy(headers, "x-dainty-user-api-key") else {
        return Ok(None);
    };
    match holder_of_key(shared, store, &key).await {
        Ok(Some(holder)) if holder.eligible() => Ok(Some(holder)),
        Ok(Some(holder)) => {
            let banned = holder
                .launcher
                .as_ref()
                .is_some_and(|l| l.status == OperationalStatus::Banned);
            let code = if banned { "account_banned" } else { "account_disabled" };
            Err(error(StatusCode::FORBIDDEN, code))
        }
        Ok(None) => Err(error(StatusCode::UNAUTHORIZED, "unauthorized")),
        Err(e) => {
            tracing::error!("[launcher-api] OAuth link failed: {e}");
            Err(util::empty(StatusCode::INTERNAL_SERVER_ERROR))
        }
    }
}

/// `POST /dainty/api/OAuthFinish`: the account key for the identity the provider's code signs in, or, with a key,
/// that identity linked to the key's account.
pub async fn finish(call: Call) -> Response {
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
    let Some(provider) = provider(&shared) else {
        return error(StatusCode::NOT_FOUND, "oauth_off");
    };
    let linking = match linking_holder(&shared, &store, request.headers()).await {
        Ok(linking) => linking,
        Err(page) => return page,
    };
    let body = match util::read_json(&mut request).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    let text = |name: &str| field(body.as_ref(), name).and_then(Value::as_str);
    let (Some(state), Some(code), Some(presented)) = (text("State"), text("Code"), text("Ticket")) else {
        return Refusal::BadRequest.response(StatusCode::BAD_REQUEST);
    };
    let username = field(body.as_ref(), "Username").filter(|v| !v.is_null());
    let invite = field(body.as_ref(), "InviteCode").filter(|v| !v.is_null());
    if username.is_some_and(|v| !v.is_string()) || invite.is_some_and(|v| !v.is_string()) || code.len() > 2048 {
        return Refusal::BadRequest.response(StatusCode::BAD_REQUEST);
    }
    let Some(secret) = secret(&shared) else {
        tracing::error!("[launcher-api] OAuth finish failed: API_KEY_HASH_SECRET must contain at least 32 characters.");
        return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
    };
    if !util::ct_eq(ticket(secret, state).as_bytes(), presented.as_bytes()) {
        return error(StatusCode::UNAUTHORIZED, "oauth_expired");
    }
    let actor = dainty_module_api::ActorContext::new(dainty_module_api::Actor::System);
    let (owned_state, now) = (state.to_owned(), iso(now_ms()));
    let consumed = store
        .write(&actor, move |tx, repos| {
            repos
                .discord_oauth_transactions
                .consume_by_state(tx, &owned_state, &now)
        })
        .await;
    let transaction = match consumed {
        Ok(Some(t)) => t,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "oauth_expired"),
        Err(e) => {
            tracing::error!("[launcher-api] OAuth finish failed: {e}");
            return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let api = LauncherApiState::of(&shared).oauth_api();
    let user = match api.sign_in(&provider, code, &transaction.code_verifier).await {
        Ok(user) => user,
        Err(failure) => {
            tracing::warn!(
                "[launcher-api] {} sign-in failed (status {:?})",
                provider.id,
                failure.status
            );
            return error(StatusCode::BAD_GATEWAY, "oauth_failed");
        }
    };
    if !provider.admits(&user) {
        return error(StatusCode::FORBIDDEN, "oauth_not_member");
    }
    if let Some(holder) = linking {
        return link(&store, &provider, user, holder).await;
    }
    let (id, subject) = (provider.id.clone(), user.subject.clone());
    let linked = store
        .read(move |conn, repos| {
            let Some(identity) = repos.auth_identities.find_for_provider(conn, &id, &subject)? else {
                return Ok(None);
            };
            let launcher = repos.launcher_accounts.find_by_account(conn, &identity.account)?;
            Ok(Some((identity.account, launcher)))
        })
        .await;
    match linked {
        Ok(Some((account, launcher))) => sign_in_existing(&shared, &store, secret, account, launcher).await,
        Ok(None) => sign_up(&shared, &store, &provider, user, username, invite).await,
        Err(e) => {
            tracing::error!("[launcher-api] OAuth finish failed: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

enum Linked {
    Done,
    Elsewhere,
    HasOther,
}

/// Links the identity to the holder's account: idempotent for the same pair, refused when either is linked to
/// another. The key stays as it is.
async fn link(
    store: &rustic_isles_store::IslesStore,
    provider: &OAuthProvider,
    user: OAuthUser,
    holder: Holder,
) -> Response {
    let record = rustic_isles_store::repo::auth::AuthIdentityRecord {
        provider_subject: user.subject.clone(),
        account: holder.account.clone(),
        provider_username: util::take_chars(&user.username, 64),
        provider_avatar_url: provider.avatar_url(&user),
        linked_at: iso(now_ms()),
    };
    let actor = dainty_module_api::ActorContext::new(dainty_module_api::Actor::Player {
        account: holder.account.global().clone(),
    });
    let id = provider.id.clone();
    let written = store
        .write(&actor, move |tx, repos| {
            let ids = &repos.auth_identities;
            if let Some(found) = ids.find_for_provider(tx.conn(), &id, &record.provider_subject)? {
                return Ok(if found.account == record.account {
                    Linked::Done
                } else {
                    Linked::Elsewhere
                });
            }
            Ok(if ids.create_for_provider(tx, &id, &record)? {
                Linked::Done
            } else {
                Linked::HasOther
            })
        })
        .await;
    match written {
        Ok(Linked::Done) => {
            tracing::info!("[launcher-api] {} linked {}", holder.username, provider.id);
            util::ok(&object(vec![
                ("Linked", Value::Bool(true)),
                ("Username", Value::from(holder.username)),
            ]))
        }
        Ok(Linked::Elsewhere) => error(StatusCode::CONFLICT, "oauth_linked_elsewhere"),
        Ok(Linked::HasOther) => error(StatusCode::CONFLICT, "oauth_account_has_link"),
        Err(e) => {
            tracing::error!("[launcher-api] OAuth link failed: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Whether the account has an identity at the server's provider (`GetUserInfo`'s `OAuthLinked`).
pub async fn is_linked(
    store: &rustic_isles_store::IslesStore,
    provider: &str,
    account: &rustic_isles_store::AccountId,
) -> Option<bool> {
    let (provider, account) = (provider.to_owned(), account.clone());
    store
        .read(move |conn, repos| {
            repos
                .auth_identities
                .find_account_at_provider(conn, &provider, &account)
        })
        .await
        .ok()
        .map(|found| found.is_some())
}

/// A new key for the linked account; the key it had stops working (one key per account).
async fn sign_in_existing(
    shared: &Shared,
    store: &rustic_isles_store::IslesStore,
    secret: &str,
    account: rustic_isles_store::AccountId,
    launcher: Option<rustic_isles_store::repo::auth::LauncherAccountRecord>,
) -> Response {
    let Some(launcher) = launcher else {
        // Erased since it was linked
        return error(StatusCode::FORBIDDEN, "account_disabled");
    };
    if !is_account_eligible(&launcher) {
        let code = if launcher.status == OperationalStatus::Banned {
            "account_banned"
        } else {
            "account_disabled"
        };
        let reason = launcher
            .approval_reason
            .as_deref()
            .filter(|r| !r.is_empty())
            .map_or_else(|| "Ask the server's host.".to_owned(), |r| util::take_chars(r, 500));
        return json(
            StatusCode::FORBIDDEN,
            &object(vec![("error", Value::from(code)), ("reason", Value::from(reason))]),
        );
    }
    let key = make_account_key();
    let hash = rustic_isles_auth::hashes::user_key_hash(secret, &key);
    let actor = dainty_module_api::ActorContext::new(dainty_module_api::Actor::Player {
        account: account.global().clone(),
    });
    let id = account.clone();
    if let Err(e) = store
        .write(&actor, move |tx, repos| {
            repos.api_keys.insert_user_key_hash(tx, &id, &hash)
        })
        .await
    {
        tracing::error!("[launcher-api] OAuth sign-in failed: {e}");
        return util::empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let username = launcher.display_name.clone();
    tracing::info!("[launcher-api] {username} signed in with a new key");
    crate::launcher_api::announce(shared, &account, &key, &username).await;
    signed_in(key, &username, false, &account.to_string())
}

/// A new account linked to the identity, under the registration mode as `Register` is.
async fn sign_up(
    shared: &Shared,
    store: &rustic_isles_store::IslesStore,
    provider: &OAuthProvider,
    user: OAuthUser,
    username: Option<&Value>,
    invite: Option<&Value>,
) -> Response {
    let mode = RegistrationMode::from_settings(shared.settings());
    if mode == RegistrationMode::None {
        return Refusal::RegistrationClosed.response(StatusCode::BAD_REQUEST);
    }
    let chosen = match username.and_then(Value::as_str).map(trim).filter(|n| !n.is_empty()) {
        Some(_) => match normalize_username(username) {
            Some(name) => name,
            None => return Refusal::UsernameInvalid.response(StatusCode::BAD_REQUEST),
        },
        None => match name_from_provider(&user.username) {
            Some(name) => name,
            None => return error(StatusCode::BAD_REQUEST, "username_needed"),
        },
    };
    let code = invite
        .and_then(Value::as_str)
        .map(|c| trim(c).to_owned())
        .unwrap_or_default();
    if mode == RegistrationMode::InviteCode && !is_invite_code(&code) {
        return Refusal::InviteInvalid.response(StatusCode::UNAUTHORIZED);
    }
    let link = LinkIdentity {
        provider: provider.id.clone(),
        subject: user.subject.clone(),
        username: util::take_chars(&user.username, 64),
        avatar_url: provider.avatar_url(&user),
    };
    match create_account(shared, store, &chosen, mode, code, Some(link)).await {
        Ok(made) => signed_in(made.key, &chosen, true, &made.account.to_string()),
        Err(page) => page,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_the_provider() {
        assert_eq!(name_from_provider("fox.tail_9").as_deref(), Some("foxtail_9"));
        assert_eq!(
            name_from_provider("a_very_long_discord_name").as_deref(),
            Some("a_very_long_disc")
        );
        assert_eq!(name_from_provider("é.x"), None);
        assert_eq!(name_from_provider(""), None);
    }

    #[test]
    fn tickets_bind_the_state() {
        let secret = "0123456789abcdef0123456789abcdef";
        assert_eq!(ticket(secret, "a"), ticket(secret, "a"));
        assert_ne!(ticket(secret, "a"), ticket(secret, "b"));
        assert_eq!(ticket(secret, "a").len(), 64);
    }
}
