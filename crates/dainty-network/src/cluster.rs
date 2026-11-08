//! The cluster CA and the node certificates (DD16, DN3).
//!
//! The operator makes one CA, `network ca-init`, and keeps it: the CA file (certificate and key) is only
//! ever given to `network issue-node-cert` on the owner's own machine and is never stored on a node. Each
//! node gets its own certificate and key and the CA's certificate, nothing else. A certificate says who
//! its node is in two URI names, `urn:dainty-lass:node:<id>` and `urn:dainty-lass:region:<region>`, beside
//! the host names it answers on. Another node accepts it as a peer when it chains to the cluster CA, is
//! valid now, and names a node in its own `[[network.nodes]]` (a CA-signed certificate alone is not
//! enough: removing a node from the list is how a lost or retired node is cut off).
//!
//! Certificates are ECDSA P-256, made with rcgen on aws-lc-rs as the gateway's are (no OpenSSL). Mutual
//! TLS is TLS 1.3 only, with its own ALPN name ([`NODE_ALPN`]) so a gateway tells a node from a player
//! before it chooses which certificate to show.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use dainty_module_api::NodeIdentity;
use dainty_module_api::network::{NodeId, Region};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, SanType, SerialNumber,
};
use rustls::RootCertStore;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The ALPN name of a node link. A connection that doesn't offer it is a player's.
pub const NODE_ALPN: &[u8] = b"dainty-lass-node/1";
const NODE_URI: &str = "urn:dainty-lass:node:";
const REGION_URI: &str = "urn:dainty-lass:region:";
const CA_NAME: &str = "Dainty Lass cluster CA";
/// How long the cluster CA's certificate is valid, in days.
pub const CA_DAYS: u32 = 3650;
/// A node certificate's validity when none is asked for, in days.
pub const DEFAULT_NODE_DAYS: u32 = 365;
/// The longest validity a node certificate may be issued for, in days.
pub const MAX_NODE_DAYS: u32 = 3650;
/// Peers one node lists; the launcher reads at most 16 nodes of a network, this node included (DN7)
pub const MAX_PEERS: usize = 15;

/// Why a certificate, key or peer was refused; the message names the file or field for the host.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CertError(pub String);

fn fail(error: impl std::fmt::Display) -> CertError {
    CertError(error.to_string())
}

type Result<T> = std::result::Result<T, CertError>;

// ---- peers ----

/// A peer as `[[network.nodes]]` lists it: where its gateway listens, and which node answers there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    /// The node that answers there.
    pub id: NodeId,
    /// The node's region.
    pub region: Region,
    /// The gateway's IP address or lower-case DNS name.
    pub host: String,
    /// The gateway's port.
    pub port: u16,
    /// SHA-256 of the peer's public gateway certificate, 64 lowercase hex: what the launcher pins when it
    /// pings that region (DN7). A peer without one is a peer of this node, but not listed to players.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// A DNS name as a certificate may carry it: letters, digits and hyphens in dot-separated labels, at
/// least one letter, no wildcard, no trailing dot.
pub fn is_dns_name(name: &str) -> bool {
    name.len() <= 253
        && name.bytes().any(|b| b.is_ascii_alphabetic())
        && !name.ends_with('.')
        && !name.contains(['_', '*'])
        && rustls::pki_types::DnsName::try_from(name).is_ok()
}

/// An IP address or a DNS name, in lower case, which is how a host is written to a certificate.
pub fn is_host(text: &str) -> bool {
    text.parse::<IpAddr>().is_ok() || (text == text.to_ascii_lowercase() && is_dns_name(text))
}

