//! One request as the progression handlers see it: Express's `req` with `AuthData`, `params` and `body`.

use std::sync::Arc;

use axum::http::HeaderMap;
use axum::response::Response;
use module_api::{Actor, ActorContext};
use serde_json::Value;

use crate::http::{Call, Shared, read_body_parts};
use crate::ids::{AccountId, UserId};
use crate::repo::{RepoError, RepoResult, Repositories};
use crate::routes::Params;
use crate::{db::IslesStore, repo::AnyConnection, repo::Tx};

use super::auth::AuthData;
use super::ports::Ports;
use super::tracks::NO_PLAYER;
use super::util::{Body, empty, envelope};

pub struct Req {
    pub shared: Arc<Shared>,
    pub params: Params,
    pub auth: AuthData,
    pub body: Body,
    pub headers: HeaderMap,
    pub request_id: String,
    pub ports: Ports,
}

impl Req {
    /// The request after the middleware: `AuthData` (401 without), the parsed body, the params.
    pub async fn new(call: Call, ports: Ports) -> Result<Self, Response> {
        let Call {
            shared,
            route,
            params,
            request,
            origin_host,
            request_id,
        } = call;
        let (parts, raw_body) = request.into_parts();
        let (parts, parsed) = read_body_parts(parts, raw_body).await?;
        let (parts, auth) = match AuthData::of_extensions(&parts.extensions) {
            Some(auth) => (parts, auth),
            None => {
                let mut auth_call = Call {
                    shared: shared.clone(),
                    route,
                    params: params.clone(),
                    request: axum::extract::Request::from_parts(parts, axum::body::Body::empty()),
                    origin_host,
                    request_id: request_id.clone(),
                };
                let auth = crate::areas::auth::game_auth(&mut auth_call).await?;
                let (parts, _) = auth_call.request.into_parts();
                (parts, AuthData::from(&auth))
            }
        };
        let body = Body::from(parsed);
        Ok(Self {
            shared,
            params,
            auth,
            body,
            headers: parts.headers,
            request_id,
            ports,
        })
    }

    /// `req.params[name]`; the routes here only name params they declare.
    pub fn param(&self, name: &str) -> &str {
        self.params.get(name).unwrap_or_default()
    }

    /// `req.get(name)`: a header that is valid text.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub fn store(&self) -> Result<&IslesStore, Response> {
        self.shared.store.as_ref().ok_or_else(|| {
            tracing::error!("progression: no database is configured");
            empty(500)
        })
    }

    /// The account a Node `userId` names, if there is one.
    pub async fn account(&self, user_id: &str) -> Result<Option<AccountId>, Response> {
        let store = self.store()?;
        let user = UserId(user_id.to_owned());
        let found = store
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
            .await;
        found.map(|account| account.map(|a| a.id)).map_err(internal)
    }

    /// A read of `user_id`'s data: `f` gets `None` for a user with no account, which reads as no rows.
    pub async fn read<T, F>(&self, user_id: &str, f: F) -> Result<T, Response>
    where
        F: FnOnce(&mut AnyConnection, &Repositories, Option<&AccountId>) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let store = self.store()?;
        let user = UserId(user_id.to_owned());
        store
            .read(move |conn, repos| {
                let account = repos.accounts.find_by_user_id(conn, &user)?.map(|a| a.id);
                f(conn, repos, account.as_ref())
            })
            .await
            .map_err(internal)
    }

    /// The actor a write is logged under: the server for a game server's or a host's call, else the player.
    pub fn actor(&self, account: &AccountId) -> ActorContext {
        let actor = if self.auth.is_gameserver || self.auth.is_player_host_runtime {
            Actor::System
        } else {
            Actor::Player {
                account: account.global().clone(),
            }
        };
        ActorContext::new(actor).with_request(self.request_id.clone())
    }

    /// A write to one account's data, through the home check (DN2).
    pub async fn write<T, F>(&self, account: &AccountId, f: F) -> Result<T, Response>
    where
        F: FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let store = self.store()?;
        store
            .account_write(account, &self.actor(account), f)
            .await
            .map_err(internal)
    }

    /// `ResolveRequestUserId(req)`: the route's `userId`, else the body's `accountId`, else the token's.
    pub fn resolve_user_id(&self) -> String {
        let param = self.param("userId");
        if !param.is_empty() {
            return param.to_owned();
        }
        if let Some(Value::String(id)) = self
            .body
            .get(&["accountId"])
            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
        {
            return id.clone();
        }
        self.auth.user_id.clone().unwrap_or_else(|| NO_PLAYER.to_owned())
    }

    /// `RequireProgressionRead`: `Some(reply)` refuses.
    pub fn require_read(&self, target: &str) -> Option<Response> {
        if target == NO_PLAYER || self.auth.may_read(target) {
            return None;
        }
        tracing::warn!(
            "Rejected cross-account progression read actor={} target={target}",
            self.auth.user_id.as_deref().unwrap_or("unknown")
        );
        Some(super::util::forbidden())
    }
}

/// A database failure is Node's thrown error: the request fails with Express's 500.
pub fn internal(error: RepoError) -> Response {
    tracing::error!("progression: {error}");
    empty(500)
}

/// The game server named a player with no account here (Node would have made rows for any id).
pub fn no_account(user_id: &str) -> Response {
    tracing::warn!(
        "progression: no account for userId {}",
        super::util::log_safe(user_id, 128)
    );
    envelope(400, "Bad Request", serde_json::json!({}))
}
