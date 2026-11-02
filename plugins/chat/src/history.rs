// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! New here (DD15, DD21): chat's own database on `store`, apart from the game's: a SQLite file of its own, so
//! its writes don't queue behind the game's. DR's chat kept nothing (a whisper to someone offline was
//! dropped); this holds room history, direct messages until they are delivered, the JID each account last
//! logged in with, and mutes.
//!
//! Every write goes through `Database::write`, so the change log records it (DD23) under the player who
//! sent the message or the admin who set the mute.

use std::path::PathBuf;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use serde::{Deserialize, Serialize};
use store::{Actor, AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, Error, portable};

const SQLITE: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("migrations/postgres");

pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    soft_delete: &[],
};

/// The SQLite file's name beside the game's database.
pub const FILE_NAME: &str = "chat.db";

diesel::table! {
    chat_room_messages (id) {
        id -> BigInt,
        room -> Text,
        sender -> Text,
        nick -> Text,
        body -> Text,
        stanza_id -> Nullable<Text>,
        at -> BigInt,
        mid -> Nullable<Text>,
    }
}

diesel::table! {
    chat_direct_messages (id) {
        id -> BigInt,
        sender -> Text,
        sender_local -> Text,
        recipient -> Text,
        body -> Text,
        stanza_id -> Nullable<Text>,
        at -> BigInt,
        delivered_at -> Nullable<BigInt>,
        mid -> Nullable<Text>,
    }
}

diesel::table! {
    chat_outbox (id) {
        id -> BigInt,
        peer -> Text,
        call -> Text,
        tries -> BigInt,
        next_at -> BigInt,
        created_at -> BigInt,
    }
}

diesel::table! {
    chat_mutes (account) {
        account -> Text,
        until_at -> Nullable<BigInt>,
        reason -> Nullable<Text>,
        muted_by -> Text,
        at -> BigInt,
    }
}

diesel::table! {
    chat_accounts (jid_local) {
        jid_local -> Text,
        account -> Text,
        module -> Text,
        domain -> Text,
        seen_at -> BigInt,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Serialize, Deserialize)]
pub struct RoomMessage {
    pub id: i64,
    /// `naming::room_key`
    pub room: String,
    /// The sender's global account id
    pub sender: String,
    pub nick: String,
    pub body: String,
    pub stanza_id: Option<String>,
    /// Unix milliseconds
    pub at: i64,
    /// The id every node knows it by (DN10b); none for a message from before federation
    pub mid: Option<String>,
}

/// A room message to store, before it has an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRoom {
    pub room: String,
    pub sender: String,
    pub nick: String,
    pub body: String,
    pub stanza_id: Option<String>,
    pub mid: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Serialize, Deserialize)]
pub struct DirectMessage {
    pub id: i64,
    pub sender: String,
    pub sender_local: String,
    pub recipient: String,
    pub body: String,
    pub stanza_id: Option<String>,
    pub at: i64,
    /// When a session of the recipient took it; none while it waits for their next login
    pub delivered_at: Option<i64>,
    /// The id every node knows it by (DN10b)
    pub mid: Option<String>,
}

/// A direct message to store, before it has an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDirect {
    pub sender: String,
    pub sender_local: String,
    pub recipient: String,
    pub body: String,
    pub stanza_id: Option<String>,
    pub at: i64,
    pub delivered_at: Option<i64>,
    pub mid: Option<String>,
}

/// A call to another node that is retried until it is answered (DN10b).
#[derive(Debug, Clone, PartialEq, Eq, Queryable)]
pub struct OutboxRow {
    pub id: i64,
    pub peer: String,
    /// The call as JSON
    pub call: String,
    pub tries: i64,
    pub next_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Serialize, Deserialize)]
pub struct Mute {
    pub account: String,
    /// Unix milliseconds; none is until lifted
    pub until: Option<i64>,
    pub reason: Option<String>,
    /// The actor that set it, as the change log writes actors
    pub by: String,
    pub at: i64,
}

impl Mute {
    pub fn holds(&self, now: i64) -> bool {
        self.until.is_none_or(|until| until > now)
    }
}

/// Chat's database. Cloning is cheap and shares the pools.
#[derive(Clone)]
pub struct History {
    db: Database,
}

impl History {
    /// Opens (or creates) chat's database and brings its schema up to date. Blocking: call it at start.
    pub fn open(config: &DatabaseConfig) -> Result<Self, Error> {
        let options = DatabaseOptions {
            postgres_schema: Some(String::from("chat")),
            ..DatabaseOptions::default()
        };
        Ok(Self {
            db: Database::open(config, &options, &SCHEMA)?,
        })
    }

