// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `/key` and `/status` (A7), apart from Discord's transport so they are tested without it. Every reply is
//! ephemeral; a login URI and its code never reach a log line or an event, only the outcome does.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use dainty_module_api::game::{Availability, GameAnswer, GameCall, UnavailableReason};
use dainty_module_api::{BoxFuture, Event, EventName, ModuleError};
use dainty_protocol::invite::{Invite, format_invite};
use serde_json::json;
use tokio::time::Instant;

use crate::catalog::Catalog;
use crate::interaction::{Command, Interaction, InteractionKind, Language, Snowflake};

/// The event each `/key` records, with its outcome (D6).
pub const KEY_REQUESTED_EVENT: &str = "discord.key.requested";
/// How long a login URI's code lives (D4: the game module's `expires_at`).
pub const INVITE_EXPIRY: Duration = Duration::from_secs(30 * 60);
/// A live code with less than this left is not handed out again; a fresh one is made instead.
pub const MIN_LEFT: Duration = Duration::from_secs(5 * 60);
/// `/key` requests a member may make in [`WINDOW`].
pub const MAX_REQUESTS: usize = 5;
/// The rate limit's window.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

/// What the bot asks of the server: the game module through the registry, the bus, and the server's invite.
pub trait GameAccess: Send + Sync {
    /// A game call to the 1.14.7 module.
    fn game(&self, call: GameCall) -> BoxFuture<'_, Result<GameAnswer, ModuleError>>;
    /// An event on the registry's bus, which the recorder keeps (A5).
    fn emit(&self, event: Event);
    /// This server's invite, whose code the bot replaces; `None` when it can't be made (no certificate yet).
    fn invite(&self) -> Option<Invite>;
}

/// Whether dainty-dash takes signups (A8, D3).
pub trait SignupSource: Send + Sync {
    /// The signup page when signups are on; `None` when off, unknown or unreachable.
    fn signup_url(&self) -> BoxFuture<'_, Option<String>>;
}

/// How a `/key` ended, as the event records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The game is down, with the module's reason.
    Unavailable(Option<UnavailableReason>),
    /// The dashboard's signup link.
    SignupLink,
    /// A login URI; `repeat` when it is the member's live one again.
    LoginUri {
        /// The member's live code, handed out again.
        repeat: bool,
    },
    /// Over the member's rate limit.
    RateLimited,
    /// The invite could not be made or written.
    Failed,
}

impl Outcome {
    fn event(self) -> serde_json::Value {
        match self {
            Self::Unavailable(reason) => {
                json!({"outcome": "unavailable", "reason": reason.map_or("unknown", UnavailableReason::code)})
            }
            Self::SignupLink => json!({"outcome": "signup_link"}),
            Self::LoginUri { repeat } => json!({"outcome": "login_uri", "repeat": repeat}),
            Self::RateLimited => json!({"outcome": "rate_limited"}),
            Self::Failed => json!({"outcome": "failed"}),
        }
    }
}

#[derive(Default)]
struct Member {
    requests: VecDeque<Instant>,
    live: Option<(String, Instant)>,
    turn: Arc<tokio::sync::Mutex<()>>,
}

/// The commands' logic.
pub struct Bot {
    access: Arc<dyn GameAccess>,
    signup: Arc<dyn SignupSource>,
    catalog: Catalog,
    guilds: Vec<Snowflake>,
    members: Mutex<HashMap<Snowflake, Member>>,
    key_event: EventName,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn minutes(left: Duration) -> String {
    left.as_secs().div_ceil(60).max(1).to_string()
}

impl Bot {
    /// A bot answering in `guilds` (every guild when empty).
    pub fn new(
        access: Arc<dyn GameAccess>,
        signup: Arc<dyn SignupSource>,
        catalog: Catalog,
        guilds: Vec<Snowflake>,
    ) -> Result<Self, String> {
        Ok(Self {
            access,
            signup,
            catalog,
            guilds,
            members: Mutex::new(HashMap::new()),
            key_event: EventName::new(KEY_REQUESTED_EVENT).map_err(|e| e.to_string())?,
        })
    }

    /// The catalogs, for the commands' descriptions.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Whether `interaction` gets a reply at all.
    pub fn answer_needed(&self, interaction: &Interaction) -> bool {
        matches!(
            interaction.kind,
            InteractionKind::Command(_) | InteractionKind::UnknownCommand
        )
    }

