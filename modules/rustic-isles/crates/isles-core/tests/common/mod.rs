//! What the Reforged API area's tests share: a module's backend served on a real socket over a SQLite database,
//! fakes for the tables the auth area owns, and a raw HTTP client that sends headers and bodies.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test helpers

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use isles_core::db::IslesStore;
use isles_core::hooks::{
    Destination, Entitlement, GameMode, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, StubReply,
};
use isles_core::http::{Shared, app};
use isles_core::repo::auth::*;
use isles_core::repo::{AnyConnection, RepoError, RepoResult, Repositories, Stub, Tx};
use isles_core::routes::{RouteSet, Service};
use isles_core::tables::{StarterManifest, VersionTables};
use isles_core::{AccountId, IslesVersion, Settings, UserId, VersionHooks};
use module_api::NodeIdentity;
use module_api::network::StandaloneHome;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub struct NoHooks;

impl VersionHooks for NoHooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        NoPlayerPolicy::Off
    }
    fn game_session_payload(&self, _: &str) -> Value {
        Value::Null
    }
    fn network_encryption_keys(&self) -> bool {
        false
    }
    fn weapons_at_base_level(&self, _: &Settings) -> bool {
        false
    }
    fn reforge_rules(&self) -> bool {
        false
    }
    fn mastery_persistence(&self) -> bool {
        false
    }
    fn huntpass_payload(&self, _: &str) -> Value {
        Value::Null
    }
    fn objective_row(&self, _: &str, _: &ObjectiveRow) -> Value {
        Value::Null
    }
    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::AsIs
    }
    fn survey_config(&self) -> StubReply {
        StubReply {
            status: 400,
            body: None,
        }
    }
    fn entitlements_payload(&self, _: &[Entitlement]) -> Value {
        Value::Null
    }
    fn candidate_destination(&self, hunt_id: &str, _: bool) -> Destination {
        Destination {
            game_mode: GameMode::Island,
            hunt_id: Some(hunt_id.to_owned()),
        }
    }
}

pub const fn tables(changelist: u32, game_version: &'static str) -> VersionTables {
    VersionTables {
        target_changelist: changelist,
        game_version,
        active_huntpass_id: "",
        recent_huntpass_seasons: &[],
        patchnotes_release_version: "",
        bounty_item_grant_data: &[],
        store_sections: &[],
        seasonal_coin: None,
        masked_journey_nodes: &[],
        starter_manifest: StarterManifest {
            bootstrap_version: "",
            stacked: &[],
            quick_emojis: None,
        },
        switch_defaults: &[],
    }
}

pub fn version(routes: RouteSet) -> IslesVersion {
    let (changelist, game) = match routes {
        RouteSet::V1147 => (647_472, "1.14.7"),
        RouteSet::V1120 => (392_819, "1.12.0"),
    };
    IslesVersion {
        hooks: Arc::new(NoHooks),
        tables: tables(changelist, game),
        routes,
    }
}

/// What the fakes of the auth area's tables hold.
#[derive(Default)]
pub struct World {
    pub accounts: Vec<AccountRecord>,
    pub launchers: Vec<LauncherAccountRecord>,
    pub keys: Vec<UserKeyHash>,
    pub refresh_revoked: Vec<AccountId>,
    pub codes_revoked: Vec<AccountId>,
}

#[derive(Clone, Default)]
pub struct Fakes(pub Arc<Mutex<World>>);

impl Fakes {
    pub fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.0.lock().unwrap()
    }
}

impl AccountRepository for Fakes {
    fn find_by_user_id(&self, _: &mut AnyConnection, user: &UserId) -> RepoResult<Option<AccountRecord>> {
        Ok(self.world().accounts.iter().find(|a| &a.user_id == user).cloned())
    }
    fn find(&self, _: &mut AnyConnection, id: &AccountId) -> RepoResult<Option<AccountRecord>> {
        Ok(self.world().accounts.iter().find(|a| &a.id == id).cloned())
    }
    fn create(&self, _: &mut Tx<'_>, account: &AccountRecord) -> RepoResult<()> {
        self.world().accounts.push(account.clone());
        Ok(())
    }
}

