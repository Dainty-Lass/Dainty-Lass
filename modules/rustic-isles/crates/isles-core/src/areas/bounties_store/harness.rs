//! A server for the version modules' tests of this area: the real pipeline and handlers over a real `store` database,
//! with the seams other areas fill (authentication, purchase tokens, the first character, the inventory transaction)
//! replaced by small fakes. Not for production code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, BoxFuture, NodeIdentity};
use serde_json::{Value, json};
pub use store::DatabaseConfig;
use store::{DatabaseOptions, PostgresConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::authn::{
    AuthData, DailyClaim, FirstCharacter, InventoryTransactions, PurchaseTokenClaims, PurchaseTokens, StoreSeams,
    StoreTransaction, TransactionFailure,
};
use super::entitlements::grant_entitlements_in_transaction;
use super::wallet::{WalletError, add_currency, spend_platinum};
use crate::db::IslesStore;
use crate::game_data::GameDataRoot;
use crate::http::{Shared, app};
use crate::ids::{AccountId, UserId};
use crate::repo::auth::AccountRecord;
use crate::routes::Service;
use crate::settings::Settings;
use crate::version::IslesVersion;

/// `x-test-auth`: `gs` (a game server), `player:<userId>`, `host:<userId>` (a player-host runtime whose roster is
/// `player-1` and `player-2`); absent is no credential.
pub const AUTH_HEADER: &str = "x-test-auth";

/// `RunInventoryTransaction`'s contract as far as the store needs it: a ledger keyed by (user, character, transaction)
/// that applies once, replays an identical request and refuses a different one; the wallet through this area's own
/// functions, entitlements granted in the same transaction.
/// (user, character, transaction) to (request hash, result)
type Ledger = HashMap<(String, String, String), (String, Value)>;

pub struct FakeInventory {
    store: IslesStore,
    ledger: Mutex<Ledger>,
    pub calls: Mutex<Vec<StoreTransaction>>,
    /// Makes the ledger lookup fail, as a database outage would
    pub broken_ledger: Mutex<Option<()>>,
}

impl InventoryTransactions for FakeInventory {
    fn run(&self, transaction: StoreTransaction) -> BoxFuture<'_, Result<Value, TransactionFailure>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(transaction.clone());
            let key = (
                transaction.user_id.clone(),
                transaction.character_id.clone(),
                transaction.transaction_id.clone(),
            );
            let hash = json!([
                transaction.instanced_to_add,
                transaction.stacked_to_add,
                transaction.stacked_to_remove
            ])
            .to_string();
            if let Some((stored_hash, result)) = self.ledger.lock().unwrap().get(&key) {
                return if *stored_hash == hash {
                    Ok(result.clone())
                } else {
                    Err(TransactionFailure::Mismatch)
                };
            }
            let account = transaction.account.clone();
            let t = transaction.clone();
            let actor = ActorContext::new(Actor::System);
            let applied = self
                .store
                .account_write(&account, &actor, move |tx, repos| {
                    let mut touched = serde_json::Map::new();
                    for item in &t.stacked_to_add {
                        let id = item["catalogId"].as_str().unwrap_or_default();
                        if id.starts_with("CURRENCY_") {
                            let next =
                                add_currency(repos, tx, &t.account, id, item["quantity"].as_f64().unwrap_or(0.0));
                            match next {
                                Ok(next) => touched.insert(id.to_owned(), json!(next)),
                                Err(e) => return Ok(Err(e)),
                            };
                        }
                    }
                    for item in &t.stacked_to_remove {
                        let id = item["catalogId"].as_str().unwrap_or_default();
                        let quantity = item["quantity"].as_f64().unwrap_or(0.0);
                        let spent = if id == "CURRENCY_PLATINUM" {
                            spend_platinum(repos, tx, &t.account, quantity).map(|_| ())
                        } else {
                            add_currency(repos, tx, &t.account, id, -quantity).map(|_| ())
                        };
                        if let Err(e) = spent {
                            return Ok(Err(e));
                        }
                    }
                    grant_entitlements_in_transaction(repos, tx, &t.account, &t.grants.entitlements, Some(&t.sku_id))?;
                    Ok(Ok(Value::Object(touched)))
                })
                .await;
            match applied {
                Ok(Ok(result)) => {
                    let result = json!({ "updatedStackedItems": result });
                    self.ledger.lock().unwrap().insert(key, (hash, result.clone()));
                    Ok(result)
                }
                Ok(Err(WalletError::Insufficient { .. })) => Err(TransactionFailure::InsufficientBalance),
                Ok(Err(other)) => Err(TransactionFailure::Failed(other.to_string())),
                Err(error) => Err(TransactionFailure::Failed(error.to_string())),
            }
        })
    }

    fn find_claim(&self, _: AccountId, transaction_id: String) -> BoxFuture<'_, Result<Option<DailyClaim>, String>> {
        Box::pin(async move {
            if self.broken_ledger.lock().unwrap().is_some() {
                return Err("ledger unavailable".into());
            }
            let ledger = self.ledger.lock().unwrap();
            Ok(ledger
                .iter()
                .find(|((_, _, id), _)| *id == transaction_id)
                .map(|((_, character, _), _)| DailyClaim {
                    character_id: Some(character.clone()),
                }))
        })
    }
}

