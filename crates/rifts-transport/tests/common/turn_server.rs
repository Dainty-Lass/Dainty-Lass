// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A TURN server on 127.0.0.2 for the relay tests. On one PC a relay on the peers' own address lets a direct-to-relay
//! pair win, so the relay lives on an address no host candidate has.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rifts_transport::link::RelayServer;
use rifts_transport::url::ServerAddr;
use turn::auth::{AuthHandler, generate_auth_key};
use turn::relay::relay_static::RelayAddressGeneratorStatic;
use turn::server::Server;
use turn::server::config::{ConnConfig, ServerConfig};
use webrtc_util::vnet::net::Net;

pub const USERNAME: &str = "rustic-test";
pub const CREDENTIAL: &str = "rustic-test-credential";
const REALM: &str = "rustic.test";
pub const RELAY_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);

type Boxed<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = webrtc_util::Result<T>> + Send + 'a>>;

/// The server's socket, minus the receive errors Windows reports for an ICMP port unreachable (a closed client
/// socket): the server's read loop ends on the first receive error, which would stop the relay for every test after.
struct Tolerant(tokio::net::UdpSocket);

impl webrtc_util::Conn for Tolerant {
    fn connect<'a, 'b>(&'a self, addr: SocketAddr) -> Boxed<'b, ()>
    where
        'a: 'b,
    {
        Box::pin(async move { Ok(self.0.connect(addr).await?) })
    }

    fn recv<'a, 'b, 'c>(&'a self, buf: &'b mut [u8]) -> Boxed<'c, usize>
    where
        'a: 'c,
        'b: 'c,
    {
        Box::pin(async move { Ok(self.0.recv(buf).await?) })
    }

    fn recv_from<'a, 'b, 'c>(&'a self, buf: &'b mut [u8]) -> Boxed<'c, (usize, SocketAddr)>
    where
        'a: 'c,
        'b: 'c,
    {
        Box::pin(async move {
            loop {
                match self.0.recv_from(buf).await {
                    Ok(received) => return Ok(received),
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                    Err(e) => return Err(e.into()),
                }
            }
        })
    }

    fn send<'a, 'b, 'c>(&'a self, buf: &'b [u8]) -> Boxed<'c, usize>
    where
        'a: 'c,
        'b: 'c,
    {
        Box::pin(async move { Ok(self.0.send(buf).await?) })
    }

    fn send_to<'a, 'b, 'c>(&'a self, buf: &'b [u8], target: SocketAddr) -> Boxed<'c, usize>
    where
        'a: 'c,
        'b: 'c,
    {
        Box::pin(async move { Ok(self.0.send_to(buf, target).await?) })
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.0.local_addr()?)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }

    fn close<'a, 'b>(&'a self) -> Boxed<'b, ()>
    where
        'a: 'b,
    {
        Box::pin(async move { Ok(()) })
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

struct Auth(Vec<u8>);

impl AuthHandler for Auth {
    fn auth_handle(&self, username: &str, _realm: &str, _src: SocketAddr) -> Result<Vec<u8>, turn::Error> {
        if username == USERNAME {
            Ok(self.0.clone())
        } else {
            Err(turn::Error::ErrFakeErr)
        }
    }
}

pub struct TurnServer {
    pub addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TurnServer {
    pub fn start() -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let socket = tokio::net::UdpSocket::bind((RELAY_IP, 0)).await.unwrap();
                let addr = socket.local_addr().unwrap();
                let server = Server::new(ServerConfig {
                    conn_configs: vec![ConnConfig {
                        conn: Arc::new(Tolerant(socket)),
                        relay_addr_generator: Box::new(RelayAddressGeneratorStatic {
                            relay_address: IpAddr::V4(RELAY_IP),
                            address: RELAY_IP.to_string(),
                            net: Arc::new(Net::new(None)),
                        }),
                    }],
                    realm: REALM.to_owned(),
                    auth_handler: Arc::new(Auth(generate_auth_key(USERNAME, REALM, CREDENTIAL))),
                    channel_bind_timeout: Duration::from_secs(0),
                    alloc_close_notify: None,
                })
                .await
                .unwrap();
                ready_tx.send(addr).unwrap();
                let _ = stop_rx.await;
                let _ = server.close().await;
            });
        });
        let addr = ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            addr,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    pub fn relay(&self) -> RelayServer {
        RelayServer {
            server: ServerAddr {
                host: self.addr.ip().to_string(),
                port: self.addr.port(),
            },
            username: USERNAME.to_owned(),
            credential: CREDENTIAL.to_owned(),
        }
    }

    pub fn url(&self) -> String {
        format!("turn:{}:{}?transport=udp", self.addr.ip(), self.addr.port())
    }
}

impl Drop for TurnServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
