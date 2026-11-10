// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The little of an interaction the bot acts on, taken from twilight's model. Everything here is untrusted input
//! until it converts (fuzzed by `fuzz/fuzz_targets/discord_interaction.rs`); an interaction's token is a credential
//! for its reply and is never shown by `Debug`.

use std::fmt;
use std::num::NonZeroU64;

use twilight_model::application::interaction::{self as model, InteractionData, InteractionType};
use twilight_model::id::Id;

/// The longest interaction token taken; it goes into a URL path.
pub const MAX_INTERACTION_TOKEN: usize = 512;
/// The longest command name kept.
const MAX_SHORT: usize = 128;

/// A Discord id: a decimal `u64`, never 0.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(NonZeroU64);

impl Snowflake {
    /// `text` as an id: 1 to 20 ASCII digits, not 0.
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.parse::<NonZeroU64>().ok().map(Self)
    }

    /// The id as a number.
    pub fn get(self) -> u64 {
        self.0.get()
    }

    /// The id as twilight types it.
    pub fn id<T>(self) -> Id<T> {
        Id::from(self.0)
    }
}

impl<T> From<Id<T>> for Snowflake {
    fn from(id: Id<T>) -> Self {
        Self(id.into_nonzero())
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An interaction's token: it answers that interaction for 15 minutes, so it is never shown.
#[derive(Clone, PartialEq, Eq)]
pub struct InteractionToken(String);

impl InteractionToken {
    /// `text` as a token: `[A-Za-z0-9._-]`, at most [`MAX_INTERACTION_TOKEN`] bytes, so it is safe in a path.
    pub fn parse(text: &str) -> Option<Self> {
        let ok = !text.is_empty()
            && text.len() <= MAX_INTERACTION_TOKEN
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        ok.then(|| Self(text.to_owned()))
    }

    /// The token, for the reply's URL only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for InteractionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InteractionToken(..)")
    }
}

/// The catalog an interaction is answered from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// English, for every locale but Portuguese.
    En,
    /// Brazilian Portuguese, for `pt-BR` (Discord has no other Portuguese).
    PtBr,
}

impl Language {
    /// The language for Discord's `locale`.
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(l) if l.eq_ignore_ascii_case("pt-BR") || l.eq_ignore_ascii_case("pt") => Self::PtBr,
            _ => Self::En,
        }
    }
}

/// The commands the bot registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    /// `/key`.
    Key,
    /// `/status`.
    Status,
}

impl Command {
    /// Every command, in the order registered.
    pub const ALL: [Self; 2] = [Self::Key, Self::Status];

    /// The name Discord shows after the slash.
    pub fn name(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Status => "status",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// What an interaction asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionKind {
    /// Discord's check that the bot answers.
    Ping,
    /// One of the bot's commands.
    Command(Command),
    /// A command the bot doesn't know (a stale registration).
    UnknownCommand,
    /// Anything else (components, autocomplete): the bot registers none, so it ignores them.
    Other,
}

/// An `INTERACTION_CREATE`, as far as the bot reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interaction {
    /// Its id, for the reply.
    pub id: Snowflake,
    /// The application it was sent to.
    pub application_id: Snowflake,
    /// The reply's credential.
    pub token: InteractionToken,
    /// What it asks.
    pub kind: InteractionKind,
    /// The guild it came from; `None` in a direct message.
    pub guild_id: Option<Snowflake>,
    /// The member or user who sent it.
    pub user_id: Snowflake,
    /// The language to answer in.
    pub language: Language,
}

/// Why an interaction was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Not the JSON an interaction is.
    Malformed,
    /// A field the bot needs is missing or out of range.
    Field(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => f.write_str("an interaction that is not JSON"),
            Self::Field(name) => write!(f, "an interaction with a bad {name}"),
        }
    }
}

/// An `INTERACTION_CREATE`'s body, as Discord sends it; what the gateway hands the bot is the same model.
pub fn parse_interaction(bytes: &[u8]) -> Result<Interaction, ParseError> {
    let model: model::Interaction = serde_json::from_slice(bytes).map_err(|_| ParseError::Malformed)?;
    Interaction::from_model(&model)
}

