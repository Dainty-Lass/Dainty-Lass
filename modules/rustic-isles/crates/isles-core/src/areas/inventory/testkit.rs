//! What this area's tests share, across `isles-core` and the two module crates: in-memory stand-ins for the other
//! areas' pieces (the ports of [`super::ports`], the characters and progression repositories), a server on a real
//! SQLite store, a small HTTP client, and the golden run that replays `tests/golden/inventory/scenarios.json` against
//! the answers recorded from Mystic Paradox's Node backend (`record.cjs`). Not for production code.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use module_api::network::StandaloneHome;
use module_api::{BoxFuture, NodeIdentity};
use regex::Regex;
use serde_json::{Map, Value};
pub use store::DatabaseConfig;
use store::DatabaseOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::clock::iso_now;
use super::ports::{
    AuthData, AuthRequest, Authentication, Authenticator, CharacterOwnership, CurrencyChange, Ports, StarterBootstrap,
    Wallet, WalletError,
};
use crate::db::IslesStore;
use crate::game_data::GameDataRoot;
use crate::http::{Handlers, Shared, app};
use crate::repo::auth::AccountRecord;
use crate::repo::characters::{CharacterRecord, CharacterRepository};
use crate::repo::inventory::{InventoryRecord, apply_reforge_update};
use crate::repo::progression::{
    ProgressionGrantRecord, ProgressionGrantRepository, ProgressionObjectiveRecord, ProgressionTrackRecord,
    ProgressionTrackRepository, WeaponReforge,
};
use crate::repo::{AccountId, AnyConnection, Json, RepoError, RepoResult, Repositories, Stub, Tx, UserId};
use crate::routes::Service;
use crate::settings::Settings;
use crate::version::IslesVersion;

pub const SCENARIOS: &str = include_str!("../../../tests/golden/inventory/scenarios.json");
pub const EXPECTED: &str = include_str!("../../../tests/golden/inventory/expected.json");

/// The game data folder the golden run reads (`inventory_storage_policy.json`, `progression_config.json`).
pub fn golden_game_data() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/inventory/game-data")
}

/// The scenario's `auth` is the `AuthData`, in the header `x-test-auth`.
pub struct HeaderAuth;

impl Authenticator for HeaderAuth {
    fn authenticate<'a>(&'a self, _: &'a Shared, request: AuthRequest<'a>) -> BoxFuture<'a, Authentication> {
        let auth = auth_of(request.headers);
        Box::pin(async move {
            match auth {
                Some(auth) => Authentication::Authenticated(auth),
                None => Authentication::Answer(super::respond::empty(401)),
            }
        })
    }
}

fn auth_of(headers: &HeaderMap) -> Option<AuthData> {
    let value: Value = serde_json::from_str(headers.get("x-test-auth")?.to_str().ok()?).ok()?;
    Some(AuthData {
        is_gameserver: value.get("IsGameserver").and_then(Value::as_bool).unwrap_or(false),
        user_id: value.get("userId").and_then(Value::as_str).map(str::to_owned),
        // `"host": true` is a realm host's runtime (IsPlayerHostRuntime)
        host_runtime: value
            .get("host")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| rifts_seam::RuntimeAuth {
                capability_id: "cap".to_owned(),
                session: "01ARZ3NDEKTSV4RRFFQ69G5FAV@local"
                    .parse()
                    .unwrap_or_else(|e| panic!("{e}")),
                generation: 1,
                host_account: rifts_seam::AccountId("host".to_owned()),
                scopes: Vec::new(),
                roster: Vec::new(),
            }),
    })
}

/// The wallet as `wallet.ts` keeps it, in memory (not rolled back with the transaction).
#[derive(Default)]
pub struct MemoryWallet {
    balances: Mutex<BTreeMap<String, Vec<(String, i64)>>>,
}

impl MemoryWallet {
    pub fn set(&self, user: &str, balances: Vec<(String, i64)>) {
        if let Ok(mut all) = self.balances.lock() {
            all.insert(user.to_owned(), balances);
        }
    }

