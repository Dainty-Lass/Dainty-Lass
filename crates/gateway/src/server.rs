// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Starting and stopping the gateway. The `run_*` entry points differ only in what they attach: the in-process
//! allowlist sink, the module routes (else the stand-in), the node-link slot and the region handler.

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::json;
use service_kit::context::ServiceContext;
use service_kit::env::Env;
use service_kit::linelog;

use crate::capture::{Capture, CaptureWriter, load_capture_options};
use crate::cert::{self, iso};
use crate::config::Config;
use crate::feed::{AllowlistFeed, InProcessSink};
use crate::gateway::{Gateway, Tls};
use crate::modules::ModuleRoutes;
use crate::network::RegionReportHandler;
use crate::nodelink::NodeLinks;

const THIRTY_DAYS_MS: i64 = 30 * 86_400_000;

/// Where the started gateway leaves its node links, for the core to bind its node services to.
pub type NodeLinkSlot = Arc<Mutex<Option<Arc<NodeLinks>>>>;

/// The public certificate (public mode): loaded, valid now, and never made here.
pub fn load_tls(config: &Config) -> Result<Option<Tls>, String> {
    if !config.tls {
        return Ok(None);
    }
    let loaded = cert::load(&config.cert_file, &config.key_file)?;
    let info = loaded.info.clone();
    let now = crate::now_ms();
    if info.not_after <= now || info.not_before > now {
        return Err(format!(
            "The certificate is not valid now (valid {} to {}). Make a new one with `gateway make-cert`; invites then need the new fingerprint.",
            iso(info.not_before),
            iso(info.not_after)
        ));
    }
    if info.not_after - now <= THIRTY_DAYS_MS {
        linelog::warn(
            "the certificate expires within 30 days; make a new one and send new invites",
            json!({ "notAfter": iso(info.not_after) }),
        );
    }
    let config = cert::server_config(&loaded).map_err(|e| {
        format!(
            "Certificate {} / key {}: {e}",
            config.cert_file.display(),
            config.key_file.display()
        )
    })?;
    Ok(Some(Tls { config, info }))
}

/// The traffic capture, when `CAPTURE_DIR` is set; errors read `Traffic capture: ...`.
pub fn load_capture(env: &Env) -> Result<Option<Capture>, String> {
    let Some(options) = load_capture_options(env).map_err(|e| format!("Traffic capture: {e}"))? else {
        return Ok(None);
    };
    let (dir, retention) = (options.dir.display().to_string(), options.retention_days);
    let writer = CaptureWriter::new(options)?;
    for name in ["GATEWAY_SECRET", "ALLOWLIST_SECRET"] {
        if let Some(secret) = env.get(name) {
            writer.add_secret(secret);
        }
    }
    linelog::warn(
        "traffic capture is on: every request and answer is recorded, redacted, for the shadow replay (personal data; see docs/security/gateway.md)",
        json!({ "dir": dir, "retentionDays": retention }),
    );
    Ok(Some(Capture::new(writer)))
}

pub async fn run(config: Config, capture: Option<Capture>) -> Result<(), String> {
    run_with_context(config, capture, ServiceContext::standalone("gateway")).await
}

pub async fn run_with_context(config: Config, capture: Option<Capture>, ctx: ServiceContext) -> Result<(), String> {
    run_with_network(config, capture, None, ctx, None, None, None).await
}

pub async fn run_with_routes(
    config: Config,
    capture: Option<Capture>,
    ctx: ServiceContext,
    routes: Arc<ModuleRoutes>,
) -> Result<(), String> {
    run_with_network(config, capture, None, ctx, Some(routes), None, None).await
}

pub async fn run_with_node_links(
    config: Config,
    capture: Option<Capture>,
    ctx: ServiceContext,
    routes: Option<Arc<ModuleRoutes>>,
    node_link_slot: NodeLinkSlot,
) -> Result<(), String> {
    run_with_network(config, capture, None, ctx, routes, Some(node_link_slot), None).await
}

fn rate(spec: crate::ratelimit::BucketSpec) -> serde_json::Value {
    json!({ "burst": spec.burst, "perMinute": spec.per_minute })
}

