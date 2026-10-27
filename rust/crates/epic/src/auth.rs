//! Signing in to Epic with an authorization code, the flow Legendary and Heroic use (DD22, DE2). Epic's account
//! service is behind [`Accounts`]: [`EpicAccounts`] talks to it through `egs-api` (DE1's verdict), and tests put a
//! double in its place. The caller keeps the session in its keystore, like the account key: never across a bridge,
//! never in a log.
//!
//! The secrets here have no `Debug`, `Display`, `Clone` or `Serialize`, so no `{:?}`, log line or bridge type can
//! carry one by accident. Reading one takes a call to `expose`, which a review can search for:
//!
//! ```compile_fail
//! let token = epic::auth::Token::new("secret".to_owned());
//! println!("{token:?}");
//! ```
//!
//! ```compile_fail
//! let token = epic::auth::Token::new("secret".to_owned());
//! println!("{token}");
//! ```
//!
//! `egs-api` derives `Debug` over its tokens (`EpicGames`, `UserData`), logs failed requests with their URL through
//! `log` and puts the access token in the URL that ends a session. So its values live only inside
//! [`EpicAccounts`]'s calls, its errors are mapped to [`AuthError`] without their text, its `egs_api` log target is
//! muted by the caller's log setup, and its SID sign-in (`auth_sid`, the SID in a URL) is never called.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use egs_api::EpicGames;
use egs_api::api::error::EpicAPIError;
use egs_api::api::types::account::UserData;

/// The page the sign-in webview opens (or the player, for the paste fallback). Epic answers with JSON holding the
/// `authorizationCode` once the player is signed in; the client id is Epic's own launcher's.
pub const LOGIN_URL: &str = "https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode";

/// Where [`LOGIN_URL`] ends: the JSON page with the code. A webview reads the code here and nowhere else.
pub const REDIRECT_ORIGIN: &str = "https://www.epicgames.com";
pub const REDIRECT_PATH: &str = "/id/api/redirect";

/// A secret string: an access or refresh token, or an authorization code.
pub struct Token(String);

impl Token {
    pub fn new(secret: String) -> Self {
        Self(secret)
    }

    /// The secret itself, for the one request or keystore write that needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// The code Epic's redirect page shows once the player has signed in, good for one exchange.
pub struct AuthorizationCode(Token);

impl AuthorizationCode {
    /// What the webview captured or the player pasted, trimmed. Epic's codes are hex; anything that isn't plain
    /// letters and digits is refused here rather than sent on.
    pub fn new(pasted: &str) -> Result<Self, AuthError> {
        let code = pasted.trim();
        if code.is_empty() || code.len() > 256 || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(AuthError::BadCode);
        }
        Ok(Self(Token::new(code.to_owned())))
    }

