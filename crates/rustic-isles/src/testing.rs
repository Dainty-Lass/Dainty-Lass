// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Test support (core spec §13.3): sample paths for every row, a raw HTTP/1.1 client, the addresses a started
//! module announced, and the sweep and drive a version crate runs over its composition.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use module_api::stub::RecordingHost;
use module_api::{HostCall, ModuleConfig, ModuleDriver, ModuleDriverExt, Phase, PortRange};
use rustic_isles_auth::CallerKind;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::module::LISTENING_EVENT;
use crate::pattern::Pattern;
use crate::pipeline::{OriginStrip, REQUEST_ID_HEADER, is_admin_path, strip_origin};
use crate::routes::{self, Method, RouteDef, RouteSet, Service, When};
use crate::version::{
    Destination, Entitlement, GameMode, IslesVersion, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule,
    StarterManifest, StubReply, VersionHooks, VersionTables,
};

/// A Gauntlet origin, which the `gauntlet` rows answer for.
pub const GAUNTLET_ORIGIN: &str = "/__origin/gauntlet-prod.steelyard.ca";

/// A path the row matches: each `:param` and each regex group `x`; a regex ending in `\/` gets a trailing `x`;
/// a `gauntlet` row behind the Gauntlet origin.
pub fn sample_path(route: &RouteDef) -> String {
    let path = if route.is_regex() {
        let body = route.path.get(2..route.path.rfind('/').unwrap_or(2)).unwrap_or("");
        let body = body.strip_suffix('$').unwrap_or(body);
        let mut out = String::new();
        let mut depth = 0usize;
        for c in body.chars() {
            match c {
                '(' => {
                    if depth == 0 {
                        out.push('x');
                    }
                    depth += 1;
                }
                ')' => depth = depth.saturating_sub(1),
                _ if depth > 0 => {}
                _ => out.push(c),
            }
        }
        let mut out = out.replace("\\/", "/");
        if body.ends_with("\\/") {
            out.push('x');
        }
        out
    } else {
        route
            .path
            .split('/')
            .map(|segment| if segment.starts_with(':') { "x" } else { segment })
            .collect::<Vec<_>>()
            .join("/")
    };
    if route.when == When::Gauntlet {
        format!("{GAUNTLET_ORIGIN}{path}")
    } else {
        path
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    /// Names lower-cased
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(line_end) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&rest[..line_end])
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok())
            .unwrap_or(0);
        rest = &rest[line_end + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or(&[]);
    }
    out
}

