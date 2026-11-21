// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The game client's side (spec §3.1-§3.3, §4.1): game auth, the sign-in stubs, the Epic Online Services
//! stand-in and the hub's encryption keys.

use axum::extract::{ConnectInfo, Request};
use axum::http::{Method, StatusCode, Uri};
use axum::response::Response;
use dainty_module_api::network::SessionTicket;
use dainty_module_api::{BoxFuture, GlobalId, IdKind};
use dainty_service_kit::serve::ConnectionInfo;
use serde_json::{Map, Value, json};

use crate::seams::{RuntimeAuth, RuntimeRequest};
use crate::store::{GameAccount, StoreError};
use crate::util::{self, empty, ok, page};
use crate::{Auth, Channel};
use crate::{AuthData, AuthStore, NoPlayerPolicy, hashes, policy, runtime};

const EXPIRES_AT: &str = "2085-09-09T01:01:01.703Z";
const SERVICE_USER: &str = "__launcher_client_credentials__";

fn has_bearer_prefix(value: &str) -> bool {
    ["bearer ", "Bearer ", "BEARER "].iter().any(|p| value.starts_with(p))
}

/// What the request's route needs from game auth.
pub(crate) enum GameAuth {
    Caller(AuthData),
    Refused(Response),
}

impl<S: AuthStore> Auth<S> {
    /// The bearer-less game server account and the sentinel's account (§4.1, §4.1.1).
    fn no_player_account(&self, sentinel: bool) -> Option<String> {
        if self.config.no_player == NoPlayerPolicy::Off {
            return None;
        }
        let configured = self
            .raw("GAMESERVER_NO_PLAYER_USER_ID")
            .or_else(|| self.raw("DEV_USER_ID"));
        let account = match configured {
            Some(value) => dainty_service_kit::js::trim(value).to_owned(),
            None if sentinel => "mystpax".to_owned(),
            None => String::new(),
        };
        (!account.is_empty()).then_some(account)
    }

    /// §4.1.2: a non-string `userId` and an unknown account are eligible.
    async fn token_eligible(&self, claims: &Map<String, Value>) -> Result<bool, StoreError> {
        let Some(Value::String(user_id)) = claims.get("userId") else {
            return Ok(true);
        };
        if self.is_kicked(claims) {
            return Ok(false);
        }
        match self.store()?.launcher_account_by_user_id(user_id).await? {
            None => Ok(true),
            Some(account) => Ok(policy::is_account_eligible(&account, &self.now_iso())),
        }
    }

    /// A player's game call is the backend's sign that they are in the game: the hub's chat socket goes to the chat
    /// service, so the client's own calls and their server's calls for them are what the admin API sees.
    async fn seen(&self, claims: &Map<String, Value>) {
        if let Some(Value::String(user_id)) = claims.get("userId") {
            self.seams.activity.touch_game_session(user_id, self.now()).await;
        }
    }

