//! Whether the signed-in account owns Dauntless (DD22): nothing is downloaded for one that doesn't, even though the
//! CDN would serve it. Epic's library is read through `egs-api` ([`EgsLibrary`]) behind the [`Library`] trait, and
//! [`Ownership`] keeps the answer a while, in memory and in a small file: the account id, the answer and when, never
//! a token. A download starts only past [`require_ownership`] (DE5).

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use egs_api::EpicGames;
use egs_api::api::types::account::UserData;
use serde::{Deserialize, Serialize};

use crate::auth::Session;

/// Dauntless's app name in Epic's library and manifests.
pub const APP_NAME: &str = "Jackal";
/// Dauntless's namespace and catalog item, from the archived manifests' `CatalogItemId` (spike DE1).
pub const NAMESPACE: &str = "jackal";
pub const CATALOG_ITEM_ID: &str = "2f415248cb524eaf950a16e55d315dac";
/// Whether a record must also carry [`NAMESPACE`] and [`CATALOG_ITEM_ID`]. Off until the maintainer reads them
/// from a real library (spike DE1, hands-on step 2), so a wrong guess can't refuse a player who owns the game.
pub const CATALOG_CONFIRMED: bool = false;

/// How long an answer is kept, in seconds. A game rarely leaves a library; a player who has just bought it
/// shouldn't wait long.
pub const OWNED_FOR: i64 = 24 * 60 * 60;
pub const REFUSED_FOR: i64 = 10 * 60;

/// The cache file in the launcher's (or the host kit's) data folder.
pub const CACHE_FILE: &str = "epic-ownership.json";
/// Anything longer is not a file this module wrote.
const CACHE_FILE_MAX: u64 = 4096;

/// One entry in the account's library, as much of it as the check reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    pub app_name: String,
    pub namespace: String,
    pub catalog_item_id: String,
}

impl Record {
    /// Whether this record is Dauntless.
    pub fn is_dauntless(&self) -> bool {
        self.matches(CATALOG_CONFIRMED)
    }

    fn matches(&self, catalog_confirmed: bool) -> bool {
        self.app_name == APP_NAME
            && (!catalog_confirmed || (self.namespace == NAMESPACE && self.catalog_item_id == CATALOG_ITEM_ID))
    }
}

/// Why the library could not be read. No variant carries a token, a URL or Epic's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LibraryError {
    /// The access token has expired: refresh the session (DE2) and ask again
    #[error("the Epic session has expired")]
    SessionExpired,
    #[error("Epic's library could not be read")]
    Unreachable,
}

/// Why a download may not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OwnershipError {
    #[error("this Epic account does not own Dauntless")]
    NotOwned,
    #[error(transparent)]
    Library(#[from] LibraryError),
}

/// The account's library: [`EgsLibrary`], or a double in tests.
pub trait Library: Send + Sync {
    fn records(&self, session: &Session) -> impl Future<Output = Result<Vec<Record>, LibraryError>> + Send;
}

/// Epic's library service, through `egs-api`. Only the access token is handed over, never the refresh token.
#[derive(Debug, Clone, Copy, Default)]
pub struct EgsLibrary;

impl Library for EgsLibrary {
    async fn records(&self, session: &Session) -> Result<Vec<Record>, LibraryError> {
        let mut user = UserData::new();
        user.set_access_token(Some(session.access_token().expose().to_owned()));
        user.token_type = Some("bearer".to_owned());
        user.account_id = Some(session.account_id().to_owned());
        let mut egs = EpicGames::new();
        egs.set_user_details(user);
        let library = egs
            .try_library_items(false)
            .await
            .map_err(|_| LibraryError::Unreachable)?;
        from_egs(library)
    }
}

/// `egs-api` logs a failed page and returns what it had so far, so a request that failed at once reads as an empty
/// library. An empty one is taken as a failure: no account that could own Dauntless has nothing in its library.
fn from_egs(library: egs_api::api::types::library::Library) -> Result<Vec<Record>, LibraryError> {
    if library.records.is_empty() {
        return Err(LibraryError::Unreachable);
    }
    Ok(library
        .records
        .into_iter()
        .map(|record| Record {
            app_name: record.app_name,
            namespace: record.namespace,
            catalog_item_id: record.catalog_item_id,
        })
        .collect())
}

/// Whether Dauntless is in the account's library, asked of Epic now (no cache).
pub async fn owns_dauntless(session: &Session) -> Result<bool, LibraryError> {
    owns(&EgsLibrary, session, unix_now()).await
}

