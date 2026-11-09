// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Discord's side: the gateway connection the interactions arrive on (outbound only, so nothing listens) and the
//! REST calls that register the commands and answer them. Over the gateway an interaction is authenticated by
//! the TLS connection to Discord, so there is no signature to check as an HTTP endpoint would.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{Semaphore, watch};
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::bot::Bot;
use crate::config::{BotToken, Config};
use crate::http::{Client, tls_config};
use crate::interaction::{Command, GatewayEvent, Interaction, MAX_FRAME, Snowflake, parse_frame};

/// Discord's REST API.
pub const API: &str = "https://discord.com/api/v10";
/// The gateway connected to first; a resume uses the one READY names.
pub const GATEWAY: &str = "wss://gateway.discord.gg";
/// The query every gateway URL takes: API v10, JSON, no compression.
const GATEWAY_QUERY: &str = "/?v=10&encoding=json";
/// Discord's limit on a message's content.
const MAX_CONTENT: usize = 2000;
/// The largest REST answer read.
const MAX_REST_ANSWER: usize = 64 * 1024;
const REST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Interactions answered at once; more are left for Discord to time out.
const MAX_IN_FLIGHT: usize = 32;

/// Discord's REST API with the bot's token.
#[derive(Clone)]
pub struct Rest {
    client: Client,
    token: BotToken,
}

impl Rest {
    /// A client for `token`; https only.
    pub fn new(token: BotToken) -> Result<Self, String> {
        Ok(Self {
            client: Client::new(false)?,
            token,
        })
    }

    /// One call; a 429 is retried once after Discord's `retry_after` (at most 5 s). The error never holds the
    /// path, which may carry an interaction token.
    async fn send(&self, method: http::Method, path: &str, body: &Value, what: &str) -> Result<Value, String> {
        for attempt in 0..2 {
            let request = http::Request::builder()
                .method(method.clone())
                .uri(format!("{API}{path}"))
                .header(http::header::AUTHORIZATION, format!("Bot {}", self.token.expose()))
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(
                    http::header::USER_AGENT,
                    concat!(
                        "DiscordBot (https://git.silentspud.com/silentspud/dainty-lass, ",
                        env!("CARGO_PKG_VERSION"),
                        ")"
                    ),
                )
                .body(Full::new(Bytes::from(body.to_string())))
                .map_err(|_| format!("{what}: a request that can't be built"))?;
            let answer = self
                .client
                .call(request, MAX_REST_ANSWER, REST_TIMEOUT)
                .await
                .map_err(|e| format!("{what}: {e}"))?;
            if answer.status == http::StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
                let wait = serde_json::from_slice::<Value>(&answer.body)
                    .ok()
                    .and_then(|v| v.get("retry_after").and_then(Value::as_f64))
                    .filter(|s| s.is_finite() && *s >= 0.0)
                    .map_or(1.0, |s| s.min(5.0));
                sleep(Duration::from_secs_f64(wait)).await;
                continue;
            }
            if !answer.status.is_success() {
                return Err(format!("{what}: Discord answered {}", answer.status.as_u16()));
            }
            return Ok(serde_json::from_slice(&answer.body).unwrap_or(Value::Null));
        }
        Err(format!("{what}: rate limited by Discord"))
    }

    /// Replaces the application's commands with `/key` and `/status`: in each of `guilds`, or globally (guild
    /// channels only) when there are none.
    pub async fn register(&self, application: Snowflake, guilds: &[Snowflake], bot: &Bot) -> Result<(), String> {
        let catalog = bot.catalog();
        let global = guilds.is_empty();
        let commands: Vec<Value> = Command::ALL
            .iter()
            .map(|command| {
                let key = format!("command.{}.description", command.name());
                let mut body = json!({
                    "name": command.name(),
                    "type": 1,
                    "description": catalog.text(crate::interaction::Language::En, &key, &[]),
                });
                if let Some(pt) = catalog.pt_br(&key) {
                    body["description_localizations"] = json!({"pt-BR": pt});
                }
                if global {
                    // Guild channels only, for a bot installed to a guild
                    body["contexts"] = json!([0]);
                    body["integration_types"] = json!([0]);
                }
                body
            })
            .collect();
        let body = Value::Array(commands);
        if global {
            self.send(
                http::Method::PUT,
                &format!("/applications/{application}/commands"),
                &body,
                "registering the commands",
            )
            .await?;
        }
        for guild in guilds {
            self.send(
                http::Method::PUT,
                &format!("/applications/{application}/guilds/{guild}/commands"),
                &body,
                "registering the commands in a guild",
            )
            .await?;
        }
        Ok(())
    }

    /// Acknowledges `interaction` within Discord's 3 s, as a reply only its sender sees (flag 64, ephemeral).
    pub async fn defer(&self, interaction: &Interaction) -> Result<(), String> {
        let path = format!(
            "/interactions/{}/{}/callback",
            interaction.id,
            interaction.token.expose()
        );
        let body = json!({"type": 5, "data": {"flags": 64}});
        self.send(http::Method::POST, &path, &body, "acknowledging an interaction")
            .await
            .map(drop)
    }

    /// Writes the deferred reply. No mention in it pings anyone.
    pub async fn reply(&self, interaction: &Interaction, content: &str) -> Result<(), String> {
        let path = format!(
            "/webhooks/{}/{}/messages/@original",
            interaction.application_id,
            interaction.token.expose()
        );
        let content: String = content.chars().take(MAX_CONTENT).collect();
        let body = json!({"content": content, "allowed_mentions": {"parse": []}});
        self.send(http::Method::PATCH, &path, &body, "replying to an interaction")
            .await
            .map(drop)
    }
}