/// One HTTP/1.1 request on a new connection (`Connection: close`, `Content-Length: 0`).
pub async fn request(address: SocketAddr, method: &str, path: &str) -> std::io::Result<Reply> {
    let mut stream = tokio::net::TcpStream::connect(address).await?;
    let head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| std::io::Error::other("no end of headers"))?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| std::io::Error::other("no status line"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.contains("chunked"))
    {
        body = dechunk(&body);
    }
    Ok(Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// The addresses a started module announced in `isles.listening`, in order.
pub fn listening(host: &RecordingHost) -> Vec<(String, String)> {
    let payload = host.calls().into_iter().rev().find_map(|(_, call)| match call {
        HostCall::Emit(event) if event.name.as_str() == LISTENING_EVENT => Some(event.payload),
        _ => None,
    });
    let object = match payload {
        Some(Value::String(text)) => serde_json::from_str(&text).unwrap_or(Value::Null),
        Some(other) => other,
        None => Value::Null,
    };
    object
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// What a composition mounts and handles.
#[derive(Debug, Clone, Default)]
pub struct Mounts {
    /// Rows an area answers: the sweep leaves them (and paths they take) alone
    pub handled: Vec<(Service, Method, &'static str)>,
    pub admin_api: bool,
    pub rifts_loaded: bool,
}

impl Mounts {
    fn mounted(&self, row: &RouteDef) -> bool {
        match row.when {
            When::Always | When::Gauntlet => true,
            When::Admin => self.admin_api,
            When::Rifts => self.rifts_loaded,
        }
    }

    fn handles(&self, row: &RouteDef) -> bool {
        self.handled
            .iter()
            .any(|(s, m, p)| *s == row.service && *m == row.method && *p == row.path)
            || row.group == routes::Group::Auth
    }
}

enum Expect {
    Skip,
    Empty404,
    NotImplemented(&'static RouteDef),
    Unauthorized,
}

fn expect(service: Service, row: &RouteDef, mounts: &Mounts) -> Expect {
    let sample = sample_path(row);
    let (path, origin) = match strip_origin(&sample) {
        OriginStrip::Stripped { host, target } => (target, Some(host)),
        _ => (sample, None),
    };
    let rows = RouteSet::V1147.routes(service);
    let admin_mark = routes::all()
        .iter()
        .position(|r| r.path.starts_with("/admin/"))
        .unwrap_or(usize::MAX);
    let mut at_admin = service != Service::Backend;
    for candidate in rows.iter().filter(|r| mounts.mounted(r)) {
        if !at_admin && candidate.index() >= admin_mark {
            at_admin = true;
            if !mounts.admin_api && is_admin_path(&path) {
                return Expect::Empty404;
            }
        }
        if !candidate.method.accepts(&row.method.http()) {
            continue;
        }
        let Ok(pattern) = Pattern::parse(candidate.path) else {
            continue;
        };
        if !matches!(pattern.matches(&path), Some(Ok(_))) {
            continue;
        }
        if candidate.when == When::Gauntlet && !origin.as_deref().is_some_and(|h| h.contains("gauntlet")) {
            continue;
        }
        if mounts.handles(candidate) {
            return Expect::Skip;
        }
        if candidate.when == When::Rifts {
            let kind = rustic_isles_auth::rifts_caller_kind(candidate.method.as_str(), candidate.path);
            return match kind {
                CallerKind::Launcher { .. } => Expect::Unauthorized,
                _ => Expect::Skip,
            };
        }
        return Expect::NotImplemented(candidate);
    }
    Expect::Empty404
}

/// Every row of `service` answers as a composition with `mounts` should; the count of rows checked.
pub async fn sweep(service: Service, address: SocketAddr, mounts: &Mounts) -> Result<usize, String> {
    let mut checked = 0;
    for row in RouteSet::V1147.routes(service) {
        let expected = expect(service, row, mounts);
        if matches!(expected, Expect::Skip) {
            continue;
        }
        let path = sample_path(row);
        let reply = request(address, row.method.as_str(), &path)
            .await
            .map_err(|e| format!("{}: {e}", row.name()))?;
        let fail = |what: &str| format!("{} ({path}): {what}, got {} {}", row.name(), reply.status, reply.body);
        let id = reply
            .header(REQUEST_ID_HEADER)
            .ok_or_else(|| fail("no request id header"))?
            .to_owned();
        match expected {
            Expect::Empty404 if reply.status == 404 && reply.body.is_empty() => {}
            Expect::Empty404 => return Err(fail("expected an empty 404")),
            Expect::NotImplemented(taker) => {
                let message = format!("isles: {} {} is not implemented yet", taker.method.as_str(), taker.path);
                let body =
                    serde_json::json!({"error": {"code": "NOT_IMPLEMENTED", "message": message, "requestId": id}});
                if reply.status != 501 || reply.body != service_kit::jsjson::stringify(&body) {
                    return Err(fail("expected the 501 envelope"));
                }
            }
            Expect::Unauthorized => {
                let code = serde_json::from_str::<Value>(&reply.body)
                    .ok()
                    .and_then(|v| v.pointer("/error/code").and_then(Value::as_str).map(str::to_owned));
                if reply.status != 401 || code.as_deref() != Some("AUTH_UNAUTHORIZED") {
                    return Err(fail("expected 401 AUTH_UNAUTHORIZED"));
                }
            }
            Expect::Skip => {}
        }
        checked += 1;
    }
    Ok(checked)
}

fn address(listening: &[(String, String)], name: &str) -> Option<SocketAddr> {
    listening.iter().find(|(n, _)| n == name)?.1.parse().ok()
}

/// Register, every port 0 and the env file, start, sweep the backend, check the QoS pong is loopback and the
/// Director's probes when they started, check `Ready`, unload.
pub async fn drive(
    driver: &dyn ModuleDriver,
    host: &RecordingHost,
    env_file: Option<&Path>,
    mounts: &Mounts,
) -> Result<(), String> {
    let registration = driver.register().await.map_err(|e| e.to_string())?;
    let mut config = ModuleConfig::defaults(driver.manifest(), &registration);
    for range in config.ports.values_mut() {
        *range = PortRange::single(0);
    }
    if let Some(file) = env_file {
        config
            .sections
            .entry("backend".to_owned())
            .or_default()
            .insert("env_file".to_owned(), Value::from(file.display().to_string()));
    }
    driver.start(config).await.map_err(|e| e.to_string())?;
    let announced = listening(host);
    let backend = address(&announced, "backend").ok_or("no backend address")?;
    if sweep(Service::Backend, backend, mounts).await? == 0 {
        return Err("the sweep checked no row".to_owned());
    }
    if let Some(qos) = address(&announced, "qos")
        && !qos.ip().is_loopback()
    {
        return Err(format!("the QoS pong listens on {qos}"));
    }
    if let Some(director) = address(&announced, "director") {
        let ready = request(director, "GET", "/health/ready")
            .await
            .map_err(|e| e.to_string())?;
        if ready.status != 503 {
            return Err(format!("the Director's /health/ready answered {}", ready.status));
        }
        let instances = request(director, "GET", "/api/instances")
            .await
            .map_err(|e| e.to_string())?;
        if (instances.status, instances.body.as_str()) != (200, "[]") {
            return Err(format!("the Director's /api/instances answered {}", instances.status));
        }
    }
    let status = driver.status().await.map_err(|e| e.to_string())?;
    if status.phase != Phase::Ready {
        return Err(format!("the module is not ready: {status:?}"));
    }
    driver.unload().await.map_err(|e| e.to_string())
}

/// A version with 1.14.7's shape and placeholder answers, for this crate's tests.
pub struct SampleHooks;

impl VersionHooks for SampleHooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        NoPlayerPolicy::ConfiguredAccount
    }

    fn weapons_at_base_level(&self, _: &crate::settings::Settings) -> bool {
        false
    }

    fn reforge_rules(&self) -> bool {
        true
    }

    fn mastery_persistence(&self) -> bool {
        true
    }

    fn huntpass_payload(&self, id: &str) -> Value {
        serde_json::json!({ "id": id })
    }

    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        serde_json::json!({ "userId": user_id, "objectiveId": row.objective_id })
    }

    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::AsIs
    }

    fn survey_config(&self) -> StubReply {
        StubReply {
            status: 204,
            body: None,
        }
    }

    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        Value::Array(list.iter().map(|e| Value::from(e.name.clone())).collect())
    }

    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        Destination {
            game_mode: if realm_hub { GameMode::City } else { GameMode::Island },
            hunt_id: Some(hunt_id.to_owned()),
        }
    }
}

