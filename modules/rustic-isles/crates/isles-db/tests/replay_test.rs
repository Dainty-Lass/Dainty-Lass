//! RU8 shadow replay harness and normalizer tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use isles_core::areas::auth::crypto::{KeyScope, hash_api_key};
use isles_core::areas::auth::data::system_actor;
use isles_core::areas::auth::tokens::test_key_settings;
use isles_core::db::IslesStore;
use isles_core::game_data::GameDataRoot;
use isles_core::hooks::{
    Destination, Entitlement, GameMode, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, StubReply, VersionHooks,
};
use isles_core::http::{Handlers, Shared, app};
use isles_core::repo::auth::AccountRecord;
use isles_core::routes::{Method, RouteSet, Service};
use isles_core::settings::{
    BOUNTY_PERSISTENCE, COOLDOWN_PERSISTENCE, MASTERY_PERSISTENCE, QOS_PORT, REFORGE, Settings,
};
use isles_core::tables::{
    ItemGrantWindow, MaskedJourneyNode, SeasonalCoin, StarterManifest, SwitchDefault, VersionTables,
};
use isles_core::version::IslesVersion;
use isles_core::{AccountId, UserId};
use isles_db::replay::{
    CaptureBody, CaptureRecord, DiffKind, HttpRecord, IdNormalizer, NormalizerConfig, ReplayConfig, ReplayEndpoint,
    ReplayHttpResponse, ReplayOptions, ReplayTarget, ReplayTargetAuth, compare_chat_stanzas, compare_http_responses,
    is_filtered_chat_stanza, json_status_response, learn_ids, load_corpus, load_key_map, mint_hs256_jwt,
    mint_rs256_jwt, normalize_chat_stanza, normalize_headers, normalize_jwts_in_str, parse_capture_line,
    parse_capture_str, prepare_http_request, run_replay, run_replay_cli, serve_ephemeral, substitute_str,
    substitute_value,
};
use isles_db::{Command, parse_args};
use module_api::NodeIdentity;
use serde_json::{Value, json};
use store::{DatabaseConfig, DatabaseOptions};

// ---------------------------------------------------------------------------
// 1. Capture JSONL Parsing Tests
// ---------------------------------------------------------------------------

#[test]
fn parses_http_and_chat_capture_jsonl_records() {
    let jsonl = r#"
{"t":"2026-10-22T17:00:00.100Z","kind":"http","conn":1,"seq":1,"ip":"127.0.0.1","method":"POST","host":"127.0.0.1:8421","target":"/account/api/oauth/token","route":"backend","req_headers":{"content-type":"application/x-www-form-urlencoded","authorization":"basic <redacted>"},"req_body":{"form":[["grant_type","exchange_code"],["exchange_code","<key:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef>"]]},"status":200,"res_headers":{"content-type":"application/json; charset=utf-8"},"res_body":{"json":{"access_token":"<token:UID-slayer01>","token_type":"bearer"}},"ms":4}
{"t":"2026-10-22T17:00:00.050Z","kind":"http","conn":1,"seq":0,"ip":"127.0.0.1","method":"GET","host":"127.0.0.1:8421","target":"/reforged/api/ServerStatus","route":"backend","req_headers":{},"req_body":null,"status":200,"res_headers":{"content-type":"application/json"},"res_body":{"json":{"serverOnline":true}},"ms":1}
{"t":"2026-10-22T17:00:00.200Z","kind":"chat","conn":7,"ip":"127.0.0.1","event":"open"}
{"t":"2026-10-22T17:00:00.210Z","kind":"chat","conn":7,"dir":"in","text":"<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'><plain:UID-slayer01></auth>"}
{"t":"2026-10-22T17:00:00.220Z","kind":"chat","conn":7,"dir":"out","text":"<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>"}
{"t":"2026-10-22T17:00:00.300Z","kind":"http","conn":2,"seq":1,"ip":"127.0.0.1","method":"POST","host":"127.0.0.1:8421","target":"/upload","route":"backend","req_headers":{},"req_body":{"binary":128},"status":204,"res_headers":{},"res_body":{"omitted":65536},"ms":2}
{"t":"2026-10-22T17:00:00.400Z","kind":"http","conn":99,"truncated_line_at_eof
"#;

    let records = parse_capture_str(jsonl);
    assert_eq!(records.len(), 6, "truncated trailing line is ignored gracefully");

    match &records[0] {
        CaptureRecord::Http(http) => {
            assert_eq!(http.method, "POST");
            assert_eq!(http.target, "/account/api/oauth/token");
            assert_eq!(
                http.req_body,
                CaptureBody::Form(vec![
                    ("grant_type".into(), "exchange_code".into()),
                    (
                        "exchange_code".into(),
                        "<key:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef>".into()
                    )
                ])
            );
            assert!(!http.req_body.is_lossy());
        }
        other => panic!("expected HttpRecord, got {other:?}"),
    }

    match &records[2] {
        CaptureRecord::Chat(chat) => {
            assert_eq!(chat.conn, 7);
            assert_eq!(chat.event.as_deref(), Some("open"));
        }
        other => panic!("expected ChatRecord, got {other:?}"),
    }

    match &records[5] {
        CaptureRecord::Http(http) => {
            assert_eq!(http.req_body, CaptureBody::Binary(128));
            assert_eq!(http.res_body, CaptureBody::Omitted(65536));
            assert!(http.req_body.is_lossy());
            assert!(http.res_body.is_lossy());
        }
        other => panic!("expected HttpRecord, got {other:?}"),
    }

    // Verify load_corpus sorts across multiple files by timestamp (`t`)
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("b.jsonl"),
        r#"{"t":"2026-10-22T17:00:02.000Z","kind":"http","conn":2,"seq":1,"method":"GET","target":"/second","route":"backend","status":200}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("a.jsonl"),
        r#"{"t":"2026-10-22T17:00:01.000Z","kind":"http","conn":1,"seq":1,"method":"GET","target":"/first","route":"backend","status":200}"#,
    )
    .unwrap();
    let loaded = load_corpus(temp.path()).unwrap();
    assert_eq!(loaded.len(), 2);
    match (&loaded[0], &loaded[1]) {
        (CaptureRecord::Http(first), CaptureRecord::Http(second)) => {
            assert_eq!(first.target, "/first");
            assert_eq!(second.target, "/second");
        }
        other => panic!("expected sorted HttpRecords, got {other:?}"),
    }
}