/// A session to resume after a dropped connection.
#[derive(Debug, Clone)]
struct Session {
    id: String,
    resume_url: Option<String>,
    seq: Option<u64>,
}

enum Ended {
    Stop,
    Reconnect,
    /// Discord refused the bot for good (a bad token, intents it lacks).
    Fatal(String),
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

async fn connect(base: &str) -> Result<Socket, String> {
    let url = url::Url::parse(base).map_err(|_| "a gateway URL that does not parse".to_owned())?;
    let host = url
        .host_str()
        .filter(|_| url.scheme() == "wss")
        .ok_or_else(|| "a gateway URL that is not wss://".to_owned())?
        .to_owned();
    let port = url.port().unwrap_or(443);
    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| "connecting timed out".to_owned())?
        .map_err(|e| format!("could not connect: {e}"))?;
    let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| "a bad gateway host".to_owned())?;
    let tls = timeout(
        CONNECT_TIMEOUT,
        tokio_rustls::TlsConnector::from(Arc::new(tls_config()?)).connect(name, tcp),
    )
    .await
    .map_err(|_| "TLS timed out".to_owned())?
    .map_err(|e| format!("TLS: {e}"))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let request = format!("wss://{host}:{port}{GATEWAY_QUERY}");
    let (socket, _) = timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::client_async_with_config(request, tls, Some(config)),
    )
    .await
    .map_err(|_| "the WebSocket handshake timed out".to_owned())?
    .map_err(|e| format!("WebSocket: {e}"))?;
    Ok(socket)
}

/// Resolves once `stop` turns true (or its sender is gone); the watch's guard is never held across an await.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// A spread of up to the interval for the first heartbeat, as Discord asks, without a random number crate.
fn jitter(interval: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    interval.mul_f64(f64::from(nanos % 1000) / 1000.0)
}

/// The gateway's state across connections.
struct Gateway {
    config: Config,
    bot: Arc<Bot>,
    rest: Rest,
    session: Option<Session>,
    application: Option<Snowflake>,
    registered: bool,
    in_flight: Arc<Semaphore>,
}

impl Gateway {
    async fn send(socket: &mut Socket, body: &Value) -> Result<(), ()> {
        socket.send(Message::text(body.to_string())).await.map_err(drop)
    }

