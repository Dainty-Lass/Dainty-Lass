// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The public certificate: loaded at start and never made there, because its SHA-256 is the `fp` of every invite
//! already sent; and `gateway make-cert`, which makes one for a new install or prints one's fingerprint.

use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

use crate::json_string;

/// What a certificate says about itself, as the gateway reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    /// Lowercase hex SHA-256 of the DER
    pub fingerprint: String,
    /// Milliseconds since the epoch
    pub not_before: i64,
    /// End of validity, milliseconds since the epoch.
    pub not_after: i64,
    /// As Node's `X509Certificate.subjectAltName` writes it
    pub subject_alt_name: String,
}

/// A certificate chain and its key, checked to belong together.
#[derive(Debug)]
pub struct Loaded {
    /// The certificate chain, leaf first.
    pub chain: Vec<CertificateDer<'static>>,
    /// The private key.
    pub key: PrivateKeyDer<'static>,
    /// What the leaf says about itself.
    pub info: CertInfo,
}

/// JavaScript's `toISOString`.
pub fn iso(ms: i64) -> String {
    crate::iso_ms(ms)
}

fn ipv6_node(bytes: &[u8]) -> String {
    bytes
        .chunks(2)
        .map(|w| format!("{:X}", u16::from_be_bytes([w[0], w.get(1).copied().unwrap_or(0)])))
        .collect::<Vec<_>>()
        .join(":")
}

/// Describes the DER of one certificate.
pub fn describe(der: &[u8]) -> Result<CertInfo, String> {
    use x509_parser::extensions::GeneralName;
    let (_, cert) = x509_parser::parse_x509_certificate(der).map_err(|e| format!("not a certificate: {e}"))?;
    let validity = cert.validity();
    let ms = |t: &x509_parser::time::ASN1Time| t.timestamp().checked_mul(1000);
    let (Some(not_before), Some(not_after)) = (ms(&validity.not_before), ms(&validity.not_after)) else {
        return Err("certificate date out of range".to_owned());
    };
    if chrono::DateTime::from_timestamp_millis(not_before).is_none()
        || chrono::DateTime::from_timestamp_millis(not_after).is_none()
    {
        return Err("certificate date out of range".to_owned());
    }
    let mut names = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            match name {
                GeneralName::DNSName(n) => names.push(format!("DNS:{n}")),
                GeneralName::RFC822Name(n) => names.push(format!("email:{n}")),
                GeneralName::URI(n) => names.push(format!("URI:{n}")),
                GeneralName::IPAddress(b) if b.len() == 4 => {
                    names.push(format!("IP Address:{}.{}.{}.{}", b[0], b[1], b[2], b[3]));
                }
                GeneralName::IPAddress(b) if b.len() == 16 => names.push(format!("IP Address:{}", ipv6_node(b))),
                _ => {}
            }
        }
    }
    Ok(CertInfo {
        fingerprint: crate::hex(&Sha256::digest(der)),
        not_before,
        not_after,
        subject_alt_name: names.join(", "),
    })
}

fn read_chain(cert_file: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let pem = std::fs::read(cert_file).map_err(|e| e.to_string())?;
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&pem).filter_map(Result::ok).collect();
    if chain.is_empty() {
        return Err("no certificate in the file".to_owned());
    }
    Ok(chain)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

fn load_inner(cert_file: &Path, key_file: &Path) -> Result<Loaded, String> {
    let chain = read_chain(cert_file)?;
    let mismatch = || "the key does not belong to the certificate".to_owned();
    let key_pem = std::fs::read(key_file).map_err(|_| mismatch())?;
    let key = PrivateKeyDer::from_pem_slice(&key_pem).map_err(|_| mismatch())?;
    let signing = provider()
        .key_provider
        .load_private_key(key.clone_key())
        .map_err(|_| mismatch())?;
    rustls::sign::CertifiedKey::new(chain.clone(), signing)
        .keys_match()
        .map_err(|_| mismatch())?;
    let info = describe(&chain[0])?;
    Ok(Loaded { chain, key, info })
}

/// The certificate chain and key at these paths; an error reads `Certificate <cert> / key <key>: <error>`.
pub fn load(cert_file: &Path, key_file: &Path) -> Result<Loaded, String> {
    load_inner(cert_file, key_file)
        .map_err(|e| format!("Certificate {} / key {}: {e}", cert_file.display(), key_file.display()))
}

