// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 29 rows black-box through core's `app()` (spec §10.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use dainty_service_kit::serve::ConnectionInfo;
use http_body_util::BodyExt;
use rustic_isles::handlers::auth_handlers;
use rustic_isles::testing::{GAUNTLET_ORIGIN, SAMPLE_TABLES, SampleHooks, sample_path};
use rustic_isles::{
    BountyItemGrant, Destination, Entitlement, GameDataRoot, Group, Handlers, IslesVersion, NoPlayerPolicy,
    ObjectiveRow, ProgressionConfigRule, RouteSet, Service, Settings, Shared, SharedParts, StubReply, VersionHooks,
    VersionTables, routes,
};
use rustic_isles_auth::test_support::{MemoryStore, throwaway_signing_keys};
use serde_json::{Value, json};
use tower::ServiceExt;

static GRANTS: [BountyItemGrant; 1] = [BountyItemGrant {
    item_id: "TOKEN_X",
    amount: 2,
    start: "2026-07-30T17:00:00.000Z",
    end: "2026-10-22T17:00:00.000Z",
}];

static TABLES: VersionTables = VersionTables {
    active_huntpass_id: "season43",
    recent_huntpass_seasons: &["season43", "season44"],
    patchnotes_release_version: "",
    bounty_item_grant_data: &GRANTS,
    ..SAMPLE_TABLES
};

static RELEASED: VersionTables = VersionTables {
    patchnotes_release_version: "9.9",
    ..TABLES
};

/// 1.14.7's answers for the two hooks this area reads; the rest as core's sample.
struct Hooks {
    survey: StubReply,
    rule: ProgressionConfigRule,
}

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        SampleHooks.no_player_policy()
    }
    fn weapons_at_base_level(&self, settings: &Settings) -> bool {
        SampleHooks.weapons_at_base_level(settings)
    }
    fn reforge_rules(&self) -> bool {
        true
    }
    fn mastery_persistence(&self) -> bool {
        true
    }
    fn huntpass_payload(&self, id: &str) -> Value {
        SampleHooks.huntpass_payload(id)
    }
    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        SampleHooks.objective_row(user_id, row)
    }
    fn progression_config(&self) -> ProgressionConfigRule {
        self.rule
    }
    fn survey_config(&self) -> StubReply {
        self.survey.clone()
    }
    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        SampleHooks.entitlements_payload(list)
    }
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        SampleHooks.candidate_destination(hunt_id, realm_hub)
    }
}

fn survey_1_14_7() -> StubReply {
    StubReply {
        status: 200,
        body: Some(rustic_isles_gamedata::envelope(json!({"Triggers": []}))),
    }
}

struct Answer {
    status: u16,
    content_type: Option<String>,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
    fn payload(&self) -> Value {
        assert_eq!(self.status, 200, "{}", self.body);
        let value = self.json();
        assert_eq!((&value["code"], &value["message"]), (&Value::Null, &json!("OK")));
        value["payload"].clone()
    }
}

struct Harness {
    app: axum::Router,
    shared: Arc<Shared>,
    _dir: tempfile::TempDir,
}

struct Options {
    settings: Vec<(&'static str, &'static str)>,
    survey: StubReply,
    rule: ProgressionConfigRule,
    tables: &'static VersionTables,
    file: Option<&'static str>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            settings: Vec::new(),
            survey: survey_1_14_7(),
            rule: ProgressionConfigRule::RollingHuntpassWindows,
            tables: &TABLES,
            file: None,
        }
    }
}

const JSON_UTF8: &str = "application/json; charset=utf-8";

