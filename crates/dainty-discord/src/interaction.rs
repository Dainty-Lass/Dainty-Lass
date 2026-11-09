// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What Discord's gateway sends, parsed into the little the bot acts on. Everything here is untrusted input until
//! it parses (fuzzed by `fuzz/fuzz_targets/discord_interaction.rs`); an interaction's token is a credential for
//! its reply and is never shown by `Debug`.

use std::fmt;

use serde::Deserialize;
use serde_json::Value;

/// The largest gateway message read; anything bigger is a protocol error.
pub const MAX_FRAME: usize = 1024 * 1024;
/// The longest interaction token taken; it goes into a URL path.
pub const MAX_INTERACTION_TOKEN: usize = 512;
/// The longest session id or command name kept.
const MAX_SHORT: usize = 128;

/// A Discord id: a decimal `u64`, never 0.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(u64);

impl Snowflake {
    /// `text` as an id: 1 to 20 ASCII digits, not 0.
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.parse::<u64>().ok().filter(|n| *n != 0).map(Self)
    }

    /// The id as a number.
    pub fn get(self) -> u64 {
        self.0
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

/// Why a message from the gateway was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Larger than [`MAX_FRAME`].
    TooLarge,
    /// Not the JSON a gateway message is.
    Malformed,
    /// A field the bot needs is missing or out of range.
    Field(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("a gateway message larger than the limit"),
            Self::Malformed => f.write_str("a gateway message that is not JSON"),
            Self::Field(name) => write!(f, "a gateway message with a bad {name}"),
        }
    }
}

/// What the gateway said, in the bot's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayEvent {
    /// Opcode 10: heartbeat every `interval_ms`.
    Hello {
        /// Discord's interval.
        interval_ms: u64,
    },
    /// Opcode 11.
    HeartbeatAck,
    /// Opcode 1: Discord asks for a heartbeat now.
    HeartbeatRequest,
    /// Opcode 7: reconnect and resume.
    Reconnect,
    /// Opcode 9: the session is gone unless `resumable`.
    InvalidSession {
        /// Whether a resume may still work.
        resumable: bool,
    },
    /// `READY`: the session to resume, and whose application this is.
    Ready {
        /// The session id.
        session_id: String,
        /// Where a resume connects.
        resume_url: Option<String>,
        /// The token's application.
        application_id: Option<Snowflake>,
    },
    /// `RESUMED`.
    Resumed,
    /// `INTERACTION_CREATE`.
    Interaction(Interaction),
    /// An `INTERACTION_CREATE` the bot can't read; the reason only, never the payload.
    BadInteraction(ParseError),
    /// Anything else.
    Other,
}

#[derive(Deserialize)]
struct RawFrame {
    op: u8,
    #[serde(default)]
    d: Value,
    #[serde(default)]
    s: Option<u64>,
    #[serde(default)]
    t: Option<String>,
}

/// One gateway message: its sequence number (for heartbeats and resume) and what it says.
pub fn parse_frame(bytes: &[u8]) -> Result<(Option<u64>, GatewayEvent), ParseError> {
    if bytes.len() > MAX_FRAME {
        return Err(ParseError::TooLarge);
    }
    let frame: RawFrame = serde_json::from_slice(bytes).map_err(|_| ParseError::Malformed)?;
    let event = match frame.op {
        0 => dispatch(frame.t.as_deref(), &frame.d)?,
        1 => GatewayEvent::HeartbeatRequest,
        7 => GatewayEvent::Reconnect,
        9 => GatewayEvent::InvalidSession {
            resumable: frame.d.as_bool().unwrap_or(false),
        },
        10 => {
            let interval_ms = frame
                .d
                .get("heartbeat_interval")
                .and_then(Value::as_u64)
                .filter(|ms| (1_000..=600_000).contains(ms))
                .ok_or(ParseError::Field("heartbeat_interval"))?;
            GatewayEvent::Hello { interval_ms }
        }
        11 => GatewayEvent::HeartbeatAck,
        _ => GatewayEvent::Other,
    };
    Ok((frame.s, event))
}

fn dispatch(name: Option<&str>, d: &Value) -> Result<GatewayEvent, ParseError> {
    Ok(match name {
        Some("READY") => {
            let session_id = short(d.get("session_id")).ok_or(ParseError::Field("session_id"))?;
            let resume_url = d
                .get("resume_gateway_url")
                .and_then(Value::as_str)
                .filter(|u| u.len() <= MAX_SHORT && u.starts_with("wss://"))
                .map(str::to_owned);
            let application_id = d
                .get("application")
                .and_then(|a| a.get("id"))
                .and_then(Value::as_str)
                .and_then(Snowflake::parse);
            GatewayEvent::Ready {
                session_id,
                resume_url,
                application_id,
            }
        }
        Some("RESUMED") => GatewayEvent::Resumed,
        Some("INTERACTION_CREATE") => match parse_interaction(d) {
            Ok(interaction) => GatewayEvent::Interaction(interaction),
            Err(error) => GatewayEvent::BadInteraction(error),
        },
        _ => GatewayEvent::Other,
    })
}

fn short(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= MAX_SHORT && s.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_owned)
}

fn id(value: &Value, name: &'static str) -> Result<Snowflake, ParseError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .and_then(Snowflake::parse)
        .ok_or(ParseError::Field(name))
}

