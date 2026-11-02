// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A fake backend on 127.0.0.1 (plain HTTP, which the client allows on loopback): it records every call and answers
//! as a healthy backend would, with an optional delay on every call but host-ready and a script of statuses for Guard
//! batches (0 = no answer).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub path: String,
    pub request_id: String,
    pub capability: String,
    pub user_agent: String,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct Log {
    pub consumes: Vec<Call>,
    pub disconnects: Vec<Call>,
    pub host_ready: Vec<Call>,
    pub leases: Vec<Call>,
    pub batches: Vec<Call>,
    pub completions: Vec<Call>,
}

#[derive(Default)]
struct State {
    log: Log,
    delay: Duration,
    script: VecDeque<u16>,
}

pub struct FakeBackend {
    pub url: String,
    state: Arc<Mutex<State>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

fn header(request: &Request<hyper::body::Incoming>, name: &str) -> String {
    request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

async fn answer(
    state: Arc<Mutex<State>>,
    request: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, std::io::Error> {
    let path = request.uri().path().to_owned();
    let request_id = header(&request, "x-rustic-request-id");
    let capability = header(&request, "x-rustic-host-capability");
    let user_agent = header(&request, "user-agent");
    let body = request
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let call = Call {
        path: path.clone(),
        request_id,
        capability,
        user_agent,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    let delay = state.lock().unwrap().delay;
    if !path.ends_with("/host-ready") && !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let (status, text) = {
        let mut state = state.lock().unwrap();
        let log = &mut state.log;
        if path.ends_with("/tickets/consume") {
            log.consumes.push(call);
            let id = format!("{:064x}", log.consumes.len());
            (
                200,
                format!("{{\"admitted\":true,\"admissionId\":\"{id}\",\"idempotent\":false}}"),
            )
        } else if path.ends_with("/host-ready") {
            log.host_ready.push(call);
            (200, "{\"ready\":true}".to_owned())
        } else if path.ends_with("/lease") {
            log.leases.push(call);
            (200, "{\"renewed\":true,\"state\":\"RUNNING\"}".to_owned())
        } else if path.ends_with("/event-batches") {
            log.batches.push(call);
            let n = log.batches.len().to_string();
            let hash = format!("{}{n}", "b".repeat(64 - n.len()));
            match state.script.pop_front() {
                Some(0) => return Err(std::io::Error::other("no answer")),
                Some(200) | None => (200, format!("{{\"accepted\":true,\"batchHash\":\"{hash}\"}}")),
                Some(status) => (status, "{}".to_owned()),
            }
        } else if path.ends_with("/completion") {
            log.completions.push(call);
            (200, "{\"accepted\":true,\"progressionApplied\":false}".to_owned())
        } else if path.ends_with("/disconnect") {
            log.disconnects.push(call);
            (200, "{}".to_owned())
        } else {
            (404, "{}".to_owned())
        }
    };
    let mut response = Response::new(Full::new(Bytes::from(text)));
    *response.status_mut() = hyper::StatusCode::from_u16(status).unwrap();
    Ok(response)
}

impl FakeBackend {
    pub fn start() -> Self {
        let state: Arc<Mutex<State>> = Arc::default();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server_state = state.clone();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                ready_tx.send(listener.local_addr().unwrap()).unwrap();
                loop {
                    tokio::select! {
                        _ = &mut stop_rx => return,
                        accepted = listener.accept() => {
                            let Ok((stream, _)) = accepted else { continue };
                            let state = server_state.clone();
                            tokio::spawn(async move {
                                let service = service_fn(move |request| answer(state.clone(), request));
                                let _ = http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
                            });
                        }
                    }
                }
            });
        });
        let addr = ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            url: format!("http://127.0.0.1:{}", addr.port()),
            state,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    pub fn set_delay(&self, delay: Duration) {
        self.state.lock().unwrap().delay = delay;
    }

    pub fn script(&self, statuses: &[u16]) {
        self.state.lock().unwrap().script = statuses.iter().copied().collect();
    }

    pub fn log(&self) -> Log {
        self.state.lock().unwrap().log.clone()
    }
}

impl Drop for FakeBackend {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