    pub fn balance(&self, user: &str, catalog_id: &str) -> Option<i64> {
        let all = self.balances.lock().ok()?;
        all.get(user)?.iter().find(|(id, _)| id == catalog_id).map(|(_, n)| *n)
    }
}

impl Wallet for MemoryWallet {
    fn apply_currency_deltas(
        &self,
        _: &mut Tx<'_>,
        _: &Repositories,
        change: &CurrencyChange<'_>,
    ) -> Result<Map<String, Value>, WalletError> {
        let mut touched = Map::new();
        let mut all = self.balances.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        let wallet = all.entry(change.user_id.to_owned()).or_default();
        let apply = |wallet: &mut Vec<(String, i64)>, id: &str, delta: i64| -> Result<i64, WalletError> {
            let index = wallet.iter().position(|(known, _)| known == id);
            let current = index.map_or(0, |i| wallet[i].1);
            if delta < 0 && current < -delta {
                return Err(WalletError::InsufficientBalance(format!(
                    "Insufficient balance: user {} has {current} of {id}, cannot apply delta {delta}",
                    change.user_id
                )));
            }
            match index {
                Some(i) => wallet[i].1 += delta,
                None => wallet.push((id.to_owned(), delta)),
            }
            Ok(current + delta)
        };
        for item in change.adds {
            if let Some(id) = item
                .get("catalogId")
                .and_then(Value::as_str)
                .filter(|id| id.starts_with("CURRENCY_"))
            {
                let quantity = item.get("quantity").and_then(Value::as_i64).unwrap_or(0);
                let balance = apply(wallet, id, quantity)?;
                touched.insert(id.to_owned(), Value::from(balance));
            }
        }
        for item in change.removes {
            if let Some(id) = item
                .get("catalogId")
                .and_then(Value::as_str)
                .filter(|id| id.starts_with("CURRENCY_"))
            {
                let quantity = item.get("quantity").and_then(Value::as_i64).unwrap_or(0);
                let balance = apply(wallet, id, -quantity)?;
                touched.insert(id.to_owned(), Value::from(balance));
            }
        }
        Ok(touched)
    }

    fn merge_into_stacked(
        &self,
        _: &mut Tx<'_>,
        _: &Repositories,
        _: &AccountId,
        user_id: &str,
        stacked: Vec<Value>,
    ) -> RepoResult<Vec<Value>> {
        let all = self.balances.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        let mut out = stacked;
        for (id, amount) in all.get(user_id).map(Vec::as_slice).unwrap_or_default() {
            match out
                .iter()
                .position(|item| item.get("catalogId").and_then(Value::as_str) == Some(id.as_str()))
            {
                Some(index) => {
                    if let Value::Object(fields) = &mut out[index] {
                        fields.insert("catalogId".into(), Value::String(id.clone()));
                        fields.insert("quantity".into(), Value::from(*amount));
                    }
                }
                None => out.push(serde_json::json!({"catalogId": id, "quantity": amount})),
            }
        }
        Ok(out)
    }

    fn wait_for_fundable_season_claim<'a>(&'a self, _: &'a IslesStore, _: &'a AccountId) -> BoxFuture<'a, bool> {
        Box::pin(async { true })
    }
}

/// Serves the inventory that exists; creating one from the starter manifest is the characters area's.
pub struct ExistingInventory;

impl StarterBootstrap for ExistingInventory {
    fn ensure(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        _: &Shared,
        _: &AccountId,
        user_id: &str,
        character_id: &str,
    ) -> RepoResult<Result<InventoryRecord, CharacterOwnership>> {
        Ok(repos
            .inventories
            .find_by_character_id(tx.conn(), character_id)?
            .ok_or_else(|| CharacterOwnership {
                character_id: character_id.to_owned(),
                user_id: user_id.to_owned(),
            }))
    }
}

/// Characters in memory, in the order they were made.
#[derive(Default)]
pub struct MemoryCharacters {
    rows: Mutex<Vec<CharacterRecord>>,
}

impl MemoryCharacters {
    pub fn add(&self, record: CharacterRecord) {
        if let Ok(mut rows) = self.rows.lock() {
            rows.push(record);
        }
    }
}