    /// The paste fallback: the code alone, or the whole JSON page Epic shows it in (its `authorizationCode`).
    pub fn from_pasted(pasted: &str) -> Result<Self, AuthError> {
        let text = pasted.trim();
        if !text.starts_with('{') {
            return Self::new(text);
        }
        let page: serde_json::Value = serde_json::from_str(text).map_err(|_| AuthError::BadCode)?;
        match page.get("authorizationCode") {
            Some(serde_json::Value::String(code)) => Self::new(code),
            _ => Err(AuthError::BadCode),
        }
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

/// A signed-in Epic account. Its `Debug` shows who, never the tokens.
pub struct Session {
    account_id: String,
    display_name: String,
    access_token: Token,
    access_expires_at: i64,
    refresh_token: Token,
    refresh_expires_at: i64,
}

impl Session {
    /// The expiry times are Unix seconds.
    pub fn new(
        account_id: String,
        display_name: String,
        access_token: Token,
        access_expires_at: i64,
        refresh_token: Token,
        refresh_expires_at: i64,
    ) -> Self {
        Self {
            account_id,
            display_name,
            access_token,
            access_expires_at,
            refresh_token,
            refresh_expires_at,
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn access_token(&self) -> &Token {
        &self.access_token
    }

    pub fn access_expires_at(&self) -> i64 {
        self.access_expires_at
    }

    pub fn refresh_token(&self) -> &Token {
        &self.refresh_token
    }

    pub fn refresh_expires_at(&self) -> i64 {
        self.refresh_expires_at
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("account_id", &self.account_id)
            .field("display_name", &self.display_name)
            .field("access_expires_at", &self.access_expires_at)
            .field("refresh_expires_at", &self.refresh_expires_at)
            .finish_non_exhaustive()
    }
}

/// Why signing in failed. No variant carries a token, a code or a URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    #[error("that is not an Epic authorization code")]
    BadCode,
    /// Epic refused the code (used, expired, another client's) or the refresh token (signed out elsewhere)
    #[error("Epic refused the sign-in")]
    Rejected,
    #[error("Epic's account service could not be reached")]
    Unreachable,
    /// An answer without the session's fields, or one that could not be read
    #[error("Epic's account service gave an unexpected answer")]
    Unexpected,
}

/// What Epic's token endpoint is given.
pub enum Grant {
    Code(AuthorizationCode),
    Refresh(Token),
}

/// A call to Epic's account service in flight.
pub type Pending<'a, T> = Pin<Box<dyn Future<Output = Result<T, AuthError>> + Send + 'a>>;

/// Epic's account service: the token endpoint and the end of a session.
pub trait Accounts: Send + Sync {
    fn token(&self, grant: Grant) -> Pending<'_, Session>;
    fn end_session(&self, access_token: Token) -> Pending<'_, ()>;
}

/// The account service through `egs-api`, with the Epic Games Launcher's client id. A new client for each call, so
/// no token outlives it.
#[derive(Debug, Default, Clone, Copy)]
pub struct EpicAccounts;

impl Accounts for EpicAccounts {
    fn token(&self, grant: Grant) -> Pending<'_, Session> {
        Box::pin(async move {
            let mut client = EpicGames::new();
            let started = match grant {
                Grant::Code(code) => client.try_auth_code(None, Some(code.expose().to_owned())).await,
                Grant::Refresh(token) => {
                    let mut user = UserData::new();
                    user.set_refresh_token(Some(token.expose().to_owned()));
                    client.set_user_details(user);
                    // Neither an exchange code nor an authorization code: the refresh-token grant
                    client.try_auth_code(None, None).await
                }
            };
            match started {
                Ok(true) => session_from(client.user_details()),
                Ok(false) => Err(AuthError::Unexpected),
                Err(error) => Err(classify(&error)),
            }
        })
    }

    fn end_session(&self, access_token: Token) -> Pending<'_, ()> {
        Box::pin(async move {
            let mut client = EpicGames::new();
            let mut user = UserData::new();
            user.set_access_token(Some(access_token.expose().to_owned()));
            client.set_user_details(user);
            if client.logout().await {
                Ok(())
            } else {
                Err(AuthError::Unreachable)
            }
        })
    }
}

/// An `egs-api` error as one of ours. Its own text is dropped: a transport error's names the URL.
fn classify(error: &EpicAPIError) -> AuthError {
    match error {
        EpicAPIError::InvalidCredentials | EpicAPIError::APIError(_) => AuthError::Rejected,
        EpicAPIError::NetworkError(_) | EpicAPIError::FabTimeout => AuthError::Unreachable,
        _ => AuthError::Unexpected,
    }
}

/// The session in Epic's token answer, as `egs-api` read it. Every field is needed but the display name.
fn session_from(user: UserData) -> Result<Session, AuthError> {
    let (Some(access), Some(refresh), Some(account_id), Some(access_expires), Some(refresh_expires)) = (
        user.access_token(),
        user.refresh_token(),
        user.account_id.as_deref(),
        user.expires_at,
        user.refresh_expires_at,
    ) else {
        return Err(AuthError::Unexpected);
    };
    if access.is_empty() || refresh.is_empty() || account_id.is_empty() {
        return Err(AuthError::Unexpected);
    }
    Ok(Session::new(
        account_id.to_owned(),
        user.display_name.clone().unwrap_or_default(),
        Token::new(access.to_owned()),
        access_expires.timestamp(),
        Token::new(refresh.to_owned()),
        refresh_expires.timestamp(),
    ))
}

/// Exchanges `code` for a session. The code is spent whatever the answer.
pub async fn sign_in(accounts: &dyn Accounts, code: AuthorizationCode) -> Result<Session, AuthError> {
    accounts.token(Grant::Code(code)).await
}

/// A fresh session from `session`'s refresh token. Epic turns the refresh token over each time, so the caller
/// stores the new session in place of the old one.
pub async fn refresh(accounts: &dyn Accounts, session: &Session) -> Result<Session, AuthError> {
    let refreshed = accounts
        .token(Grant::Refresh(Token::new(session.refresh_token().expose().to_owned())))
        .await?;
    if refreshed.account_id != session.account_id {
        return Err(AuthError::Unexpected);
    }
    if refreshed.display_name.is_empty() {
        return Ok(Session {
            display_name: session.display_name.clone(),
            ..refreshed
        });
    }
    Ok(refreshed)
}

/// Ends the session at Epic. The caller deletes its keystore entry whatever this answers.
pub async fn sign_out(accounts: &dyn Accounts, session: Session) -> Result<(), AuthError> {
    let Session { access_token, .. } = session;
    accounts.end_session(access_token).await
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn session(account: &str, name: &str, access: &str, refresh: &str) -> Session {
        Session::new(
            account.to_owned(),
            name.to_owned(),
            Token::new(access.to_owned()),
            1_800_000_000,
            Token::new(refresh.to_owned()),
            1_900_000_000,
        )
    }

    #[test]
    fn a_session_debugs_without_its_tokens() {
        let session = session(
            "8645b4947bbc4c0092a8b7236df169d1",
            "Slayer",
            "eg1~access-secret",
            "eg1~refresh-secret",
        );
        let shown = format!("{session:?} {session:#?}");
        assert!(shown.contains("8645b4947bbc4c0092a8b7236df169d1") && shown.contains("Slayer"));
        assert!(!shown.contains("secret"), "{shown}");
        assert_eq!(session.access_token().expose(), "eg1~access-secret");
        assert_eq!(session.refresh_token().expose(), "eg1~refresh-secret");
    }

    #[test]
    fn codes_are_trimmed_and_checked() {
        assert_eq!(
            AuthorizationCode::new("  0123456789abcdef0123456789abcdef\n")
                .unwrap()
                .expose(),
            "0123456789abcdef0123456789abcdef"
        );
        for bad in [
            "",
            "   ",
            "abc def",
            "{\"authorizationCode\":\"x\"}",
            "abc\0",
            "ü",
            &"a".repeat(257),
        ] {
            assert!(
                matches!(AuthorizationCode::new(bad), Err(AuthError::BadCode)),
                "{bad:?}"
            );
        }
    }

    /// The redirect page as Epic shows it once the player is signed in.
    const PAGE: &str = r#"{"warning":"Do not share this code with any 3rd party service. It allows full access to your Epic account.","redirectUrl":"https://localhost/launcher/authorized?code=0123456789abcdef0123456789abcdef","authorizationCode":"0123456789abcdef0123456789abcdef","exchangeCode":null,"sid":null}"#;

    #[test]
    fn a_pasted_page_gives_its_code() {
        let code = "0123456789abcdef0123456789abcdef";
        assert_eq!(AuthorizationCode::from_pasted(PAGE).unwrap().expose(), code);
        assert_eq!(
            AuthorizationCode::from_pasted(&format!("\n  {PAGE}  \r\n"))
                .unwrap()
                .expose(),
            code
        );
        assert_eq!(
            AuthorizationCode::from_pasted(&format!(" {code} ")).unwrap().expose(),
            code
        );
        for bad in [
            "",
            "{",
            r#"{"authorizationCode":null,"sid":null}"#,
            r#"{"authorizationCode":7}"#,
            r#"{"authorizationCode":"a b"}"#,
            r#"{"errorCode":"errors.com.epicgames.common.authentication.authentication_failed"}"#,
            r#"["0123456789abcdef"]"#,
            "https://localhost/launcher/authorized?code=0123456789abcdef",
        ] {
            assert!(
                matches!(AuthorizationCode::from_pasted(bad), Err(AuthError::BadCode)),
                "{bad:?}"
            );
        }
    }

    /// Epic's token answer for an authorization code, read as `egs-api` reads it.
    fn token_answer(display_name: Option<&str>) -> UserData {
        let mut answer = serde_json::json!({
            "access_token": "eg1~access-from-epic",
            "expires_in": 7200,
            "expires_at": "2026-10-26T18:00:00.000Z",
            "token_type": "bearer",
            "refresh_token": "eg1~refresh-from-epic",
            "refresh_expires": 1_987_200,
            "refresh_expires_at": "2026-11-18T16:00:00.000Z",
            "account_id": "8645b4947bbc4c0092a8b7236df169d1",
            "client_id": "34a02cf8f4414e29b15921876da36f9a",
            "internal_client": true,
            "client_service": "launcher",
            "app": "launcher",
            "in_app_id": "8645b4947bbc4c0092a8b7236df169d1",
            "device_id": "164fbd3e4e2c4ff4b9c5b1d3b0e8e1a2"
        });
        if let Some(name) = display_name {
            answer["displayName"] = name.into();
        }
        serde_json::from_value(answer).unwrap()
    }

    #[test]
    fn epics_token_answer_makes_a_session() {
        let session = session_from(token_answer(Some("Slayer"))).unwrap();
        assert_eq!(session.account_id(), "8645b4947bbc4c0092a8b7236df169d1");
        assert_eq!(session.display_name(), "Slayer");
        assert_eq!(session.access_token().expose(), "eg1~access-from-epic");
        assert_eq!(session.access_expires_at(), 1_793_037_600);
        assert_eq!(session.refresh_token().expose(), "eg1~refresh-from-epic");
        assert_eq!(session.refresh_expires_at(), 1_795_017_600);
        assert_eq!(session_from(token_answer(None)).unwrap().display_name(), "");
        let mut partial = token_answer(Some("Slayer"));
        partial.set_refresh_token(None);
        assert_eq!(session_from(partial).unwrap_err(), AuthError::Unexpected);
        let mut partial = token_answer(Some("Slayer"));
        partial.expires_at = None;
        assert_eq!(session_from(partial).unwrap_err(), AuthError::Unexpected);
        assert_eq!(session_from(UserData::new()).unwrap_err(), AuthError::Unexpected);
    }

    #[test]
    fn egs_api_errors_lose_their_text() {
        let refused = classify(&EpicAPIError::APIError(
            "Sorry the authorization code you supplied was not found".into(),
        ));
        assert_eq!(refused, AuthError::Rejected);
        assert_eq!(classify(&EpicAPIError::InvalidCredentials), AuthError::Rejected);
        assert_eq!(classify(&EpicAPIError::Server), AuthError::Unexpected);
        assert_eq!(
            classify(&EpicAPIError::DeserializationError("eg1~secret".into())).to_string(),
            "Epic's account service gave an unexpected answer"
        );
    }

    /// Epic's account service as a double: what it was given, and the session it hands out.
    #[derive(Default)]
    struct Double {
        seen: Mutex<Vec<String>>,
        display_name: &'static str,
    }

    impl Accounts for Double {
        fn token(&self, grant: Grant) -> Pending<'_, Session> {
            let seen = match &grant {
                Grant::Code(code) => format!("code {}", code.expose()),
                Grant::Refresh(token) => format!("refresh {}", token.expose()),
            };
            self.seen.lock().unwrap().push(seen);
            Box::pin(async move {
                match grant {
                    Grant::Code(code) if code.expose() == "used" => Err(AuthError::Rejected),
                    _ => Ok(session("acct", self.display_name, "access-2", "refresh-2")),
                }
            })
        }

        fn end_session(&self, access_token: Token) -> Pending<'_, ()> {
            self.seen.lock().unwrap().push(format!("end {}", access_token.expose()));
            Box::pin(async { Ok(()) })
        }
    }

    /// The double never waits, so one poll finishes each call.
    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(out) => out,
            std::task::Poll::Pending => panic!("the double waited"),
        }
    }

    #[test]
    fn sign_in_refresh_and_sign_out_go_to_the_account_service() {
        let epic = Double {
            display_name: "",
            ..Double::default()
        };
        let signed_in = ready(sign_in(&epic, AuthorizationCode::new("abc123").unwrap())).unwrap();
        assert_eq!(signed_in.access_token().expose(), "access-2");
        assert_eq!(
            ready(sign_in(&epic, AuthorizationCode::new("used").unwrap())).unwrap_err(),
            AuthError::Rejected
        );
        let old = session("acct", "Slayer", "access-1", "refresh-1");
        let refreshed = ready(refresh(&epic, &old)).unwrap();
        assert_eq!(refreshed.refresh_token().expose(), "refresh-2");
        assert_eq!(refreshed.display_name(), "Slayer", "kept when Epic leaves it out");
        let other = session("someone-else", "X", "a", "r");
        assert_eq!(ready(refresh(&epic, &other)).unwrap_err(), AuthError::Unexpected);
        ready(sign_out(&epic, refreshed)).unwrap();
        assert_eq!(
            *epic.seen.lock().unwrap(),
            [
                "code abc123",
                "code used",
                "refresh refresh-1",
                "refresh r",
                "end access-2"
            ]
        );
    }
}