#[test]
fn parses_text_body_and_unknown_kind_returns_none() {
    let line = r#"{"t":"2026-10-22T17:00:00.000Z","kind":"http","conn":1,"seq":1,"method":"POST","target":"/raw","route":"backend","req_body":{"text":"hello=world"},"status":200,"res_body":{"text":"<pre>Bad Request</pre>"}}"#;
    let rec = parse_capture_line(line).unwrap();
    match rec {
        CaptureRecord::Http(http) => {
            assert_eq!(http.req_body, CaptureBody::Text("hello=world".into()));
            assert_eq!(http.res_body, CaptureBody::Text("<pre>Bad Request</pre>".into()));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(parse_capture_line(r#"{"kind":"unknown"}"#).is_none());
}

// ---------------------------------------------------------------------------
// 2. Placeholder Substitution Tests
// ---------------------------------------------------------------------------

#[test]
fn substitutes_tokens_keys_gskeys_and_plain_sasl() {
    let mut auth = ReplayTargetAuth {
        jwt_secret: Some("unit-test-hs256-secret".into()),
        signing_key_b64: None,
        gs_key: Some("gs-secret-key-999".into()),
        gateway_secret: Some("gw-secret-42".into()),
        key_map: HashMap::new(),
        deterministic_fallback_keys: false,
    };
    let raw_account_key = "RRK_test_account_key_abcdef123456";
    let key_hash = auth.register_account_key(raw_account_key);
    let now_ms = 1_792_688_400_000;

    // 1. <token:UID-...> -> deterministic HS256 JWT with userId == UID-...
    let sub_token = substitute_str("Bearer <token:UID-alice>", &auth, now_ms);
    assert!(!sub_token.inexact);
    assert!(sub_token.value.starts_with("Bearer eyJ"));
    assert_eq!(normalize_jwts_in_str(&sub_token.value), "Bearer <jwt:UID-alice>");

    // 2. Bare <token> -> default replay user JWT
    let sub_bare = substitute_str("Bearer <token>", &auth, now_ms);
    assert!(!sub_bare.inexact);
    assert_eq!(normalize_jwts_in_str(&sub_bare.value), "Bearer <jwt:UID-replay-anon>");

    // 3. <key:sha256> resolved from key_map -> exact
    let sub_key = substitute_str(&format!("code=<key:{key_hash}>"), &auth, now_ms);
    assert!(!sub_key.inexact);
    assert_eq!(sub_key.value, format!("code={raw_account_key}"));

    // 4. <key:sha256> unknown when deterministic_fallback_keys == false -> inexact
    let unknown_hash = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let sub_unknown = substitute_str(&format!("<key:{unknown_hash}>"), &auth, now_ms);
    assert!(sub_unknown.inexact);

    // 5. <key:sha256> unknown when deterministic_fallback_keys == true -> deterministic RRK_replay_...
    auth.deterministic_fallback_keys = true;
    let sub_fallback = substitute_str(&format!("<key:{unknown_hash}>"), &auth, now_ms);
    assert!(!sub_fallback.inexact);
    assert_eq!(sub_fallback.value, "RRK_replay_ffffffffffffffffffffffff");

    // 6. <gskey> -> gs_key
    let sub_gs = substitute_str("<gskey>", &auth, now_ms);
    assert!(!sub_gs.inexact);
    assert_eq!(sub_gs.value, "gs-secret-key-999");

    // 7. <plain:UID-alice> -> base64("\0UID-alice\0<jwt>")
    let sub_plain = substitute_str("<auth><plain:UID-alice></auth>", &auth, now_ms);
    assert!(!sub_plain.inexact);
    let inner_b64 = sub_plain
        .value
        .strip_prefix("<auth>")
        .and_then(|s| s.strip_suffix("</auth>"))
        .unwrap();
    let decoded = String::from_utf8(BASE64.decode(inner_b64).unwrap()).unwrap();
    let parts: Vec<&str> = decoded.split('\0').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "");
    assert_eq!(parts[1], "UID-alice");
    assert_eq!(normalize_jwts_in_str(parts[2]), "<jwt:UID-alice>");

    // 8. <redacted> marks substitution as inexact
    let sub_redacted = substitute_value(&json!({"secret": "<redacted>"}), &auth, now_ms);
    assert!(sub_redacted.inexact);

    // 9. RS256 signing key support
    let rs256_auth = ReplayTargetAuth::with_test_keys(Some("gs-1"));
    let rs_sub = substitute_str("Bearer <token:UID-bob>", &rs256_auth, now_ms);
    assert!(!rs_sub.inexact);
    assert_eq!(normalize_jwts_in_str(&rs_sub.value), "Bearer <jwt:UID-bob>");
}

#[test]
fn prepare_http_request_encodes_form_and_injects_replay_headers() {
    let mut auth = ReplayTargetAuth::with_test_keys(Some("gs-secret"));
    auth.gateway_secret = Some("gw-secret".into());
    let key_hash = auth.register_account_key("RRK_my_key_123");

    let record = HttpRecord {
        t: "2026-10-22T17:00:00.000Z".into(),
        conn: 3,
        seq: 2,
        ip: "10.0.0.5".into(),
        src: "game".into(),
        method: "POST".into(),
        target: "/account/api/oauth/token".into(),
        route: "backend".into(),
        req_headers: serde_json::Map::from_iter([
            ("x-rustic-isles-gameserver-apikey".into(), json!("<gskey>")),
            ("authorization".into(), json!("basic <redacted>")),
        ]),
        req_body: CaptureBody::Form(vec![
            ("grant_type".into(), "exchange_code".into()),
            ("exchange_code".into(), format!("<key:{key_hash}>")),
        ]),
        status: 200,
        res_headers: serde_json::Map::new(),
        res_body: CaptureBody::Null,
        ms: 1,
        aborted: false,
    };

    let prep = prepare_http_request(&record, &auth, &HashMap::new(), true);
    assert!(
        !prep.inexact,
        "Basic client-credentials header on oauth/token does not mark exchange_code flow inexact"
    );
    let body_str = String::from_utf8(prep.body.clone()).unwrap();
    assert_eq!(body_str, "grant_type=exchange_code&exchange_code=RRK_my_key_123");
    assert!(
        prep.headers
            .iter()
            .any(|(k, v)| k == "x-rustic-isles-gameserver-apikey" && v == "gs-secret")
    );
    assert!(
        prep.headers
            .iter()
            .any(|(k, v)| k == "x-gateway-secret" && v == "gw-secret")
    );
    assert!(
        prep.headers
            .iter()
            .any(|(k, v)| k == "x-isles-replay-now-ms" && v == "1792688400000")
    );
    assert!(
        prep.headers
            .iter()
            .any(|(k, v)| k == "x-isles-replay-seed" && v == "3:2:POST:/account/api/oauth/token")
    );
}

// ---------------------------------------------------------------------------
// 3. Normalizer Rules & Diff Classification Tests
// ---------------------------------------------------------------------------

#[test]
fn normalizer_classifies_status_header_value_text_and_chat_diffs() {
    let cfg = NormalizerConfig::default();

    // 1. JWT normalization: valid HS256 & RS256 tokens -> <jwt:sub>, malformed token -> untouched
    let valid_hs = mint_hs256_jwt("UID-player1", "sec", 1_790_000_000_000).unwrap();
    let test_settings = test_key_settings(&[]);
    let valid_rs = mint_rs256_jwt(
        "UID-player1",
        test_settings.raw("AUTH_SIGNING_PRIVKEY_B64").unwrap(),
        1_790_000_000_000,
    )
    .unwrap();
    assert_eq!(normalize_jwts_in_str(&valid_hs), "<jwt:UID-player1>");
    assert_eq!(normalize_jwts_in_str(&valid_rs), "<jwt:UID-player1>");
    let malformed_jwt = "eyJnot_base64_json.eyJalso_bad.sig";
    assert_eq!(
        normalize_jwts_in_str(malformed_jwt),
        malformed_jwt,
        "malformed JWT must not be masked by normalizer"
    );

    // 2. Volatile headers stripped while content-type & custom headers are preserved
    let mut norm_l = IdNormalizer::new(cfg.clone());
    let mut norm_r = IdNormalizer::new(cfg.clone());
    let left_resp = ReplayHttpResponse {
        status: 200,
        headers: BTreeMap::from([
            ("date".into(), "Wed, 22 Oct 2026 17:00:00 GMT".into()),
            ("etag".into(), "W/\"123-abc\"".into()),
            ("x-powered-by".into(), "Express".into()),
            ("x-rustic-isles-request-id".into(), "req-node-1".into()),
            ("content-type".into(), "application/json; charset=UTF-8".into()),
            ("x-mystic-party-revision".into(), "3".into()),
        ]),
        body_text: format!(
            r#"{{"token":"{valid_hs}","createdAt":"2026-10-22T17:00:00.123Z","id":"11111111-2222-4333-8444-555555555555","instanceId":"a1b2c3d4e5f6a7b8","partyId":"local-a1b2c3d4e5f6a7b8c9d0e1f2_us-east-1","grant":"gs-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee","uptimeSeconds":120,"processId":4412}}"#
        ),
    };
    let right_resp = ReplayHttpResponse {
        status: 200,
        headers: BTreeMap::from([
            ("date".into(), "Thu, 23 Oct 2026 18:00:00 GMT".into()),
            ("content-length".into(), "999".into()),
            ("x-rustic-isles-request-id".into(), "req-rust-2".into()),
            ("content-type".into(), "application/json; charset=utf-8".into()),
            ("x-mystic-party-revision".into(), "3".into()),
        ]),
        body_text: format!(
            r#"{{"token":"{valid_rs}","createdAt":"2026-10-22T17:05:09.999Z","id":"99999999-8888-4777-8666-555555555555","instanceId":"f0e1d2c3b4a59687","partyId":"local-11223344556677889900aabb_us-east-1","grant":"gs-12345678-1234-4123-8123-123456789abc","uptimeSeconds":5,"processId":99001}}"#
        ),
    };
    let diffs = compare_http_responses(&left_resp, &right_resp, &mut norm_l, &mut norm_r);
    assert!(
        diffs.is_empty(),
        "expected zero diffs after normalization, got {diffs:?}"
    );

    // 3. Co-reference preservation: if left reuses UUID #1 and right introduces a new UUID #2, ValueDiff catches it!
    let left_coref = ReplayHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body_text:
            r#"{"primary":"11111111-2222-4333-8444-555555555555","secondary":"11111111-2222-4333-8444-555555555555"}"#
                .into(),
    };
    let right_coref = ReplayHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body_text:
            r#"{"primary":"99999999-8888-4777-8666-555555555555","secondary":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"}"#
                .into(),
    };
    let coref_diffs = compare_http_responses(&left_coref, &right_coref, &mut norm_l, &mut norm_r);
    assert_eq!(
        coref_diffs,
        vec![DiffKind::ValueDiff {
            path: "$.secondary".into(),
            left: json!("<uuid#1>"),
            right: json!("<uuid#2>"),
        }]
    );

    // 4. StatusDiff and HeaderDiff classification
    let status_header_diffs = compare_http_responses(
        &ReplayHttpResponse {
            status: 200,
            headers: BTreeMap::from([("content-type".into(), "application/json".into())]),
            body_text: "{}".into(),
        },
        &ReplayHttpResponse {
            status: 500,
            headers: BTreeMap::from([("content-type".into(), "text/html".into())]),
            body_text: "{}".into(),
        },
        &mut IdNormalizer::new(cfg.clone()),
        &mut IdNormalizer::new(cfg.clone()),
    );
    assert!(status_header_diffs.contains(&DiffKind::StatusDiff { left: 200, right: 500 }));
    assert!(status_header_diffs.contains(&DiffKind::HeaderDiff {
        header: "content-type".into(),
        left: Some("application/json".into()),
        right: Some("text/html".into()),
    }));

    // 5. TextDiff classification when JSON values are semantically equal but key order differs
    let key_order_diffs = compare_http_responses(
        &ReplayHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body_text: r#"{"alpha":1,"beta":2}"#.into(),
        },
        &ReplayHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body_text: r#"{"beta":2,"alpha":1}"#.into(),
        },
        &mut IdNormalizer::new(cfg.clone()),
        &mut IdNormalizer::new(cfg),
    );
    assert_eq!(
        key_order_diffs,
        vec![DiffKind::TextDiff {
            left: r#"{"alpha":1,"beta":2}"#.into(),
            right: r#"{"beta":2,"alpha":1}"#.into(),
        }]
    );

    // 6. ChatStanzaDiff and keepalive ping / barrier filtering
    assert!(is_filtered_chat_stanza(
        "<iq from='chat.steelyard.ca' to='u@chat.steelyard.ca/g' id='srv-1' type='get'><ping xmlns='urn:xmpp:ping'/></iq>"
    ));
    assert!(is_filtered_chat_stanza("<iq type='result' id='replay-barrier-1'/>"));
    assert_eq!(
        normalize_chat_stanza("<stream:stream from='chat.steelyard.ca' id='abc-123' version='1.0'>"),
        "<stream:stream from='chat.steelyard.ca' id='<stream>' version='1.0'>"
    );
    let chat_diffs_ok = compare_chat_stanzas(
        9,
        &[
            "<stream:stream from='chat' id='s1'>".into(),
            "<iq id='srv-111' type='get'><ping xmlns='urn:xmpp:ping'/></iq>".into(),
            "<iq id='srv-222' type='result'/>".into(),
        ],
        &[
            "<stream:stream from='chat' id='s2'>".into(),
            "<iq id='srv-999' type='result'/>".into(),
        ],
    );
    assert!(chat_diffs_ok.is_empty());

    let chat_diffs_bad = compare_chat_stanzas(9, &["<presence from='a'/>".into()], &["<presence from='b'/>".into()]);
    assert_eq!(
        chat_diffs_bad,
        vec![DiffKind::ChatStanzaDiff {
            conn: 9,
            index: 0,
            left: Some("<presence from='a'/>".into()),
            right: Some("<presence from='b'/>".into()),
        }]
    );
}

