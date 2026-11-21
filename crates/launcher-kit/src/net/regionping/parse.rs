// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The home's node list (DN7), without the network: the fuzz target `launcher_region` compiles this
//! file in. Each node's host, port and fingerprint decide where the launcher connects, so an entry
//! with anything unexpected is dropped. The shapes match the gateway's `network.rs` until DN2's
//! `crates/module-api` defines them once.

use protocol::invite::{is_valid_fingerprint, is_valid_host};
use serde_json::Value;

/// The gateway lists no more; anything past this many valid entries is dropped
pub const MAX_NODES: usize = 16;

/// One node of the joined server's network: its region and the gateway the launcher pings, pinned
/// to `fingerprint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeEntry {
    pub region: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeList {
    /// Empty when the server is in no network
    Nodes(Vec<NodeEntry>),
    BadAnswer,
}

/// A region id: lower-case letters, digits and inner hyphens, 1 to 32 long (`oregon`, `new-york`).
pub fn is_region_id(text: &str) -> bool {
    (1..=32).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !text.starts_with('-')
        && !text.ends_with('-')
}

fn read_node(value: &Value) -> Option<NodeEntry> {
    let node = value.as_object()?;
    let region = node.get("region")?.as_str().filter(|id| is_region_id(id))?;
    let host = node.get("host")?.as_str().filter(|host| is_valid_host(host))?;
    let port = node
        .get("port")?
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port >= 1)?;
    let fingerprint = node
        .get("fingerprint")?
        .as_str()
        .filter(|fp| is_valid_fingerprint(fp))?;
    Some(NodeEntry {
        region: region.to_owned(),
        host: host.to_owned(),
        port,
        fingerprint: fingerprint.to_owned(),
    })
}

/// The answer to `GET /network/v1/nodes`. A 404 is a server without the route (older, or 1.4.4), which
/// is in no network. Each region is kept once, the first time it is listed.
pub fn read_nodes(status: u16, body: Option<&Value>) -> NodeList {
    match status {
        404 => NodeList::Nodes(Vec::new()),
        200 => {
            let Some(list) = body.and_then(|body| body.get("nodes")).and_then(Value::as_array) else {
                return NodeList::BadAnswer;
            };
            let mut nodes: Vec<NodeEntry> = Vec::new();
            for node in list.iter().filter_map(read_node) {
                if nodes.len() == MAX_NODES {
                    break;
                }
                if !nodes.iter().any(|seen| seen.region == node.region) {
                    nodes.push(node);
                }
            }
            NodeList::Nodes(nodes)
        }
        _ => NodeList::BadAnswer,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fp(byte: &str) -> String {
        byte.repeat(32)
    }

    #[test]
    fn region_ids_are_the_gateways() {
        for good in ["oregon", "new-york", "uk", "de-2"] {
            assert!(is_region_id(good), "{good}");
        }
        for bad in ["", "-uk", "uk-", "New-York", "new york", "uk_2", &"a".repeat(33)] {
            assert!(!is_region_id(bad), "{bad}");
        }
    }

    #[test]
    fn no_route_or_no_nodes_is_no_network() {
        assert_eq!(read_nodes(404, None), NodeList::Nodes(Vec::new()));
        assert_eq!(
            read_nodes(200, Some(&json!({ "nodes": [] }))),
            NodeList::Nodes(Vec::new())
        );
        assert_eq!(read_nodes(200, Some(&json!({ "nodes": {} }))), NodeList::BadAnswer);
        assert_eq!(read_nodes(200, None), NodeList::BadAnswer);
        assert_eq!(read_nodes(500, Some(&json!({ "nodes": [] }))), NodeList::BadAnswer);
    }

    #[test]
    fn odd_entries_are_dropped_and_each_region_kept_once() {
        let body = json!({ "nodes": [
            { "region": "oregon", "host": "or.example.org", "port": 443, "fingerprint": fp("ab") },
            { "region": "oregon", "host": "203.0.113.9", "port": 443, "fingerprint": fp("cd") },
            { "region": "uk", "host": "203.0.113.7", "port": 8443, "fingerprint": fp("ef") },
            { "region": "UK", "host": "203.0.113.7", "port": 8443, "fingerprint": fp("ef") },
            { "region": "de", "host": "1.2.3", "port": 443, "fingerprint": fp("ab") },
            { "region": "de", "host": "de.example.org", "port": 0, "fingerprint": fp("ab") },
            { "region": "de", "host": "de.example.org", "port": 65536, "fingerprint": fp("ab") },
            { "region": "de", "host": "de.example.org", "port": 443, "fingerprint": fp("AB") },
            { "region": "de", "host": "de.example.org", "port": "443", "fingerprint": fp("ab") },
            "de.example.org",
        ] });
        assert_eq!(
            read_nodes(200, Some(&body)),
            NodeList::Nodes(vec![
                NodeEntry {
                    region: "oregon".into(),
                    host: "or.example.org".into(),
                    port: 443,
                    fingerprint: fp("ab"),
                },
                NodeEntry {
                    region: "uk".into(),
                    host: "203.0.113.7".into(),
                    port: 8443,
                    fingerprint: fp("ef"),
                },
            ])
        );
    }

    #[test]
    fn at_most_sixteen_nodes() {
        let nodes: Vec<Value> = (0..40)
            .map(|n| json!({ "region": format!("r{n}"), "host": "203.0.113.1", "port": 443, "fingerprint": fp("ab") }))
            .collect();
        let NodeList::Nodes(read) = read_nodes(200, Some(&json!({ "nodes": nodes }))) else {
            panic!("not read");
        };
        assert_eq!(read.len(), MAX_NODES);
        assert_eq!(read[15].region, "r15");
    }
}