impl CharacterRepository for MemoryCharacters {
    fn find_many_by_account(&self, _: &mut AnyConnection, account: &AccountId) -> RepoResult<Vec<CharacterRecord>> {
        let rows = self.rows.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        Ok(rows.iter().filter(|c| c.account == *account).cloned().collect())
    }

    fn find(
        &self,
        _: &mut AnyConnection,
        character_id: &str,
        account: &AccountId,
    ) -> RepoResult<Option<CharacterRecord>> {
        let rows = self.rows.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        Ok(rows
            .iter()
            .find(|c| c.character_id == character_id && c.account == *account)
            .cloned())
    }

    fn create(&self, tx: &mut Tx<'_>, character: &CharacterRecord) -> RepoResult<CharacterRecord> {
        Stub.create(tx, character)
    }

    fn update_data_conditional(
        &self,
        tx: &mut Tx<'_>,
        character_id: &str,
        account: &AccountId,
        data: &str,
        version: i64,
    ) -> RepoResult<()> {
        Stub.update_data_conditional(tx, character_id, account, data, version)
    }
}

/// `progressionTracks` in memory, with the compare-and-set rules of the SQLite provider's `beginReforge` and
/// `updateReforge`.
#[derive(Default)]
pub struct MemoryTracks {
    rows: Mutex<Vec<ProgressionTrackRecord>>,
}

impl MemoryTracks {
    fn with<T>(&self, f: impl FnOnce(&mut Vec<ProgressionTrackRecord>) -> T) -> RepoResult<T> {
        let mut rows = self.rows.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        Ok(f(&mut rows))
    }

    pub fn seed(&self, account: &AccountId, progression_id: &str, progress: i64) {
        let _ = self.with(|rows| {
            rows.push(ProgressionTrackRecord {
                account: account.clone(),
                progression_id: progression_id.to_owned(),
                progress,
                confirmed_fremium_rank: 0,
                confirmed_premium_rank: 0,
                update_version: 1,
                created_at: iso_now(),
                updated_at: iso_now(),
                reforge: None,
                prestige_claimed: None,
            });
        });
    }
}