#[test]
fn normalize_headers_strips_ignored_and_canonicalizes_content_type() {
    let mut norm = IdNormalizer::default();
    let raw = BTreeMap::from([
        ("Date".into(), "Wed, 22 Oct 2026 17:00:00 GMT".into()),
        ("Content-Type".into(), "Application/JSON ; Charset=UTF-8".into()),
        ("X-Custom".into(), "11111111-2222-4333-8444-555555555555".into()),
    ]);
    let norm_h = normalize_headers(&raw, &mut norm);
    assert!(!norm_h.contains_key("date"));
    assert_eq!(
        norm_h.get("content-type").map(String::as_str),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(norm_h.get("x-custom").map(String::as_str), Some("<uuid#1>"));
}

#[test]
fn learn_ids_maps_recorded_character_and_instance_ids() {
    let recorded = json!({
        "payload": {
            "id": "65f1a2b3c4d5e6f7a8b9c0d1",
            "instanceId": "aaaa1111bbbb2222"
        }
    });
    let actual = json!({
        "payload": {
            "id": "999988887777666655554444",
            "instanceId": "cccc3333dddd4444"
        }
    });
    let mut map = HashMap::new();
    learn_ids(&recorded, &actual, &mut map);
    assert_eq!(
        map.get("65f1a2b3c4d5e6f7a8b9c0d1").map(String::as_str),
        Some("999988887777666655554444")
    );
    assert_eq!(
        map.get("aaaa1111bbbb2222").map(String::as_str),
        Some("cccc3333dddd4444")
    );
}

// ---------------------------------------------------------------------------
// 4. CLI Argument Parsing & Key Map Tests
// ---------------------------------------------------------------------------

#[test]
fn cli_parses_replay_subcommand_and_loads_key_map() {
    let cmd = parse_args([
        "replay",
        "--corpus",
        "captures/session1.jsonl",
        "--left",
        "recorded",
        "--right",
        "http://127.0.0.1:61000",
        "--right-director",
        "http://127.0.0.1:61001",
        "--gs-key",
        "gs-123",
        "--jwt-secret",
        "jwt-sec",
        "--gateway-secret",
        "gw-sec",
        "--filter",
        "/character",
        "--out",
        "report.json",
        "--json",
    ])
    .unwrap();

    assert_eq!(
        cmd,
        Command::Replay {
            opts: ReplayOptions {
                corpus: "captures/session1.jsonl".into(),
                left: "recorded".into(),
                right: "http://127.0.0.1:61000".into(),
                left_director: None,
                right_director: Some("http://127.0.0.1:61001".into()),
                left_chat: None,
                right_chat: None,
                left_jwt_secret: Some("jwt-sec".into()),
                right_jwt_secret: Some("jwt-sec".into()),
                left_signing_key: None,
                right_signing_key: None,
                left_gs_key: Some("gs-123".into()),
                right_gs_key: Some("gs-123".into()),
                gateway_secret: Some("gw-sec".into()),
                key_map_path: None,
                pin_clock: true,
                filter: Some("/character".into()),
                out_path: Some("report.json".into()),
            },
            json: true,
        }
    );

    let temp = tempfile::tempdir().unwrap();
    let key_map_file = temp.path().join("keys.json");
    std::fs::write(&key_map_file, r#"["RRK_alpha_key","UUK_beta_key"]"#).unwrap();
    let loaded_map = load_key_map(&key_map_file).unwrap();
    assert_eq!(loaded_map.len(), 2);
    assert!(loaded_map.values().any(|v| v == "RRK_alpha_key"));
    assert!(loaded_map.values().any(|v| v == "UUK_beta_key"));
}

// ---------------------------------------------------------------------------
// 5. End-to-End Target-Against-Target & Golden Node Capture Replay
// ---------------------------------------------------------------------------

const TEST_STARTER_STACKED: &[&str] = &[
    "TOKEN_DAILY_CHALLENGE_DRAFT",
    "QI_BASIC_FLARE_DURABLE",
    "EM_INTRO_BEGINNER_01",
];

const TEST_TABLES_1147: VersionTables = VersionTables {
    target_changelist: 647_472,
    game_version: "1.14.7",
    active_huntpass_id: "season43",
    recent_huntpass_seasons: &[
        "season43", "season44", "season45", "season46", "season47", "season48", "season49", "season50",
    ],
    patchnotes_release_version: "",
    bounty_item_grant_data: &[
        ItemGrantWindow {
            item_id: "TOKEN_BOUNTY_DRAFT_PREMIUM",
            amount: 1,
            start: "2026-01-01T00:00:00.000Z",
            end: "2027-12-31T00:00:00.000Z",
        },
        ItemGrantWindow {
            item_id: "CURRENCY_PRESTIGE",
            amount: 5,
            start: "2026-01-01T00:00:00.000Z",
            end: "2027-12-31T00:00:00.000Z",
        },
    ],
    store_sections: &[
        "ladyluckstore",
        "season_store",
        "webstore",
        "dyes",
        "hp_level_skip_rank",
        "huntpass_store",
        "exchange_vendor_slot_2",
        "exchange_vendor_slot_3",
        "fountain_daily_free_bundle",
    ],
    seasonal_coin: Some(SeasonalCoin {
        catalog_id: "CURRENCY_SEASONAL_COIN",
        max_per_claim: 1000,
    }),
    masked_journey_nodes: &[MaskedJourneyNode {
        node: "Gauntlet_Unlock",
        unless: "RUSTIC_ISLES_GAUNTLET",
    }],
    starter_manifest: StarterManifest {
        bootstrap_version: "starter-1.12-v5",
        stacked: TEST_STARTER_STACKED,
        quick_emojis: None,
    },
    switch_defaults: &[
        SwitchDefault {
            switch: QOS_PORT,
            value: "80",
        },
        SwitchDefault {
            switch: REFORGE,
            value: "on",
        },
        SwitchDefault {
            switch: MASTERY_PERSISTENCE,
            value: "on",
        },
        SwitchDefault {
            switch: BOUNTY_PERSISTENCE,
            value: "on",
        },
        SwitchDefault {
            switch: COOLDOWN_PERSISTENCE,
            value: "on",
        },
    ],
};

struct TestHooks1147;

impl VersionHooks for TestHooks1147 {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        NoPlayerPolicy::ConfiguredAccount
    }

    fn game_session_payload(&self, token: &str) -> Value {
        json!({ "error_code": null, "sessionid": token, "sessiontoken": token, "sessionToken": token })
    }

    fn weapons_at_base_level(&self, _settings: &Settings) -> bool {
        true
    }

    fn reforge_rules(&self) -> bool {
        true
    }

    fn mastery_persistence(&self) -> bool {
        true
    }

    fn huntpass_payload(&self, id: &str) -> Value {
        json!({ "huntpass_id": id, "season": id })
    }

    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        json!({
            "phx_account_id": user_id,
            "objective_id": row.objective_id,
            "progress": row.value,
            "completed_count": row.completed_count,
            "created_date": row.created_date,
            "last_modified_date": row.last_modified_date,
        })
    }

    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::RollingHuntpassWindows
    }

    fn survey_config(&self) -> StubReply {
        StubReply {
            status: 200,
            body: Some(json!({ "code": null, "message": "OK", "payload": { "Triggers": [] } })),
        }
    }

    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        let entitlements: Vec<Value> = list
            .iter()
            .map(|e| json!({ "activatedDate": e.activated_date, "duration": e.duration, "name": e.name }))
            .collect();
        json!({ "entitlements": entitlements })
    }

    fn candidate_destination(&self, hunt_id: &str, _realm_hub: bool) -> Destination {
        if hunt_id == "ShatteredIsles_ReturnToRamsgate" {
            Destination {
                game_mode: GameMode::City,
                hunt_id: None,
            }
        } else {
            Destination {
                game_mode: GameMode::Island,
                hunt_id: Some(hunt_id.to_owned()),
            }
        }
    }
}

