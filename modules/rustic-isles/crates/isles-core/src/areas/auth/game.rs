//! The game client's sign-in: `routes/login.ts`, `routes/eos.ts` (the Epic Online Services stand-in) and the network
//! encryption keys of `routes/matchmaking.ts` (`/key/generate`, `/key/consume`). Handlers return the `Response` Node
//! sends; an exception Node would not catch (an undefined `req.body`) is Express's 500 page.

use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use service_kit::json::Json;

use super::apikeys::user_id_for_api_key;
use super::crypto::{hash_opaque_token, random_base64url, random_hex};
use super::data::{account_by_user, launcher_by_user, store, system_actor};
use super::errors::LauncherApiError;
use super::launcher::seam_error;
use super::middleware::{AuthData, bearer_token, empty, game_auth, header, server_error};
use super::policy::is_account_eligible;
use super::runtime::{IssuedEncryptionKey, encryption_key_token_fingerprint};
use super::runtime_authorization::{
    assert_runtime_channel_matches_account, assert_runtime_hash_approved, runtime_update_root,
};
use super::tokens::{sign_game_token, validate_game_token};
use super::util::{field, now_ms, read_body};
use crate::http::{Call, Shared};
use crate::repo::auth::{AccountRecord, RuntimeChannel};
use crate::repo::{AccountId, UserId};

fn json_reply(value: Value) -> Response {
    Json(value).into_response()
}

fn status_json(status: u16, value: Value) -> Response {
    (
        axum::http::StatusCode::from_u16(status).unwrap_or(axum::http::StatusCode::OK),
        Json(value),
    )
        .into_response()
}

/// The username lookup threw: an undefined id, or a store that failed. Express answers it with a 500.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoUsername;

/// `GetUsernameForUserId`: launcher accounts are the authoritative source for player-facing names; the game account's
/// name is the fallback, then the id itself. `None` is JavaScript's `undefined` (an id the client did not send).
pub async fn username_for_user_id(shared: &Shared, user_id: Option<&Value>) -> Result<Value, NoUsername> {
    // An undefined id makes the account lookup throw, which Express answers with a 500
    let Some(user_id) = user_id else { return Err(NoUsername) };
    let Some(text) = user_id.as_str() else {
        return Ok(user_id.clone());
    };
    // An unavailable optional name source must not turn the whole game login into a 500
    if let Ok(Some(launcher)) = launcher_by_user(shared, text).await
        && !launcher.display_name.is_empty()
    {
        return Ok(json!(launcher.display_name));
    }
    match account_by_user(shared, text).await {
        Ok(Some(account)) => Ok(json!(account.name)),
        Ok(None) => Ok(json!(text)),
        Err(error) => {
            tracing::error!("account lookup: {error}");
            Err(NoUsername)
        }
    }
}

// ------------------------------------------------------------------ routes/login.ts

pub async fn features_platform_win(_call: Call) -> Response {
    tracing::info!("Features");
    json_reply(json!({"code": null, "message": "OK", "payload": {"crossplay": true, "crossprogression": true}}))
}

pub async fn account_link_epic(_call: Call) -> Response {
    tracing::info!("Account Linking");
    json_reply(json!({"code": null, "message": "OK", "payload": {"isLinked": true}}))
}