struct FakeTokens;

impl PurchaseTokens for FakeTokens {
    fn sign(&self, claims: &PurchaseTokenClaims) -> Result<String, String> {
        let body = json!({ "userId": claims.user_id, "characterId": claims.character_id, "skuId": claims.sku_id,
                "storeTag": claims.store_tag })
        .to_string();
        // hex, since a real token is URL-safe
        Ok(format!(
            "tok.{}",
            body.bytes().map(|b| format!("{b:02x}")).collect::<String>()
        ))
    }

    fn validate(&self, token: &str) -> Result<PurchaseTokenClaims, String> {
        let hex = token.strip_prefix("tok.").ok_or("not a token")?;
        let bytes = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2).ok_or("odd")?, 16).map_err(|e| e.to_string()))
            .collect::<Result<Vec<u8>, String>>()?;
        let body: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let text = |k: &str| body[k].as_str().map(str::to_owned);
        Ok(PurchaseTokenClaims {
            user_id: text("userId").ok_or("no user")?,
            character_id: text("characterId").ok_or("no character")?,
            sku_id: text("skuId").ok_or("no sku")?,
            store_tag: text("storeTag"),
        })
    }
}

struct FakeCharacters;

impl FirstCharacter for FakeCharacters {
    fn first_character_id(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move { Ok(Some(format!("char-of-{}", &account.to_string()[8..16]))) })
    }
}

pub struct Server {
    pub address: SocketAddr,
    pub store: IslesStore,
    pub inventory: Arc<FakeInventory>,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
}

fn auth_from(value: &str) -> Option<AuthData> {
    match value.split_once(':') {
        None if value == "gs" => Some(AuthData::gameserver()),
        Some(("player", user)) => Some(AuthData::player(user)),
        Some(("host", user)) => {
            let mut auth = AuthData::player(user);
            auth.host_runtime = Some(rifts_seam::RuntimeAuth {
                capability_id: "capability".into(),
                session: "01ARZ3NDEKTSV4RRFFQ69G5FAV@standalone".parse().ok()?,
                generation: 1,
                host_account: rifts_seam::AccountId("host".into()),
                scopes: vec![],
                roster: vec![
                    rifts_seam::AccountId("player-1".into()),
                    rifts_seam::AccountId("player-2".into()),
                ],
                host_peer: None,
                bridge_instance: None,
                wire_session: None,
            });
            Some(auth)
        }
        _ => None,
    }
}

