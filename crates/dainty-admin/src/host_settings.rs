// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! settings.toml's `[logs]` for the dashboard (`docs/logging.md`): `logSettings` reads what the file says and what this
//! run started with, `setLogSettings` writes the file. The log is set up once at start, so a change takes effect on
//! the next start, and `restartNeeded` says so. The host kit hands the file in through
//! [`crate::module_control::ModuleControl::host_settings`]; a standalone admin API has none and says so.

use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions, InputObject, SimpleObject};

use crate::module_control::Modules;

/// `[logs]`, as the file says and as the running server started with it.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct LogSettings {
    /// text or json
    pub format: String,
    /// auto, always or never
    pub color: String,
    /// The GELF collector (Graylog), when one is named
    pub gelf_url: Option<String>,
    /// The file differs from what this run started with
    pub restart_needed: bool,
}

/// A change to `[logs]`: a field left out keeps its value; an empty `gelfUrl` stops shipping.
#[derive(InputObject, Clone, Debug, Default, PartialEq, Eq)]
pub struct LogSettingsInput {
    /// `text` or `json`.
    pub format: Option<String>,
    /// `auto`, `always` or `never`.
    pub color: Option<String>,
    /// The GELF collector's URL; empty stops shipping.
    pub gelf_url: Option<String>,
}

/// What the admin API may do with settings.toml.
pub trait HostSettings: Send + Sync {
    /// `[logs]` as the file holds it.
    fn logs(&self) -> Result<LogSettings, String>;
    /// Validated as `setup` validates the file; refused whole when any part is wrong.
    fn set_logs(&self, change: &LogSettingsInput) -> Result<LogSettings, String>;
}

fn fail(code: &str, message: impl Into<String>) -> async_graphql::Error {
    async_graphql::Error::new(message).extend_with(|_, e| e.set("code", code))
}

fn settings(ctx: &Context<'_>) -> async_graphql::Result<Arc<dyn HostSettings>> {
    ctx.data::<Modules>()?
        .0
        .as_ref()
        .and_then(|control| control.host_settings())
        .ok_or_else(|| {
            fail(
                "SETTINGS_UNAVAILABLE",
                "this admin API runs outside the server, so it has no settings.toml: run it through dainty-host serve",
            )
        })
}

/// The `logSettings` query: `[logs]`, or a coded error when there is no settings.toml or it cannot be read.
pub fn logs(ctx: &Context<'_>) -> async_graphql::Result<LogSettings> {
    settings(ctx)?.logs().map_err(|e| fail("SETTINGS_UNREADABLE", e))
}

/// The `setLogSettings` mutation: writes `[logs]` and answers it, or a coded error naming what was refused.
pub fn set_logs(ctx: &Context<'_>, change: &LogSettingsInput) -> async_graphql::Result<LogSettings> {
    settings(ctx)?.set_logs(change).map_err(|e| fail("SETTINGS_REFUSED", e))
}