pub async fn login(mut call: Call) -> Response {
    let auth = match game_auth(&mut call).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let shared = call.shared.clone();
    let body = match read_body(&mut call.request).await {
        Ok(Some(body)) => body,
        // `req.body.email` on an undefined body throws
        Ok(None) => return server_error(),
        Err(response) => return response,
    };
    let email = field(Some(&body), "email").and_then(Value::as_str);
    if auth.user_id.is_none() || auth.user_id.as_deref() != email {
        tracing::error!(
            "UserID from ParadoxBackend Auth {:?} didn't match UserID from token",
            auth.user_id
        );
        return empty(400);
    }
    let user_id = auth.user_id.unwrap_or_default();
    let record = match account_by_user(&shared, &user_id).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            tracing::error!("UserID from ParadoxBackend Auth {user_id} had no database entry!");
            return empty(400);
        }
        Err(error) => {
            tracing::error!("login: {error}");
            return server_error();
        }
    };
    tracing::info!("{user_id} is logging in!");
    // Supplies the host refills at every session (REFORGED_SESSION_SUPPLIES_FILE)
    let game_data = shared.auth.game_data.clone();
    crate::areas::progression::supplies::refill_session_supplies(
        &shared,
        shared.auth.starter.as_ref(),
        &record.id,
        &user_id,
        &|catalog_id| game_data.is_instanced(&shared, catalog_id),
    )
    .await;
    json_reply(json!({"error_code": "TicketRateOk", "message": "", "state": "OPEN", "timeout": 8000, "title": ""}))
}

pub async fn accountinfo(mut call: Call) -> Response {
    let auth = match game_auth(&mut call).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    tracing::info!("Account info");
    let user = auth.user_id.map(Value::String);
    let Ok(username) = username_for_user_id(&call.shared, user.as_ref()).await else {
        return server_error();
    };
    let mut body = Map::new();
    if let Some(user) = &user {
        body.insert("accountId".into(), user.clone());
    }
    body.insert("creationDate".into(), json!("2000-01-01 00:00:00"));
    body.insert("email".into(), Value::Null);
    body.insert("preferredLanguage".into(), Value::Null);
    body.insert("username".into(), username);
    body.insert("verified".into(), json!(true));
    json_reply(Value::Object(body))
}

pub async fn tags(mut call: Call) -> Response {
    let auth = match game_auth(&mut call).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    tracing::info!("Tags");
    let mut body = Map::new();
    if let Some(user) = auth.user_id {
        body.insert("accountId".into(), json!(user));
    }
    body.insert("tags".into(), json!([]));
    json_reply(Value::Object(body))
}

/// `PUT /gamesession/epic` and `/gamesession/epiceos`: one token serves all three steps of the Epic flow, so this is an
/// echo. The payload is the version's (RU3c `game_session_payload`).
pub async fn game_session(mut call: Call) -> Response {
    if let Err(response) = game_auth(&mut call).await {
        return response;
    }
    // `AuthHeader.slice(...)` on a missing header throws
    let Some(authorization) = header(call.request.headers(), "authorization")
        .or_else(|| header(call.request.headers(), "x-reforged-session-ticket"))
    else {
        return server_error();
    };
    let token = bearer_token(&authorization);
    let payload = call.shared.version.hooks.game_session_payload(token);
    json_reply(json!({"code": null, "message": "OK", "payload": payload}))
}

pub async fn accountinfo_public(mut call: Call) -> Response {
    let auth = match game_auth(&mut call).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let shared = call.shared.clone();
    let body = match read_body(&mut call.request).await {
        Ok(Some(body)) => body,
        Ok(None) => return server_error(),
        Err(response) => return response,
    };
    let requested = body.get("accountId");
    // Phoenix still sends the retained `mystpax` self-alias in one account-info request after launcher auth has
    // established the UUID. The requested accountId is kept in the answer, but the name comes from the authenticated
    // launcher account so the local Social header has a usable name.
    let dev_user = shared.settings.raw("DEV_USER_ID").unwrap_or("mystpax");
    let lookup = match (requested.and_then(Value::as_str), auth.user_id.as_deref()) {
        (Some(requested_id), Some(authenticated)) if requested_id == dev_user && authenticated != requested_id => {
            Some(Value::String(authenticated.to_owned()))
        }
        _ => requested.cloned(),
    };
    let Ok(username) = username_for_user_id(&shared, lookup.as_ref()).await else {
        return server_error();
    };
    // We allow anybody to look up anybody's account from their account id. The LOOKED-UP accountId is echoed, not the
    // requestor's own, so every other player resolves to themselves.
    let mut linked = Map::new();
    if let Some(requested) = requested {
        linked.insert("accountId".into(), requested.clone());
    }
    linked.insert("accountType".into(), json!("epic"));
    let mut answer = Map::new();
    if let Some(requested) = requested {
        answer.insert("accountId".into(), requested.clone());
    }
    answer.insert("isSubscribed".into(), json!(true));
    answer.insert("language".into(), Value::Null);
    answer.insert("linkedAccounts".into(), json!([linked]));
    answer.insert("username".into(), username);
    json_reply(Value::Object(answer))
}

