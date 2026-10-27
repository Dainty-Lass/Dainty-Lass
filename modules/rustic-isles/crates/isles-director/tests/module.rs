#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

//! RU6's verify at the module level, on both versions: the module loads into DaintyLass's framework as it does
//! compiled in, its Director boots Ramsgate and the Training Grounds on the controller's game port range from the
//! env file's fake game server, starts a hunt, and unloading the module ends every game server.

mod common;

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use common::{FAKE, bounded, free_range, send, udp_free, within, write_tables};
use futures_util::FutureExt;
use isles_core::module::IslesModule;
use module_api::stub::recording_host;
use module_api::{InProcessModule, ModuleConfig, ModuleDriver, ModuleDriverExt, NodeIdentity, PortRange};
use serde_json::{Value, json};

async fn module_starts_and_stops_game_servers(module: IslesModule, trials_new: bool) {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("game-data");
    std::fs::create_dir_all(&data).unwrap();
    write_tables(&data, trials_new);
    let logs = root.path().join("logs");
    let (begin, end) = free_range(5);
    let mut env = tempfile::NamedTempFile::new().unwrap();
    let quoted = |path: &std::path::Path| format!("'{}'", path.display());
    write!(
        env,
        "MY_IP=100.101.102.103\nGAMESERVER_BINARY_PATH='{FAKE}'\nMETAGAME_API_KEY=gameserver-key\n\
         SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP=0\nGAMESERVER_READY_TIMEOUT_MS=15000\n\
         GAMESERVER_LOG_DIR={}\n",
        quoted(&logs)
    )
    .unwrap();
    env.flush().unwrap();

    let host = recording_host();
    let driver = InProcessModule::new(Arc::new(module), NodeIdentity::standalone(), host.clone()).unwrap();
    let registration = within("register", driver.register()).await.unwrap();
    let mut config = ModuleConfig::defaults(driver.manifest(), &registration);
    for port in ["backend", "director"] {
        config.ports.insert(port.to_owned(), PortRange::single(0));
    }
    config.ports.insert(
        "game".to_owned(),
        PortRange {
            first: begin,
            last: end,
        },
    );
    config
        .sections
        .get_mut("backend")
        .unwrap()
        .insert("env_file".into(), env.path().display().to_string().into());
    config
        .sections
        .get_mut("game_data")
        .unwrap()
        .insert("director_dir".into(), data.display().to_string().into());
    // However the checks end, the module is unloaded after them, which ends every game server
    within("start", driver.start(config)).await.unwrap();
    let checks = async {
        let director = isles_core::testing::listening(&host)["director"];

        // The hubs boot in the background on the controller's range: Ramsgate last, the Training Grounds before it
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let status = loop {
            let status: Value =
                serde_json::from_str(&send(director, "GET", "/api/hub-status", &[], "").await.text).unwrap();
            if !status["ramsgate"].is_null() && !status["trainingDojo"].is_null() {
                break status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the hubs never came up: {status}; logs {:?}; calls {:?}",
                std::fs::read_dir(&logs).map(|d| d.count()),
                host.calls()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        assert_eq!(status["ramsgate"]["port"], end);
        assert_eq!(status["trainingDojo"]["port"], end - 1);
        assert_eq!(send(director, "GET", "/health/ready", &[], "").await.status, 200);
        assert!(!udp_free(end) && !udp_free(end - 1));

        let hunt = send(
            director,
            "POST",
            "/api/matchmaker/handle-matchmaking-for-player",
            &[("Content-Type", "application/json")],
            &json!({"GameMode": "SHARED", "HuntId": "Boss_Hunt", "ExpectedPlayers": ["acc-1"]}).to_string(),
        )
        .await;
        assert_eq!(hunt.status, 200, "{}", hunt.text);
        assert_eq!(hunt.json()["port"], begin);
        let instances = send(director, "GET", "/api/instances", &[], "").await.json();
        assert_eq!(instances.as_array().unwrap().len(), 3);
    };
    let outcome = std::panic::AssertUnwindSafe(bounded(checks)).catch_unwind().await;
    let unloaded = tokio::time::timeout(Duration::from_secs(60), ModuleDriver::unload(&driver)).await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    unloaded.expect("unload took over 60 s").unwrap();
    // Unloading ended every game server and freed every port
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !(begin..=end).all(udp_free) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a game server outlived the module"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_1_14_7_module_starts_and_stops_game_servers() {
    module_starts_and_stops_game_servers(isles_1_14_7::module(), true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_1_12_0_module_starts_and_stops_game_servers() {
    module_starts_and_stops_game_servers(isles_1_12_0::module(), false).await;
}