pub static SAMPLE_TABLES: VersionTables = VersionTables {
    target_changelist: 647_472,
    game_version: "1.14.7",
    active_huntpass_id: "sample",
    recent_huntpass_seasons: &[],
    patchnotes_release_version: "1.14.7",
    bounty_item_grant_data: &[],
    store_sections: &[],
    seasonal_coin: None,
    masked_journey_nodes: &[],
    starter_manifest: StarterManifest {
        bootstrap_version: "1",
        stacked_item_ids: &[],
        quick_emoji_wheel: &[],
    },
    switch_defaults: &[
        ("qos_port", "80"),
        ("reforge", "on"),
        ("mastery_persistence", "on"),
        ("bounty_persistence", "on"),
        ("cooldown_persistence", "on"),
    ],
    session_payload: rustic_isles_auth::session_payload_1_14_7,
};

pub fn sample_version() -> IslesVersion {
    IslesVersion {
        hooks: Arc::new(SampleHooks),
        tables: &SAMPLE_TABLES,
        routes: RouteSet::V1147,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_paths() {
        let find = |path: &str| routes::all().iter().find(|r| r.path == path).unwrap();
        assert_eq!(sample_path(find("/loadout/:userId//all")), "/loadout/x//all");
        assert_eq!(
            sample_path(find(r"/^\/progression\/([^/]*)\/([^/]+)$/i")),
            "/__origin/gauntlet-prod.steelyard.ca/progression/x/x"
        );
        assert_eq!(
            sample_path(find(r"/^\/leaderboard\/get_leaderboard\//i")),
            "/__origin/gauntlet-prod.steelyard.ca/leaderboard/get_leaderboard/x"
        );
        assert_eq!(sample_path(find(r"/^\/inventory\/([^/]*)\/(.+)$/")), "/inventory/x/x");
        for row in routes::all() {
            let sample = sample_path(row);
            let path = match strip_origin(&sample) {
                OriginStrip::Stripped { target, .. } => target,
                _ => sample,
            };
            assert!(
                matches!(Pattern::parse(row.path).unwrap().matches(&path), Some(Ok(_))),
                "{}",
                row.name()
            );
        }
    }
}