pub async fn migration_trigger(mut call: Call) -> Response {
    if let Err(response) = game_auth(&mut call).await {
        return response;
    }
    tracing::debug!("Migration trigger (stubbed)");
    json_reply(json!({"migration_failed": false, "migration_finished": true}))
}

pub async fn migration_status(mut call: Call) -> Response {
    if let Err(response) = game_auth(&mut call).await {
        return response;
    }
    tracing::debug!("Migration status (stubbed)");
    json_reply(
        json!({"code": null, "message": "OK", "payload": {"migration_failed": false, "migration_finished": true}}),
    )
}

/// The 1.12 login flow blocks (404) without this.
pub async fn isbanned(_call: Call) -> Response {
    tracing::info!("Is banned check (stubbed)");
    json_reply(json!({"isBanned": false}))
}

// ------------------------------------------------------------------ routes/eos.ts

const EXPIRES_AT: &str = "2085-09-09T01:01:01.703Z";
/// The OAuth client-credentials grant authenticates the game application, not a player: its token is distinct from any
/// launcher account.
const CLIENT_CREDENTIALS_USER_ID: &str = "__launcher_client_credentials__";

fn dev_user_id(shared: &Shared) -> String {
    shared.settings.raw("DEV_USER_ID").unwrap_or("dev-user").to_owned()
}

fn dev_user_name(shared: &Shared) -> String {
    shared.settings.raw("DEV_USER_NAME").unwrap_or("Dev Slayer").to_owned()
}

/// `ALLOW_NO_AUTH_DEV_MODE === "true"` and `NODE_ENV !== "production"`: the no-auth branches create and log in as any
/// account with no real authentication. They exist for local development only, behind two explicit flags.
fn no_auth_dev_mode(shared: &Shared) -> bool {
    shared.settings.raw("ALLOW_NO_AUTH_DEV_MODE") == Some("true")
        && shared.settings.raw("NODE_ENV") != Some("production")
}

/// `EnsureDevUser`.
async fn ensure_dev_user(shared: &Shared, user_id: &str) -> Result<(), crate::repo::RepoError> {
    let (user, name, node) = (
        UserId(user_id.to_owned()),
        dev_user_name(shared),
        shared.node.node.clone(),
    );
    store(shared)?
        .write(&system_actor(), move |tx, repos| {
            if repos.accounts.find_by_user_id(tx.conn(), &user)?.is_none() {
                repos.accounts.create(
                    tx,
                    &AccountRecord {
                        id: AccountId::new(&node),
                        user_id: user,
                        name,
                        notes: 0,
                    },
                )?;
            }
            Ok(())
        })
        .await
}

/// `GetBearerToken`: exactly two space-separated parts, the first `bearer` in any case.
fn eos_bearer(call: &Call) -> Option<String> {
    let authorization = header(call.request.headers(), "authorization")?;
    let parts: Vec<&str> = authorization.split(' ').collect();
    (parts.len() == 2 && parts[0].eq_ignore_ascii_case("bearer")).then(|| parts[1].to_owned())
}

/// `GetUserIdFromBearerToken`: the user of a valid game token, for the routes that keep a session's identity.
fn user_from_bearer(call: &Call) -> Option<String> {
    let token = eos_bearer(call)?;
    match validate_game_token(&call.shared.settings, &token) {
        Ok(claims) => claims
            .get("userId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        Err(_) => {
            tracing::warn!(
                "EOS oauth/token received an invalid bearer token while trying to preserve session identity"
            );
            None
        }
    }
}

