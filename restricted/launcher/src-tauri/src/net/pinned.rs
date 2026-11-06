//! TLS pinned to the invite's certificate (row L19), from launcher-kit's `net::pinned` (RL2). The test
//! certificate and acceptor stay here for this crate's tests.

pub use launcher_kit::net::pinned::PinnedTarget;
#[cfg(test)]
pub use launcher_kit::net::pinned::{ConnectError, certificate_fingerprint, pinned_connect};

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rustls::pki_types::CertificateDer;

    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    /// A self-signed certificate for 127.0.0.1 and localhost: (certificate DER, key DER).
    pub fn certificate() -> (CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>) {
        let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into(), "localhost".into()]).unwrap();
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
        (cert.cert.der().clone(), key)
    }

    pub fn acceptor(cert: CertificateDer<'static>, key: rustls::pki_types::PrivateKeyDer<'static>) -> TlsAcceptor {
        let config =
            rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap();
        TlsAcceptor::from(Arc::new(config))
    }

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
}
