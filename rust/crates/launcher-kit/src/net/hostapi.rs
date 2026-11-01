// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use crate::net::http::{Endpoint, check_endpoint};

pub const KEY_HEADER: &str = "x-reforged-user-api-key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flavor {
    pub id: &'static str,
    pub key_header: &'static str,
    pub status_path: &'static str,
    pub register_path: &'static str,
    pub account_path: &'static str,
    pub versions_path: &'static str,
    pub runtime_manifest_path: &'static str,
    pub region_ping_path: &'static str,
}

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

pub fn key_headers(endpoint: &Endpoint, key: Option<&str>) -> Vec<(&'static str, String)> {
    key_headers_as(REFORGED, endpoint, key)
}

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
