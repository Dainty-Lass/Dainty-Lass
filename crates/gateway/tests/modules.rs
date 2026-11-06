// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

//! The gateway on the module registry (DF4a), over TLS: a module the controller attaches gets its routes,
//! never its host-only ones, and the node-link group refuses everything until DN3.

mod common;

use std::sync::Arc;

use common::*;
use gateway::cert;
use gateway::config::Endpoint;
use gateway::gateway::{Gateway, Tls};
use gateway::modules::{ModuleRoutes, RegistryView};
use module_api::stub::StubModule;
use module_api::{CorePlugin, GameModule, ModuleManifest, Registrar};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// The controller's view in this test: the module's `http` port is the fake upstream.
struct View(u16);

impl RegistryView for View {
    fn endpoint(&self, _module: &ModuleManifest, port: &str) -> Option<Endpoint> {
        (port == "http").then(|| Endpoint {
            host: "127.0.0.1".into(),
            port: self.0,
        })
    }
}

/// The stub, as a module for 1.14.7: every path to its `http` port but its admin routes.
fn module() -> (ModuleManifest, module_api::Registration) {
    let mut manifest = StubModule::new().manifest();
    manifest.game_version = "1.14.7".parse().unwrap();
    manifest.invite_aliases = vec!["reforged".into()];
    let mut registrar = Registrar::new(manifest.clone());
    registrar.route("/", "http").host_only_route("/stub/admin", "http");
    (manifest, registrar.finish().unwrap())
}

#[tokio::test]
async fn an_attached_module_gets_its_routes_and_never_its_host_only_ones() {
    let cert = make_cert();
    let backend = start_upstream("backend", None).await;
    let module_upstream = start_upstream("module", None).await;
    let content = start_upstream("content", None).await;
    let ws = start_ws_upstream().await;
    let config = test_config(
        &cert,
        &Ports {
            backend: backend.port,
            content: content.port,
            ws: ws.port,
        },
        &[],
    );
    let routes = Arc::new(ModuleRoutes::new(Arc::new(View(module_upstream.port))));
    let (manifest, registration) = module();
    routes.attach(&manifest, &registration).unwrap();

    let loaded = cert::load(&cert.cert_file, &cert.key_file).unwrap();
    let tls = Tls {
        config: cert::server_config(&loaded).unwrap(),
        info: loaded.info,
    };
    let gateway = Gateway::with_routes(config, Some(tls), None, None, routes.clone()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopping) = watch::channel(false);
    let task = tokio::spawn(gateway.serve(listener, stopping));

    let get = |target: &'static str| https_request(port, target, Options::default());
    let party = get("/__origin/gamedev-prod.steelyard.ca/party/invites").await.unwrap();
    assert_eq!(party.status, 200, "{}", party.text());
    let seen = module_upstream.last();
    assert_eq!(seen.url, "/__origin/gamedev-prod.steelyard.ca/party/invites");
    assert_eq!(
        seen.headers.get("x-dainty-gateway").map(String::as_str),
        Some(GATEWAY_SECRET)
    );

    for target in [
        "/stub/admin",
        "/Stub/Admin/kick/",
        "/__origin/x.example/stub/admin",
        "/__node/hello",
    ] {
        let refused = get(target).await.unwrap();
        assert_eq!(
            (refused.status, refused.json()),
            (403, json!({ "error": "forbidden" })),
            "{target}"
        );
    }
    assert_eq!(module_upstream.count(), 1);

    // Content stays the content server's, and the policy's refusals come first
    assert_eq!(get("/content/v1/manifest").await.unwrap().status, 200);
    assert_eq!(content.count(), 1);
    assert_eq!(get("/reforged/api/CreateInvite").await.unwrap().status, 403);
    assert_eq!(backend.count(), 0);

    routes.detach(&manifest.id);
    let gone = get("/party/invites").await.unwrap();
    assert_eq!((gone.status, gone.json()), (503, json!({ "error": "unavailable" })));
    assert_eq!(module_upstream.count(), 1);

    let _ = stop.send(true);
    let _ = task.await;
}