async fn owns(library: &impl Library, session: &Session, now: i64) -> Result<bool, LibraryError> {
    if session.access_expires_at() <= now {
        return Err(LibraryError::SessionExpired);
    }
    Ok(library.records(session).await?.iter().any(Record::is_dauntless))
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

/// An answer and when it was given (Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checked {
    pub owned: bool,
    pub checked_at: i64,
}

/// What the cache file holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    account_id: String,
    owned: bool,
    checked_at: i64,
}

impl Entry {
    /// For this account and young enough. One from the future (a clock set back) is stale.
    fn fresh_for(&self, account_id: &str, now: i64) -> bool {
        let keep = if self.owned { OWNED_FOR } else { REFUSED_FOR };
        self.account_id == account_id && self.checked_at <= now && now - self.checked_at < keep
    }

    fn checked(&self) -> Checked {
        Checked {
            owned: self.owned,
            checked_at: self.checked_at,
        }
    }
}

type Clock = Box<dyn Fn() -> i64 + Send + Sync>;

/// The ownership check with its cache. One per launcher (or host kit); errors are never cached.
pub struct Ownership<L> {
    library: L,
    file: Option<PathBuf>,
    memory: Mutex<Option<Entry>>,
    clock: Clock,
}

impl<L: Library> Ownership<L> {
    /// `data_dir`: the folder [`CACHE_FILE`] goes in, or None to keep the answer in memory only.
    pub fn new(library: L, data_dir: Option<PathBuf>) -> Self {
        Self {
            library,
            file: data_dir.map(|dir| dir.join(CACHE_FILE)),
            memory: Mutex::new(None),
            clock: Box::new(unix_now),
        }
    }

    /// The same, with another clock (Unix seconds), for tests.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// The account's answer: the cached one while it is fresh, Epic's otherwise.
    pub async fn check(&self, session: &Session) -> Result<Checked, LibraryError> {
        let now = (self.clock)();
        if let Some(entry) = self.cached(session.account_id(), now) {
            return Ok(entry.checked());
        }
        let owned = owns(&self.library, session, now).await?;
        let entry = Entry {
            account_id: session.account_id().to_owned(),
            owned,
            checked_at: now,
        };
        self.save(&entry);
        let checked = entry.checked();
        *self.lock() = Some(entry);
        Ok(checked)
    }

    /// Ok only when the account owns Dauntless: what a download checks before it starts (DE5).
    pub async fn require(&self, session: &Session) -> Result<(), OwnershipError> {
        match self.check(session).await? {
            Checked { owned: true, .. } => Ok(()),
            Checked { owned: false, .. } => Err(OwnershipError::NotOwned),
        }
    }