impl Peer {
    /// Checks the host, the fingerprint's form and the names; errors name the node.
    pub fn check(&self) -> Result<()> {
        let id = &self.id;
        if !is_host(&self.host) {
            return Err(fail(format!(
                "node {id}: host must be an IP address or a lower-case DNS name, not {:?}",
                self.host
            )));
        }
        if self.port == 0 {
            return Err(fail(format!("node {id}: port must be from 1 to 65535")));
        }
        if let Some(fingerprint) = &self.fingerprint
            && !(fingerprint.len() == 64
                && fingerprint
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        {
            return Err(fail(format!(
                "node {id}: fingerprint must be 64 lowercase hex characters (`gateway make-cert --fingerprint <cert>` shows it)"
            )));
        }
        Ok(())
    }
}

/// `peers` as a node `own` may list them: each valid, each id once, none of them itself, at most
/// [`MAX_PEERS`].
pub fn check_peers(own: &NodeId, peers: &[Peer]) -> Result<()> {
    if peers.len() > MAX_PEERS {
        return Err(fail(format!("a node lists at most {MAX_PEERS} peers")));
    }
    for (index, peer) in peers.iter().enumerate() {
        peer.check()?;
        if &peer.id == own {
            return Err(fail(format!("node {own} lists itself under [[network.nodes]]")));
        }
        if peers[..index].iter().any(|earlier| earlier.id == peer.id) {
            return Err(fail(format!(
                "node {} is listed twice under [[network.nodes]]",
                peer.id
            )));
        }
    }
    Ok(())
}

// ---- the CA ----

fn serial() -> Result<SerialNumber> {
    // 20 random octets with the top bit clear: a positive number of the longest length X.509 allows
    let mut bytes = [0u8; 20];
    rustls::crypto::aws_lc_rs::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| fail("no random numbers"))?;
    bytes[0] = (bytes[0] & 0x3f) | 0x40;
    Ok(SerialNumber::from_slice(&bytes))
}

/// A day of slack before now for clocks that run behind, then `days`.
fn validity(days: u32) -> (time::OffsetDateTime, time::OffsetDateTime) {
    let now = time::OffsetDateTime::now_utc();
    (
        now - time::Duration::days(1),
        now + time::Duration::days(i64::from(days)),
    )
}

fn name(common: &str, organization: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common);
    dn.push(DnType::OrganizationName, organization);
    dn
}

/// PEM with LF line endings, whatever the encoder wrote.
fn lf(pem: impl AsRef<str>) -> String {
    pem.as_ref().replace("\r\n", "\n")
}

/// The block of `text` between `-----BEGIN <label>-----` and its END line, lines included.
fn pem_block<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let from = text.find(&begin)?;
    let to = from + text[from..].find(&end)? + end.len();
    Some(&text[from..to])
}

/// The cluster CA: its certificate and the key that signs node certificates. The operator's file, kept
/// off every node.
pub struct ClusterCa {
    cert_pem: String,
    key_pem: String,
}

impl std::fmt::Debug for ClusterCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterCa").finish_non_exhaustive()
    }
}

