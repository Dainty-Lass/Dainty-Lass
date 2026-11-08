// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The composed module (spec §12.2): driven with no database in and out of process, chat, the version data at
//! start, and the accounts core and realms adapter.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dainty_module_api::chat::{ChatQuery, PRESENCE_EVENT};
use dainty_module_api::host::file_sha256;
use dainty_module_api::stub::RecordingHost;
use dainty_module_api::{
    ChildModule, ChildOptions, ErrorKind, ExternalModule, GameModule, GlobalId, IdKind, InProcessModule, ModuleConfig,
    ModuleContext, ModuleId, NodeIdentity, Phase, PortRange,
};
use rustic_isles::routes::all;
use rustic_isles::testing::{self, Mounts};
use rustic_isles::{IslesModule, Settings};
use rustic_isles_1_14_7::test_support::{area_handlers, synthetic_document};
use rustic_isles_1_14_7::{AccountsCore, RealmsAdapter, module, spec, spec_with};
use rustic_rifts_seam::{AccountId, GuardMode, IslesCore};

fn mounts(admin_api: bool, rifts_loaded: bool) -> Mounts {
    let areas = area_handlers();
    let handled = all()
        .iter()
        .filter(|row| areas.contains(row.service, row.method, row.path))
        .map(|row| (row.service, row.method, row.path))
        .collect();
    Mounts {
        handled,
        admin_api,
        rifts_loaded,
    }
}

