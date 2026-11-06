//! Who made a change (DH1, DD23): every command and admin action reaches a module with the actor that
//! asked, so the change log `store` writes can name them and `revert --actor` can find them. A stub until
//! DF7 with DH1 gives `store` the log and DH2 the per-person admin keys.

use serde::{Deserialize, Serialize};

use crate::ids::ModuleId;
use crate::network::GlobalId;

/// The most bytes an actor's free-text parts (a key's name, a request id) may hold.
pub const MAX_ACTOR_TEXT: usize = 128;

/// Who asked for a change.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Actor {
    /// The server itself: a timer, a startup task, a retention purge
    System,
    /// Whoever has the host's terminal (the console and TUI have no login, by design)
    Console,
    /// A person with an admin key (DD23's one key per person; `key` is the key's name, never the key)
    Admin {
        /// The admin key's name.
        key: String,
    },
    /// A player, acting on their own account
    Player {
        /// The player's account.
        account: GlobalId,
    },
    /// Another module, through an event it handled
    Module {
        /// The module that handled the event.
        module: ModuleId,
    },
}

/// The actor and the request that carried them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorContext {
    /// Who asked.
    pub actor: Actor,
    /// The request id the gateway or admin API logged, to tie a log entry to its request
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
}

impl ActorContext {
    /// `actor`, with no request id.
    pub fn new(actor: Actor) -> Self {
        Self { actor, request: None }
    }

    /// Ties the context to the request id the gateway or admin API logged.
    pub fn with_request(mut self, request: impl Into<String>) -> Self {
        self.request = Some(request.into());
        self
    }

    /// Whether the free-text parts fit, so a log entry stays small and holds no line breaks.
    pub fn is_valid(&self) -> bool {
        let fits = |text: &str| text.len() <= MAX_ACTOR_TEXT && !text.chars().any(char::is_control);
        let key_fits = match &self.actor {
            Actor::Admin { key } => !key.is_empty() && fits(key),
            _ => true,
        };
        key_fits && self.request.as_deref().is_none_or(fits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actors_serialise_and_check_their_text() {
        let admin = ActorContext::new(Actor::Admin {
            key: String::from("nick"),
        })
        .with_request("req-1");
        assert!(admin.is_valid());
        let json = serde_json::to_string(&admin).unwrap();
        assert_eq!(json, r#"{"actor":{"admin":{"key":"nick"}},"request":"req-1"}"#);
        assert_eq!(serde_json::from_str::<ActorContext>(&json).unwrap(), admin);
        assert_eq!(
            serde_json::to_string(&ActorContext::new(Actor::Console)).unwrap(),
            r#"{"actor":"console"}"#
        );
        assert!(!ActorContext::new(Actor::Admin { key: String::new() }).is_valid());
        assert!(!ActorContext::new(Actor::System).with_request("a\nb").is_valid());
        assert!(
            !ActorContext::new(Actor::System)
                .with_request("x".repeat(MAX_ACTOR_TEXT + 1))
                .is_valid()
        );
    }
}