impl LauncherAccountRepository for Fakes {
    fn find_by_user_id(&self, _: &mut AnyConnection, user: &UserId) -> RepoResult<Option<LauncherAccountRecord>> {
        Ok(self.world().launchers.iter().find(|a| &a.user_id == user).cloned())
    }
    fn find_by_account(&self, _: &mut AnyConnection, id: &AccountId) -> RepoResult<Option<LauncherAccountRecord>> {
        Ok(self.world().launchers.iter().find(|a| &a.account == id).cloned())
    }
    fn find_by_email(&self, _: &mut AnyConnection, e: &str) -> RepoResult<Option<LauncherAccountRecord>> {
        Ok(self
            .world()
            .launchers
            .iter()
            .find(|a| a.email.as_deref() == Some(e))
            .cloned())
    }
    fn find_by_display_name_normalized(
        &self,
        _: &mut AnyConnection,
        normalized: &str,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        Ok(self
            .world()
            .launchers
            .iter()
            .find(|a| a.display_name_normalized == normalized)
            .cloned())
    }
    fn create(&self, _: &mut Tx<'_>, account: &LauncherAccountRecord) -> RepoResult<CreateOutcome> {
        self.world().launchers.push(account.clone());
        Ok(CreateOutcome::Created)
    }
    fn update_last_login(&self, t: &mut Tx<'_>, a: &AccountId, w: &str) -> RepoResult<()> {
        Stub.update_last_login(t, a, w)
    }
    fn set_username(&self, t: &mut Tx<'_>, a: &AccountId, d: &str, n: &str) -> RepoResult<CreateOutcome> {
        Stub.set_username(t, a, d, n)
    }
    fn set_consent(&self, t: &mut Tx<'_>, a: &AccountId, c: &LauncherConsent) -> RepoResult<()> {
        Stub.set_consent(t, a, c)
    }
    fn set_deletion(&self, t: &mut Tx<'_>, a: &AccountId, d: Option<&LauncherDeletion>) -> RepoResult<()> {
        Stub.set_deletion(t, a, d)
    }
    fn list_deletions_due(&self, c: &mut AnyConnection, n: &str, l: u32) -> RepoResult<Vec<LauncherAccountRecord>> {
        Stub.list_deletions_due(c, n, l)
    }
    fn set_email_verified(&self, t: &mut Tx<'_>, a: &AccountId, v: &str) -> RepoResult<()> {
        Stub.set_email_verified(t, a, v)
    }
    fn set_password_hash(&self, t: &mut Tx<'_>, a: &AccountId, p: &str) -> RepoResult<()> {
        Stub.set_password_hash(t, a, p)
    }
    fn list_unverified_registrations(
        &self,
        c: &mut AnyConnection,
        p: &str,
        l: u32,
    ) -> RepoResult<Vec<LauncherAccountRecord>> {
        Stub.list_unverified_registrations(c, p, l)
    }
    fn delete_unverified_registration(&self, t: &mut Tx<'_>, a: &AccountId, p: &str) -> RepoResult<bool> {
        Stub.delete_unverified_registration(t, a, p)
    }
    fn list_for_admin(
        &self,
        _: &mut AnyConnection,
        f: &AdminAccountFilter,
    ) -> RepoResult<(Vec<LauncherAccountRecord>, u64)> {
        let world = self.world();
        let matching: Vec<_> = world
            .launchers
            .iter()
            .filter(|a| f.status.is_none_or(|s| a.status == s))
            .filter(|a| {
                f.search
                    .as_deref()
                    .is_none_or(|q| a.display_name.to_lowercase().contains(&q.to_lowercase()))
            })
            .cloned()
            .collect();
        let total = matching.len() as u64;
        Ok((
            matching
                .into_iter()
                .skip(f.skip as usize)
                .take(f.limit as usize)
                .collect(),
            total,
        ))
    }
    fn set_access_state(
        &self,
        _: &mut Tx<'_>,
        id: &AccountId,
        change: &AccessChange,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        let mut world = self.world();
        let Some(account) = world.launchers.iter_mut().find(|a| &a.account == id) else {
            return Ok(None);
        };
        if let Some(status) = change.status {
            account.status = status;
        }
        if let Some(approval) = change.approval_status {
            account.approval_status = Some(approval);
        }
        account.approval_updated_at = Some(change.approval_updated_at.clone());
        account.approval_updated_by = Some(change.approval_updated_by.clone());
        if change.approval_reason.is_some() {
            account.approval_reason = change.approval_reason.clone();
        }
        Ok(Some(account.clone()))
    }
    fn set_timeout(
        &self,
        _: &mut Tx<'_>,
        id: &AccountId,
        timeout: Option<&AccountTimeout>,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        let mut world = self.world();
        let Some(account) = world.launchers.iter_mut().find(|a| &a.account == id) else {
            return Ok(None);
        };
        account.timeout = timeout.cloned();
        Ok(Some(account.clone()))
    }
    fn erase(&self, t: &mut Tx<'_>, a: &AccountId) -> RepoResult<bool> {
        Stub.erase(t, a)
    }
    fn update_roles(
        &self,
        _: &mut Tx<'_>,
        id: &AccountId,
        roles: &[String],
        at: &str,
        by: &str,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        let mut world = self.world();
        let Some(account) = world.launchers.iter_mut().find(|a| &a.account == id) else {
            return Ok(None);
        };
        account.roles = roles.to_vec();
        account.roles_updated_at = Some(at.to_owned());
        account.roles_updated_by = Some(by.to_owned());
        Ok(Some(account.clone()))
    }
}

