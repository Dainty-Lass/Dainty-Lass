//! What every replay shares: the composed MIT module started in process on a temporary SQLite file with its
//! listeners on 127.0.0.1:0, restricted's version data, game tokens signed with the run's keys, a raw HTTP client,
//! and the per-suite tally written to `target/tmp/mit-parity/<suite>.json`.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test helpers

pub mod normalise;
pub mod reward_cache;
pub mod starter;
pub mod version_data;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use dainty_module_api::stub::RecordingHost;
use dainty_module_api::{
    BoxFuture, GameModule, ModuleConfig, ModuleContext, ModuleError, ModuleId, NodeIdentity, PortRange,
};
use rustic_isles::{ComposeInputs, Composer, Composition, IslesModule};
use rustic_isles_auth::test_support::{RecordingMail, StubDiscord};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The game server key every run registers (restricted's auth golden uses this one).
pub const GAMESERVER_KEY: &str = "gs-key-1";
pub const API_KEY_HASH_SECRET: &str = "test-only-api-key-hash-secret-0123456789abcdef";

/// The version composer with restricted's test seams in place of the outside world: a recording mail and the stub
/// Discord (restricted's auth harness records mail and answers Discord the same way).
struct WithFakes {
    inner: Arc<dyn Composer>,
    mail: Arc<RecordingMail>,
}

impl Composer for WithFakes {
    fn compose(&self, inputs: ComposeInputs) -> BoxFuture<'_, Result<Composition, ModuleError>> {
        Box::pin(async move {
            let mut composition = self.inner.compose(inputs).await?;
            composition.auth_seams.mail = self.mail.clone();
            composition.auth_seams.discord = Arc::new(StubDiscord);
            Ok(composition)
        })
    }
}