/// aws-lc-rs, TLS 1.3 and 1.2, no client authentication, ALPN `http/1.1` only.
pub fn server_config(loaded: &Loaded) -> Result<Arc<rustls::ServerConfig>, String> {
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(loaded.chain.clone(), loaded.key.clone_key())
        .map_err(|e| e.to_string())?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// A DNS name a certificate may carry: no wildcard, no trailing dot, at least one letter.
pub fn is_dns_name(name: &str) -> bool {
    name.len() <= 253
        && name.bytes().any(|b| b.is_ascii_alphabetic())
        && name.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// Where `make-cert` writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Where {
    /// `gateway-cert.pem` and `gateway-key.pem` in this folder (`--out`).
    Out(PathBuf),
    /// These two files (`--cert`, `--key`).
    Files {
        /// The certificate file.
        cert: PathBuf,
        /// The key file.
        key: PathBuf,
    },
}

/// What `gateway make-cert` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MakeCert {
    /// Print the usage.
    Help,
    /// Print a certificate's fingerprint (`--fingerprint`).
    Fingerprint {
        /// The certificate file.
        file: PathBuf,
        /// Print JSON.
        json: bool,
    },
    /// Make a new certificate and key.
    Make {
        /// Names and addresses it is for (`--host`).
        hosts: Vec<String>,
        /// Where to write it.
        place: Where,
        /// Overwrite existing files (`--force`).
        force: bool,
        /// Print JSON.
        json: bool,
    },
}

/// One subject alternative name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Name {
    Ip(IpAddr),
    Dns(String),
}

fn check_hosts(raw: &[String]) -> Result<Vec<Name>, String> {
    if raw.is_empty() {
        return Err("give at least one --host (the server's public IP address or DNS name)".to_owned());
    }
    let mut seen: Vec<String> = Vec::new();
    let mut names = Vec::new();
    for host in raw {
        let text = host.trim().to_lowercase();
        if seen.contains(&text) {
            continue;
        }
        seen.push(text.clone());
        if let Some((addr, _)) = text.split_once('%')
            && addr.parse::<IpAddr>().is_ok()
        {
            return Err(format!("{host}: no zone ids"));
        }
        if let Ok(ip) = text.parse::<IpAddr>() {
            names.push(Name::Ip(ip));
        } else if is_dns_name(&text) {
            names.push(Name::Dns(text));
        } else {
            return Err(format!("{} is neither an IP address nor a DNS name", json_string(host)));
        }
    }
    Ok(names)
}

/// Reads `make-cert`'s arguments in order.
pub fn parse_make_cert_args(args: &[String]) -> Result<MakeCert, String> {
    let mut hosts = Vec::new();
    let (mut out, mut cert, mut key, mut fingerprint) = (None, None, None, None);
    let (mut force, mut json) = (false, false);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || {
            iter.next()
                .filter(|v| !v.starts_with("--"))
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--host" => hosts.push(value()?),
            "--out" => out = Some(value()?),
            "--cert" => cert = Some(value()?),
            "--key" => key = Some(value()?),
            "--fingerprint" => fingerprint = Some(value()?),
            "--force" => force = true,
            "--json" => json = true,
            "--help" | "-h" => return Ok(MakeCert::Help),
            other => return Err(format!("unknown argument {}", json_string(other))),
        }
    }
    if let Some(file) = fingerprint {
        return Ok(MakeCert::Fingerprint {
            file: crate::config::absolute(&file),
            json,
        });
    }
    let place = match (out, cert, key) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => return Err("use either --out or --cert/--key".to_owned()),
        (Some(dir), None, None) => Where::Out(crate::config::absolute(&dir)),
        (None, Some(cert), Some(key)) => Where::Files {
            cert: crate::config::absolute(&cert),
            key: crate::config::absolute(&key),
        },
        _ => return Err("say where to write: --out <dir>, or --cert <file> and --key <file>".to_owned()),
    };
    Ok(MakeCert::Make {
        hosts,
        place,
        force,
        json,
    })
}

const USAGE: &str = "usage: gateway make-cert --host <public IP or DNS name> [--host ...] (--out <dir> | --cert <file> --key <file>) [--force] [--json]
       gateway make-cert --fingerprint <cert.pem> [--json]

  gateway make-cert --host 203.0.113.7 [--host play.example.org] --out C:\\ReforgedServer\\tls
  gateway make-cert --fingerprint C:\\ReforgedServer\\tls\\gateway-cert.pem";

fn usage(message: Option<&str>) -> ExitCode {
    let mut err = std::io::stderr().lock();
    if let Some(message) = message {
        _ = writeln!(err, "make-cert: {message}");
    }
    _ = writeln!(err, "{USAGE}");
    ExitCode::from(2)
}

/// A new certificate and key, as PEM.
pub struct Made {
    /// The certificate.
    pub cert_pem: String,
    /// The private key.
    pub key_pem: String,
}

fn offset(ms: i64) -> Result<time::OffsetDateTime, String> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).map_err(|e| e.to_string())
}

