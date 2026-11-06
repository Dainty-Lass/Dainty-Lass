// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rooms (MUC). A room is a `RoomKind`; each occupant keeps the room JID its client joined with, so two `City-*`
//! rooms of one node are one room that every client sees under its own name (DD21).

use module_api::chat::{ChatAnswer, ChatQuery, RoomKind, RoomRules};
use tracing::{info, warn};

use crate::federation::{Envelope, RoomWire};
use crate::history::NewRoom;
use crate::hooks::{default_rules, is_unsupported};
use crate::moderation::{Line, Person, Verdict, channel_of};
use crate::naming::{room_class, room_key, room_kind_of};
use crate::nick::{NickReason, check_nickname, name_part_of};
use crate::server::{BODY_LIMIT, Chat, MAX_ID, random_hex, utf16_len};
use crate::settings::NickCheck;
use crate::xmpp::{Element, Jid, NS_MUC, NS_MUC_USER, NS_STANZAS, escape, log_text, text_of};

/// The longest room nick, in bytes.
pub const MAX_NICK_BYTES: usize = 1023;
/// The most rooms one session may be in.
pub const MAX_ROOMS_PER_SESSION: usize = 8;
/// The most occupants a room takes.
pub const MAX_OCCUPANTS: usize = 128;
/// The most rooms that may exist at once.
pub const MAX_ROOMS: usize = 1000;
/// Joins a session may make at once before being throttled.
pub const JOIN_BURST: u32 = 10;
/// Room messages a session may send at once before being throttled.
pub const MESSAGE_BURST: u32 = 8;
const JOIN_REFILL_MS: f64 = 6000.0;
const MESSAGE_REFILL_MS: f64 = 1000.0;
const MINUTE: i64 = 60_000;
const TEN_MINUTES: i64 = 10 * MINUTE;

/// `muc.<domain>` or `conference.<domain>` of the session's domain, in any case.
pub fn is_muc_domain(account_domain: &str, domain: &str) -> bool {
    let (account, domain) = (account_domain.to_lowercase(), domain.to_lowercase());
    domain == format!("muc.{account}") || domain == format!("conference.{account}")
}

#[derive(Debug, Clone)]
pub(crate) struct Occupant {
    pub(crate) session: u64,
    pub(crate) nick: String,
    /// The room JID as this occupant's client spelled it
    pub(crate) jid: String,
    pub(crate) rules: RoomRules,
}

#[derive(Debug)]
pub(crate) struct Room {
    pub(crate) kind: RoomKind,
    pub(crate) occupants: Vec<Occupant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leave {
    Left,
    Disconnect,
    Evicted,
    Replaced,
}

impl Leave {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Disconnect => "disconnect",
            Self::Evicted => "evicted",
            Self::Replaced => "replaced",
        }
    }
}

/// A refused join's log reason, error type and condition.
fn refusal(reason: &str) -> (&'static str, &'static str) {
    match reason {
        "not-allowed" => ("cancel", "not-allowed"),
        "conflict" => ("cancel", "conflict"),
        "limit" | "unavailable" => ("wait", "service-unavailable"),
        _ => ("auth", "forbidden"),
    }
}

