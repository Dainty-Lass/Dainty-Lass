#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

//! The cluster CA and node certificates (DN3): what a CA issues, what a node accepts, and mutual TLS between
//! two nodes in memory.

use std::sync::Arc;

use network::cluster::{ClusterCa, NODE_ALPN, NodeCertRequest, NodeCredentials, peer_identity};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{TlsAcceptor, TlsConnector};

fn request(node: &str, region: &str, hosts: &[&str]) -> NodeCertRequest {
    NodeCertRequest {
        node: node.parse().unwrap(),
        region: region.parse().unwrap(),
        hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
        days: 30,
    }
}

fn credentials(ca: &ClusterCa, node: &str, region: &str, hosts: &[&str]) -> NodeCredentials {
    let issued = ca.issue(&request(node, region, hosts)).unwrap();
    NodeCredentials::from_pem(
        issued.cert_pem.as_bytes(),
        issued.key_pem.as_bytes(),
        issued.ca_cert_pem.as_bytes(),
    )
    .unwrap()
}

#[test]
fn a_ca_issues_a_certificate_that_says_who_the_node_is() {
    let ca = ClusterCa::create(365).unwrap();
    let hosts = ["127.0.0.1", "Play.Example.org", "127.0.0.1"];
    let issued = ca.issue(&request("oregon-1", "oregon", &hosts)).unwrap();
    assert_eq!(issued.info.identity.node.as_str(), "oregon-1");
    assert_eq!(issued.info.identity.region.as_str(), "oregon");
    assert_eq!(issued.info.hosts, ["127.0.0.1", "play.example.org"]);
    assert_eq!(issued.info.fingerprint.len(), 64);
    assert!(issued.info.not_after > issued.info.not_before + 29 * 86_400);
    // The node's files hold no CA key
    assert!(!issued.ca_cert_pem.contains("PRIVATE KEY") && !issued.cert_pem.contains("PRIVATE KEY"));
    assert!(issued.key_pem.contains("PRIVATE KEY"));
    let creds = credentials(&ca, "oregon-1", "oregon", &["127.0.0.1"]);
    assert_eq!(creds.identity().node.as_str(), "oregon-1");
}

#[test]
fn the_ca_file_round_trips_and_a_nodes_copy_cannot_issue() {
    let ca = ClusterCa::create(365).unwrap();
    let again = ClusterCa::parse(&ca.to_pem()).unwrap();
    assert_eq!(again.certificate_pem(), ca.certificate_pem());
    again.issue(&request("uk-1", "uk", &["uk.example.org"])).unwrap();
    let error = ClusterCa::parse(ca.certificate_pem()).unwrap_err().to_string();
    assert!(error.contains("no private key"), "{error}");
    // Another CA's key with this certificate
    let other = ClusterCa::create(365).unwrap().to_pem();
    let key = &other[other.find("-----BEGIN PRIVATE KEY-----").unwrap()..];
    let mixed = format!("{}{key}", ca.certificate_pem());
    assert!(
        ClusterCa::parse(&mixed)
            .unwrap_err()
            .to_string()
            .contains("does not belong")
    );
}

#[test]
fn bad_requests_are_refused() {
    let ca = ClusterCa::create(365).unwrap();
    assert!(ca.issue(&request("a", "b", &[])).is_err());
    assert!(ca.issue(&request("a", "b", &["not a host"])).is_err());
    assert!(ca.issue(&request("a", "b", &["*.example.org"])).is_err());
    let mut long = request("a", "b", &["127.0.0.1"]);
    long.days = 0;
    assert!(ca.issue(&long).is_err());
    long.days = 3651;
    assert!(ca.issue(&long).is_err());
}

#[test]
fn a_node_accepts_only_credentials_of_its_own_cluster() {
    let ca = ClusterCa::create(365).unwrap();
    let issued = ca.issue(&request("oregon-1", "oregon", &["127.0.0.1"])).unwrap();
    let other = ClusterCa::create(365).unwrap();
    let foreign = other.issue(&request("oregon-1", "oregon", &["127.0.0.1"])).unwrap();
    let load =
        |cert: &str, key: &str, ca: &str| NodeCredentials::from_pem(cert.as_bytes(), key.as_bytes(), ca.as_bytes());
    assert!(load(&issued.cert_pem, &issued.key_pem, &issued.ca_cert_pem).is_ok());
    // Chains to a different CA
    let error = load(&foreign.cert_pem, &foreign.key_pem, &issued.ca_cert_pem)
        .err()
        .unwrap();
    assert!(error.to_string().contains("does not chain"), "{error}");
    // Another node's key
    let error = load(&issued.cert_pem, &foreign.key_pem, &issued.ca_cert_pem)
        .err()
        .unwrap();
    assert!(error.to_string().contains("does not belong"), "{error}");
    // The CA's own file on a node
    let error = load(&issued.cert_pem, &issued.key_pem, &ca.to_pem()).err().unwrap();
    assert!(error.to_string().contains("CA's certificate only"), "{error}");
}

type Seen = (Result<String, String>, Result<String, String>);

/// A client talking to a server over memory: what each side saw of the other, or the error it stopped with.
async fn handshake(client: &NodeCredentials, server: &NodeCredentials, name: &str) -> Seen {
    handshake_with(client.client_config().unwrap(), server, name).await
}