    /// Drops the cached answer, as signing out does (DE2).
    pub fn forget(&self) {
        *self.lock() = None;
        if let Some(file) = &self.file {
            let _ = std::fs::remove_file(file);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Entry>> {
        self.memory.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cached(&self, account_id: &str, now: i64) -> Option<Entry> {
        let mut memory = self.lock();
        if let Some(entry) = memory.as_ref().filter(|entry| entry.fresh_for(account_id, now)) {
            return Some(entry.clone());
        }
        let entry = self.load().filter(|entry| entry.fresh_for(account_id, now))?;
        *memory = Some(entry.clone());
        Some(entry)
    }

    /// A missing, oversized or unreadable file is no answer.
    fn load(&self) -> Option<Entry> {
        let file = self.file.as_ref()?;
        if std::fs::metadata(file).ok()?.len() > CACHE_FILE_MAX {
            return None;
        }
        serde_json::from_slice(&std::fs::read(file).ok()?).ok()
    }

    /// Best effort: without the file the answer is only kept in memory.
    fn save(&self, entry: &Entry) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec(entry) {
            let _ = std::fs::write(file, json);
        }
    }
}

/// Ok only when the session's account owns Dauntless; the check a download start makes (DE5).
pub async fn require_ownership<L: Library>(ownership: &Ownership<L>, session: &Session) -> Result<(), OwnershipError> {
    ownership.require(session).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use super::*;
    use crate::auth::Token;

    const NOW: i64 = 1_800_000_000;
    /// Past every expiry the tests move the clock to.
    const ACCESS_EXPIRES: i64 = NOW + 30 * 86_400;

    fn session(account_id: &str) -> Session {
        Session::new(
            account_id.to_owned(),
            "Slayer".to_owned(),
            Token::new("eg1~access-secret".to_owned()),
            ACCESS_EXPIRES,
            Token::new("eg1~refresh-secret".to_owned()),
            ACCESS_EXPIRES + 86_400,
        )
    }

    fn record(app_name: &str) -> Record {
        Record {
            app_name: app_name.to_owned(),
            namespace: NAMESPACE.to_owned(),
            catalog_item_id: CATALOG_ITEM_ID.to_owned(),
        }
    }

    /// A library that answers from a script and counts the calls.
    #[derive(Clone)]
    struct Double {
        answer: Arc<Mutex<Result<Vec<Record>, LibraryError>>>,
        calls: Arc<AtomicUsize>,
    }

    impl Double {
        fn new(answer: Result<Vec<Record>, LibraryError>) -> Self {
            Self {
                answer: Arc::new(Mutex::new(answer)),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn set(&self, answer: Result<Vec<Record>, LibraryError>) {
            *self.answer.lock().unwrap() = answer;
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Library for Double {
        async fn records(&self, session: &Session) -> Result<Vec<Record>, LibraryError> {
            assert_eq!(session.access_token().expose(), "eg1~access-secret");
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer.lock().unwrap().clone()
        }
    }

    fn owned() -> Result<Vec<Record>, LibraryError> {
        Ok(vec![record("Fortnite"), record(APP_NAME)])
    }

    fn not_owned() -> Result<Vec<Record>, LibraryError> {
        Ok(vec![record("Fortnite")])
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// An ownership check on a clock the test moves.
    fn ownership(double: &Double, dir: Option<PathBuf>) -> (Ownership<Double>, Arc<AtomicI64>) {
        let clock = Arc::new(AtomicI64::new(NOW));
        let read = Arc::clone(&clock);
        let ownership = Ownership::new(double.clone(), dir).with_clock(move || read.load(Ordering::SeqCst));
        (ownership, clock)
    }

    #[test]
    fn jackal_is_matched_by_app_name_until_the_catalog_is_confirmed() {
        let mut jackal = record(APP_NAME);
        assert!(jackal.matches(false) && jackal.matches(true));
        jackal.catalog_item_id = "something-else".into();
        assert!(jackal.matches(false));
        assert!(!jackal.matches(true));
        assert_eq!(jackal.is_dauntless(), jackal.matches(CATALOG_CONFIRMED));
        for other in ["jackal", "Jackal ", "JackalTest", "", "Fortnite"] {
            assert!(!record(other).matches(false), "{other:?}");
        }
    }

    #[test]
    fn an_owner_may_download() {
        let double = Double::new(owned());
        let (ownership, _) = ownership(&double, None);
        let session = session("owner");
        assert_eq!(
            block_on(ownership.check(&session)),
            Ok(Checked {
                owned: true,
                checked_at: NOW
            })
        );
        assert_eq!(block_on(require_ownership(&ownership, &session)), Ok(()));
        assert_eq!(double.calls(), 1, "the second check came from the cache");
    }

    #[test]
    fn an_account_without_dauntless_cant_start_a_download() {
        let double = Double::new(not_owned());
        let (ownership, _) = ownership(&double, None);
        let session = session("not-an-owner");
        assert_eq!(
            block_on(require_ownership(&ownership, &session)),
            Err(OwnershipError::NotOwned)
        );
        assert_eq!(
            OwnershipError::NotOwned.to_string(),
            "this Epic account does not own Dauntless"
        );
    }

    #[test]
    fn a_network_error_refuses_and_is_never_cached() {
        let double = Double::new(Err(LibraryError::Unreachable));
        let (ownership, _) = ownership(&double, None);
        let session = session("owner");
        assert_eq!(
            block_on(require_ownership(&ownership, &session)),
            Err(OwnershipError::Library(LibraryError::Unreachable))
        );
        double.set(owned());
        assert_eq!(block_on(require_ownership(&ownership, &session)), Ok(()));
        assert_eq!(double.calls(), 2);
    }

    #[test]
    fn an_expired_session_is_refused_before_asking_epic() {
        let double = Double::new(owned());
        let (ownership, clock) = ownership(&double, None);
        clock.store(ACCESS_EXPIRES, Ordering::SeqCst);
        assert_eq!(
            block_on(ownership.require(&session("owner"))),
            Err(OwnershipError::Library(LibraryError::SessionExpired))
        );
        assert_eq!(double.calls(), 0);
    }

    #[test]
    fn answers_expire() {
        let double = Double::new(owned());
        let (ownership, clock) = ownership(&double, None);
        let session = session("owner");
        assert!(block_on(ownership.check(&session)).unwrap().owned);
        clock.store(NOW + OWNED_FOR - 1, Ordering::SeqCst);
        assert!(block_on(ownership.check(&session)).unwrap().owned);
        assert_eq!(double.calls(), 1);
        // The game left the library: seen once the day is up
        double.set(not_owned());
        clock.store(NOW + OWNED_FOR, Ordering::SeqCst);
        assert_eq!(
            block_on(ownership.check(&session)),
            Ok(Checked {
                owned: false,
                checked_at: NOW + OWNED_FOR
            })
        );
        assert_eq!(double.calls(), 2);
        // A refusal is kept only minutes: buying the game is seen soon after
        double.set(owned());
        clock.store(NOW + OWNED_FOR + REFUSED_FOR - 1, Ordering::SeqCst);
        assert!(!block_on(ownership.check(&session)).unwrap().owned);
        clock.store(NOW + OWNED_FOR + REFUSED_FOR, Ordering::SeqCst);
        assert!(block_on(ownership.check(&session)).unwrap().owned);
        assert_eq!(double.calls(), 3);
    }

    #[test]
    fn an_answer_from_the_future_is_stale() {
        let double = Double::new(owned());
        let (ownership, clock) = ownership(&double, None);
        let session = session("owner");
        clock.store(NOW + 60, Ordering::SeqCst);
        block_on(ownership.check(&session)).unwrap();
        clock.store(NOW, Ordering::SeqCst);
        block_on(ownership.check(&session)).unwrap();
        assert_eq!(double.calls(), 2);
    }

    #[test]
    fn another_account_is_asked_again() {
        let double = Double::new(owned());
        let (ownership, _) = ownership(&double, None);
        block_on(ownership.check(&session("owner"))).unwrap();
        double.set(not_owned());
        assert_eq!(
            block_on(ownership.require(&session("someone-else"))),
            Err(OwnershipError::NotOwned)
        );
        assert_eq!(double.calls(), 2);
    }

    #[test]
    fn the_file_keeps_the_answer_across_restarts_and_never_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let double = Double::new(owned());
        let (first, _) = ownership(&double, Some(dir.path().to_path_buf()));
        let session = session("8645b4947bbc4c0092a8b7236df169d1");
        block_on(first.check(&session)).unwrap();
        let written = std::fs::read_to_string(dir.path().join(CACHE_FILE)).unwrap();
        assert_eq!(
            written,
            r#"{"accountId":"8645b4947bbc4c0092a8b7236df169d1","owned":true,"checkedAt":1800000000}"#
        );
        assert!(!written.contains("secret"));

        let (restarted, clock) = ownership(&double, Some(dir.path().to_path_buf()));
        assert!(block_on(restarted.check(&session)).unwrap().owned);
        assert_eq!(double.calls(), 1, "the restarted check read the file");
        clock.store(NOW + OWNED_FOR, Ordering::SeqCst);
        block_on(restarted.check(&session)).unwrap();
        assert_eq!(double.calls(), 2, "the file's answer expires too");

        restarted.forget();
        assert!(!dir.path().join(CACHE_FILE).exists());
        block_on(restarted.check(&session)).unwrap();
        assert_eq!(double.calls(), 3);
    }

    #[test]
    fn a_damaged_or_foreign_file_is_no_answer() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(CACHE_FILE);
        let double = Double::new(not_owned());
        let session = session("owner");
        for bad in [
            "".to_owned(),
            "{".to_owned(),
            r#"{"accountId":"owner","owned":true}"#.to_owned(),
            format!(r#"{{"accountId":"owner","owned":true,"checkedAt":{NOW},"token":"x"}}"#),
            format!(
                r#"{{"accountId":"owner","owned":true,"checkedAt":{NOW}}}{}"#,
                " ".repeat(5000)
            ),
        ] {
            std::fs::write(&file, &bad).unwrap();
            let (ownership, _) = ownership(&double, Some(dir.path().to_path_buf()));
            assert_eq!(
                block_on(ownership.require(&session)),
                Err(OwnershipError::NotOwned),
                "{bad:?}"
            );
        }
        assert_eq!(double.calls(), 5);
    }

    #[test]
    fn an_empty_answer_from_egs_api_is_a_failure() {
        use egs_api::api::types::library::{Library as EgsList, Record as EgsRecord};
        assert_eq!(from_egs(EgsList::default()), Err(LibraryError::Unreachable));
        let list = EgsList {
            records: vec![EgsRecord {
                app_name: APP_NAME.into(),
                catalog_item_id: CATALOG_ITEM_ID.into(),
                namespace: NAMESPACE.into(),
                product_id: "x".into(),
                sandbox_name: "Live".into(),
            }],
            response_metadata: None,
        };
        assert_eq!(from_egs(list), Ok(vec![record(APP_NAME)]));
    }
}