fn token_reply(shared: &Shared, user_id: &str, with_account: bool) -> Response {
    let Ok(access_token) = sign_game_token(&shared.settings, user_id) else {
        return server_error();
    };
    let mut body = Map::new();
    body.insert("access_token".into(), json!(access_token));
    body.insert("token_type".into(), json!("bearer"));
    body.insert("expires_at".into(), json!(EXPIRES_AT));
    body.insert("features".into(), json!(["Achievements", "AntiCheat", "Ecom", "Voice"]));
    body.insert("organization_id".into(), json!("o-krlzxj88qrtb69fredeuaf887bl5az"));
    body.insert("product_id".into(), json!("prod-jackal"));
    body.insert("sandbox_id".into(), json!("jackal"));
    body.insert("deployment_id".into(), json!("53565ba467df4edbb6f5a3d939a8b4f2"));
    body.insert("expires_in".into(), json!(86400));
    if with_account {
        body.insert("refresh_token".into(), json!("refresh.token.lol"));
        body.insert("refresh_expires_at".into(), json!(EXPIRES_AT));
        body.insert("account_id".into(), json!(user_id));
    }
    json_reply(Value::Object(body))
}

/// A login token for a user that has an account id of its own: with the account's refresh fields.
fn login_reply(shared: &Shared, user_id: &str) -> Response {
    token_reply(shared, user_id, true)
}

pub async fn oauth_token(mut call: Call) -> Response {
    let shared = call.shared.clone();
    let mode = shared.settings.raw("AUTH_MODE").unwrap_or_default().to_owned();
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let exchange_code = || field(body.as_ref(), "exchange_code");
    let client_credentials =
        || field(body.as_ref(), "grant_type").and_then(Value::as_str) == Some("client_credentials");

    if mode == "NONE" && no_auth_dev_mode(&shared) {
        // `req.body.exchange_code` on an undefined body throws
        if body.is_none() {
            return server_error();
        }
        let code = exchange_code()
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty() && *c != "INVALID");
        let user_id = match code {
            Some(code) => code.to_owned(),
            None => user_from_bearer(&call).unwrap_or_else(|| dev_user_id(&shared)),
        };
        if let Err(error) = ensure_dev_user(&shared, &user_id).await {
            tracing::error!("oauth/token: {error}");
            return server_error();
        }
        tracing::info!("Logging in a user in no-auth dev mode.");
        return login_reply(&shared, &user_id);
    }

    if mode == "APIKEY" {
        if body.is_none() {
            return server_error();
        }
        let key = exchange_code().and_then(Value::as_str);
        // The client asks twice: once for the player (the account key as exchange_code) and once for the game itself
        // (client_credentials, no code). The second gets the service token rather than a 500 from hashing an empty key.
        if key.is_none_or(str::is_empty) && client_credentials() {
            tracing::info!("APIKEY auth: client-credentials token for the game app");
            return token_reply(&shared, CLIENT_CREDENTIALS_USER_ID, false);
        }
        let user_id = match key.filter(|k| !k.is_empty() && k.encode_utf16().count() <= 256) {
            Some(key) => match user_id_for_api_key(&shared, key).await {
                Ok(user) => user,
                Err(error) => {
                    tracing::error!("oauth/token: {error}");
                    return server_error();
                }
            },
            None => None,
        };
        // A key whose launcher account is banned or disabled opens no session
        if let Some(user_id) = &user_id
            && let Ok(Some(account)) = launcher_by_user(&shared, user_id).await
            && !is_account_eligible(&account)
        {
            tracing::warn!("APIKEY auth: {user_id} is {}, refused", account.status.as_str());
            return empty(403);
        }
        return match user_id {
            Some(user_id) => {
                if let Some(raw_key) = key
                    && let Ok(Some(acc)) = account_by_user(&shared, &user_id).await
                {
                    use sha2::{Digest, Sha256};
                    let key_sha256: String = Sha256::digest(raw_key.as_bytes())
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    let home = shared.home();
                    let _ = home.register_account_key_hash(&key_sha256, acc.id.global()).await;
                    let _ = home.register_directory_account(acc.id.global(), &acc.name).await;
                }
                tracing::info!("Logging in a user by API key.");
                login_reply(&shared, &user_id)
            }
            None => {
                tracing::error!("Invalid API key auth!");
                empty(400)
            }
        };
    }

    if mode == "LAUNCHER" {
        // Consumes the one-time codes POST /launcher/v1/game-sessions issues
        if body.is_none() {
            return server_error();
        }
        let Some(raw_code) = exchange_code().and_then(Value::as_str).filter(|c| !c.is_empty()) else {
            // The client makes a second OAuth request after its player exchange: client_credentials, with no exchange
            // code, for the game app and not the player. A player handoff code is neither consumed nor replayed here.
            if client_credentials() {
                tracing::info!("LAUNCHER auth: issuing client-credentials token");
                return token_reply(&shared, CLIENT_CREDENTIALS_USER_ID, false);
            }
            tracing::error!("LAUNCHER auth: missing exchange_code");
            return empty(400);
        };
        return launcher_exchange(&shared, raw_code).await;
    }

    tracing::error!(
        "No login method configured! (AUTH_MODE is unset/unrecognized, or AUTH_MODE=NONE without ALLOW_NO_AUTH_DEV_MODE=true)"
    );
    empty(500)
}