const TEST_GS_KEY: &str = "gs-replay-test-key-1147";
const TEST_RAW_API_KEY: &str = "RRK_replay_slayer_key_0123456789abcdef";
const TEST_USER_ID: &str = "UID-a1b2c3d4e5f6a7b8c9d0e1f2";

async fn build_live_1147_server() -> (
    String,
    String,
    ReplayTargetAuth,
    tempfile::TempDir,
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("isles.sqlite");
    let game_data_dir = dir.path().join("game_data");
    std::fs::create_dir_all(&game_data_dir).unwrap();

    // Seed progression_config.json and inventory_storage_policy.json so progression and inventory routes work deterministically
    std::fs::write(
        game_data_dir.join("progression_config.json"),
        r#"{"code":null,"message":"OK","payload":{"paths":[{"progression_id":"ExperienceTrack_Weapon_Axe","requirements":[{"rank_id":1,"xp_required":100},{"rank_id":2,"xp_required":200}],"prestige":{"xp_per_level":1000}}]}}"#,
    )
    .unwrap();
    std::fs::write(
        game_data_dir.join("inventory_storage_policy.json"),
        r#"{"entries":{"id_item_axe_0001_00":{"storage":"instanced","maxQuantity":1,"tags":["weapon"]},"CURRENCY_NOTES":{"storage":"stacked","maxQuantity":999999999,"tags":["currency"]}}}"#,
    )
    .unwrap();

    let node = NodeIdentity::standalone();
    let home = Arc::new(module_api::network::StandaloneHome { identity: node.clone() });
    let store = IslesStore::open(
        &DatabaseConfig::Sqlite { file: db_path },
        &DatabaseOptions::default(),
        home,
        &node,
    )
    .unwrap();

    let settings = test_key_settings(&[
        ("AUTH_MODE", "APIKEY"),
        ("GAMESERVER_API_KEYS", TEST_GS_KEY),
        ("RUSTIC_ISLES_ADMIN_API", "on"),
    ]);

    // Seed player account and API key
    let account_id = AccountId::new(&node.node);
    let key_hash = hash_api_key(&settings, TEST_RAW_API_KEY, KeyScope::User).unwrap();
    let acct_clone = account_id.clone();
    store
        .write(&system_actor(), move |tx, repos| {
            repos.accounts.create(
                tx,
                &AccountRecord {
                    id: acct_clone.clone(),
                    user_id: UserId(TEST_USER_ID.into()),
                    name: "ReplaySlayer".into(),
                    notes: 1000,
                },
            )?;
            repos.api_keys.insert_user_key_hash(tx, &acct_clone, &key_hash)?;
            Ok(())
        })
        .await
        .unwrap();

    let version = IslesVersion {
        routes: RouteSet::V1147,
        hooks: Arc::new(TestHooks1147),
        tables: TEST_TABLES_1147,
    };
    let mut shared = Shared::bare(version);
    shared.settings = settings;
    shared.store = Some(store);
    shared.game_data = GameDataRoot::from_config(game_data_dir.to_str(), None);
    let shared = Arc::new(shared);

    isles_core::areas::auth::start(&shared).await.unwrap();

    let backend_router = app(shared.clone(), Service::Backend, &isles_core::areas::backend()).unwrap();

    // Build Director router on Service::Director
    let mut director_handlers = Handlers::default();
    director_handlers.on(Service::Director, Method::Get, "/health/ready", |_call| {
        Box::pin(async { json_status_response(200, json!({"ready": true, "hubsReady": true})) })
    });
    director_handlers.on(Service::Director, Method::Get, "/api/hub-status", |_call| {
        Box::pin(async {
            json_status_response(
                200,
                json!({
                    "ramsgate": {"ready": true, "port": 9000, "players": 0},
                    "trainingDojo": {"ready": true, "port": 9001, "players": 0}
                }),
            )
        })
    });
    director_handlers.on(Service::Director, Method::Get, "/api/instances", |_call| {
        Box::pin(async { json_status_response(200, json!([])) })
    });
    director_handlers.on(
        Service::Director,
        Method::Post,
        "/api/matchmaker/handle-matchmaking-for-player",
        |_call| {
            Box::pin(async {
                json_status_response(
                    200,
                    json!({
                        "ip": "127.0.0.1",
                        "port": 9000,
                        "reused": true
                    }),
                )
            })
        },
    );
    let director_router = app(shared, Service::Director, &director_handlers).unwrap();

    let (backend_url, stop_backend, _backend_task) = serve_ephemeral(backend_router).await.unwrap();
    let (director_url, stop_director, _director_task) = serve_ephemeral(director_router).await.unwrap();

    let mut target_auth = ReplayTargetAuth::with_test_keys(Some(TEST_GS_KEY));
    target_auth.register_account_key(TEST_RAW_API_KEY);

    (backend_url, director_url, target_auth, dir, stop_backend, stop_director)
}