/// Starts the version's backend routes on an ephemeral port over a database at `target`.
pub async fn start(
    version: IslesVersion,
    target: &DatabaseConfig,
    game_data: Option<&Path>,
    settings: &[(&str, &str)],
) -> Server {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    let store = IslesStore::open(target, &DatabaseOptions::default(), home, &node).unwrap();
    let mut shared = Shared::bare(version);
    shared.settings = Settings::from_pairs(settings.iter().copied());
    shared.game_data = GameDataRoot {
        backend: game_data.map(Path::to_path_buf),
        director: None,
    };
    shared.store = Some(store.clone());
    let inventory = Arc::new(FakeInventory {
        store: store.clone(),
        ledger: Mutex::default(),
        calls: Mutex::default(),
        broken_ledger: Mutex::default(),
    });
    let seams = StoreSeams {
        tokens: Arc::new(FakeTokens),
        characters: Arc::new(FakeCharacters),
        inventory: inventory.clone(),
    };
    let router = app(Arc::new(shared), Service::Backend, &crate::areas::backend()).unwrap();
    let router = router.layer(axum::middleware::from_fn(move |mut request: Request, next: Next| {
        let seams = seams.clone();
        async move {
            let auth = request
                .headers()
                .get(AUTH_HEADER)
                .and_then(|v| v.to_str().ok())
                .and_then(auth_from);
            if let Some(auth) = auth {
                request.extensions_mut().insert(auth);
            }
            request.extensions_mut().insert(seams);
            let response: Response = next.run(request).await;
            response
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    Server {
        address,
        store,
        inventory,
    }
}

/// Makes the account a game would have logged in with.
pub async fn make_account(store: &IslesStore, user_id: &str) -> AccountId {
    let record = AccountRecord {
        id: AccountId::new(&NodeIdentity::standalone().node),
        user_id: UserId(user_id.to_owned()),
        name: user_id.to_owned(),
        notes: 0,
    };
    let id = record.id.clone();
    store
        .account_write(&id, &ActorContext::new(Actor::System), move |tx, repos| {
            repos.accounts.create(tx, &record)
        })
        .await
        .unwrap();
    id
}

impl Server {
    /// One HTTP/1.1 request on a fresh connection.
    pub async fn call(&self, method: &str, path: &str, auth: Option<&str>, body: Option<&str>) -> Reply {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.address
        );
        if let Some(auth) = auth {
            head.push_str(&format!("{AUTH_HEADER}: {auth}\r\n"));
        }
        let payload = body.unwrap_or_default();
        if body.is_some() {
            head.push_str("Content-Type: application/json\r\n");
        }
        head.push_str(&format!("Content-Length: {}\r\n\r\n{payload}", payload.len()));
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        // Chunked bodies are decoded; Content-Length bodies are as they are
        let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
        let body = if chunked { dechunk(body) } else { body.to_owned() };
        Reply { status, headers, body }
    }

    pub async fn get(&self, path: &str, auth: Option<&str>) -> Reply {
        self.call("GET", path, auth, None).await
    }

    pub async fn send_json(&self, method: &str, path: &str, auth: Option<&str>, body: &Value) -> Reply {
        self.call(method, path, auth, Some(&body.to_string())).await
    }
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.trim(), 16) else {
            break;
        };
        if n == 0 || tail.len() < n {
            break;
        }
        out.push_str(&tail[..n]);
        rest = tail[n..].trim_start_matches("\r\n");
    }
    out
}

/// A throwaway Postgres database on the server `REFORGED_TEST_POSTGRES_URL` names
/// (`postgres://user:password@127.0.0.1:5432/postgres`); dropped with the guard. `None` when the variable is unset.
pub struct PgDatabase {
    server: PostgresConfig,
    pub config: DatabaseConfig,
}

pub fn postgres_target() -> Option<PgDatabase> {
    use diesel::Connection;
    use diesel::connection::SimpleConnection;
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))?;
    let (credentials, location) = rest.rsplit_once('@')?;
    let (user, password) = credentials.split_once(':').unwrap_or((credentials, ""));
    let (host_port, database) = location.split_once('/').unwrap_or((location, "postgres"));
    let (host, port) = host_port.split_once(':').unwrap_or((host_port, "5432"));
    let server = PostgresConfig {
        host: host.to_owned(),
        port: port.parse().ok()?,
        user: user.to_owned(),
        database: database.to_owned(),
        password: password.to_owned(),
        allow_remote: false,
    };
    let name = format!(
        "isles_test_{}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    );
    diesel::PgConnection::establish(&server.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .unwrap();
    let config = DatabaseConfig::Postgres(PostgresConfig {
        database: name,
        ..server.clone()
    });
    Some(PgDatabase { server, config })
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        use diesel::Connection;
        use diesel::connection::SimpleConnection;
        if let (DatabaseConfig::Postgres(pg), Ok(mut admin)) =
            (&self.config, diesel::PgConnection::establish(&self.server.conninfo()))
        {
            let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", pg.database));
        }
    }
}

