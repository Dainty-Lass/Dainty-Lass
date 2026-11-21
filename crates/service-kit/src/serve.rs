// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::future::{Future, pending};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::connect_info::Connected;
use axum::serve::{IncomingStream, Listener};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy)]
pub struct ConnectionInfo {
    pub peer: SocketAddr,
    pub local: SocketAddr,
}

impl Connected<IncomingStream<'_, TcpListener>> for ConnectionInfo {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        let peer = *stream.remote_addr();
        let local = stream
            .io()
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        Self { peer, local }
    }
}

impl Connected<IncomingStream<'_, NoDelay>> for ConnectionInfo {
    fn connect_info(stream: IncomingStream<'_, NoDelay>) -> Self {
        let peer = *stream.remote_addr();
        let local = stream
            .io()
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        Self { peer, local }
    }
}

pub struct NoDelay(pub TcpListener);

impl Listener for NoDelay {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (stream, addr) = Listener::accept(&mut self.0).await;
        let _ = stream.set_nodelay(true);
        (stream, addr)
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Listener::local_addr(&self.0)
    }
}

async fn stdin_shutdown() {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim() == "shutdown" {
            return;
        }
    }
    pending::<()>().await;
}

async fn terminate_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            sig.recv().await;
            return;
        }
    }
    #[cfg(windows)]
    {
        if let Ok(mut sig) = tokio::signal::windows::ctrl_break() {
            sig.recv().await;
            return;
        }
    }
    pending::<()>().await;
}

pub async fn stop_requested() {
    tokio::select! {
        res = tokio::signal::ctrl_c() => {
            if res.is_err() {
                pending::<()>().await;
            }
        }
        () = terminate_signal() => {}
        () = stdin_shutdown() => {}
    }
}

pub async fn serve(
    listener: TcpListener,
    app: Router,
    stop: impl Future<Output = ()> + Send + 'static,
    drain: Duration,
) -> io::Result<()> {
    let notify = Arc::new(Notify::new());
    let notify_stop = Arc::clone(&notify);

    let server = std::future::IntoFuture::into_future(
        axum::serve(
            NoDelay(listener),
            app.into_make_service_with_connect_info::<ConnectionInfo>(),
        )
        .with_graceful_shutdown(async move {
            stop.await;
            notify_stop.notify_one();
        }),
    );

    tokio::pin!(server);

    tokio::select! {
        res = &mut server => res,
        () = notify.notified() => {
            match tokio::time::timeout(drain, &mut server).await {
                Ok(res) => res,
                Err(_) => Ok(()),
            }
        }
    }
}
