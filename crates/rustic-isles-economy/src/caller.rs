// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Who a request is from, as game auth found it, and the host runtime's rifts capability.

use axum::extract::Request;
use axum::response::Response;
use rifts_seam::RuntimeAuth;
use rustic_isles::Shared;
use rustic_isles_auth::{AuthData, RuntimeCaller};

/// The realms seam's handle, when it is rifts' runtime (decision item 2 of the inventory spec).
pub(crate) fn capability(runtime: &RuntimeCaller) -> Option<RuntimeAuth> {
    runtime.handle()?.downcast_ref::<RuntimeAuth>().cloned()
}

/// The caller already authenticated by another layer, else game auth (its refusal is the answer).
pub(crate) async fn game_auth(shared: &Shared, request: &mut Request) -> Result<AuthData, Response> {
    if let Some(data) = request.extensions().get::<AuthData>() {
        return Ok(data.clone());
    }
    shared.auth().game_auth(request).await
}

/// A `HostWriteKind`'s name as the logs and tags write it.
pub(crate) fn kind_name(kind: rifts_seam::HostWriteKind) -> String {
    format!("{kind:?}")
}
