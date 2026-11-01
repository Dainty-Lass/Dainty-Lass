//! The three RS256 token kinds the backend signs with one key pair (`AUTH_SIGNING_PRIVKEY_B64` and
//! `AUTH_SIGNING_PUBKEY_B64`, base64 of PEM): the game's account token (`controllers/auth.ts`), the launcher's access
//! token (`security/launcherTokens.ts`) and the store's single-purchase token. The audience is what keeps one kind from
//! being replayed as another, so each has its own.

use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::util::now_ms;
use crate::settings::Settings;

const GAME_AUDIENCE: &str = "paradox-backend";
const GAME_ISSUER: &str = "paradox-backend";
const GAME_TTL_SECS: i64 = 24 * 60 * 60;
const LAUNCHER_AUDIENCE: &str = "rustic-isles-launcher";
const LAUNCHER_TTL_SECS: i64 = 12 * 60;
const STORE_ISSUER: &str = "rustic-isles-store";
const STORE_AUDIENCE: &str = "rustic-isles-store-purchase";
// Signed before the rename and still within their TTL at an update's restart (crate::legacy)
const LEGACY_LAUNCHER_AUDIENCE: &str = "mysticparadox-launcher";
const LEGACY_STORE_ISSUER: &str = "mysticparadox-store";
const LEGACY_STORE_AUDIENCE: &str = "mysticparadox-store-purchase";
const STORE_TTL_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("the token signing keys can't be read: {0}")]
    Keys(String),
    #[error("the token is not valid")]
    Invalid,
}

pub struct TokenKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl TokenKeys {
    pub fn from_base64_pem(private_b64: &str, public_b64: &str) -> Result<Self, TokenError> {
        let pem = |b64: &str| {
            BASE64
                .decode(b64.trim())
                .map_err(|_| TokenError::Keys("not base64".into()))
        };
        let encoding =
            EncodingKey::from_rsa_pem(&pem(private_b64)?).map_err(|_| TokenError::Keys("no RSA private key".into()))?;
        let decoding =
            DecodingKey::from_rsa_pem(&pem(public_b64)?).map_err(|_| TokenError::Keys("no RSA public key".into()))?;
        Ok(Self { encoding, decoding })
    }

    fn sign<T: Serialize>(&self, claims: &T) -> Result<String, TokenError> {
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), claims, &self.encoding)
            .map_err(|e| TokenError::Keys(e.to_string()))
    }

    /// Signature, algorithm, issuer, audience and expiry, with no leeway (jsonwebtoken's default for Node).
    fn verify(&self, token: &str, issuers: &[&str], audiences: &[&str]) -> Result<Map<String, Value>, TokenError> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(issuers);
        validation.set_audience(audiences);
        validation.leeway = 0;
        validation.validate_nbf = true;
        jsonwebtoken::decode::<Map<String, Value>>(token, &self.decoding, &validation)
            .map(|data| data.claims)
            .map_err(|_| TokenError::Invalid)
    }
}

/// The key pair of a settings value, parsed once: parsing a PEM costs more than a request should.
pub fn keys(settings: &Settings) -> Result<Arc<TokenKeys>, TokenError> {
    type Cached = Option<(String, String, Arc<TokenKeys>)>;
    static CACHE: LazyLock<Mutex<Cached>> = LazyLock::new(Mutex::default);
    let private = settings.raw("AUTH_SIGNING_PRIVKEY_B64").unwrap_or_default();
    let public = settings.raw("AUTH_SIGNING_PUBKEY_B64").unwrap_or_default();
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((cached_private, cached_public, keys)) = cache.as_ref()
        && cached_private == private
        && cached_public == public
    {
        return Ok(keys.clone());
    }
    let parsed = Arc::new(TokenKeys::from_base64_pem(private, public)?);
    *cache = Some((private.to_owned(), public.to_owned(), parsed.clone()));
    Ok(parsed)
}

#[derive(Serialize)]
struct GameClaims<'a> {
    #[serde(rename = "userId")]
    user_id: &'a str,
    iat: i64,
    exp: i64,
    aud: &'static str,
    iss: &'static str,
}

/// `SignMetagameJWTForUid`: 24 hours.
pub fn sign_game_token(settings: &Settings, user_id: &str) -> Result<String, TokenError> {
    let iat = now_ms().div_euclid(1000);
    keys(settings)?.sign(&GameClaims {
        user_id,
        iat,
        exp: iat + GAME_TTL_SECS,
        aud: GAME_AUDIENCE,
        iss: GAME_ISSUER,
    })
}

/// `ValidateMetagameJWTAndGetPayload`: every claim of a good token, as Node's `AuthData` carries them.
pub fn validate_game_token(settings: &Settings, token: &str) -> Result<Map<String, Value>, TokenError> {
    keys(settings)?.verify(token, &[GAME_ISSUER], &[GAME_AUDIENCE])
}

#[derive(Serialize)]
struct LauncherClaims<'a> {
    #[serde(rename = "userId")]
    user_id: &'a str,
    sid: &'a str,
    iat: i64,
    exp: i64,
    aud: &'static str,
    iss: &'static str,
}

