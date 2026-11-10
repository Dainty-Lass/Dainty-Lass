// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Discord bot (module sharing A7), a built-in service of the server process: `/key` replies, to the member
//! alone, with dainty-dash's signup link when signups are on and otherwise a single-use login URI; `/status` says
//! whether the game is up. It asks the game module through the registry (`GameCall`) and records each `/key`'s
//! outcome on the bus (D6). Its token comes only from its env file (D5). `docs/security/discord.md` is its
//! checklist.

pub mod bot;
pub mod catalog;
pub mod config;
pub mod discord;
pub mod interaction;
pub mod signup;

use std::sync::Arc;

use dainty_service_kit::context::ServiceContext;

pub use bot::{Bot, GameAccess, SignupSource};
pub use config::Config;

/// Runs the bot until `ctx` stops. Ready as soon as it starts connecting: Discord being down never holds the
/// server's start, and the bot keeps reconnecting.
pub async fn run(config: Config, ctx: ServiceContext, access: Arc<dyn GameAccess>) -> Result<(), String> {
    let catalog = catalog::Catalog::load()?;
    let signup = Arc::new(signup::DashboardSignup::new(config.dashboard_url.as_ref()));
    let bot = Arc::new(Bot::new(access, signup, catalog, config.guild_ids.clone())?);
    ctx.status().ready(None, Some("connecting to Discord".to_owned()));
    let result = discord::run(config, bot, ctx.stop_receiver()).await;
    match &result {
        Ok(()) => ctx.status().stopped(),
        Err(error) => ctx.status().failed(error.clone()),
    }
    result
}