async fn launcher_exchange(shared: &Shared, raw_code: &str) -> Response {
    let hash = hash_opaque_token(raw_code);
    let now = super::util::iso_now();
    let consumed = match store(shared) {
        Ok(store) => {
            store
                .write(&system_actor(), move |tx, repos| {
                    repos.game_exchange_codes.consume_by_code_hash(tx, &hash, &now)
                })
                .await
        }
        Err(error) => Err(error),
    };
    let record = match consumed {
        Ok(record) => record,
        Err(error) => {
            tracing::error!("oauth/token: {error}");
            return server_error();
        }
    };
    let Some(record) = record else {
        // The retained dev/backup identity: when no one-time code matches AND dev mode is explicitly enabled, the value is
        // a dev userId, so the kept-for-backup account can still play alongside real launcher accounts. Launcher codes
        // stay single-use, and a real deployment (flag unset) rejects any non-code value.
        if no_auth_dev_mode(shared) {
            tracing::info!("LAUNCHER auth: no game-session code matched; dev fallback login as \"{raw_code}\"");
            if let Err(error) = ensure_dev_user(shared, raw_code).await {
                tracing::error!("oauth/token: {error}");
                return server_error();
            }
            return login_reply(shared, raw_code);
        }
        tracing::warn!("LAUNCHER auth: exchange code missing, expired, or already consumed");
        return empty(401);
    };

    let id = record.account.clone();
    let account = match store(shared) {
        Ok(store) => {
            let find = id.clone();
            store
                .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &find))
                .await
        }
        Err(error) => Err(error),
    };
    let account = match account {
        Ok(account) => account,
        Err(error) => {
            tracing::error!("oauth/token: {error}");
            return server_error();
        }
    };
    let target = shared.version.tables.target_changelist;
    let user_id = account
        .as_ref()
        .map(|a| a.user_id.as_str().to_owned())
        .unwrap_or_default();
    // Fails closed if the changelist does not match the one this module serves
    if record.build_changelist != target {
        tracing::warn!(
            "LAUNCHER auth: build mismatch for {user_id} (code build={}, target={target})",
            record.build_changelist
        );
        return empty(409);
    }
    let Some(account) = account.filter(is_account_eligible) else {
        tracing::warn!("LAUNCHER auth: account {} is not eligible", record.account);
        return empty(403);
    };

    // Re-check the channel and the exact signed-manifest runtime hash when the game redeems the one-time code. A Tester
    // revocation or runtime release change during the code's 60-second lifetime therefore invalidates the code instead
    // of allowing stale beta access.
    let checked = (|| -> Result<(), LauncherApiError> {
        assert_runtime_channel_matches_account(record.runtime_channel, &account.roles)?;
        assert_runtime_hash_approved(
            record.runtime_channel,
            Some(&Value::String(record.runtime_sha256.clone())),
            &runtime_update_root(&shared.settings),
            target,
        )?;
        Ok(())
    })();
    if let Err(error) = checked {
        tracing::warn!(
            "LAUNCHER auth: runtime authorization failed for {user_id} ({})",
            error.code
        );
        return empty(403);
    }

    // The authoritative realm bind: this login's realm (the one the code was issued for, or none) replaces whatever
    // realm context the account had, so an official Play never keeps a stale or spoofed realm. A no-op while the realms
    // module is off. Resetting to "no realm" never blocks a login; a login that meant a realm and cannot be bound is
    // refused (it must not go on as official play).
    let realm = match record.realm_id.as_deref().map(str::parse::<rifts_seam::RealmId>) {
        None => None,
        Some(Ok(realm)) => Some(realm),
        Some(Err(error)) => {
            tracing::error!("LAUNCHER auth: could not bind the realm of {user_id}: {error}");
            return empty(503);
        }
    };
    let rifts_account = rifts_seam::AccountId(user_id.clone());
    if let Err(error) = shared
        .rifts
        .entry()
        .bind_realm_presence_at_login(rifts_account, realm)
        .await
    {
        tracing::error!(
            "LAUNCHER auth: could not bind the realm of {user_id}: {}",
            seam_error(error).message
        );
        return empty(503);
    }
    tracing::info!("Logging in a user via a launcher exchange code.");
    login_reply(shared, &user_id)
}