/// `SignLauncherAccessToken`: 12 minutes; `sid` is the refresh session's family.
pub fn sign_launcher_access_token(settings: &Settings, user_id: &str, sid: &str) -> Result<String, TokenError> {
    let iat = now_ms().div_euclid(1000);
    keys(settings)?.sign(&LauncherClaims {
        user_id,
        sid,
        iat,
        exp: iat + LAUNCHER_TTL_SECS,
        aud: LAUNCHER_AUDIENCE,
        iss: GAME_ISSUER,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherAccessToken {
    pub user_id: String,
    pub sid: String,
}

/// `ValidateLauncherAccessToken`. Node returns the two claims as they are, so a token without them reaches the
/// account lookup with `undefined`, which finds nothing: here that is `Invalid` at once.
pub fn validate_launcher_access_token(settings: &Settings, token: &str) -> Result<LauncherAccessToken, TokenError> {
    let claims = keys(settings)?.verify(token, &[GAME_ISSUER], &[LAUNCHER_AUDIENCE, LEGACY_LAUNCHER_AUDIENCE])?;
    let text = |name: &str| claims.get(name).and_then(Value::as_str).map(str::to_owned);
    match (text("userId"), text("sid")) {
        (Some(user_id), Some(sid)) => Ok(LauncherAccessToken { user_id, sid }),
        _ => Err(TokenError::Invalid),
    }
}

/// `StorePurchaseTokenPayload`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorePurchaseToken {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "characterId")]
    pub character_id: String,
    #[serde(rename = "skuId")]
    pub sku_id: String,
    /// Absent only on a token minted by the build before the store registry
    #[serde(rename = "storeTag", skip_serializing_if = "Option::is_none", default)]
    pub store_tag: Option<String>,
}

#[derive(Serialize)]
struct StoreClaims<'a> {
    #[serde(flatten)]
    payload: &'a StorePurchaseToken,
    iat: i64,
    exp: i64,
    aud: &'static str,
    iss: &'static str,
}

/// `SignStorePurchaseToken`: 10 minutes, a different issuer and audience from the account token.
pub fn sign_store_purchase_token(settings: &Settings, payload: &StorePurchaseToken) -> Result<String, TokenError> {
    let iat = now_ms().div_euclid(1000);
    keys(settings)?.sign(&StoreClaims {
        payload,
        iat,
        exp: iat + STORE_TTL_SECS,
        aud: STORE_AUDIENCE,
        iss: STORE_ISSUER,
    })
}

/// `ValidateStorePurchaseToken`.
pub fn validate_store_purchase_token(settings: &Settings, token: &str) -> Result<StorePurchaseToken, TokenError> {
    let claims = keys(settings)?.verify(
        token,
        &[STORE_ISSUER, LEGACY_STORE_ISSUER],
        &[STORE_AUDIENCE, LEGACY_STORE_AUDIENCE],
    )?;
    serde_json::from_value(Value::Object(claims)).map_err(|_| TokenError::Invalid)
}

/// `ValidateMetagameJWTAndGetPayload` as the chat plugin's login asks for it (`isles_chat::login::TokenVerifier`): the
/// token's `userId` when the token holds, `Ok(None)` for a valid token with no string `userId`.
#[derive(Debug, Clone)]
pub struct GameTokens(pub Settings);

impl isles_chat::login::TokenVerifier for GameTokens {
    fn user_id(&self, token: &str) -> Result<Option<String>, isles_chat::login::TokenRefused> {
        let claims = validate_game_token(&self.0, token).map_err(|_| isles_chat::login::TokenRefused)?;
        Ok(claims.get("userId").and_then(Value::as_str).map(str::to_owned))
    }
}

/// `SignStorePurchaseToken` and `ValidateStorePurchaseToken` for the store routes (bounties-store's `PurchaseTokens` seam).
#[derive(Debug, Clone)]
pub struct StoreTokens(pub Settings);

impl crate::areas::bounties_store::authn::PurchaseTokens for StoreTokens {
    fn sign(&self, claims: &crate::areas::bounties_store::authn::PurchaseTokenClaims) -> Result<String, String> {
        sign_store_purchase_token(
            &self.0,
            &StorePurchaseToken {
                user_id: claims.user_id.clone(),
                character_id: claims.character_id.clone(),
                sku_id: claims.sku_id.clone(),
                store_tag: claims.store_tag.clone(),
            },
        )
        .map_err(|e| e.to_string())
    }

    fn validate(&self, token: &str) -> Result<crate::areas::bounties_store::authn::PurchaseTokenClaims, String> {
        let claims = validate_store_purchase_token(&self.0, token).map_err(|e| e.to_string())?;
        Ok(crate::areas::bounties_store::authn::PurchaseTokenClaims {
            user_id: claims.user_id,
            character_id: claims.character_id,
            sku_id: claims.sku_id,
            store_tag: claims.store_tag,
        })
    }
}

fn pem(label: &str, der: &[u8]) -> String {
    let body = BASE64.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// A throwaway RS256 key pair, generated once per process: the PEMs a host would put in `AUTH_SIGNING_PRIVKEY_B64` and
/// `AUTH_SIGNING_PUBKEY_B64` (before base64). Nothing here is ever written to a file or committed.
fn throwaway_key_pair() -> &'static (String, String) {
    use aws_lc_rs::encoding::AsDer as _;
    use aws_lc_rs::rsa::{KeyPair, KeySize};
    use aws_lc_rs::signature::KeyPair as _;
    static KEYS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| {
        let Ok(pair) = KeyPair::generate(KeySize::Rsa2048) else {
            return (String::new(), String::new());
        };
        let private = pair
            .as_der()
            .map(|der| pem("PRIVATE KEY", der.as_ref()))
            .unwrap_or_default();
        let public = pair
            .public_key()
            .as_der()
            .map(|der| pem("PUBLIC KEY", der.as_ref()))
            .unwrap_or_default();
        (private, public)
    })
}