/// Section 16 of the front page: start, serve until the context stops, shut down.
#[allow(clippy::too_many_arguments)]
pub async fn run_with_network(
    config: Config,
    capture: Option<Capture>,
    in_process_sink: Option<InProcessSink>,
    mut ctx: ServiceContext,
    routes: Option<Arc<ModuleRoutes>>,
    node_link_slot: Option<NodeLinkSlot>,
    region_handler: Option<RegionReportHandler>,
) -> Result<(), String> {
    let status = ctx.status().clone();
    status.starting();
    let fail = |error: String| {
        status.failed(error.clone());
        error
    };
    let tls = load_tls(&config).map_err(fail)?;
    let feed = AllowlistFeed::new(config.allowlist.as_ref());
    let feed = match in_process_sink {
        Some(sink) => feed.with_in_process(sink),
        None => feed,
    };
    if !feed.is_on() {
        linelog::warn(
            "allowlist feed is off (GATEWAY_ALLOWLIST=0): the game ports open for nobody unless the firewall is managed another way",
            json!({}),
        );
    }
    let routes = match routes {
        Some(routes) => routes,
        None => Arc::new(ModuleRoutes::standalone(&config).map_err(fail)?),
    };
    let gateway = Gateway::with_routes(config.clone(), tls, Some(feed), capture, routes).map_err(fail)?;
    if let Some(handler) = region_handler {
        gateway.set_region_handler(handler);
    }
    let listen = config.listen_address();
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .map_err(|e| fail(format!("Could not listen on {listen}: {e}")))?;
    if let (Some(link_config), Some(links)) = (&config.node_links, gateway.node_links()) {
        let info = links.certificate();
        let fields = json!({
            "node": link_config.identity.node.to_string(),
            "region": link_config.identity.region.to_string(),
            "fingerprint": info.fingerprint,
            "certificateValidUntil": iso(info.not_after * 1000),
        });
        linelog::info("node links on", fields.clone());
        if info.not_after * 1000 - crate::now_ms() <= THIRTY_DAYS_MS {
            linelog::warn(
                "the node certificate expires within 30 days; issue a new one (`network issue-node-cert`)",
                fields,
            );
        }
        if let Some(slot) = &node_link_slot {
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(links);
        }
    }
    status.ready(
        Some(listen.clone()),
        Some(if config.tls { "TLS" } else { "HTTP" }.to_owned()),
    );
    let mut ready = serde_json::Map::new();
    ready.insert("listen".into(), json!(listen));
    ready.insert(
        "mode".into(),
        json!(if config.tls {
            "public (TLS)"
        } else {
            "private (plain HTTP inside the tailnet)"
        }),
    );
    if let Some(info) = gateway.certificate() {
        ready.insert("fingerprint".into(), json!(info.fingerprint));
        ready.insert("certificateNames".into(), json!(info.subject_alt_name));
        ready.insert("certificateValidUntil".into(), json!(iso(info.not_after)));
    }
    ready.insert("backend".into(), json!(config.backend.authority()));
    ready.insert("content".into(), json!(config.content.authority()));
    ready.insert("websocket".into(), json!(config.ws.authority()));
    ready.insert(
        "allowlistFeed".into(),
        json!(config.allowlist.as_ref().map_or("off", |a| a.url.as_str())),
    );
    ready.insert("maxBodyBytes".into(), json!(config.limits.max_body_bytes));
    ready.insert("maxConnections".into(), json!(config.limits.max_connections));
    ready.insert(
        "maxConnectionsPerIp".into(),
        json!(config.limits.max_connections_per_ip),
    );
    let r = config.limits.rates;
    ready.insert(
        "rate".into(),
        json!({ "general": rate(r.general), "content": rate(r.content), "register": rate(r.register),
            "token": rate(r.token), "ws": rate(r.ws), "connect": rate(r.connect) }),
    );
    linelog::info("gateway ready", serde_json::Value::Object(ready));
    let grace_ms = config.timeouts.shutdown_grace.as_millis() as u64;
    let stopping = status.clone();
    gateway
        .serve_until(listener, async move {
            ctx.wait_stop().await;
            stopping.stopping();
            linelog::info("stopping", json!({ "signal": "stop", "graceMs": grace_ms }));
        })
        .await;
    if let Some(slot) = &node_link_slot {
        slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
    status.stopped();
    linelog::info("stopped", json!({}));
    Ok(())
}
