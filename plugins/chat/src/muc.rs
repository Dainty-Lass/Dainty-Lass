//! From Dauntless Revived's metagame chat at `e1c5fec` (`services/metagame/src/chat/muc.rs`; docs/findings/
//! chat.md, "Rooms"), with DaintyLass's changes (DD21):
//!
//! - A room is a [`RoomKind`] (`naming.rs`), not the client's room JID: every `City-<id>` of a region is the
//!   region's one room. Each occupant keeps the JID it joined with, and every stanza it gets names that JID, so
//!   its client sees the room it asked for. A session joining a second JID of a room it holds leaves the first.
//! - Who may join and speak is the game module's (`ChatQuery::Room`), asked at the join and every 60 s
//!   (DR asked its party and guild tables at every message too; a module call crosses a pipe). A player the
//!   module no longer admits is removed (status 307).
//! - A muted account's room messages are refused. A room whose rules keep history stores each message, and a
//!   joiner gets the last ones after its own presence, each with its time (XEP-0045, XEP-0203).
//!
//! The rest is DR's: a room presence is the client's only if it comes from exactly "muc." + its domain; each
//! occupant is known by the nickname it joined with, byte for byte, and `<item jid>` names the member; the
//! joiner's own presence completes the join; occupants learn about each other both ways; every message goes
//! back to its sender too; a refused join is an error presence, never a rewritten nickname. Two sessions of one
//! account never see each other in a room, and one account is in a room with one session at most: when a
//! reconnected session joins a room its ghost is still in, the ghost leaves first ("replaced"). Room messages
//! skip recipients who blocked the sender.

use module_api::chat::{RoomKind, RoomRules};

use crate::federation::Envelope;
use crate::history::NewRoom;
use crate::naming::{room_class, room_key, room_kind_of};
use crate::nick::{check_nickname, name_part_of};
use crate::server::{Chat, Session, stamp};
use crate::settings::NickCheck;
use crate::xmpp::{Element, Jid, NS_CLIENT, NS_DELAY, NS_MUC, NS_MUC_USER, NS_STANZAS, escape, log_text, text_of};

pub const MAX_ROOMS_PER_SESSION: usize = 8;
pub const MAX_OCCUPANTS: usize = 128;
pub const MAX_ROOMS: usize = 1000;
pub const MAX_NICK_BYTES: usize = 1023;
pub const JOIN_BURST: u32 = 10;
const JOIN_REFILL_MS: f64 = 6.0 * 1000.0;
pub const MESSAGE_BURST: u32 = 8;
const MESSAGE_REFILL_MS: f64 = 1000.0;
const BODY_LIMIT: usize = 2048;
const MAX_ID: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveReason {
    Left,
    Disconnect,
    Evicted,
    Replaced,
}

impl LeaveReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Disconnect => "disconnect",
            Self::Evicted => "evicted",
            Self::Replaced => "replaced",
        }
    }
}

/// `IsMucDomain`: muc.<domain>, or the legacy conference.<domain> alias some clients send.
pub fn is_muc_domain(account_domain: &str, domain: &str) -> bool {
    let (actual, account) = (domain.to_lowercase(), account_domain.to_lowercase());
    actual == format!("muc.{account}") || actual == format!("conference.{account}")
}

pub fn take_message_token(session: &mut Session, now: i64) -> bool {
    session.messages.take(MESSAGE_BURST, MESSAGE_REFILL_MS, now)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    Nick(crate::nick::NickReason),
    NotMember,
    NotAllowed,
    Conflict,
    Limit,
    /// The game module could not say
    Unavailable,
}

