// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What every service shares: env files, Express-compatible routing, bodies, JSON and rate limits, JavaScript value
//! semantics, logging and metrics, and the in-process lifecycle and status registry.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

pub mod body;
pub mod collate;
pub mod console;
pub mod context;
pub mod env;
pub mod gelf;
pub mod js;
pub mod jsjson;
pub mod json;
pub mod linelog;
pub mod log;
pub mod metrics;
pub mod ratelimit;
pub mod routing;
pub mod serve;
pub mod task;
pub mod telemetry;