pub async fn oauth_verify(call: Call) -> Response {
    tracing::info!("Verifying token");
    let Some(token) = eos_bearer(&call) else {
        return empty(401);
    };
    let Ok(claims) = validate_game_token(&call.shared.settings, &token) else {
        return empty(401);
    };
    // `Payload.userId` as the token carries it (a missing claim is `undefined`, which JSON leaves out)
    let mut body = Map::new();
    body.insert("active".into(), json!(true));
    body.insert("scope".into(), json!("basic_profile friends_list presence"));
    body.insert("token_type".into(), json!("bearer"));
    body.insert("expires_in".into(), json!(86400));
    body.insert("expires_at".into(), json!(EXPIRES_AT));
    if let Some(account_id) = claims.get("userId") {
        body.insert("account_id".into(), account_id.clone());
    }
    body.insert("client_id".into(), json!("xyza7891lhxMVYGCON7LgnKZZ8HQGD5H"));
    body.insert("application_id".into(), json!("fghi4567O03HROxEjwbn7kgXpBhnhWwv"));
    json_reply(Value::Object(body))
}

/// `LookupEpicPublicAccount`: a game account or a launcher account by `userId`.
async fn lookup_epic_public_account(
    shared: &Shared,
    account_id: &str,
    display_name_override: Option<&str>,
) -> Option<Value> {
    let account = account_by_user(shared, account_id).await.ok().flatten();
    let launcher = launcher_by_user(shared, account_id).await.ok().flatten();
    if account.is_none() && launcher.is_none() {
        return None;
    }
    let display_name = display_name_override
        .map(str::to_owned)
        .or_else(|| {
            launcher
                .as_ref()
                .map(|l| l.display_name.clone())
                .filter(|n| !n.is_empty())
        })
        .or_else(|| account.as_ref().map(|a| a.name.clone()))
        .unwrap_or_else(|| account_id.to_owned());
    // The Epic account is the core profile; this map is only for linked external platforms, so no "epic" entry is made up
    Some(json!({"id": account_id, "displayName": display_name, "externalAuths": {}}))
}