    /// Chat's database next to the game's: on SQLite `chat.db` in the same folder; on Postgres the `chat`
    /// schema in the same database.
    pub fn beside(game: &DatabaseConfig) -> DatabaseConfig {
        match game {
            DatabaseConfig::Sqlite { file } => DatabaseConfig::Sqlite {
                file: file
                    .parent()
                    .map_or_else(|| PathBuf::from(FILE_NAME), |dir| dir.join(FILE_NAME)),
            },
            DatabaseConfig::Postgres(pg) => DatabaseConfig::Postgres(pg.clone()),
        }
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    /// The account a JID local part last logged in as, so a direct message reaches someone offline.
    pub async fn remember_account(
        &self,
        local: &str,
        account: &str,
        module: &str,
        domain: &str,
        at: i64,
    ) -> Result<(), Error> {
        let row = (
            local.to_owned(),
            account.to_owned(),
            module.to_owned(),
            domain.to_owned(),
        );
        self.db
            .write(Actor::Player(account.to_owned()), move |tx| {
                let sql = portable::upsert(
                    "chat_accounts",
                    &["jid_local", "account", "module", "domain", "seen_at"],
                    &["jid_local"],
                    &["account", "module", "domain", "seen_at"],
                );
                diesel::sql_query(sql)
                    .bind::<Text, _>(row.0)
                    .bind::<Text, _>(row.1)
                    .bind::<Text, _>(row.2)
                    .bind::<Text, _>(row.3)
                    .bind::<BigInt, _>(at)
                    .execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    pub async fn account_of(&self, local: &str) -> Result<Option<String>, Error> {
        let local = local.to_owned();
        self.db
            .read(move |conn: &mut AnyConnection| {
                Ok(chat_accounts::table
                    .find(local)
                    .select(chat_accounts::account)
                    .first::<String>(conn)
                    .optional()?)
            })
            .await
    }

    pub async fn add_room_message(
        &self,
        room: &str,
        sender: &str,
        nick: &str,
        body: &str,
        stanza_id: Option<&str>,
        at: i64,
    ) -> Result<i64, Error> {
        self.add_room(NewRoom {
            room: room.to_owned(),
            sender: sender.to_owned(),
            nick: nick.to_owned(),
            body: body.to_owned(),
            stanza_id: stanza_id.map(str::to_owned),
            mid: None,
            at,
        })
        .await
    }

    pub async fn add_room(&self, message: NewRoom) -> Result<i64, Error> {
        let actor = Actor::Player(message.sender.clone());
        self.db
            .write(actor, move |tx| {
                Ok(diesel::insert_into(chat_room_messages::table)
                    .values((
                        chat_room_messages::room.eq(message.room),
                        chat_room_messages::sender.eq(message.sender),
                        chat_room_messages::nick.eq(message.nick),
                        chat_room_messages::body.eq(message.body),
                        chat_room_messages::stanza_id.eq(message.stanza_id),
                        chat_room_messages::at.eq(message.at),
                        chat_room_messages::mid.eq(message.mid),
                    ))
                    .returning(chat_room_messages::id)
                    .get_result(tx.conn())?)
            })
            .await
    }

    /// Stores a message another node sent unless this node already holds its `mid`: whether it was new (DN10b).
    pub async fn add_room_once(&self, message: NewRoom) -> Result<bool, Error> {
        let actor = Actor::System(String::from("chat-federation"));
        self.db
            .write(actor, move |tx| {
                let sql = portable::insert_ignore(
                    "chat_room_messages",
                    &["room", "sender", "nick", "body", "stanza_id", "at", "mid"],
                );
                let rows = diesel::sql_query(sql)
                    .bind::<Text, _>(message.room)
                    .bind::<Text, _>(message.sender)
                    .bind::<Text, _>(message.nick)
                    .bind::<Text, _>(message.body)
                    .bind::<diesel::sql_types::Nullable<Text>, _>(message.stanza_id)
                    .bind::<BigInt, _>(message.at)
                    .bind::<diesel::sql_types::Nullable<Text>, _>(message.mid)
                    .execute(tx.conn())?;
                Ok(rows > 0)
            })
            .await
    }

    /// The room's last `limit` messages, oldest first.
    pub async fn room_history(&self, room: &str, limit: i64) -> Result<Vec<RoomMessage>, Error> {
        let room = room.to_owned();
        self.db
            .read(move |conn: &mut AnyConnection| {
                let mut rows: Vec<RoomMessage> = chat_room_messages::table
                    .filter(chat_room_messages::room.eq(room))
                    .order((chat_room_messages::at.desc(), chat_room_messages::id.desc()))
                    .limit(limit)
                    .load(conn)?;
                rows.reverse();
                Ok(rows)
            })
            .await
    }

    pub async fn add_direct(&self, message: NewDirect) -> Result<i64, Error> {
        let actor = Actor::Player(message.sender.clone());
        self.db
            .write(actor, move |tx| {
                Ok(diesel::insert_into(chat_direct_messages::table)
                    .values((
                        chat_direct_messages::sender.eq(message.sender),
                        chat_direct_messages::sender_local.eq(message.sender_local),
                        chat_direct_messages::recipient.eq(message.recipient),
                        chat_direct_messages::body.eq(message.body),
                        chat_direct_messages::stanza_id.eq(message.stanza_id),
                        chat_direct_messages::at.eq(message.at),
                        chat_direct_messages::delivered_at.eq(message.delivered_at),
                        chat_direct_messages::mid.eq(message.mid),
                    ))
                    .returning(chat_direct_messages::id)
                    .get_result(tx.conn())?)
            })
            .await
    }

    /// Stores a direct message unless this node already holds its `mid`: whether it was new (DN10b).
    pub async fn add_direct_once(&self, message: NewDirect) -> Result<bool, Error> {
        let actor = Actor::Player(message.sender.clone());
        self.db
            .write(actor, move |tx| {
                let sql = portable::insert_ignore(
                    "chat_direct_messages",
                    &[
                        "sender",
                        "sender_local",
                        "recipient",
                        "body",
                        "stanza_id",
                        "at",
                        "delivered_at",
                        "mid",
                    ],
                );
                let rows = diesel::sql_query(sql)
                    .bind::<Text, _>(message.sender)
                    .bind::<Text, _>(message.sender_local)
                    .bind::<Text, _>(message.recipient)
                    .bind::<Text, _>(message.body)
                    .bind::<diesel::sql_types::Nullable<Text>, _>(message.stanza_id)
                    .bind::<BigInt, _>(message.at)
                    .bind::<diesel::sql_types::Nullable<BigInt>, _>(message.delivered_at)
                    .bind::<diesel::sql_types::Nullable<Text>, _>(message.mid)
                    .execute(tx.conn())?;
                Ok(rows > 0)
            })
            .await
    }

    pub async fn direct_by_mid(&self, mid: &str) -> Result<Option<DirectMessage>, Error> {
        let mid = mid.to_owned();
        self.db
            .read(move |conn: &mut AnyConnection| {
                Ok(chat_direct_messages::table
                    .filter(chat_direct_messages::mid.eq(mid))
                    .first(conn)
                    .optional()?)
            })
            .await
    }

    /// Marks the recipient's waiting messages with these ids delivered (DN10b).
    pub async fn mark_delivered_mids(&self, recipient: &str, mids: Vec<String>, at: i64) -> Result<(), Error> {
        if mids.is_empty() {
            return Ok(());
        }
        let recipient_text = recipient.to_owned();
        self.db
            .write(Actor::Player(recipient.to_owned()), move |tx| {
                diesel::update(
                    chat_direct_messages::table
                        .filter(chat_direct_messages::recipient.eq(recipient_text))
                        .filter(chat_direct_messages::delivered_at.is_null())
                        .filter(chat_direct_messages::mid.eq_any(mids)),
                )
                .set(chat_direct_messages::delivered_at.eq(Some(at)))
                .execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    /// Direct messages still waiting for `recipient`, oldest first.
    pub async fn pending_for(&self, recipient: &str, limit: i64) -> Result<Vec<DirectMessage>, Error> {
        let recipient = recipient.to_owned();
        self.db
            .read(move |conn: &mut AnyConnection| {
                Ok(chat_direct_messages::table
                    .filter(chat_direct_messages::recipient.eq(recipient))
                    .filter(chat_direct_messages::delivered_at.is_null())
                    .order(chat_direct_messages::id.asc())
                    .limit(limit)
                    .load(conn)?)
            })
            .await
    }

    pub async fn mark_delivered(&self, recipient: &str, ids: Vec<i64>, at: i64) -> Result<(), Error> {
        if ids.is_empty() {
            return Ok(());
        }
        self.db
            .write(Actor::Player(recipient.to_owned()), move |tx| {
                diesel::update(chat_direct_messages::table.filter(chat_direct_messages::id.eq_any(ids)))
                    .set(chat_direct_messages::delivered_at.eq(Some(at)))
                    .execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    /// The last `limit` direct messages between two accounts, either way, oldest first.
    pub async fn direct_between(&self, a: &str, b: &str, limit: i64) -> Result<Vec<DirectMessage>, Error> {
        let (a, b) = (a.to_owned(), b.to_owned());
        self.db
            .read(move |conn: &mut AnyConnection| {
                use chat_direct_messages::dsl::{id, recipient, sender};
                let mut rows: Vec<DirectMessage> = chat_direct_messages::table
                    .filter(
                        sender
                            .eq(a.clone())
                            .and(recipient.eq(b.clone()))
                            .or(sender.eq(b).and(recipient.eq(a))),
                    )
                    .order(id.desc())
                    .limit(limit)
                    .load(conn)?;
                rows.reverse();
                Ok(rows)
            })
            .await
    }

    pub async fn mute_of(&self, account: &str) -> Result<Option<Mute>, Error> {
        let account = account.to_owned();
        self.db
            .read(move |conn: &mut AnyConnection| Ok(chat_mutes::table.find(account).first(conn).optional()?))
            .await
    }

    pub async fn set_mute(&self, mute: Mute, actor: Actor) -> Result<(), Error> {
        self.db
            .write(actor, move |tx| {
                let sql = portable::upsert(
                    "chat_mutes",
                    &["account", "until_at", "reason", "muted_by", "at"],
                    &["account"],
                    &["until_at", "reason", "muted_by", "at"],
                );
                diesel::sql_query(sql)
                    .bind::<Text, _>(mute.account)
                    .bind::<diesel::sql_types::Nullable<BigInt>, _>(mute.until)
                    .bind::<diesel::sql_types::Nullable<Text>, _>(mute.reason)
                    .bind::<Text, _>(mute.by)
                    .bind::<BigInt, _>(mute.at)
                    .execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    /// Lifts a mute. Returns whether there was one.
    pub async fn clear_mute(&self, account: &str, actor: Actor) -> Result<bool, Error> {
        let account = account.to_owned();
        self.db
            .write(actor, move |tx| {
                Ok(diesel::delete(chat_mutes::table.find(account)).execute(tx.conn())? > 0)
            })
            .await
    }

    // ---- The outbox (DN10b) ----

    pub async fn outbox_add(&self, peer: &str, call: &str, now: i64) -> Result<i64, Error> {
        let values = (
            chat_outbox::peer.eq(peer.to_owned()),
            chat_outbox::call.eq(call.to_owned()),
            chat_outbox::tries.eq(0_i64),
            chat_outbox::next_at.eq(now),
            chat_outbox::created_at.eq(now),
        );
        self.db
            .write(Actor::System(String::from("chat-federation")), move |tx| {
                Ok(diesel::insert_into(chat_outbox::table)
                    .values(values)
                    .returning(chat_outbox::id)
                    .get_result(tx.conn())?)
            })
            .await
    }

    /// Calls whose time has come, oldest first.
    pub async fn outbox_due(&self, now: i64, limit: i64) -> Result<Vec<OutboxRow>, Error> {
        self.db
            .read(move |conn: &mut AnyConnection| {
                Ok(chat_outbox::table
                    .filter(chat_outbox::next_at.le(now))
                    .order(chat_outbox::id.asc())
                    .limit(limit)
                    .load(conn)?)
            })
            .await
    }

    pub async fn outbox_done(&self, id: i64) -> Result<(), Error> {
        self.db
            .write(Actor::System(String::from("chat-federation")), move |tx| {
                diesel::delete(chat_outbox::table.find(id)).execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    pub async fn outbox_retry(&self, id: i64, tries: i64, next_at: i64) -> Result<(), Error> {
        self.db
            .write(Actor::System(String::from("chat-federation")), move |tx| {
                diesel::update(chat_outbox::table.find(id))
                    .set((chat_outbox::tries.eq(tries), chat_outbox::next_at.eq(next_at)))
                    .execute(tx.conn())?;
                Ok(())
            })
            .await
    }

    /// Drops calls created before `before` that nobody answered: the message they carry stays at this node.
    pub async fn outbox_expire(&self, before: i64) -> Result<usize, Error> {
        self.db
            .write(Actor::System(String::from("chat-federation")), move |tx| {
                Ok(diesel::delete(chat_outbox::table.filter(chat_outbox::created_at.lt(before))).execute(tx.conn())?)
            })
            .await
    }

    pub async fn outbox_len(&self) -> Result<i64, Error> {
        self.db
            .read(move |conn: &mut AnyConnection| Ok(chat_outbox::table.count().get_result(conn)?))
            .await
    }
}
