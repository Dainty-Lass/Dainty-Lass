// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway's self-signed certificate: made once, from host.toml's addresses, and never regenerated, since every
//! invite's `fp` is its SHA-256.

use std::net::Ipv4Addr;

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
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

fn pem_encode(label: &str, der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in body.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn der_len(len: usize) -> Vec<u8> {
    if len < 0x80 {
        vec![len as u8]
    } else if len <= 0xff {
        vec![0x81, len as u8]
    } else if len <= 0xffff {
        vec![0x82, (len >> 8) as u8, (len & 0xff) as u8]
    } else {
        vec![0x83, (len >> 16) as u8, ((len >> 8) & 0xff) as u8, (len & 0xff) as u8]
    }
}

fn der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let len_bytes = der_len(content.len());
    let mut out = Vec::with_capacity(1 + len_bytes.len() + content.len());
    out.push(tag);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(content);
    out
}

fn der_seq(items: &[Vec<u8>]) -> Vec<u8> {
    let total: usize = items.iter().map(Vec::len).sum();
    let mut content = Vec::with_capacity(total);
    for item in items {
        content.extend_from_slice(item);
    }
    der_tlv(0x30, &content)
}

fn der_set(items: &[Vec<u8>]) -> Vec<u8> {
    let total: usize = items.iter().map(Vec::len).sum();
    let mut content = Vec::with_capacity(total);
    for item in items {
        content.extend_from_slice(item);
    }
    der_tlv(0x31, &content)
}

fn der_oid(oid: &[u8]) -> Vec<u8> {
    der_tlv(0x06, oid)
}

fn der_utf8_string(s: &str) -> Vec<u8> {
    der_tlv(0x0c, s.as_bytes())
}

fn der_bit_string(bytes: &[u8]) -> Vec<u8> {
    let mut content = Vec::with_capacity(1 + bytes.len());
    content.push(0x00);
    content.extend_from_slice(bytes);
    der_tlv(0x03, &content)
}

fn der_octet_string(bytes: &[u8]) -> Vec<u8> {
    der_tlv(0x04, bytes)
}

fn der_time(dt: DateTime<Utc>) -> Vec<u8> {
    let year = dt.year();
    if (1950..=2049).contains(&year) {
        let s = format!(
            "{:02}{:02}{:02}{:02}{:02}{:02}Z",
            year % 100,
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second()
        );
        der_tlv(0x17, s.as_bytes())
    } else {
        let s = format!(
            "{:04}{:02}{:02}{:02}{:02}{:02}Z",
            year,
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second()
        );
        der_tlv(0x18, s.as_bytes())
    }
}

