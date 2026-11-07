// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `content serve`: the stores, a listener per bind host, and the connection rules Node's server had (512
//! connections, a 20 s header time-out, a 120 s idle time-out).

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use dainty_service_kit::context::ServiceContext;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tokio::time::{Instant, Sleep};

use crate::app::{AppState, handle};
use crate::auth::Auth;
use crate::branding::Branding;
use crate::config::{Config, bind_address};
use crate::limits::Slots;
use crate::log;
use crate::modules::ContentModules;
use crate::news::News;
use crate::runtime::{Status, Store};
use crate::{Clock, now_ms};

/// Node's `server.maxConnections`
pub const MAX_CONNECTIONS: usize = 512;
/// How long a client may take to send its request headers.
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(20);
/// Node's `server.setTimeout(120000)`: a client that stops reading a download frees its slot
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Serves `config` standalone until stopped, with no modules attached.
pub async fn run(config: Config) -> Result<(), String> {
    run_with_context(config, ServiceContext::standalone("content")).await
}

/// Serves `config` until `ctx` says stop, with no modules attached.
pub async fn run_with_context(config: Config, ctx: ServiceContext) -> Result<(), String> {
    run_with_modules(config, ctx, Arc::new(ContentModules::default())).await
}

/// Serves until `ctx` says stop, with what the controller attached to `modules`.
pub async fn run_with_modules(
    config: Config,
    mut ctx: ServiceContext,
    modules: Arc<ContentModules>,
) -> Result<(), String> {
    ctx.status().report_starting("preparing content server");
    let app = prepare_with(&config, modules, Arc::new(now_ms)).await;
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut listeners = Vec::new();
    for host in &config.bind_hosts {
        let ip =
            bind_address(host).ok_or_else(|| format!("Could not listen on {host}:{}: not an address", config.port))?;
        let listener = TcpListener::bind(SocketAddr::new(ip, config.port))
            .await
            .map_err(|e| format!("Could not listen on {host}:{}: {e}", config.port))?;
        listeners.push(listener);
    }
    let serving: Vec<_> = listeners
        .into_iter()
        .map(|listener| tokio::spawn(serve(listener, app.clone(), stop_rx.clone())))
        .collect();
    ctx.status().report_ready(Some(config.port), "listening");
    let shown = |path: &Option<std::path::PathBuf>| path.as_ref().map(|p| p.display().to_string());
    let news = shown(&config.news_file).or_else(|| config.news_url.clone());
    log::info(
        "content server ready",
        json!({
            "listen": config.bind_hosts.iter().map(|h| format!("{h}:{}", config.port)).collect::<Vec<_>>(),
            "backend": config.backend_url,
            "runtime": shown(&config.runtime_dir),
            "branding": shown(&config.branding_dir),
            "news": news,
            "maxStreamsPerAccount": config.max_streams_per_account,
            "maxStreamsTotal": config.max_streams_total,
        }),
    );
    ctx.wait_stop().await;
    ctx.status().report_stopping("stopping");
    log::info("stopping", json!({"signal": "stop"}));
    _ = stop_tx.send(true);
    for task in serving {
        _ = task.await;
    }
    ctx.status().report_stopped("stopped");
    Ok(())
}

/// Everything up to listening, with no modules attached.
pub fn prepare(config: &Config) -> AppState {
    prepare_with_modules(config, Arc::new(ContentModules::default()))
}

/// [`prepare`], serving what is attached to `modules`.
pub fn prepare_with_modules(config: &Config, modules: Arc<ContentModules>) -> AppState {
    build(config, modules, Arc::new(now_ms))
}

async fn prepare_with(config: &Config, modules: Arc<ContentModules>, clock: Clock) -> AppState {
    let owned = config.clone();
    let (m, c) = (modules.clone(), clock.clone());
    // The first runtime check may hash the files
    match tokio::task::spawn_blocking(move || build(&owned, m, c)).await {
        Ok(app) => app,
        Err(_) => build(config, modules, clock),
    }
}

fn build(config: &Config, modules: Arc<ContentModules>, clock: Clock) -> AppState {
    let runtime = Store::new(config.runtime_dir.clone(), clock.clone());
    let first = runtime.status();
    if !matches!(first, Status::Ok(_)) {
        log::warn(
            "no runtime is offered to players yet; the launcher shows them that the host has to publish one",
            json!({
                "dir": config.runtime_dir.as_ref().map(|d| d.display().to_string()),
                "reason": first.reason(),
            }),
        );
    }
    let auth = Auth::backend(&config.backend_url, config.auth_cache_seconds, clock.clone());
    let slots = Slots::new(config.max_streams_per_account, config.max_streams_total);
    let branding = Branding::new(config.branding_dir.clone(), clock.clone());
    let news = News::new(config.news_file.clone(), config.news_url.clone(), clock);
    AppState::new(runtime, auth, slots, branding, news, modules)
}

/// Serves `listener` until `stop` turns true, then aborts every open connection.
pub async fn serve(listener: TcpListener, app: AppState, mut stop: watch::Receiver<bool>) {
    let room = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let mut connections = JoinSet::new();
    loop {
        if *stop.borrow() {
            break;
        }
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            changed = stop.changed() => {
                if changed.is_err() {
                    break;
                }
                continue;
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => continue,
        };
        let Ok((stream, peer)) = accepted else {
            continue;
        };
        // Past the limit a connection is closed at once
        let Ok(permit) = room.clone().try_acquire_owned() else {
            drop(stream);
            continue;
        };
        _ = stream.set_nodelay(true);
        let app = app.clone();
        connections.spawn(async move {
            let service = service_fn(move |request| {
                let app = app.clone();
                async move { Ok::<_, Infallible>(handle(app, request, peer).await) }
            });
            _ = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .serve_connection(TokioIo::new(Idle::new(stream)), service)
                .await;
            drop(permit);
        });
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

/// A connection that fails once nothing could be read or written for [`IDLE_TIMEOUT`].
struct Idle {
    stream: TcpStream,
    deadline: Pin<Box<Sleep>>,
}

impl Idle {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            deadline: Box::pin(tokio::time::sleep(IDLE_TIMEOUT)),
        }
    }

    fn progressed(&mut self) {
        self.deadline.as_mut().reset(Instant::now() + IDLE_TIMEOUT);
    }

    fn stalled<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        ready!(self.deadline.as_mut().poll(cx));
        Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "connection idle")))
    }
}

impl AsyncRead for Idle {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.stream).poll_read(cx, buf) {
            Poll::Ready(result) => {
                self.progressed();
                Poll::Ready(result)
            }
            Poll::Pending => self.stalled(cx),
        }
    }
}

impl AsyncWrite for Idle {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.stream).poll_write(cx, buf) {
            Poll::Ready(result) => {
                self.progressed();
                Poll::Ready(result)
            }
            Poll::Pending => self.stalled(cx),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