pub async fn public_account(call: Call) -> Response {
    let shared = call.shared.clone();
    let account_id = call.params.get("AccId").unwrap_or_default().to_owned();
    // Some Phoenix builds make this single-account request with the bearer token attached, even though the endpoint is
    // public: the legacy self alias gets the same treatment as the bulk endpoint when the token identifies a launcher
    // account
    let requester = user_from_bearer(&call);
    let dev = dev_user_id(&shared);
    let alias = account_id == dev && requester.as_deref().is_some_and(|r| r != dev);
    let override_name = match (&requester, alias) {
        (Some(requester), true) => launcher_by_user(&shared, requester)
            .await
            .ok()
            .flatten()
            .map(|l| l.display_name),
        _ => None,
    };
    match lookup_epic_public_account(&shared, &account_id, override_name.as_deref()).await {
        Some(account) => json_reply(account),
        None => empty(404),
    }
}

pub async fn public_account_external_auths(call: Call) -> Response {
    let account_id = call.params.get("AccId").unwrap_or_default().to_owned();
    match account_by_user(&call.shared, &account_id).await {
        Ok(Some(_)) => json_reply(json!([])),
        Ok(None) => empty(404),
        Err(error) => {
            tracing::error!("externalAuths: {error}");
            server_error()
        }
    }
}

/// Resolves a display name to an accountId so the client can invite a friend by name (Epic's display-name lookup).
pub async fn public_account_by_display_name(call: Call) -> Response {
    let name = service_kit::js::trim(call.params.get("displayName").unwrap_or_default()).to_owned();
    match super::data::launcher_by_display_name(&call.shared, &name.to_lowercase()).await {
        Ok(Some(account)) => {
            tracing::info!("Display-name lookup: {name} -> {}", account.user_id);
            json_reply(
                json!({"id": account.user_id.as_str(), "displayName": account.display_name, "externalAuths": {}}),
            )
        }
        Ok(None) => {
            tracing::info!("Display-name lookup miss: {name}");
            empty(404)
        }
        Err(error) => {
            tracing::error!("display-name lookup: {error}");
            server_error()
        }
    }
}

pub async fn sessions_kill(_call: Call) -> Response {
    tracing::info!("Session kill (stubbed)");
    json_reply(json!({}))
}

/// Epic's bulk public-account lookup: the query key may repeat (`?accountId=a&accountId=b`) and the answer is always an
/// array.
pub async fn public_account_bulk(mut call: Call) -> Response {
    let auth = match game_auth(&mut call).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let shared = call.shared.clone();
    let mut requested: Vec<String> = call
        .request
        .uri()
        .query()
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (super::util::decode_query(key) == "accountId").then(|| super::util::decode_query(value))
        })
        .filter(|value| !value.is_empty())
        .collect();
    // The old self-lookup for callers that omit accountId, still honouring the array contract
    if requested.is_empty() {
        requested.push(auth.user_id.clone().unwrap_or_default());
    }
    // The 1.12 client still performs one legacy self-lookup as `mystpax` after launcher authentication has established
    // the real UUID: the requested id is kept (the client uses it as its subsystem key) with the launcher's display name
    let dev = dev_user_id(&shared);
    let launcher_name = match &auth.user_id {
        Some(user) => launcher_by_user(&shared, user)
            .await
            .ok()
            .flatten()
            .map(|l| l.display_name),
        None => None,
    };
    let mut accounts = Vec::new();
    for id in &requested {
        let alias = *id == dev && auth.user_id.as_deref() != Some(dev.as_str());
        let name = if alias { launcher_name.as_deref() } else { None };
        if let Some(account) = lookup_epic_public_account(&shared, id, name).await {
            accounts.push(account);
        }
    }
    tracing::info!(
        "Bulk account info requested={} resolved={}",
        requested.len(),
        accounts.len()
    );
    json_reply(Value::Array(accounts))
}

// ------------------------------------------------------------------ routes/matchmaking.ts: the network keys

