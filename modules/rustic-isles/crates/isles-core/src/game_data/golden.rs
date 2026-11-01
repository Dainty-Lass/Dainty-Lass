//! The golden comparison for the game-data group, for the module crates' tests (not for production code): each case of
//! `tests/golden/game_data.json` was answered by the reference Node handlers under Node 22 (`capture.mjs`), and is
//! requested here over real HTTP from the version's Rust module. Status, content type and body text must be equal.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::season_window::pin_now_for_tests;
use crate::game_data::GameDataRoot;
use crate::http::{Shared, app};
use crate::routes::Service;
use crate::settings::Settings;
use crate::version::IslesVersion;

struct Reply {
    status: u16,
    content_type: Option<String>,
    body: String,
}

async fn exchange(
    address: SocketAddr,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> std::io::Result<Reply> {
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    let mut stream = TcpStream::connect(address).await?;
    stream.write_all(head.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
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
    Ok(Reply {
        status,
        content_type,
        body: body.to_owned(),
    })
}

/// A game token and a game server key the module under test accepts (the auth area's middleware runs ahead of these
/// routes): `serve` registers both.
const GAME_SERVER_KEY: &str = "k";

fn bearer(settings: &Settings) -> String {
    let token = crate::areas::auth::tokens::sign_game_token(settings, "golden-user").unwrap_or_default();
    format!("Bearer {token}")
}

async fn fetch(address: SocketAddr, case: &Value) -> std::io::Result<Reply> {
    let request = &case["request"];
    let text = |name: &str| request[name].as_str();
    let origin = text("origin")
        .map(|host| format!("/__origin/{host}"))
        .unwrap_or_default();
    let (content_type, body) = match (text("json"), text("form")) {
        (Some(json), _) => (Some("application/json"), json),
        (None, Some(form)) => (Some("application/x-www-form-urlencoded"), form),
        (None, None) => (None, ""),
    };
    let token = bearer(&crate::areas::auth::tokens::test_key_settings(&[]));
    let mut headers = vec![("Authorization", token.as_str())];
    if let Some(content_type) = content_type {
        headers.push(("Content-Type", content_type));
    }
    let target = format!("{origin}{}", text("path").unwrap_or("/"));
    exchange(address, text("method").unwrap_or("GET"), &target, &headers, body).await
}

async fn serve(version: &IslesVersion, settings: Settings, game_data: GameDataRoot) -> Result<SocketAddr, String> {
    let mut shared = Shared::bare(version.clone());
    shared.settings = crate::areas::auth::tokens::test_key_settings(&[]).overlay(&settings);
    shared.game_data = game_data;
    // A database for the auth area's middleware to check accounts and game server keys against
    let dir = std::env::temp_dir().join(format!("isles-golden-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let node = module_api::NodeIdentity::standalone();
    let home = Arc::new(module_api::network::StandaloneHome { identity: node.clone() });
    let config = store::DatabaseConfig::Sqlite {
        file: dir.join("isles.db"),
    };
    let store = crate::db::IslesStore::open(&config, &store::DatabaseOptions::default(), home, &node)
        .map_err(|e| e.to_string())?;
    shared.store = Some(store);
    crate::areas::auth::apikeys::synchronize_configured_gameserver_api_keys(&shared, GAME_SERVER_KEY)
        .await
        .map_err(|e| e.to_string())?;
    let router = app(Arc::new(shared), Service::Backend, &crate::areas::backend()).map_err(|e| e.to_string())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    tokio::spawn(async move { axum::serve(listener, router).await });
    Ok(address)
}

/// What the cases cannot say (the reference harness stubs `HasParadoxBackendAuth` and the body parsers): the gate's
/// 401 on the routes that list it and not on the ones that don't, a progression config that isn't there, and a body
/// Express's parser refuses. The failures, as text.
pub async fn check_edges(version: IslesVersion) -> Vec<String> {
    let mut failures = Vec::new();
    let address = match serve(&version, Settings::default(), GameDataRoot::default()).await {
        Ok(address) => address,
        Err(error) => return vec![error],
    };
    let mut expect = |what: &str, reply: std::io::Result<Reply>, status: u16, body: Option<&str>| match reply {
        Err(error) => failures.push(format!("{what}: {error}")),
        Ok(reply) => {
            if reply.status != status || body.is_some_and(|body| !reply.body.contains(body)) {
                failures.push(format!("{what}: {} {}", reply.status, reply.body));
            }
        }
    };
    for (method, path) in [
        ("GET", "/survey/config"),
        ("POST", "/account/migrate"),
        ("POST", "/profile/update"),
        ("POST", "/trials/leaderboards/all"),
        ("POST", "/trials/leaderboards/solo/individual"),
        ("POST", "/motd/"),
        ("GET", "/playertreatments/u"),
        ("GET", "/eventstats/"),
        ("GET", "/progression/config"),
        ("GET", "/all/"),
        ("GET", "/__origin/gauntlet-prod.steelyard.ca/config"),
        (
            "GET",
            "/__origin/gauntlet-prod.steelyard.ca/leaderboard/get_leaderboard/x",
        ),
    ] {
        let reply = exchange(address, method, path, &[], "").await;
        expect(&format!("{method} {path} without credentials"), reply, 401, Some(""));
        if path != "/progression/config" {
            let reply = exchange(address, method, path, &[("x-mysticparadox-gameserver-apikey", "k")], "").await;
            let status = match path {
                "/motd/" => 204,
                _ => 200,
            };
            expect(&format!("{method} {path} as a game server"), reply, status, None);
        }
    }
    for (method, path) in [
        ("POST", "/heartbeat"),
        ("POST", "/event"),
        ("GET", "/crash/CrashReporter/Ping"),
        ("GET", "/patchnotes/en/1"),
        ("GET", "/game_tuning/bounty_game_data"),
    ] {
        let reply = exchange(address, method, path, &[], "").await;
        expect(&format!("{method} {path} without credentials"), reply, 200, None);
    }
    let token = bearer(&crate::areas::auth::tokens::test_key_settings(&[]));
    let auth = [("Authorization", token.as_str())];
    // A quiet 404 for a player, a game server and (after the gate) nobody
    let reply = exchange(address, "GET", "/motd/trigger?event_name=test", &auth, "").await;
    expect("motd trigger as a player", reply, 404, Some(""));
    let reply = exchange(address, "GET", "/motd/trigger?event_name=test", &[], "").await;
    expect("motd trigger without credentials", reply, 401, Some(""));
    // Node refuses to start without the file; here the route fails like an Express handler that throws
    let reply = exchange(address, "GET", "/progression/config", &auth, "").await;
    expect(
        "progression config without a game data folder",
        reply,
        500,
        Some("Internal Server Error"),
    );
    let json = [("Authorization", token.as_str()), ("Content-Type", "application/json")];
    let reply = exchange(address, "POST", "/trials/leaderboards/all", &json, "{\"difficulty\":").await;
    expect("a malformed JSON body", reply, 400, Some("<pre>Bad Request</pre>"));
    let reply = exchange(address, "POST", "/trials/leaderboards/all", &json, "\"text\"").await;
    expect(
        "a JSON body that isn't an object",
        reply,
        400,
        Some("<pre>Bad Request</pre>"),
    );
    let latin = [
        ("Authorization", token.as_str()),
        ("Content-Type", "application/json; charset=latin1"),
    ];
    let reply = exchange(address, "POST", "/trials/leaderboards/all", &latin, "{}").await;
    expect("an unsupported charset", reply, 415, None);
    let text = [("Authorization", token.as_str()), ("Content-Type", "text/plain")];
    let reply = exchange(address, "POST", "/trials/leaderboards/all", &text, "difficulty=7").await;
    expect("a body Express leaves unparsed", reply, 200, Some(r#""difficulty":1,"#));
    failures
}

/// Requests every fixture case of `label` (`"1.14.7"`) from `version`, with the synthetic progression
/// config as its `progression_config.json`. The cases that differ from Node, as text.
pub async fn check(version: IslesVersion, label: &str, fixtures: &Path) -> Vec<String> {
    let golden: Value = match std::fs::read_to_string(fixtures.join("game_data.json"))
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
    {
        Ok(golden) => golden,
        Err(error) => return vec![format!("game_data.json: {error}")],
    };
    pin_now_for_tests(golden["now"].as_i64());
    let data = std::env::temp_dir().join(format!("isles-golden-{}", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::create_dir_all(&data).and_then(|()| {
        std::fs::copy(
            fixtures.join("progression_config.synthetic.json"),
            data.join("progression_config.json"),
        )
        .map(drop)
    }) {
        return vec![format!("progression_config.synthetic.json: {error}")];
    }
    let mut failures = Vec::new();
    let cases = golden["cases"].as_array().map(Vec::as_slice).unwrap_or_default();
    for case in cases.iter().filter(|case| case["version"] == label) {
        let name = case["name"].as_str().unwrap_or("?");
        let env = case["request"]["env"].as_object().into_iter().flatten();
        let settings = Settings::from_pairs(env.filter_map(|(key, value)| Some((key.as_str(), value.as_str()?))));
        let address = match serve(&version, settings, GameDataRoot::from_config(data.to_str(), None)).await {
            Ok(address) => address,
            Err(error) => return vec![format!("{name}: {error}")],
        };
        let expect = &case["expect"];
        match fetch(address, case).await {
            Err(error) => failures.push(format!("{name}: {error}")),
            Ok(reply) => {
                let status = expect["status"].as_u64().unwrap_or(0);
                let content_type = expect["content_type"].as_str();
                let body = expect["body"].as_str().unwrap_or_default();
                if u64::from(reply.status) != status
                    || reply.content_type.as_deref() != content_type
                    || reply.body != body
                {
                    failures.push(format!(
                        "{name}\n  node: {status} {content_type:?} {body}\n  rust: {} {:?} {}",
                        reply.status, reply.content_type, reply.body
                    ));
                }
            }
        }
    }
    pin_now_for_tests(None);
    let _ = std::fs::remove_dir_all(&data);
    if failures.is_empty() && cases.iter().all(|case| case["version"] != label) {
        failures.push(format!("no golden cases for {label}"));
    }
    failures
}