/// Synthetic game data for the store: one Lady Luck offer (`ll_sword`, 50 Steel Marks, instanced), a repeatable one
/// (`ll_tonic`, 5 Steel Marks, stacked), a Reward Cache offer, two Platinum offers (a dye in `webstore` and `dyes`, a
/// 100 Platinum cosmetic), and the fountain's daily bundle (`fountain_bundle`, 500 Rams).
pub fn write_game_data(dir: &Path) {
    let offer =
        |id: &str, tag: &str, currency: &str, price: u32, max: Value, item: &str, quantity: u32, instanced: bool| {
            json!({ "id": id, "displayName": id, "displayDescription": "d", "displayPriority": 1,
            "prices": [{ "currencyId": currency, "price": price }], "maxAllowed": max, "images": {},
            "tags": [tag], "items": [{ "catalogId": item, "quantity": quantity, "instanced": instanced }],
            "duplicateInstancedItems": [] })
        };
    let write = |name: &str, value: Value| std::fs::write(dir.join(name), value.to_string()).unwrap();
    write(
        "ladyluck_store.json",
        json!([
            offer(
                "ll_sword",
                "ladyluckstore",
                "id_currency_marks_steel",
                50,
                json!(1),
                "WP_SWORD",
                1,
                true
            ),
            offer(
                "ll_tonic",
                "ladyluckstore",
                "id_currency_marks_steel",
                5,
                Value::Null,
                "TONIC",
                2,
                false
            ),
        ]),
    );
    write(
        "reward_cache_store.json",
        json!([offer(
            "rc_hat",
            "season_store",
            "id_currency_seasonal_coin",
            40,
            json!(1),
            "AC_HAT",
            1,
            true
        )]),
    );
    let mut dye = offer(
        "dye_red",
        "webstore",
        "id_currency_platinum",
        100,
        json!(1),
        "DYE_RED",
        1,
        false,
    );
    dye["tags"] = json!(["webstore", "dyes"]);
    write(
        "platinum_store.json",
        json!([
            dye,
            offer(
                "plat_cape",
                "webstore",
                "id_currency_platinum",
                100,
                json!(1),
                "AC_CAPE",
                1,
                true
            )
        ]),
    );
    write(
        "fountain_daily_bundle.json",
        json!({ "skus": [offer("fountain_bundle", "fountain_daily_free_bundle", "id_currency_platinum", 0, json!(1),
            "CURRENCY_NOTES", 500, false)] }),
    );
    write(
        "inventory_storage_policy.json",
        json!({ "schema": 1, "entries": {
            "CURRENCY_NOTES": { "storage": "stacked", "maxQuantity": 0, "tags": ["Currency"] } } }),
    );
    write("progression_config.json", json!({ "payload": { "paths": [] } }));
}

/// Replays `steps` of a golden file (`tests/golden/bounties_store/<version>.json`, recorded from Node's own routers) and
/// returns what differs: status, content type and body text, exactly. `{token}` in a path is the purchase token the last
/// `/token` answered; a token response is compared as `{"purchaseToken":"<token>"}`. Before the first token step the
/// first account gets 100 Steel Marks, the wallet Node's fake inventory pretends to hold.
pub async fn replay_golden(server: &Server, golden: &str) -> Vec<String> {
    let golden: Value = serde_json::from_str(golden).unwrap();
    let mut last_token = String::from("none");
    let mut credited = false;
    let mut diffs = Vec::new();
    for step in golden["steps"].as_array().unwrap() {
        let name = step["name"].as_str().unwrap();
        let method = step["method"].as_str().unwrap();
        let path = step["path"].as_str().unwrap().replace("{token}", &last_token);
        let auth = step["auth"].as_str();
        let body = step["body"].as_str();
        if !credited && path.starts_with("/token/") {
            credited = true;
            let account = make_account_if_missing(&server.store, "p1").await;
            let id = account.clone();
            server
                .store
                .account_write(&account, &ActorContext::new(Actor::System), move |tx, repos| {
                    add_currency(repos, tx, &id, "CURRENCY_MARKS_STEEL", 100.0)
                        .map(|_| ())
                        .map_err(|e| crate::repo::RepoError::Data(e.to_string()))
                })
                .await
                .unwrap();
        }
        let reply = server.call(method, &path, auth, body).await;
        let mut text = reply.body.clone();
        if path.starts_with("/token/") && reply.status == 200 {
            last_token = reply.json()["purchaseToken"].as_str().unwrap().to_owned();
            text = r#"{"purchaseToken":"<token>"}"#.to_owned();
        }
        let content_type = reply
            .headers
            .iter()
            .find(|(n, _)| n == "content-type")
            .map(|(_, v)| v.as_str());
        let want_status = step["status"].as_u64().unwrap();
        let want_type = step["contentType"].as_str();
        let want_text = step["text"].as_str().unwrap();
        // Express's error page names the parser's message; only its status and type are the contract here
        let page = want_text.starts_with("<!DOCTYPE html>");
        if u64::from(reply.status) != want_status
            || content_type != want_type
            || (!page && text != want_text)
            || (page && !text.starts_with("<!DOCTYPE html>"))
        {
            diffs.push(format!(
                "{name}: {} {content_type:?} {text:?}\n   wanted {want_status} {want_type:?} {want_text:?}",
                reply.status
            ));
        }
    }
    diffs
}

async fn make_account_if_missing(store: &IslesStore, user: &str) -> AccountId {
    let user_id = UserId(user.to_owned());
    match store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
        .await
        .unwrap()
    {
        Some(found) => found.id,
        None => make_account(store, user).await,
    }
}

/// The pinned clock of the golden files: 2026-10-26T12:00:00Z.
pub fn pin_golden_clock() {
    crate::game_data::season_window::pin_now_for_tests(Some(1_793_016_000_000));
}