impl ProgressionTrackRepository for MemoryTracks {
    fn get_all_for_account(
        &self,
        _: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Vec<ProgressionTrackRecord>> {
        self.with(|rows| rows.iter().filter(|r| r.account == *account).cloned().collect())
    }

    fn get(&self, _: &mut AnyConnection, account: &AccountId, id: &str) -> RepoResult<Option<ProgressionTrackRecord>> {
        self.with(|rows| {
            rows.iter()
                .find(|r| r.account == *account && r.progression_id == id)
                .cloned()
        })
    }

    fn increment(
        &self,
        _: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        amount: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        self.with(|rows| {
            if let Some(row) = rows
                .iter_mut()
                .find(|r| r.account == *account && r.progression_id == id)
            {
                row.progress += amount;
                row.update_version += 1;
                row.updated_at = iso_now();
                return row.clone();
            }
            let row = ProgressionTrackRecord {
                account: account.clone(),
                progression_id: id.to_owned(),
                progress: amount,
                confirmed_fremium_rank: 0,
                confirmed_premium_rank: 0,
                update_version: 1,
                created_at: iso_now(),
                updated_at: iso_now(),
                reforge: None,
                prestige_claimed: None,
            };
            rows.push(row.clone());
            row
        })
    }

    fn set_progress_if_greater(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        value: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        Stub.set_progress_if_greater(tx, account, id, value)
    }

    fn set_confirmed_fremium_rank(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        rank: i64,
    ) -> RepoResult<ProgressionTrackRecord> {
        Stub.set_confirmed_fremium_rank(tx, account, id, rank)
    }

    fn append_objective_event(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        raw_body: &str,
        received_at: &str,
    ) -> RepoResult<()> {
        Stub.append_objective_event(tx, account, raw_body, received_at)
    }

    fn get_all_objectives_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Vec<ProgressionObjectiveRecord>> {
        Stub.get_all_objectives_for_account(conn, account)
    }

    fn set_objective_if_greater(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        value: i64,
        completed: i64,
    ) -> RepoResult<ProgressionObjectiveRecord> {
        Stub.set_objective_if_greater(tx, account, id, value, completed)
    }

    fn begin_reforge(
        &self,
        _: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        expected_progress: i64,
        expected_version: i64,
        reforge: &WeaponReforge,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        self.with(|rows| {
            let row = rows
                .iter_mut()
                .find(|r| r.account == *account && r.progression_id == id)?;
            if row.progress != expected_progress
                || row.update_version != expected_version
                || row.reforge.as_ref().is_some_and(|r| r.open)
            {
                return None;
            }
            row.progress = 0;
            row.confirmed_fremium_rank = 0;
            row.confirmed_premium_rank = 0;
            row.reforge = Some(reforge.clone());
            row.updated_at = reforge.opened_at.clone();
            row.update_version += 1;
            Some(row.clone())
        })
    }

    fn update_reforge(
        &self,
        _: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        reforge_id: &str,
        expected: &Json,
        set: &Json,
    ) -> RepoResult<bool> {
        self.with(|rows| {
            let Some(row) = rows
                .iter_mut()
                .find(|r| r.account == *account && r.progression_id == id)
            else {
                return false;
            };
            let Some(next) = row
                .reforge
                .as_ref()
                .and_then(|current| apply_reforge_update(current, reforge_id, expected, set))
            else {
                return false;
            };
            row.reforge = Some(next);
            true
        })
    }

    fn claim_prestige_levels(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        before: i64,
        levels: i64,
        min: i64,
    ) -> RepoResult<bool> {
        Stub.claim_prestige_levels(tx, account, id, before, levels, min)
    }

    fn get_objectives(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        ids: &[String],
    ) -> RepoResult<Vec<ProgressionObjectiveRecord>> {
        Stub.get_objectives(conn, account, ids)
    }

    fn merge_objectives(&self, tx: &mut Tx<'_>, account: &AccountId, rows: &[(String, i64, i64)]) -> RepoResult<()> {
        Stub.merge_objectives(tx, account, rows)
    }

    fn spend(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        id: &str,
        amount: i64,
    ) -> RepoResult<Option<ProgressionTrackRecord>> {
        Stub.spend(tx, account, id, amount)
    }
}

#[derive(Default)]
pub struct MemoryGrants {
    rows: Mutex<Vec<ProgressionGrantRecord>>,
}

impl ProgressionGrantRepository for MemoryGrants {
    fn find_by_grant_id(&self, _: &mut AnyConnection, grant_id: &str) -> RepoResult<Option<ProgressionGrantRecord>> {
        let rows = self.rows.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        Ok(rows.iter().find(|r| r.grant_id == grant_id).cloned())
    }

    fn insert_applied(&self, _: &mut Tx<'_>, record: &ProgressionGrantRecord) -> RepoResult<()> {
        let mut rows = self.rows.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        rows.push(record.clone());
        Ok(())
    }

    fn list_recent_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<ProgressionGrantRecord>> {
        Stub.list_recent_for_account(conn, account, limit)
    }
}

/// A server on a SQLite store with the stand-ins above.
pub struct Harness {
    pub address: SocketAddr,
    pub store: IslesStore,
    pub accounts: BTreeMap<String, AccountId>,
    pub wallet: Arc<MemoryWallet>,
    pub tracks: Arc<MemoryTracks>,
    pub characters: Arc<MemoryCharacters>,
    pub repos: Repositories,
}

/// The SQLite database in `dir`.
pub fn sqlite_config(dir: &Path) -> DatabaseConfig {
    DatabaseConfig::Sqlite {
        file: dir.join("isles.db"),
    }
}

/// The Postgres half's database, `REFORGED_TEST_POSTGRES_URL` (`postgres://user:password@host:port/database`, a
/// throwaway on 127.0.0.1), when set.
pub fn postgres_config() -> Option<DatabaseConfig> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))?;
    let (credentials, location) = rest.rsplit_once('@')?;
    let (user, password) = credentials.split_once(':').unwrap_or((credentials, ""));
    let (authority, database) = location.split_once('/')?;
    let (host, port) = authority.rsplit_once(':').unwrap_or((authority, "5432"));
    Some(DatabaseConfig::Postgres(store::PostgresConfig {
        host: host.to_owned(),
        port: port.parse().ok()?,
        user: user.to_owned(),
        database: database.to_owned(),
        password: password.to_owned(),
        allow_remote: false,
    }))
}

