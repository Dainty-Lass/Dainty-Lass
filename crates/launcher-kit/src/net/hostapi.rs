// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The launcher API's routes, and where the account key may go (AGENTS.md: only to its own server).

use crate::net::http::{Endpoint, check_endpoint};

/// Header that carries the account key (`RRK_...`) to the server it was made for.
pub const KEY_HEADER: &str = "x-reforged-user-api-key";

/// The routes and key header of one server family's launcher API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flavor {
    /// Family name.
    pub id: &'static str,
    /// Header the account key goes in.
    pub key_header: &'static str,
    /// Server status route.
    pub status_path: &'static str,
    /// Account registration route.
    pub register_path: &'static str,
    /// Account lookup route.
    pub account_path: &'static str,
    /// Content's game version list route.
    pub versions_path: &'static str,
    /// Runtime manifest route.
    pub runtime_manifest_path: &'static str,
    /// Network node list route, for region ping.
    pub region_ping_path: &'static str,
}

/// DaintyLass's own routes.
pub const REFORGED: Flavor = Flavor {
    id: "reforged",
    key_header: KEY_HEADER,
    status_path: "/api/launcher/status",
    register_path: "/api/launcher/register",
    account_path: "/api/launcher/account",
    versions_path: "/content/v1/versions",
    runtime_manifest_path: "/api/launcher/runtime-files",
    region_ping_path: "/network/v1/nodes",
};

/// The account key header for `endpoint`, or none when there is no usable key or the endpoint may not receive one.
pub fn key_headers(endpoint: &Endpoint, key: Option<&str>) -> Vec<(&'static str, String)> {
    key_headers_as(REFORGED, endpoint, key)
}

/// [`key_headers`] with `flavor`'s key header. A key goes only to a pinned endpoint, or a private or loopback one.
pub fn key_headers_as(flavor: Flavor, endpoint: &Endpoint, key: Option<&str>) -> Vec<(&'static str, String)> {
    let Some(raw) = key else {
        return Vec::new();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Vec::new();
    }
    if check_endpoint(endpoint).is_err() {
        return Vec::new();
    }
    vec![(flavor.key_header, trimmed.to_owned())]
}
