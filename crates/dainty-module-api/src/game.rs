//! What a game module knows about its own game (MS1, the module-sharing plan's A1): whether players can get
//! in, the game servers it runs, and invites. Asked through the registry like `admin` and `chat`, so the
//! same call reaches a compiled-in module and a child.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::module::{ErrorKind, ModuleError};

/// Event a game module emits when one of its game servers comes up (`Payload`: [`GameServer`]).
pub const SERVER_UP_EVENT: &str = "game.server.up";

/// Event a game module emits when one of its game servers goes away (`Payload`: [`GameServer`]).
pub const SERVER_DOWN_EVENT: &str = "game.server.down";

/// Event a game module emits when its [`Availability`] changes (`Payload`: [`Availability`]).
pub const AVAILABILITY_CHANGED_EVENT: &str = "game.availability.changed";

/// What the host asks a game module about its game.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GameCall {
    /// Whether players can get in now.
    Availability,
    /// Every game server the module runs.
    Servers,
    /// Makes an invite code.
    CreateInvite {
        /// How many logins it admits, at least 1.
        uses: u32,
        /// None never expires.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_in_secs: Option<u32>,
        /// Words kept with the invite for staff.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        /// The module rechecks availability inside the write and refuses with [`UnavailableReason::error`]:
        /// the recheck, not the caller's earlier `Availability`, is the guarantee
        #[serde(default)]
        require_up: bool,
    },
    /// Revokes an invite code; [`ErrorKind::NotFound`] when there is none.
    RevokeInvite {
        /// The code.
        code: String,
    },
}

impl GameCall {
    /// The call's [`GameCallKind`].
    pub fn kind(&self) -> GameCallKind {
        match self {
            Self::Availability => GameCallKind::Availability,
            Self::Servers => GameCallKind::Servers,
            Self::CreateInvite { .. } => GameCallKind::CreateInvite,
            Self::RevokeInvite { .. } => GameCallKind::RevokeInvite,
        }
    }
}

/// Which [`GameCall`]s a module answers, as it registers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameCallKind {
    /// [`GameCall::Availability`].
    Availability,
    /// [`GameCall::Servers`].
    Servers,
    /// [`GameCall::CreateInvite`].
    CreateInvite,
    /// [`GameCall::RevokeInvite`].
    RevokeInvite,
}

/// A game module's answer to a [`GameCall`], the variant matching the call's.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GameAnswer {
    /// Whether players can get in.
    Availability(Availability),
    /// The game servers.
    Servers(Vec<GameServer>),
    /// The invite made.
    Invite {
        /// Its code: a credential, never logged or put in an event.
        code: String,
    },
    /// The call was carried out.
    Done,
}

// An invite code is a credential: Debug, which error messages and logs use, never shows one
impl fmt::Debug for GameCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Availability => f.write_str("Availability"),
            Self::Servers => f.write_str("Servers"),
            Self::CreateInvite {
                uses,
                expires_in_secs,
                note,
                require_up,
            } => f
                .debug_struct("CreateInvite")
                .field("uses", uses)
                .field("expires_in_secs", expires_in_secs)
                .field("note", note)
                .field("require_up", require_up)
                .finish(),
            Self::RevokeInvite { .. } => f.debug_struct("RevokeInvite").finish_non_exhaustive(),
        }
    }
}

impl fmt::Debug for GameAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Availability(availability) => f.debug_tuple("Availability").field(availability).finish(),
            Self::Servers(servers) => f.debug_tuple("Servers").field(servers).finish(),
            Self::Invite { .. } => f.debug_struct("Invite").finish_non_exhaustive(),
            Self::Done => f.write_str("Done"),
        }
    }
}

/// Whether players can get in now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Availability {
    /// The hub is running and ready.
    pub up: bool,
    /// Why not, when not `up`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<UnavailableReason>,
    /// The hub, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub: Option<GameServer>,
    /// Informational; never gates `up`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_hunts: Option<u32>,
}

impl Availability {
    /// Up, with its hub.
    pub fn up(hub: GameServer) -> Self {
        Self {
            up: true,
            reason: None,
            hub: Some(hub),
            free_hunts: None,
        }
    }

    /// Down for `reason`.
    pub fn down(reason: UnavailableReason) -> Self {
        Self {
            up: false,
            reason: Some(reason),
            hub: None,
            free_hunts: None,
        }
    }
}

/// Why players can't get in: a code the reader translates, never a sentence (DU2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The game's Director did not answer.
    DirectorUnreachable,
    /// The Director runs no hub.
    NoHub,
    /// The hub is there and not ready yet.
    HubNotReady,
    /// The module itself is not started.
    ModuleNotReady,
}

impl UnavailableReason {
    const ALL: [Self; 4] = [
        Self::DirectorUnreachable,
        Self::NoHub,
        Self::HubNotReady,
        Self::ModuleNotReady,
    ];

