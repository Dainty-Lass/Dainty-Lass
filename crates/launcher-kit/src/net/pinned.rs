// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedTarget {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    Pin { expected: String, actual: String },
    Timeout,
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

    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ConnectError::Io(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

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
    #![allow(clippy::unwrap_used, clippy::expect_used)]

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
}
