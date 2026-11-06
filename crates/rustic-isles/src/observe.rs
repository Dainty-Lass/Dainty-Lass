// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The request observation hook: the Reforged API's request log and metrics plug in here (core spec §3.1).

use axum::extract::Request;
use http::StatusCode;

pub trait Observer: Send + Sync {
    /// Called with the request as received (before the origin strip) and its id.
    fn begin(&self, request: &Request, request_id: &str) -> Box<dyn Observation>;
}

pub trait Observation: Send {
    fn finish(self: Box<Self>, status: StatusCode);
}

/// Observes nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoObserver;

struct Nothing;

impl Observation for Nothing {
    fn finish(self: Box<Self>, _: StatusCode) {}
}

impl Observer for NoObserver {
    fn begin(&self, _: &Request, _: &str) -> Box<dyn Observation> {
        Box::new(Nothing)
    }
}
