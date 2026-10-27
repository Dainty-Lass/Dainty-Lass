//! The Epic session (DD22, DE2), kept like an account key: `keys/epic.session` beside the key files, encrypted
//! by the same [`Encryptor`] (DPAPI, or the Secret Service's key on Linux). One file, since the launcher signs in
//! one Epic account. No Documents copy: signing in again is how a lost session comes back. Its tokens go on the
//! log's secret list when read or saved, and stay there after sign out.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ::epic::auth::{Session, Token};
use serde_json::{Value, json};

use super::{Encryptor, KeyStoreError};
use crate::platform::log;

/// The file's name in the keys folder; its extension is not an account key's (`.key2`, `.key`).
pub const FILE_NAME: &str = "epic.session";
/// The decrypted file's format, for a later change to recognise
const VERSION: u64 = 1;

pub struct EpicSessionStore {
    file: PathBuf,
    crypto: Arc<dyn Encryptor>,
}

impl EpicSessionStore {
    /// `keys_dir` is the account keys' folder (`userData/keys`).
    pub fn new(keys_dir: &Path, crypto: Arc<dyn Encryptor>) -> Self {
        Self {
            file: keys_dir.join(FILE_NAME),
            crypto,
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn save(&self, session: &Session) -> Result<(), KeyStoreError> {
        if !self.crypto.is_available() {
            return Err(KeyStoreError::Unavailable);
        }
        hide(session);
        let data = self.crypto.encrypt(&text(session)).ok_or(KeyStoreError::Unavailable)?;
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|_| KeyStoreError::Io)?;
        }
        let mut temporary = self.file.clone().into_os_string();
        temporary.push(".tmp");
        std::fs::write(&temporary, data).map_err(|_| KeyStoreError::Io)?;
        std::fs::rename(&temporary, &self.file).map_err(|_| KeyStoreError::Io)
    }

    /// The stored session, or None: no file, one this user or PC can't decrypt, or one Epic would refuse anyway
    /// (its refresh token expired by `now`, Unix seconds).
    pub fn load(&self, now: i64) -> Result<Option<Session>, KeyStoreError> {
        let Ok(data) = std::fs::read(&self.file) else {
            return Ok(None);
        };
        if !self.crypto.is_available() {
            return Err(KeyStoreError::Unavailable);
        }
        let session = self.crypto.decrypt(&data).and_then(|text| parse(&text));
        if let Some(session) = &session {
            hide(session);
        }
        Ok(session.filter(|session| session.refresh_expires_at() > now))
    }

    /// Sign out: the file is gone, or this says why not.
    pub fn clear(&self) -> Result<(), KeyStoreError> {
        match std::fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(KeyStoreError::Io),
        }
    }
}

fn hide(session: &Session) {
    log::add_secret(session.access_token().expose());
    log::add_secret(session.refresh_token().expose());
}

fn text(session: &Session) -> String {
    json!({
        "v": VERSION,
        "accountId": session.account_id(),
        "displayName": session.display_name(),
        "accessToken": session.access_token().expose(),
        "accessExpiresAt": session.access_expires_at(),
        "refreshToken": session.refresh_token().expose(),
        "refreshExpiresAt": session.refresh_expires_at(),
    })
    .to_string()
}

