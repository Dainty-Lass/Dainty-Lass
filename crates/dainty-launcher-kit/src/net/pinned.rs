// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! TLS pinned to a certificate's SHA-256, the trust an invite's `fp` gives (AGENTS.md: checked before any byte is sent).

use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::core::pins::{normalize_pin, sha256_hex};

/// A TLS server to reach, trusted by its certificate's fingerprint rather than a CA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedTarget {
    /// Host name or IP address.
    pub host: String,
    /// TCP port, never 0.
    pub port: u16,
    /// SHA-256 of the certificate the server must present, in any form [`normalize_pin`] takes.
    pub fingerprint: String,
}

/// Why a connection could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// The server's certificate does not match the pin.
    Pin {
        /// The pinned fingerprint, lowercase hex SHA-256.
        expected: String,
        /// The fingerprint the server presented.
        actual: String,
    },
    /// The deadline passed.
    Timeout,
    /// The target was refused, or connecting or the handshake failed.
    Io(String),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pin { expected, actual } => {
                write!(f, "certificate pin mismatch: expected {expected}, got {actual}")
            }
            Self::Timeout => write!(f, "connection timed out"),
            Self::Io(msg) => write!(f, "connection failed: {msg}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// SHA-256 of a DER certificate as lowercase hex, the form invites carry as `fp`.
pub fn certificate_fingerprint(der: &[u8]) -> String {
    sha256_hex(der)
}

#[derive(Debug)]
struct PinVerifier {
    expected_pin: String,
    mismatch_actual: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let actual = certificate_fingerprint(end_entity.as_ref());
        if actual == self.expected_pin {
            Ok(ServerCertVerified::assertion())
        } else {
            if let Ok(mut guard) = self.mismatch_actual.lock() {
                *guard = Some(actual);
            }
            Err(TlsError::General("certificate pin mismatch".to_owned()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// `host` without IPv6 brackets, when it is an IP address or a plain DNS name (no spaces, at most 253 bytes).
pub fn validate_host(host: &str) -> Result<&str, ConnectError> {
    let trimmed = host.trim();
    if trimmed.is_empty() || trimmed != host {
        return Err(ConnectError::Io("invalid host".to_owned()));
    }
    let unbracketed = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    if unbracketed.parse::<IpAddr>().is_ok() {
        return Ok(unbracketed);
    }
    if unbracketed.len() > 253 {
        return Err(ConnectError::Io("host is too long".to_owned()));
    }
    for byte in unbracketed.bytes() {
        let valid = byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.' || byte == b'_';
        if !valid {
            return Err(ConnectError::Io("invalid host character".to_owned()));
        }
    }
    Ok(unbracketed)
}

/// A TLS connection to `target` whose certificate's SHA-256 matches the pin; CA trust and the name are not checked.
/// `timeout` covers the TCP connect and the handshake.
pub async fn pinned_connect(target: &PinnedTarget, timeout: Duration) -> Result<TlsStream<TcpStream>, ConnectError> {
    if target.port == 0 {
        return Err(ConnectError::Io("port cannot be 0".to_owned()));
    }
    let host = validate_host(&target.host)?;
    let expected_pin =
        normalize_pin(&target.fingerprint).ok_or_else(|| ConnectError::Io("invalid certificate pin".to_owned()))?;

    let server_name = match host.parse::<IpAddr>() {
        Ok(ip) => ServerName::IpAddress(ip.into()),
        Err(_) => ServerName::try_from(host.to_owned()).map_err(|e| ConnectError::Io(e.to_string()))?,
    };

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mismatch_actual = Arc::new(Mutex::new(None));
    let verifier = Arc::new(PinVerifier {
        expected_pin: expected_pin.clone(),
        mismatch_actual: Arc::clone(&mismatch_actual),
        provider: Arc::clone(&provider),
    });

    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| ConnectError::Io(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    // An IP address goes in as `ServerName::IpAddress`, for which rustls sends no SNI
    let connector = TlsConnector::from(Arc::new(config));
    let addr = if host.contains(':') {
        format!("[{host}]:{}", target.port)
    } else {
        format!("{host}:{}", target.port)
    };

    let fut = async {
        let tcp = TcpStream::connect(&addr)
            .await
            .map_err(|e| ConnectError::Io(e.to_string()))?;
        let _ = tcp.set_nodelay(true);
        match connector.connect(server_name, tcp).await {
            Ok(tls) => Ok(tls),
            Err(err) => {
                if let Ok(mut guard) = mismatch_actual.lock()
                    && let Some(actual) = guard.take()
                {
                    return Err(ConnectError::Pin {
                        expected: expected_pin,
                        actual,
                    });
                }
                Err(ConnectError::Io(err.to_string()))
            }
        }
    };

    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| ConnectError::Timeout)?
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::sync::Arc;
    use std::time::Duration;

    use rustls::ServerConfig;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    use super::*;

    pub fn certificate() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
            .expect("self-signed certificate generation should succeed");
        let cert_der = certified.cert.der().clone();
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()));
        (cert_der, key_der)
    }

    pub fn acceptor(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> TlsAcceptor {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("single cert");
        TlsAcceptor::from(Arc::new(config))
    }

    #[tokio::test]
    async fn connects_when_pin_matches_and_rejects_mismatch() {
        let (cert, key) = certificate();
        let pin = certificate_fingerprint(cert.as_ref());
        let tls_acceptor = acceptor(cert, key);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                if let Ok((tcp, _)) = listener.accept().await
                    && let Ok(mut stream) = tls_acceptor.accept(tcp).await
                {
                    let _ = stream.write_all(b"ok").await;
                    let _ = stream.shutdown().await;
                }
            }
        });

        let ok_target = PinnedTarget {
            host: "127.0.0.1".to_owned(),
            port,
            fingerprint: pin.clone(),
        };
        let mut stream = pinned_connect(&ok_target, Duration::from_secs(3)).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, b"ok");

        let bad_target = PinnedTarget {
            host: "127.0.0.1".to_owned(),
            port,
            fingerprint: "0".repeat(64),
        };
        let err = pinned_connect(&bad_target, Duration::from_secs(3)).await.unwrap_err();
        assert_eq!(
            err,
            ConnectError::Pin {
                expected: "0".repeat(64),
                actual: pin,
            }
        );

        server.abort();
    }

    #[derive(Debug)]
    struct Fixed(Arc<rustls::sign::CertifiedKey>);

    impl rustls::server::ResolvesServerCert for Fixed {
        fn resolve(&self, _: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
            Some(Arc::clone(&self.0))
        }
    }

    type Seen = (Option<String>, Option<Vec<u8>>, Option<rustls::ProtocolVersion>);

    /// Accepts one TLS client and answers its SNI, ALPN and version.
    async fn serve_once(config: ServerConfig) -> (u16, tokio::task::JoinHandle<Option<Seen>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.ok()?;
            let mut stream = acceptor.accept(tcp).await.ok()?;
            let (_, conn) = stream.get_ref();
            let seen = (
                conn.server_name().map(str::to_owned),
                conn.alpn_protocol().map(<[u8]>::to_vec),
                conn.protocol_version(),
            );
            let _ = stream.shutdown().await;
            Some(seen)
        });
        (port, task)
    }

    fn server_config(versions: &[&'static rustls::SupportedProtocolVersion]) -> (ServerConfig, String) {
        let (cert, key) = certificate();
        let pin = certificate_fingerprint(cert.as_ref());
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        (config, pin)
    }

    #[tokio::test]
    async fn offers_http11_and_names_only_a_name() {
        for (host, sni) in [("127.0.0.1", None), ("localhost", Some("localhost".to_owned()))] {
            let (config, pin) = server_config(&[&rustls::version::TLS13]);
            let (port, server) = serve_once(config).await;
            let target = PinnedTarget {
                host: host.to_owned(),
                port,
                fingerprint: pin,
            };
            let _stream = pinned_connect(&target, Duration::from_secs(3)).await.unwrap();
            let (seen_sni, alpn, _) = server.await.unwrap().unwrap();
            assert_eq!(seen_sni, sni, "{host}");
            assert_eq!(alpn.as_deref(), Some(&b"http/1.1"[..]));
        }
    }

    #[tokio::test]
    async fn speaks_tls12_and_tls13() {
        for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            let (config, pin) = server_config(&[version]);
            let (port, server) = serve_once(config).await;
            let target = PinnedTarget {
                host: "127.0.0.1".to_owned(),
                port,
                fingerprint: pin,
            };
            pinned_connect(&target, Duration::from_secs(3)).await.unwrap();
            let (_, _, seen) = server.await.unwrap().unwrap();
            assert_eq!(seen, Some(version.version));
        }
    }

    #[tokio::test]
    async fn a_matching_certificate_signed_by_another_key_fails() {
        let (cert, _) = certificate();
        let (_, other_key) = certificate();
        let pin = certificate_fingerprint(cert.as_ref());
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let signing = provider.key_provider.load_private_key(other_key).unwrap();
        let resolver = Fixed(Arc::new(rustls::sign::CertifiedKey::new(vec![cert], signing)));
        for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            let config = ServerConfig::builder_with_provider(Arc::clone(&provider))
                .with_protocol_versions(&[version])
                .unwrap()
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(Fixed(Arc::clone(&resolver.0))));
            let (port, server) = serve_once(config).await;
            let target = PinnedTarget {
                host: "127.0.0.1".to_owned(),
                port,
                fingerprint: pin.clone(),
            };
            let err = pinned_connect(&target, Duration::from_secs(3)).await.unwrap_err();
            assert!(matches!(err, ConnectError::Io(_)), "{err:?}");
            server.abort();
        }
    }

    #[tokio::test]
    async fn the_deadline_covers_the_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // Takes the connection and never answers the hello
        let server = tokio::spawn(async move {
            let held = listener.accept().await;
            std::future::pending::<()>().await;
            drop(held);
        });
        let target = PinnedTarget {
            host: "127.0.0.1".to_owned(),
            port,
            fingerprint: "a".repeat(64),
        };
        let err = pinned_connect(&target, Duration::from_millis(200)).await.unwrap_err();
        assert_eq!(err, ConnectError::Timeout);
        server.abort();
    }
}