impl Harness {
    fn new(options: Options) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if let Some(file) = options.file {
            std::fs::write(dir.path().join("progression_config.json"), file).unwrap();
        }
        let version = IslesVersion {
            hooks: Arc::new(Hooks {
                survey: options.survey,
                rule: options.rule,
            }),
            tables: options.tables,
            routes: RouteSet::V1147,
        };
        let (private, public) = throwaway_signing_keys();
        let mut pairs: Vec<(&str, &str)> = vec![
            ("AUTH_SIGNING_PRIVKEY_B64", &private),
            ("AUTH_SIGNING_PUBKEY_B64", &public),
            ("RUSTIC_ISLES_REPLAY_HEADERS", "on"),
        ];
        pairs.extend(options.settings.iter().copied());
        let mut parts = SharedParts::bare(version);
        parts.settings = Settings::from_pairs(pairs);
        parts.auth_store = Some(Arc::new(MemoryStore::new()));
        parts.game_data = GameDataRoot::new(Some(dir.path().to_path_buf()), None);
        let mut handlers = auth_handlers().unwrap();
        rustic_isles_gamedata::register(&mut handlers);
        parts.handlers = handlers.clone();
        let shared = Shared::new(parts).unwrap();
        let app = rustic_isles::app(Arc::clone(&shared), Service::Backend, &handlers).unwrap();
        Self { app, shared, _dir: dir }
    }

    fn bare() -> Self {
        Self::new(Options::default())
    }

    fn dir(&self) -> &Path {
        self._dir.path()
    }

    fn token(&self) -> String {
        format!("Bearer {}", self.shared.auth().sign_game_token("user-1").unwrap())
    }

    async fn send(&self, method: &str, uri: &str, headers: &[(&str, &str)], body: Option<&str>) -> Answer {
        let mut builder = Request::builder().method(method).uri(uri).header("host", "localhost");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let body = body.unwrap_or("");
        builder = builder.header("content-length", body.len().to_string());
        let mut request = builder.body(Body::from(body.to_owned())).unwrap();
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(ConnectionInfo {
            peer,
            local: "127.0.0.1:61000".parse().unwrap(),
        }));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_owned());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            content_type,
            body: String::from_utf8_lossy(&body).into_owned(),
        }
    }

    async fn game(&self, method: &str, uri: &str) -> Answer {
        let token = self.token();
        self.send(method, uri, &[("authorization", &token)], None).await
    }

    async fn post_json(&self, uri: &str, content_type: &str, body: &str) -> Answer {
        let token = self.token();
        let headers = [("authorization", token.as_str()), ("content-type", content_type)];
        self.send("POST", uri, &headers, Some(body)).await
    }
}

const OPEN: [&str; 11] = [
    "/heartbeat",
    "/event",
    "/crash/CrashReporter/Ping",
    "/crash/CrashReporter/CheckReport",
    "/patchnotes/:language/:gameversion",
    "/game_tuning/seasonal_event_schedule",
    "/game_tuning/huntpass_xp_config",
    "/game_tuning/island_content_config",
    "/game_tuning/bounty_game_data",
    "/game_tuning/bounty_game_data_daily",
    "/game_tuning/bounty_game_data_weekly",
];

#[test]
fn routes_are_the_game_data_rows() {
    let rows: Vec<_> = routes::all()
        .iter()
        .filter(|r| r.group == Group::GameData)
        .map(|r| (r.method, r.path))
        .collect();
    assert_eq!(rustic_isles_gamedata::routes(), rows.as_slice());
    let mut handlers = Handlers::new();
    rustic_isles_gamedata::register(&mut handlers);
    assert_eq!(handlers.check(), Ok(()));
    for (method, path) in rustic_isles_gamedata::routes() {
        assert!(handlers.contains(Service::Backend, *method, path), "{path}");
    }
}

#[tokio::test]
async fn gates_without_a_token() {
    let h = Harness::bare();
    for row in routes::all().iter().filter(|r| r.group == Group::GameData) {
        let answer = h.send(row.method.as_str(), &sample_path(row), &[], None).await;
        if OPEN.contains(&row.path) {
            assert!(answer.status == 200, "{} answered {}", row.path, answer.status);
        } else {
            assert_eq!((answer.status, answer.body.as_str()), (401, ""), "{}", row.path);
        }
    }
}