fn presence(
    from_room: &str,
    nick: &str,
    to: &str,
    occupant_jid: &str,
    unavailable: bool,
    own: bool,
    evicted: bool,
) -> String {
    format!(
        r#"<presence xmlns="jabber:client"{} from="{}/{}" to="{}"><x xmlns="{NS_MUC_USER}"><item affiliation="none" role="{}" jid="{}"/>{}{}</x></presence>"#,
        if unavailable { r#" type="unavailable""# } else { "" },
        escape(from_room),
        escape(nick),
        escape(to),
        if unavailable { "none" } else { "participant" },
        escape(occupant_jid),
        if own { r#"<status code="110"/>"# } else { "" },
        if evicted { r#"<status code="307"/>"# } else { "" },
    )
}

fn groupchat(id: Option<&str>, from: &str, to: &str, body: &str) -> String {
    format!(
        r#"<message xmlns="jabber:client" type="groupchat"{} from="{}" to="{}"><body>{}</body></message>"#,
        id.map(|id| format!(r#" id="{}""#, escape(id))).unwrap_or_default(),
        escape(from),
        escape(to),
        escape(body)
    )
}

impl Chat {
    fn full_of(&self, session: u64) -> String {
        self.sessions.get(&session).map(|s| s.full_jid()).unwrap_or_default()
    }

    fn uid_of(&self, session: u64) -> Option<String> {
        self.sessions.get(&session).and_then(|s| s.uid.clone())
    }

    fn room_index(&self, kind: &RoomKind) -> Option<usize> {
        self.rooms.iter().position(|r| &r.kind == kind)
    }

    /// What a room local part names: chat's naming, else the session's module.
    async fn room_kind(&self, session: u64, local: &str) -> Option<RoomKind> {
        if let Some(kind) = room_kind_of(local, &self.node.node, &self.node.region) {
            return Some(kind);
        }
        let module = self.sessions.get(&session).and_then(|s| s.module.clone());
        let module = module.filter(|m| self.answers_chat(Some(m)))?;
        match self
            .host
            .ask(&module, ChatQuery::RoomOf { name: local.to_owned() })
            .await
        {
            Ok(ChatAnswer::RoomOf(kind)) => kind,
            _ => None,
        }
    }

    /// A session's rules in a room; none when its module could not say.
    async fn rules_for(&self, session: u64, kind: &RoomKind) -> Option<RoomRules> {
        let s = self.sessions.get(&session)?;
        let (module, account) = (s.module.clone(), s.account.clone());
        let Some(module) = module.filter(|m| self.answers_chat(Some(m))) else {
            return Some(default_rules(kind));
        };
        let account = account?;
        let query = ChatQuery::Room {
            room: kind.clone(),
            account,
        };
        match self.host.ask(&module, query).await {
            Ok(ChatAnswer::Room(rules)) => Some(rules),
            Ok(_) => None,
            Err(e) if is_unsupported(&e) => Some(default_rules(kind)),
            Err(e) => {
                warn!("chat: {module} gave no room rules: {e}");
                None
            }
        }
    }

    async fn refuse_join(&mut self, session: u64, local: &str, room_jid: &str, nick: &str, reason: &str) {
        let now = self.now();
        let (kind, condition) = refusal(reason);
        let to = self.full_of(session);
        let stanza = format!(
            r#"<presence xmlns="jabber:client" type="error" from="{}/{}" to="{}"><x xmlns="{NS_MUC}"/><error type="{kind}"><{condition} xmlns="{NS_STANZAS}"/></error></presence>"#,
            escape(room_jid),
            escape(nick),
            escape(&to)
        );
        self.send(session, &stanza);
        if self
            .log_once
            .check(&format!("join|{session}|{reason}"), TEN_MINUTES, now)
        {
            info!(
                "chat: join refused c={session} room={} uid={} reason={reason}",
                log_text(local, 80),
                self.uid_of(session).unwrap_or_else(|| String::from("-"))
            );
        }
        self.drop_stanza(session, "refused").await;
    }

    /// Presence to `local@domain/nick`.
    pub(crate) async fn join(&mut self, session: u64, jid: Jid) {
        let now = self.now();
        let nick = jid.resource.clone().unwrap_or_default();
        let room_jid = format!("{}@{}", jid.local, jid.domain);
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        if !s.joins.take(JOIN_BURST, JOIN_REFILL_MS, now) {
            return self.refuse_join(session, &jid.local, &room_jid, &nick, "limit").await;
        }
        let session_domain = s.domain.clone();
        let kind = self.room_kind(session, &jid.local).await;
        let Some(kind) = kind.filter(|_| is_muc_domain(&session_domain, &jid.domain)) else {
            return self
                .refuse_join(session, &jid.local, &room_jid, &nick, "not-allowed")
                .await;
        };
        let Some(rules) = self.rules_for(session, &kind).await else {
            return self
                .refuse_join(session, &jid.local, &room_jid, &nick, "unavailable")
                .await;
        };
        let held = self
            .room_index(&kind)
            .and_then(|i| self.rooms[i].occupants.iter().find(|o| o.session == session).cloned());
        if !rules.may_join {
            if held.is_some() {
                self.remove_from_room(&kind, session, Leave::Evicted);
            }
            return self
                .refuse_join(session, &jid.local, &room_jid, &nick, "not-member")
                .await;
        }
        if let Some(held) = &held
            && held.nick == nick
            && held.jid == room_jid
        {
            return self.rejoin(session, &kind, held);
        }
        if nick.is_empty() || nick.len() > MAX_NICK_BYTES {
            return self
                .refuse_join(session, &jid.local, &room_jid, &nick, "nick-format")
                .await;
        }
        let Some(s) = self.sessions.get(&session) else { return };
        let uid = s.uid.clone().unwrap_or_default();
        let resource = s.resource.clone().unwrap_or_default();
        let names = [s.name.as_deref()];
        if let Err(reason) = check_nickname(&nick, &uid, &resource, &names) {
            if self.nick_check() == NickCheck::Enforce || reason == NickReason::Account {
                return self
                    .refuse_join(session, &jid.local, &room_jid, &nick, reason.as_str())
                    .await;
            }
            if self
                .log_once
                .check(&format!("nick|{session}|{}", room_key(&kind)), TEN_MINUTES, now)
            {
                warn!(
                    "chat: nick admitted under log c={session} uid={uid} reason={}",
                    reason.as_str()
                );
            }
        }
        if held.is_some() {
            self.remove_from_room(&kind, session, Leave::Left);
        }
        let index = self.room_index(&kind);
        let others: Vec<Occupant> = index
            .map(|i| {
                self.rooms[i]
                    .occupants
                    .iter()
                    .filter(|o| o.session != session && self.uid_of(o.session).as_deref() != Some(uid.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if others.iter().any(|o| o.nick == nick) {
            return self
                .refuse_join(session, &jid.local, &room_jid, &nick, "conflict")
                .await;
        }
        let held_rooms = self.sessions.get(&session).map_or(0, |s| s.rooms.len());
        if held_rooms >= MAX_ROOMS_PER_SESSION
            || others.len() >= MAX_OCCUPANTS
            || (index.is_none() && self.rooms.len() >= MAX_ROOMS)
        {
            return self.refuse_join(session, &jid.local, &room_jid, &nick, "limit").await;
        }
        let ghosts: Vec<u64> = index
            .map(|i| {
                self.rooms[i]
                    .occupants
                    .iter()
                    .filter(|o| o.session != session && self.uid_of(o.session).as_deref() == Some(uid.as_str()))
                    .map(|o| o.session)
                    .collect()
            })
            .unwrap_or_default();
        for ghost in ghosts {
            self.remove_from_room(&kind, ghost, Leave::Replaced);
        }
        let index = match self.room_index(&kind) {
            Some(i) => i,
            None => {
                self.rooms.push(Room {
                    kind: kind.clone(),
                    occupants: Vec::new(),
                });
                self.rooms.len() - 1
            }
        };
        let full = self.full_of(session);
        for other in &others {
            let stanza = presence(
                &room_jid,
                &other.nick,
                &full,
                &self.full_of(other.session),
                false,
                false,
                false,
            );
            self.send(session, &stanza);
        }
        for other in &others {
            let stanza = presence(
                &other.jid,
                &nick,
                &self.full_of(other.session),
                &full,
                false,
                false,
                false,
            );
            self.send(other.session, &stanza);
        }
        self.rooms[index].occupants.push(Occupant {
            session,
            nick: nick.clone(),
            jid: room_jid.clone(),
            rules,
        });
        if let Some(s) = self.sessions.get_mut(&session) {
            s.rooms.push((room_jid.clone(), nick.clone()));
        }
        self.send(session, &presence(&room_jid, &nick, &full, &full, false, true, false));
        let limit = if rules.history {
            self.settings.history_on_join
        } else {
            0
        };
        let pulled = self.federation.room_joined(&kind, limit).await;
        if rules.history {
            let key = room_key(&kind);
            for message in pulled {
                self.remember_mid(&message.mid);
                let stored = NewRoom {
                    room: key.clone(),
                    sender: message.from.to_string(),
                    nick: message.nick,
                    body: message.body,
                    stanza_id: message.stanza_id,
                    mid: Some(message.mid),
                    at: message.at,
                };
                if let Err(e) = self.history.add_room_once(stored).await {
                    warn!("chat: could not store pulled room history: {e}");
                }
            }
            self.send_history(session, &key, &room_jid, &jid.domain).await;
        }
        info!(
            "chat: join c={session} room={} uid={uid} name={} others={}",
            room_class(&kind),
            name_part_of(&nick).map_or_else(|| String::from("?"), |n| log_text(&n, 64)),
            others.len()
        );
    }

    fn rejoin(&mut self, session: u64, kind: &RoomKind, held: &Occupant) {
        let Some(index) = self.room_index(kind) else { return };
        let uid = self.uid_of(session);
        let full = self.full_of(session);
        let others: Vec<Occupant> = self.rooms[index]
            .occupants
            .iter()
            .filter(|o| o.session != session && self.uid_of(o.session) != uid)
            .cloned()
            .collect();
        for other in &others {
            let stanza = presence(
                &held.jid,
                &other.nick,
                &full,
                &self.full_of(other.session),
                false,
                false,
                false,
            );
            self.send(session, &stanza);
        }
        self.send(
            session,
            &presence(&held.jid, &held.nick, &full, &full, false, true, false),
        );
        info!(
            "chat: join c={session} room={} uid={} rejoin",
            room_class(kind),
            uid.as_deref().unwrap_or("-")
        );
    }

    async fn send_history(&mut self, session: u64, key: &str, room_jid: &str, room_domain: &str) {
        if self.settings.history_on_join == 0 {
            return;
        }
        let rows = match self.history.room_history(key, self.settings.history_on_join).await {
            Ok(rows) => rows,
            Err(e) => {
                warn!("chat: could not read room history: {e}");
                return;
            }
        };
        let full = self.full_of(session);
        for row in rows {
            let stanza = format!(
                r#"<message xmlns="jabber:client" type="groupchat"{} from="{}/{}" to="{}"><body>{}</body><delay xmlns="{}" from="{}" stamp="{}"/></message>"#,
                row.stanza_id
                    .as_deref()
                    .map(|id| format!(r#" id="{}""#, escape(id)))
                    .unwrap_or_default(),
                escape(room_jid),
                escape(&row.nick),
                escape(&full),
                escape(&row.body),
                crate::xmpp::NS_DELAY,
                escape(room_domain),
                crate::server::stamp(row.at)
            );
            self.send(session, &stanza);
        }
    }

    /// Presence `unavailable` to `local@domain/r`.
    pub(crate) fn leave(&mut self, session: u64, jid: &Jid) {
        let room_jid = format!("{}@{}", jid.local, jid.domain);
        let held = self.rooms.iter().find_map(|r| {
            r.occupants
                .iter()
                .any(|o| o.session == session && o.jid == room_jid)
                .then(|| r.kind.clone())
        });
        if let Some(kind) = held {
            return self.remove_from_room(&kind, session, Leave::Left);
        }
        let Some(s) = self.sessions.get(&session) else { return };
        let nick = jid.resource.clone().unwrap_or_default();
        if is_muc_domain(&s.domain, &jid.domain) && !nick.is_empty() {
            let full = s.full_jid();
            self.send(session, &presence(&room_jid, &nick, &full, &full, true, true, false));
        }
    }

    pub(crate) fn remove_from_room(&mut self, kind: &RoomKind, session: u64, reason: Leave) {
        let Some(index) = self.room_index(kind) else { return };
        let Some(position) = self.rooms[index].occupants.iter().position(|o| o.session == session) else {
            return;
        };
        let leaving = self.rooms[index].occupants[position].clone();
        let uid = self.uid_of(session);
        let full = self.full_of(session);
        let evicted = reason == Leave::Evicted;
        let others: Vec<Occupant> = self.rooms[index]
            .occupants
            .iter()
            .filter(|o| o.session != session && self.uid_of(o.session) != uid)
            .cloned()
            .collect();
        for other in &others {
            let stanza = presence(
                &other.jid,
                &leaving.nick,
                &self.full_of(other.session),
                &full,
                true,
                false,
                evicted,
            );
            self.send(other.session, &stanza);
        }
        self.rooms[index].occupants.remove(position);
        let ended = match self.sessions.get_mut(&session) {
            Some(s) => {
                if let Some(at) = s.rooms.iter().position(|(jid, _)| *jid == leaving.jid) {
                    s.rooms.remove(at);
                }
                s.ended
            }
            None => true,
        };
        if !ended && matches!(reason, Leave::Left | Leave::Evicted) {
            let stanza = presence(&leaving.jid, &leaving.nick, &full, &full, true, true, evicted);
            self.send(session, &stanza);
        }
        if self.rooms[index].occupants.is_empty() {
            let room = self.rooms.remove(index);
            let federation = self.federation.clone();
            tokio::spawn(async move { federation.room_left(&room.kind).await });
        }
        info!(
            "chat: leave c={session} room={} uid={} reason={}",
            room_class(kind),
            uid.as_deref().unwrap_or("-"),
            reason.as_str()
        );
    }

    pub(crate) fn leave_all(&mut self, session: u64, reason: Leave) {
        let held: Vec<RoomKind> = self
            .rooms
            .iter()
            .filter(|r| r.occupants.iter().any(|o| o.session == session))
            .map(|r| r.kind.clone())
            .collect();
        for kind in held {
            self.remove_from_room(&kind, session, reason);
        }
        if let Some(s) = self.sessions.get_mut(&session) {
            s.rooms.clear();
        }
    }

    fn message_error(&mut self, session: u64, id: Option<&str>, room_jid: &str, kind: &str, condition: &str) {
        let to = self.full_of(session);
        let stanza = format!(
            r#"<message xmlns="jabber:client" type="error"{} from="{}" to="{}"><error type="{kind}"><{condition} xmlns="{NS_STANZAS}"/></error></message>"#,
            id.map(|id| format!(r#" id="{}""#, escape(id))).unwrap_or_default(),
            escape(room_jid),
            escape(&to)
        );
        self.send(session, &stanza);
    }

    /// A `groupchat` message to `local@domain`.
    pub(crate) async fn room_message(&mut self, session: u64, root: &Element, jid: Jid) {
        let now = self.now();
        let room_jid = format!("{}@{}", jid.local, jid.domain);
        let id = root.attr("id").filter(|i| utf16_len(i) <= MAX_ID).map(str::to_owned);
        let body_element = root.child("body");
        if body_element.is_none() && root.child("subject").is_some() {
            return;
        }
        let body = text_of(body_element);
        let length = body.chars().count();
        let held = self.rooms.iter().find_map(|r| {
            r.occupants
                .iter()
                .find(|o| o.session == session && o.jid == room_jid)
                .map(|o| (r.kind.clone(), o.clone()))
        });
        let Some((kind, own)) = held.filter(|_| (1..=BODY_LIMIT).contains(&length)) else {
            return self.message_error(session, id.as_deref(), &room_jid, "modify", "not-acceptable");
        };
        let Some(s) = self.sessions.get_mut(&session) else {
            return;
        };
        if s.muted(now) || !own.rules.may_speak {
            return self.message_error(session, id.as_deref(), &room_jid, "auth", "forbidden");
        }
        if !s.messages.take(MESSAGE_BURST, MESSAGE_REFILL_MS, now) {
            if s.last_limit_log.is_none_or(|at| now - at >= MINUTE) {
                s.last_limit_log = Some(now);
                warn!(
                    "chat: room messages over the limit c={session} uid={}",
                    s.uid.as_deref().unwrap_or("-")
                );
            }
            return self.message_error(session, id.as_deref(), &room_jid, "modify", "not-acceptable");
        }
        let (uid, sender, name) = (s.uid.clone().unwrap_or_default(), s.account.clone(), s.name.clone());
        let Some(sender) = sender else { return };
        let key = room_key(&kind);
        if let Some(command) = crate::moderation::parse_report(&body) {
            return self.report(session, command, "groupchat", room_jid, Some(key)).await;
        }
        let mid = format!("rm-{}", random_hex(16));
        let channel = channel_of(&kind);
        let rated = self.options.moderation.rate(&body, channel);
        let (verdict, shown) = (rated.verdict, rated.shown);
        self.options.moderation.record(Line {
            id: mid.clone(),
            at: now,
            channel: channel.as_str(),
            room_id: Some(key.clone()),
            sender: Person {
                user_id: uid.clone(),
                name: name.unwrap_or_else(|| uid.clone()),
            },
            recipient: None,
            body,
            verdict,
        });
        let full = self.full_of(session);
        let own_from = format!("{room_jid}/{}", own.nick);
        if verdict == Verdict::Blocked {
            self.send(session, &groupchat(id.as_deref(), &own_from, &full, &shown));
            info!(
                "chat: message blocked by the filter c={session} uid={uid} room={}",
                room_class(&kind)
            );
            return;
        }
        let recipients: Vec<Occupant> = self
            .room_index(&kind)
            .map(|i| {
                self.rooms[i]
                    .occupants
                    .iter()
                    .filter(|o| o.session == session || self.uid_of(o.session).as_deref() != Some(uid.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.remember_mid(&mid);
        if own.rules.history {
            let stored = NewRoom {
                room: key.clone(),
                sender: sender.to_string(),
                nick: own.nick.clone(),
                body: shown.clone(),
                stanza_id: id.clone(),
                mid: Some(mid.clone()),
                at: now,
            };
            if let Err(e) = self.history.add_room_once(stored).await {
                warn!("chat: could not store a room message uid={uid}: {e}");
            }
        }
        let (mut delivered, mut blocked) = (0, 0);
        for r in &recipients {
            if r.session != session {
                let account = self.sessions.get(&r.session).and_then(|s| s.account.clone());
                if let Some(account) = account
                    && self.host.blocked(&sender, &account).await
                {
                    blocked += 1;
                    continue;
                }
            }
            let stanza = groupchat(
                id.as_deref(),
                &format!("{}/{}", r.jid, own.nick),
                &self.full_of(r.session),
                &shown,
            );
            self.send(r.session, &stanza);
            delivered += 1;
        }
        if blocked > 0 {
            info!(
                "chat: message c={session} room={} uid={uid} length={length} delivered={delivered} blocked={blocked}",
                room_class(&kind)
            );
        } else {
            info!(
                "chat: message c={session} room={} uid={uid} length={length} delivered={delivered}",
                room_class(&kind)
            );
        }
        let envelope = Envelope::Room {
            mid,
            room: kind,
            from: sender,
            nick: own.nick,
            body: shown,
            id,
            at: now,
            history: own.rules.history,
        };
        if let Err(e) = self.federation.publish(envelope).await {
            warn!("chat: room message not federated: {e}");
        }
    }

    /// Every occupant's rules asked again: updated, or the occupant evicted.
    pub(crate) async fn sweep(&mut self) {
        let plan: Vec<(RoomKind, Vec<u64>)> = self
            .rooms
            .iter()
            .map(|r| (r.kind.clone(), r.occupants.iter().map(|o| o.session).collect()))
            .collect();
        for (kind, sessions) in plan {
            for session in sessions {
                let Some(rules) = self.rules_for(session, &kind).await else {
                    continue;
                };
                if !rules.may_join {
                    self.remove_from_room(&kind, session, Leave::Evicted);
                    continue;
                }
                if let Some(index) = self.room_index(&kind)
                    && let Some(occupant) = self.rooms[index].occupants.iter_mut().find(|o| o.session == session)
                {
                    occupant.rules = rules;
                }
            }
        }
    }

    /// A room message another node published.
    pub(crate) async fn remote_room(&mut self, message: RoomWire) {
        if !self.remember_mid(&message.mid) {
            return;
        }
        let Some(index) = self.room_index(&message.room) else {
            return;
        };
        let occupants = self.rooms[index].occupants.clone();
        if message.history || occupants.iter().any(|o| o.rules.history) {
            let stored = NewRoom {
                room: room_key(&message.room),
                sender: message.from.to_string(),
                nick: message.nick.clone(),
                body: message.body.clone(),
                stanza_id: message.stanza_id.clone(),
                mid: Some(message.mid.clone()),
                at: message.at,
            };
            if let Err(e) = self.history.add_room_once(stored).await {
                warn!("chat: could not store a federated room message: {e}");
            }
        }
        for occupant in occupants {
            let account = self.sessions.get(&occupant.session).and_then(|s| s.account.clone());
            if let Some(account) = account
                && self.host.blocked(&message.from, &account).await
            {
                continue;
            }
            let stanza = groupchat(
                message.stanza_id.as_deref(),
                &format!("{}/{}", occupant.jid, message.nick),
                &self.full_of(occupant.session),
                &message.body,
            );
            self.send(occupant.session, &stanza);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muc_domains_follow_the_account_domain() {
        assert!(is_muc_domain("Dom", "MUC.dom"));
        assert!(is_muc_domain("Dom", "conference.dom"));
        assert!(!is_muc_domain("Dom", "muc.other"));
        assert!(!is_muc_domain("dom", "dom"));
    }

    #[test]
    fn occupant_presence_is_exact() {
        assert_eq!(
            presence("City-a@muc.d", "n", "u@d/r", "o@d/r", false, true, false),
            r#"<presence xmlns="jabber:client" from="City-a@muc.d/n" to="u@d/r"><x xmlns="http://jabber.org/protocol/muc#user"><item affiliation="none" role="participant" jid="o@d/r"/><status code="110"/></x></presence>"#
        );
        assert_eq!(
            presence("r@m", "n", "t", "o", true, false, true),
            r#"<presence xmlns="jabber:client" type="unavailable" from="r@m/n" to="t"><x xmlns="http://jabber.org/protocol/muc#user"><item affiliation="none" role="none" jid="o"/><status code="307"/></x></presence>"#
        );
        assert_eq!(refusal("nick-name"), ("auth", "forbidden"));
        assert_eq!(refusal("limit"), ("wait", "service-unavailable"));
    }
}