/// Self-signed RSA 2048 with SHA-256, valid from a day ago for ten calendar years.
fn make(names: &[Name]) -> Result<Made, String> {
    use aws_lc_rs::rand::SecureRandom as _;
    use rcgen::{
        CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType, SerialNumber,
    };
    let key = KeyPair::generate_for(&rcgen::PKCS_RSA_SHA256).map_err(|e| e.to_string())?;
    let mut serial = [0u8; 16];
    aws_lc_rs::rand::SystemRandom::new()
        .fill(&mut serial)
        .map_err(|_| "no random bytes".to_owned())?;
    // Positive and non-zero
    serial[0] = (serial[0] & 0x7f) | 0x01;
    let mut params = CertificateParams::default();
    params.serial_number = Some(SerialNumber::from_slice(&serial));
    let not_before = offset(crate::now_ms() - 86_400_000)?;
    params.not_after = not_before
        .replace_year(not_before.year() + 10)
        .or_else(|_| {
            // 29 February ten years on is 1 March, as JavaScript's setFullYear makes it
            not_before
                .replace_day(28)
                .and_then(|d| d.replace_year(not_before.year() + 10))
                .map(|d| d + time::Duration::days(1))
        })
        .map_err(|e| e.to_string())?;
    params.not_before = not_before;
    let first = match names.first() {
        Some(Name::Ip(ip)) => ip.to_string(),
        Some(Name::Dns(d)) => d.clone(),
        None => String::new(),
    };
    params.distinguished_name.push(DnType::CommonName, first);
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Dainty Lass server");
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    for name in names {
        params.subject_alt_names.push(match name {
            Name::Ip(ip) => SanType::IpAddress(*ip),
            Name::Dns(d) => SanType::DnsName(d.clone().try_into().map_err(|e: rcgen::Error| e.to_string())?),
        });
    }
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    Ok(Made {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// Folders made, the key written first (0600 where there are modes), then the certificate.
fn write_pair(cert_file: &Path, key_file: &Path, made: &Made) -> Result<(), String> {
    for file in [cert_file, key_file] {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    write_private(key_file, &made.key_pem).map_err(|e| format!("{}: {e}", key_file.display()))?;
    std::fs::write(cert_file, &made.cert_pem).map_err(|e| format!("{}: {e}", cert_file.display()))
}

fn first_info(file: &Path) -> Result<CertInfo, String> {
    describe(&read_chain(file)?[0])
}

/// `gateway make-cert <args>`.
pub fn make_cert_main(args: &[String]) -> ExitCode {
    let command = match parse_make_cert_args(args) {
        Ok(MakeCert::Help) => return usage(None),
        Ok(command) => command,
        Err(e) => return usage(Some(&e)),
    };
    let (hosts, place, force, json) = match command {
        MakeCert::Fingerprint { file, json } => {
            let info = match first_info(&file) {
                Ok(info) => info,
                Err(e) => return usage(Some(&format!("cannot read {}: {e}", file.display()))),
            };
            if json {
                let line = serde_json::json!({
                    "cert": file.display().to_string(),
                    "fingerprint": info.fingerprint,
                    "notBefore": iso(info.not_before),
                    "notAfter": iso(info.not_after),
                    "names": info.subject_alt_name,
                });
                println!("{line}");
            } else {
                println!("Names:       {}", info.subject_alt_name);
                println!("Valid:       {} to {}", iso(info.not_before), iso(info.not_after));
                println!("Fingerprint: {}", info.fingerprint);
            }
            return ExitCode::SUCCESS;
        }
        MakeCert::Make {
            hosts,
            place,
            force,
            json,
        } => (hosts, place, force, json),
        MakeCert::Help => return usage(None),
    };
    let names = match check_hosts(&hosts) {
        Ok(names) => names,
        Err(e) => return usage(Some(&e)),
    };
    let (cert_file, key_file) = match place {
        Where::Out(dir) => (dir.join("gateway-cert.pem"), dir.join("gateway-key.pem")),
        Where::Files { cert, key } => (cert, key),
    };
    if !force && let Some(existing) = [&cert_file, &key_file].into_iter().find(|f| f.exists()) {
        return usage(Some(&format!(
            "{} exists. A new certificate changes the fingerprint and breaks every invite already sent; add --force if that is what you want",
            existing.display()
        )));
    }
    let result = make(&names).and_then(|made| {
        write_pair(&cert_file, &key_file, &made)?;
        first_info(&cert_file)
    });
    let info = match result {
        Ok(info) => info,
        Err(e) => {
            eprintln!("make-cert: {e}");
            return ExitCode::from(1);
        }
    };
    if json {
        let line = serde_json::json!({
            "cert": cert_file.display().to_string(),
            "key": key_file.display().to_string(),
            "fingerprint": info.fingerprint,
            "notBefore": iso(info.not_before),
            "notAfter": iso(info.not_after),
            "names": info.subject_alt_name,
        });
        println!("{line}");
    } else {
        println!("Certificate: {}", cert_file.display());
        println!("Key:         {}  (keep private)", key_file.display());
        println!("Names:       {}", info.subject_alt_name);
        println!("Valid:       {} to {}", iso(info.not_before), iso(info.not_after));
        println!("Fingerprint: {}", info.fingerprint);
        println!();
        println!("Invites for this server carry: fp={}", info.fingerprint);
    }
    ExitCode::SUCCESS
}

/// The host kit's cold start (never the gateway's): load both, refuse with one, or make a pair for `hosts`
/// (default `127.0.0.1` and `localhost`). The flag says whether one was made.
pub fn load_or_generate(cert_file: &Path, key_file: &Path, hosts: &[String]) -> Result<(Loaded, bool), String> {
    match (cert_file.exists(), key_file.exists()) {
        (true, true) => load(cert_file, key_file).map(|l| (l, false)),
        (true, false) | (false, true) => Err(format!(
            "only one of {} and {} exists; refusing to overwrite",
            cert_file.display(),
            key_file.display()
        )),
        (false, false) => {
            let defaults = ["127.0.0.1".to_owned(), "localhost".to_owned()];
            let names = check_hosts(if hosts.is_empty() { &defaults } else { hosts })?;
            write_pair(cert_file, key_file, &make(&names)?)?;
            let loaded = load(cert_file, key_file)?;
            tracing::info!(cert = %cert_file.display(), fingerprint = %loaded.info.fingerprint, "made a gateway certificate");
            Ok((loaded, true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn argument_errors() {
        assert_eq!(
            parse_make_cert_args(&args(&["--host"])).unwrap_err(),
            "--host needs a value"
        );
        assert_eq!(
            parse_make_cert_args(&args(&["--out", "--force"])).unwrap_err(),
            "--out needs a value"
        );
        assert_eq!(
            parse_make_cert_args(&args(&["-x"])).unwrap_err(),
            "unknown argument \"-x\""
        );
        assert_eq!(
            parse_make_cert_args(&args(&["--host", "a", "--out", "d", "--key", "k"])).unwrap_err(),
            "use either --out or --cert/--key"
        );
        assert_eq!(
            parse_make_cert_args(&args(&["--host", "a", "--cert", "c"])).unwrap_err(),
            "say where to write: --out <dir>, or --cert <file> and --key <file>"
        );
        assert_eq!(parse_make_cert_args(&args(&["-h"])).unwrap(), MakeCert::Help);
    }

    #[test]
    fn hosts() {
        assert_eq!(
            check_hosts(&[]).unwrap_err(),
            "give at least one --host (the server's public IP address or DNS name)"
        );
        assert_eq!(
            check_hosts(&args(&["fe80::1%eth0"])).unwrap_err(),
            "fe80::1%eth0: no zone ids"
        );
        assert_eq!(
            check_hosts(&args(&["*.example.org"])).unwrap_err(),
            "\"*.example.org\" is neither an IP address nor a DNS name"
        );
        let names = check_hosts(&args(&[" Play.Example.org ", "play.example.org", "203.0.113.7"])).unwrap();
        assert_eq!(names.len(), 2);
        assert!(
            is_dns_name("a-b.example") && !is_dns_name("-a.example") && !is_dns_name("example.") && !is_dns_name("123")
        );
    }

    #[test]
    fn made_certificate_loads_and_describes_itself() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = (dir.path().join("c.pem"), dir.path().join("k.pem"));
        let (loaded, made) =
            load_or_generate(&cert, &key, &args(&["203.0.113.7", "play.example.org", "2001:db8::1"])).unwrap();
        assert!(made);
        assert_eq!(
            loaded.info.subject_alt_name,
            "IP Address:203.0.113.7, DNS:play.example.org, IP Address:2001:DB8:0:0:0:0:0:1"
        );
        assert_eq!(loaded.info.fingerprint.len(), 64);
        let year = 365 * 86_400_000_i64;
        assert!(loaded.info.not_after - loaded.info.not_before >= 10 * year);
        assert!(server_config(&loaded).is_ok());
        let (again, made) = load_or_generate(&cert, &key, &[]).unwrap();
        assert!(!made);
        assert_eq!(again.info, loaded.info);
        std::fs::remove_file(&key).unwrap();
        assert!(
            load_or_generate(&cert, &key, &[])
                .unwrap_err()
                .contains("refusing to overwrite")
        );
        // A key of another certificate
        let other = dir.path().join("o");
        let (_, _) = load_or_generate(&other.join("c.pem"), &other.join("k.pem"), &[]).unwrap();
        let error = load(&cert, &other.join("k.pem")).unwrap_err();
        assert!(
            error.ends_with(": the key does not belong to the certificate"),
            "{error}"
        );
        std::fs::write(&key, "x").unwrap();
        std::fs::write(&cert, "x").unwrap();
        assert!(load(&cert, &key).unwrap_err().ends_with(": no certificate in the file"));
    }
}