    fn rewrite_sentinel(&self, request: &mut Request) {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return;
        }
        let Some(account) = self.no_player_account(true) else {
            return;
        };
        let path = request.uri().path();
        if !path.split('/').any(|s| s == "INVALID") {
            return;
        }
        let encoded = util::encode_uri_component(&account);
        let rewritten: Vec<&str> = path
            .split('/')
            .map(|s| if s == "INVALID" { encoded.as_str() } else { s })
            .collect();
        let rewritten = rewritten.join("/");
        let target = match request.uri().query() {
            Some(query) => format!("{rewritten}?{query}"),
            None => rewritten.clone(),
        };
        let mut parts = request.uri().clone().into_parts();
        if let Ok(pq) = target.parse() {
            parts.path_and_query = Some(pq);
            if let Ok(uri) = Uri::from_parts(parts) {
                tracing::info!(
                    "Gameserver INVALID sentinel resolved to '{account}' for {} {}",
                    request.method(),
                    request.uri().path()
                );
                *request.uri_mut() = uri;
            }
        }
    }

    async fn game_auth_inner(&self, request: &mut Request) -> GameAuth {
        let headers = request.headers().clone();
        let authorization = util::header(&headers, "authorization").filter(|a| !a.is_empty());
        let key = util::header(&headers, "x-rustic-isles-gameserver-apikey")
            .or_else(|| util::header(&headers, "x-rustic-isles-gameserver-key"));
        if let Some(key) = key {
            let matched = async {
                let secret = self.secret().map_err(|e| StoreError::new(e.to_string()))?;
                let candidate = hashes::gameserver_key_hash(secret, &key);
                let stored = self.store()?.gameserver_key_hashes().await?;
                Ok::<bool, StoreError>(
                    hashes::first_match(&candidate, stored.iter().map(|h| Some(h.as_str()))).is_some(),
                )
            }
            .await;
            match matched {
                Err(error) => {
                    tracing::error!("game server key check failed: {error}");
                    return GameAuth::Refused(page(StatusCode::INTERNAL_SERVER_ERROR));
                }
                Ok(false) => {
                    tracing::warn!("Invalid Gameserver API Key Auth");
                    return GameAuth::Refused(empty(StatusCode::UNAUTHORIZED));
                }
                Ok(true) => {}
            }
            self.rewrite_sentinel(request);
            let Some(authorization) = authorization else {
                return GameAuth::Caller(AuthData {
                    user_id: self.no_player_account(false),
                    is_gameserver: true,
                    claims: Map::new(),
                    runtime: None,
                });
            };
            // Node doesn't guard this one: a bad token is its 500 (spec §14 item 2)
            let Ok(claims) = self.validate_game_token(&util::drop_bytes(&authorization, 7)) else {
                return GameAuth::Refused(page(StatusCode::INTERNAL_SERVER_ERROR));
            };
            return match self.token_eligible(&claims).await {
                Ok(true) => {
                    self.seen(&claims).await;
                    GameAuth::Caller(AuthData {
                        user_id: claims.get("userId").and_then(Value::as_str).map(str::to_owned),
                        is_gameserver: true,
                        claims,
                        runtime: None,
                    })
                }
                Ok(false) => GameAuth::Refused(empty(StatusCode::FORBIDDEN)),
                Err(_) => GameAuth::Refused(page(StatusCode::INTERNAL_SERVER_ERROR)),
            };
        }

        let peer = request
            .extensions()
            .get::<ConnectInfo<ConnectionInfo>>()
            .map(|info| info.0.peer);
        let request_id = util::header(&headers, "x-rustic-isles-request-id");
        let runtime = self
            .seams
            .realms
            .authenticate_runtime(RuntimeRequest {
                path: request.uri().path(),
                headers: &headers,
                peer,
                request_id: request_id.as_deref(),
            })
            .await;
        match runtime {
            Ok(RuntimeAuth::Authenticated(caller)) => {
                return GameAuth::Caller(AuthData {
                    user_id: None,
                    is_gameserver: false,
                    claims: Map::new(),
                    runtime: Some(caller),
                });
            }
            Ok(RuntimeAuth::Refused { status, body }) => {
                let status = StatusCode::from_u16(status).unwrap_or(StatusCode::FORBIDDEN);
                return GameAuth::Refused(util::json(status, body));
            }
            Ok(RuntimeAuth::NotMine) => {}
            Err(error) => {
                tracing::error!("player host runtime check failed: {error}");
                return GameAuth::Refused(page(StatusCode::INTERNAL_SERVER_ERROR));
            }
        }

        if let Some(ticket) = util::header(&headers, "x-dainty-session-ticket") {
            let ticket = dainty_service_kit::js::trim(&ticket).to_owned();
            if !ticket.is_empty() {
                let Ok((ticket, _)) = SessionTicket::from_token(&ticket) else {
                    return GameAuth::Refused(empty(StatusCode::UNAUTHORIZED));
                };
                return match self
                    .seams
                    .home
                    .verify_session_ticket(&ticket, &self.config.game_version)
                    .await
                {
                    Ok(account) => {
                        let user_id = account.to_string();
                        let mut claims = Map::new();
                        claims.insert("userId".into(), Value::String(user_id.clone()));
                        GameAuth::Caller(AuthData {
                            user_id: Some(user_id),
                            is_gameserver: false,
                            claims,
                            runtime: None,
                        })
                    }
                    Err(_) => GameAuth::Refused(empty(StatusCode::UNAUTHORIZED)),
                };
            }
        }

        let Some(authorization) = authorization.filter(|a| has_bearer_prefix(a)) else {
            tracing::warn!("missing game bearer for {} {}", request.method(), request.uri().path());
            return GameAuth::Refused(empty(StatusCode::UNAUTHORIZED));
        };
        let Ok(claims) = self.validate_game_token(&authorization[7..]) else {
            return GameAuth::Refused(empty(StatusCode::UNAUTHORIZED));
        };
        match self.token_eligible(&claims).await {
            Ok(true) => {
                self.seen(&claims).await;
                GameAuth::Caller(AuthData {
                    user_id: claims.get("userId").and_then(Value::as_str).map(str::to_owned),
                    is_gameserver: false,
                    claims,
                    runtime: None,
                })
            }
            Ok(false) => GameAuth::Refused(empty(StatusCode::FORBIDDEN)),
            Err(_) => GameAuth::Refused(empty(StatusCode::UNAUTHORIZED)),
        }
    }

    /// §4.1: rewrites the URI for the sentinel and puts [`AuthData`] into the extensions.
    pub fn game_auth<'a>(&'a self, request: &'a mut Request) -> BoxFuture<'a, Result<AuthData, Response>> {
        Box::pin(async move {
            match self.game_auth_inner(request).await {
                GameAuth::Caller(data) => {
                    request.extensions_mut().insert(data.clone());
                    Ok(data)
                }
                GameAuth::Refused(response) => Err(response),
            }
        })
    }

    /// The username of a userId as the client sent it (§3.1); `Err` is a 500 page.
    async fn username_of(&self, value: Option<&Value>) -> Result<Value, ()> {
        let text = match value {
            None => return Err(()),
            Some(Value::String(text)) => text,
            Some(other) => return Ok(other.clone()),
        };
        let store = self.store().map_err(|_| ())?;
        if let Ok(Some(launcher)) = store.launcher_account_by_user_id(text).await
            && !launcher.display_name.is_empty()
        {
            return Ok(Value::String(launcher.display_name));
        }
        match store.game_account_by_user_id(text).await {
            Ok(Some(account)) => Ok(Value::String(account.name)),
            Ok(None) => Ok(Value::String(text.clone())),
            Err(_) => Err(()),
        }
    }

    pub(crate) async fn login(&self, data: &AuthData, body: Option<Value>) -> Response {
        let Some(body) = body else {
            return page(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let email = util::field(Some(&body), "email").and_then(Value::as_str);
        let (Some(user_id), Some(email)) = (data.user_id.as_deref(), email) else {
            return empty(StatusCode::BAD_REQUEST);
        };
        if email != user_id {
            return empty(StatusCode::BAD_REQUEST);
        }
        let account = match self.store() {
            Ok(store) => store.game_account_by_user_id(user_id).await,
            Err(error) => Err(error),
        };
        match account {
            Ok(Some(account)) => {
                self.seams.game_login.session_supplies(&account).await;
                ok(json!({"error_code": "TicketRateOk", "message": "", "state": "OPEN", "timeout": 8000, "title": ""}))
            }
            Ok(None) => empty(StatusCode::BAD_REQUEST),
            Err(_) => page(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    pub(crate) fn tags(&self, data: &AuthData) -> Response {
        let mut answer = Map::new();
        if let Some(user_id) = &data.user_id {
            answer.insert("accountId".into(), Value::String(user_id.clone()));
        }
        answer.insert("tags".into(), Value::Array(Vec::new()));
        ok(Value::Object(answer))
    }

    pub(crate) async fn account_info(&self, data: &AuthData) -> Response {
        let Some(user_id) = &data.user_id else {
            return page(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let Ok(name) = self.username_of(Some(&Value::String(user_id.clone()))).await else {
            return page(StatusCode::INTERNAL_SERVER_ERROR);
        };
        ok(util::object(vec![
            ("accountId", Value::String(user_id.clone())),
            ("creationDate", json!("2000-01-01 00:00:00")),
            ("email", Value::Null),
            ("preferredLanguage", Value::Null),
            ("username", name),
            ("verified", Value::Bool(true)),
        ]))
    }

    pub(crate) fn game_session(&self, request: &Request) -> Response {
        let headers = request.headers();
        let token = match util::header(headers, "authorization") {
            Some(authorization) => util::drop_bytes(&authorization, 7),
            None => match util::header(headers, "x-dainty-session-ticket") {
                Some(ticket) => dainty_service_kit::js::trim(&ticket).to_owned(),
                None => return page(StatusCode::INTERNAL_SERVER_ERROR),
            },
        };
        ok(json!({"code": null, "message": "OK", "payload": (self.config.session_payload)(&token)}))
    }

    pub(crate) async fn account_info_public(&self, data: &AuthData, body: Option<Value>) -> Response {
        let Some(body) = body else {
            return page(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let requested = util::field(Some(&body), "accountId").cloned();
        let dev = self.raw("DEV_USER_ID").unwrap_or("mystpax");
        let lookup = match (&requested, &data.user_id) {
            (Some(Value::String(text)), Some(caller)) if text == dev && caller != dev => {
                Some(Value::String(caller.clone()))
            }
            _ => requested.clone(),
        };
        let Ok(name) = self.username_of(lookup.as_ref()).await else {
            return page(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let mut answer = Map::new();
        if let Some(requested) = &requested {
            answer.insert("accountId".into(), requested.clone());
        }
        answer.insert("isSubscribed".into(), Value::Bool(true));
        answer.insert("language".into(), Value::Null);
        let mut linked = Map::new();
        if let Some(requested) = &requested {
            linked.insert("accountId".into(), requested.clone());
        }
        linked.insert("accountType".into(), json!("epic"));
        answer.insert("linkedAccounts".into(), Value::Array(vec![Value::Object(linked)]));
        answer.insert("username".into(), name);
        ok(Value::Object(answer))
    }

    // EOS (§3.2)

    fn dev_mode(&self) -> bool {
        self.raw("ALLOW_NO_AUTH_DEV_MODE") == Some("true") && self.raw("NODE_ENV") != Some("production")
    }

    fn token_answer(&self, user_id: &str, login: bool) -> Response {
        let Ok(token) = self.sign_game_token(user_id) else {
            return page(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let mut answer = util::object(vec![
            ("access_token", Value::String(token)),
            ("token_type", json!("bearer")),
            ("expires_at", json!(EXPIRES_AT)),
            ("features", json!(["Achievements", "AntiCheat", "Ecom", "Voice"])),
            ("organization_id", json!("o-krlzxj88qrtb69fredeuaf887bl5az")),
            ("product_id", json!("prod-jackal")),
            ("sandbox_id", json!("jackal")),
            ("deployment_id", json!("53565ba467df4edbb6f5a3d939a8b4f2")),
            ("expires_in", json!(86400)),
        ]);
        if login && let Value::Object(map) = &mut answer {
            map.insert("refresh_token".into(), json!("refresh.token.lol"));
            map.insert("refresh_expires_at".into(), json!(EXPIRES_AT));
            map.insert("account_id".into(), Value::String(user_id.to_owned()));
        }
        ok(answer)
    }

    async fn ensure_dev_user(&self, user_id: &str) -> Result<(), StoreError> {
        let account = GameAccount {
            id: GlobalId::new(IdKind::Account, &self.config.node.node).to_string(),
            user_id: user_id.to_owned(),
            name: self.raw("DEV_USER_NAME").unwrap_or("Dev Slayer").to_owned(),
            notes: 0,
        };
        self.store()?
            .ensure_game_account(&self.system_actor(), &account)
            .await
            .map(|_| ())
    }

    /// The EOS bearer: two parts split on single spaces, the first `bearer` in any case.
    fn eos_bearer(request: &Request) -> Option<String> {
        let authorization = util::header(request.headers(), "authorization")?;
        let parts: Vec<&str> = authorization.split(' ').collect();
        match parts.as_slice() {
            [scheme, token] if scheme.eq_ignore_ascii_case("bearer") => Some((*token).to_owned()),
            _ => None,
        }
    }

    pub(crate) fn bearer_user(&self, request: &Request) -> Option<String> {
        let token = Self::eos_bearer(request)?;
        match self.validate_game_token(&token) {
            Ok(claims) => claims
                .get("userId")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
                .map(str::to_owned),
            Err(_) => {
                tracing::warn!("invalid EOS bearer token");
                None
            }
        }
    }

    pub(crate) async fn oauth_token(&self, request: Request) -> Response {
        let bearer_user = self.bearer_user(&request);
        let body = match util::read_body(request).await {
            Ok(body) => body,
            Err(response) => return response,
        };
        let mode = self.raw("AUTH_MODE").unwrap_or("");
        let field = |name: &str| util::field(body.as_ref(), name);
        let client_credentials = field("grant_type").and_then(Value::as_str) == Some("client_credentials");
        match mode {
            "NONE" if self.dev_mode() => {
                if body.is_none() {
                    return page(StatusCode::INTERNAL_SERVER_ERROR);
                }
                let user_id = util::non_empty_str(field("exchange_code"))
                    .filter(|c| *c != "INVALID")
                    .map(str::to_owned)
                    .or(bearer_user)
                    .unwrap_or_else(|| self.raw("DEV_USER_ID").unwrap_or("dev-user").to_owned());
                if self.ensure_dev_user(&user_id).await.is_err() {
                    return page(StatusCode::INTERNAL_SERVER_ERROR);
                }
                self.token_answer(&user_id, true)
            }
            "APIKEY" => {
                if body.is_none() {
                    return page(StatusCode::INTERNAL_SERVER_ERROR);
                }
                let key = field("exchange_code").and_then(Value::as_str).unwrap_or("");
                if key.is_empty() && client_credentials {
                    return self.token_answer(SERVICE_USER, false);
                }
                if !(1..=256).contains(&util::utf16_len(key)) {
                    return empty(StatusCode::BAD_REQUEST);
                }
                if key.starts_with(crate::game_tickets::PREFIX) {
                    return self.ticket_login(key).await;
                }
                let user_id = match self.user_key_owner(key).await {
                    Ok(owner) => owner,
                    Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR),
                };
                let Some((user_id, account_id)) = user_id else {
                    return empty(StatusCode::BAD_REQUEST);
                };
                let store = match self.store() {
                    Ok(store) => store,
                    Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR),
                };
                match store.launcher_account_by_user_id(&user_id).await {
                    Ok(Some(account)) if !policy::is_account_eligible(&account, &self.now_iso()) => {
                        return empty(StatusCode::FORBIDDEN);
                    }
                    Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR),
                    _ => {}
                }
                if let Ok(Some(game)) = store.game_account_by_id(&account_id).await
                    && let Ok(global) = game.id.parse::<GlobalId>()
                {
                    let digest = util::sha256_hex(key);
                    let _ = self.seams.home.register_account_key_hash(&digest, &global).await;
                    let _ = self.seams.home.register_directory_account(&global, &game.name).await;
                }
                self.token_answer(&user_id, true)
            }
            "LAUNCHER" => {
                if body.is_none() {
                    return page(StatusCode::INTERNAL_SERVER_ERROR);
                }
                match util::non_empty_str(field("exchange_code")) {
                    Some(code) => self.launcher_exchange(code).await,
                    None if client_credentials => self.token_answer(SERVICE_USER, false),
                    None => empty(StatusCode::BAD_REQUEST),
                }
            }
            _ => {
                tracing::error!("AUTH_MODE is not one this server runs");
                empty(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    /// `APIKEY` mode with a game ticket for the key: the launcher's one-time login (`game_tickets`).
    async fn ticket_login(&self, ticket: &str) -> Response {
        let Some(holder) = self.tickets.redeem(ticket, self.now()) else {
            return empty(StatusCode::BAD_REQUEST);
        };
        let store = match self.store() {
            Ok(store) => store,
            Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR),
        };
        // Eligibility again: a ban between minting and redeeming still holds
        match store.launcher_account_by_user_id(&holder.user_id).await {
            Ok(Some(account)) if !policy::is_account_eligible(&account, &self.now_iso()) => {
                return empty(StatusCode::FORBIDDEN);
            }
            Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR),
            _ => {}
        }
        let user_id = holder.user_id.clone();
        let next = self.tickets.successor(holder, self.now());
        let mut answer = self.token_answer(&user_id, true);
        if answer.status() == StatusCode::OK
            && let Ok(value) = axum::http::HeaderValue::from_str(&next)
        {
            answer.headers_mut().insert(crate::game_tickets::REFRESH_HEADER, value);
        }
        answer
    }

    /// The `(userId, account id)` a player account key names.
    pub(crate) async fn user_key_owner(&self, key: &str) -> Result<Option<(String, String)>, StoreError> {
        let secret = self.secret().map_err(|e| StoreError::new(e.to_string()))?;
        let candidate = hashes::user_key_hash(secret, key);
        let store = self.store()?;
        let records = store.user_key_hashes().await?;
        let Some(index) = hashes::first_match(&candidate, records.iter().map(|(h, _)| Some(h.as_str()))) else {
            return Ok(None);
        };
        let account_id = &records[index].1;
        Ok(store
            .game_account_by_id(account_id)
            .await?
            .map(|account| (account.user_id, account_id.clone())))
    }

    async fn launcher_exchange(&self, code: &str) -> Response {
        let consumed = match self.store() {
            Ok(store) => {
                store
                    .consume_exchange_code(&self.system_actor(), &util::sha256_hex(code), &self.now_iso())
                    .await
            }
            Err(error) => Err(error),
        };
        let exchange = match consumed {
            Ok(Some(exchange)) => exchange,
            Ok(None) if self.dev_mode() => {
                if self.ensure_dev_user(code).await.is_err() {
                    return page(StatusCode::INTERNAL_SERVER_ERROR);
                }
                return self.token_answer(code, true);
            }
            Ok(None) => return empty(StatusCode::UNAUTHORIZED),
            Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR),
        };
        let account = match self.store() {
            Ok(store) => store.launcher_account_by_id(&exchange.account_id).await,
            Err(error) => Err(error),
        };
        let account = match account {
            Ok(account) => account,
            Err(_) => return page(StatusCode::INTERNAL_SERVER_ERROR),
        };
        if exchange.build_changelist != i64::from(self.config.target_changelist) {
            return empty(StatusCode::CONFLICT);
        }
        let Some(account) = account.filter(|a| policy::is_account_eligible(a, &self.now_iso())) else {
            return empty(StatusCode::FORBIDDEN);
        };
        if runtime::assert_channel(exchange.runtime_channel).is_err() || !self.runtime_still_published(&exchange).await
        {
            return empty(StatusCode::FORBIDDEN);
        }
        if self
            .seams
            .realms
            .bind_realm_presence(&account, exchange.realm_id.as_deref())
            .await
            .is_err()
        {
            return empty(StatusCode::SERVICE_UNAVAILABLE);
        }
        self.token_answer(&account.user_id, true)
    }

    async fn runtime_still_published(&self, exchange: &crate::ExchangeCode) -> bool {
        let channel: Channel = exchange.runtime_channel;
        let Ok(manifest) = runtime::load_manifest(self.raw("LAUNCHER_UPDATE_ROOT"), channel).await else {
            return false;
        };
        let reported = Value::String(exchange.runtime_sha256.clone());
        runtime::assert_main_hash(&manifest, self.config.target_changelist, channel, Some(&reported)).is_ok()
    }

    pub(crate) fn oauth_verify(&self, request: &Request) -> Response {
        let Some(token) = Self::eos_bearer(request) else {
            return empty(StatusCode::UNAUTHORIZED);
        };
        let Ok(claims) = self.validate_game_token(&token) else {
            return empty(StatusCode::UNAUTHORIZED);
        };
        let mut answer = util::object(vec![
            ("active", Value::Bool(true)),
            ("scope", json!("basic_profile friends_list presence")),
            ("token_type", json!("bearer")),
            ("expires_in", json!(86400)),
            ("expires_at", json!(EXPIRES_AT)),
        ]);
        if let Value::Object(map) = &mut answer {
            if let Some(user) = claims.get("userId") {
                map.insert("account_id".into(), user.clone());
            }
            map.insert("client_id".into(), json!("xyza7891lhxMVYGCON7LgnKZZ8HQGD5H"));
            map.insert("application_id".into(), json!("fghi4567O03HROxEjwbn7kgXpBhnhWwv"));
        }
        ok(answer)
    }

    /// `{"id","displayName","externalAuths":{}}`, or `None` when neither account exists.
    async fn public_account(&self, id: &str, name_override: Option<&str>) -> Option<Value> {
        let store = self.store().ok()?;
        let game = store.game_account_by_user_id(id).await.ok().flatten();
        let launcher = store.launcher_account_by_user_id(id).await.ok().flatten();
        if game.is_none() && launcher.is_none() {
            return None;
        }
        let name = name_override
            .map(str::to_owned)
            .or_else(|| launcher.map(|l| l.display_name).filter(|n| !n.is_empty()))
            .or_else(|| game.map(|g| g.name))
            .unwrap_or_else(|| id.to_owned());
        Some(json!({"id": id, "displayName": name, "externalAuths": {}}))
    }

    /// The caller's launcher display name, when `id` is the dev alias and the caller is someone else.
    async fn dev_override(&self, id: &str, caller: Option<&str>) -> Option<String> {
        let dev = self.raw("DEV_USER_ID").unwrap_or("dev-user");
        let caller = caller.filter(|c| id == dev && *c != dev)?;
        let launcher = self
            .store()
            .ok()?
            .launcher_account_by_user_id(caller)
            .await
            .ok()
            .flatten()?;
        Some(launcher.display_name)
    }

    pub(crate) async fn public_account_route(&self, caller: Option<String>, id: &str) -> Response {
        let name = self.dev_override(id, caller.as_deref()).await;
        match self.public_account(id, name.as_deref()).await {
            Some(account) => ok(account),
            None => empty(StatusCode::NOT_FOUND),
        }
    }

    pub(crate) async fn external_auths(&self, id: &str) -> Response {
        let found = match self.store() {
            Ok(store) => store.game_account_by_user_id(id).await,
            Err(error) => Err(error),
        };
        match found {
            Ok(Some(_)) => ok(json!([])),
            Ok(None) => empty(StatusCode::NOT_FOUND),
            Err(_) => page(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    pub(crate) async fn by_display_name(&self, name: &str) -> Response {
        let normalized = util::normalize_lower(name);
        let found = match self.store() {
            Ok(store) => store.launcher_account_by_name(&normalized).await,
            Err(error) => Err(error),
        };
        match found {
            Ok(Some(account)) => {
                ok(json!({"id": account.user_id, "displayName": account.display_name, "externalAuths": {}}))
            }
            Ok(None) => empty(StatusCode::NOT_FOUND),
            Err(_) => page(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    pub(crate) async fn bulk_accounts(&self, data: &AuthData, query: Option<&str>) -> Response {
        let mut ids: Vec<String> = util::query_pairs(query)
            .into_iter()
            .filter(|(k, v)| k == "accountId" && !v.is_empty())
            .map(|(_, v)| v)
            .collect();
        if ids.is_empty() {
            ids.push(data.user_id.clone().unwrap_or_default());
        }
        let mut out = Vec::new();
        for id in ids {
            let name = self.dev_override(&id, data.user_id.as_deref()).await;
            if let Some(account) = self.public_account(&id, name.as_deref()).await {
                out.push(account);
            }
        }
        ok(Value::Array(out))
    }

    // Encryption keys (§3.3)

    pub(crate) async fn key_generate(&self, data: &AuthData, request: Request) -> Response {
        let token = util::header(request.headers(), "authorization")
            .map(|a| a.strip_prefix("bearer ").map(str::to_owned).unwrap_or(a))
            .unwrap_or_default();
        let body = match util::read_body(request).await {
            Ok(body) => body,
            Err(response) => return response,
        };
        let candidate = util::field(body.as_ref(), "candidateId");
        let candidate = if dainty_service_kit::js::truthy(candidate) {
            dainty_service_kit::js::string(candidate)
        } else {
            String::new()
        };
        let account = data.user_id.as_deref().filter(|u| !u.is_empty());
        let issued = self.keys.issue(&candidate, &token, account, self.now());
        ok(json!({"key": issued.key, "nonce": issued.nonce, "token": issued.token}))
    }

    pub(crate) async fn key_consume(&self, data: &AuthData, request: Request) -> Response {
        let pairs = util::query_pairs(request.uri().query());
        let body = match util::read_body(request).await {
            Ok(body) => body,
            Err(response) => return response,
        };
        let read = |name: &str| match util::field(body.as_ref(), name) {
            Some(value) if !value.is_null() => dainty_service_kit::js::string(Some(value)),
            _ => match util::query_value(&pairs, name) {
                util::QueryValue::One(v) => v,
                _ => String::new(),
            },
        };
        let (candidate, token) = (read("candidateId"), read("token"));
        let roster = data.runtime.as_ref().map(|r| r.roster().to_vec());
        let found = self.keys.lookup(&candidate, &token, roster.as_deref(), self.now());
        let fingerprint = crate::enckeys::fingerprint(&token);
        if found.roster_rejected {
            tracing::warn!("encryption key for token {fingerprint} is outside the host's roster");
        }
        if found.unknown_token {
            tracing::warn!("encryption key for unknown token {fingerprint} served from the most recent");
        }
        match found.key {
            Some(key) => ok(json!({"key": key.key, "nonce": key.nonce, "token": key.token})),
            None => util::json(StatusCode::NOT_FOUND, json!({"error": "no issued key"})),
        }
    }
}