impl ClusterCa {
    /// A new CA, valid for `days`; it may sign node certificates and no other CA.
    pub fn create(days: u32) -> Result<Self> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(fail)?;
        let mut params = CertificateParams::default();
        params.serial_number = Some(serial()?);
        (params.not_before, params.not_after) = validity(days);
        params.distinguished_name = name(CA_NAME, "Dainty Lass cluster");
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params.self_signed(&key).map_err(fail)?;
        Ok(Self {
            cert_pem: lf(cert.pem()),
            key_pem: lf(key.serialize_pem()),
        })
    }

    /// The CA file: the certificate and its key, as [`to_pem`](Self::to_pem) writes them.
    pub fn parse(pem: &str) -> Result<Self> {
        let pem = &lf(pem);
        let cert_pem = pem_block(pem, "CERTIFICATE").ok_or_else(|| fail("the CA file holds no certificate"))?;
        let key_pem = pem_block(pem, "PRIVATE KEY").ok_or_else(|| {
            fail("the CA file holds no private key (a node's copy of the CA certificate can't issue)")
        })?;
        let der = CertificateDer::from_pem_slice(cert_pem.as_bytes()).map_err(fail)?;
        let (_, x509) =
            x509_parser::parse_x509_certificate(&der).map_err(|e| fail(format!("not a certificate: {e}")))?;
        if !x509.is_ca() {
            return Err(fail("the CA file's certificate is not a CA"));
        }
        let key = KeyPair::from_pem(key_pem).map_err(fail)?;
        if key.public_key_raw() != &*x509.public_key().subject_public_key.data {
            return Err(fail("the CA file's key does not belong to its certificate"));
        }
        if !x509.validity().is_valid() {
            return Err(fail(
                "the CA certificate is not valid now: make a new CA and reissue every node certificate",
            ));
        }
        Ok(Self {
            cert_pem: cert_pem.to_owned() + "\n",
            key_pem: key_pem.to_owned() + "\n",
        })
    }

    /// Reads and parses the CA file; errors name the file.
    pub fn read(file: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(file).map_err(|e| fail(format!("{}: {e}", file.display())))?;
        Self::parse(&text).map_err(|e| fail(format!("{}: {e}", file.display())))
    }

    /// The CA file's text: certificate, then key. Secret.
    pub fn to_pem(&self) -> String {
        format!("{}{}", self.cert_pem, self.key_pem)
    }

    /// The CA certificate alone: what every node holds.
    pub fn certificate_pem(&self) -> &str {
        &self.cert_pem
    }

    /// A certificate and key for the node `request` names, signed by this CA.
    pub fn issue(&self, request: &NodeCertRequest) -> Result<IssuedNodeCert> {
        if !(1..=MAX_NODE_DAYS).contains(&request.days) {
            return Err(fail(format!(
                "a node certificate is valid for 1 to {MAX_NODE_DAYS} days"
            )));
        }
        let mut names = host_names(&request.hosts)?;
        for (prefix, text) in [(NODE_URI, request.node.as_str()), (REGION_URI, request.region.as_str())] {
            let uri = format!("{prefix}{text}").try_into().map_err(fail)?;
            names.push(SanType::URI(uri));
        }
        let issuer =
            Issuer::from_ca_cert_pem(&self.cert_pem, KeyPair::from_pem(&self.key_pem).map_err(fail)?).map_err(fail)?;
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(fail)?;
        let mut params = CertificateParams::default();
        params.serial_number = Some(serial()?);
        (params.not_before, params.not_after) = validity(request.days);
        params.distinguished_name = name(request.node.as_str(), "Dainty Lass node");
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        // The same certificate answers a peer's calls and makes this node's own
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth, ExtendedKeyUsagePurpose::ClientAuth];
        params.subject_alt_names = names;
        let cert = params.signed_by(&key, &issuer).map_err(fail)?;
        let info = describe(cert.der())?;
        Ok(IssuedNodeCert {
            cert_pem: lf(cert.pem()),
            key_pem: lf(key.serialize_pem()),
            ca_cert_pem: self.cert_pem.clone(),
            info,
        })
    }
}

/// What a node certificate is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCertRequest {
    /// The node the certificate names.
    pub node: NodeId,
    /// The region it names.
    pub region: Region,
    /// The IP addresses and DNS names the node's gateway answers on, as `[[network.nodes]]` lists them
    pub hosts: Vec<String>,
    /// Validity in days, up to [`MAX_NODE_DAYS`].
    pub days: u32,
}

/// A node's files: the certificate, its key (secret) and the CA certificate.
pub struct IssuedNodeCert {
    /// The node certificate.
    pub cert_pem: String,
    /// Its private key. Secret.
    pub key_pem: String,
    /// The cluster CA's certificate, without its key.
    pub ca_cert_pem: String,
    /// What the issued certificate says.
    pub info: NodeCertInfo,
}