fn env_file(dir: &Path, lines: &[&str]) -> PathBuf {
    let path = dir.join("backend.env");
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drive_without_a_database_in_process() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_file(dir.path(), &["RUSTIC_ISLES_ADMIN_API=on"]);
    let host = dainty_module_api::stub::recording_host();
    let driver = InProcessModule::new(Arc::new(module().unwrap()), NodeIdentity::standalone(), host.clone()).unwrap();
    testing::drive(&driver, &host, Some(&env), &mounts(true, cfg!(feature = "realms")))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drive_without_a_database_out_of_process() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_file(dir.path(), &["RUSTIC_ISLES_ADMIN_API=on"]);
    let path = PathBuf::from(env!("CARGO_BIN_EXE_rustic-isles-1-14-7"));
    let external = ExternalModule {
        sha256: file_sha256(&path).await.unwrap(),
        path,
        args: vec![],
        expected: Some(ModuleId::new("isles-1-14-7").unwrap()),
    };
    let host = dainty_module_api::stub::recording_host();
    let driver = ChildModule::spawn(
        &external,
        NodeIdentity::standalone(),
        host.clone(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    testing::drive(&driver, &host, Some(&env), &mounts(true, cfg!(feature = "realms")))
        .await
        .unwrap();
}

fn context(host: &Arc<RecordingHost>) -> ModuleContext {
    ModuleContext::new(
        ModuleId::new("isles-1-14-7").unwrap(),
        NodeIdentity::standalone(),
        host.clone(),
    )
}

/// Every port 0, the env file and the backend game data folder.
fn config(module: &IslesModule, env: &Path, backend_dir: Option<&Path>) -> ModuleConfig {
    let mut registrar = dainty_module_api::Registrar::new(module.manifest());
    module.register(&mut registrar).unwrap();
    let registration = registrar.finish().unwrap();
    let mut config = ModuleConfig::defaults(&module.manifest(), &registration);
    for range in config.ports.values_mut() {
        *range = PortRange::single(0);
    }
    let set = |config: &mut ModuleConfig, section: &str, key: &str, value: String| {
        config
            .sections
            .entry(section.to_owned())
            .or_default()
            .insert(key.to_owned(), value.into());
    };
    set(&mut config, "backend", "env_file", env.display().to_string());
    if let Some(dir) = backend_dir {
        set(&mut config, "game_data", "backend_dir", dir.display().to_string());
    }
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_before_during_and_after() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_file(dir.path(), &["RUSTIC_RIFTS_P2P_MODULE=off"]);
    let host = dainty_module_api::stub::recording_host();
    let module = rustic_isles_1_14_7::module().unwrap();
    let ctx = context(&host);
    let account = GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node);
    let query = || ChatQuery::Jid {
        account: account.clone(),
    };
    let before = module.chat(ctx.clone(), query()).await.unwrap_err();
    assert_eq!(before.kind, ErrorKind::Unavailable);
    module.start(ctx.clone(), config(&module, &env, None)).await.unwrap();
    let running = module.chat(ctx.clone(), query()).await.unwrap_err();
    assert_eq!(running.kind, ErrorKind::Unsupported, "{running:?}");
    // A presence event for an unknown player is harmless without a database
    let event = dainty_module_api::Event {
        name: dainty_module_api::EventName::new(PRESENCE_EVENT).unwrap(),
        payload: serde_json::json!({ "user_id": "nobody", "online": false }),
    };
    module.event(ctx.clone(), event).await.unwrap();
    module.stop(ctx.clone()).await.unwrap();
    let after = module.chat(ctx, query()).await.unwrap_err();
    assert_eq!(after.kind, ErrorKind::Unavailable);
}

/// A SQLite store and the auth secrets a started module with a database needs (made per test, never committed).
fn database_lines(db: &Path, extra: &[&str]) -> Vec<String> {
    let (private, public) = rustic_isles_auth::test_support::throwaway_signing_keys();
    let mut lines = vec![
        format!("DAINTY_DB_FILE='{}'", db.display()),
        "API_KEY_HASH_SECRET=test-secret-of-at-least-thirty-two-chars".to_owned(),
        format!("AUTH_SIGNING_PRIVKEY_B64={private}"),
        format!("AUTH_SIGNING_PUBKEY_B64={public}"),
    ];
    lines.extend(extra.iter().map(|line| (*line).to_owned()));
    lines
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn version_data_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db").join("isles.sqlite");
    let env = env_file(
        dir.path(),
        &database_lines(&db, &["RUSTIC_RIFTS_P2P_MODULE=off"])
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let host = dainty_module_api::stub::recording_host();

    // A database wanted and no file
    let module = rustic_isles_1_14_7::module().unwrap();
    let error = module
        .start(context(&host), config(&module, &env, Some(&data)))
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .starts_with("[game-data] Required game data file is missing: ")
            && error.message.contains("version_data.json"),
        "{}",
        error.message
    );
    assert_eq!(module.status().phase, Phase::Failed);

    // A broken file
    let mut broken = synthetic_document();
    broken["format"] = 2.into();
    std::fs::write(data.join("version_data.json"), broken.to_string()).unwrap();
    let module = rustic_isles_1_14_7::module().unwrap();
    let error = module
        .start(context(&host), config(&module, &env, Some(&data)))
        .await
        .unwrap_err();
    assert_eq!(error.message, "version_data.json: format must be 1");

    // A good file: the module starts on the store and stops with a checkpoint
    std::fs::write(data.join("version_data.json"), synthetic_document().to_string()).unwrap();
    let module = rustic_isles_1_14_7::module().unwrap();
    module
        .start(context(&host), config(&module, &env, Some(&data)))
        .await
        .unwrap();
    assert_eq!(module.status().phase, Phase::Ready);
    module.stop(context(&host)).await.unwrap();

    // No database: starts without the file
    let bare_env = env_file(dir.path(), &["RUSTIC_RIFTS_P2P_MODULE=off"]);
    let module = IslesModule::new(spec().unwrap()).unwrap();
    module
        .start(context(&host), config(&module, &bare_env, None))
        .await
        .unwrap();
    module.stop(context(&host)).await.unwrap();

    // Given data needs no file either
    let given = rustic_isles_1_14_7::test_support::synthetic();
    let module = IslesModule::new(spec_with(given).unwrap()).unwrap();
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    module
        .start(context(&host), config(&module, &env, Some(&empty)))
        .await
        .unwrap();
    module.stop(context(&host)).await.unwrap();
}

#[tokio::test]
async fn accounts_core_without_a_store() {
    let cell = Arc::new(rustic_isles_1_14_7::SharedCell::new());
    let core = AccountsCore::new(None, Settings::default(), Arc::clone(&cell));
    assert_eq!(core.display_name(AccountId("u".into())).await.unwrap(), None);
    assert_eq!(core.find_by_display_name("x".into()).await.unwrap(), None);
    assert!(core.launcher_account_eligible(AccountId("u".into())).await.unwrap());
    assert!(
        !core
            .verify_operations_step_up("k".into(), "123456".into())
            .await
            .unwrap()
    );
    assert!(core.operations_policy().await.unwrap().0.starts_with('{'));
    for (value, mode) in [
        (None, GuardMode::Observe),
        (Some(""), GuardMode::Observe),
        (Some("Observe"), GuardMode::Observe),
        (Some("off"), GuardMode::Enforce),
    ] {
        let settings = match value {
            Some(v) => Settings::from_pairs([("RUSTIC_ISLES_GUARD_MODE", v)]),
            None => Settings::default(),
        };
        assert_eq!(AccountsCore::new(None, settings, Arc::clone(&cell)).guard_mode(), mode);
    }
}

#[tokio::test]
async fn realms_adapter_dedicated_only() {
    use rustic_isles_auth::{Realms, RuntimeAuth, RuntimeRequest};
    let adapter = RealmsAdapter::new(rustic_rifts_seam::Rifts::dedicated_only());
    let headers = http::HeaderMap::new();
    let request = RuntimeRequest {
        path: "/x",
        headers: &headers,
        peer: None,
        request_id: None,
    };
    assert!(matches!(
        adapter.authenticate_runtime(request).await.unwrap(),
        RuntimeAuth::NotMine
    ));
    let account = rustic_isles_auth::LauncherAccount {
        account_id: "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@local".into(),
        user_id: "u1".into(),
        email: None,
        display_name: "U1".into(),
        display_name_normalized: "u1".into(),
        password_hash: None,
        status: rustic_isles_auth::Status::Active,
        approval: None,
        roles: vec![],
        roles_updated_at: None,
        created_at: "2026-01-01T00:00:00.000Z".into(),
        last_login_at: None,
        username_set: None,
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    };
    assert!(adapter.launcher_policy_fields(&account).await.unwrap().is_empty());
    assert_eq!(adapter.authorize_game_session(&account, None).await.unwrap(), None);
    adapter.bind_realm_presence(&account, None).await.unwrap();
}

/// The full composition on a SQLite store with rifts on (when built with realms).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_composition_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("isles.sqlite");
    let lines = database_lines(&db, &["RUSTIC_ISLES_ADMIN_API=on"]);
    let env = env_file(dir.path(), &lines.iter().map(String::as_str).collect::<Vec<_>>());
    let host = dainty_module_api::stub::recording_host();
    let given = rustic_isles_1_14_7::test_support::synthetic();
    let module = IslesModule::new(spec_with(given).unwrap()).unwrap();
    module.start(context(&host), config(&module, &env, None)).await.unwrap();
    assert_eq!(module.status().phase, Phase::Ready);
    let listening = testing::listening(&host);
    let backend = listening
        .iter()
        .find(|(n, _)| n == "backend")
        .unwrap()
        .1
        .parse()
        .unwrap();
    let live = testing::request(backend, "GET", "/health/live").await.unwrap();
    assert_eq!(live.status, 200, "{}", live.body);
    let anonymous = testing::request(backend, "GET", "/launcher/v1/realms/mine")
        .await
        .unwrap();
    let expected = if cfg!(feature = "realms") { 401 } else { 404 };
    assert_eq!(anonymous.status, expected, "{}", anonymous.body);
    // A game route behind game auth is the area's, not the 501 envelope
    let survey = testing::request(backend, "GET", "/survey/config").await.unwrap();
    assert_eq!((survey.status, survey.body.as_str()), (401, ""));
    let status = testing::request(backend, "GET", "/dauntless-status").await.unwrap();
    assert_eq!(status.status, 200, "{}", status.body);
    module.stop(context(&host)).await.unwrap();
}