/// Opens the store the config names.
pub fn open_store(config: &DatabaseConfig) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    match IslesStore::open(config, &DatabaseOptions::default(), home, &node) {
        Ok(store) => store,
        Err(error) => panic!("the store did not open: {error}"),
    }
}

/// The repositories of a store with the stand-ins in place of the other areas'.
pub fn repositories(store: &IslesStore) -> (Repositories, Arc<MemoryCharacters>, Arc<MemoryTracks>) {
    let characters = Arc::new(MemoryCharacters::default());
    let tracks = Arc::new(MemoryTracks::default());
    let mut repos = store.repos().clone();
    repos.characters = characters.clone();
    repos.progression_tracks = tracks.clone();
    repos.progression_grants = Arc::new(MemoryGrants::default());
    (repos, characters, tracks)
}

impl Harness {
    /// [`Harness::start_on`] with the SQLite database in `dir`.
    pub async fn start(dir: &Path, version: IslesVersion, settings: Settings, seed: &Value) -> Self {
        Self::start_on(&sqlite_config(dir), version, settings, seed).await
    }

    /// Seeds `scenarios.json`'s users and serves `version`.
    pub async fn start_on(config: &DatabaseConfig, version: IslesVersion, settings: Settings, seed: &Value) -> Self {
        Self::start_with(config, version, settings, seed, rifts_seam::Rifts::dedicated_only()).await
    }

    /// [`Harness::start_on`] with realm hooks (rustic-rifts) in place of the dedicated-only ones.
    pub async fn start_with(
        config: &DatabaseConfig,
        version: IslesVersion,
        settings: Settings,
        seed: &Value,
        rifts: rifts_seam::Rifts,
    ) -> Self {
        let store = open_store(config);
        let (repos, characters, tracks) = repositories(&store);
        let wallet = Arc::new(MemoryWallet::default());
        let mut accounts = BTreeMap::new();
        let node = NodeIdentity::standalone();
        for user in seed["users"].as_array().into_iter().flatten() {
            let user_id = user["userId"].as_str().unwrap_or_default().to_owned();
            let account = AccountId::new(&node.node);
            let record = AccountRecord {
                id: account.clone(),
                user_id: UserId(user_id.clone()),
                name: format!("Hunter {user_id}"),
                notes: 0,
            };
            let actor = super::transaction::player_actor(&account, None);
            let wrote = store
                .account_write(&account, &actor, {
                    let (record, repos) = (record, repos.clone());
                    move |tx, _| repos.accounts.create(tx, &record)
                })
                .await;
            assert!(wrote.is_ok(), "{wrote:?}");
            wallet.set(
                &user_id,
                user["wallet"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(id, n)| (id.clone(), n.as_i64().unwrap_or(0)))
                    .collect(),
            );
            for character in user["characters"].as_array().into_iter().flatten() {
                let character_id = character["characterId"].as_str().unwrap_or_default().to_owned();
                characters.add(CharacterRecord {
                    character_id: character_id.clone(),
                    account: account.clone(),
                    created_date: "2026-10-01T00:00:00.000Z".into(),
                    last_modified_date: "2026-10-01T00:00:00.000Z".into(),
                    name: format!("Hunter {character_id}"),
                    update_version: 1,
                    data: "{}".into(),
                });
                let inventory = InventoryRecord {
                    character_id,
                    account: Some(account.clone()),
                    instanced_items: character["instanced"].to_string(),
                    stacked_items: character["stacked"].to_string(),
                    revision: Some(0),
                    bootstrap_version: Some("golden".into()),
                };
                let wrote = store
                    .account_write(&account, &actor, {
                        let repos = repos.clone();
                        move |tx, _| repos.inventories.create(tx, &inventory)
                    })
                    .await;
                assert!(wrote.is_ok(), "{wrote:?}");
            }
            for (track, progress) in user["tracks"].as_object().into_iter().flatten() {
                tracks.seed(&account, track, progress.as_i64().unwrap_or(0));
            }
            accounts.insert(user_id, account);
        }

        let mut shared = Shared::bare(version);
        shared.settings = settings;
        shared.game_data = GameDataRoot::from_config(golden_game_data().to_str(), None);
        shared.store = Some(store.clone());
        shared.rifts = rifts;
        let ports = Ports {
            auth: Arc::new(HeaderAuth),
            wallet: wallet.clone(),
            starter: Arc::new(ExistingInventory),
            repos: Some(repos.clone()),
            ..Ports::default()
        };
        let mut handlers = Handlers::default();
        super::register_with(&mut handlers, ports);
        let router = match app(Arc::new(shared), Service::Backend, &handlers) {
            Ok(router) => router,
            Err(error) => panic!("the app did not mount: {error}"),
        };
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(error) => panic!("no port: {error}"),
        };
        let address = listener.local_addr().unwrap_or_else(|error| panic!("{error}"));
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self {
            address,
            store,
            accounts,
            wallet,
            tracks,
            characters,
            repos,
        }
    }

    pub async fn send(&self, method: &str, path: &str, headers: &[(&str, String)], body: Option<&str>) -> Reply {
        request(self.address, method, path, headers, body).await
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

/// One HTTP/1.1 request on a fresh connection; a body (even an empty one) is sent with its Content-Length.
pub async fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
) -> Reply {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = body {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    if let Some(body) = body {
        head.push_str(body);
    }
    let mut stream = TcpStream::connect(address)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    stream
        .write_all(head.as_bytes())
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let content_type = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.trim().to_owned());
    Reply {
        status,
        content_type,
        body: body.to_owned(),
    }
}