/// SubjectAltName entries for hosts: IP addresses and DNS names, each once.
fn host_names(hosts: &[String]) -> Result<Vec<SanType>> {
    let mut wanted: Vec<String> = Vec::new();
    for host in hosts.iter().map(|h| h.trim().to_lowercase()) {
        if !wanted.contains(&host) {
            wanted.push(host);
        }
    }
    if wanted.is_empty() {
        return Err(fail(
            "give at least one --host (the node's public IP address or DNS name)",
        ));
    }
    wanted
        .into_iter()
        .map(|host| match host.parse::<IpAddr>() {
            Ok(ip) => Ok(SanType::IpAddress(ip)),
            Err(_) if is_dns_name(&host) => host.try_into().map(SanType::DnsName).map_err(fail),
            Err(_) => Err(fail(format!("{host:?} is neither an IP address nor a DNS name"))),
        })
        .collect()
}

// ---- reading a node certificate ----

/// What a node certificate says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeCertInfo {
    /// The node and region it names.
    pub identity: NodeIdentity,
    /// The IP addresses and DNS names it answers on
    pub hosts: Vec<String>,
    /// Hex
    pub serial: String,
    /// SHA-256 of the DER, 64 lowercase hex
    pub fingerprint: String,
    /// Unix seconds
    pub not_before: i64,
    /// Unix seconds.
    pub not_after: i64,
}

/// Reads a node certificate: exactly one node name and one region name, a leaf and not a CA. Nothing is
/// trusted from it until a verifier has chained it to the cluster CA.
pub fn describe(der: &[u8]) -> Result<NodeCertInfo> {
    use x509_parser::extensions::GeneralName;
    let (rest, cert) = x509_parser::parse_x509_certificate(der).map_err(|e| fail(format!("not a certificate: {e}")))?;
    if !rest.is_empty() {
        return Err(fail("a certificate followed by more bytes"));
    }
    if cert.is_ca() {
        return Err(fail("a CA certificate is not a node's"));
    }
    let san = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .ok_or_else(|| fail("the certificate names no node"))?;
    let (mut node, mut region, mut hosts) = (None, None, Vec::new());
    for general in &san.value.general_names {
        match general {
            GeneralName::URI(uri) => {
                if let Some(id) = uri.strip_prefix(NODE_URI) {
                    let id = NodeId::new(id).map_err(fail)?;
                    if node.replace(id).is_some() {
                        return Err(fail("the certificate names two nodes"));
                    }
                } else if let Some(text) = uri.strip_prefix(REGION_URI) {
                    let text = Region::new(text).map_err(fail)?;
                    if region.replace(text).is_some() {
                        return Err(fail("the certificate names two regions"));
                    }
                }
            }
            GeneralName::DNSName(dns) => hosts.push((*dns).to_owned()),
            GeneralName::IPAddress(bytes) => {
                if let Ok(v4) = <[u8; 4]>::try_from(*bytes) {
                    hosts.push(IpAddr::from(v4).to_string());
                } else if let Ok(v6) = <[u8; 16]>::try_from(*bytes) {
                    hosts.push(IpAddr::from(v6).to_string());
                }
            }
            _ => {}
        }
    }
    let (Some(node), Some(region)) = (node, region) else {
        return Err(fail("the certificate does not name both a node and its region"));
    };
    Ok(NodeCertInfo {
        identity: NodeIdentity { node, region },
        hosts,
        serial: hex::encode(cert.tbs_certificate.raw_serial()),
        fingerprint: hex::encode(Sha256::digest(der)),
        not_before: cert.validity().not_before.timestamp(),
        not_after: cert.validity().not_after.timestamp(),
    })
}