/// An `INTERACTION_CREATE`'s data.
pub fn parse_interaction(d: &Value) -> Result<Interaction, ParseError> {
    let kind = match d.get("type").and_then(Value::as_u64) {
        Some(1) => InteractionKind::Ping,
        Some(2) => {
            let name = d
                .get("data")
                .and_then(|data| data.get("name"))
                .and_then(Value::as_str)
                .filter(|n| n.len() <= MAX_SHORT)
                .ok_or(ParseError::Field("data.name"))?;
            Command::from_name(name).map_or(InteractionKind::UnknownCommand, InteractionKind::Command)
        }
        Some(_) => InteractionKind::Other,
        None => return Err(ParseError::Field("type")),
    };
    let token = d
        .get("token")
        .and_then(Value::as_str)
        .and_then(InteractionToken::parse)
        .ok_or(ParseError::Field("token"))?;
    let guild_id = match d.get("guild_id") {
        None | Some(Value::Null) => None,
        Some(_) => Some(id(d, "guild_id")?),
    };
    // A guild interaction names its sender in member.user, a direct message in user
    let user = d
        .get("member")
        .and_then(|m| m.get("user"))
        .or_else(|| d.get("user"))
        .ok_or(ParseError::Field("user"))?;
    let locale = d.get("locale").and_then(Value::as_str).filter(|l| l.len() <= 16);
    Ok(Interaction {
        id: id(d, "id")?,
        application_id: id(d, "application_id")?,
        token,
        kind,
        guild_id,
        user_id: id(user, "id")?,
        language: Language::from_locale(locale),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::json;

    fn interaction(name: &str) -> Value {
        json!({
            "op": 0, "s": 42, "t": "INTERACTION_CREATE",
            "d": {
                "id": "1100000000000000001", "application_id": "1200000000000000002", "type": 2,
                "token": "aW50ZXJhY3Rpb24.dG9rZW4-_x", "guild_id": "1300000000000000003", "locale": "pt-BR",
                "member": {"user": {"id": "1400000000000000004", "username": "alex"}},
                "data": {"id": "9", "name": name, "type": 1}
            }
        })
    }

    #[test]
    fn reads_a_command() {
        let bytes = serde_json::to_vec(&interaction("key")).unwrap();
        let (seq, event) = parse_frame(&bytes).unwrap();
        assert_eq!(seq, Some(42));
        let GatewayEvent::Interaction(i) = event else {
            panic!("{event:?}")
        };
        assert_eq!(i.kind, InteractionKind::Command(Command::Key));
        assert_eq!(i.user_id, Snowflake::parse("1400000000000000004").unwrap());
        assert_eq!(i.guild_id, Snowflake::parse("1300000000000000003"));
        assert_eq!(i.language, Language::PtBr);
        assert!(!format!("{i:?}").contains("aW50"), "the token is never shown");

        let bytes = serde_json::to_vec(&interaction("player")).unwrap();
        let (_, event) = parse_frame(&bytes).unwrap();
        let GatewayEvent::Interaction(i) = event else {
            panic!("{event:?}")
        };
        assert_eq!(i.kind, InteractionKind::UnknownCommand);
    }

    #[test]
    fn reads_a_direct_message_and_the_gateway_opcodes() {
        let mut dm = interaction("status");
        let d = dm.get_mut("d").unwrap().as_object_mut().unwrap();
        d.remove("member");
        d.remove("guild_id");
        d.insert("user".into(), json!({"id": "15"}));
        d.insert("locale".into(), json!("en-US"));
        let (_, event) = parse_frame(&serde_json::to_vec(&dm).unwrap()).unwrap();
        let GatewayEvent::Interaction(i) = event else {
            panic!("{event:?}")
        };
        assert_eq!((i.guild_id, i.language), (None, Language::En));

        let hello = br#"{"op":10,"d":{"heartbeat_interval":41250}}"#;
        assert_eq!(
            parse_frame(hello).unwrap().1,
            GatewayEvent::Hello { interval_ms: 41250 }
        );
        assert_eq!(parse_frame(br#"{"op":11}"#).unwrap().1, GatewayEvent::HeartbeatAck);
        assert_eq!(
            parse_frame(br#"{"op":9,"d":true}"#).unwrap().1,
            GatewayEvent::InvalidSession { resumable: true }
        );
        let ready = br#"{"op":0,"s":1,"t":"READY","d":{"session_id":"abc","resume_gateway_url":"wss://r.discord.gg","application":{"id":"77"}}}"#;
        assert_eq!(
            parse_frame(ready).unwrap().1,
            GatewayEvent::Ready {
                session_id: "abc".into(),
                resume_url: Some("wss://r.discord.gg".into()),
                application_id: Snowflake::parse("77"),
            }
        );
    }

    #[test]
    fn refuses_what_it_cannot_use() {
        assert_eq!(parse_frame(b"nope"), Err(ParseError::Malformed));
        assert_eq!(parse_frame(&vec![b' '; MAX_FRAME + 1]), Err(ParseError::TooLarge));
        assert_eq!(
            parse_frame(br#"{"op":10,"d":{"heartbeat_interval":0}}"#),
            Err(ParseError::Field("heartbeat_interval"))
        );
        for (field, value) in [
            ("token", json!("has/slash")),
            ("token", json!("x".repeat(MAX_INTERACTION_TOKEN + 1))),
            ("id", json!("0")),
            ("id", json!(11)),
            ("guild_id", json!("-3")),
            ("application_id", json!("1e9")),
        ] {
            let mut frame = interaction("key");
            frame["d"][field] = value;
            let (_, event) = parse_frame(&serde_json::to_vec(&frame).unwrap()).unwrap();
            assert!(matches!(event, GatewayEvent::BadInteraction(_)), "{field}: {event:?}");
        }
        assert_eq!(Snowflake::parse("18446744073709551616"), None);
        assert_eq!(Snowflake::parse("+1"), None);
    }
}