impl Refusal {
    fn as_str(self) -> &'static str {
        match self {
            Self::Nick(reason) => reason.as_str(),
            Self::NotMember => "not-member",
            Self::NotAllowed => "not-allowed",
            Self::Conflict => "conflict",
            Self::Limit => "limit",
            Self::Unavailable => "unavailable",
        }
    }

    fn error(self) -> (&'static str, &'static str) {
        match self {
            Self::Nick(_) | Self::NotMember => ("auth", "forbidden"),
            Self::NotAllowed => ("cancel", "not-allowed"),
            Self::Conflict => ("cancel", "conflict"),
            Self::Limit | Self::Unavailable => ("wait", "service-unavailable"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Occupant {
    pub session: u64,
    pub nick: String,
    /// <local>@<muc domain> as this occupant's client sent it
    pub jid: String,
    pub rules: RoomRules,
}

#[derive(Debug, Clone)]
pub struct Room {
    pub kind: RoomKind,
    /// In join order
    pub occupants: Vec<Occupant>,
}

impl Room {
    fn occupant(&self, id: u64) -> Option<&Occupant> {
        self.occupants.iter().find(|o| o.session == id)
    }
}

#[derive(Default)]
struct Shown {
    self_: bool,
    unavailable: bool,
    evicted: bool,
}

impl Chat {
    fn room_index(&self, kind: &RoomKind) -> Option<usize> {
        self.rooms.iter().position(|r| r.kind == *kind)
    }

    /// The room this session holds under the JID its client names.
    fn room_held(&self, id: u64, room_jid: &str) -> Option<usize> {
        self.rooms
            .iter()
            .position(|r| r.occupant(id).is_some_and(|o| o.jid == room_jid))
    }

    fn full_jid_of(&self, id: u64) -> String {
        self.sessions.get(&id).map(Session::full_jid).unwrap_or_default()
    }

    /// The presence of `occupant` (as `nick`) for `to`, from the room as `to`'s client names it (`room_jid`).
    fn occupant_presence(&self, room_jid: &str, nick: &str, occupant: u64, to: u64, shown: Shown) -> String {
        let statuses = format!(
            "{}{}",
            if shown.self_ { "<status code=\"110\"/>" } else { "" },
            if shown.evicted { "<status code=\"307\"/>" } else { "" }
        );
        format!(
            "<presence xmlns=\"{NS_CLIENT}\"{} from=\"{}\" to=\"{}\"><x xmlns=\"{NS_MUC_USER}\"><item affiliation=\"none\" role=\"{}\" jid=\"{}\"/>{statuses}</x></presence>",
            if shown.unavailable { " type=\"unavailable\"" } else { "" },
            escape(&format!("{room_jid}/{nick}")),
            escape(&self.full_jid_of(to)),
            if shown.unavailable { "none" } else { "participant" },
            escape(&self.full_jid_of(occupant))
        )
    }

    /// A refused join: the error presence, one log line per session, room class and reason every 10
    /// minutes, and one dropped stanza toward the session's abuse limit.
    async fn refuse_join(&mut self, id: u64, room_jid: &str, local: &str, nick: &str, reason: Refusal) {
        if self.log_once(&format!("join-refused|{id}|{}", reason.as_str()), None) {
            tracing::info!(
                "chat: join refused room={} uid={} reason={}",
                log_text(local, 80),
                self.sessions
                    .get(&id)
                    .and_then(|s| s.uid.as_deref())
                    .unwrap_or("undefined"),
                reason.as_str()
            );
        }
        let (kind, condition) = reason.error();
        let stanza = format!(
            "<presence xmlns=\"{NS_CLIENT}\" type=\"error\" from=\"{}\" to=\"{}\"><x xmlns=\"{NS_MUC}\"/><error type=\"{kind}\"><{condition} xmlns=\"{NS_STANZAS}\"/></error></presence>",
            escape(&format!("{room_jid}/{nick}")),
            escape(&self.full_jid_of(id))
        );
        self.send(id, &stanza);
        self.drop_stanza(id, "refused").await;
    }

    fn message_error(&mut self, id: u64, room_jid: &str, message_id: Option<&str>, condition: (&str, &str)) {
        let (kind, name) = condition;
        let stanza = format!(
            "<message xmlns=\"{NS_CLIENT}\" type=\"error\"{} from=\"{}\" to=\"{}\"><error type=\"{kind}\"><{name} xmlns=\"{NS_STANZAS}\"/></error></message>",
            message_id.map(|i| format!(" id=\"{}\"", escape(i))).unwrap_or_default(),
            escape(room_jid),
            escape(&self.full_jid_of(id))
        );
        self.send(id, &stanza);
    }

    /// Everyone in the room this session may see: not itself, not its own account's other sessions.
    fn others(&self, room: usize, id: u64) -> Vec<Occupant> {
        let uid = self.sessions.get(&id).and_then(|s| s.uid.clone());
        self.rooms[room]
            .occupants
            .iter()
            .filter(|o| o.session != id && self.sessions.get(&o.session).and_then(|s| s.uid.clone()) != uid)
            .cloned()
            .collect()
    }

    pub async fn join(&mut self, id: u64, to: &Jid) {
        let now = self.now();
        let nick = to.resource.clone().unwrap_or_default();
        let room_jid = format!("{}@{}", to.local, to.domain);
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let uid = session.uid.clone().unwrap_or_default();
        // Every join takes a token first, refused ones included
        if !session.joins.take(JOIN_BURST, JOIN_REFILL_MS, now) {
            self.refuse_join(id, &room_jid, &to.local, &nick, Refusal::Limit).await;
            return;
        }
        let domain = session.domain.clone();
        let module = session.module.clone();
        let node = &self.backends.node;
        let mut kind = room_kind_of(&to.local, &node.node, &node.region);
        if kind.is_none()
            && let Some(module) = &module
            && self.answers_chat(module)
            && let Ok(module_api::chat::ChatAnswer::RoomOf(found)) = self
                .backends
                .host
                .ask(module, module_api::chat::ChatQuery::RoomOf { name: to.local.clone() })
                .await
        {
            kind = found;
        }
        let (Some(kind), true) = (kind, is_muc_domain(&domain, &to.domain)) else {
            self.refuse_join(id, &room_jid, &to.local, &nick, Refusal::NotAllowed)
                .await;
            return;
        };
        let existing = self.room_index(&kind);
        let held = existing.and_then(|r| self.rooms[r].occupant(id)).cloned();
        let Some(rules) = self.rules_of(id, &kind).await else {
            self.refuse_join(id, &room_jid, &to.local, &nick, Refusal::Unavailable)
                .await;
            return;
        };
        // The game says who may join; someone it no longer admits is also removed from the room
        if !rules.may_join {
            if let (Some(room), Some(_)) = (existing, &held) {
                self.remove(id, room, LeaveReason::Evicted);
            }
            self.refuse_join(id, &room_jid, &to.local, &nick, Refusal::NotMember)
                .await;
            return;
        }
        // Rejoin with the same JID and nickname: this session alone gets the room again
        if let (Some(room), Some(held)) = (existing, &held)
            && held.nick == nick
            && held.jid == room_jid
        {
            self.welcome(id, room);
            tracing::info!(
                "chat: join room={} uid={uid} name={} occupants={} rejoin",
                room_class(&kind),
                name_part_of(&nick).unwrap_or_else(|| "?".into()),
                self.others(room, id).len()
            );
            return;
        }
        if nick.is_empty() || nick.len() > MAX_NICK_BYTES {
            self.refuse_join(
                id,
                &room_jid,
                &to.local,
                &nick,
                Refusal::Nick(crate::nick::NickReason::Format),
            )
            .await;
            return;
        }
        let (resource, at_login) = self
            .sessions
            .get(&id)
            .map(|s| (s.resource.clone().unwrap_or_default(), s.name_at_login.clone()))
            .unwrap_or_default();
        let check = check_nickname(&nick, &uid, &resource, &[at_login.as_deref()]);
        // A relaxed check admits a wrong resource, format or name only; another account's id is refused in
        // both modes
        if let Err(reason) = check {
            if self.nick_check == NickCheck::Enforce || reason == crate::nick::NickReason::Account {
                self.refuse_join(id, &room_jid, &to.local, &nick, Refusal::Nick(reason))
                    .await;
                return;
            }
            if self.log_once(&format!("nick-log|{id}|{}", room_key(&kind)), None) {
                tracing::warn!(
                    "chat: join nickname not checked room={} uid={uid} reason={} (nick check log)",
                    room_class(&kind),
                    reason.as_str()
                );
            }
        }
        // A different nickname or room JID in a room this session is already in: leave with the old one first
        if let (Some(room), Some(_)) = (existing, &held) {
            self.remove(id, room, LeaveReason::Left);
        }
        let room = self.room_index(&kind);
        // The occupants of other accounts. An older session of this account is replaced below.
        let other_accounts: Vec<Occupant> = room
            .map(|r| {
                self.rooms[r]
                    .occupants
                    .iter()
                    .filter(|o| {
                        o.session != id
                            && self.sessions.get(&o.session).and_then(|s| s.uid.as_deref()) != Some(uid.as_str())
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if other_accounts.iter().any(|o| o.nick == nick) {
            self.refuse_join(id, &room_jid, &to.local, &nick, Refusal::Conflict)
                .await;
            return;
        }
        let held_rooms = self.sessions.get(&id).map_or(0, |s| s.rooms.len());
        if held_rooms >= MAX_ROOMS_PER_SESSION
            || other_accounts.len() >= MAX_OCCUPANTS
            || (room.is_none() && self.rooms.len() >= MAX_ROOMS)
        {
            self.refuse_join(id, &room_jid, &to.local, &nick, Refusal::Limit).await;
            return;
        }
        // 0. This account's older session in the room (normally a ghost) leaves first, told to the
        // others but not to itself
        if let Some(room) = room {
            let ghosts: Vec<u64> = self.rooms[room]
                .occupants
                .iter()
                .map(|o| o.session)
                .filter(|other| {
                    *other != id && self.sessions.get(other).and_then(|s| s.uid.as_deref()) == Some(uid.as_str())
                })
                .collect();
            for ghost in ghosts {
                if let Some(room) = self.room_index(&kind) {
                    self.remove(ghost, room, LeaveReason::Replaced);
                }
            }
        }
        // remove() drops an emptied room; the joiner's own room is held until it is in it
        let room = match self.room_index(&kind) {
            Some(room) => room,
            None => {
                self.rooms.push(Room {
                    kind: kind.clone(),
                    occupants: Vec::new(),
                });
                self.rooms.len() - 1
            }
        };
        let others = self.others(room, id);
        // 1. The joiner hears of everyone already there; 2. they hear of the joiner; 3. the joiner's
        // own presence last, which completes the join on its screen
        for other in &others {
            let stanza = self.occupant_presence(&room_jid, &other.nick, other.session, id, Shown::default());
            self.send(id, &stanza);
        }
        for other in &others {
            let stanza = self.occupant_presence(&other.jid, &nick, id, other.session, Shown::default());
            self.send(other.session, &stanza);
        }
        self.rooms[room].occupants.push(Occupant {
            session: id,
            nick: nick.clone(),
            jid: room_jid.clone(),
            rules,
        });
        if let Some(session) = self.sessions.get_mut(&id) {
            session.rooms.push((room_jid.clone(), nick.clone()));
        }
        let own = self.occupant_presence(
            &room_jid,
            &nick,
            id,
            id,
            Shown {
                self_: true,
                ..Shown::default()
            },
        );
        self.send(id, &own);
        let history_limit = if rules.history { self.history_on_join } else { 0 };
        let pulled = self.backends.federation.room_joined(&kind, history_limit).await;
        if rules.history {
            let key = room_key(&kind);
            for msg in pulled {
                self.remember_mid(&msg.mid);
                let _ = self
                    .backends
                    .history
                    .add_room_once(NewRoom {
                        room: key.clone(),
                        sender: msg.from.to_string(),
                        nick: msg.nick,
                        body: msg.body,
                        stanza_id: msg.stanza_id,
                        mid: Some(msg.mid),
                        at: msg.at,
                    })
                    .await;
            }
            self.send_history(id, &kind, &room_jid).await;
        }
        tracing::info!(
            "chat: join room={} uid={uid} name={} occupants={}",
            room_class(&kind),
            check.ok().or_else(|| name_part_of(&nick)).unwrap_or_else(|| "?".into()),
            others.len()
        );
    }

    /// The room's last messages, after the joiner's own presence, from the room as its client names it.
    async fn send_history(&mut self, id: u64, kind: &RoomKind, room_jid: &str) {
        if self.history_on_join == 0 {
            return;
        }
        let rows = match self
            .backends
            .history
            .room_history(&room_key(kind), self.history_on_join)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!("chat: reading room history failed ({error})");
                return;
            }
        };
        let to = escape(&self.full_jid_of(id));
        let domain = room_jid.split_once('@').map_or("", |(_, d)| d).to_owned();
        for row in rows {
            let id_attr = row
                .stanza_id
                .as_deref()
                .map(|i| format!(" id=\"{}\"", escape(i)))
                .unwrap_or_default();
            let stanza = format!(
                "<message xmlns=\"{NS_CLIENT}\" type=\"groupchat\"{id_attr} from=\"{}\" to=\"{to}\"><body>{}</body><delay xmlns=\"{NS_DELAY}\" from=\"{}\" stamp=\"{}\"/></message>",
                escape(&format!("{room_jid}/{}", row.nick)),
                escape(&row.body),
                escape(&domain),
                stamp(row.at)
            );
            self.send(id, &stanza);
        }
    }

    /// Re-asks the game who may stay; anyone it no longer admits is removed.
    pub async fn sweep_rooms(&mut self) {
        let kinds: Vec<RoomKind> = self.rooms.iter().map(|r| r.kind.clone()).collect();
        for kind in kinds {
            let occupants: Vec<u64> = self
                .room_index(&kind)
                .map(|r| self.rooms[r].occupants.iter().map(|o| o.session).collect())
                .unwrap_or_default();
            for occupant in occupants {
                let rules = self.rules_of(occupant, &kind).await;
                let Some(room) = self.room_index(&kind) else {
                    break;
                };
                match rules {
                    Some(rules) if rules.may_join => {
                        if let Some(slot) = self.rooms[room].occupants.iter_mut().find(|o| o.session == occupant) {
                            slot.rules = rules;
                        }
                    }
                    Some(_) => self.remove(occupant, room, LeaveReason::Evicted),
                    // The module could not say: keep what it said last
                    None => {}
                }
            }
        }
    }

    fn welcome(&mut self, id: u64, room: usize) {
        let Some(own) = self.rooms[room].occupant(id).cloned() else {
            return;
        };
        for other in self.others(room, id) {
            let stanza = self.occupant_presence(&own.jid, &other.nick, other.session, id, Shown::default());
            self.send(id, &stanza);
        }
        let stanza = self.occupant_presence(
            &own.jid,
            &own.nick,
            id,
            id,
            Shown {
                self_: true,
                ..Shown::default()
            },
        );
        self.send(id, &stanza);
    }

    pub fn leave(&mut self, id: u64, to: &Jid) {
        let room_jid = format!("{}@{}", to.local, to.domain);
        let Some(room) = self.room_held(id, &room_jid) else {
            // Not in it (after a refused join, a move to another JID of the room, or a restart): confirm the
            // leave anyway, so the client's room does not stay waiting for it
            let Some(session) = self.sessions.get(&id) else {
                return;
            };
            if is_muc_domain(&session.domain, &to.domain)
                && let Some(resource) = to.resource.as_deref().filter(|r| !r.is_empty())
            {
                let jid = escape(&session.full_jid());
                let stanza = format!(
                    "<presence xmlns=\"{NS_CLIENT}\" type=\"unavailable\" from=\"{}\" to=\"{jid}\"><x xmlns=\"{NS_MUC_USER}\"><item affiliation=\"none\" role=\"none\" jid=\"{jid}\"/><status code=\"110\"/></x></presence>",
                    escape(&format!("{room_jid}/{resource}"))
                );
                self.send(id, &stanza);
            }
            return;
        };
        self.remove(id, room, LeaveReason::Left);
    }

    /// A broadcast unavailable, or the end of the session.
    pub fn leave_all(&mut self, id: u64, reason: LeaveReason) {
        let held: Vec<RoomKind> = self
            .rooms
            .iter()
            .filter(|r| r.occupant(id).is_some())
            .map(|r| r.kind.clone())
            .collect();
        for kind in held {
            if let Some(room) = self.room_index(&kind) {
                self.remove(id, room, reason);
            }
        }
        if let Some(session) = self.sessions.get_mut(&id) {
            session.rooms.clear();
        }
    }

    fn remove(&mut self, id: u64, room: usize, reason: LeaveReason) {
        let Some(own) = self.rooms[room].occupant(id).cloned() else {
            return;
        };
        let evicted = reason == LeaveReason::Evicted;
        for other in self.others(room, id) {
            let stanza = self.occupant_presence(
                &other.jid,
                &own.nick,
                id,
                other.session,
                Shown {
                    unavailable: true,
                    evicted,
                    ..Shown::default()
                },
            );
            self.send(other.session, &stanza);
        }
        self.rooms[room].occupants.retain(|o| o.session != id);
        let ended = match self.sessions.get_mut(&id) {
            Some(session) => {
                session.rooms.retain(|(r, _)| *r != own.jid);
                session.ended
            }
            None => true,
        };
        if !ended && matches!(reason, LeaveReason::Left | LeaveReason::Evicted) {
            let stanza = self.occupant_presence(
                &own.jid,
                &own.nick,
                id,
                id,
                Shown {
                    self_: true,
                    unavailable: true,
                    evicted,
                },
            );
            self.send(id, &stanza);
        }
        let class = room_class(&self.rooms[room].kind);
        if self.rooms[room].occupants.is_empty() {
            let removed = self.rooms.remove(room);
            let fed = self.backends.federation.clone();
            tokio::spawn(async move {
                fed.room_left(&removed.kind).await;
            });
        }
        tracing::info!(
            "chat: leave room={class} uid={} reason={}",
            self.sessions
                .get(&id)
                .and_then(|s| s.uid.as_deref())
                .unwrap_or("undefined"),
            reason.as_str()
        );
    }

    // ---- Room messages ----

    pub async fn group_chat(&mut self, id: u64, to: &Jid, node: &Element) {
        let room_jid = format!("{}@{}", to.local, to.domain);
        let message_id = node
            .attr("id")
            .filter(|i| i.encode_utf16().count() <= MAX_ID)
            .map(str::to_owned);
        let body_node = node.child("body");
        // A subject change alone is ignored
        if body_node.is_none() && node.child("subject").is_some() {
            return;
        }
        let not_acceptable = ("modify", "not-acceptable");
        let room = self.room_held(id, &room_jid);
        let own = room.and_then(|r| self.rooms[r].occupant(id)).cloned();
        let body = text_of(body_node);
        let length = body.chars().count();
        let (Some(room), Some(own), true) = (room, own, (1..=BODY_LIMIT).contains(&length)) else {
            self.message_error(id, &room_jid, message_id.as_deref(), not_acceptable);
            return;
        };
        let now = self.now();
        let muted = self.sessions.get(&id).is_some_and(|s| s.is_muted(now));
        if muted || !own.rules.may_speak {
            self.message_error(id, &room_jid, message_id.as_deref(), ("auth", "forbidden"));
            return;
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if !take_message_token(session, now) {
            let uid = session.uid.clone().unwrap_or_default();
            if self.log_once(&format!("message-rate|{id}"), Some(60 * 1000)) {
                tracing::warn!(
                    "chat: message limit c={id} uid={uid} room={}",
                    room_class(&self.rooms[room].kind)
                );
            }
            self.message_error(id, &room_jid, message_id.as_deref(), not_acceptable);
            return;
        }
        let uid = session.uid.clone().unwrap_or_default();
        let name = session.name_at_login.clone().unwrap_or_else(|| uid.clone());
        let Some(sender) = session.account.clone() else {
            return;
        };
        let kind = self.rooms[room].kind.clone();
        if let Some(command) = crate::moderation::parse_report(&body) {
            let from = room_jid.clone();
            self.report_command(id, command, Some(room_key(&kind)), "groupchat", &from);
            return;
        }
        let mid = format!("rm-{}", crate::server::random_hex(16));
        let rated = self.moderation.rate(&body, crate::moderation::channel_of(&kind));
        self.moderation.record(crate::moderation::Line {
            id: mid.clone(),
            at: now,
            channel: crate::moderation::channel_of(&kind).as_str(),
            room_id: Some(room_key(&kind)),
            sender: crate::moderation::Person {
                user_id: uid.clone(),
                name,
            },
            recipient: None,
            body: body.clone(),
            verdict: rated.verdict,
        });
        let body = rated.shown;
        let text = escape(&body);
        // Blocked: only the sender's own echo, so it reads as sent (dashboard integration.md)
        if rated.verdict == crate::moderation::Verdict::Blocked {
            let stanza = format!(
                "<message xmlns=\"{NS_CLIENT}\" type=\"groupchat\"{} from=\"{}\" to=\"{}\"><body>{text}</body></message>",
                message_id
                    .as_deref()
                    .map(|i| format!(" id=\"{}\"", escape(i)))
                    .unwrap_or_default(),
                escape(&format!("{}/{}", own.jid, own.nick)),
                escape(&self.full_jid_of(id))
            );
            self.send(id, &stanza);
            tracing::info!(
                "chat: message room={} uid={uid} len={length} blocked by filter",
                room_class(&kind)
            );
            return;
        }
        let recipients: Vec<Occupant> = self.rooms[room]
            .occupants
            .iter()
            .filter(|o| {
                o.session == id || self.sessions.get(&o.session).and_then(|s| s.uid.as_deref()) != Some(uid.as_str())
            })
            .cloned()
            .collect();
        let id_attr = message_id
            .as_deref()
            .map(|i| format!(" id=\"{}\"", escape(i)))
            .unwrap_or_default();
        self.remember_mid(&mid);
        // Stored before it is sent: a message nobody can read back from history is never on a screen
        if own.rules.history
            && let Err(error) = self
                .backends
                .history
                .add_room_once(NewRoom {
                    room: room_key(&kind),
                    sender: sender.to_string(),
                    nick: own.nick.clone(),
                    body: body.clone(),
                    stanza_id: message_id.clone(),
                    mid: Some(mid.clone()),
                    at: now,
                })
                .await
        {
            tracing::error!("chat: storing a room message failed ({error})");
        }
        let (mut delivered, mut blocked) = (0, 0);
        for recipient in recipients {
            if recipient.session != id {
                let account = self.sessions.get(&recipient.session).and_then(|s| s.account.clone());
                if let Some(account) = account
                    && self.backends.host.blocked(&sender, &account).await
                {
                    blocked += 1;
                    continue;
                }
            }
            let stanza = format!(
                "<message xmlns=\"{NS_CLIENT}\" type=\"groupchat\"{id_attr} from=\"{}\" to=\"{}\"><body>{text}</body></message>",
                escape(&format!("{}/{}", recipient.jid, own.nick)),
                escape(&self.full_jid_of(recipient.session))
            );
            self.send(recipient.session, &stanza);
            delivered += 1;
        }
        tracing::info!(
            "chat: message room={} uid={uid} len={length} to={delivered}{}",
            room_class(&kind),
            if blocked > 0 {
                format!(" blocked={blocked}")
            } else {
                String::new()
            }
        );
        self.publish(Envelope::Room {
            mid,
            room: kind,
            from: sender,
            nick: own.nick,
            body,
            id: message_id,
            at: now,
            history: own.rules.history,
        })
        .await;
    }

    pub async fn remote_room(&mut self, message: crate::federation::RoomWire) {
        if !self.remember_mid(&message.mid) {
            return;
        }
        let Some(room) = self.room_index(&message.room) else {
            return;
        };
        let keep_history = message.history || self.rooms[room].occupants.iter().any(|o| o.rules.history);
        if keep_history {
            let _ = self
                .backends
                .history
                .add_room_once(NewRoom {
                    room: room_key(&message.room),
                    sender: message.from.to_string(),
                    nick: message.nick.clone(),
                    body: message.body.clone(),
                    stanza_id: message.stanza_id.clone(),
                    mid: Some(message.mid.clone()),
                    at: message.at,
                })
                .await;
        }
        let Some(room) = self.room_index(&message.room) else {
            return;
        };
        let recipients: Vec<Occupant> = self.rooms[room].occupants.clone();
        let text = escape(&message.body);
        let id_attr = message
            .stanza_id
            .as_deref()
            .map(|i| format!(" id=\"{}\"", escape(i)))
            .unwrap_or_default();
        for recipient in recipients {
            let account = self.sessions.get(&recipient.session).and_then(|s| s.account.clone());
            if let Some(account) = account
                && self.backends.host.blocked(&message.from, &account).await
            {
                continue;
            }
            let stanza = format!(
                "<message xmlns=\"{NS_CLIENT}\" type=\"groupchat\"{id_attr} from=\"{}\" to=\"{}\"><body>{text}</body></message>",
                escape(&format!("{}/{}", recipient.jid, message.nick)),
                escape(&self.full_jid_of(recipient.session))
            );
            self.send(recipient.session, &stanza);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muc_domains_follow_the_account_domain() {
        assert!(is_muc_domain("Dom", "MUC.dom"));
        assert!(is_muc_domain("dom", "conference.dom"));
        assert!(!is_muc_domain("dom", "muc.other"));
    }
}
