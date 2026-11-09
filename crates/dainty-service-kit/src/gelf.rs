// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Log shipping to Graylog in GELF 1.1 (`[logs] gelf_url`, `docs/logging.md`). Logstash's `gelf` input, Fluentd,
//! Vector and Docker's own `gelf` driver read the same messages. Records are redacted before they get here.
//!
//! - `https://host:12201/gelf`: Graylog's GELF HTTP input, one message per POST on a kept connection.
//! - `http://` and `tcp://` (GELF TCP, NUL-delimited): only to this machine or a private network, since they are
//!   plain text.
//!
//! The layer never blocks a service: records queue (10 000 at most, the rest dropped and counted) for one thread
//! that sends them, and the shipper's own failures go to stderr, never back into the log.

use std::cell::Cell;
use std::io::Write;
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Method, Request};
use serde_json::{Map, Number, Value};

use crate::console::Record;

const QUEUE: usize = 10_000;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const SHORT_MESSAGE: usize = 250;

thread_local! {
    /// Set on the shipper's thread: what its HTTP client logs must not be shipped again.
    static SHIPPING: Cell<bool> = const { Cell::new(false) };
}

/// Where the records go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GelfTarget {
    /// GELF over HTTP, POSTed to `url`.
    Http {
        /// The collector's GELF HTTP input URL.
        url: String,
    },
    /// GELF over TCP, NUL-delimited, to `authority`.
    Tcp {
        /// `host:port` of the collector's GELF TCP input.
        authority: String,
    },
}

/// Loopback, a private or link-local address, or a name with no dot (a Docker or LAN service name).
fn is_private_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => !host.contains('.'),
    }
}

impl GelfTarget {
    /// `https://…`, or `http://…`/`tcp://host:port` to a private host.
    pub fn parse(text: &str) -> Result<Self, String> {
        let url = url::Url::parse(text.trim()).map_err(|e| format!("{text:?} is not a URL: {e}"))?;
        let host = url
            .host_str()
            .ok_or_else(|| format!("{text} names no host"))?
            .to_owned();
        if !url.username().is_empty() || url.password().is_some() {
            return Err("the GELF URL holds no user or password".into());
        }
        let private = is_private_host(&host);
        match url.scheme() {
            "https" => Ok(Self::Http { url: url.to_string() }),
            "http" if private => Ok(Self::Http { url: url.to_string() }),
            "tcp" if private => {
                let port = url
                    .port()
                    .ok_or_else(|| "a tcp:// GELF URL names its port".to_owned())?;
                if url.path() != "" && url.path() != "/" {
                    return Err("a tcp:// GELF URL is a host and a port, no more".into());
                }
                Ok(Self::Tcp {
                    authority: format!("{}:{port}", url.host_str().unwrap_or(&host)),
                })
            }
            "http" | "tcp" => Err(format!(
                "{text} is plain text to {host}: use https://, or http:// or tcp:// only to this machine or a private \
                 network"
            )),
            other => Err(format!("{other}:// is not a GELF transport (https, http or tcp)")),
        }
    }
}

/// Graylog's levels are syslog's.
fn syslog_level(level: &str) -> u8 {
    match level {
        "error" => 3,
        "warn" => 4,
        "info" => 6,
        _ => 7,
    }
}

/// `_name` for an additional field: GELF allows `[\w.-]`, and `_id` is Graylog's own.
fn field_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned == "id" {
        "_field_id".into()
    } else {
        format!("_{cleaned}")
    }
}

/// One GELF 1.1 message. `extra` are the server's own fields (`_server`, `_node`).
pub fn message(host: &str, record: &Record<'_>, extra: &[(String, String)]) -> Value {
    let mut map = Map::new();
    map.insert("version".into(), Value::from("1.1"));
    map.insert("host".into(), Value::from(host));
    let first = record.message.lines().next().unwrap_or("");
    let short: String = if first.is_empty() {
        record.target.to_owned()
    } else {
        first.chars().take(SHORT_MESSAGE).collect()
    };
    map.insert("short_message".into(), Value::String(short.clone()));
    if record.message != short {
        map.insert("full_message".into(), Value::from(record.message));
    }
    let seconds = record.timestamp_ms as f64 / 1000.0;
    map.insert(
        "timestamp".into(),
        Number::from_f64(seconds).map_or(Value::Null, Value::Number),
    );
    map.insert("level".into(), Value::from(syslog_level(record.level)));
    map.insert("_service".into(), Value::from(record.service));
    map.insert("_target".into(), Value::from(record.target));
    map.insert("_level_name".into(), Value::from(record.level));
    for (key, value) in extra.iter().chain(record.fields) {
        map.entry(field_name(key))
            .or_insert_with(|| Value::String(value.clone()));
    }
    Value::Object(map)
}

/// The queue the layer feeds and the shipper drains.
#[derive(Clone)]
pub struct GelfSink {
    tx: SyncSender<Vec<u8>>,
    host: Arc<str>,
    extra: Arc<[(String, String)]>,
    dropped: Arc<AtomicU64>,
}

impl GelfSink {
    /// Starts the shipper's thread.
    pub fn start(target: GelfTarget, extra: Vec<(String, String)>) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::sync_channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped);
        std::thread::Builder::new()
            .name("gelf".into())
            .spawn(move || {
                SHIPPING.with(|s| s.set(true));
                ship(target, rx, counter);
            })
            .map_err(|e| format!("could not start the GELF thread: {e}"))?;
        Ok(Self {
            tx,
            host: Arc::from(hostname()),
            extra: Arc::from(extra),
            dropped,
        })
    }

    /// Queues one record; never waits.
    pub fn send(&self, record: &Record<'_>) {
        if SHIPPING.with(Cell::get) {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(&message(&self.host, record, &self.extra)) else {
            return;
        };
        if let Err(TrySendError::Full(_)) = self.tx.try_send(bytes) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Records dropped so far: the queue was full or the collector did not take them.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

fn hostname() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| gethostname::gethostname().to_string_lossy().into_owned())
        .clone()
}

