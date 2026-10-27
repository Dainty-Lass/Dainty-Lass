//! What the module crates' tests share: a request for every route of the table, over real HTTP, and what each
//! should answer. Not for production code.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use module_api::HostCall;
use module_api::stub::RecordingHost;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::routes::{Pattern, RouteDef, RouteSet, Service, When};

/// The host the runtime prefixes for the Gauntlet routes.
pub const GAUNTLET_ORIGIN: &str = "/__origin/gauntlet-prod.steelyard.ca";

/// A path that reaches `route`: `x` for each parameter or regex group.
pub fn sample_path(route: &RouteDef) -> String {
    let path = if route.is_regex() {
        let literal = route.path.trim_start_matches("/^");
        let body = literal.rsplit_once('/').map_or(literal, |(body, _flags)| body);
        body.trim_end_matches('$')
            .replace("([^/]*)", "x")
            .replace("([^/]+)", "x")
            .replace("(.+)", "x")
            .replace("\\/", "/")
            // A regex that ends in a separator takes anything after it
            + if body.ends_with("\\/") { "x" } else { "" }
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

/// One HTTP/1.1 request on a fresh connection: the status, the headers (lower-case names) and the body.
pub async fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
) -> std::io::Result<(u16, Vec<(String, String)>, String)> {
    let mut stream = TcpStream::connect(address).await?;
    let head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    Ok((status, headers, body.to_owned()))
}

/// The addresses a started module announced (`isles.listening`), by port name.
pub fn listening(host: &RecordingHost) -> BTreeMap<String, SocketAddr> {
    host.calls()
        .into_iter()
        .filter_map(|(_, call)| match call {
            HostCall::Emit(event) if event.name.as_str() == crate::module::LISTENING_EVENT => Some(event.payload),
            _ => None,
        })
        .flat_map(|payload| {
            payload
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, address)| Some((name.clone(), address.as_str()?.parse().ok()?)))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// What a module mounts, as `http::app` decides it.
#[derive(Debug, Clone, Copy)]
pub struct Mounts {
    pub set: RouteSet,
    pub rifts: bool,
    pub admin: bool,
}

impl Mounts {
    pub fn mounted(&self, route: &RouteDef) -> bool {
        self.set.serves(route)
            && match route.when {
                When::Admin => self.admin,
                When::Rifts => self.rifts,
                _ => true,
            }
    }
}

/// Requests every route of the table on the service's address and returns what answered unlike expected: a
/// mounted route 501 in the launcher envelope, any other 404 (or 501 when a mounted route takes its path).
pub async fn sweep(service: Service, address: SocketAddr, mounts: Mounts) -> Vec<String> {
    let table = crate::routes::all();
    let handlers = crate::areas::backend();
    let mut failures = Vec::new();
    // A route an area has ported answers its own way; its own tests check it
    let handlers = if service == Service::Backend {
        crate::areas::backend()
    } else {
        crate::http::Handlers::default()
    };
    for route in table.iter().filter(|r| r.service == service) {
        let path = sample_path(route);
        let matches_path = |other: &RouteDef| {
            other.service == service
                && other.method == route.method
                && mounts.mounted(other)
                && (other.when != When::Gauntlet || route.when == When::Gauntlet)
                && Pattern::parse(other.path)
                    .ok()
                    .and_then(|p| p.matches(path.trim_start_matches(GAUNTLET_ORIGIN)))
                    .is_some()
        };
        if handlers.get(route).is_some()
            || table
                .iter()
                .any(|other| matches_path(other) && handlers.get(other).is_some())
        {
            continue;
        }
        let taken = mounts.mounted(route)
            || table.iter().any(|other| {
                other.service == service
                    && other.method == route.method
                    && mounts.mounted(other)
                    && (other.when != When::Gauntlet || route.when == When::Gauntlet)
                    && Pattern::parse(other.path)
                        .ok()
                        .and_then(|p| p.matches(path.trim_start_matches(GAUNTLET_ORIGIN)))
                        .is_some()
            });
        let gated = !mounts.admin && service == Service::Backend && path.to_ascii_lowercase().starts_with("/admin");
        let (want, envelope) = if taken && !gated { (501, true) } else { (404, false) };
        // An area that ported the route answers it itself, or the 501 envelope when no database is configured
        let handled = service == Service::Backend && mounts.mounted(route) && handlers.get(route).is_some();
        match request(address, route.method.as_str(), &path).await {
            Ok((status, headers, body)) if handled => {
                let has_id = headers.iter().any(|(name, _)| name == crate::http::REQUEST_ID_HEADER);
                if !has_id {
                    failures.push(format!(
                        "{} {path}: {status} {body} (no request id)",
                        route.method.as_str()
                    ));
                }
            }
            Ok((status, headers, body)) => {
                let has_id = headers.iter().any(|(name, _)| name == crate::http::REQUEST_ID_HEADER);
                let ok = status == want
                    && has_id
                    && (!envelope
                        || (body.contains(r#""code":"NOT_IMPLEMENTED""#) && body.contains(r#""requestId":""#)))
                    && (envelope || body.is_empty());
                if !ok {
                    failures.push(format!(
                        "{} {path}: {status} {body} (wanted {want})",
                        route.method.as_str()
                    ));
                }
            }
            Err(error) => failures.push(format!("{} {path}: {error}", route.method.as_str())),
        }
    }
    failures
}

/// Registers, starts (both listeners on ephemeral ports, `env_file` as the backend's env file), sweeps every
/// route of the backend and probes the Director, stops; the failures.
pub async fn drive(
    driver: &dyn module_api::ModuleDriver,
    host: &RecordingHost,
    env_file: &std::path::Path,
    mounts: Mounts,
) -> Vec<String> {
    use module_api::{ModuleConfig, ModuleDriverExt, Phase, PortRange};
    let mut failures = Vec::new();
    let registration = match driver.register().await {
        Ok(registration) => registration,
        Err(error) => return vec![format!("register: {error}")],
    };
    let mut config = ModuleConfig::defaults(driver.manifest(), &registration);
    for port in ["backend", "director"] {
        config.ports.insert(port.to_owned(), PortRange::single(0));
    }
    if let Some(backend) = config.sections.get_mut("backend") {
        backend.insert("env_file".into(), env_file.display().to_string().into());
    }
    if let Err(error) = driver.start(config).await {
        return vec![format!("start: {error}")];
    }
    let addresses = listening(host);
    match addresses.get("backend") {
        Some(address) => failures.extend(sweep(Service::Backend, *address, mounts).await),
        None => failures.push(format!("no backend address in {addresses:?}")),
    }
    // The Director answers for real (RU6): with no game server configured it is not ready and lists no instance.
    match addresses.get("director") {
        Some(address) => {
            let ready = request(*address, "GET", "/health/ready").await.map(|r| r.0);
            let instances = request(*address, "GET", "/api/instances").await.map(|r| (r.0, r.2));
            if ready.as_ref().ok() != Some(&503) || instances.as_ref().ok() != Some(&(200, "[]".to_owned())) {
                failures.push(format!("director: ready {ready:?}, instances {instances:?}"));
            }
        }
        None => failures.push(format!("no director address in {addresses:?}")),
    }
    match driver.status().await {
        Ok(status) if status.phase == Phase::Ready => {}
        other => failures.push(format!("status after start: {other:?}")),
    }
    if let Err(error) = driver.unload().await {
        failures.push(format!("unload: {error}"));
    }
    failures
}
