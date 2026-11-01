//! The QoS pong listener (`server.ts`, `config/listenerPlan.ts`): the client measures regions by pinging every
//! `regionUrls` entry of `GET /candidate/regions` over PLAIN HTTP and expects
//! `<!DOCTYPE html><html><body>pong</body></html>` on every path. Nothing listening there leaves every region
//! "9999ms Invalid" and the client travelling with no region at all.
//!
//! Node opens it on every interface at `QOS_PORT` (80). A game module opens no public listener (DD3, AGENTS.md), so
//! this binds 127.0.0.1 on the manifest's `qos` port and announces it in `isles.listening` (`qos`, with the public
//! port asked for in `qos_public_port`). Nothing publishes it yet: that needs a decision (`docs/security/qos.md`).
//! Under DaintyLass the host kit writes `QOS_PORT=0` and points `QOS_TARGET_URL` at the launcher's relay, which
//! answers the pings on the player's own machine, so no listener starts.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::settings::{QOS_PORT, Settings};
use crate::tables::VersionTables;

pub const PONG: &str = "<!DOCTYPE html><html><body>pong</body></html>";
pub const DEFAULT_QOS_PORT: u16 = 80;

/// What `QOS_PORT` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QosPlan {
    /// The public port the pings go to; `None` starts no listener
    pub port: Option<u16>,
    /// Why no listener starts when the value is neither a port nor `off`
    pub problem: Option<String>,
}

/// `ReadQosPort`: blank is the default, `off` and `0` start none, a port number is that port; anything else is
/// reported, not fatal.
pub fn read_qos_port(raw: Option<&str>, default: u16) -> QosPlan {
    let value = raw.unwrap_or_default().trim();
    if value.is_empty() {
        return QosPlan {
            port: Some(default),
            problem: None,
        };
    }
    if value.eq_ignore_ascii_case("off") || value == "0" {
        return QosPlan {
            port: None,
            problem: None,
        };
    }
    // `Number(value)` accepted as an integer from 1 to 65535
    let port = value
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| value.parse::<u32>().ok())
        .flatten()
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p >= 1);
    match port {
        Some(port) => QosPlan {
            port: Some(port),
            problem: None,
        },
        None => QosPlan {
            port: None,
            problem: Some(format!("QOS_PORT='{value}' is not a port number or 'off'")),
        },
    }
}

/// The module's plan: `QOS_PORT` from the env file or the switches, else the version's default.
pub fn plan(settings: &Settings, tables: &VersionTables) -> QosPlan {
    read_qos_port(
        settings.raw(QOS_PORT.env).or(tables.switch_default(QOS_PORT)),
        DEFAULT_QOS_PORT,
    )
}

/// Every method and path answers 200 `text/html` with the pong.
pub fn pong() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html"))],
        PONG,
    )
        .into_response()
}

pub fn app() -> Router {
    Router::new().fallback(|| async { pong() })
}

/// A running pong listener on loopback.
pub struct PongListener {
    pub address: SocketAddr,
    /// The public port `QOS_PORT` asked for, which only the gateway could open
    pub public_port: u16,
    pub stop: oneshot::Sender<()>,
    pub task: JoinHandle<()>,
}

/// Binds the loopback side on `bind_port` (0 for any). A failure is logged and changes nothing else: the game is
/// playable without a latency measurement, so a busy QoS port never takes the API down.
pub async fn start(settings: &Settings, tables: &VersionTables, bind_port: u16) -> Option<PongListener> {
    let plan = plan(settings, tables);
    let Some(public_port) = plan.port else {
        match plan.problem {
            Some(problem) => tracing::warn!("QoS listener not started: {problem}"),
            None => tracing::info!(
                "QoS latency-test listener off (QOS_PORT=off); the relay or a proxy answers the region pings"
            ),
        }
        return None;
    };
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", bind_port)).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!("QoS listener could not start: {error}");
            return None;
        }
    };
    let address = listener.local_addr().ok()?;
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let stop = async {
            _ = stopped.await;
        };
        if let Err(error) = service_kit::serve::serve(listener, app(), stop, Duration::from_secs(1)).await {
            tracing::warn!("QoS listener failed: {error}");
        }
    });
    tracing::info!(
        "QoS latency-test listener on {address} for public port {public_port} (answers pong for region pings)"
    );
    Some(PongListener {
        address,
        public_port,
        stop,
        task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(raw: Option<&str>) -> (Option<u16>, Option<String>) {
        let plan = read_qos_port(raw, 80);
        (plan.port, plan.problem)
    }

    #[test]
    fn qos_port_unset_is_the_default_off_starts_none_and_a_number_is_that_port() {
        assert_eq!(read(None), (Some(80), None));
        assert_eq!(read(Some("  ")), (Some(80), None));
        assert_eq!(read(Some("OFF")), (None, None));
        assert_eq!(read(Some("8080")), (Some(8080), None));
        // 0 is the host kit's spelling of off
        assert_eq!(read(Some("0")), (None, None));
        assert_eq!(
            read(Some("70000")),
            (None, Some("QOS_PORT='70000' is not a port number or 'off'".into()))
        );
        assert_eq!(
            read(Some("http")),
            (None, Some("QOS_PORT='http' is not a port number or 'off'".into()))
        );
        assert_eq!(read_qos_port(None, 8080).port, Some(8080));
    }

    #[test]
    fn the_module_plan_reads_the_switch() {
        assert_eq!(
            plan(
                &Settings::from_pairs([("QOS_PORT", "81")]),
                &crate::areas::matchmaking::testkit::TABLES
            )
            .port,
            Some(81)
        );
        assert_eq!(
            plan(
                &Settings::from_pairs([("QOS_PORT", "off")]),
                &crate::areas::matchmaking::testkit::TABLES
            )
            .port,
            None
        );
    }

    #[tokio::test]
    async fn the_loopback_listener_answers_pong_on_every_path() {
        let listener = start(
            &Settings::from_pairs([("QOS_PORT", "8081")]),
            &crate::areas::matchmaking::testkit::TABLES,
            0,
        )
        .await
        .unwrap();
        assert!(listener.address.ip().is_loopback());
        assert_eq!(listener.public_port, 8081);
        for (method, path) in [("GET", "/"), ("POST", "/anything/at/all"), ("GET", "/QoS")] {
            let (status, headers, body) = crate::testing::request(listener.address, method, path).await.unwrap();
            assert_eq!(status, 200);
            assert_eq!(body, PONG);
            assert!(headers.contains(&("content-type".to_owned(), "text/html".to_owned())));
        }
        _ = listener.stop.send(());
        _ = listener.task.await;
        assert!(
            start(
                &Settings::from_pairs([("QOS_PORT", "off")]),
                &crate::areas::matchmaking::testkit::TABLES,
                0
            )
            .await
            .is_none()
        );
    }
}
