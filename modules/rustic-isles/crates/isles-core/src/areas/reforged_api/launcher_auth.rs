//! `middleware/HasLauncherAuth.ts` as this area needs it, and `SendLauncherError`.
//!
//! The launcher's access token (RS256, `security/launcherTokens.ts`), the refresh-session check and
//! `AssertAccountAdmitted` belong to the auth area. This area reaches them through [`LauncherAuthenticator`], which the
//! auth area (or the integration pass) registers once; until one is registered every call is Node's
//! "Sign in required" 401, which is also what a module without auth answers.

use std::sync::{Arc, PoisonError, RwLock};

use axum::response::Response;
use module_api::BoxFuture;
use serde_json::json;

use super::util;
use crate::http::Shared;
use crate::ids::AccountId;
use crate::repo::auth::LauncherAccountRecord;

/// What `HasLauncherAuth` leaves on the request: `LauncherAuthData` and the account it found.
#[derive(Debug, Clone)]
pub struct LauncherCaller {
    pub account: AccountId,
    /// The client's `userId`
    pub user_id: String,
    /// The refresh session's family
    pub session: String,
    pub record: LauncherAccountRecord,
}

/// A `LauncherApiError`, already resolved to its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherFailure {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl LauncherFailure {
    pub fn new(status: u16, code: &str, message: &str) -> Self {
        Self {
            status,
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }

    pub fn unauthorized(message: &str) -> Self {
        Self::new(401, "AUTH_UNAUTHORIZED", message)
    }

    /// `SendLauncherError`: `{error: {code, message, requestId}}` with a request id of its own.
    pub fn response(&self) -> Response {
        util::json(
            self.status,
            &json!({
                "error": { "code": self.code, "message": self.message, "requestId": uuid::Uuid::new_v4().to_string() }
            }),
        )
    }
}

/// Validates a launcher bearer token the way `HasLauncherAuth` does: the signature, the refresh session still being
/// active, the account existing and admitted.
pub trait LauncherAuthenticator: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        shared: &'a Shared,
        token: &'a str,
    ) -> BoxFuture<'a, Result<LauncherCaller, LauncherFailure>>;
}

static AUTHENTICATOR: RwLock<Option<Arc<dyn LauncherAuthenticator>>> = RwLock::new(None);

/// Registers the launcher authentication (the auth area at start).
pub fn set_authenticator(authenticator: Option<Arc<dyn LauncherAuthenticator>>) {
    *AUTHENTICATOR.write().unwrap_or_else(PoisonError::into_inner) = authenticator;
}

/// `HasLauncherAuth`: the `Authorization: Bearer` header, then the registered validation.
pub async fn has_launcher_auth(
    shared: &Shared,
    authorization: Option<&str>,
) -> Result<LauncherCaller, LauncherFailure> {
    let Some(header) =
        authorization.filter(|h| h.starts_with("bearer ") || h.starts_with("Bearer ") || h.starts_with("BEARER "))
    else {
        return Err(LauncherFailure::unauthorized("Sign in required."));
    };
    let authenticator = AUTHENTICATOR.read().unwrap_or_else(PoisonError::into_inner).clone();
    match authenticator {
        Some(authenticator) => authenticator.authenticate(shared, &header[7..]).await,
        None => Err(LauncherFailure::unauthorized(
            "Your session has expired. Please sign in again.",
        )),
    }
}
