//! `authenticateSasl` (`realtime/XMPPAuth.ts`) without the SASL framing, which is the plugin's: a PLAIN login's
//! authcid and its password, the game token. For the `Login { authcid, token }` query DN10a asks DF2 for; written
//! against [`TokenVerifier`], which the auth area implements with `ValidateMetagameJWTAndGetPayload`.
//!
//! Identity is bound from the token, never the JID: the token's `userId` must equal the authcid, the client
//! credentials service token is never a player, and the account must still exist and be eligible. The reason a login
//! was refused is for the server log only; the client is told `not-authorized` whichever it was.

use crate::source::{ChatSource, Player};

/// The `userId` of a token minted for the launcher's client-credentials grant (`routes/eos.ts`): a service, not a player.
pub const CLIENT_CREDENTIALS_USER_ID: &str = "__launcher_client_credentials__";

/// What the game's token check says about a token.
pub trait TokenVerifier: Send + Sync {
    /// The token's `userId` when its signature, issuer, audience and expiry hold; `Err` when they do not. `Ok(None)` is
    /// a valid token with no string `userId`.
    fn user_id(&self, token: &str) -> Result<Option<String>, TokenRefused>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenRefused;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    BadToken,
    NoUserId,
    ClientCredentials,
    Mismatch,
    NotEligible,
    /// The account could not be looked up: the plugin answers `temporary-auth-failure`
    Unavailable,
}

impl Refusal {
    /// Node's log reason.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadToken => "jwt verify failed",
            Self::NoUserId => "no userId in token",
            Self::ClientCredentials => "client-credentials token not allowed",
            Self::Mismatch => "authcid/token mismatch",
            Self::NotEligible => "account not eligible",
            Self::Unavailable => "account lookup error",
        }
    }
}

/// Validates a login and resolves the player.
pub async fn authenticate(
    authcid: &str,
    token: &str,
    verifier: &dyn TokenVerifier,
    source: &dyn ChatSource,
) -> Result<Player, Refusal> {
    let user_id = verifier
        .user_id(token)
        .map_err(|_| Refusal::BadToken)?
        .filter(|id| !id.is_empty())
        .ok_or(Refusal::NoUserId)?;
    if user_id == CLIENT_CREDENTIALS_USER_ID {
        return Err(Refusal::ClientCredentials);
    }
    if user_id != authcid {
        return Err(Refusal::Mismatch);
    }
    match source.player_by_user(&user_id).await {
        Ok(Some(player)) => Ok(player),
        Ok(None) => Err(Refusal::NotEligible),
        Err(_) => Err(Refusal::Unavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    /// A token that is its own `userId` after `ok:`.
    struct Tokens;

    impl TokenVerifier for Tokens {
        fn user_id(&self, token: &str) -> Result<Option<String>, TokenRefused> {
            match token.strip_prefix("ok:") {
                Some("") => Ok(None),
                Some(user) => Ok(Some(user.to_owned())),
                None => Err(TokenRefused),
            }
        }
    }

    async fn login(fake: &Fake, authcid: &str, token: &str) -> Result<Player, Refusal> {
        authenticate(authcid, token, &Tokens, fake).await
    }

    #[tokio::test]
    async fn a_login_is_bound_to_the_token_and_to_a_live_account() {
        let fake = Fake::default();
        let alice = fake.add("alice", "Alice");
        let player = login(&fake, "alice", "ok:alice").await.unwrap();
        assert_eq!((player.account, player.name.as_str()), (alice, "Alice"));
        assert_eq!(login(&fake, "alice", "forged").await, Err(Refusal::BadToken));
        assert_eq!(login(&fake, "alice", "ok:").await, Err(Refusal::NoUserId));
        assert_eq!(
            login(&fake, "mallory", "ok:alice").await,
            Err(Refusal::Mismatch),
            "the authcid is not trusted"
        );
        assert_eq!(login(&fake, "alice", "ok:mallory").await, Err(Refusal::Mismatch));
        assert_eq!(login(&fake, "ghost", "ok:ghost").await, Err(Refusal::NotEligible));
        let service = format!("ok:{CLIENT_CREDENTIALS_USER_ID}");
        assert_eq!(
            login(&fake, CLIENT_CREDENTIALS_USER_ID, &service).await,
            Err(Refusal::ClientCredentials)
        );
        let reasons: Vec<&str> = [Refusal::BadToken, Refusal::Mismatch, Refusal::NotEligible]
            .into_iter()
            .map(Refusal::as_str)
            .collect();
        assert_eq!(
            reasons,
            ["jwt verify failed", "authcid/token mismatch", "account not eligible"]
        );
    }
}