fn der_cn_name(cn: &str) -> Vec<u8> {
    // OID 2.5.4.3 (id-at-commonName) = 55 04 03
    let atv = der_seq(&[der_oid(&[0x55, 0x04, 0x03]), der_utf8_string(cn)]);
    let rdn = der_set(&[atv]);
    der_seq(&[rdn])
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
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
        .map_err(|_| HostError::message("could not generate ECDSA P-256 key pair"))?;
    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref())
        .map_err(|_| HostError::message("could not parse generated PKCS#8 key"))?;

    let mut serial_bytes = [0u8; 16];
    rng.fill(&mut serial_bytes)
        .map_err(|_| HostError::message("could not generate certificate serial number"))?;
    serial_bytes[0] &= 0x7f;
    if serial_bytes[0] == 0 {
        serial_bytes[0] = 0x01;
    }

    // ecdsa-with-SHA256 OID: 1.2.840.10045.4.3.2 -> 2A 86 48 CE 3D 04 03 02
    let sig_alg = der_seq(&[der_oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02])]);

    // SubjectPublicKeyInfo for P-256:
    // id-ecPublicKey (1.2.840.10045.2.1): 2A 86 48 CE 3D 02 01
    // prime256v1 (1.2.840.10045.3.1.7): 2A 86 48 CE 3D 03 01 07
    let spki_alg = der_seq(&[
        der_oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
        der_oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]),
    ]);
    let spki = der_seq(&[spki_alg, der_bit_string(key_pair.public_key().as_ref())]);

    let cn = if req.common_name.trim().is_empty() {
        "localhost"
    } else {
        req.common_name.trim()
    };
    let dn = der_cn_name(cn);
    let validity_der = der_seq(&[der_time(validity.not_before), der_time(validity.not_after)]);

    // Extensions:
    // 1. BasicConstraints (2.5.29.19 = 55 1D 13): critical=true, CA=false (empty sequence)
    let basic_constraints_ext = der_seq(&[
        der_oid(&[0x55, 0x1d, 0x13]),
        der_tlv(0x01, &[0xff]),
        der_octet_string(&der_seq(&[])),
    ]);

    // 2. KeyUsage (2.5.29.15 = 55 1D 0F): digitalSignature (bit 0) -> BIT STRING 03 02 07 80
    let key_usage_ext = der_seq(&[
        der_oid(&[0x55, 0x1d, 0x0f]),
        der_tlv(0x01, &[0xff]),
        der_octet_string(&[0x03, 0x02, 0x07, 0x80]),
    ]);

    // 3. ExtendedKeyUsage (2.5.29.37 = 55 1D 25): serverAuth (1.3.6.1.5.5.7.3.1)
    let eku_ext = der_seq(&[
        der_oid(&[0x55, 0x1d, 0x25]),
        der_octet_string(&der_seq(&[der_oid(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01])])),
    ]);

    let mut exts = vec![basic_constraints_ext, key_usage_ext, eku_ext];

    // 4. SubjectAltName (2.5.29.17 = 55 1D 11)
    let mut san_items = Vec::new();
    for dns in &req.dns_names {
        let trimmed = dns.trim();
        if !trimmed.is_empty() {
            // dNSName [2] IA5String (tag 0x82)
            san_items.push(der_tlv(0x82, trimmed.as_bytes()));
        }
    }
    for ip in &req.ip_addresses {
        if let Ok(addr) = ip.trim().parse::<Ipv4Addr>() {
            // iPAddress [7] OCTET STRING (tag 0x87)
            san_items.push(der_tlv(0x87, &addr.octets()));
        }
    }
    if !san_items.is_empty() {
        let san_ext = der_seq(&[der_oid(&[0x55, 0x1d, 0x11]), der_octet_string(&der_seq(&san_items))]);
        exts.push(san_ext);
    }

    let extensions_wrapper = der_tlv(0xa3, &der_seq(&exts));

    // TBSCertificate:
    // version [0] EXPLICIT INTEGER v3 (2) -> A0 03 02 01 02
    let version = vec![0xa0, 0x03, 0x02, 0x01, 0x02];
    let serial = der_tlv(0x02, &serial_bytes);
    let tbs = der_seq(&[
        version,
        serial,
        sig_alg.clone(),
        dn.clone(),
        validity_der,
        dn,
        spki,
        extensions_wrapper,
    ]);

    let sig = key_pair
        .sign(&rng, &tbs)
        .map_err(|_| HostError::message("could not sign self-signed certificate"))?;
    let cert_der = der_seq(&[tbs, sig_alg, der_bit_string(sig.as_ref())]);

    let mut hasher = Sha256::new();
    hasher.update(&cert_der);
    let fingerprint: String = hasher
        .finalize()
        .iter()
        .flat_map(|&b| [b >> 4, b & 15])
        .filter_map(|n| char::from_digit(u32::from(n), 16))
        .collect();

    Ok(Cert {
        cert_pem: pem_encode("CERTIFICATE", &cert_der),
        key_pem: pem_encode("PRIVATE KEY", pkcs8.as_ref()),
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

/// Writes the gateway's certificate and key for host.toml's host and game address unless both are there.
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
}