impl ApiKeyRepository for Fakes {
    fn find_all_game_server_key_hashes(&self, c: &mut AnyConnection) -> RepoResult<Vec<GameServerKeyHash>> {
        Stub.find_all_game_server_key_hashes(c)
    }
    fn insert_game_server_key_hash(&self, t: &mut Tx<'_>, h: &str) -> RepoResult<()> {
        Stub.insert_game_server_key_hash(t, h)
    }
    fn replace_game_server_key_hashes(&self, t: &mut Tx<'_>, h: &[String]) -> RepoResult<()> {
        Stub.replace_game_server_key_hashes(t, h)
    }
    fn find_all_user_key_hashes(&self, _: &mut AnyConnection) -> RepoResult<Vec<UserKeyHash>> {
        Ok(self.world().keys.clone())
    }
    fn insert_user_key_hash(&self, _: &mut Tx<'_>, account: &AccountId, key_hash: &str) -> RepoResult<()> {
        self.world().keys.push(UserKeyHash {
            account: account.clone(),
            key_hash: key_hash.to_owned(),
        });
        Ok(())
    }
    fn delete_user_key_hash(&self, t: &mut Tx<'_>, a: &AccountId) -> RepoResult<bool> {
        Stub.delete_user_key_hash(t, a)
    }
}

impl RefreshSessionRepository for Fakes {
    fn create(&self, t: &mut Tx<'_>, s: &RefreshSessionRecord) -> RepoResult<()> {
        RefreshSessionRepository::create(&Stub, t, s)
    }
    fn find_by_token_hash(&self, c: &mut AnyConnection, h: &str) -> RepoResult<Option<RefreshSessionRecord>> {
        Stub.find_by_token_hash(c, h)
    }
    fn revoke_by_id(&self, t: &mut Tx<'_>, id: &str, now: &str) -> RepoResult<()> {
        Stub.revoke_by_id(t, id, now)
    }
    fn revoke_family(&self, t: &mut Tx<'_>, f: &str, now: &str) -> RepoResult<()> {
        Stub.revoke_family(t, f, now)
    }
    fn revoke_all_for_account(&self, _: &mut Tx<'_>, account: &AccountId, _now: &str) -> RepoResult<()> {
        self.world().refresh_revoked.push(account.clone());
        Ok(())
    }
    fn delete_all_for_account(&self, _: &mut Tx<'_>, _: &AccountId) -> RepoResult<()> {
        Ok(())
    }
    fn is_family_active(&self, c: &mut AnyConnection, f: &str, a: &AccountId, now: &str) -> RepoResult<bool> {
        Stub.is_family_active(c, f, a, now)
    }
    fn purge_expired(&self, t: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        RefreshSessionRepository::purge_expired(&Stub, t, now)
    }
}