    /// The code, as it is serialised.
    pub fn code(self) -> &'static str {
        match self {
            Self::DirectorUnreachable => "director_unreachable",
            Self::NoHub => "no_hub",
            Self::HubNotReady => "hub_not_ready",
            Self::ModuleNotReady => "module_not_ready",
        }
    }

    /// The reason as an [`ErrorKind::Unavailable`] error, so it crosses the pipe as a code.
    pub fn error(self) -> ModuleError {
        ModuleError::new(ErrorKind::Unavailable, self.code())
    }

    /// The reason a [`Self::error`] carries; none for any other error.
    pub fn from_error(error: &ModuleError) -> Option<Self> {
        if error.kind != ErrorKind::Unavailable {
            return None;
        }
        Self::ALL.into_iter().find(|reason| reason.code() == error.message)
    }
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// What a game server is for: the Director's instance kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameServerKind {
    /// The city players gather in (Ramsgate).
    Hub,
    /// A hunt.
    Hunt,
    /// The dojo.
    Dojo,
    /// The tutorial.
    Tutorial,
}

/// One game server a module's Director runs. Not a network `InstanceRecord`: no owner node, queue or shard
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameServer {
    /// The Director's id for it.
    pub id: String,
    /// What it is for.
    pub kind: GameServerKind,
    /// Name shown to staff.
    pub title: String,
    /// The map it runs.
    pub map: String,
    /// Where players connect.
    pub host: String,
    /// Its UDP port.
    pub port: u16,
    /// Players on it now.
    pub players: u32,
    /// Most players it takes.
    pub max_players: u32,
    /// Accepting players.
    pub ready: bool,
    /// RFC 3339
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::EventName;

    fn hub() -> GameServer {
        GameServer {
            id: String::from("hub-1"),
            kind: GameServerKind::Hub,
            title: String::from("Ramsgate"),
            map: String::from("Ramsgate"),
            host: String::from("127.0.0.1"),
            port: 7777,
            players: 3,
            max_players: 50,
            ready: true,
            started_at: None,
        }
    }

    #[test]
    fn calls_and_answers_round_trip_and_keep_their_wire_names() {
        let create = GameCall::CreateInvite {
            uses: 1,
            expires_in_secs: Some(1800),
            note: None,
            require_up: true,
        };
        assert_eq!(
            serde_json::to_string(&create).unwrap(),
            r#"{"create_invite":{"uses":1,"expires_in_secs":1800,"require_up":true}}"#
        );
        assert_eq!(
            serde_json::to_string(&GameCall::Availability).unwrap(),
            r#""availability""#
        );
        for call in [
            GameCall::Availability,
            GameCall::Servers,
            create,
            GameCall::RevokeInvite {
                code: String::from("c"),
            },
        ] {
            let json = serde_json::to_string(&call).unwrap();
            assert_eq!(serde_json::from_str::<GameCall>(&json).unwrap(), call);
        }
        for answer in [
            GameAnswer::Availability(Availability::up(hub())),
            GameAnswer::Availability(Availability::down(UnavailableReason::HubNotReady)),
            GameAnswer::Servers(vec![hub()]),
            GameAnswer::Invite {
                code: String::from("c"),
            },
            GameAnswer::Done,
        ] {
            let json = serde_json::to_string(&answer).unwrap();
            assert_eq!(serde_json::from_str::<GameAnswer>(&json).unwrap(), answer);
        }
        let down = serde_json::to_string(&Availability::down(UnavailableReason::DirectorUnreachable)).unwrap();
        assert_eq!(down, r#"{"up":false,"reason":"director_unreachable"}"#);
        assert!(serde_json::from_str::<GameCall>(r#"{"revoke_invite":{"code":"c","x":1}}"#).is_err());
        assert!(serde_json::from_str::<Availability>(r#"{"up":false,"reason":"because"}"#).is_err());
    }

    #[test]
    fn invite_codes_never_show_in_debug() {
        let answer = format!(
            "{:?}",
            GameAnswer::Invite {
                code: String::from("SECRET1")
            }
        );
        let call = format!(
            "{:?}",
            GameCall::RevokeInvite {
                code: String::from("SECRET1")
            }
        );
        assert!(
            !answer.contains("SECRET1") && !call.contains("SECRET1"),
            "{answer} {call}"
        );
    }

    #[test]
    fn reasons_are_codes_and_cross_as_errors() {
        for reason in UnavailableReason::ALL {
            assert_eq!(
                serde_json::to_string(&reason).unwrap(),
                format!("\"{}\"", reason.code())
            );
            assert_eq!(UnavailableReason::from_error(&reason.error()), Some(reason));
        }
        assert_eq!(UnavailableReason::from_error(&ModuleError::failed("no_hub")), None);
        let words = ModuleError::new(ErrorKind::Unavailable, "gone");
        assert_eq!(UnavailableReason::from_error(&words), None);
    }

    #[test]
    fn event_names_are_valid() {
        for name in [SERVER_UP_EVENT, SERVER_DOWN_EVENT, AVAILABILITY_CHANGED_EVENT] {
            assert!(EventName::new(name).is_ok(), "{name}");
        }
    }
}