    /// The reply to `interaction`; `None` for one that gets none (a ping, a component).
    pub async fn answer(&self, interaction: &Interaction) -> Option<String> {
        let language = interaction.language;
        let command = match interaction.kind {
            InteractionKind::Command(command) => command,
            InteractionKind::UnknownCommand => return Some(self.catalog.text(language, "command.unknown", &[])),
            InteractionKind::Ping | InteractionKind::Other => return None,
        };
        // Never in a direct message, and only in the configured guilds: a key is for this server's community
        let in_place = interaction
            .guild_id
            .is_some_and(|g| self.guilds.is_empty() || self.guilds.contains(&g));
        if !in_place {
            return Some(self.catalog.text(language, "command.wrong_guild", &[]));
        }
        Some(match command {
            Command::Status => self.status(language).await,
            Command::Key => self.key(interaction.user_id, language).await,
        })
    }

    async fn availability(&self) -> Result<Availability, Option<UnavailableReason>> {
        match self.access.game(GameCall::Availability).await {
            Ok(GameAnswer::Availability(availability)) if availability.up => Ok(availability),
            Ok(GameAnswer::Availability(availability)) => Err(availability.reason),
            Ok(_) => Err(None),
            Err(error) => Err(UnavailableReason::from_error(&error)),
        }
    }

    async fn status(&self, language: Language) -> String {
        match self.availability().await {
            Ok(availability) => {
                let players = availability.hub.map_or(0, |hub| hub.players).to_string();
                self.catalog.text(language, "status.up", &[("players", &players)])
            }
            Err(reason) => self.catalog.reason(language, reason),
        }
    }

    async fn key(&self, user: Snowflake, language: Language) -> String {
        // One /key at a time per member, so two quick ones can't make two codes
        let turn = lock(&self.members).entry(user).or_default().turn.clone();
        let _turn = turn.lock().await;
        let (text, outcome) = self.key_turn(user, language).await;
        self.access.emit(Event {
            name: self.key_event.clone(),
            payload: outcome.event(),
        });
        text
    }

    async fn key_turn(&self, user: Snowflake, language: Language) -> (String, Outcome) {
        let now = Instant::now();
        let live = {
            let mut members = lock(&self.members);
            members.retain(|id, m| {
                m.requests.retain(|at| now.duration_since(*at) < WINDOW);
                m.live = m.live.take().filter(|(_, until)| *until > now);
                *id == user || !m.requests.is_empty() || m.live.is_some() || Arc::strong_count(&m.turn) > 1
            });
            let member = members.entry(user).or_default();
            if member.requests.len() >= MAX_REQUESTS {
                let oldest = member.requests.front().copied().unwrap_or(now);
                let left = (oldest + WINDOW).saturating_duration_since(now);
                let text = self
                    .catalog
                    .text(language, "key.rate_limited", &[("minutes", &minutes(left))]);
                return (text, Outcome::RateLimited);
            }
            member.requests.push_back(now);
            member.live.clone()
        };

        if let Err(reason) = self.availability().await {
            return (self.catalog.reason(language, reason), Outcome::Unavailable(reason));
        }
        if let Some(url) = self.signup.signup_url().await {
            return (
                self.catalog.text(language, "key.signup", &[("url", &url)]),
                Outcome::SignupLink,
            );
        }
        if let Some((uri, until)) = live.filter(|(_, until)| until.saturating_duration_since(now) >= MIN_LEFT) {
            return (
                self.login_text(language, &uri, until.saturating_duration_since(now)),
                Outcome::LoginUri { repeat: true },
            );
        }

        let Some(template) = self.access.invite() else {
            return (self.catalog.text(language, "key.failed", &[]), Outcome::Failed);
        };
        let call = GameCall::CreateInvite {
            uses: 1,
            expires_in_secs: Some(INVITE_EXPIRY.as_secs().try_into().unwrap_or(u32::MAX)),
            note: Some(format!("Discord /key {user}")),
            require_up: true,
        };
        let code = match self.access.game(call).await {
            Ok(GameAnswer::Invite { code }) => code,
            Ok(_) => return (self.catalog.text(language, "key.failed", &[]), Outcome::Failed),
            Err(error) => {
                return match UnavailableReason::from_error(&error) {
                    // The hub went down since the check: the module's recheck (A3)
                    Some(reason) => (
                        self.catalog.reason(language, Some(reason)),
                        Outcome::Unavailable(Some(reason)),
                    ),
                    None => {
                        tracing::warn!(kind = ?error.kind, "discord: /key could not make an invite");
                        (self.catalog.text(language, "key.failed", &[]), Outcome::Failed)
                    }
                };
            }
        };
        let Ok(uri) = format_invite(&Invite { code, ..template }) else {
            return (self.catalog.text(language, "key.failed", &[]), Outcome::Failed);
        };
        let until = now + INVITE_EXPIRY;
        if let Some(member) = lock(&self.members).get_mut(&user) {
            member.live = Some((uri.clone(), until));
        }
        (
            self.login_text(language, &uri, INVITE_EXPIRY),
            Outcome::LoginUri { repeat: false },
        )
    }