#[tokio::test(flavor = "multi_thread")]
async fn target_against_target_and_golden_node_corpus_replay_with_zero_diffs() {
    let (left_url, left_dir_url, left_auth, _left_tmp, _stop_lb, _stop_ld) = build_live_1147_server().await;
    let (right_url, right_dir_url, right_auth, _right_tmp, _stop_rb, _stop_rd) = build_live_1147_server().await;

    let mut key_probe = ReplayTargetAuth::default();
    let key_hash = key_probe.register_account_key(TEST_RAW_API_KEY);

    // Build a multi-step capture corpus covering:
    // 1. GET /reforged/api/ServerStatus
    // 2. POST /account/api/oauth/token (with <key:sha256> exchange_code -> mints real RS256 JWT)
    // 3. POST /login (with <token:UID-...> -> session supply check & TicketRateOk)
    // 4. PUT /gamesession/epic (echoes sessionToken JWT)
    // 5. GET /character (auto-provisions starter character with deterministic ID when clock+seed are pinned)
    // 6. PUT /character (updates character data JSON)
    // 7. POST /character (creates a second character and learns its dynamic characterId)
    // 8. GET /loadout/:userId/:characterId/all
    // 9. POST /loadout/:userId/:characterId/0
    // 10. POST /loadout/:userId/:characterId/active/0
    // 11. GET /inventory/:userId/:characterId
    // 12. POST /inventory (grants stacked currency & instanced item)
    // 13. GET /progression/:userId
    // 14. POST /progression/:userId/id_progression_axe/150 (with gs-<uuid> grantId)
    // 15. GET /huntpass/:userId
    // 16. GET /escalation/Escalation_Season_00/:userId
    // 17. GET /bounty/game-data
    // 18. GET /bounty/:userId
    // 19. POST /bounty/:userId (saves active/draft/completed bounties as game server)
    // 20. GET /balance
    // 21. GET /product/skus/public?requiredTags=exchange_vendor_slot_2
    // 22. GET /token/id_currency_platinum/single_exchange_slot_2 (mints store purchaseToken JWT)
    // 23. POST /candidate/player/register
    // 24. POST /candidate/player/alive
    // 25. GET /candidate/regions
    // 26. POST /party (creates party with dynamic partyId)
    // 27. GET /party/invites
    // 28. Director: GET /health/ready, GET /api/hub-status, GET /api/instances, POST /api/matchmaker/handle-matchmaking-for-player
    // 29. Chat: XMPP stream open, <auth><plain:UID-...></auth>, <success/>, roster IQ, and filtered keepalive ping
    let placeholder_char_id = "65f000000000000000000001";
    let corpus_lines = vec![
        json!({
            "t": "2026-10-22T17:00:01.000Z", "kind": "http", "conn": 1, "seq": 1, "ip": "127.0.0.1",
            "method": "GET", "target": "/reforged/api/ServerStatus", "route": "backend",
            "req_headers": {}, "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:02.000Z", "kind": "http", "conn": 1, "seq": 2, "ip": "127.0.0.1",
            "method": "POST", "target": "/account/api/oauth/token", "route": "backend",
            "req_headers": {"content-type": "application/x-www-form-urlencoded"},
            "req_body": {"form": [["grant_type", "exchange_code"], ["exchange_code", format!("<key:{key_hash}>")]]},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:03.000Z", "kind": "http", "conn": 1, "seq": 3, "ip": "127.0.0.1",
            "method": "POST", "target": "/login", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>"), "content-type": "application/json"},
            "req_body": {"json": {"email": TEST_USER_ID}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:04.000Z", "kind": "http", "conn": 1, "seq": 4, "ip": "127.0.0.1",
            "method": "PUT", "target": "/gamesession/epic", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null,
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:05.000Z", "kind": "http", "conn": 1, "seq": 5, "ip": "127.0.0.1",
            "method": "GET", "target": format!("/character?userId={TEST_USER_ID}&version=1"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null,
            "status": 200, "res_headers": {},
            "res_body": {"json": [{"id": placeholder_char_id}]}
        }),
        json!({
            "t": "2026-10-22T17:00:06.000Z", "kind": "http", "conn": 1, "seq": 6, "ip": "127.0.0.1",
            "method": "POST", "target": "/character", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>"), "content-type": "application/json"},
            "req_body": {"json": {"characterId": placeholder_char_id, "updateVersion": 1, "data": "{\"PrimaryDye\":2}"}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:07.000Z", "kind": "http", "conn": 1, "seq": 7, "ip": "127.0.0.1",
            "method": "GET", "target": format!("/loadout/{TEST_USER_ID}/{placeholder_char_id}/all"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:08.000Z", "kind": "http", "conn": 1, "seq": 8, "ip": "127.0.0.1",
            "method": "POST", "target": format!("/loadout/{TEST_USER_ID}/{placeholder_char_id}/0"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>"), "content-type": "application/json"},
            "req_body": {"json": {"data": "{\"PresetIndex\":0,\"Weapon\":\"Axe\"}"}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:09.000Z", "kind": "http", "conn": 1, "seq": 9, "ip": "127.0.0.1",
            "method": "POST", "target": format!("/loadout/{TEST_USER_ID}/{placeholder_char_id}/active/0"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:10.000Z", "kind": "http", "conn": 1, "seq": 10, "ip": "127.0.0.1",
            "method": "GET", "target": format!("/inventory/{TEST_USER_ID}/{placeholder_char_id}"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:11.000Z", "kind": "http", "conn": 1, "seq": 11, "ip": "127.0.0.1",
            "method": "POST", "target": "/inventory", "route": "backend",
            "req_headers": {"x-rustic-isles-gameserver-apikey": "<gskey>", "content-type": "application/json"},
            "req_body": {"json": {
                "accountId": TEST_USER_ID,
                "characterId": placeholder_char_id,
                "transactionId": "txn-replay-001",
                "addInstancedItems": [{"catalogId": "id_item_axe_0001_00", "instanceId": "inst-axe-0001", "itemData": "{}"}],
                "addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 250}],
                "removeInstancedItems": [],
                "removeStackedItems": [],
                "saveInstancedItems": []
            }},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:12.000Z", "kind": "http", "conn": 1, "seq": 12, "ip": "127.0.0.1",
            "method": "GET", "target": format!("/progression/{TEST_USER_ID}"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:13.000Z", "kind": "http", "conn": 1, "seq": 13, "ip": "127.0.0.1",
            "method": "POST", "target": format!("/progression/{TEST_USER_ID}/ExperienceTrack_Weapon_Axe/150"), "route": "backend",
            "req_headers": {"x-rustic-isles-gameserver-apikey": "<gskey>", "content-type": "application/json"},
            "req_body": {"json": {"grantId": "gs-11111111-2222-4333-8444-555555555555"}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:14.000Z", "kind": "http", "conn": 1, "seq": 14, "ip": "127.0.0.1",
            "method": "GET", "target": format!("/huntpass/{TEST_USER_ID}"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:15.000Z", "kind": "http", "conn": 1, "seq": 15, "ip": "127.0.0.1",
            "method": "GET", "target": format!("/escalation/Escalation_Season_00/{TEST_USER_ID}"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:16.000Z", "kind": "http", "conn": 1, "seq": 16, "ip": "127.0.0.1",
            "method": "GET", "target": "/bounty/game-data", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:17.000Z", "kind": "http", "conn": 1, "seq": 17, "ip": "127.0.0.1",
            "method": "GET", "target": format!("/bounty/{TEST_USER_ID}"), "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:18.000Z", "kind": "http", "conn": 1, "seq": 18, "ip": "127.0.0.1",
            "method": "POST", "target": format!("/bounty/{TEST_USER_ID}"), "route": "backend",
            "req_headers": {"x-rustic-isles-gameserver-apikey": "<gskey>", "content-type": "application/json"},
            "req_body": {"json": {
                "bounties": [{"bounty_id": "bounty_slay_behemoths_01", "objectives": [{"objective_id": "obj_1", "progress": 2}]}]
            }},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:19.000Z", "kind": "http", "conn": 1, "seq": 19, "ip": "127.0.0.1",
            "method": "GET", "target": "/balance", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:20.000Z", "kind": "http", "conn": 1, "seq": 20, "ip": "127.0.0.1",
            "method": "GET", "target": "/product/skus/public?requiredTags=exchange_vendor_slot_2", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:21.000Z", "kind": "http", "conn": 1, "seq": 21, "ip": "127.0.0.1",
            "method": "GET", "target": "/token/id_currency_platinum/single_exchange_slot_2", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:22.000Z", "kind": "http", "conn": 1, "seq": 22, "ip": "127.0.0.1",
            "method": "POST", "target": "/candidate/player/register", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>"), "content-type": "application/json"},
            "req_body": {"json": {"clientInfo": {"buildVersion": "1.14.7"}}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:23.000Z", "kind": "http", "conn": 1, "seq": 23, "ip": "127.0.0.1",
            "method": "POST", "target": "/candidate/player/alive", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>"), "content-type": "application/json"},
            "req_body": {"json": {}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:24.000Z", "kind": "http", "conn": 1, "seq": 24, "ip": "127.0.0.1",
            "method": "GET", "target": "/candidate/regions", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:25.000Z", "kind": "http", "conn": 1, "seq": 25, "ip": "127.0.0.1",
            "method": "POST", "target": "/party", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>"), "content-type": "application/json"},
            "req_body": {"json": {"region": "us-east-1"}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:26.000Z", "kind": "http", "conn": 1, "seq": 26, "ip": "127.0.0.1",
            "method": "GET", "target": "/party/invites", "route": "backend",
            "req_headers": {"authorization": format!("Bearer <token:{TEST_USER_ID}>")},
            "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:27.000Z", "kind": "http", "conn": 2, "seq": 1, "ip": "127.0.0.1",
            "method": "GET", "target": "/health/ready", "route": "director",
            "req_headers": {}, "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:28.000Z", "kind": "http", "conn": 2, "seq": 2, "ip": "127.0.0.1",
            "method": "GET", "target": "/api/hub-status", "route": "director",
            "req_headers": {}, "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:29.000Z", "kind": "http", "conn": 2, "seq": 3, "ip": "127.0.0.1",
            "method": "GET", "target": "/api/instances", "route": "director",
            "req_headers": {}, "req_body": null, "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:30.000Z", "kind": "http", "conn": 2, "seq": 4, "ip": "127.0.0.1",
            "method": "POST", "target": "/api/matchmaker/handle-matchmaking-for-player", "route": "director",
            "req_headers": {"content-type": "application/json"},
            "req_body": {"json": {"mapName": "/Game/Maps/ramsgate/ramsgate_01_persistent", "partyId": "local-0001_us-east-1"}},
            "status": 200, "res_headers": {}, "res_body": null
        }),
        json!({
            "t": "2026-10-22T17:00:31.000Z", "kind": "chat", "conn": 10, "ip": "127.0.0.1", "event": "open"
        }),
        json!({
            "t": "2026-10-22T17:00:31.010Z", "kind": "chat", "conn": 10, "dir": "in",
            "text": format!("<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'><plain:{TEST_USER_ID}></auth>")
        }),
        json!({
            "t": "2026-10-22T17:00:31.020Z", "kind": "chat", "conn": 10, "dir": "out",
            "text": "<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>"
        }),
        json!({
            "t": "2026-10-22T17:00:31.030Z", "kind": "chat", "conn": 10, "dir": "out",
            "text": "<iq from='chat.steelyard.ca' id='srv-keepalive-1' type='get'><ping xmlns='urn:xmpp:ping'/></iq>"
        }),
    ];

    let jsonl_str = corpus_lines
        .iter()
        .map(|v| serde_json::to_string(v).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let records = parse_capture_str(&jsonl_str);
    assert_eq!(records.len(), 34);

    // Run 1: Live Target A vs Live Target B
    let live_config = ReplayConfig {
        left: ReplayTarget {
            name: "left-1147".into(),
            endpoint: ReplayEndpoint::Http {
                base_url: left_url.clone(),
                director_url: Some(left_dir_url.clone()),
                chat_addr: None,
            },
            auth: left_auth.clone(),
            pin_clock: true,
        },
        right: ReplayTarget {
            name: "right-1147".into(),
            endpoint: ReplayEndpoint::Http {
                base_url: right_url.clone(),
                director_url: Some(right_dir_url.clone()),
                chat_addr: None,
            },
            auth: right_auth.clone(),
            pin_clock: true,
        },
        normalizer: NormalizerConfig::default(),
        filter: None,
    };

    let report = run_replay(&records, &live_config).await.unwrap();
    assert!(
        report.passed(),
        "expected zero unexplained diffs in target-vs-target replay, got report: {report:#?}"
    );
    assert_eq!(report.http_replayed, 30);
    assert_eq!(report.chat_connections_replayed, 1);
    assert_eq!(report.unexplained_diffed, 0);
    assert_eq!(report.status_diffs, 0);
    assert_eq!(report.header_diffs, 0);
    assert_eq!(report.value_diffs, 0);
    assert_eq!(report.text_diffs, 0);
    assert_eq!(report.chat_stanza_diffs, 0);
    for entry in &report.entries {
        if entry.kind == "http" {
            assert!(
                matches!(entry.left_status, Some(200 | 204)),
                "entry {} ({}) returned non-2xx status {:?}",
                entry.index,
                entry.target,
                entry.left_status
            );
        }
    }

    // Run 2: Verify `run_replay_cli` writes the JSON report file and passes
    let cli_dir = tempfile::tempdir().unwrap();
    let corpus_file = cli_dir.path().join("capture.jsonl");
    let key_map_file = cli_dir.path().join("keymap.json");
    let out_file = cli_dir.path().join("report.json");
    std::fs::write(&corpus_file, &jsonl_str).unwrap();
    std::fs::write(&key_map_file, serde_json::to_string(&vec![TEST_RAW_API_KEY]).unwrap()).unwrap();

    let (fresh_l_url, fresh_l_dir, fresh_l_auth, _fl_tmp, _stop_flb, _stop_fld) = build_live_1147_server().await;
    let (fresh_r_url, fresh_r_dir, _fresh_r_auth, _fr_tmp, _stop_frb, _stop_frd) = build_live_1147_server().await;

    let cli_opts = ReplayOptions {
        corpus: corpus_file,
        left: fresh_l_url,
        right: fresh_r_url,
        left_director: Some(fresh_l_dir),
        right_director: Some(fresh_r_dir),
        left_chat: None,
        right_chat: None,
        left_jwt_secret: None,
        right_jwt_secret: None,
        left_signing_key: fresh_l_auth.signing_key_b64.clone(),
        right_signing_key: fresh_l_auth.signing_key_b64,
        left_gs_key: Some(TEST_GS_KEY.into()),
        right_gs_key: Some(TEST_GS_KEY.into()),
        gateway_secret: None,
        key_map_path: Some(key_map_file),
        pin_clock: true,
        filter: None,
        out_path: Some(out_file.clone()),
    };
    let cli_report = run_replay_cli(&cli_opts).await.unwrap();
    assert!(cli_report.passed());
    assert!(out_file.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn recorded_corpus_and_live_xmpp_chat_replay_with_zero_diffs() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Start an ephemeral mock XMPP server that accepts a connection, reads the <auth> stanza,
    // and replies with <success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/> + barrier reply when needed.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let chat_addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            while let Ok(n) = sock.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buf[..n]);
                if text.contains("<auth ") {
                    let _ = sock
                        .write_all(b"<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>")
                        .await;
                }
                if let Some(pos) = text.find("id='replay-barrier-") {
                    let rest = &text[pos + 4..];
                    if let Some(end) = rest.find('\'') {
                        let barrier_id = &rest[..end];
                        let reply = format!("<iq type='result' id='{barrier_id}'/>");
                        let _ = sock.write_all(reply.as_bytes()).await;
                    }
                }
            }
        }
    });

    let (right_url, right_dir_url, right_auth, _right_tmp, _stop_rb, _stop_rd) = build_live_1147_server().await;

    let golden_lines = [
        json!({
            "t": "2026-10-22T17:00:01.000Z", "kind": "http", "conn": 1, "seq": 1, "ip": "127.0.0.1",
            "method": "GET", "target": "/reforged/api/ServerStatus", "route": "backend",
            "req_headers": {}, "req_body": null,
            "status": 200,
            "res_headers": {
                "content-type": "application/json; charset=utf-8",
                "cache-control": "no-store",
                "ratelimit": "\"600-in-1min\"; r=599; t=60",
                "ratelimit-policy": "\"600-in-1min\"; q=600; w=60; pk=:YjU5MDczYThlN2Fj:"
            },
            "res_body": {"json": {
                "name": "DaintyLass server",
                "online": true,
                "version": "0.1.0",
                "commit": "",
                "sourceUrl": null,
                "registration": "INVITECODE",
                "uptimeSeconds": 999,
                "contentPort": null,
                "game": {
                    "build": "rel-1.14.7-Archon",
                    "changelist": 647472,
                    "executableSha256": null
                },
                "gameData": "missing",
                "chat": false,
                "playersOnline": 0,
                "players": [],
                "instances": [],
                "limited": true
            }}
        }),
        json!({
            "t": "2026-10-22T17:00:02.000Z", "kind": "http", "conn": 1, "seq": 2, "ip": "127.0.0.1",
            "method": "GET", "target": "/health/ready", "route": "director",
            "req_headers": {}, "req_body": null,
            "status": 200,
            "res_headers": {"content-type": "application/json; charset=utf-8"},
            "res_body": {"json": {"ready": true, "hubsReady": true}}
        }),
        json!({
            "t": "2026-10-22T17:00:03.000Z", "kind": "chat", "conn": 55, "ip": "127.0.0.1", "event": "open"
        }),
        json!({
            "t": "2026-10-22T17:00:03.010Z", "kind": "chat", "conn": 55, "dir": "in",
            "text": format!("<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'><plain:{TEST_USER_ID}></auth>")
        }),
        json!({
            "t": "2026-10-22T17:00:03.020Z", "kind": "chat", "conn": 55, "dir": "out",
            "text": "<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'/>"
        }),
        json!({
            "t": "2026-10-22T17:00:03.030Z", "kind": "chat", "conn": 55, "event": "close"
        }),
    ];

    let jsonl = golden_lines
        .iter()
        .map(|v| serde_json::to_string(v).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let records = parse_capture_str(&jsonl);

    let cfg = ReplayConfig {
        left: ReplayTarget::recorded(),
        right: ReplayTarget {
            name: "rust-live".into(),
            endpoint: ReplayEndpoint::Http {
                base_url: right_url,
                director_url: Some(right_dir_url),
                chat_addr: Some(chat_addr),
            },
            auth: right_auth,
            pin_clock: true,
        },
        normalizer: NormalizerConfig::default(),
        filter: None,
    };

    let report = run_replay(&records, &cfg).await.unwrap();
    assert!(
        report.passed(),
        "expected recorded vs live + XMPP chat replay to pass with zero diffs, got: {report:#?}"
    );
    assert_eq!(report.http_replayed, 2);
    assert_eq!(report.chat_connections_replayed, 1);
    assert_eq!(report.unexplained_diffed, 0);
}

#[test]
fn aligns_gateway_redacted_fields_without_masking_structural_or_non_secret_diffs() {
    let headers = BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
    let recorded = ReplayHttpResponse {
        status: 200,
        headers: headers.clone(),
        body_text: serde_json::to_string(&json!({
            "game": {
                "executableSha256": "<redacted>"
            },
            "token_type": "<redacted>",
            "refresh_token": "<redacted>",
            "payload": {
                "huntpass_id": "<redacted>",
                "bounty_token_id": "<redacted>"
            }
        }))
        .unwrap(),
    };
    let live = ReplayHttpResponse {
        status: 200,
        headers: headers.clone(),
        body_text: serde_json::to_string(&json!({
            "game": {
                "executableSha256": "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b"
            },
            "token_type": "bearer",
            "refresh_token": "opaque_refresh_secret_1234567890",
            "payload": {
                "huntpass_id": "season43",
                "bounty_token_id": "TOKEN_BOUNTY_DRAFT"
            }
        }))
        .unwrap(),
    };
    let mut left_norm = IdNormalizer::default();
    let mut right_norm = IdNormalizer::default();
    let diffs = compare_http_responses(&recorded, &live, &mut left_norm, &mut right_norm);
    assert!(diffs.is_empty(), "expected zero diffs, got: {diffs:?}");

    // Non-secret field with <redacted> must still produce a ValueDiff
    let non_secret_recorded = ReplayHttpResponse {
        status: 200,
        headers: headers.clone(),
        body_text: r#"{"displayName":"<redacted>"}"#.to_owned(),
    };
    let non_secret_live = ReplayHttpResponse {
        status: 200,
        headers,
        body_text: r#"{"displayName":"CanarySlayer"}"#.to_owned(),
    };
    let diffs2 = compare_http_responses(
        &non_secret_recorded,
        &non_secret_live,
        &mut IdNormalizer::default(),
        &mut IdNormalizer::default(),
    );
    assert_eq!(diffs2.len(), 1, "non-secret <redacted> field must still diff");
}