/// Says a failure once, until the next success.
struct Health {
    failing: bool,
}

impl Health {
    fn failed(&mut self, error: &str) {
        if !self.failing {
            eprintln!("gelf: the log collector did not take the logs ({error}); dropping them until it does");
            self.failing = true;
        }
    }

    fn ok(&mut self) {
        if self.failing {
            eprintln!("gelf: the log collector takes the logs again");
            self.failing = false;
        }
    }
}

fn ship(target: GelfTarget, rx: Receiver<Vec<u8>>, dropped: Arc<AtomicU64>) {
    let mut health = Health { failing: false };
    match target {
        GelfTarget::Tcp { authority } => {
            let mut stream: Option<TcpStream> = None;
            while let Ok(bytes) = rx.recv() {
                if stream.is_none() {
                    stream = connect(&authority).map_err(|e| health.failed(&e)).ok();
                }
                let sent = stream.as_mut().is_some_and(|s| {
                    s.write_all(&bytes)
                        .and_then(|()| s.write_all(&[0]))
                        .map_err(|e| health.failed(&e.to_string()))
                        .is_ok()
                });
                if sent {
                    health.ok();
                } else {
                    stream = None;
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        GelfTarget::Http { url } => {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                eprintln!("gelf: could not start the shipper's runtime; logs are not shipped");
                return;
            };
            let client: dainty_http::Client = dainty_http::client(dainty_http::Schemes::HttpOrHttps);
            while let Ok(bytes) = rx.recv() {
                let request = Request::builder()
                    .method(Method::POST)
                    .uri(url.as_str())
                    .header(hyper::header::CONTENT_TYPE, "application/json")
                    .body(Full::new(Bytes::from(bytes)));
                let Ok(request) = request else {
                    dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let answer =
                    runtime.block_on(async { tokio::time::timeout(SEND_TIMEOUT, client.request(request)).await });
                match answer {
                    Ok(Ok(response)) if response.status().is_success() => health.ok(),
                    Ok(Ok(response)) => {
                        health.failed(&format!("it answered {}", response.status()));
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Err(e)) => {
                        health.failed(&e.to_string());
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        health.failed("no answer in time");
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

fn connect(authority: &str) -> Result<TcpStream, String> {
    let address = authority
        .to_socket_addrs()
        .map_err(|e| format!("{authority}: {e}"))?
        .next()
        .ok_or_else(|| format!("{authority} has no address"))?;
    let stream = TcpStream::connect_timeout(&address, SEND_TIMEOUT).map_err(|e| format!("{authority}: {e}"))?;
    let _ = stream.set_write_timeout(Some(SEND_TIMEOUT));
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn record<'a>(message: &'a str, fields: &'a [(String, String)]) -> Record<'a> {
        Record {
            timestamp_ms: 1_793_966_400_123,
            level: "warn",
            service: "gateway",
            target: "gateway::server",
            message,
            fields,
        }
    }

    #[test]
    fn plain_text_only_stays_close() {
        assert!(GelfTarget::parse("https://logs.example.org:12201/gelf").is_ok());
        assert!(GelfTarget::parse("http://127.0.0.1:12201/gelf").is_ok());
        assert!(GelfTarget::parse("http://graylog:12201/gelf").is_ok());
        assert!(GelfTarget::parse("tcp://10.0.0.5:12201").is_ok());
        assert!(GelfTarget::parse("http://logs.example.org:12201/gelf").is_err());
        assert!(GelfTarget::parse("tcp://203.0.113.7:12201").is_err());
        assert!(GelfTarget::parse("tcp://graylog").is_err());
        assert!(GelfTarget::parse("udp://graylog:12201").is_err());
        assert!(GelfTarget::parse("https://user:pw@logs.example.org/gelf").is_err());
    }

    #[test]
    fn a_message_is_gelf_1_1() {
        let fields = vec![("port".to_owned(), "443".to_owned()), ("id".to_owned(), "7".to_owned())];
        let extra = vec![("server".to_owned(), "Friday Hunts".to_owned())];
        let value = message("box", &record("first\nsecond", &fields), &extra);
        assert_eq!(value["version"], "1.1");
        assert_eq!(value["short_message"], "first");
        assert_eq!(value["full_message"], "first\nsecond");
        assert_eq!(value["level"], 4);
        assert_eq!(value["timestamp"], 1_793_966_400.123);
        assert_eq!(value["_service"], "gateway");
        assert_eq!(value["_target"], "gateway::server");
        assert_eq!(value["_port"], "443");
        assert_eq!(value["_field_id"], "7");
        assert_eq!(value["_server"], "Friday Hunts");
        assert!(value.get("_id").is_none());
    }

    #[test]
    fn ships_over_tcp_nul_delimited() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let target = GelfTarget::parse(&format!("tcp://127.0.0.1:{port}")).unwrap();
        let sink = GelfSink::start(target, Vec::new()).unwrap();
        sink.send(&record("one", &[]));
        sink.send(&record("two", &[]));
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 1024];
        while got.iter().filter(|b| **b == 0).count() < 2 {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            got.extend_from_slice(&buf[..n]);
        }
        let frames: Vec<Value> = got
            .split(|b| *b == 0)
            .filter(|f| !f.is_empty())
            .map(|f| serde_json::from_slice(f).unwrap())
            .collect();
        assert_eq!(frames[0]["short_message"], "one");
        assert_eq!(frames[1]["short_message"], "two");
        assert_eq!(sink.dropped(), 0);
    }
}
