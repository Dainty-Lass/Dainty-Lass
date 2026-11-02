// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! CLEAN-ROOM STUB (RD27): stands in for the Category A `history.rs` (chat's database on `store`), which the
//! integration pass copies in unchanged. This one keeps everything in memory so the rewrite's own tests run; its
//! API is the rewrite's reading of spec-rd27-chat §1.4, §2.3, §2.5, §2.11, §2.13 and §3.2. `History::memory` is
//! the stub's alone; tests that call it go with it.

use std::sync::{Arc, Mutex, PoisonError};

use module_api::{GlobalId, ModuleId};
use serde::Serialize;
use store::Actor;

/// A mute as stored: `until` none is for good.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mute {
    pub account: String,
    pub until: Option<i64>,
    pub reason: Option<String>,
    pub by: String,
    pub at: i64,
}

impl Mute {
    pub fn holds(&self, now: i64) -> bool {
        self.until.is_none_or(|until| now < until)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectMessage {
    pub id: i64,
    pub sender: String,
    pub sender_local: String,
    pub recipient: String,
    pub body: String,
    pub stanza_id: Option<String>,
    pub at: i64,
    pub delivered_at: Option<i64>,
    pub mid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomMessage {
    pub id: i64,
    pub room: String,
    pub sender: String,
    pub nick: String,
    pub body: String,
    pub stanza_id: Option<String>,
    pub at: i64,
    pub mid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDirect {
    pub sender: GlobalId,
    pub sender_local: String,
    pub recipient: GlobalId,
    pub body: String,
    pub stanza_id: Option<String>,
    pub at: i64,
    pub delivered_at: Option<i64>,
    pub mid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRoom {
    pub room: String,
    pub sender: GlobalId,
    pub nick: String,
    pub body: String,
    pub stanza_id: Option<String>,
    pub mid: Option<String>,
    pub at: i64,
}

#[derive(Default)]
struct Tables {
    accounts: Vec<(String, String)>,
    direct: Vec<DirectMessage>,
    room: Vec<RoomMessage>,
    mutes: Vec<Mute>,
}

#[derive(Clone, Default)]
pub struct History {
    tables: Arc<Mutex<Tables>>,
}

type Result<T> = std::result::Result<T, store::Error>;

impl History {
    pub fn memory() -> Self {
        Self::default()
    }

    /// The real one opens and migrates `chat.db` (or the `chat` schema); the stub keeps it in memory.
    pub async fn open(_config: store::DatabaseConfig) -> Result<Self> {
        Ok(Self::default())
    }

    fn tables(&self) -> std::sync::MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub async fn remember_account(
        &self,
        local: &str,
        account: &GlobalId,
        _module: &ModuleId,
        _domain: &str,
        _at: i64,
    ) -> Result<()> {
        let mut tables = self.tables();
        tables.accounts.retain(|(l, _)| l != local);
        tables.accounts.push((local.to_owned(), account.to_string()));
        Ok(())
    }

    pub async fn account_of(&self, local: &str) -> Result<Option<String>> {
        Ok(self
            .tables()
            .accounts
            .iter()
            .find(|(l, _)| l == local)
            .map(|(_, a)| a.clone()))
    }

    pub async fn mute_of(&self, account: &str) -> Result<Option<Mute>> {
        Ok(self.tables().mutes.iter().find(|m| m.account == account).cloned())
    }

    pub async fn set_mute(&self, mute: Mute, _actor: Actor) -> Result<()> {
        let mut tables = self.tables();
        tables.mutes.retain(|m| m.account != mute.account);
        tables.mutes.push(mute);
        Ok(())
    }

    pub async fn clear_mute(&self, account: &str, _actor: Actor) -> Result<bool> {
        let mut tables = self.tables();
        let before = tables.mutes.len();
        tables.mutes.retain(|m| m.account != account);
        Ok(tables.mutes.len() != before)
    }

    pub async fn pending_for(&self, account: &GlobalId, limit: i64) -> Result<Vec<DirectMessage>> {
        let account = account.to_string();
        let limit = usize::try_from(limit).unwrap_or(0);
        Ok(self
            .tables()
            .direct
            .iter()
            .filter(|m| m.recipient == account && m.delivered_at.is_none())
            .take(limit)
            .cloned()
            .collect())
    }

    pub async fn mark_delivered(&self, account: &GlobalId, ids: &[i64], at: i64) -> Result<()> {
        let account = account.to_string();
        for m in &mut self.tables().direct {
            if m.recipient == account && ids.contains(&m.id) && m.delivered_at.is_none() {
                m.delivered_at = Some(at);
            }
        }
        Ok(())
    }

    pub async fn add_direct_once(&self, message: NewDirect) -> Result<()> {
        let mut tables = self.tables();
        if message.mid.is_some() && tables.direct.iter().any(|m| m.mid == message.mid) {
            return Ok(());
        }
        let id = i64::try_from(tables.direct.len()).unwrap_or(i64::MAX) + 1;
        tables.direct.push(DirectMessage {
            id,
            sender: message.sender.to_string(),
            sender_local: message.sender_local,
            recipient: message.recipient.to_string(),
            body: message.body,
            stanza_id: message.stanza_id,
            at: message.at,
            delivered_at: message.delivered_at,
            mid: message.mid,
        });
        Ok(())
    }

    pub async fn add_room_once(&self, message: NewRoom) -> Result<()> {
        let mut tables = self.tables();
        if message.mid.is_some()
            && tables
                .room
                .iter()
                .any(|m| m.room == message.room && m.mid == message.mid)
        {
            return Ok(());
        }
        let id = i64::try_from(tables.room.len()).unwrap_or(i64::MAX) + 1;
        tables.room.push(RoomMessage {
            id,
            room: message.room,
            sender: message.sender.to_string(),
            nick: message.nick,
            body: message.body,
            stanza_id: message.stanza_id,
            at: message.at,
            mid: message.mid,
        });
        Ok(())
    }

    pub async fn room_history(&self, room: &str, limit: i64) -> Result<Vec<RoomMessage>> {
        let tables = self.tables();
        let rows: Vec<_> = tables.room.iter().filter(|m| m.room == room).cloned().collect();
        let skip = rows.len().saturating_sub(usize::try_from(limit).unwrap_or(0));
        Ok(rows.into_iter().skip(skip).collect())
    }

    pub async fn direct_between(&self, a: &GlobalId, b: &GlobalId, limit: i64) -> Result<Vec<DirectMessage>> {
        let (a, b) = (a.to_string(), b.to_string());
        let tables = self.tables();
        let rows: Vec<_> = tables
            .direct
            .iter()
            .filter(|m| (m.sender == a && m.recipient == b) || (m.sender == b && m.recipient == a))
            .cloned()
            .collect();
        let skip = rows.len().saturating_sub(usize::try_from(limit).unwrap_or(0));
        Ok(rows.into_iter().skip(skip).collect())
    }
}