/// `POST /key/generate`: a 256-bit key and a nonce, kept under the caller's own token for the hub's `/key/consume`.
pub async fn key_generate(mut call: Call) -> Response {
    let auth = match game_auth(&mut call).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let shared = call.shared.clone();
    let key = random_hex(32);
    let nonce = random_base64url(32);
    let authorization = header(call.request.headers(), "authorization").unwrap_or_default();
    let token = authorization
        .strip_prefix("bearer ")
        .unwrap_or(&authorization)
        .to_owned();
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let candidate_id = match field(body.as_ref(), "candidateId") {
        Some(value)
            if !matches!(value, Value::Null | Value::Bool(false)) && value != &json!(0) && value != &json!("") =>
        {
            service_kit::js::string(Some(value))
        }
        _ => String::new(),
    };
    // The issuing account lets a player-host's consume be limited to its hunt roster
    let account = auth.user_id.filter(|id| !id.is_empty());
    shared
        .auth
        .encryption_keys
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .issue(
            &candidate_id,
            IssuedEncryptionKey {
                key: key.clone(),
                nonce: nonce.clone(),
                token: token.clone(),
                at: now_ms(),
                account,
            },
        );
    tracing::info!(
        "key/generate served (256-bit key + nonce; candidateId={} token={})",
        if candidate_id.is_empty() { "none" } else { &candidate_id },
        encryption_key_token_fingerprint(&token)
    );
    status_json(200, json!({"key": key, "nonce": nonce, "token": token}))
}

/// `POST /key/consume`: the hub's key fetch. Accepts the candidateId in the body or the query; falls back to the most
/// recently issued key when the hub does not name one. A player host's runtime gets only keys issued to its own hunt
/// roster, never the fallback.
pub async fn key_consume(mut call: Call) -> Response {
    let auth: AuthData = match game_auth(&mut call).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let shared = call.shared.clone();
    let query = call.request.uri().query().map(str::to_owned);
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let body_object = body.clone().unwrap_or_else(|| json!({}));
    let query_value = |name: &str| super::util::query_param(query.as_deref(), name);
    let candidate_id = match field(Some(&body_object), "candidateId") {
        Some(value) if !value.is_null() => service_kit::js::string(Some(value)),
        _ => query_value("candidateId")
            .map(|v| v.unwrap_or_default())
            .unwrap_or_default(),
    };
    let consume_token = match field(Some(&body_object), "token") {
        Some(value) if !value.is_null() => service_kit::js::string(Some(value)),
        _ => query_value("token").map(|v| v.unwrap_or_default()).unwrap_or_default(),
    };
    let token_fingerprint = encryption_key_token_fingerprint(&consume_token);
    let body_keys = body_object
        .as_object()
        .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    tracing::info!(
        "key/consume request: candidateId={} bodyKeys={body_keys} token={token_fingerprint}",
        if candidate_id.is_empty() { "none" } else { &candidate_id }
    );
    let roster = auth.runtime_roster();
    let lookup = shared
        .auth
        .encryption_keys
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .lookup(&candidate_id, &consume_token, now_ms(), roster.as_ref());
    let not_found = || status_json(404, json!({"error": "no issued key"}));
    if roster.is_some() && lookup.issued.is_none() {
        tracing::warn!(
            "key/consume: player-host runtime asked for a key outside its hunt roster or with no issued key (rosterRejected={} token={token_fingerprint}); no fallback for hosts",
            u8::from(lookup.roster_rejected)
        );
        return not_found();
    }
    let Some(issued) = lookup.issued else {
        tracing::warn!("key/consume: no issued key is available yet (hub asked before any /key/generate)");
        return not_found();
    };
    if lookup.unknown_token {
        tracing::warn!(
            "key/consume: token {token_fingerprint} matches no issued key; falling back to the most recent key (may belong to another player)"
        );
    }
    tracing::info!(
        "key/consume served the issued key (matchedBy={} candidateId={} token={token_fingerprint})",
        lookup.matched_by.as_str(),
        if candidate_id.is_empty() { "none" } else { &candidate_id }
    );
    status_json(
        200,
        json!({"key": issued.key, "nonce": issued.nonce, "token": issued.token}),
    )
}

/// `RuntimeChannel` for the callers that need to name a channel in a log line.
pub fn channel_name(channel: RuntimeChannel) -> &'static str {
    channel.as_str()
}