impl GameExchangeCodeRepository for Fakes {
    fn create(&self, t: &mut Tx<'_>, c: &GameExchangeCodeRecord) -> RepoResult<()> {
        GameExchangeCodeRepository::create(&Stub, t, c)
    }
    fn consume_by_code_hash(&self, t: &mut Tx<'_>, h: &str, now: &str) -> RepoResult<Option<GameExchangeCodeRecord>> {
        Stub.consume_by_code_hash(t, h, now)
    }
    fn revoke_unused_for_account(&self, _: &mut Tx<'_>, account: &AccountId, _now: &str) -> RepoResult<()> {
        self.world().codes_revoked.push(account.clone());
        Ok(())
    }
    fn delete_for_account(&self, t: &mut Tx<'_>, a: &AccountId) -> RepoResult<u64> {
        GameExchangeCodeRepository::delete_for_account(&Stub, t, a)
    }
    fn purge_expired(&self, t: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        GameExchangeCodeRepository::purge_expired(&Stub, t, now)
    }
}

/// A SQLite-backed store whose auth tables are `fakes`.
pub fn store(dir: &tempfile::TempDir, fakes: &Fakes) -> IslesStore {
    let config = store::DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    let opened = IslesStore::open(&config, &store::DatabaseOptions::default(), home, &node).unwrap();
    let mut repos: Repositories = opened.repos().clone();
    repos.accounts = Arc::new(fakes.clone());
    repos.launcher_accounts = Arc::new(fakes.clone());
    repos.api_keys = Arc::new(fakes.clone());
    repos.refresh_sessions = Arc::new(fakes.clone());
    repos.game_exchange_codes = Arc::new(fakes.clone());
    opened.with_repos(repos)
}

pub fn shared(routes: RouteSet, pairs: &[(&str, &str)], store: Option<IslesStore>) -> Shared {
    let mut shared = Shared::bare(version(routes));
    shared.settings = Settings::from_pairs(pairs.iter().copied());
    shared.store = store;
    shared.admin_api = shared.settings.admin_api().unwrap();
    shared
}

/// Serves the backend of `shared` with every area's handlers; the address it listens on.
pub async fn serve(shared: Shared) -> SocketAddr {
    let router = app(Arc::new(shared), Service::Backend, &isles_core::areas::backend()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        service_kit::serve::serve(listener, router, std::future::pending(), Duration::from_secs(1)).await
    });
    address
}

pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// One HTTP/1.1 request on a fresh connection.
pub async fn send(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<(&str, &[u8])>,
) -> Answer {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let payload = body.map_or(&[][..], |(_, bytes)| bytes);
    if let Some((content_type, _)) = body {
        head.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", payload.len()));
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(payload).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect::<Vec<_>>();
    let mut body = raw[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.contains("chunked"))
    {
        body = unchunk(&body);
    }
    Answer { status, headers, body }
}

fn unchunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n").unwrap();
        let size = usize::from_str_radix(std::str::from_utf8(&data[..line_end]).unwrap().trim(), 16).unwrap();
        data = &data[line_end + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

pub async fn get(address: SocketAddr, path: &str, headers: &[(&str, &str)]) -> Answer {
    send(address, "GET", path, headers, None).await
}

pub async fn post_json(address: SocketAddr, path: &str, headers: &[(&str, &str)], body: &str) -> Answer {
    send(
        address,
        "POST",
        path,
        headers,
        Some(("application/json", body.as_bytes())),
    )
    .await
}

pub fn unimplemented_error(error: &RepoError) -> bool {
    matches!(error, RepoError::Unimplemented { .. })
}