    async fn drive(&mut self, socket: &mut Socket, stop: &mut watch::Receiver<bool>) -> Ended {
        let hello = loop {
            let next = tokio::select! {
                () = stopped(stop) => return Ended::Stop,
                next = timeout(HELLO_TIMEOUT, socket.next()) => next,
            };
            match next {
                Ok(Some(Ok(Message::Text(text)))) => match parse_frame(text.as_bytes()) {
                    Ok((_, GatewayEvent::Hello { interval_ms })) => break Duration::from_millis(interval_ms),
                    Ok(_) => {}
                    Err(_) => return Ended::Reconnect,
                },
                Ok(Some(Ok(_))) => {}
                _ => return Ended::Reconnect,
            }
        };
        let open = match &self.session {
            Some(session) => json!({"op": 6, "d": {
                "token": self.config.token.expose(), "session_id": session.id, "seq": session.seq,
            }}),
            None => json!({"op": 2, "d": {
                "token": self.config.token.expose(),
                // No privileged or message intents: interactions arrive without any
                "intents": 0,
                "properties": {"os": std::env::consts::OS, "browser": "dainty-lass", "device": "dainty-lass"},
            }}),
        };
        if Self::send(socket, &open).await.is_err() {
            return Ended::Reconnect;
        }
        let mut next_beat = Instant::now() + jitter(hello);
        let mut acked = true;
        loop {
            let message = tokio::select! {
                () = stopped(stop) => {
                    let _ = socket.close(None).await;
                    return Ended::Stop;
                }
                () = sleep_until(next_beat) => {
                    // No ack since the last beat: a dead connection Discord asks to drop
                    if !acked {
                        return Ended::Reconnect;
                    }
                    acked = false;
                    next_beat = Instant::now() + hello;
                    let seq = self.session.as_ref().and_then(|s| s.seq);
                    if Self::send(socket, &json!({"op": 1, "d": seq})).await.is_err() {
                        return Ended::Reconnect;
                    }
                    continue;
                }
                message = socket.next() => message,
            };
            let text = match message {
                Some(Ok(Message::Text(text))) => text,
                Some(Ok(Message::Close(frame))) => return self.closed(frame.map(|f| f.code)),
                Some(Ok(Message::Binary(_))) | Some(Err(_)) | None => return Ended::Reconnect,
                Some(Ok(_)) => continue,
            };
            let (seq, event) = match parse_frame(text.as_bytes()) {
                Ok(parsed) => parsed,
                Err(error) => {
                    tracing::warn!(%error, "discord: dropping the gateway connection");
                    return Ended::Reconnect;
                }
            };
            if let (Some(seq), Some(session)) = (seq, self.session.as_mut()) {
                session.seq = Some(seq);
            }
            match event {
                GatewayEvent::HeartbeatAck => acked = true,
                GatewayEvent::HeartbeatRequest => next_beat = Instant::now(),
                GatewayEvent::Reconnect => return Ended::Reconnect,
                GatewayEvent::InvalidSession { resumable } => {
                    if !resumable {
                        self.session = None;
                    }
                    // Discord asks for a pause of 1 to 5 s before identifying again
                    sleep(Duration::from_secs(2)).await;
                    return Ended::Reconnect;
                }
                GatewayEvent::Ready {
                    session_id,
                    resume_url,
                    application_id,
                } => {
                    if let Some(ended) = self.ready(session_id, resume_url, application_id, seq).await {
                        return ended;
                    }
                }
                GatewayEvent::Interaction(interaction) => self.interaction(interaction),
                GatewayEvent::BadInteraction(error) => tracing::warn!(%error, "discord: an interaction refused"),
                GatewayEvent::Hello { .. } | GatewayEvent::Resumed | GatewayEvent::Other => {}
            }
        }
    }

    async fn ready(
        &mut self,
        session_id: String,
        resume_url: Option<String>,
        application_id: Option<Snowflake>,
        seq: Option<u64>,
    ) -> Option<Ended> {
        let Some(application) = application_id.or(self.config.application_id) else {
            return Some(Ended::Fatal("Discord's READY names no application".into()));
        };
        if self.config.application_id.is_some_and(|named| named != application) {
            return Some(Ended::Fatal(
                "the bot token belongs to another application than application_id".into(),
            ));
        }
        self.application = Some(application);
        self.session = Some(Session {
            id: session_id,
            resume_url,
            seq,
        });
        if !self.registered {
            match self.rest.register(application, &self.config.guild_ids, &self.bot).await {
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
        None
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

    fn closed(&mut self, code: Option<CloseCode>) -> Ended {
        let code = code.map(u16::from);
        match code {
            Some(4004) => Ended::Fatal("Discord refused the bot token (close 4004): set it again with setup".into()),
            Some(code @ (4010..=4014)) => Ended::Fatal(format!("Discord refused the bot (close {code})")),
            // An invalid sequence or a session timed out: identify afresh
            Some(4007 | 4009) => {
                self.session = None;
                Ended::Reconnect
            }
            _ => Ended::Reconnect,
        }
    }
}

/// Runs the gateway until `stop`, reconnecting (resuming where it can) with a backoff of 1 s doubling to 60 s.
pub async fn run(config: Config, bot: Arc<Bot>, mut stop: watch::Receiver<bool>) -> Result<(), String> {
    let rest = Rest::new(config.token.clone())?;
    let mut gateway = Gateway {
        config,
        bot,
        rest,
        session: None,
        application: None,
        registered: false,
        in_flight: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
    };
    let mut backoff = Duration::from_secs(1);
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let base = gateway
            .session
            .as_ref()
            .and_then(|s| s.resume_url.clone())
            .unwrap_or_else(|| GATEWAY.to_owned());
        let started = Instant::now();
        match connect(&base).await {
            Ok(mut socket) => match gateway.drive(&mut socket, &mut stop).await {
                Ended::Stop => return Ok(()),
                Ended::Fatal(reason) => return Err(reason),
                Ended::Reconnect => {
                    if started.elapsed() > Duration::from_secs(60) {
                        backoff = Duration::from_secs(1);
                    }
                }
            },
            Err(error) => {
                tracing::warn!(%error, "discord: could not reach the gateway");
                // A resume URL that fails is dropped with its session
                gateway.session = None;
            }
        }
        tokio::select! {
            () = stopped(&mut stop) => return Ok(()),
            () = sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_heartbeat_falls_inside_the_interval() {
        let interval = Duration::from_millis(41_250);
        assert!(jitter(interval) <= interval);
    }
}
