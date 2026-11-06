//! The split server's tunnel messages (docs/split-roles.md, docs/security/tunnel.md): a tunnelled request or answer
//! from the other half is read whole or refused, never panics, holds nothing that could split a request line or
//! smuggle a hop-by-hop header, stays inside its body limit, and round-trips.
#![no_main]

use libfuzzer_sys::fuzz_target;
use network::tunnel::{MAX_BODY, Port, TunnelRequest, TunnelResponse, carries};

fuzz_target!(|data: &[u8]| {
    if let Ok(request) = TunnelRequest::decode(data) {
        assert!(request.body.len() <= MAX_BODY);
        assert!(request.target.starts_with('/'));
        assert!(request.target.bytes().all(|b| (0x21..=0x7e).contains(&b)));
        assert!(request.headers.iter().all(|(n, v)| !n.contains(['\r', '\n']) && !v.contains(['\r', '\n'])));
        if request.port == Port::Director && carries(Port::Director, &request.method, &request.target) {
            assert!(request.method == "GET" || request.method == "POST");
        }
        let written = request.encode().expect("writes back");
        assert_eq!(TunnelRequest::decode(&written).expect("reads back"), request);
    }
    if let Ok(response) = TunnelResponse::decode(data) {
        assert!(response.body.len() <= MAX_BODY);
        assert!((100..=599).contains(&response.status));
        let written = response.encode().expect("writes back");
        assert_eq!(TunnelResponse::decode(&written).expect("reads back"), response);
    }
});