impl Interaction {
    /// The little of twilight's interaction the bot acts on.
    pub fn from_model(d: &model::Interaction) -> Result<Self, ParseError> {
        let kind = match d.kind {
            InteractionType::Ping => InteractionKind::Ping,
            InteractionType::ApplicationCommand => match &d.data {
                Some(InteractionData::ApplicationCommand(data)) if data.name.len() <= MAX_SHORT => {
                    Command::from_name(&data.name).map_or(InteractionKind::UnknownCommand, InteractionKind::Command)
                }
                _ => return Err(ParseError::Field("data.name")),
            },
            _ => InteractionKind::Other,
        };
        let token = InteractionToken::parse(&d.token).ok_or(ParseError::Field("token"))?;
        // A guild interaction names its sender in member.user, a direct message in user
        let user_id = d.author_id().ok_or(ParseError::Field("user"))?;
        let locale = d.locale.as_deref().filter(|l| l.len() <= 16);
        Ok(Self {
            id: d.id.into(),
            application_id: d.application_id.into(),
            token,
            kind,
            guild_id: d.guild_id.map(Snowflake::from),
            user_id: user_id.into(),
            language: Language::from_locale(locale),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::{Value, json};

    fn user(id: &str) -> Value {
        json!({"id": id, "username": "alex", "discriminator": "0", "avatar": null, "global_name": "Alex"})
    }

    /// An `INTERACTION_CREATE`'s `d` as Discord sends a slash command in a guild.
    fn interaction(name: &str) -> Value {
        json!({
            "id": "1100000000000000001", "application_id": "1200000000000000002", "type": 2,
            "token": "aW50ZXJhY3Rpb24.dG9rZW4-_x", "version": 1, "guild_id": "1300000000000000003",
            "locale": "pt-BR", "guild_locale": "en-US", "app_permissions": "0", "entitlements": [],
            "attachment_size_limit": 8_388_608, "authorizing_integration_owners": {"0": "1300000000000000003"},
            "context": 0, "channel_id": "1500000000000000005",
            "member": {
                "user": user("1400000000000000004"), "roles": [], "joined_at": "2026-01-01T00:00:00.000000+00:00",
                "deaf": false, "mute": false, "flags": 0, "permissions": "0", "nick": null, "avatar": null,
                "premium_since": null, "pending": false, "communication_disabled_until": null
            },
            "data": {"id": "9", "name": name, "type": 1}
        })
    }

    fn parse(value: &Value) -> Result<Interaction, ParseError> {
        parse_interaction(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn reads_a_command() {
        let i = parse(&interaction("key")).unwrap();
        assert_eq!(i.kind, InteractionKind::Command(Command::Key));
        assert_eq!(i.user_id, Snowflake::parse("1400000000000000004").unwrap());
        assert_eq!(i.guild_id, Snowflake::parse("1300000000000000003"));
        assert_eq!(i.language, Language::PtBr);
        assert!(!format!("{i:?}").contains("aW50"), "the token is never shown");

        assert_eq!(
            parse(&interaction("player")).unwrap().kind,
            InteractionKind::UnknownCommand
        );
    }

    #[test]
    fn reads_a_direct_message() {
        let mut dm = interaction("status");
        let d = dm.as_object_mut().unwrap();
        d.remove("member");
        d.remove("guild_id");
        d.insert("user".into(), user("15"));
        d.insert("locale".into(), json!("en-US"));
        d.insert("context".into(), json!(1));
        let i = parse(&dm).unwrap();
        assert_eq!((i.guild_id, i.language, i.user_id.get()), (None, Language::En, 15));
    }

    #[test]
    fn refuses_what_it_cannot_use() {
        assert_eq!(parse_interaction(b"nope"), Err(ParseError::Malformed));
        for (field, value) in [
            ("token", json!("has/slash")),
            ("token", json!("x".repeat(MAX_INTERACTION_TOKEN + 1))),
            ("id", json!("0")),
            ("guild_id", json!("-3")),
            ("application_id", json!("1e9")),
        ] {
            let mut frame = interaction("key");
            frame[field] = value;
            assert!(parse(&frame).is_err(), "{field}");
        }
        let mut anonymous = interaction("key");
        anonymous.as_object_mut().unwrap().remove("member");
        assert_eq!(parse(&anonymous), Err(ParseError::Field("user")));
        assert_eq!(Snowflake::parse("18446744073709551616"), None);
        assert_eq!(Snowflake::parse("+1"), None);
    }
}