    fn login_text(&self, language: Language, uri: &str, left: Duration) -> String {
        self.catalog
            .text(language, "key.login", &[("uri", uri), ("minutes", &minutes(left))])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::sync::atomic::{AtomicU32, Ordering};

    use dainty_module_api::ErrorKind;
    use dainty_module_api::game::{GameServer, GameServerKind};
    use dainty_protocol::invite::{ServerMode, dainty_version, parse_invite};

    use super::*;
    use crate::interaction::InteractionToken;

    const GUILD: u64 = 77;

    struct Fake {
        up: Mutex<Result<(), UnavailableReason>>,
        recheck_down: Mutex<bool>,
        invites: AtomicU32,
        events: Mutex<Vec<Event>>,
    }

    impl Fake {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                up: Mutex::new(Ok(())),
                recheck_down: Mutex::new(false),
                invites: AtomicU32::new(0),
                events: Mutex::new(Vec::new()),
            })
        }

        fn outcomes(&self) -> Vec<serde_json::Value> {
            lock(&self.events).iter().map(|e| e.payload.clone()).collect()
        }
    }

    fn hub() -> GameServer {
        GameServer {
            id: "hub".into(),
            kind: GameServerKind::Hub,
            title: "Ramsgate".into(),
            map: "Ramsgate".into(),
            host: "127.0.0.1".into(),
            port: 7777,
            players: 4,
            max_players: 50,
            ready: true,
            started_at: None,
        }
    }

    impl GameAccess for Fake {
        fn game(&self, call: GameCall) -> BoxFuture<'_, Result<GameAnswer, ModuleError>> {
            Box::pin(async move {
                match call {
                    GameCall::Availability => Ok(GameAnswer::Availability(match *lock(&self.up) {
                        Ok(()) => Availability::up(hub()),
                        Err(reason) => Availability::down(reason),
                    })),
                    GameCall::CreateInvite {
                        uses,
                        require_up,
                        expires_in_secs,
                        ..
                    } => {
                        assert_eq!((uses, require_up, expires_in_secs), (1, true, Some(1800)));
                        if *lock(&self.recheck_down) {
                            return Err(UnavailableReason::HubNotReady.error());
                        }
                        let n = self.invites.fetch_add(1, Ordering::SeqCst);
                        Ok(GameAnswer::Invite {
                            code: format!("CODE{n}"),
                        })
                    }
                    _ => Err(ModuleError::new(ErrorKind::Unsupported, "no")),
                }
            })
        }

        fn emit(&self, event: Event) {
            lock(&self.events).push(event);
        }

        fn invite(&self) -> Option<Invite> {
            Some(Invite {
                version: dainty_version(),
                mode: ServerMode::Public,
                host: "hunts.example.org".into(),
                port: 443,
                code: String::new(),
                name: "Friday Hunts".into(),
                share: None,
                fp: Some("ab".repeat(32)),
            })
        }
    }

    struct Signup(Option<String>);

    impl SignupSource for Signup {
        fn signup_url(&self) -> BoxFuture<'_, Option<String>> {
            Box::pin(async move { self.0.clone() })
        }
    }

    fn bot(fake: &Arc<Fake>, signup: Option<&str>) -> Bot {
        Bot::new(
            fake.clone(),
            Arc::new(Signup(signup.map(str::to_owned))),
            Catalog::load().unwrap(),
            vec![Snowflake::parse(&GUILD.to_string()).unwrap()],
        )
        .unwrap()
    }

    fn asks(command: Command, user: u64, guild: Option<u64>) -> Interaction {
        Interaction {
            id: Snowflake::parse("1").unwrap(),
            application_id: Snowflake::parse("2").unwrap(),
            token: InteractionToken::parse("t").unwrap(),
            kind: InteractionKind::Command(command),
            guild_id: guild.and_then(|g| Snowflake::parse(&g.to_string())),
            user_id: Snowflake::parse(&user.to_string()).unwrap(),
            language: Language::En,
        }
    }

    fn uri_in(text: &str) -> Invite {
        let uri = text.split('`').nth(1).unwrap();
        parse_invite(uri).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn key_hands_out_one_live_code_per_member() {
        let fake = Fake::new();
        let bot = bot(&fake, None);
        let first = bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        assert_eq!(uri_in(&first).code, "CODE0");
        assert!(first.contains("30 minutes"), "{first}");
        let again = bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        assert_eq!(uri_in(&again).code, "CODE0", "the same URI inside the expiry");
        let other = bot.answer(&asks(Command::Key, 6, Some(GUILD))).await.unwrap();
        assert_eq!(uri_in(&other).code, "CODE1");
        // Too close to the expiry: a fresh one
        tokio::time::advance(INVITE_EXPIRY - MIN_LEFT + Duration::from_secs(1)).await;
        let late = bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        assert_eq!(uri_in(&late).code, "CODE2");
        assert_eq!(
            fake.outcomes(),
            vec![
                json!({"outcome": "login_uri", "repeat": false}),
                json!({"outcome": "login_uri", "repeat": true}),
                json!({"outcome": "login_uri", "repeat": false}),
                json!({"outcome": "login_uri", "repeat": false}),
            ]
        );
        assert!(
            !lock(&fake.events)
                .iter()
                .any(|e| e.payload.to_string().contains("CODE")),
            "an event never holds a code"
        );
        assert_eq!(lock(&fake.events)[0].name.as_str(), KEY_REQUESTED_EVENT);
    }

    #[tokio::test(start_paused = true)]
    async fn key_is_rate_limited_per_member() {
        let fake = Fake::new();
        let bot = bot(&fake, None);
        for _ in 0..MAX_REQUESTS {
            bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        }
        let limited = bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        assert!(
            limited.contains("too many") && limited.contains("15 minutes"),
            "{limited}"
        );
        assert_eq!(fake.outcomes().last().unwrap(), &json!({"outcome": "rate_limited"}));
        assert!(
            bot.answer(&asks(Command::Key, 6, Some(GUILD)))
                .await
                .unwrap()
                .contains('`')
        );
        tokio::time::advance(WINDOW).await;
        assert!(
            bot.answer(&asks(Command::Key, 5, Some(GUILD)))
                .await
                .unwrap()
                .contains('`')
        );
    }

    #[tokio::test(start_paused = true)]
    async fn key_sends_the_signup_link_when_signups_are_on() {
        let fake = Fake::new();
        let bot = bot(&fake, Some("https://dash.example/signup"));
        let text = bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        assert!(text.contains("https://dash.example/signup"), "{text}");
        assert_eq!(fake.invites.load(Ordering::SeqCst), 0, "no invite is made");
        assert_eq!(fake.outcomes(), vec![json!({"outcome": "signup_link"})]);
    }

    #[tokio::test(start_paused = true)]
    async fn key_and_status_say_why_the_game_is_down() {
        let fake = Fake::new();
        let bot = bot(&fake, None);
        assert!(
            bot.answer(&asks(Command::Status, 5, Some(GUILD)))
                .await
                .unwrap()
                .contains(": 4.")
        );
        *lock(&fake.up) = Err(UnavailableReason::NoHub);
        let text = bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        assert!(text.contains("isn't running a hub"), "{text}");
        let status = bot.answer(&asks(Command::Status, 5, Some(GUILD))).await.unwrap();
        assert_eq!(status, text);
        *lock(&fake.up) = Ok(());
        *lock(&fake.recheck_down) = true;
        let text = bot.answer(&asks(Command::Key, 5, Some(GUILD))).await.unwrap();
        assert!(text.contains("isn't ready yet"), "{text}");
        assert_eq!(
            fake.outcomes(),
            vec![
                json!({"outcome": "unavailable", "reason": "no_hub"}),
                json!({"outcome": "unavailable", "reason": "hub_not_ready"}),
            ]
        );
        assert_eq!(fake.invites.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn answers_only_in_its_guilds() {
        let fake = Fake::new();
        let bot = bot(&fake, None);
        for guild in [None, Some(GUILD + 1)] {
            let text = bot.answer(&asks(Command::Key, 5, guild)).await.unwrap();
            assert!(text.contains("only in"), "{text}");
        }
        assert!(fake.outcomes().is_empty());
        let mut unknown = asks(Command::Key, 5, Some(GUILD));
        unknown.kind = InteractionKind::UnknownCommand;
        assert!(bot.answer(&unknown).await.unwrap().contains("isn't available"));
        unknown.kind = InteractionKind::Other;
        assert_eq!(bot.answer(&unknown).await, None);
    }

    #[test]
    fn the_event_name_is_valid() {
        assert_eq!(
            EventName::new(KEY_REQUESTED_EVENT).unwrap().as_str(),
            KEY_REQUESTED_EVENT
        );
    }
}
