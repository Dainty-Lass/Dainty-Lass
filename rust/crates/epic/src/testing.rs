//! A fake Epic CDN for tests, here and in the host kit's (`test-support`): three HTTPS hosts on 127.0.0.1:0 with a
//! CA of their own, serving the chunks of a synthetic manifest. Nothing reaches Epic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::chunk::synthetic::encode;
use crate::manifest::synthetic::{Build, chunk, file};
use crate::manifest::{Manifest, Sha1Digest};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Good,
    /// Every chunk with one byte flipped in its payload
    Corrupt,
    /// A redirect to a good copy, which must not be followed
    Redirect,
    Missing,
    /// Serves this many chunks, then never answers (a download killed halfway)
    Stall(usize),
}

pub struct Cdn {
    pub bases: Vec<String>,
    /// Paths asked of each host
    seen: Vec<Arc<Mutex<Vec<String>>>>,
    pub tls: Arc<rustls::ClientConfig>,
}

impl Cdn {
    pub fn requests(&self, host: usize) -> Vec<String> {
        self.seen[host].lock().unwrap().clone()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// `files` by URL path; each host serves them as its behaviour says.
pub async fn cdn(hosts: &[Host], files: HashMap<String, Vec<u8>>) -> Cdn {
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let der = cert.cert.der().clone();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
    let server = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let mut roots = rustls::RootCertStore::empty();
    roots.add(der).unwrap();
    let tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let files = Arc::new(files);
    let mut bases = Vec::new();
    let mut seen = Vec::new();
    let mut ports = Vec::new();
    for _ in hosts {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        ports.push(listener.local_addr().unwrap().port());
        bases.push(listener);
    }
    let good_port = hosts.iter().position(|h| *h == Host::Good).map(|i| ports[i]);
    let mut out_bases = Vec::new();
    for (i, listener) in bases.into_iter().enumerate() {
        let behaviour = hosts[i];
        let log = Arc::new(Mutex::new(Vec::new()));
        seen.push(log.clone());
        out_bases.push(format!("https://127.0.0.1:{}/Builds/Jackal", ports[i]));
        let (acceptor, files) = (acceptor.clone(), files.clone());
        let served = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else { return };
                let (acceptor, files, log, served) = (acceptor.clone(), files.clone(), log.clone(), served.clone());
                tokio::spawn(async move {
                    let Ok(stream) = acceptor.accept(tcp).await else { return };
                    let service = service_fn(move |request: Request<Incoming>| {
                        let (files, log, served) = (files.clone(), log.clone(), served.clone());
                        async move {
                            let path = request.uri().path().to_owned();
                            log.lock().unwrap().push(path.clone());
                            let found = files.get(&path).cloned();
                            let response = match (behaviour, found) {
                                (_, None) | (Host::Missing, _) => Response::builder().status(404),
                                (Host::Redirect, Some(_)) => Response::builder().status(302).header(
                                    "location",
                                    format!("https://127.0.0.1:{}{path}", good_port.unwrap_or(1)),
                                ),
                                (Host::Stall(n), Some(body)) => {
                                    if served.fetch_add(1, Ordering::SeqCst) >= n {
                                        std::future::pending::<()>().await;
                                    }
                                    return Ok::<_, Infallible>(Response::new(Full::new(Bytes::from(body))));
                                }
                                (Host::Corrupt, Some(mut body)) => {
                                    let last = body.len() - 1;
                                    body[last] ^= 0x55;
                                    return Ok(Response::new(Full::new(Bytes::from(body))));
                                }
                                (Host::Good, Some(body)) => {
                                    return Ok(Response::new(Full::new(Bytes::from(body))));
                                }
                            };
                            Ok(response.body(Full::new(Bytes::new())).unwrap())
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
    }
    Cdn {
        bases: out_bases,
        seen,
        tls: Arc::new(tls),
    }
}

pub const EXE: &str = "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe";

/// A small version: chunks shared between files, one file with no parts, packed and stored chunks.
pub fn build() -> Build {
    Build {
        chunks: (1..=6).map(|n| chunk(n, 300 + usize::from(n) * 97)).collect(),
        files: vec![
            file("Version.txt", &[(0, 0, 59)]),
            file(EXE, &[(0, 59, 241), (1, 0, 494), (2, 10, 500)]),
            file("Archon/Content/Paks/a.pak", &[(3, 0, 688), (4, 0, 785), (2, 0, 10)]),
            file("Archon/Content/Paks/b.pak", &[(5, 0, 882), (1, 0, 4)]),
            file("Engine/Empty.ini", &[]),
        ],
        ..Build::default()
    }
}

pub struct Version {
    pub build: Build,
    pub manifest: Manifest,
    pub pin: Sha1Digest,
    /// URL path -> `.chunk` file
    pub files: HashMap<String, Vec<u8>>,
}

pub fn version() -> Version {
    version_of(build())
}

pub fn version_of(build: Build) -> Version {
    let bytes = build.bytes();
    let pin = Sha1Digest::of(&bytes);
    let manifest = Manifest::parse(&bytes, &pin).unwrap();
    let mut files = HashMap::new();
    for (i, (test, info)) in build.chunks.iter().zip(manifest.chunks()).enumerate() {
        let raw = encode(test.guid, &test.data, i % 2 == 0, 3);
        files.insert(format!("/Builds/Jackal/{}", info.cdn_path(manifest.feature_level)), raw);
    }
    files.insert("/m/good.manifest".into(), bytes);
    Version {
        build,
        manifest,
        pin,
        files,
    }
}
