// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Chat as a library the core loads (RD27): the same service the core can compile in (`chat::service`).

service_plugin::export_service!(chat::service::ChatService::default());
