//! The node list the launcher reads from the joined server (docs/dainty-lass-plan.md DN7, RX3;
//! docs/security/region-ping.md): each entry's host, port and fingerprint decide where the launcher
//! connects. The launcher's own parser file, compiled in here, since the launcher crate needs a
//! webview to build.
#![no_main]

use libfuzzer_sys::fuzz_target;
use serde_json::Value;

#[path = "../../launcher/src-tauri/src/net/regionping/parse.rs"]
mod parse;

const STATUSES: [u16; 5] = [200, 204, 404, 429, 500];

fuzz_target!(|data: &[u8]| {
    let Ok(value) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    for status in STATUSES {
        let parse::NodeList::Nodes(nodes) = parse::read_nodes(status, Some(&value)) else {
            continue;
        };
        assert!(status == 200 || (status == 404 && nodes.is_empty()));
        assert!(nodes.len() <= parse::MAX_NODES);
        for (at, node) in nodes.iter().enumerate() {
            // Only a pinned TLS target the launcher can name is ever connected to
            assert!(parse::is_region_id(&node.region));
            assert!(protocol::invite::is_valid_host(&node.host) && node.port >= 1);
            assert!(protocol::invite::is_valid_fingerprint(&node.fingerprint));
            assert!(nodes[..at].iter().all(|seen| seen.region != node.region));
        }
    }
});