async fn handshake_with(client: Arc<rustls::ClientConfig>, server: &NodeCredentials, name: &str) -> Seen {
    let (c, s) = tokio::io::duplex(16 * 1024);
    let acceptor = TlsAcceptor::from(server.server_config().unwrap());
    let connector = TlsConnector::from(client);
    let name = ServerName::try_from(name.to_owned()).unwrap();
    let serving = tokio::spawn(async move {
        let mut tls = acceptor.accept(s).await.map_err(|e| e.to_string())?;
        let (_, session) = tls.get_ref();
        let who = peer_identity(session.peer_certificates().unwrap()).map_err(|e| e.to_string())?;
        tls.write_all(b"hi").await.map_err(|e| e.to_string())?;
        tls.shutdown().await.ok();
        Ok(who.node.to_string())
    });
    let calling = async {
        let mut tls = connector.connect(name, c).await.map_err(|e| e.to_string())?;
        let (_, session) = tls.get_ref();
        assert_eq!(session.alpn_protocol(), Some(NODE_ALPN));
        let who = peer_identity(session.peer_certificates().unwrap()).map_err(|e| e.to_string())?;
        let mut hello = [0u8; 2];
        tls.read_exact(&mut hello).await.map_err(|e| e.to_string())?;
        Ok(who.node.to_string())
    };
    let calling = calling.await;
    (calling, serving.await.unwrap())
}

/// A client config that trusts `ca` and presents `issued`'s certificate, whoever signed it.
fn client_with(ca: &ClusterCa, issued: &network::cluster::IssuedNodeCert) -> Arc<rustls::ClientConfig> {
    let chain: Vec<_> = CertificateDer::pem_slice_iter(issued.cert_pem.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(issued.key_pem.as_bytes()).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(ca.certificate_pem().as_bytes()).unwrap())
        .unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots);
    let mut config = builder.with_client_auth_cert(chain, key).unwrap();
    config.alpn_protocols = vec![NODE_ALPN.to_vec()];
    Arc::new(config)
}

#[tokio::test]
async fn two_nodes_of_one_cluster_authenticate_each_other() {
    let ca = ClusterCa::create(365).unwrap();
    let oregon = credentials(&ca, "oregon-1", "oregon", &["127.0.0.1"]);
    let uk = credentials(&ca, "uk-1", "uk", &["uk.example.org"]);
    let (calling, serving) = handshake(&oregon, &uk, "uk.example.org").await;
    assert_eq!(calling, Ok("uk-1".to_owned()));
    assert_eq!(serving, Ok("oregon-1".to_owned()));
}

#[tokio::test]
async fn a_node_with_another_clusters_certificate_is_refused_both_ways() {
    let ca = ClusterCa::create(365).unwrap();
    let uk = credentials(&ca, "uk-1", "uk", &["127.0.0.1"]);
    // A foreign client that trusts this cluster's CA: the server refuses its certificate
    let other = ClusterCa::create(365).unwrap();
    let intruder = other.issue(&request("oregon-1", "oregon", &["127.0.0.1"])).unwrap();
    let (calling, serving) = handshake_with(client_with(&ca, &intruder), &uk, "127.0.0.1").await;
    let served = serving.unwrap_err();
    assert!(served.to_lowercase().contains("certificate"), "{served}");
    assert!(calling.is_err(), "{calling:?}");
    // A foreign server: the client refuses its certificate
    let foreign = credentials(&other, "oregon-1", "oregon", &["127.0.0.1"]);
    let (calling, _) = handshake(&uk, &foreign, "127.0.0.1").await;
    assert!(calling.unwrap_err().to_lowercase().contains("certificate"));
}

#[tokio::test]
async fn the_server_must_be_at_the_address_its_certificate_names() {
    let ca = ClusterCa::create(365).unwrap();
    let oregon = credentials(&ca, "oregon-1", "oregon", &["127.0.0.1"]);
    let uk = credentials(&ca, "uk-1", "uk", &["uk.example.org"]);
    let (calling, _) = handshake(&oregon, &uk, "elsewhere.example.org").await;
    assert!(calling.is_err(), "{calling:?}");
}

#[tokio::test]
async fn a_client_without_a_certificate_is_refused() {
    let ca = ClusterCa::create(365).unwrap();
    let uk = credentials(&ca, "uk-1", "uk", &["127.0.0.1"]);
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(ca.certificate_pem().as_bytes()).unwrap())
        .unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![NODE_ALPN.to_vec()];
    let (c, s) = tokio::io::duplex(16 * 1024);
    let acceptor = TlsAcceptor::from(uk.server_config().unwrap());
    let serving = tokio::spawn(async move { acceptor.accept(s).await.map(|_| ()).map_err(|e| e.to_string()) });
    let name = ServerName::try_from("127.0.0.1".to_owned()).unwrap();
    let mut client = TlsConnector::from(Arc::new(config)).connect(name, c).await;
    if let Ok(tls) = &mut client {
        let mut byte = [0u8; 1];
        let _ = tls.read(&mut byte).await;
    }
    assert!(
        serving.await.unwrap().is_err(),
        "a client with no certificate got through"
    );
}

#[test]
fn hosts_are_ip_addresses_and_lower_case_dns_names() {
    for good in [
        "127.0.0.1",
        "::1",
        "2001:db8::1",
        "play.example.org",
        "uk-1.example.org",
        "localhost",
    ] {
        assert!(network::cluster::is_host(good), "{good}");
    }
    for bad in [
        "",
        "*.example.org",
        "example.org.",
        "-a.example",
        "a-.example",
        "1234",
        "a_b.example",
        "UK.example.org",
        "a..example",
        "not a host",
        "fe80::1%eth0",
        "http://a.example",
    ] {
        assert!(!network::cluster::is_host(bad), "{bad:?}");
    }
    assert!(!network::cluster::is_dns_name(&format!("{}.example", "a".repeat(64))));
}
