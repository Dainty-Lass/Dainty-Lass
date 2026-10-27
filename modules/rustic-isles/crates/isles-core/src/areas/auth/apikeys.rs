//! `controllers/apikeys.ts` and the key half of `controllers/auth.ts`: the dedicated game servers' keys
//! (`GAMESERVER_API_KEYS`, the Director's `METAGAME_API_KEY`) and the players' account keys, both kept only as
//! HMAC-SHA256 hashes (`security/apiKeyHash.ts`). A key is never logged.

use super::crypto::{KeyScope, find_key_record, hash_api_key};
use super::data::{store, system_actor};
use crate::http::Shared;
use crate::repo::{RepoError, RepoResult};

fn secret_error(error: super::crypto::SecretError) -> RepoError {
    RepoError::Data(error.to_string())
}

/// `ParseConfiguredGameserverAPIKeys`: comma-separated, trimmed, non-empty and unique, in the order given.
pub fn parse_configured_gameserver_api_keys(value: Option<&str>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for key in value
        .unwrap_or_default()
        .split(',')
        .map(service_kit::js::trim)
        .filter(|k| !k.is_empty())
    {
        if !keys.iter().any(|known| known == key) {
            keys.push(key.to_owned());
        }
    }
    keys
}

/// `SynchronizeConfiguredGameserverAPIKeys`: the configured list is the complete key set, so removing a compromised
/// key from the configuration revokes it at the next start. Returns how many hashes it stored.
pub async fn synchronize_configured_gameserver_api_keys(shared: &Shared, value: &str) -> RepoResult<usize> {
    let hashes = parse_configured_gameserver_api_keys(Some(value))
        .iter()
        .map(|key| hash_api_key(&shared.settings, key, KeyScope::Gameserver).map_err(secret_error))
        .collect::<RepoResult<Vec<String>>>()?;
    let count = hashes.len();
    store(shared)?
        .write(&system_actor(), move |tx, repos| {
            repos.api_keys.replace_game_server_key_hashes(tx, &hashes)
        })
        .await?;
    Ok(count)
}

/// `DrainAndRegisterAPIKeys`, at start: a non-empty `GAMESERVER_API_KEYS` is authoritative; unset or empty keeps the
/// keys already stored, so a deployment that registers keys another way is not stripped by a blank template line.
pub async fn register_gameserver_api_keys(shared: &Shared) -> RepoResult<()> {
    let configured = shared.settings.raw("GAMESERVER_API_KEYS").unwrap_or_default();
    if parse_configured_gameserver_api_keys(Some(configured)).is_empty() {
        tracing::info!("Registered 0 new Gameserver API Key(s) on boot!");
        return Ok(());
    }
    let count = synchronize_configured_gameserver_api_keys(shared, configured).await?;
    tracing::info!("Synchronized {count} Gameserver API key hash(es) from GAMESERVER_API_KEYS");
    Ok(())
}

/// `IsValidGameserverAPIKey`: every stored hash is compared in constant time.
pub async fn is_valid_gameserver_api_key(shared: &Shared, key: &str) -> RepoResult<bool> {
    let hash = hash_api_key(&shared.settings, key, KeyScope::Gameserver).map_err(secret_error)?;
    let stored = store(shared)?
        .read(|conn, repos| repos.api_keys.find_all_game_server_key_hashes(conn))
        .await?;
    Ok(find_key_record(&stored, |record| record.key_hash.as_deref(), &hash).is_some())
}

/// `GetUserIDForAPIKey`: the `userId` of the account the key belongs to. Only HMAC records match.
pub async fn user_id_for_api_key(shared: &Shared, key: &str) -> RepoResult<Option<String>> {
    let hash = hash_api_key(&shared.settings, key, KeyScope::User).map_err(secret_error)?;
    store(shared)?
        .read(move |conn, repos| {
            let stored = repos.api_keys.find_all_user_key_hashes(conn)?;
            let Some(found) = find_key_record(&stored, |record| Some(record.key_hash.as_str()), &hash) else {
                return Ok(None);
            };
            Ok(repos
                .accounts
                .find(conn, &found.account)?
                .map(|account| account.user_id.0))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::areas::auth::test_support::TestServer;

    #[test]
    fn configured_keys_are_trimmed_non_empty_and_unique() {
        assert_eq!(
            parse_configured_gameserver_api_keys(Some(" first, second ,,first,third ")),
            ["first", "second", "third"]
        );
        assert!(parse_configured_gameserver_api_keys(None).is_empty());
        assert!(parse_configured_gameserver_api_keys(Some("")).is_empty());
        assert!(parse_configured_gameserver_api_keys(Some(" , ")).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn configured_keys_replace_the_persisted_set() {
        let server = TestServer::new(&[]);
        let shared = &server.shared;
        assert_eq!(
            synchronize_configured_gameserver_api_keys(shared, "old-key")
                .await
                .unwrap(),
            1
        );
        assert!(is_valid_gameserver_api_key(shared, "old-key").await.unwrap());
        assert_eq!(
            synchronize_configured_gameserver_api_keys(shared, "new-key")
                .await
                .unwrap(),
            1
        );
        assert!(is_valid_gameserver_api_key(shared, "new-key").await.unwrap());
        assert!(!is_valid_gameserver_api_key(shared, "old-key").await.unwrap());
        let stored = store(shared)
            .unwrap()
            .read(|conn, repos| repos.api_keys.find_all_game_server_key_hashes(conn))
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].key_hash.as_ref().map(String::len), Some(64));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_configuration_keeps_the_stored_keys() {
        let server = TestServer::new(&[("GAMESERVER_API_KEYS", "kept")]);
        register_gameserver_api_keys(&server.shared).await.unwrap();
        assert!(is_valid_gameserver_api_key(&server.shared, "kept").await.unwrap());
        let blank = TestServer::with_store_of(&server, &[("GAMESERVER_API_KEYS", "  ")]);
        register_gameserver_api_keys(&blank.shared).await.unwrap();
        assert!(is_valid_gameserver_api_key(&blank.shared, "kept").await.unwrap());
        assert!(!is_valid_gameserver_api_key(&blank.shared, "other").await.unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_user_key_finds_its_account() {
        let server = TestServer::new(&[]);
        let user = server
            .add_launcher_account("Fox", "fox@example.test", "password-123")
            .await;
        server.user_key(&user, "RRK_secret").await;
        assert_eq!(
            user_id_for_api_key(&server.shared, "RRK_secret").await.unwrap(),
            Some(user.user_id.0.clone())
        );
        assert_eq!(user_id_for_api_key(&server.shared, "RRK_other").await.unwrap(), None);
    }
}
