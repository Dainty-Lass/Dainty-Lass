//! Signing in to Epic with an authorization code, the flow Legendary and Heroic use (DD22). A stub until DE2, which
//! fills it with `egs-api` (DE1's verdict) and keeps the session in the keystore like the account key: never across
//! the bridge, never in a log.
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

use std::fmt;

use crate::NotImplemented;

/// The page the sign-in webview opens (or the player, for the paste fallback). Epic answers with JSON holding the
/// `authorizationCode` once the player is signed in; the client id is Epic's own launcher's.
pub const LOGIN_URL: &str = "https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode";

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

/// Why signing in failed. No variant carries a token or a code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    #[error("that is not an Epic authorization code")]
    BadCode,
    #[error(transparent)]
    NotImplemented(#[from] NotImplemented),
}

/// Exchanges `code` for a session (DE2).
pub async fn sign_in(code: AuthorizationCode) -> Result<Session, AuthError> {
    drop(code);
    Err(NotImplemented { row: "DE2" }.into())
}

/// A fresh access token from the session's refresh token (DE2).
pub async fn refresh(session: &Session) -> Result<Session, AuthError> {
    let _ = session;
    Err(NotImplemented { row: "DE2" }.into())
}

/// Ends the session at Epic; the caller then deletes its keystore entry (DE2).
pub async fn sign_out(session: Session) -> Result<(), AuthError> {
    drop(session);
    Err(NotImplemented { row: "DE2" }.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_debugs_without_its_tokens() {
        let session = Session::new(
            "8645b4947bbc4c0092a8b7236df169d1".to_owned(),
            "Slayer".to_owned(),
            Token::new("eg1~access-secret".to_owned()),
            1_800_000_000,
            Token::new("eg1~refresh-secret".to_owned()),
            1_900_000_000,
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

    #[test]
    fn the_stubs_say_which_row_fills_them() {
        let code = AuthorizationCode::new("abc").unwrap();
        let err = futures_block_on(sign_in(code)).unwrap_err();
        assert_eq!(err, AuthError::NotImplemented(NotImplemented { row: "DE2" }));
        assert_eq!(err.to_string(), "not implemented yet (DE2)");
    }

    /// The stubs never wait, so one poll finishes them.
    fn futures_block_on<F: std::future::Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(out) => out,
            std::task::Poll::Pending => panic!("a stub waited"),
        }
    }
}
