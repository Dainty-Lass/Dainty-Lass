// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway and the allowlist as one library the core loads (RD27): `gateway::service::GatewayService`, the same
//! service the core runs compiled in.

#![allow(
    missing_docs,
    reason = "the exported entry symbol is the macro's and carries no doc; an attribute on the invocation is ignored"
)]

service_plugin::export_service!(gateway::service::GatewayService::library());