fn parse(text: &str) -> Option<Session> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("v")?.as_u64()? != VERSION {
        return None;
    }
    let field = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
    let secret = |name: &str| field(name).filter(|token| !token.is_empty()).map(Token::new);
    Some(Session::new(
        field("accountId").filter(|id| !id.is_empty())?,
        field("displayName")?,
        secret("accessToken")?,
        value.get("accessExpiresAt")?.as_i64()?,
        secret("refreshToken")?,
        value.get("refreshExpiresAt")?.as_i64()?,
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use base64::Engine as _;

    use super::*;

    /// "enc:" + base64, as the account keys' tests.
    pub struct Fake;

    impl Encryptor for Fake {
        fn is_available(&self) -> bool {
            true
        }

        fn encrypt(&self, plain: &str) -> Option<Vec<u8>> {
            Some(format!("enc:{}", base64::engine::general_purpose::STANDARD.encode(plain)).into_bytes())
        }

        fn decrypt(&self, data: &[u8]) -> Option<String> {
            let text = std::str::from_utf8(data).ok()?.strip_prefix("enc:")?;
            String::from_utf8(base64::engine::general_purpose::STANDARD.decode(text).ok()?).ok()
        }
    }

    pub fn session(access: &str, refresh: &str) -> Session {
        Session::new(
            "8645b4947bbc4c0092a8b7236df169d1".into(),
            "Slayer".into(),
            Token::new(access.into()),
            1_800_000_000,
            Token::new(refresh.into()),
            1_900_000_000,
        )
    }

    #[test]
    fn a_session_round_trips_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let store = EpicSessionStore::new(&dir.path().join("keys"), Arc::new(Fake));
        assert!(store.load(0).unwrap().is_none());
        store.save(&session("eg1~access-a1b2c3", "eg1~refresh-d4e5f6")).unwrap();
        assert_eq!(store.file(), dir.path().join("keys").join("epic.session"));
        let raw = std::fs::read_to_string(store.file()).unwrap();
        assert!(raw.starts_with("enc:") && !raw.contains("refresh-d4e5f6"), "{raw}");
        let loaded = store.load(1_850_000_000).unwrap().unwrap();
        assert_eq!(loaded.account_id(), "8645b4947bbc4c0092a8b7236df169d1");
        assert_eq!(loaded.display_name(), "Slayer");
        assert_eq!(loaded.access_token().expose(), "eg1~access-a1b2c3");
        assert_eq!(loaded.access_expires_at(), 1_800_000_000);
        assert_eq!(loaded.refresh_token().expose(), "eg1~refresh-d4e5f6");
        assert_eq!(loaded.refresh_expires_at(), 1_900_000_000);
        assert!(!log::redact("eg1~refresh-d4e5f6").contains("d4e5f6"));
        assert!(
            store.load(1_900_000_000).unwrap().is_none(),
            "a refresh token past its expiry"
        );
        store.clear().unwrap();
        assert!(!store.file().exists());
        store.clear().unwrap();
        assert!(store.load(0).unwrap().is_none());
    }

    #[test]
    fn a_damaged_or_foreign_file_is_no_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = EpicSessionStore::new(dir.path(), Arc::new(Fake));
        for plain in [
            "",
            "{}",
            r#"{"v":2,"accountId":"a","displayName":"","accessToken":"x","accessExpiresAt":1,"refreshToken":"y","refreshExpiresAt":2}"#,
            r#"{"v":1,"accountId":"a","displayName":"","accessToken":"","accessExpiresAt":1,"refreshToken":"y","refreshExpiresAt":2}"#,
            r#"{"v":1,"accountId":"a","displayName":"","accessToken":"x","accessExpiresAt":"1","refreshToken":"y","refreshExpiresAt":2}"#,
        ] {
            std::fs::write(store.file(), Fake.encrypt(plain).unwrap()).unwrap();
            assert!(store.load(0).unwrap().is_none(), "{plain}");
        }
        std::fs::write(store.file(), b"another user's DPAPI blob").unwrap();
        assert!(store.load(0).unwrap().is_none());
    }

    #[test]
    fn an_unavailable_store_saves_nothing() {
        struct Off;
        impl Encryptor for Off {
            fn is_available(&self) -> bool {
                false
            }
            fn encrypt(&self, _: &str) -> Option<Vec<u8>> {
                None
            }
            fn decrypt(&self, _: &[u8]) -> Option<String> {
                None
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let store = EpicSessionStore::new(dir.path(), Arc::new(Off));
        assert_eq!(store.save(&session("a1", "r1")), Err(KeyStoreError::Unavailable));
        assert!(!store.file().exists());
        std::fs::write(store.file(), b"x").unwrap();
        assert_eq!(store.load(0).err(), Some(KeyStoreError::Unavailable));
    }
}
