//! Node-link requests and certificates from another node (docs/dainty-lass-plan.md DN3, DX2;
//! docs/security/network.md): the route table, the time header, a ping's answer, an error answer, a peer as
//! `[[network.nodes]]` lists it, a certificate's names and the endpoint's admission of a request head are read
//! whole or refused, never panic, and hold only what their rules accept.
#![no_main]

use libfuzzer_sys::fuzz_target;
use dainty_module_api::NodeIdentity;
use dainty_network::cluster::{self, Peer};
use dainty_network::endpoint::{Head, NodeEndpoint};
use dainty_network::link::MAX_MESSAGE;
use dainty_network::wire::{self, ErrorBody, PingAnswer, Target};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);

    if let Some(target) = wire::parse_target(&text) {
        assert_eq!(target.path(), text);
    }
    if let Some(time) = wire::parse_time(&text) {
        assert!(time >= 0 && text.len() <= 16);
    }

    if let Ok(answer) = PingAnswer::decode(data) {
        assert_eq!(answer.version, wire::VERSION);
        assert_eq!(PingAnswer::decode(&answer.encode()), Ok(answer));
    }
    if let Some(body) = ErrorBody::decode(data) {
        assert_eq!(ErrorBody::decode(&body.encode()), Some(body));
    }
    let node = "uk-1".parse().expect("a node id");
    for status in [400, 403, 404, 409, 413, 502, 503] {
        let _ = wire::link_error(&node, Some(dainty_network::Service::Directory), status, data);
    }

    if let Ok(peer) = serde_json::from_slice::<Peer>(data) {
        if peer.check().is_ok() {
            assert!(cluster::is_host(&peer.host) && peer.port != 0);
            let own = "oregon-1".parse().expect("a node id");
            let back: Peer = serde_json::from_slice(&serde_json::to_vec(&peer).expect("writes")).expect("reads back");
            assert_eq!(back, peer);
            let _ = cluster::check_peers(&own, &[peer]);
        }
    }

    if let Ok(info) = cluster::describe(data) {
        assert_eq!(info.fingerprint.len(), 64);
        let der = rustls::pki_types::CertificateDer::from(data.to_vec());
        let said: NodeIdentity = cluster::peer_identity(&[der]).expect("describe and peer_identity agree");
        assert_eq!(said, info.identity);
    }

    // A request head from a peer: whatever the endpoint admits is a route of the node link with the right
    // method, a time within the bound and a body that isn't declared too large
    let mut parts = text.split('\0');
    let (method, target, time) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    let length = parts.next().and_then(|n| n.parse::<u64>().ok());
    let me = NodeIdentity::standalone();
    let peer: dainty_module_api::network::NodeId = "uk-1".parse().expect("a node id");
    let endpoint = NodeEndpoint::new(me, [peer.clone()]);
    let head = Head {
        method,
        target,
        content_length: length,
        time: Some(time),
    };
    if let Ok(admitted) = endpoint.admit(&peer, &head) {
        assert_eq!(wire::parse_target(target), Some(admitted));
        match admitted {
            Target::Ping => assert_eq!(method, "GET"),
            Target::Service(_) => assert_eq!(method, "POST"),
        }
        assert!(length.is_none_or(|n| n <= MAX_MESSAGE as u64));
        let theirs = wire::parse_time(time).expect("a time");
        assert!(wire::within(
            wire::skew_ms(endpoint.clock().now_ms(), theirs),
            dainty_network::link::MAX_CLOCK_SKEW + std::time::Duration::from_secs(5)
        ));
    }
    let stranger: dainty_module_api::network::NodeId = "de-1".parse().expect("a node id");
    assert!(endpoint.admit(&stranger, &head).is_err());
});