/// The settings a test (or a harness) signs with, over a key pair generated for the process.
#[doc(hidden)]
pub fn test_key_settings(extra: &[(&str, &str)]) -> Settings {
    let (private, public) = throwaway_key_pair();
    let (private, public) = (BASE64.encode(private), BASE64.encode(public));
    let mut pairs = vec![
        ("AUTH_SIGNING_PRIVKEY_B64".to_owned(), private),
        ("AUTH_SIGNING_PUBKEY_B64".to_owned(), public),
        (
            "API_KEY_HASH_SECRET".to_owned(),
            "test-only-api-key-hash-secret-0123456789abcdef".to_owned(),
        ),
    ];
    pairs.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
    Settings::new(pairs.into_iter().collect(), Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_token_round_trips_and_a_launcher_token_is_not_one() {
        let settings = test_key_settings(&[]);
        let game = sign_game_token(&settings, "user-1").unwrap();
        let claims = validate_game_token(&settings, &game).unwrap();
        assert_eq!(claims.get("userId").and_then(Value::as_str), Some("user-1"));
        assert_eq!(claims.get("aud").and_then(Value::as_str), Some("paradox-backend"));
        let launcher = sign_launcher_access_token(&settings, "user-1", "family").unwrap();
        assert_eq!(
            validate_launcher_access_token(&settings, &launcher).unwrap(),
            LauncherAccessToken {
                user_id: "user-1".into(),
                sid: "family".into()
            }
        );
        // The audiences keep each kind where it was issued
        assert!(validate_game_token(&settings, &launcher).is_err());
        assert!(validate_launcher_access_token(&settings, &game).is_err());
        assert!(validate_game_token(&settings, "garbage").is_err());
    }

    #[test]
    fn tokens_signed_before_the_rename_still_validate() {
        let settings = test_key_settings(&[]);
        let keys = keys(&settings).unwrap();
        let iat = now_ms().div_euclid(1000);
        let launcher = keys
            .sign(
                &serde_json::json!({ "userId": "u", "sid": "s", "iat": iat, "exp": iat + 60,
                "aud": "mysticparadox-launcher", "iss": GAME_ISSUER }),
            )
            .unwrap();
        assert_eq!(
            validate_launcher_access_token(&settings, &launcher).unwrap().user_id,
            "u"
        );
        let store = keys
            .sign(
                &serde_json::json!({ "userId": "u", "characterId": "c", "skuId": "k", "iat": iat, "exp": iat + 60,
                "aud": "mysticparadox-store-purchase", "iss": "mysticparadox-store" }),
            )
            .unwrap();
        assert_eq!(validate_store_purchase_token(&settings, &store).unwrap().sku_id, "k");
    }

    #[test]
    fn a_store_token_carries_its_payload_and_no_other_kind_opens_it() {
        let settings = test_key_settings(&[]);
        let payload = StorePurchaseToken {
            user_id: "u".into(),
            character_id: "c".into(),
            sku_id: "sku".into(),
            store_tag: Some("ladyluck".into()),
        };
        let token = sign_store_purchase_token(&settings, &payload).unwrap();
        assert_eq!(validate_store_purchase_token(&settings, &token).unwrap(), payload);
        assert!(validate_game_token(&settings, &token).is_err());
        let game = sign_game_token(&settings, "u").unwrap();
        assert!(validate_store_purchase_token(&settings, &game).is_err());
    }

    #[test]
    fn the_chat_login_reads_the_user_id_of_a_game_token() {
        use isles_chat::login::TokenVerifier;
        let settings = test_key_settings(&[]);
        let verifier = GameTokens(settings.clone());
        let token = sign_game_token(&settings, "alice").unwrap();
        assert_eq!(verifier.user_id(&token), Ok(Some("alice".to_owned())));
        let launcher = sign_launcher_access_token(&settings, "alice", "family").unwrap();
        assert!(verifier.user_id(&launcher).is_err());
        assert!(verifier.user_id("forged").is_err());
    }

    #[test]
    fn unreadable_keys_are_a_keys_error() {
        let settings = Settings::from_pairs([("AUTH_SIGNING_PRIVKEY_B64", "!!"), ("AUTH_SIGNING_PUBKEY_B64", "!!")]);
        assert!(matches!(sign_game_token(&settings, "u"), Err(TokenError::Keys(_))));
    }
}
