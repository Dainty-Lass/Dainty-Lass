// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Discord's side, through twilight: the gateway shard the interactions arrive on (outbound only, so nothing
//! listens; twilight heartbeats, resumes and reconnects) and the REST calls that register the commands and answer
//! them (twilight's rate limiter). Over the gateway an interaction is authenticated by the TLS connection to
//! Discord, so there is no signature to check as an HTTP endpoint would.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, watch};
use twilight_gateway::{CloseFrame, Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client;
use twilight_model::application::command::{Command as CommandModel, CommandType};
use twilight_model::application::interaction::InteractionContextType;
use twilight_model::channel::message::{AllowedMentions, MessageFlags};
use twilight_model::gateway::CloseCode;
use twilight_model::gateway::payload::outgoing::identify::IdentifyProperties;
use twilight_model::http::interaction::{InteractionResponse, InteractionResponseData, InteractionResponseType};
use twilight_model::oauth::ApplicationIntegrationType;
use twilight_util::builder::command::CommandBuilder;

use crate::bot::Bot;
use crate::config::{BotToken, Config};
use crate::interaction::{Command, Interaction, Language, Snowflake};

/// Discord's limit on a message's content.
const MAX_CONTENT: usize = 2000;
const REST_TIMEOUT: Duration = Duration::from_secs(10);
/// Interactions answered at once; more are left for Discord to time out.
const MAX_IN_FLIGHT: usize = 32;

/// Discord's REST API with the bot's token.
#[derive(Clone)]
pub struct Rest {
    client: Arc<Client>,
}

impl Rest {
    /// A client for `token`. No reply pings anyone, whatever its text holds.
    pub fn new(token: &BotToken) -> Self {
        let client = Client::builder()
            .token(token.expose().to_owned())
            .timeout(REST_TIMEOUT)
            .default_allowed_mentions(AllowedMentions::default())
            .build();
        Self {
            client: Arc::new(client),
        }
    }

    /// `/key` and `/status`, described from the catalog; guild channels only when registered globally.
    fn commands(bot: &Bot, global: bool) -> Vec<CommandModel> {
        let catalog = bot.catalog();
        Command::ALL
            .iter()
            .map(|command| {
                let key = format!("command.{}.description", command.name());
                let description = catalog.text(Language::En, &key, &[]);
                let mut builder = CommandBuilder::new(command.name(), description, CommandType::ChatInput);
                if let Some(pt) = catalog.pt_br(&key) {
                    builder = builder.description_localizations([("pt-BR", pt)]);
                }
                if global {
                    builder = builder
                        .contexts([InteractionContextType::Guild])
                        .integration_types([ApplicationIntegrationType::GuildInstall]);
                }
                builder.build()
            })
            .collect()
    }

    /// Replaces the application's commands with `/key` and `/status`: in each of `guilds`, or globally when there
    /// are none.
    pub async fn register(&self, application: Snowflake, guilds: &[Snowflake], bot: &Bot) -> Result<(), String> {
        let client = self.client.interaction(application.id());
        let commands = Self::commands(bot, guilds.is_empty());
        if guilds.is_empty() {
            client
                .set_global_commands(&commands)
                .await
                .map_err(|e| format!("registering the commands: {e}"))?;
        }
        for guild in guilds {
            client
                .set_guild_commands(guild.id(), &commands)
                .await
                .map_err(|e| format!("registering the commands in a guild: {e}"))?;
        }
        Ok(())
    }

    /// Acknowledges `interaction` within Discord's 3 s, as a reply only its sender sees (ephemeral).
    pub async fn defer(&self, interaction: &Interaction) -> Result<(), String> {
        let response = InteractionResponse {
            kind: InteractionResponseType::DeferredChannelMessageWithSource,
            data: Some(InteractionResponseData {
                flags: Some(MessageFlags::EPHEMERAL),
                ..InteractionResponseData::default()
            }),
        };
        self.client
            .interaction(interaction.application_id.id())
            .create_response(interaction.id.id(), interaction.token.expose(), &response)
            .await
            .map(drop)
            .map_err(|e| format!("acknowledging an interaction: {e}"))
    }

    /// Writes the deferred reply.
    pub async fn reply(&self, interaction: &Interaction, content: &str) -> Result<(), String> {
        let content: String = content.chars().take(MAX_CONTENT).collect();
        self.client
            .interaction(interaction.application_id.id())
            .update_response(interaction.token.expose())
            .content(Some(&content))
            .await
            .map(drop)
            .map_err(|e| format!("replying to an interaction: {e}"))
    }
}

/// Resolves once `stop` turns true (or its sender is gone); the watch's guard is never held across an await.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// Why Discord closed the gateway for good, in words a host can act on.
fn fatal(frame: Option<&CloseFrame<'_>>) -> Option<String> {
    let code = frame?.code;
    match CloseCode::try_from(code) {
        Ok(CloseCode::AuthenticationFailed) => {
            Some("Discord refused the bot token (close 4004): set it again with setup".into())
        }
        Ok(close) if !close.can_reconnect() => Some(format!("Discord refused the bot (close {code})")),
        _ => None,
    }
}

/// The gateway's state across reconnects.
struct Gateway {
    config: Config,
    bot: Arc<Bot>,
    rest: Rest,
    application: Option<Snowflake>,
    registered: bool,
    in_flight: Arc<Semaphore>,
}

impl Gateway {
    async fn ready(&mut self, application_id: Snowflake) -> Result<(), String> {
        if self.config.application_id.is_some_and(|named| named != application_id) {
            return Err("the bot token belongs to another application than application_id".into());
        }
        self.application = Some(application_id);
        if !self.registered {
            match self
                .rest
                .register(application_id, &self.config.guild_ids, &self.bot)
                .await
            {
                Ok(()) => {
                    self.registered = true;
                    tracing::info!(
                        guilds = self.config.guild_ids.len(),
                        "discord: /key and /status registered"
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "discord: the commands are not registered; retried on the next connection")
                }
            }
        }
        Ok(())
    }

    fn interaction(&self, interaction: Interaction) {
        if self.application != Some(interaction.application_id) {
            return;
        }
        let Ok(permit) = self.in_flight.clone().try_acquire_owned() else {
            tracing::warn!("discord: too many interactions at once; one is left unanswered");
            return;
        };
        let (bot, rest) = (self.bot.clone(), self.rest.clone());
        tokio::spawn(async move {
            let _permit = permit;
            if bot.answer_needed(&interaction) {
                if let Err(error) = rest.defer(&interaction).await {
                    tracing::warn!(%error, "discord: an interaction was not acknowledged");
                    return;
                }
                if let Some(text) = bot.answer(&interaction).await
                    && let Err(error) = rest.reply(&interaction, &text).await
                {
                    tracing::warn!(%error, "discord: a reply was not sent");
                }
            }
        });
    }
}

/// Runs the gateway until `stop`; twilight reconnects, resuming where it can, until Discord refuses the bot.
pub async fn run(config: Config, bot: Arc<Bot>, mut stop: watch::Receiver<bool>) -> Result<(), String> {
    // aws-lc-rs by name: another provider compiled in leaves rustls no default (D10)
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let shard_config = twilight_gateway::ConfigBuilder::new(config.token.expose().to_owned(), Intents::empty())
        .identify_properties(IdentifyProperties::new(
            "dainty-lass",
            "dainty-lass",
            std::env::consts::OS,
        ))
        .build();
    let mut shard = Shard::with_config(ShardId::ONE, shard_config);
    let mut gateway = Gateway {
        rest: Rest::new(&config.token),
        config,
        bot,
        application: None,
        registered: false,
        in_flight: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
    };
    let wanted = EventTypeFlags::READY | EventTypeFlags::INTERACTION_CREATE;
    let mut refused = None;
    loop {
        let next = tokio::select! {
            () = stopped(&mut stop) => {
                shard.close(CloseFrame::NORMAL);
                return Ok(());
            }
            next = shard.next_event(wanted) => next,
        };
        match next {
            // The shard ends only after a close it may not reconnect from
            None => return Err(refused.unwrap_or_else(|| "Discord closed the gateway for good".to_owned())),
            Some(Err(error)) => tracing::warn!(%error, "discord: a gateway message was dropped"),
            Some(Ok(Event::GatewayClose(frame))) => refused = fatal(frame.as_ref()).or(refused),
            Some(Ok(Event::Ready(ready))) => gateway.ready(Snowflake::from(ready.application.id)).await?,
            Some(Ok(Event::InteractionCreate(create))) => match Interaction::from_model(&create.0) {
                Ok(interaction) => gateway.interaction(interaction),
                Err(error) => tracing::warn!(%error, "discord: an interaction refused"),
            },
            Some(Ok(_)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_refusals_end_the_bot() {
        assert!(fatal(Some(&CloseFrame::new(4004, ""))).is_some_and(|r| r.contains("4004")));
        assert!(fatal(Some(&CloseFrame::new(4014, ""))).is_some());
        assert_eq!(fatal(Some(&CloseFrame::new(4009, ""))), None);
        assert_eq!(fatal(Some(&CloseFrame::NORMAL)), None);
        assert_eq!(fatal(None), None);
    }
}
