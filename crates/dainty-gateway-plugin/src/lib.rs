// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The gateway and the allowlist as one library the core loads (RD27): `gateway::service::GatewayService`, the same
//! service the core runs compiled in.

#![allow(
    missing_docs,
    reason = "the exported entry symbol is the macro's and carries no doc; an attribute on the invocation is ignored"
)]

dainty_service_plugin::export_service!(dainty_gateway::service::GatewayService::library());
