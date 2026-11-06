//! The edge checks restricted keeps beside two goldens, written from the Node source rather than recorded:
//! `game_data::golden::check_edges` (the auth gate on the game-data routes, a missing progression config, the bodies
//! Express refuses) and the extra cases of `chat_golden.rs` (`/vivox/login`, a friend with no account, bad JSON).
//! Each is a status and, where restricted checks one, a text the body must contain.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Caller, Mit, Tally, credentials, seed_account, send};
use serde_json::json;

#[allow(clippy::too_many_arguments)] // one row of the edge table
fn check(
    tally: &mut Tally,
    n: usize,
    what: &str,
    method: &str,
    path: &str,
    reply: &common::Reply,
    status: u16,
    body: Option<&str>,
) {
    let expected = json!({ "status": status, "body contains": body });
    let shown = match body {
        Some(text) if reply.body.contains(text) => json!(text),
        Some(_) => json!(reply.body),
        None => json!(null),
    };
    tally.step(
        n,
        what,
        method,
        path,
        expected,
        json!({ "status": reply.status, "body contains": shown }),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn game_data_edges() {
    let mit = Mit::start(&[], |_| {}).await;
    let mut tally = Tally::new("game_data_edges");
    let mut n = 0;
    let key = credentials(Caller::Gameserver(None));
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
        let reply = send(mit.backend, method, path, &[], Some(b"")).await;
        n += 1;
        check(
            &mut tally,
            n,
            "without credentials",
            method,
            path,
            &reply,
            401,
            Some(""),
        );
        if path != "/progression/config" {
            let reply = send(mit.backend, method, path, &key, Some(b"")).await;
            n += 1;
            let status = if path == "/motd/" { 204 } else { 200 };
            check(&mut tally, n, "as a game server", method, path, &reply, status, None);
        }
    }
    for (method, path) in [
        ("POST", "/heartbeat"),
        ("POST", "/event"),
        ("GET", "/crash/CrashReporter/Ping"),
        ("GET", "/patchnotes/en/1"),
        ("GET", "/game_tuning/bounty_game_data"),
    ] {
        let reply = send(mit.backend, method, path, &[], Some(b"")).await;
        n += 1;
        check(&mut tally, n, "without credentials", method, path, &reply, 200, None);
    }
    let player = credentials(Caller::Player("golden-user"));
    let path = "/motd/trigger?event_name=test";
    let reply = send(mit.backend, "GET", path, &player, Some(b"")).await;
    n += 1;
    check(
        &mut tally,
        n,
        "motd trigger as a player",
        "GET",
        path,
        &reply,
        404,
        Some(""),
    );
    let reply = send(mit.backend, "GET", path, &[], Some(b"")).await;
    n += 1;
    check(
        &mut tally,
        n,
        "motd trigger without credentials",
        "GET",
        path,
        &reply,
        401,
        Some(""),
    );
    let reply = send(mit.backend, "GET", "/progression/config", &player, Some(b"")).await;
    n += 1;
    check(
        &mut tally,
        n,
        "progression config without the file",
        "GET",
        "/progression/config",
        &reply,
        500,
        Some("Internal Server Error"),
    );
    let with = |content_type: &str| {
        let mut h = player.clone();
        h.push(("content-type".into(), content_type.into()));
        h
    };
    let leaderboards = "/trials/leaderboards/all";
    for (what, content_type, body, status, contains) in [
        (
            "a malformed JSON body",
            "application/json",
            "{\"difficulty\":",
            400,
            Some("<pre>Bad Request</pre>"),
        ),
        (
            "a JSON body that isn't an object",
            "application/json",
            "\"text\"",
            400,
            Some("<pre>Bad Request</pre>"),
        ),
        (
            "an unsupported charset",
            "application/json; charset=latin1",
            "{}",
            415,
            None,
        ),
        (
            "a body Express leaves unparsed",
            "text/plain",
            "difficulty=7",
            200,
            Some(r#""difficulty":1,"#),
        ),
    ] {
        let reply = send(
            mit.backend,
            "POST",
            leaderboards,
            &with(content_type),
            Some(body.as_bytes()),
        )
        .await;
        n += 1;
        check(&mut tally, n, what, "POST", leaderboards, &reply, status, contains);
    }
    tally.finish();
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_edges() {
    let mit = Mit::start(&[], |_| {}).await;
    let store = mit.store();
    for user in ["alice", "bob", "carol"] {
        seed_account(&store, user, user).await;
    }
    let mut tally = Tally::new("chat_edges");
    let alice = credentials(Caller::Player("alice"));
    let reply = send(mit.backend, "GET", "/vivox/login", &alice, None).await;
    check(
        &mut tally,
        1,
        "vivox, signed in",
        "GET",
        "/vivox/login",
        &reply,
        404,
        Some(""),
    );
    let reply = send(mit.backend, "GET", "/vivox/login", &[], None).await;
    check(
        &mut tally,
        2,
        "vivox, anonymous",
        "GET",
        "/vivox/login",
        &reply,
        401,
        Some(""),
    );
    let invite = "/friends/api/public/friends/alice/ghost";
    let reply = send(mit.backend, "POST", invite, &alice, None).await;
    check(
        &mut tally,
        3,
        "a friend with no account",
        "POST",
        invite,
        &reply,
        204,
        None,
    );
    let list = "/friends/api/public/friends/alice?includePending=true";
    let reply = send(mit.backend, "GET", list, &alice, None).await;
    let expected = json!({ "status": 200, "body": "[]" });
    tally.step(
        4,
        "takes no edge",
        "GET",
        list,
        expected,
        json!({ "status": reply.status, "body": reply.body }),
    );
    let json_type = vec![("content-type".to_owned(), "application/json".to_owned())];
    let mut alice_json = alice.clone();
    alice_json.extend(json_type.clone());
    let slayer = "/slayerlink/availability";
    let reply = send(mit.backend, "POST", slayer, &json_type, Some(b"{nope")).await;
    check(&mut tally, 5, "bad JSON before auth", "POST", slayer, &reply, 400, None);
    let reply = send(mit.backend, "POST", slayer, &alice_json, Some(b"7")).await;
    check(
        &mut tally,
        6,
        "a JSON primitive (strict)",
        "POST",
        slayer,
        &reply,
        400,
        None,
    );
    let big = "x".repeat(101 * 1024);
    let reply = send(mit.backend, "POST", slayer, &alice_json, Some(big.as_bytes())).await;
    check(&mut tally, 7, "a body over 100 KB", "POST", slayer, &reply, 413, None);
    tally.finish();
}
