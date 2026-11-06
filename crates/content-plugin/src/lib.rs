// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The content server as a library the core loads (RD27): `content::service::ContentService`, the same service the
//! core runs compiled in.

#![allow(
    missing_docs,
    reason = "the exported entry symbol is the macro's and carries no doc; an attribute on the invocation is ignored"
)]

service_plugin::export_service!(content::service::ContentService::default());
