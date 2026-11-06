//! Leader election's node-link messages (docs/distributed-mode.md, docs/security/network.md): a call or answer
//! from another node is read whole or refused, never panics, and writes back as the same message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use network::election::{ElectionRequest, ElectionResponse, HardState};

fuzz_target!(|data: &[u8]| {
    if let Ok(request) = serde_json::from_slice::<ElectionRequest>(data) {
        let again: ElectionRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).expect("writes back")).expect("reads back");
        assert_eq!(again, request);
    }
    if let Ok(response) = serde_json::from_slice::<ElectionResponse>(data) {
        let again: ElectionResponse =
            serde_json::from_slice(&serde_json::to_vec(&response).expect("writes back")).expect("reads back");
        assert_eq!(again, response);
    }
    // The term file a node reads at start
    if let Ok(state) = serde_json::from_slice::<HardState>(data) {
        let again: HardState =
            serde_json::from_slice(&serde_json::to_vec(&state).expect("writes back")).expect("reads back");
        assert_eq!(again, state);
    }
});