#[tokio::test]
async fn constant_answers() {
    let h = Harness::bare();
    let a = h.send("POST", "/heartbeat", &[], None).await;
    assert_eq!(
        (a.status, a.content_type.as_deref(), a.body.as_str()),
        (200, Some("text/plain; charset=utf-8"), "20000")
    );
    let a = h.send("GET", "/crash/CrashReporter/Ping", &[], None).await;
    assert_eq!((a.status, a.body.as_str()), (200, "OK"));
    for uri in ["/event", "/crash/CrashReporter/CheckReport"] {
        let a = h.send("POST", uri, &[], None).await;
        assert_eq!((a.content_type.as_deref(), a.body.as_str()), (Some(JSON_UTF8), "{}"));
    }
    let a = h.game("POST", "/account/migrate").await;
    assert_eq!(a.body, r#"{"migration_failed":false,"migration_finished":true}"#);
    let a = h.game("POST", "/profile/update").await;
    assert_eq!((a.status, a.content_type, a.body.as_str()), (200, None, ""));
    let a = h.game("POST", "/motd").await;
    assert_eq!((a.status, a.content_type, a.body.as_str()), (204, None, ""));
    let a = h.game("GET", "/motd/trigger?event_name=test").await;
    assert_eq!((a.status, a.body.as_str()), (404, ""));
    let a = h.game("GET", "/eventstats/").await;
    assert_eq!(
        (a.content_type.as_deref(), a.body.as_str()),
        (Some(JSON_UTF8), r#"{"stats":[]}"#)
    );
    let a = h.game("GET", "/all/").await;
    assert_eq!(a.body, r#"{"code":null,"message":"OK","payload":{"messages":[]}}"#);
    let a = h.game("POST", "/trials/leaderboards/solo/individual").await;
    assert_eq!(a.body, r#"{"code":null,"message":"OK","payload":{}}"#);

    let tuning = |payload: &str| format!(r#"{{"code":null,"message":"OK","payload":{payload}}}"#);
    let cases = [
        ("seasonal_event_schedule", r#"{"ScheduledItems":[]}"#.to_owned()),
        (
            "huntpass_xp_config",
            r#"{"EventConfigs":[],"GlobalConfig":{"DifficultyBias":1,"GlobalMultiplier":1,"MaxXPAwarded":200}}"#.into(),
        ),
        ("island_content_config", r#"{"DisallowedIslandContentAssets":[]}"#.into()),
        (
            "bounty_game_data",
            concat!(
                r#"{"bounty_data":[{"bounty_id":"Bounty_Bronze_KillWithFriends","enabled":false},"#,
                r#"{"bounty_id":"Bounty_Silver_KillWithFriends","enabled":false},"#,
                r#"{"bounty_id":"Bounty_Gold_KillWithFriends","enabled":false}],"bounty_token_grant_hour":0,"#,
                r#""bounty_token_id":"TOKEN_BOUNTY_DRAFT","bronze_count":9,"silver_count":3,"gold_count":1,"#,
                r#""history_length":10,"item_grant_data":[{"on_claim_grant_item_amount":2,"#,
                r#""on_claim_grant_item_end_date":"2026-10-22T17:00:00.000Z","on_claim_grant_item_event_id":"","#,
                r#""on_claim_grant_item_id":"TOKEN_X","on_claim_grant_item_start_date":"2026-07-30T17:00:00.000Z"}],"#,
                r#""max_slots":4,"new_season_reset_bounties":false,"num_draft_options":3,"num_spicy_options":1,"#,
                r#""num_tokens_hp_start":4,"num_tokens_per_day":0,"premium_bounty_token_id":"TOKEN_BOUNTY_DRAFT_PREMIUM","#,
                r#""token_rollover_warning_days":1000}"#
            )
            .into(),
        ),
        (
            "bounty_game_data_daily",
            concat!(
                r#"{"automatic_claim":true,"automatic_draft":true,"bounty_data":[{"bounty_id":"#,
                r#""Challenge_Daily_Bronze_GetHuntPassXP","enabled":false}],"bounty_token_grant_hour":0,"#,
                r#""bounty_token_id":"TOKEN_DAILY_CHALLENGE_DRAFT","bronze_count":1,"delete_claimed_bounties":false,"#,
                r#""gold_count":0,"history_length":10,"item_grant_data":[],"max_slots":1,"#,
                r#""new_season_reset_bounties":true,"num_draft_options":3,"num_spicy_options":1,"#,
                r#""num_tokens_hp_start":1,"num_tokens_per_day":0,"#,
                r#""premium_bounty_token_id":"TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM","silver_count":0,"#,
                r#""token_rollover_warning_days":1000}"#
            )
            .into(),
        ),
        (
            "bounty_game_data_weekly",
            concat!(
                r#"{"automatic_claim":true,"automatic_draft":true,"bounty_data":[{"bounty_id":"#,
                r#""26_11_7_Challenge_Season_BreakParts_Firesacs_Aether_Sally_Terra","enabled":false}],"#,
                r#""item_grant_data":[],"bounty_token_id":"TOKEN_WEEKLY_CHALLENGE_DRAFT","bounty_token_grant_hour":0,"#,
                r#""bronze_count":0,"silver_count":0,"gold_count":0,"history_length":8,"max_slots":4,"#,
                r#""num_draft_options":3,"num_spicy_options":1,"num_tokens_hp_start":4,"num_tokens_per_day":0,"#,
                r#""new_season_reset_bounties":true,"delete_claimed_bounties":false,"#,
                r#""premium_bounty_token_id":"TOKEN_WEEKLY_CHALLENGE_DRAFT_PREMIUM","token_rollover_warning_days":1000}"#
            )
            .into(),
        ),
    ];
    for (name, payload) in cases {
        let a = h.send("GET", &format!("/game_tuning/{name}"), &[], None).await;
        assert_eq!((a.status, a.content_type.as_deref()), (200, Some(JSON_UTF8)), "{name}");
        assert_eq!(a.body, tuning(&payload), "{name}");
    }
}

#[tokio::test]
async fn treatments() {
    let dojo = r#"{"treatments":["CohortTreatment.Dojo.B"]}"#;
    let none = r#"{"treatments":[]}"#;
    for (setting, uri, want) in [
        (None, "/playertreatments", dojo),
        (Some("1"), "/playertreatments/", dojo),
        (Some("0"), "/playertreatments", none),
        (Some(""), "/playertreatments", none),
        (None, "/playertreatments/some-user", dojo),
    ] {
        let settings = setting
            .map(|v| vec![("COHORT_TREATMENT_LEGACY", v)])
            .unwrap_or_default();
        let h = Harness::new(Options {
            settings,
            ..Options::default()
        });
        assert_eq!(h.game("GET", uri).await.body, want, "{setting:?} {uri}");
    }
}

#[tokio::test]
async fn trials_leaderboard() {
    let h = Harness::bare();
    let fields = |p: &Value| {
        (
            p["difficulty"].clone(),
            p["page"].clone(),
            p["page_size"].clone(),
            p["trial_id"].clone(),
        )
    };
    let defaults = (json!(1), json!(0), json!(100), json!(""));
    let a = h.game("POST", "/trials/leaderboards/all").await;
    assert_eq!(
        a.body,
        concat!(
            r#"{"code":null,"message":"OK","payload":{"difficulty":1,"guild":{},"page":0,"page_size":100,"#,
            r#""trial_id":"","world":{"group":{"difficulty":1,"entries":[]},"solo":{"all":{"difficulty":1,"#,
            r#""entries":[]}}}}}"#
        )
    );
    for body in ["{}", r#"[{"difficulty":9}]"#] {
        let a = h.post_json("/trials/leaderboards/all", "application/json", body).await;
        assert_eq!(fields(&a.payload()), defaults, "{body}");
    }
    let a = h
        .post_json(
            "/trials/leaderboards/all",
            "application/json",
            r#"{"difficulty":3.0,"page":2,"page_size":"50"}"#,
        )
        .await;
    assert_eq!(fields(&a.payload()), (json!(3), json!(2), json!("50"), json!("")));
    assert!(a.body.contains(r#""world":{"group":{"difficulty":3,"#));
    let a = h
        .post_json(
            "/trials/leaderboards/all",
            "application/json",
            r#"{"difficulty":null,"page":{"a":[1,2e0]}}"#,
        )
        .await;
    assert_eq!(fields(&a.payload()).0, json!(1));
    assert!(a.body.contains(r#""page":{"a":[1,2]}"#));
    let a = h
        .post_json(
            "/trials/leaderboards/all",
            "application/x-www-form-urlencoded",
            "difficulty=2&trial_id=a%20b",
        )
        .await;
    let p = a.payload();
    assert_eq!((&p["difficulty"], &p["trial_id"]), (&json!("2"), &json!("a b")));
    let a = h
        .post_json("/trials/leaderboards/all", "application/json", "{bad")
        .await;
    assert_eq!(a.status, 400);
    assert!(a.content_type.unwrap().starts_with("text/html"));
    let headers = [("content-type", "application/json")];
    let a = h.send("POST", "/trials/leaderboards/all", &headers, Some("{bad")).await;
    assert_eq!((a.status, a.body.as_str()), (401, ""));
}

#[tokio::test]
async fn survey() {
    let h = Harness::bare();
    let a = h.game("GET", "/survey/config").await;
    assert_eq!(
        (a.status, a.body.as_str()),
        (200, r#"{"code":null,"message":"OK","payload":{"Triggers":[]}}"#)
    );
    let h = Harness::new(Options {
        survey: StubReply {
            status: 204,
            body: None,
        },
        ..Options::default()
    });
    let a = h.game("GET", "/survey/config").await;
    assert_eq!((a.status, a.content_type, a.body.as_str()), (204, None, ""));
    let h = Harness::new(Options {
        survey: StubReply {
            status: 1000,
            body: Some(json!({})),
        },
        ..Options::default()
    });
    assert_eq!(h.game("GET", "/survey/config").await.status, 500);
}

#[tokio::test]
async fn gauntlet() {
    let h = Harness::bare();
    let at = |uri: &str| format!("{GAUNTLET_ORIGIN}{uri}");
    let both = json!({"personal_progression": 0, "guild_progression": 0});
    for (uri, want) in [
        ("/progression/ESC_SEASON_1/user-1", both.clone()),
        ("/progression//INVALID", both),
        ("/progression/level_access/user-1?account_id=x", json!(0)),
        ("/PROGRESSION/LEVEL_ACCESS/user-1", json!(0)),
        ("/rewards/personal/ESC_SEASON_1/user-1", json!({})),
        ("/rewards/guild/ESC_SEASON_1/user-1", json!(0)),
        ("/leaderboard/get_leaderboard/x", json!([])),
    ] {
        assert_eq!(h.game("GET", &at(uri)).await.payload(), want, "{uri}");
    }
    // The client asks level access with no bearer; every other Gauntlet query still needs one.
    let bare = h
        .send("GET", &at("/progression/level_access/ESC_SEASON_1"), &[], None)
        .await;
    assert_eq!(bare.body, r#"{"code":null,"message":"OK","payload":0}"#);
    let refused = h.send("GET", &at("/progression/ESC_SEASON_1/user-1"), &[], None).await;
    assert_eq!((refused.status, refused.body.as_str()), (401, ""));
    let elsewhere = h.send("GET", "/progression/level_access/ESC_SEASON_1", &[], None).await;
    assert_ne!(elsewhere.status, 200, "only the Gauntlet origin answers it");
    let token = h.token();
    let pinned = [
        ("authorization", token.as_str()),
        ("x-isles-replay-now-ms", "2026-10-16T08:30:00Z"),
    ];
    let a = h.send("GET", &at("/config"), &pinned, None).await;
    assert_eq!(
        a.body,
        r#"{"code":null,"message":"OK","payload":{"gauntlet_id":"ESC_SEASON_1","start_at":"2026-07-30T17:00:00+00:00","end_at":"2026-10-22T17:00:00+00:00"}}"#
    );
    let a = h
        .send("GET", "/__origin/mailbox-prod.steelyard.ca/config", &pinned, None)
        .await;
    assert_eq!((a.status, a.body.as_str()), (404, ""));
    let off = Harness::new(Options {
        settings: vec![("GAUNTLET_CONFIG_PROBE", "0")],
        ..Options::default()
    });
    assert_eq!(off.game("GET", &at("/config")).await.payload(), json!({}));
    // A write on the Gauntlet host answers an empty envelope; elsewhere the row passes on
    let write = h.game("PUT", &at("/anything/ESC_SEASON_1/user-1")).await;
    assert_eq!((write.status, write.payload()), (200, json!({})));
    assert_ne!(h.game("PUT", "/anything/ESC_SEASON_1/user-1").await.status, 200);
    // A player's own token may not record a finished run
    let token = h.token();
    let run = r#"{"gauntlet_id":"ESC_SEASON_1","players":["user-1"],"level":9,"remaining_sec":1,"data":{}}"#;
    let forged = h
        .send(
            "PUT",
            &at("/process_run"),
            &[("authorization", token.as_str())],
            Some(run),
        )
        .await;
    assert_eq!(forged.status, 403);
    // With Gauntlet on, level access opens the first level; the other progression answers are unchanged
    let on = Harness::new(Options {
        settings: vec![("RUSTIC_ISLES_GAUNTLET", " On ")],
        ..Options::default()
    });
    let access = on
        .send("GET", &at("/progression/level_access/ESC_SEASON_1"), &[], None)
        .await;
    assert_eq!(access.body, r#"{"code":null,"message":"OK","payload":1}"#);
    assert_eq!(
        on.game("GET", &at("/progression/ESC_SEASON_1/user-1")).await.payload(),
        json!({"personal_progression": 0, "guild_progression": 0})
    );
}

#[tokio::test]
async fn patch_notes() {
    let h = Harness::bare();
    let p = h.send("GET", "/patchnotes/pt%2DBR/1.0", &[], None).await.payload();
    let keys: Vec<&String> = p.as_object().unwrap().keys().collect();
    let order = [
        "date",
        "description",
        "language",
        "notes",
        "permalink",
        "release_version",
        "title",
    ];
    assert_eq!(keys, order);
    assert_eq!(
        (&p["language"], &p["release_version"], &p["date"]),
        (&json!("pt-BR"), &json!(""), &json!("2026-10-08T00:00:00.000+00:00"))
    );
    let notes = p["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 2);
    let shape = |n: &Value| (n["title"].clone(), n["type"].clone());
    assert_eq!(
        shape(&notes[0]),
        (json!("Welcome to Rustic Isles"), json!("new_to_dauntless"))
    );
    assert_eq!(shape(&notes[1]), (json!("Community"), json!("quality_of_life")));
    let sections = notes[0]["sections"].as_array().unwrap();
    assert_eq!(shape(&sections[0]), (json!("The hunt continues"), json!("featured")));
    assert!(sections[0]["description"].is_string());
    assert_eq!(shape(&sections[1]), (json!("Getting started"), json!("default")));
    assert_eq!(sections[1]["changes"][0]["list"].as_array().unwrap().len(), 3);
    let help = &notes[1]["sections"][0];
    assert_eq!(shape(help), (json!("Help and feedback"), json!("default")));
    assert_eq!(help["changes"][0]["list"].as_array().unwrap().len(), 3);

    let h = Harness::new(Options {
        tables: &RELEASED,
        ..Options::default()
    });
    let p = h.send("GET", "/patchnotes/fr/9.9", &[], None).await.payload();
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&p),
        r#"{"date":"2024-12-19T18:00:00.000+00:00","description":"Welcome to Rustic Isles!","language":"fr","notes":[],"permalink":"/patch-notes/rusticisles/","release_version":"9.9","title":"Rustic Isles"}"#
    );
}

const CONFIG: &str = r#"{"code":null,"message":"OK","payload":{"paths":[{"progression_id":"season43","start_date":"2020-01-01T00:00:00+00:00","end_date":"2021-01-01T00:00:00+00:00","tracks":[1,2]}]}}"#;

#[tokio::test]
async fn progression_config() {
    let h = Harness::bare();
    let a = h.game("GET", "/progression/config").await;
    assert_eq!(a.status, 500);
    assert!(a.content_type.unwrap().starts_with("text/html"));

    let h = Harness::new(Options {
        rule: ProgressionConfigRule::AsIs,
        file: Some(CONFIG),
        ..Options::default()
    });
    assert_eq!(h.game("GET", "/progression/config").await.body, CONFIG);

    let h = Harness::new(Options {
        file: Some(CONFIG),
        ..Options::default()
    });
    let token = h.token();
    let at = |now: &'static str| {
        [
            ("authorization", token.clone()),
            ("x-isles-replay-now-ms", now.to_owned()),
        ]
    };
    let get = |now: &'static str| {
        let headers = at(now);
        let h = &h;
        async move {
            let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            h.send("GET", "/progression/config", &headers, None).await.payload()
        }
    };
    let p = get("2026-10-13T18:00:00.000Z").await;
    let paths = p["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0]["start_date"], "2026-07-30T17:00:00+00:00");
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&paths[1]),
        r#"{"progression_id":"season44","start_date":"2099-01-02T00:00:00+00:00","end_date":"2099-12-31T00:00:00+00:00","tracks":[1,2]}"#
    );
    // The cached read is kept, and each request rewrites from it at its own time
    std::fs::write(h.dir().join("progression_config.json"), "{}").unwrap();
    let later = get("2026-10-23T00:00:00.000Z").await;
    assert_eq!(later["paths"][0]["start_date"], "2026-10-22T17:00:00+00:00");
    assert_eq!(later["paths"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn trials_boards_without_a_store() {
    let h = Harness::bare();
    let body = r#"{"trial_id":"CR19_PlayerHunt_Arena_Elite","difficulty":2,"page":0,"page_size":10}"#;
    let a = h.post_json("/trials/leaderboards/solo", "application/json", body).await;
    assert_eq!(
        a.payload(),
        json!({"difficulty":2,"page":0,"page_size":10,"trial_id":"CR19_PlayerHunt_Arena_Elite","entries":[]})
    );
    let a = h
        .post_json("/trials/leaderboards/group", "application/json", "{}")
        .await;
    assert_eq!(a.payload()["entries"], json!([]));
    for own in [
        "/trials/leaderboards/solo/individual",
        "/trials/leaderboards/group/individual",
    ] {
        let a = h.post_json(own, "application/json", body).await;
        assert_eq!(a.payload(), json!({}), "{own}");
    }
    // Any other Trials write is logged and answered with an empty envelope
    let a = h
        .post_json("/trials/leaderboards/guild", "application/json", "{}")
        .await;
    assert_eq!(a.payload(), json!({}));
    // A player's own token may not record a finished run
    let run = r#"{"trial_id":"T","completion_time":90,"phx_account_id":"user-1","weapon":1}"#;
    let forged = h.post_json("/trials/leaderboards", "application/json", run).await;
    assert_eq!((forged.status, forged.body.as_str()), (403, ""));
}