fn quote(value: &str) -> String {
    if !value.contains('\'') {
        format!("'{value}'")
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// One running module and the folders it owns.
pub struct Mit {
    pub dir: tempfile::TempDir,
    pub db: PathBuf,
    pub game_data: PathBuf,
    pub mail: Arc<RecordingMail>,
    pub backend: SocketAddr,
    pub listening: Vec<(String, String)>,
    module: Option<IslesModule>,
    host: Arc<RecordingHost>,
}

/// The settings every run starts from; a suite's own win.
pub fn base_settings(db: &Path) -> BTreeMap<String, String> {
    let (private, public) = rustic_isles_auth::test_support::throwaway_signing_keys();
    let mut settings = BTreeMap::new();
    for (key, value) in [
        ("DAINTY_DB_FILE", db.display().to_string()),
        ("API_KEY_HASH_SECRET", API_KEY_HASH_SECRET.to_owned()),
        ("AUTH_SIGNING_PRIVKEY_B64", private),
        ("AUTH_SIGNING_PUBKEY_B64", public),
        ("GAMESERVER_API_KEYS", GAMESERVER_KEY.to_owned()),
        // Core §17 item 1: the replay headers count only when a host turns them on
        ("RUSTIC_ISLES_REPLAY_HEADERS", "on".to_owned()),
        // Restricted's harnesses run dedicated-only (`Rifts::dedicated_only()`)
        ("RUSTIC_RIFTS_P2P_MODULE", "off".to_owned()),
        // Many requests from one loopback peer; the request-budget suite turns it back on
        ("RUSTIC_ISLES_RATE_LIMIT", "off".to_owned()),
    ] {
        settings.insert(key.to_owned(), value);
    }
    settings
}

fn context(host: &Arc<RecordingHost>) -> ModuleContext {
    ModuleContext::new(
        ModuleId::new("isles-1-14-7").unwrap(),
        NodeIdentity::standalone(),
        host.clone(),
    )
}

fn config(module: &IslesModule, env: &Path, backend_dir: &Path) -> ModuleConfig {
    let mut registrar = dainty_module_api::Registrar::new(module.manifest());
    module.register(&mut registrar).unwrap();
    let registration = registrar.finish().unwrap();
    let mut config = ModuleConfig::defaults(&module.manifest(), &registration);
    for range in config.ports.values_mut() {
        *range = PortRange::single(0);
    }
    let mut set = |section: &str, key: &str, value: String| {
        config
            .sections
            .entry(section.to_owned())
            .or_default()
            .insert(key.to_owned(), value.into());
    };
    set("backend", "env_file", env.display().to_string());
    set("game_data", "backend_dir", backend_dir.display().to_string());
    config
}

impl Mit {
    /// A fresh database and game data folder; `files` fills the folder before the module reads it.
    pub async fn start(settings: &[(&str, &str)], files: impl FnOnce(&Path)) -> Self {
        match Self::try_start(settings, files).await {
            Ok(mit) => mit,
            Err(error) => panic!("the module did not start: {error}"),
        }
    }

    /// As `start`, a failed start given back (some golden cases set values the module refuses at start).
    pub async fn try_start(settings: &[(&str, &str)], files: impl FnOnce(&Path)) -> Result<Self, String> {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("isles.sqlite");
        let game_data = dir.path().join("game-data");
        std::fs::create_dir_all(&game_data).unwrap();
        files(&game_data);
        let mut mit = Self {
            dir,
            db,
            game_data,
            mail: Arc::new(RecordingMail::default()),
            backend: "127.0.0.1:1".parse().unwrap(),
            listening: Vec::new(),
            module: None,
            host: dainty_module_api::stub::recording_host(),
        };
        mit.boot(settings).await?;
        Ok(mit)
    }

    /// The same database and folders under other settings, as a restart with a changed env file is.
    pub async fn restart(&mut self, settings: &[(&str, &str)]) {
        self.stop().await;
        self.boot(settings).await.unwrap();
    }

    pub async fn try_restart(&mut self, settings: &[(&str, &str)]) -> Result<(), String> {
        self.stop().await;
        self.boot(settings).await
    }

    async fn boot(&mut self, settings: &[(&str, &str)]) -> Result<(), String> {
        let mut all = base_settings(&self.db);
        for (key, value) in settings {
            all.insert((*key).to_owned(), (*value).to_owned());
        }
        let env = self.dir.path().join("backend.env");
        let lines: Vec<String> = all.iter().map(|(k, v)| format!("{k}={}", quote(v))).collect();
        std::fs::write(&env, lines.join("\n")).unwrap();
        let mut spec = rustic_isles_1_14_7::spec_with(version_data::restricted()).unwrap();
        spec.composer = Arc::new(WithFakes {
            inner: spec.composer.clone(),
            mail: self.mail.clone(),
        });
        let module = IslesModule::new(spec).unwrap();
        self.host = dainty_module_api::stub::recording_host();
        module
            .start(context(&self.host), config(&module, &env, &self.game_data))
            .await
            .map_err(|e| e.message)?;
        self.listening = rustic_isles::testing::listening(&self.host);
        self.backend = self
            .listening
            .iter()
            .find(|(name, _)| name == "backend")
            .ok_or("no backend listener")?
            .1
            .parse()
            .map_err(|e| format!("{e}"))?;
        self.module = Some(module);
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(module) = self.module.take() {
            let _ = module.stop(context(&self.host)).await;
        }
    }

    /// The MIT store on the module's database, for seeding rows the way restricted's harnesses write them.
    pub fn store(&self) -> rustic_isles_store::IslesStore {
        let path = self.db.display().to_string();
        let home = Arc::new(dainty_module_api::network::StandaloneHome {
            identity: NodeIdentity::standalone(),
        });
        rustic_isles_store::IslesStore::open_configured(
            |name| (name == "DAINTY_DB_FILE").then(|| path.clone()),
            home,
            &NodeIdentity::standalone(),
        )
        .unwrap()
        .unwrap()
    }
}

// ---- tokens

fn pem_der(b64_pem: &str) -> Vec<u8> {
    let pem = String::from_utf8(STANDARD.decode(b64_pem).unwrap()).unwrap();
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    STANDARD.decode(body).unwrap()
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// A game token for `user_id` (auth §5.1's claims, RS256 with the run's key; validated at the system clock).
pub fn game_token(user_id: &str) -> String {
    let (private, _) = rustic_isles_auth::test_support::throwaway_signing_keys();
    let pair = aws_lc_rs::rsa::KeyPair::from_pkcs8(&pem_der(&private)).unwrap();
    let now = now_s();
    let claims = json!({ "userId": user_id, "iat": now, "exp": now + 86_400, "aud": "paradox-backend",
        "iss": "paradox-backend" });
    let signing = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let mut signature = vec![0; pair.public_modulus_len()];
    pair.sign(
        &aws_lc_rs::signature::RSA_PKCS1_SHA256,
        &aws_lc_rs::rand::SystemRandom::new(),
        signing.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{signing}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// The real credentials for a harness's stand-in caller: a player's bearer token, or a game server key with
/// (`Some`) or without (`None`) a player's bearer.
pub enum Caller<'a> {
    Nobody,
    Player(&'a str),
    Gameserver(Option<&'a str>),
}

pub fn credentials(caller: Caller<'_>) -> Vec<(String, String)> {
    match caller {
        Caller::Nobody => vec![],
        Caller::Player(user) => vec![("authorization".into(), format!("Bearer {}", game_token(user)))],
        Caller::Gameserver(user) => {
            let mut headers = vec![("x-rustic-isles-gameserver-apikey".into(), GAMESERVER_KEY.to_owned())];
            if let Some(user) = user {
                headers.push(("authorization".into(), format!("Bearer {}", game_token(user))));
            }
            headers
        }
    }
}

// ---- HTTP

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

pub fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&rest[..end]).unwrap_or("0");
        let size = size.split(';').next().unwrap_or("0").trim();
        let Ok(n) = usize::from_str_radix(size, 16) else {
            break;
        };
        rest = &rest[end + 2..];
        if n == 0 || rest.len() < n {
            break;
        }
        out.extend_from_slice(&rest[..n]);
        rest = rest.get(n + 2..).unwrap_or_default();
    }
    out
}

/// One HTTP/1.1 request on a fresh connection; `body` is sent with a content length (none when `None`).
pub async fn send(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Reply {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = body {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    if let Some(body) = body {
        stream.write_all(body).await.unwrap();
    }
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw).await;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let rest = raw.get(split + 4..).unwrap_or_default();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    let body = if chunked { dechunk(rest) } else { rest.to_vec() };
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

// ---- the tally

/// What one suite found: every step counted, every mismatch kept with both normalised answers.
pub struct Tally {
    pub suite: String,
    pub total: usize,
    pub mismatches: Vec<Value>,
    pub skipped: Vec<Value>,
    pub notes: Vec<String>,
    /// The actual statuses, so a run of matching refusals stands out
    pub statuses: BTreeMap<String, usize>,
}

impl Tally {
    pub fn new(suite: &str) -> Self {
        Self {
            suite: suite.to_owned(),
            total: 0,
            mismatches: Vec::new(),
            skipped: Vec::new(),
            notes: Vec::new(),
            statuses: BTreeMap::new(),
        }
    }

    /// Counts a step; `expected` and `actual` are the normalised answers (`status`, `headers`, `body`).
    pub fn step(&mut self, number: usize, name: &str, method: &str, path: &str, expected: Value, actual: Value) {
        self.total += 1;
        let status = actual.get("status").map_or("-".to_owned(), Value::to_string);
        *self.statuses.entry(status).or_default() += 1;
        if expected != actual {
            self.mismatches.push(json!({
                "step": number, "name": name, "method": method, "path": path,
                "expected": expected, "actual": actual,
            }));
        }
    }

    /// A step the composed module can't be driven through as the golden was recorded; not counted.
    pub fn skip(&mut self, number: usize, name: &str, method: &str, path: &str, why: &str, actual: Value) {
        self.skipped.push(
            json!({ "step": number, "name": name, "method": method, "path": path, "why": why,
            "actual": actual }),
        );
    }

    pub fn note(&mut self, text: impl Into<String>) {
        self.notes.push(text.into());
    }

    /// Writes the tally and prints a summary; never fails the test, so every suite runs to the end.
    pub fn finish(self) {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mit-parity");
        std::fs::create_dir_all(&dir).unwrap();
        let matched = self.total - self.mismatches.len();
        let out = json!({
            "suite": self.suite, "total": self.total, "matched": matched,
            "mismatched": self.mismatches.len(), "notes": self.notes, "statuses": self.statuses,
            "skipped": self.skipped,
            "mismatches": self.mismatches,
        });
        std::fs::write(
            dir.join(format!("{}.json", self.suite)),
            serde_json::to_string_pretty(&out).unwrap(),
        )
        .unwrap();
        eprintln!(
            "[{}] {} steps, {} matched, {} mismatched",
            self.suite,
            self.total,
            matched,
            self.mismatches.len()
        );
        for m in out["mismatches"].as_array().unwrap() {
            eprintln!(
                "  #{} {} {} {}\n    expected {}\n    actual   {}",
                m["step"], m["name"], m["method"], m["path"], m["expected"], m["actual"]
            );
        }
    }
}

// ---- seeding

pub fn system() -> dainty_module_api::ActorContext {
    dainty_module_api::ActorContext::new(dainty_module_api::Actor::System)
}

/// A game account as a game login makes it (`userId` and name), through the MIT store as the System.
pub async fn seed_account(store: &rustic_isles_store::IslesStore, user: &str, name: &str) -> rustic_isles::AccountId {
    let id = rustic_isles::AccountId::new(&NodeIdentity::standalone().node);
    let record = rustic_isles_store::repo::auth::AccountRecord {
        id: id.clone(),
        user_id: rustic_isles::UserId::new(user),
        name: name.to_owned(),
        notes: 0,
    };
    store
        .account_write(&id, &system(), move |tx, repos| repos.accounts.create(tx, &record))
        .await
        .unwrap();
    id
}

/// A character row (and, with `items`, its inventory: instanced and stacked JSON texts at revision 0).
pub async fn seed_character(
    store: &rustic_isles_store::IslesStore,
    account: &rustic_isles::AccountId,
    character_id: &str,
    items: Option<(&str, &str)>,
) {
    let row = rustic_isles_store::repo::characters::CharacterRecord {
        character_id: character_id.to_owned(),
        account: account.clone(),
        created_date: "2026-10-01T00:00:00.000Z".into(),
        last_modified_date: "2026-10-01T00:00:00.000Z".into(),
        name: format!("Hunter {character_id}"),
        update_version: 1,
        data: "{}".into(),
    };
    store
        .account_write(account, &system(), move |tx, repos| {
            repos.characters.create(tx, &row).map(|_| ())
        })
        .await
        .unwrap();
    if let Some((instanced, stacked)) = items {
        let inventory = rustic_isles_store::repo::inventory::InventoryRecord {
            character_id: character_id.to_owned(),
            account: Some(account.clone()),
            instanced_items: instanced.to_owned(),
            stacked_items: stacked.to_owned(),
            revision: Some(0),
            bootstrap_version: Some("golden".into()),
        };
        store
            .account_write(account, &system(), move |tx, repos| {
                repos.inventories.create(tx, &inventory)
            })
            .await
            .unwrap();
    }
}

/// Adds `amount` of a currency to the account's wallet (made empty first when there is none).
pub async fn credit(
    store: &rustic_isles_store::IslesStore,
    account: &rustic_isles::AccountId,
    currency: &str,
    amount: f64,
) {
    let (id, currency) = (account.clone(), currency.to_owned());
    store
        .account_write(account, &system(), move |tx, repos| {
            let empty = rustic_isles_store::repo::bounties::WalletRecord {
                account: id.clone(),
                balances: Vec::new(),
                bootstrap_version: None,
            };
            repos.wallets.create_if_missing(tx, &empty)?;
            repos.wallets.increment_balance(tx, &id, &currency, amount).map(|_| ())
        })
        .await
        .unwrap();
}

/// A wallet row with exactly these balances and no bootstrap version (Node's `wallets.createIfMissing` in a recording).
pub async fn seed_wallet(
    store: &rustic_isles_store::IslesStore,
    account: &rustic_isles::AccountId,
    balances: Vec<(String, f64)>,
) {
    let wallet = rustic_isles_store::repo::bounties::WalletRecord {
        account: account.clone(),
        balances,
        bootstrap_version: None,
    };
    store
        .account_write(account, &system(), move |tx, repos| {
            repos.wallets.create_if_missing(tx, &wallet).map(|_| ())
        })
        .await
        .unwrap();
}

pub async fn seed_track(
    store: &rustic_isles_store::IslesStore,
    account: &rustic_isles::AccountId,
    track: &str,
    amount: i64,
) {
    let (id, track) = (account.clone(), track.to_owned());
    store
        .account_write(account, &system(), move |tx, repos| {
            repos.progression_tracks.increment(tx, &id, &track, amount).map(|_| ())
        })
        .await
        .unwrap();
}

pub async fn find_account(store: &rustic_isles_store::IslesStore, user: &str) -> Option<rustic_isles::AccountId> {
    let user = rustic_isles::UserId::new(user);
    store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await
        .unwrap()
        .map(|record| record.id)
}
