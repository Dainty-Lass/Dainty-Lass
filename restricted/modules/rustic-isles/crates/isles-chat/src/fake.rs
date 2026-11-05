//! A game in memory for the rules' tests.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use module_api::{BoxFuture, ErrorKind, GlobalId, IdKind, ModuleError, NodeIdentity};

use crate::source::{ChatSettings, ChatSource, Player};

pub struct Fake {
    pub identity: NodeIdentity,
    pub settings: ChatSettings,
    players: Mutex<Vec<Player>>,
    parties: Mutex<BTreeMap<GlobalId, String>>,
    friends: Mutex<BTreeMap<GlobalId, Vec<GlobalId>>>,
    party_down: Mutex<bool>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            identity: NodeIdentity::standalone(),
            settings: ChatSettings::default(),
            players: Mutex::default(),
            parties: Mutex::default(),
            friends: Mutex::default(),
            party_down: Mutex::default(),
        }
    }
}

impl Fake {
    pub fn add(&self, user_id: &str, name: &str) -> GlobalId {
        let account = GlobalId::new(IdKind::Account, &self.identity.node);
        self.players
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Player {
                account: account.clone(),
                user_id: user_id.into(),
                name: name.into(),
            });
        account
    }

    pub fn put_party(&self, account: &GlobalId, party_id: &str) {
        self.parties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.clone(), party_id.into());
    }

    pub fn fail_party_lookup(&self) {
        *self.party_down.lock().unwrap_or_else(PoisonError::into_inner) = true;
    }
}

impl ChatSource for Fake {
    fn node(&self) -> &NodeIdentity {
        &self.identity
    }

    fn settings(&self) -> ChatSettings {
        self.settings
    }

    fn player<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            let players = self.players.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(players.iter().find(|p| &p.account == account).cloned())
        })
    }

    fn player_by_user<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            let players = self.players.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(players.iter().find(|p| p.user_id == user_id).cloned())
        })
    }

    fn party_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<String>, ModuleError>> {
        Box::pin(async move {
            if *self.party_down.lock().unwrap_or_else(PoisonError::into_inner) {
                return Err(ModuleError::new(ErrorKind::Unavailable, "parties are down"));
            }
            Ok(self
                .parties
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(account)
                .cloned())
        })
    }

    fn friends<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>> {
        Box::pin(async move {
            Ok(self
                .friends
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(account)
                .cloned()
                .unwrap_or_default())
        })
    }
}
