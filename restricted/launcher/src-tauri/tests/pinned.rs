//! launcher-kit's pinned TLS (row L19, RL2), as the shell's `net/pinned.rs` tested it.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::time::Duration;

use common::{acceptor, certificate};
use launcher_kit::net::pinned::{ConnectError, PinnedTarget, certificate_fingerprint, pinned_connect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// A TLS server that writes "hello" and records the first bytes it received.
async fn server(acceptor: TlsAcceptor) -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        if let Ok(mut tls) = acceptor.accept(tcp).await {
            let _ = tls.write_all(b"hello").await;
            let mut buffer = [0; 64];
            if let Ok(n) = tls.read(&mut buffer).await {
                received.extend_from_slice(&buffer[..n]);
            }
        }
        let _ = tx.send(received);
    });
    (port, rx)
}

#[tokio::test]
async fn the_right_certificate_connects() {
    let (cert, key) = certificate();
    let fingerprint = certificate_fingerprint(&cert);
    let (port, received) = server(acceptor(cert, key)).await;
    let target = PinnedTarget {
        host: "127.0.0.1".into(),
        port,
        fingerprint,
    };
    let mut tls = pinned_connect(&target, Duration::from_secs(5)).await.unwrap();
    let mut hello = [0; 5];
    tls.read_exact(&mut hello).await.unwrap();
    assert_eq!(&hello, b"hello");
    tls.write_all(b"GET").await.unwrap();
    tls.flush().await.unwrap();
    assert_eq!(received.await.unwrap(), b"GET");
}

#[tokio::test]
async fn a_wrong_or_rotated_certificate_is_refused_before_anything_is_sent() {
    let (cert, key) = certificate();
    let actual = certificate_fingerprint(&cert);
    let (old, _) = certificate();
    let (port, received) = server(acceptor(cert, key)).await;
    let target = PinnedTarget {
        host: "localhost".into(),
        port,
        fingerprint: certificate_fingerprint(&old),
    };
    match pinned_connect(&target, Duration::from_secs(5)).await {
        Err(ConnectError::Pin { actual: seen, .. }) => assert_eq!(seen, actual),
        other => panic!("expected a pin error, got {other:?}"),
    }
    assert!(received.await.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn no_answer_times_out() {
    // A listener that never finishes the handshake
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let target = PinnedTarget {
        host: "127.0.0.1".into(),
        port,
        fingerprint: "0".repeat(64),
    };
    let result = pinned_connect(&target, Duration::from_millis(200)).await;
    assert!(matches!(result, Err(ConnectError::Timeout)), "{result:?}");
    drop(listener);
}