/// The recorder's `normalize`: timestamps, UUIDs and minted ids differ by run.
pub fn normalize(text: &str) -> String {
    let ts = Regex::new(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z").unwrap_or_else(|e| panic!("{e}"));
    let uuid =
        Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap_or_else(|e| panic!("{e}"));
    let id = Regex::new(r"[A-Z2-7]{26}").unwrap_or_else(|e| panic!("{e}"));
    let text = ts.replace_all(text, "<TS>");
    let text = uuid.replace_all(&text, "<UUID>");
    id.replace_all(&text, "<ID>").into_owned()
}

/// Replays the scenarios against `harness` and reports what differs from the recording (or from `override_for`'s
/// expectations, by scenario name).
pub async fn replay(harness: &Harness) -> Vec<(String, Reply)> {
    let scenarios: Value = serde_json::from_str(SCENARIOS).unwrap_or_else(|e| panic!("{e}"));
    let mut out = Vec::new();
    for scenario in scenarios["scenarios"].as_array().into_iter().flatten() {
        let mut headers = Vec::new();
        if let Some(auth) = scenario.get("auth") {
            headers.push(("x-test-auth", auth.to_string()));
        }
        let body = match (scenario.get("raw"), scenario.get("body")) {
            (Some(raw), _) => Some(raw.as_str().unwrap_or_default().to_owned()),
            (None, Some(body)) => Some(body.to_string()),
            _ => None,
        };
        if body.is_some() {
            let content_type = scenario
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap_or("application/json");
            headers.push(("content-type", content_type.to_owned()));
        }
        let mut reply = harness
            .send(
                scenario["method"].as_str().unwrap_or("GET"),
                scenario["path"].as_str().unwrap_or("/"),
                &headers,
                body.as_deref(),
            )
            .await;
        reply.body = normalize(&reply.body);
        out.push((scenario["name"].as_str().unwrap_or_default().to_owned(), reply));
    }
    out
}

/// What Node recorded, by scenario name.
pub fn recorded() -> Vec<(String, Reply)> {
    let expected: Value = serde_json::from_str(EXPECTED).unwrap_or_else(|e| panic!("{e}"));
    expected
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap_or_default().to_owned(),
                Reply {
                    status: entry["status"].as_u64().unwrap_or(0) as u16,
                    content_type: entry["contentType"].as_str().map(str::to_owned),
                    body: entry["body"].as_str().unwrap_or_default().to_owned(),
                },
            )
        })
        .collect()
}