/// Who the first certificate of a peer's chain says it is. Only after the TLS handshake verified the
/// chain to the cluster CA does the name mean anything.
pub fn peer_identity(chain: &[CertificateDer<'_>]) -> Result<NodeIdentity> {
    let leaf = chain.first().ok_or_else(|| fail("the peer presented no certificate"))?;
    describe(leaf).map(|info| info.identity)
}

// ---- a node's own credentials ----

/// A node's certificate chain, key and the cluster CA's certificate, checked together.
pub struct NodeCredentials {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    ca: CertificateDer<'static>,
    info: NodeCertInfo,
}

impl std::fmt::Debug for NodeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeCredentials")
            .field("identity", &self.info.identity)
            .finish_non_exhaustive()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

impl NodeCredentials {
    /// Reads the three PEM files and checks them as [`NodeCredentials::from_pem`] does.
    pub fn load(cert_file: &Path, key_file: &Path, ca_file: &Path) -> Result<Self> {
        let read = |file: &Path| std::fs::read(file).map_err(|e| fail(format!("{}: {e}", file.display())));
        Self::from_pem(&read(cert_file)?, &read(key_file)?, &read(ca_file)?)
    }

    /// Refuses a CA file that holds a key (a node never holds the CA's), a key that doesn't belong to the
    /// certificate, and a certificate that doesn't chain to the CA or isn't valid now.
    pub fn from_pem(cert_pem: &[u8], key_pem: &[u8], ca_pem: &[u8]) -> Result<Self> {
        if String::from_utf8_lossy(ca_pem).contains("PRIVATE KEY") {
            return Err(fail(
                "the CA file holds a private key: a node keeps the CA's certificate only. Never copy the CA's key to a node",
            ));
        }
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| fail(format!("node certificate: {e}")))?;
        let ca = CertificateDer::from_pem_slice(ca_pem).map_err(|e| fail(format!("CA certificate: {e}")))?;
        let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| fail(format!("node key: {e}")))?;
        let Some(leaf) = chain.first() else {
            return Err(fail("no certificate in the node certificate file"));
        };
        let signing = provider()
            .key_provider
            .load_private_key(key.clone_key())
            .map_err(fail)?;
        rustls::sign::CertifiedKey::new(chain.clone(), signing)
            .keys_match()
            .map_err(|_| fail("the node key does not belong to the node certificate"))?;
        let info = describe(leaf)?;
        let credentials = Self { chain, key, ca, info };
        credentials
            .client_verifier()?
            .verify_client_cert(&credentials.chain[0], &credentials.chain[1..], UnixTime::now())
            .map_err(|e| {
                fail(format!(
                    "the node certificate does not chain to the cluster CA or is not valid now: {e}"
                ))
            })?;
        Ok(credentials)
    }

    /// The node and region the certificate names.
    pub fn identity(&self) -> &NodeIdentity {
        &self.info.identity
    }

    /// What the certificate says.
    pub fn info(&self) -> &NodeCertInfo {
        &self.info
    }

    fn roots(&self) -> Result<Arc<RootCertStore>> {
        let mut roots = RootCertStore::empty();
        roots
            .add(self.ca.clone())
            .map_err(|e| fail(format!("CA certificate: {e}")))?;
        Ok(Arc::new(roots))
    }

    fn client_verifier(&self) -> Result<Arc<dyn rustls::server::danger::ClientCertVerifier>> {
        rustls::server::WebPkiClientVerifier::builder_with_provider(self.roots()?, provider())
            .build()
            .map_err(fail)
    }

    /// The answering side: TLS 1.3, this node's certificate, and a client certificate that must chain to
    /// the cluster CA. Offered only to a client that asks for [`NODE_ALPN`].
    pub fn server_config(&self) -> Result<Arc<rustls::ServerConfig>> {
        let mut config = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(fail)?
            .with_client_cert_verifier(self.client_verifier()?)
            .with_single_cert(self.chain.clone(), self.key.clone_key())
            .map_err(fail)?;
        config.alpn_protocols = vec![NODE_ALPN.to_vec()];
        Ok(Arc::new(config))
    }

    /// The calling side: TLS 1.3, this node's certificate for the peer to check, and the peer's
    /// certificate checked against the cluster CA and the host it was called at.
    pub fn client_config(&self) -> Result<Arc<rustls::ClientConfig>> {
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(self.roots()?, provider())
            .build()
            .map_err(fail)?;
        let mut config = rustls::ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(fail)?
            .with_webpki_verifier(verifier)
            .with_client_auth_cert(self.chain.clone(), self.key.clone_key())
            .map_err(fail)?;
        config.alpn_protocols = vec![NODE_ALPN.to_vec()];
        Ok(Arc::new(config))
    }
}
