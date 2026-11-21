// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The Director as the module's `director` service and dedicated-servers slot (spec §12.3, §14.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{DIRECTOR_1_14_7, port_band, send, udp_free, wait_for};
use dainty_module_api::{
    Capabilities, InProcessModule, ModuleConfig, ModuleDriver, ModuleDriverExt, ModuleId, NodeIdentity,
};
use rustic_isles::testing::{listening, sample_version};
use rustic_isles::{BackendService, BareComposer, DefaultPorts, IslesModule, ModuleSpec, PortService};
use rustic_isles_director::DirectorService;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_slot_is_in_process_without_an_http_call() {
    let dir = tempfile::tempdir().unwrap();
    let range = port_band();
    let settings = common::settings(dir.path(), range, &[("DEPLOYSERVER_URL", "127.0.0.1:1")]);
    let shared = common::shared(settings, None);
    let service = DirectorService::new(DIRECTOR_1_14_7);
    let _router = service.app(Arc::clone(&shared)).unwrap();
    let slot = shared.dedicated_servers();
    assert!(slot.configured());
    let status = slot.hub_status().await.unwrap();
    assert_eq!((status.ramsgate, status.training), (None, None));
    assert!(service.director().is_some());
    service.stopping().await;
    service.stopping().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_module_runs_the_director() {
    tokio::time::timeout(Duration::from_secs(180), async {
        let dir = tempfile::tempdir().unwrap();
        let range = port_band();
        let settings = common::settings(dir.path(), range, &[]);
        let env: String = settings
            .env_map()
            .iter()
            .filter(|(k, _)| !k.starts_with("PORT_RANGE"))
            .map(|(k, v)| format!("{k}='{v}'\n"))
            .collect();
        let env_file = dir.path().join("director.env");
        std::fs::write(&env_file, env).unwrap();
        let tables = dir.path().join("tables");
        common::write_tables(&tables);

        let spec = ModuleSpec {
            id: ModuleId::new("isles-1-14-7").unwrap(),
            display_name: "Rustic Isles 1.14.7".to_owned(),
            invite_aliases: vec!["reforged".to_owned()],
            capabilities: Capabilities::default(),
            unverified: false,
            ports: DefaultPorts {
                backend: 61000,
                director: 61001,
                qos: 61010,
                game: range,
            },
            version: sample_version(),
            services: vec![
                Arc::new(BackendService),
                Arc::new(DirectorService::new(DIRECTOR_1_14_7)),
            ],
            composer: Arc::new(BareComposer),
        };
        let module = Arc::new(IslesModule::new(spec).unwrap());
        let host = dainty_module_api::stub::recording_host();
        let driver = InProcessModule::new(module, NodeIdentity::standalone(), host.clone()).unwrap();
        let registration = driver.register().await.unwrap();
        let mut config = ModuleConfig::defaults(driver.manifest(), &registration);
        for (name, ports) in config.ports.iter_mut() {
            *ports = if name == "game" {
                range
            } else {
                dainty_module_api::PortRange::single(0)
            };
        }
        let sections = &mut config.sections;
        sections
            .entry("backend".to_owned())
            .or_default()
            .insert("env_file".to_owned(), json!(env_file.display().to_string()));
        sections
            .entry("game_data".to_owned())
            .or_default()
            .insert("director_dir".to_owned(), json!(tables.display().to_string()));
        driver.start(config).await.unwrap();
        let address: std::net::SocketAddr = listening(&host)
            .into_iter()
            .find(|(n, _)| n == "director")
            .unwrap()
            .1
            .parse()
            .unwrap();
        let mut ready = false;
        for _ in 0..600 {
            if send(address, "GET", "/health/ready", &[], None).await.status == 200 {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ready);
        let hubs = send(address, "GET", "/api/hub-status", &[], None).await.json();
        assert_eq!(
            (hubs["ramsgate"]["port"].clone(), hubs["trainingDojo"]["port"].clone()),
            (json!(range.last), json!(range.last - 1))
        );
        let body = json!({"GameMode": "SHARED", "HuntId": "Plain_Hunt", "ExpectedPlayers": ["acc-1"]}).to_string();
        let hunt = send(
            address,
            "POST",
            "/api/matchmaker/handle-matchmaking-for-player",
            &[("content-type", "application/json")],
            Some(body.as_bytes()),
        )
        .await;
        assert_eq!(hunt.json()["port"], range.first);
        let instances = send(address, "GET", "/api/instances", &[], None).await.json();
        assert_eq!(instances.as_array().unwrap().len(), 3);
        driver.unload().await.unwrap();
        for port in range.first..=range.last {
            assert!(wait_for(15, || udp_free(port)).await, "port {port} still held");
        }
    })
    .await
    .unwrap();
}
