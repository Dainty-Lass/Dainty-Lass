// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The gateway's self-signed certificate: made once, from settings.toml's addresses, and never regenerated, since every
//! invite's `fp` is its SHA-256.

use std::net::{IpAddr, Ipv4Addr};

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::{DateTime, Duration, Utc};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
    SerialNumber,
};
use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::hostconfig::{HostConfig, is_ipv4, write_secret_file};
use crate::paths::Paths;

/// What a self-signed certificate names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertRequest {
    /// Its subject's common name.
    pub common_name: String,
    /// The DNS names it is valid for.
    pub dns_names: Vec<String>,
    /// The IPv4 addresses it is valid for.
    pub ip_addresses: Vec<String>,
}

/// When a certificate is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Validity {
    /// The first moment it is valid.
    pub not_before: DateTime<Utc>,
    /// The last moment it is valid.
    pub not_after: DateTime<Utc>,
}

/// A made certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cert {
    /// The certificate, PEM.
    pub cert_pem: String,
    /// Its ECDSA P-256 private key, PKCS#8 PEM.
    pub key_pem: String,
    /// The certificate's SHA-256, lowercase hex.
    pub fingerprint: String,
}

fn to_offset(dt: DateTime<Utc>) -> Result<time::OffsetDateTime, HostError> {
    time::OffsetDateTime::from_unix_timestamp(dt.timestamp())
        .map_err(|e| HostError::message(format!("invalid certificate timestamp: {e}")))
}

/// A certificate for `req`, valid from a day ago for ten years.
pub fn generate_self_signed_cert(req: &CertRequest) -> Result<Cert, HostError> {
    let now = Utc::now();
    let validity = Validity {
        not_before: now - Duration::days(1),
        not_after: now + Duration::days(3650),
    };
    generate_self_signed_cert_at(req, validity)
}

/// A certificate for `req` with an ECDSA P-256 key, valid for `validity`.
pub fn generate_self_signed_cert_at(req: &CertRequest, validity: Validity) -> Result<Cert, HostError> {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| HostError::message(format!("could not generate ECDSA P-256 key pair: {e}")))?;

    let mut serial_bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut serial_bytes)
        .map_err(|_| HostError::message("could not generate certificate serial number"))?;
    // Positive and non-zero
    serial_bytes[0] &= 0x7f;
    if serial_bytes[0] == 0 {
        serial_bytes[0] = 0x01;
    }

    let cn = if req.common_name.trim().is_empty() {
        "localhost"
    } else {
        req.common_name.trim()
    };

    let mut params = CertificateParams::default();
    params.serial_number = Some(SerialNumber::from_slice(&serial_bytes));
    params.not_before = to_offset(validity.not_before)?;
    params.not_after = to_offset(validity.not_after)?;
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, cn);
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    for dns in &req.dns_names {
        let trimmed = dns.trim();
        if !trimmed.is_empty() {
            let ia5 = trimmed
                .try_into()
                .map_err(|e| HostError::message(format!("invalid DNS SAN {trimmed:?}: {e}")))?;
            params.subject_alt_names.push(SanType::DnsName(ia5));
        }
    }
    for ip in &req.ip_addresses {
        if let Ok(addr) = ip.trim().parse::<Ipv4Addr>() {
            params.subject_alt_names.push(SanType::IpAddress(IpAddr::V4(addr)));
        }
    }

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| HostError::message(format!("could not sign self-signed certificate: {e}")))?;
    let fingerprint = hex::encode(Sha256::digest(cert.der()));

    Ok(Cert {
        // pem defaults to CRLF on Windows and LF on Unix; normalize to LF.
        cert_pem: cert.pem().replace("\r\n", "\n"),
        key_pem: key_pair.serialize_pem().replace("\r\n", "\n"),
        fingerprint,
    })
}

/// The certificate and key PEMs for `hosts`, the first one its common name.
pub fn generate_self_signed_cert_pem(hosts: &[String]) -> Result<(String, String), HostError> {
    let common_name = hosts.first().cloned().unwrap_or_else(|| "127.0.0.1".to_owned());
    let mut dns_names = Vec::new();
    let mut ip_addresses = Vec::new();
    for h in hosts {
        if is_ipv4(h) {
            if !ip_addresses.contains(h) {
                ip_addresses.push(h.clone());
            }
        } else if !dns_names.contains(h) {
            dns_names.push(h.clone());
        }
    }
    let cert = generate_self_signed_cert(&CertRequest {
        common_name,
        dns_names,
        ip_addresses,
    })?;
    Ok((cert.cert_pem, cert.key_pem))
}

/// Writes the gateway's certificate and key for settings.toml's host and game address unless both are there.
/// Returns whether it wrote them; the key is readable by its owner only.
pub fn write_gateway_cert_if_missing(paths: &Paths, config: &HostConfig) -> Result<bool, HostError> {
    if paths.cert_file.is_file() && paths.key_file.is_file() {
        return Ok(false);
    }
    std::fs::create_dir_all(&paths.tls).map_err(|e| HostError::io(&paths.tls, e))?;
    let mut dns_names = Vec::new();
    let mut ip_addresses = Vec::new();
    if is_ipv4(&config.host) {
        ip_addresses.push(config.host.clone());
    } else {
        dns_names.push(config.host.clone());
    }
    if is_ipv4(&config.game_address) && !ip_addresses.contains(&config.game_address) {
        ip_addresses.push(config.game_address.clone());
    }
    let req = CertRequest {
        common_name: config.host.clone(),
        dns_names,
        ip_addresses,
    };
    let cert = generate_self_signed_cert(&req)?;
    std::fs::write(&paths.cert_file, &cert.cert_pem).map_err(|e| HostError::io(&paths.cert_file, e))?;
    write_secret_file(&paths.key_file, &cert.key_pem)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::sync::Arc;

    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    use super::*;

    #[test]
    fn generates_valid_self_signed_ecdsa_cert_and_fingerprint() {
        let req = CertRequest {
            common_name: "203.0.113.10".to_owned(),
            dns_names: vec!["hunts.example.com".to_owned()],
            ip_addresses: vec!["203.0.113.10".to_owned()],
        };
        let cert = generate_self_signed_cert(&req).unwrap();
        assert!(cert.cert_pem.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert!(cert.key_pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert_eq!(cert.fingerprint.len(), 64);
        assert!(cert.fingerprint.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn generated_cert_matches_invite_fingerprint_and_loads_in_rustls() {
        let req = CertRequest {
            common_name: "203.0.113.10".to_owned(),
            dns_names: vec!["hunts.example.com".to_owned()],
            ip_addresses: vec!["203.0.113.10".to_owned()],
        };
        let cert = generate_self_signed_cert(&req).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("gateway-cert.pem");
        std::fs::write(&cert_path, &cert.cert_pem).unwrap();

        let fp = crate::invite::certificate_fingerprint(&cert_path).unwrap();
        assert_eq!(fp, cert.fingerprint);

        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert.cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let key = PrivateKeyDer::from_pem_slice(cert.key_pem.as_bytes()).unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
    }
}
