//! From Dauntless Revived's metagame chat at `e1c5fec` (`services/metagame/src/chat/mod.rs`; DR M32-M34,
//! docs/findings/chat.md), with DaintyLass's changes (DD21):
//!
//! - The metagame's state (`AppState`: its tokens, accounts, parties and blocks) is the [`ChatHost`]: a login
//!   is checked by the game module its token belongs to, and the JID and room rules come from that module.
//! - A direct message to someone offline is stored and delivered at their next login, with its time; every
//!   direct message and every message in a room whose rules keep history is stored (`history.rs`).
//! - Mutes: a muted account's room messages are refused and its direct messages dropped.
//! - Friends' online status (DR's `presence.rs`) is not carried: DD21 doesn't name it, and the 1.14.7 rules
//!   (RU5) decide whether it comes back. A broadcast presence is recorded only.
//! - Federation (DN10b) is told what was handled here, through [`Federation`].
//!
//! The rest is DR's: a player logs in with the game's own signed token (SASL PLAIN), never with an account id
//! alone; no stanza, token, SASL payload or message text is logged (`trace` logs frames redacted); bad input
//! never ends a logged-in connection, only the client's `<close/>`, a ping timeout, replacement, shutdown, an
//! oversized frame, unread output or sustained abuse do; replacement, abuse, an oversized frame and unread
//! output hold that account's next login back for 60 s. Pings: about 150 s of silence pass on both sides.
//!
//! One task owns all of chat ([`Chat`]): connections hand it their frames and it answers them one at a time,
//! in order, as the Node server's single thread did. Each connection has a reader and a writer task; the
//! writer counts what waits unsent, Node's `bufferedAmount`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use module_api::chat::{ChatAnswer, ChatQuery, Jid as ModuleJid, RoomKind, RoomRules};
use module_api::{GlobalId, ModuleId, NodeIdentity};
use rand::RngExt;
use tokio::sync::{mpsc, oneshot};

use crate::federation::{Envelope, Federation};
use crate::history::{History, Mute, NewDirect};
use crate::hooks::{self, ChatHost, Login, LoginRefusal};
use crate::muc::{self, Room};
use crate::settings::{NickCheck, Settings};
use crate::xmpp::{
    Bucket, DEFAULT_DOMAIN, Element, NS_BIND, NS_CLIENT, NS_DELAY, NS_FRAMING, NS_PING, NS_SASL, NS_SESSION,
    NS_STANZAS, NS_STREAM_ERRORS, NS_STREAMS, escape, has_control_character, has_markup_declaration, is_xmpp_domain,
    local_name, parse_frame, parse_jid, redact_frame, text_of,
};

pub const FRAME_LIMIT: usize = 32768;
pub const BODY_LIMIT: usize = 2048;
/// Shared by every region; leave headroom beyond the realm's 100+ concurrent players.
const MAX_SOCKETS: usize = 512;
/// Connections that have not bound yet (not logged in, or logged in and not bound), per address
const MAX_UNBOUND_PER_ADDRESS: usize = 8;
const MAX_FRAMES_BEFORE_LOGIN: u32 = 4;
const LOGIN_DEADLINE_MS: i64 = 15 * 1000;
/// From the login: the game binds right after `<success/>`
const BIND_DEADLINE_MS: i64 = 10 * 1000;
const BIND_TIMEOUT_LIMIT: usize = 3;
const BIND_TIMEOUT_WINDOW_MS: i64 = 10 * 60 * 1000;
const SASL_FAILURE_LIMIT: usize = 10;
const SASL_FAILURE_WINDOW_MS: i64 = 10 * 60 * 1000;
const SASL_THROTTLE_MS: i64 = 10 * 60 * 1000;
const UID_THROTTLE_MS: i64 = 60 * 1000;
const IDLE_PING_MS: i64 = 50 * 1000;
const PING_TIMEOUT_MS: i64 = 100 * 1000;
const STANZA_BURST: u32 = 60;
const STANZA_REFILL_MS: f64 = 1000.0 / 30.0;
const ABUSE_DROPS: usize = 100;
const ABUSE_WINDOW_MS: i64 = 60 * 1000;
const LOG_ONCE_MS: i64 = 10 * 60 * 1000;
const LOG_ONCE_MAX: usize = 10000;
pub const OUTPUT_LIMIT: usize = 256 * 1024;
const MAX_RESOURCE: usize = 256;
const MAX_ID: usize = 128;
/// The SASL authcid: the account's name in its game, which becomes a JID local part
const MAX_AUTHCID: usize = 128;
const MAX_SESSIONS_PER_ACCOUNT: usize = 2;
/// Bound or not: two bound sessions (a live one and a ghost) and one login in progress
const MAX_CONNECTIONS_PER_ACCOUNT: usize = 3;
const GHOST_PING_MS: i64 = 10 * 1000;
const SWEEP_MS: i64 = 60 * 1000;

// ---- Sessions ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Close,
    Socket,
    PingTimeout,
    Replaced,
    Abuse,
    Size,
    Backlog,
    Shutdown,
    Timeout,
    Refused,
}

impl EndReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Socket => "socket",
            Self::PingTimeout => "ping-timeout",
            Self::Replaced => "replaced",
            Self::Abuse => "abuse",
            Self::Size => "size",
            Self::Backlog => "backlog",
            Self::Shutdown => "shutdown",
            Self::Timeout => "timeout",
            Self::Refused => "refused",
        }
    }
}

/// What the writer task of a connection is asked to do.
#[derive(Debug)]
pub enum Out {
    Text(String),
    /// A close frame with this code, then the end once the client answers (or soon after)
    Close(u16),
    /// Cut the connection at once
    Terminate,
}

/// The connection's sending side, as the chat task sees it.
#[derive(Debug, Clone)]
pub struct Outbox {
    pub sender: mpsc::UnboundedSender<Out>,
    /// Bytes handed to the writer and not yet written to the socket (`bufferedAmount`)
    pub queued: Arc<AtomicUsize>,
    /// The connection is gone (both directions), for the shutdown's wait
    pub closed: Arc<AtomicBool>,
    /// Cuts the connection at once (`terminate()`)
    pub terminate: Arc<tokio::sync::Notify>,
}

#[derive(Debug)]
pub struct Session {
    pub id: u64,
    pub out: Outbox,
    /// Still open for sending (`readyState === OPEN`): false once a close was sent, the connection
    /// was cut or the client closed it
    pub open: bool,
    pub address: String,
    pub via: &'static str,
    pub opened_at: i64,
    pub domain: String,
    /// The JID local part: the authcid, checked against the module's JID for the account
    pub uid: Option<String>,
    pub account: Option<GlobalId>,
    pub module: Option<ModuleId>,
    pub name_at_login: Option<String>,
    pub logged_in_at: Option<i64>,
    pub resource: Option<String>,
    pub ended: bool,
    early_frames: u32,
    login_failed: bool,
    backlogged: bool,
    pub last_inbound: i64,
    pub ping_id: Option<String>,
    pub ping_deadline: Option<i64>,
    stanzas: Bucket,
    drops: Vec<i64>,
    last_drop_log: i64,
    namespaces: HashSet<String>,
    pub available: bool,
    /// Bare room JID as the client spelled it -> the nickname held there, in join order
    pub rooms: Vec<(String, String)>,
    pub joins: Bucket,
    pub messages: Bucket,
    pub muted: Option<Mute>,
}

impl Session {
    pub fn full_jid(&self) -> String {
        format!(
            "{}@{}/{}",
            self.uid.as_deref().unwrap_or("undefined"),
            self.domain,
            self.resource.as_deref().unwrap_or("undefined")
        )
    }

    pub fn room_nick(&self, room: &str) -> Option<&str> {
        self.rooms.iter().find(|(r, _)| r == room).map(|(_, n)| n.as_str())
    }

    pub fn is_muted(&self, now: i64) -> bool {
        self.muted.as_ref().is_some_and(|mute| mute.holds(now))
    }

    pub fn account_text(&self) -> String {
        self.account.as_ref().map(ToString::to_string).unwrap_or_default()
    }
}

/// What the chat task is asked: by a connection (opened, a frame, gone), the upgrade check, or the plugin's
/// handle (a mute, shutdown).
#[derive(Debug)]
pub enum Event {
    /// May this address open one more connection?
    Admit {
        address: String,
        via: &'static str,
        reply: oneshot::Sender<bool>,
    },
    Connected {
        address: String,
        via: &'static str,
        out: Outbox,
        reply: oneshot::Sender<u64>,
    },
    Frame {
        id: u64,
        text: String,
    },
    /// The connection ended on its own (the client closed it, an error, an oversized frame)
    Gone {
        id: u64,
        size: bool,
    },
    /// A mute was set (or lifted, `None`) in the database: the account's live sessions follow it
    Muted {
        account: GlobalId,
        mute: Option<Mute>,
    },
    /// A direct message delivered from another node (DN10b)
    RemoteDirect {
        message: crate::federation::DirectWire,
        reply: oneshot::Sender<bool>,
    },
    /// A room message fanned out or replicated from another node (DN10b)
    RemoteRoom {
        message: crate::federation::RoomWire,
    },
    Shutdown(oneshot::Sender<()>),
    /// One tick now, for a test that turned the timer off
    Tick(oneshot::Sender<()>),
}

/// Log keys -> when last logged, oldest first (at most `LOG_ONCE_MAX`).
#[derive(Debug, Default)]
struct LogOnce {
    at: HashMap<String, (i64, u64)>,
    order: BTreeMap<u64, String>,
    next: u64,
}

impl LogOnce {
    fn check(&mut self, key: &str, window_ms: i64, now: i64) -> bool {
        if let Some((last, _)) = self.at.get(key)
            && now - last < window_ms
        {
            return false;
        }
        if let Some((_, seq)) = self.at.remove(key) {
            self.order.remove(&seq);
        }
        while self.at.len() >= LOG_ONCE_MAX {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.at.remove(&oldest);
        }
        self.next += 1;
        self.at.insert(key.to_owned(), (now, self.next));
        self.order.insert(self.next, key.to_owned());
        true
    }

    fn prune(&mut self, now: i64) {
        let old: Vec<String> = self
            .at
            .iter()
            .filter(|(_, (at, _))| now - at >= LOG_ONCE_MS)
            .map(|(k, _)| k.clone())
            .collect();
        for key in old {
            if let Some((_, seq)) = self.at.remove(&key) {
                self.order.remove(&seq);
            }
        }
    }
}

/// The clock chat reads: the real one, or a test's.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Sees every stanza sent, with the receiving session's JID local part (DR's `ObserveChatOutputForTests`).
pub type Observer = Arc<dyn Fn(Option<&str>, &str) + Send + Sync>;

/// The game modules that declared the chat capability (`CorePlugin::attach`); the others get the defaults.
pub type ChatModules = Arc<Mutex<BTreeSet<ModuleId>>>;

/// What chat runs against.
#[derive(Clone)]
pub struct Backends {
    pub host: Arc<dyn ChatHost>,
    pub federation: Arc<dyn Federation>,
    pub history: History,
    pub node: NodeIdentity,
    pub modules: ChatModules,
}

/// Everything chat holds. One task owns it (`run`).
pub struct Chat {
    pub backends: Backends,
    /// In connection order (ids only grow)
    pub sessions: BTreeMap<u64, Session>,
    /// Bound sessions per JID local part, oldest first
    pub by_uid: HashMap<String, Vec<u64>>,
    sasl_failures: HashMap<String, Vec<i64>>,
    throttled_addresses: HashMap<String, i64>,
    throttled_uids: HashMap<String, i64>,
    logged_once: LogOnce,
    bind_timeouts: HashMap<String, Vec<i64>>,
    clock: Clock,
    trace: bool,
    pub nick_check: NickCheck,
    output_limit: usize,
    pub(crate) history_on_join: i64,
    offline_batch: i64,
    /// In creation order
    pub rooms: Vec<Room>,
    seen_mids: HashSet<String>,
    seen_mids_order: std::collections::VecDeque<String>,
    next_id: u64,
    next_ping: u64,
    last_sweep: i64,
    auto_tick: bool,
    observe: Option<Observer>,
    /// The most unsent output held for one session (`LargestBacklog`)
    pub largest_backlog: Arc<AtomicUsize>,
    pub(crate) moderation: crate::Moderation,
}

/// What a test may change.
#[derive(Clone)]
pub struct Options {
    pub clock: Clock,
    pub output_limit: usize,
    /// Ticks every second (false: only when asked)
    pub auto_tick: bool,
    pub observe: Option<Observer>,
    /// The staff dashboard's filter, chat log and reports (off by default)
    pub moderation: crate::Moderation,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            clock: Arc::new(now_ms),
            output_limit: OUTPUT_LIMIT,
            auto_tick: true,
            observe: None,
            moderation: crate::Moderation::off(),
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// `n` random bytes in lower-case hex (DR's `crypto::random_hex`).
pub fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::rng().fill(&mut bytes[..]);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// XEP-0203's stamp: UTC, milliseconds.
pub fn stamp(at: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Chat {
    pub fn new(backends: Backends, settings: &Settings, options: Options) -> Self {
        Self {
            backends,
            sessions: BTreeMap::new(),
            by_uid: HashMap::new(),
            sasl_failures: HashMap::new(),
            throttled_addresses: HashMap::new(),
            throttled_uids: HashMap::new(),
            logged_once: LogOnce::default(),
            bind_timeouts: HashMap::new(),
            clock: options.clock,
            trace: settings.trace,
            nick_check: settings.nick_check,
            output_limit: options.output_limit,
            history_on_join: settings.history_on_join,
            offline_batch: settings.offline_batch,
            rooms: Vec::new(),
            seen_mids: HashSet::new(),
            seen_mids_order: std::collections::VecDeque::new(),
            next_id: 1,
            next_ping: 1,
            last_sweep: 0,
            auto_tick: options.auto_tick,
            observe: options.observe,
            largest_backlog: Arc::default(),
            moderation: options.moderation,
        }
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Records `mid` as seen on this node; returns `true` when `mid` is new.
    pub(crate) fn remember_mid(&mut self, mid: &str) -> bool {
        if mid.is_empty() {
            return true;
        }
        if !self.seen_mids.insert(mid.to_owned()) {
            return false;
        }
        self.seen_mids_order.push_back(mid.to_owned());
        while self.seen_mids_order.len() > 4096 {
            if let Some(old) = self.seen_mids_order.pop_front() {
                self.seen_mids.remove(&old);
            }
        }
        true
    }

    pub fn log_once(&mut self, key: &str, window_ms: Option<i64>) -> bool {
        let now = self.now();
        self.logged_once.check(key, window_ms.unwrap_or(LOG_ONCE_MS), now)
    }

    /// A session that is still in the server (not ended and removed).
    pub fn session(&self, id: u64) -> Option<&Session> {
        self.sessions.get(&id)
    }

    /// The bound sessions of a JID local part, oldest first.
    pub fn sessions_of(&self, uid: &str) -> Vec<u64> {
        self.by_uid.get(uid).cloned().unwrap_or_default()
    }

    // ---- The modules ----

    pub(crate) fn answers_chat(&self, module: &ModuleId) -> bool {
        self.backends
            .modules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(module)
    }

    /// The account's JID in its game, or the default for a module that doesn't answer; `None` refuses.
    async fn jid_of(&self, module: &ModuleId, account: &GlobalId, authcid: &str, domain: &str) -> Option<ModuleJid> {
        if !self.answers_chat(module) {
            return hooks::default_jid(authcid, domain);
        }
        let query = ChatQuery::Jid {
            account: account.clone(),
        };
        match self.backends.host.ask(module, query).await {
            Ok(ChatAnswer::Jid(jid)) => Some(jid),
            Ok(_) => None,
            Err(error) if hooks::is_unsupported(&error) => hooks::default_jid(authcid, domain),
            Err(error) => {
                tracing::warn!("chat: module {module} did not give a JID ({})", error.kind);
                None
            }
        }
    }

    /// What the session's player may do in a room, or `None` when the module could not say (refused).
    pub async fn rules_of(&self, id: u64, room: &RoomKind) -> Option<RoomRules> {
        let session = self.sessions.get(&id)?;
        let (module, account) = (session.module.clone()?, session.account.clone()?);
        if !self.answers_chat(&module) {
            return Some(hooks::default_rules(room));
        }
        let query = ChatQuery::Room {
            room: room.clone(),
            account,
        };
        match self.backends.host.ask(&module, query).await {
            Ok(ChatAnswer::Room(rules)) => Some(rules),
            Ok(_) => None,
            Err(error) if hooks::is_unsupported(&error) => Some(hooks::default_rules(room)),
            Err(error) => {
                tracing::warn!("chat: module {module} did not give room rules ({})", error.kind);
                None
            }
        }
    }

    async fn may_whisper(&self, module: Option<&ModuleId>, from: &GlobalId, to: &GlobalId) -> bool {
        let Some(module) = module.filter(|m| self.answers_chat(m)) else {
            return true;
        };
        let query = ChatQuery::MayWhisper {
            from: from.clone(),
            to: to.clone(),
        };
        !matches!(
            self.backends.host.ask(module, query).await,
            Ok(ChatAnswer::MayWhisper(false))
        )
    }

    pub async fn publish(&self, envelope: Envelope) {
        if let Err(error) = self.backends.federation.publish(envelope).await {
            tracing::warn!("chat: federation: {error}");
        }
    }

    // ---- Output ----

    fn is_backlogged(&self, session: &Session) -> bool {
        session.backlogged || session.out.queued.load(Ordering::Relaxed) > self.output_limit
    }

    /// A client that does not read its output gets no more once `output_limit` bytes wait unsent; the
    /// session ends at the next frame or tick (ending it here would change rooms mid fan-out).
    pub fn send(&mut self, id: u64, stanza: &str) {
        let (trace, limit) = (self.trace, self.output_limit);
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if !session.open {
            return;
        }
        if !session.ended && (session.backlogged || session.out.queued.load(Ordering::Relaxed) > limit) {
            session.backlogged = true;
            return;
        }
        if trace {
            tracing::info!("chat: trace c={} out {}", session.id, redact_frame(stanza));
        }
        if let Some(observe) = &self.observe {
            observe(session.uid.as_deref(), stanza);
        }
        let queued = session.out.queued.fetch_add(stanza.len(), Ordering::Relaxed) + stanza.len();
        self.largest_backlog.fetch_max(queued, Ordering::Relaxed);
        if session.out.sender.send(Out::Text(stanza.to_owned())).is_err() {
            session.open = false;
        }
    }

    fn close_socket(&mut self, id: u64, how: Out) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.open = false;
            let _ = session.out.sender.send(how);
        }
    }

    // ---- Connections ----

    fn admit(&mut self, address: &str, via: &str) -> bool {
        let unbound = self
            .sessions
            .values()
            .filter(|s| s.address == address && s.resource.is_none())
            .count();
        if self.sessions.len() >= MAX_SOCKETS || unbound >= MAX_UNBOUND_PER_ADDRESS {
            if self.log_once(&format!("busy|{address}"), None) {
                tracing::warn!(
                    "chat: refused a connection from={address} via={via} ({})",
                    if self.sessions.len() >= MAX_SOCKETS {
                        "too many connections"
                    } else {
                        "too many logins in progress from this address"
                    }
                );
            }
            return false;
        }
        true
    }

    fn connected(&mut self, address: String, via: &'static str, out: Outbox) -> u64 {
        let now = self.now();
        let id = self.next_id;
        self.next_id += 1;
        tracing::info!("chat: connect c={id} from={address} via={via}");
        self.sessions.insert(
            id,
            Session {
                id,
                out,
                open: true,
                address,
                via,
                opened_at: now,
                domain: DEFAULT_DOMAIN.to_owned(),
                uid: None,
                account: None,
                module: None,
                name_at_login: None,
                logged_in_at: None,
                resource: None,
                ended: false,
                early_frames: 0,
                login_failed: false,
                backlogged: false,
                last_inbound: now,
                ping_id: None,
                ping_deadline: None,
                stanzas: Bucket::new(STANZA_BURST, now),
                drops: Vec::new(),
                last_drop_log: 0,
                namespaces: HashSet::new(),
                available: false,
                rooms: Vec::new(),
                joins: Bucket::new(muc::JOIN_BURST, now),
                messages: Bucket::new(muc::MESSAGE_BURST, now),
                muted: None,
            },
        );
        id
    }

    /// Ends a session once. Replacement, abuse, an oversized frame and unread output hold that
    /// account's next login back for 60 s; a ping timeout does not.
    pub async fn end(&mut self, id: u64, reason: EndReason) {
        let now = self.now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.ended {
            return;
        }
        session.ended = true;
        let leave = if reason == EndReason::Replaced {
            muc::LeaveReason::Replaced
        } else {
            muc::LeaveReason::Disconnect
        };
        self.leave_all(id, leave);
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let (uid, account, resource, opened_at) = (
            session.uid.clone(),
            session.account.clone(),
            session.resource.clone(),
            session.opened_at,
        );
        if let (Some(uid), Some(_)) = (&uid, &resource) {
            let list: Vec<u64> = self.sessions_of(uid).into_iter().filter(|other| *other != id).collect();
            if list.is_empty() {
                self.by_uid.remove(uid);
                if let Some(account) = &account {
                    self.backends.federation.presence(account, uid, false).await;
                    self.backends.host.presence(account, uid, false).await;
                }
            } else {
                self.by_uid.insert(uid.clone(), list);
            }
            if matches!(
                reason,
                EndReason::Replaced | EndReason::Abuse | EndReason::Size | EndReason::Backlog
            ) {
                self.throttled_uids.insert(uid.clone(), now + UID_THROTTLE_MS);
            }
        }
        tracing::info!(
            "chat: closed c={id} uid={} reason={} after={}s",
            uid.as_deref().unwrap_or("-"),
            reason.as_str(),
            ((now - opened_at) as f64 / 1000.0).round() as i64
        );
        if self.sessions.get(&id).is_some_and(|s| s.open) {
            match reason {
                EndReason::Close | EndReason::Shutdown | EndReason::Replaced => {
                    self.send(id, &format!("<close xmlns=\"{NS_FRAMING}\"/>"));
                    self.close_socket(id, Out::Close(1000));
                }
                EndReason::Size => self.close_socket(id, Out::Close(1009)),
                _ => self.close_socket(id, Out::Terminate),
            }
        }
        self.sessions.remove(&id);
    }

    pub fn ping(&mut self, id: u64, timeout_ms: i64) {
        let now = self.now();
        let ping_id = format!("sp{}", self.next_ping);
        self.next_ping += 1;
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.ping_id = Some(ping_id.clone());
        session.ping_deadline = Some(now + timeout_ms);
        let stanza = format!(
            "<iq xmlns=\"{NS_CLIENT}\" type=\"get\" id=\"{ping_id}\" from=\"{}\" to=\"{}\"><ping xmlns=\"{NS_PING}\"/></iq>",
            escape(&session.domain),
            escape(&session.full_jid())
        );
        self.send(id, &stanza);
    }

    // ---- Throttles ----

    /// Three logins of one account in 10 minutes that never bound: its logins wait 60 s
    fn bind_timed_out(&mut self, uid: &str, now: i64) {
        let recent = self.bind_timeouts.entry(uid.to_owned()).or_default();
        recent.retain(|at| now - at < BIND_TIMEOUT_WINDOW_MS);
        recent.push(now);
        if recent.len() >= BIND_TIMEOUT_LIMIT {
            self.throttled_uids.insert(uid.to_owned(), now + UID_THROTTLE_MS);
        }
    }

    fn count_failure(&mut self, address: &str, now: i64) {
        let recent = self.sasl_failures.entry(address.to_owned()).or_default();
        recent.retain(|at| now - at < SASL_FAILURE_WINDOW_MS);
        recent.push(now);
        if recent.len() >= SASL_FAILURE_LIMIT {
            self.throttled_addresses
                .insert(address.to_owned(), now + SASL_THROTTLE_MS);
        }
    }

    fn prune(&mut self, now: i64) {
        self.throttled_addresses.retain(|_, until| *until > now);
        self.throttled_uids.retain(|_, until| *until > now);
        self.logged_once.prune(now);
        self.bind_timeouts
            .retain(|_, times| !times.iter().all(|at| now - at >= BIND_TIMEOUT_WINDOW_MS));
        self.sasl_failures.retain(|_, times| {
            times.retain(|at| now - at < SASL_FAILURE_WINDOW_MS);
            !times.is_empty()
        });
    }

    /// A dropped stanza; past `ABUSE_DROPS` in a minute the session ends.
    pub async fn drop_stanza(&mut self, id: u64, reason: &str) {
        let now = self.now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.drops.retain(|at| now - at < ABUSE_WINDOW_MS);
        session.drops.push(now);
        if now - session.last_drop_log >= 60 * 1000 {
            session.last_drop_log = now;
            tracing::info!("chat: dropped c={id} reason={reason}");
        }
        if session.drops.len() > ABUSE_DROPS {
            self.end(id, EndReason::Abuse).await;
        }
    }

    // ---- The tick ----

    /// The deadlines, pings and sweeps, every second.
    pub async fn tick(&mut self) {
        let now = self.now();
        let ids: Vec<u64> = self.sessions.keys().copied().collect();
        for id in ids {
            let Some(session) = self.sessions.get(&id) else {
                continue;
            };
            if session.ended {
                continue;
            }
            if self.is_backlogged(session) {
                self.end(id, EndReason::Backlog).await;
            } else if session.uid.is_none() && now - session.opened_at >= LOGIN_DEADLINE_MS {
                self.end(id, EndReason::Timeout).await;
            } else if let (Some(uid), None) = (&session.uid, &session.resource) {
                if now - session.logged_in_at.unwrap_or(session.opened_at) >= BIND_DEADLINE_MS {
                    let uid = uid.clone();
                    self.bind_timed_out(&uid, now);
                    self.end(id, EndReason::Timeout).await;
                }
            } else if session.resource.is_some() {
                match session.ping_deadline {
                    Some(deadline) => {
                        if now >= deadline {
                            self.end(id, EndReason::PingTimeout).await;
                        }
                    }
                    None => {
                        if now - session.last_inbound >= IDLE_PING_MS {
                            self.ping(id, PING_TIMEOUT_MS);
                        }
                    }
                }
            }
        }
        if now - self.last_sweep >= SWEEP_MS {
            self.last_sweep = now;
            self.sweep_rooms().await;
        }
        self.backends.federation.tick(now).await;
        self.prune(now);
    }

    // ---- Stanzas ----

    pub async fn frame(&mut self, id: u64, raw: &str) {
        let now = self.now();
        let trace = self.trace;
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.ended {
            return;
        }
        session.last_inbound = now;
        session.ping_deadline = None;
        session.ping_id = None;
        if trace {
            tracing::info!("chat: trace c={id} in {}", redact_frame(raw));
        }
        if self.sessions.get(&id).is_some_and(|s| self.is_backlogged(s)) {
            self.end(id, EndReason::Backlog).await;
            return;
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        // Before login the game sends <open>, <auth> and, after a refused login, its legacy login: a
        // few frames at most
        if session.uid.is_none() {
            session.early_frames += 1;
            if session.early_frames > MAX_FRAMES_BEFORE_LOGIN {
                let address = session.address.clone();
                self.count_failure(&address, now);
                self.end(id, EndReason::Refused).await;
                return;
            }
        } else if !session.stanzas.take(STANZA_BURST, STANZA_REFILL_MS, now) {
            self.drop_stanza(id, "rate").await;
            return;
        }
        let logged_in = self.sessions.get(&id).is_some_and(|s| s.uid.is_some());
        // No DTDs or processing instructions, ever. Before login that ends the connection; after it the
        // frame is dropped (the game never sends one, and closing would start a reconnect loop).
        if has_markup_declaration(raw) {
            if logged_in {
                self.drop_stanza(id, "parse").await;
            } else {
                self.send(
                    id,
                    &format!(
                        "<stream:error xmlns:stream=\"{NS_STREAMS}\"><restricted-xml xmlns=\"{NS_STREAM_ERRORS}\"/></stream:error>"
                    ),
                );
                self.end(id, EndReason::Refused).await;
            }
            return;
        }
        let Some(node) = parse_frame(raw) else {
            if logged_in {
                self.drop_stanza(id, "parse").await;
            } else {
                self.end(id, EndReason::Refused).await;
            }
            return;
        };
        let kind = local_name(&node.name);
        if kind == "open" {
            self.open(id, &node);
            return;
        }
        if kind == "close" {
            self.end(id, EndReason::Close).await;
            return;
        }
        if !logged_in {
            self.before_login(id, &node, &kind).await;
            return;
        }
        if kind == "iq" {
            self.iq(id, &node).await;
            return;
        }
        if self.sessions.get(&id).is_some_and(|s| s.resource.is_none()) {
            self.drop_stanza(id, "unknown").await;
            return;
        }
        match kind.as_str() {
            "presence" => self.presence(id, &node).await,
            "message" => self.message(id, &node).await,
            _ => self.drop_stanza(id, "unknown").await,
        }
    }

    fn open(&mut self, id: u64, node: &Element) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.resource.is_none() {
            match node.attr("to").filter(|to| is_xmpp_domain(to)) {
                Some(to) => session.domain = to.to_owned(),
                None => {
                    if session.uid.is_none() {
                        session.domain = DEFAULT_DOMAIN.to_owned();
                    }
                }
            }
        }
        let (domain, logged_in) = (escape(&session.domain), session.uid.is_some());
        self.send(
            id,
            &format!(
                "<open xmlns=\"{NS_FRAMING}\" from=\"{domain}\" id=\"{}\" version=\"1.0\" xml:lang=\"en\"/>",
                random_hex(8)
            ),
        );
        self.send(
            id,
            &if logged_in {
                format!("<stream:features xmlns:stream=\"{NS_STREAMS}\"><bind xmlns=\"{NS_BIND}\"/></stream:features>")
            } else {
                format!(
                    "<stream:features xmlns:stream=\"{NS_STREAMS}\"><mechanisms xmlns=\"{NS_SASL}\"><mechanism>PLAIN</mechanism></mechanisms></stream:features>"
                )
            },
        );
    }

    async fn before_login(&mut self, id: u64, node: &Element, kind: &str) {
        if kind == "auth" {
            self.auth(id, node).await;
            return;
        }
        // After a failed SASL the game tries legacy jabber:iq:auth: refuse it and hang up
        if kind == "iq" {
            let iq_id = node.attr("id").unwrap_or("");
            let id_attr = if !iq_id.is_empty() && iq_id.encode_utf16().count() <= MAX_ID {
                format!(" id=\"{}\"", escape(iq_id))
            } else {
                String::new()
            };
            self.send(
                id,
                &format!(
                    "<iq xmlns=\"{NS_CLIENT}\" type=\"error\"{id_attr}><error type=\"auth\"><not-authorized xmlns=\"{NS_STANZAS}\"/></error></iq>"
                ),
            );
        }
        self.end(id, EndReason::Refused).await;
    }

    fn refuse_login(&mut self, id: u64, reason: &str, uid: Option<&str>, condition: &str) {
        let now = self.now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.login_failed = true;
        let address = session.address.clone();
        if reason != "throttled" && reason != "unavailable" {
            self.count_failure(&address, now);
        }
        if self.log_once(&format!("login|{}|{reason}", uid.unwrap_or(&address)), None) {
            tracing::info!(
                "chat: login refused c={id} reason={reason}{}",
                uid.map(|u| format!(" uid={u}")).unwrap_or_default()
            );
        }
        self.send(id, &format!("<failure xmlns=\"{NS_SASL}\"><{condition}/></failure>"));
    }

    async fn auth(&mut self, id: u64, node: &Element) {
        let now = self.now();
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        // One SASL attempt per connection: after a refusal the game tries its legacy login, never <auth>
        if session.login_failed {
            self.end(id, EndReason::Refused).await;
            return;
        }
        if self
            .throttled_addresses
            .get(&session.address)
            .is_some_and(|until| *until > now)
        {
            self.refuse_login(id, "throttled", None, "temporary-auth-failure");
            return;
        }
        if node.attr("mechanism") != Some("PLAIN") {
            self.refuse_login(id, "bad-mechanism", None, "invalid-mechanism");
            return;
        }
        let text = node.text();
        let encoded = service_kit::js::trim(&text);
        let valid_shape = !encoded.is_empty() && encoded.len() <= 12000 && {
            let body = encoded.trim_end_matches('=');
            encoded.len() - body.len() <= 2
                && !body.is_empty()
                && body
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
        };
        if !valid_shape {
            self.refuse_login(id, "bad-format", None, "not-authorized");
            return;
        }
        let decoded = String::from_utf8_lossy(&decode_base64_like_node(encoded)).into_owned();
        let fields: Vec<&str> = decoded.split('\0').collect();
        if fields.len() != 3
            || (!fields[0].is_empty() && fields[0] != fields[1])
            || !is_authcid_shape(fields[1])
            || fields[2].is_empty()
            || fields[2].encode_utf16().count() > 8192
        {
            self.refuse_login(id, "bad-format", None, "not-authorized");
            return;
        }
        let (uid, token) = (fields[1].to_owned(), fields[2].to_owned());
        if self.throttled_uids.get(&uid).is_some_and(|until| *until > now) {
            self.refuse_login(id, "throttled", Some(&uid), "temporary-auth-failure");
            return;
        }
        let domain = self.sessions.get(&id).map(|s| s.domain.clone()).unwrap_or_default();
        let login = Login {
            authcid: &uid,
            token: &token,
            domain: &domain,
        };
        let player = match self.backends.host.login(login).await {
            Ok(player) => player,
            Err(refusal) => {
                let condition = if refusal == LoginRefusal::Unavailable {
                    "temporary-auth-failure"
                } else {
                    "not-authorized"
                };
                self.refuse_login(id, refusal.as_str(), Some(&uid), condition);
                return;
            }
        };
        // The identity is the module's JID for the token's account, never the name the client sent
        let Some(jid) = self.jid_of(&player.module, &player.account, &uid, &domain).await else {
            self.refuse_login(id, "unavailable", Some(&uid), "temporary-auth-failure");
            return;
        };
        if jid.local() != uid {
            self.refuse_login(id, "uid-mismatch", Some(&uid), "not-authorized");
            return;
        }
        let account_text = player.account.to_string();
        let mut mute = match self.backends.history.mute_of(&account_text).await {
            Ok(mute) => mute,
            Err(error) => {
                tracing::error!("chat: reading mutes failed ({error})");
                self.refuse_login(id, "unavailable", Some(&uid), "temporary-auth-failure");
                return;
            }
        };
        if mute.is_none() {
            mute = self.backends.federation.mute_of(&player.account).await;
        }
        if let Err(error) = self
            .backends
            .history
            .remember_account(&uid, &account_text, player.module.as_str(), jid.domain(), now)
            .await
        {
            tracing::warn!("chat: could not record uid={uid} ({error})");
        }
        // At most three connections per account, bound or not: past that the oldest one that has not
        // bound goes (bound ones are limited to two at the bind)
        let mine: Vec<(u64, bool)> = self
            .sessions
            .values()
            .filter(|other| other.id != id && !other.ended && other.uid.as_deref() == Some(uid.as_str()))
            .map(|other| (other.id, other.resource.is_none()))
            .collect();
        let excess = (mine.len() + 1).saturating_sub(MAX_CONNECTIONS_PER_ACCOUNT);
        for old in mine
            .iter()
            .filter(|(_, unbound)| *unbound)
            .take(excess)
            .map(|(i, _)| *i)
            .collect::<Vec<_>>()
        {
            self.end(old, EndReason::Replaced).await;
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.uid = Some(uid.clone());
        session.account = Some(player.account);
        session.module = Some(player.module);
        session.name_at_login = Some(player.name);
        session.logged_in_at = Some(now);
        session.muted = mute;
        tracing::info!("chat: login ok c={id} uid={uid}");
        self.send(id, &format!("<success xmlns=\"{NS_SASL}\"/>"));
    }

    async fn iq(&mut self, id: u64, node: &Element) {
        let kind = node.attr("type");
        let iq_id = node.attr("id").unwrap_or("").to_owned();
        if matches!(kind, Some("result" | "error")) {
            return;
        }
        if !matches!(kind, Some("get" | "set")) || iq_id.encode_utf16().count() > MAX_ID {
            self.drop_stanza(id, "unknown").await;
            return;
        }
        if kind == Some("set")
            && let Some(binding) = node.child("bind")
        {
            let requested = text_of(binding.child("resource"));
            self.bind(id, &iq_id, &requested).await;
            return;
        }
        // Pings, the never-advertised session IQ, and anything else: an empty result. The namespace of
        // an unknown one is logged once per session.
        let namespace = node
            .elements()
            .next()
            .map(|child| child.attr("xmlns").unwrap_or("").to_owned());
        if let Some(namespace) = namespace
            && namespace != NS_PING
            && namespace != NS_SESSION
            && let Some(session) = self.sessions.get_mut(&id)
            && !session.namespaces.contains(&namespace)
            && session.namespaces.len() < 32
        {
            tracing::debug!(
                "chat: iq c={id} {} namespace={}",
                kind.unwrap_or(""),
                namespace.chars().take(80).collect::<String>()
            );
            session.namespaces.insert(namespace);
        }
        self.send(
            id,
            &format!("<iq xmlns=\"{NS_CLIENT}\" type=\"result\" id=\"{}\"/>", escape(&iq_id)),
        );
    }

    async fn bind(&mut self, id: u64, iq_id: &str, requested: &str) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let uid = session.uid.clone().unwrap_or_default();
        let bound = |jid: &str| {
            format!(
                "<iq xmlns=\"{NS_CLIENT}\" type=\"result\" id=\"{}\"><bind xmlns=\"{NS_BIND}\"><jid>{}</jid></bind></iq>",
                escape(iq_id),
                escape(jid)
            )
        };
        if session.resource.is_some() {
            let stanza = bound(&session.full_jid());
            self.send(id, &stanza);
            return;
        }
        if requested.encode_utf16().count() > MAX_RESOURCE || has_control_character(requested) {
            self.send(
                id,
                &format!(
                    "<iq xmlns=\"{NS_CLIENT}\" type=\"error\" id=\"{}\"><error type=\"modify\"><bad-request xmlns=\"{NS_STANZAS}\"/></error></iq>",
                    escape(iq_id)
                ),
            );
            return;
        }
        let resource = if requested.is_empty() {
            format!("srv-{}", random_hex(16))
        } else {
            requested.to_owned()
        };
        // At most two sessions per account. The same resource again is a ghost or a forgery, and a
        // third session replaces the one silent longest.
        let same = self.sessions_of(&uid).into_iter().find(|other| {
            self.sessions
                .get(other)
                .is_some_and(|s| s.resource.as_deref() == Some(resource.as_str()))
        });
        if let Some(same) = same {
            self.end(same, EndReason::Replaced).await;
        }
        let remaining = self.sessions_of(&uid);
        if remaining.len() >= MAX_SESSIONS_PER_ACCOUNT {
            // The first of the least recently heard, as reduce keeps
            let mut oldest: Option<(u64, i64)> = None;
            for other in remaining {
                let heard = self.sessions.get(&other).map_or(i64::MAX, |s| s.last_inbound);
                if oldest.is_none_or(|(_, at)| heard < at) {
                    oldest = Some((other, heard));
                }
            }
            if let Some((other, _)) = oldest {
                self.end(other, EndReason::Replaced).await;
            }
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.resource = Some(resource.clone());
        let account = session.account.clone();
        let domain = session.domain.clone();
        let jid = session.full_jid();
        self.by_uid.entry(uid.clone()).or_default().push(id);
        tracing::info!(
            "chat: bound c={id} uid={uid} resource={resource} domain={domain} sessions={}",
            self.sessions_of(&uid).len()
        );
        self.send(id, &bound(&jid));
        // An older session of the same account that does not answer a ping within 10 s is a ghost
        let now = self.now();
        for other in self.sessions_of(&uid) {
            if other != id
                && self
                    .sessions
                    .get(&other)
                    .is_some_and(|s| s.ping_deadline.is_none_or(|deadline| deadline > now + GHOST_PING_MS))
            {
                self.ping(other, GHOST_PING_MS);
            }
        }
        if let Some(account) = &account {
            self.backends.federation.presence(account, &uid, true).await;
            self.backends.host.presence(account, &uid, true).await;
        }
        self.deliver_waiting(id).await;
    }

    /// Direct messages that waited for this account, oldest first, each with the time it was sent (XEP-0203).
    async fn deliver_waiting(&mut self, id: u64) {
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let (account_id, account, domain, to) = (
            session.account.clone(),
            session.account_text(),
            session.domain.clone(),
            session.full_jid(),
        );
        let waiting = match self.backends.history.pending_for(&account, self.offline_batch).await {
            Ok(waiting) => waiting,
            Err(error) => {
                tracing::error!("chat: reading waiting messages failed ({error})");
                return;
            }
        };
        let mut delivered = Vec::new();
        for message in &waiting {
            if let Some(mid) = &message.mid
                && !self.remember_mid(mid)
            {
                delivered.push(message.id);
                continue;
            }
            let id_attr = message
                .stanza_id
                .as_deref()
                .map(|i| format!(" id=\"{}\"", escape(i)))
                .unwrap_or_default();
            self.send(
                id,
                &format!(
                    "<message xmlns=\"{NS_CLIENT}\" type=\"chat\" from=\"{}\" to=\"{}\"{id_attr}><body>{}</body><delay xmlns=\"{NS_DELAY}\" from=\"{}\" stamp=\"{}\"/></message>",
                    escape(&format!("{}@{domain}", message.sender_local)),
                    escape(&to),
                    escape(&message.body),
                    escape(&domain),
                    stamp(message.at)
                ),
            );
            delivered.push(message.id);
        }
        let mut count = delivered.len();
        let now = self.now();
        if !delivered.is_empty()
            && let Err(error) = self.backends.history.mark_delivered(&account, delivered, now).await
        {
            tracing::error!("chat: marking messages delivered failed ({error})");
        }
        if let Some(account_id) = account_id
            && self.backends.federation.is_federated()
            && account_id.owner() != &self.backends.node.node
        {
            let remote_waiting = self
                .backends
                .federation
                .pull_waiting(&account_id, self.offline_batch)
                .await;
            let mut acked_mids = Vec::new();
            let mut stanzas = Vec::new();
            for message in remote_waiting {
                if !self.remember_mid(&message.mid) {
                    acked_mids.push(message.mid);
                    continue;
                }
                let id_attr = message
                    .stanza_id
                    .as_deref()
                    .map(|i| format!(" id=\"{}\"", escape(i)))
                    .unwrap_or_default();
                let _ = self
                    .backends
                    .history
                    .add_direct_once(NewDirect {
                        sender: message.from.to_string(),
                        sender_local: message.from_local.clone(),
                        recipient: message.to.to_string(),
                        body: message.body.clone(),
                        stanza_id: message.stanza_id.clone(),
                        at: message.at,
                        delivered_at: Some(now),
                        mid: Some(message.mid.clone()),
                    })
                    .await;
                stanzas.push(format!(
                    "<message xmlns=\"{NS_CLIENT}\" type=\"chat\" from=\"{}\" to=\"{}\"{id_attr}><body>{}</body><delay xmlns=\"{NS_DELAY}\" from=\"{}\" stamp=\"{}\"/></message>",
                    escape(&format!("{}@{domain}", message.from_local)),
                    escape(&to),
                    escape(&message.body),
                    escape(&domain),
                    stamp(message.at)
                ));
                acked_mids.push(message.mid);
                count += 1;
            }
            if !acked_mids.is_empty() {
                self.backends.federation.ack_waiting(&account_id, acked_mids, now).await;
            }
            for stanza in stanzas {
                self.send(id, &stanza);
            }
        }
        if count > 0 {
            tracing::info!("chat: offline messages c={id} delivered={count}");
        }
    }

    // ---- Presence and messages ----

    async fn presence(&mut self, id: u64, node: &Element) {
        let kind = node.attr("type");
        // A broadcast presence (no "to") is recorded; it is never echoed to its sender or to another
        // session of the same account. An unavailable one leaves every room.
        let Some(to_text) = node.attr("to") else {
            match kind {
                Some("unavailable") => {
                    self.leave_all(id, muc::LeaveReason::Left);
                    if let Some(session) = self.sessions.get_mut(&id) {
                        session.available = false;
                    }
                }
                None => {
                    if let Some(session) = self.sessions.get_mut(&id) {
                        session.available = true;
                    }
                }
                Some(_) => {}
            }
            return;
        };
        let to = if to_text.encode_utf16().count() <= 2048 {
            parse_jid(to_text)
        } else {
            None
        };
        let muc_x = node.child("x");
        let to_room = to.as_ref().is_some_and(|to| {
            self.sessions
                .get(&id)
                .is_some_and(|s| muc::is_muc_domain(&s.domain, &to.domain))
                || starts_with_ignore_case(&to.domain, "muc.")
                || starts_with_ignore_case(&to.domain, "conference.")
                || muc_x.is_some_and(|x| x.attr("xmlns") == Some(crate::xmpp::NS_MUC))
        });
        let (Some(to), true) = (to, to_room) else {
            // Directed presence to a user, subscriptions and probes: dropped
            tracing::debug!("chat: presence to a user dropped c={id}");
            return;
        };
        match kind {
            None => self.join(id, &to).await,
            Some("unavailable") => self.leave(id, &to),
            Some(_) => {}
        }
    }

    async fn message(&mut self, id: u64, node: &Element) {
        let kind = node.attr("type");
        let to_text = node.attr("to").unwrap_or("");
        let to = if to_text.encode_utf16().count() <= 2048 {
            parse_jid(to_text)
        } else {
            None
        };
        let to_room = |to: &crate::xmpp::Jid| {
            self.sessions
                .get(&id)
                .is_some_and(|s| muc::is_muc_domain(&s.domain, &to.domain))
        };
        match (kind, to) {
            (Some("groupchat"), Some(to)) => self.group_chat(id, &to, node).await,
            // DR takes "chat"; Mystic Paradox's gateway also "normal" and none, as the 1.14.7 client may send
            (Some("chat" | "normal") | None, Some(to)) if !to_room(&to) => {
                self.direct(id, node, &to.local, to.resource.as_deref()).await;
            }
            _ => tracing::debug!(
                "chat: message dropped c={id} type={}",
                kind.unwrap_or("undefined").chars().take(16).collect::<String>()
            ),
        }
    }

    /// The account a JID local part names: a session here, the account it last logged in as here, or one
    /// federation knows.
    async fn account_named(&self, local: &str) -> Option<GlobalId> {
        let online = self
            .sessions_of(local)
            .into_iter()
            .find_map(|other| self.sessions.get(&other).and_then(|s| s.account.clone()));
        if online.is_some() {
            return online;
        }
        match self.backends.history.account_of(local).await {
            Ok(Some(text)) => return text.parse().ok(),
            Ok(None) => {}
            Err(error) => tracing::error!("chat: reading accounts failed ({error})"),
        }
        if let Some(account) = self.backends.host.account_of(local).await {
            return Some(account);
        }
        self.backends.federation.resolve(local).await
    }

    /// DR's whisper: to one session of the account when the resource names one, else to all of them, from
    /// the sender's full JID. Nothing goes back when it is not delivered. An account with no session here
    /// gets it at its next login.
    async fn direct(&mut self, id: u64, node: &Element, target: &str, resource: Option<&str>) {
        let now = self.now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let uid = session.uid.clone().unwrap_or_default();
        let body = text_of(node.child("body"));
        let length = body.chars().count();
        let message_id = node
            .attr("id")
            .filter(|i| i.encode_utf16().count() <= MAX_ID)
            .map(str::to_owned);
        let shown = if is_authcid_shape(target) { target } else { "?" };
        let refused = |reason: &str| tracing::info!("chat: whisper from={uid} to={shown} len={length} reason={reason}");
        // Every whisper takes a token first, refused ones included, so its log lines are as limited
        // as delivered ones
        if !muc::take_message_token(session, now) {
            if self.log_once(&format!("whisper-rate|{id}"), Some(60 * 1000)) {
                refused("limit");
            }
            return;
        }
        let muted = session.is_muted(now);
        let name = session.name_at_login.clone().unwrap_or_else(|| uid.clone());
        let domain = session.domain.clone();
        let Some(sender) = session.account.clone() else {
            return;
        };
        let sender_module = session.module.clone();
        if length == 0 || length > BODY_LIMIT {
            refused("size");
            return;
        }
        if let Some(command) = crate::moderation::parse_report(&body) {
            self.report_command(id, command, None, "chat", &domain);
            return;
        }
        if target == uid {
            refused("self");
            return;
        }
        if muted {
            refused("muted");
            return;
        }
        if !is_authcid_shape(target) {
            refused("unknown");
            return;
        }
        let Some(recipient) = self.account_named(target).await else {
            refused("unknown");
            return;
        };
        // Either player blocked the other: not delivered (the same rule as party invites)
        if self.backends.host.blocked(&sender, &recipient).await
            || !self.may_whisper(sender_module.as_ref(), &sender, &recipient).await
        {
            refused("blocked");
            return;
        }
        let mid = format!("dm-{}", random_hex(16));
        let rated = self.moderation.rate(&body, crate::moderation::Channel::Direct);
        self.moderation.record(crate::moderation::Line {
            id: mid.clone(),
            at: now,
            channel: crate::moderation::Channel::Direct.as_str(),
            room_id: None,
            sender: crate::moderation::Person {
                user_id: uid.clone(),
                name,
            },
            recipient: Some(crate::moderation::Person {
                user_id: target.to_owned(),
                name: target.to_owned(),
            }),
            body: body.clone(),
            verdict: rated.verdict,
        });
        if rated.verdict == crate::moderation::Verdict::Blocked {
            // A refused whisper sends nothing back either (DR), so blocked looks sent
            refused("filter");
            return;
        }
        let body = rated.shown;
        let sessions: Vec<u64> = self
            .sessions_of(target)
            .into_iter()
            .filter(|other| self.sessions.get(other).and_then(|s| s.account.as_ref()) == Some(&recipient))
            .collect();
        let chosen: Vec<u64> = sessions
            .iter()
            .copied()
            .filter(|other| self.sessions.get(other).and_then(|s| s.resource.as_deref()) == resource)
            .collect();
        let recipients = if chosen.is_empty() { sessions } else { chosen };
        let from = self.sessions.get(&id).map(Session::full_jid).unwrap_or_default();
        let text = escape(&body);
        let id_attr = message_id
            .as_deref()
            .map(|i| format!(" id=\"{}\"", escape(i)))
            .unwrap_or_default();
        let delivered = !recipients.is_empty();
        if delivered {
            self.remember_mid(&mid);
        }
        let local_delivered_at = if delivered
            || (self.backends.federation.is_federated() && recipient.owner() != &self.backends.node.node)
        {
            Some(now)
        } else {
            None
        };
        let stored = self
            .backends
            .history
            .add_direct_once(NewDirect {
                sender: sender.to_string(),
                sender_local: uid.clone(),
                recipient: recipient.to_string(),
                body: body.clone(),
                stanza_id: message_id.clone(),
                at: now,
                delivered_at: local_delivered_at,
                mid: Some(mid.clone()),
            })
            .await;
        if let Err(error) = stored {
            tracing::error!("chat: storing a whisper failed ({error})");
        }
        // Stored before it is sent: a message nobody can read back from history is never on a screen
        for recipient in &recipients {
            let to = self.sessions.get(recipient).map(Session::full_jid).unwrap_or_default();
            self.send(
                *recipient,
                &format!(
                    "<message xmlns=\"{NS_CLIENT}\" type=\"chat\" from=\"{}\" to=\"{}\"{id_attr}><body>{text}</body></message>",
                    escape(&from),
                    escape(&to)
                ),
            );
        }
        tracing::info!(
            "chat: whisper from={uid} to={target} len={length} delivered={}",
            recipients.len()
        );
        self.publish(Envelope::Direct {
            mid,
            from: sender,
            from_local: uid,
            to: recipient,
            body,
            id: message_id,
            at: now,
            delivered,
        })
        .await;
    }

    /// `/report <player> [category] <text>` in any chat: filed with the dashboard, never delivered. The answer
    /// goes to the sender alone, from `from` (the room, or the chat domain for a whisper).
    pub(crate) fn report_command(
        &mut self,
        id: u64,
        command: Result<(String, &'static str, String), ()>,
        room: Option<String>,
        kind: &str,
        from: &str,
    ) {
        let now = self.now();
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let uid = session.uid.clone().unwrap_or_default();
        let reporter = crate::moderation::Person {
            user_id: uid.clone(),
            name: session.name_at_login.clone().unwrap_or_else(|| uid.clone()),
        };
        // Server text, not launcher text: the game shows chat bodies as they come
        let answer = match command {
            Err(()) => "Usage: /report <player> [category] <what happened>",
            Ok(_) if !self.moderation.is_on() => "Reports are not set up on this server.",
            Ok(_) if !self.log_once(&format!("report|{uid}"), Some(30 * 1000)) => {
                "Please wait a little before sending another report."
            }
            Ok((target, category, description)) => {
                let target_uid = self
                    .sessions
                    .values()
                    .find(|s| {
                        s.uid.as_deref() == Some(target.as_str())
                            || s.name_at_login
                                .as_deref()
                                .is_some_and(|n| n.eq_ignore_ascii_case(&target))
                    })
                    .and_then(|s| s.uid.clone());
                let filed = self.moderation.report(crate::moderation::Report {
                    id: format!("report-{}", random_hex(16)),
                    source: "game",
                    reporter,
                    target: crate::moderation::TargetPerson {
                        user_id: target_uid,
                        name: target,
                    },
                    category,
                    description,
                    context: crate::moderation::ReportContext { room_id: room, at: now },
                });
                tracing::info!("chat: report from={uid} category={category} queued={filed}");
                if filed {
                    "Report sent. Thank you."
                } else {
                    "The report could not be sent. Please try again later."
                }
            }
        };
        let to = self.sessions.get(&id).map(Session::full_jid).unwrap_or_default();
        self.send(
            id,
            &format!(
                "<message xmlns=\"{NS_CLIENT}\" type=\"{kind}\" from=\"{}\" to=\"{}\"><body>{}</body></message>",
                escape(from),
                escape(&to),
                escape(answer)
            ),
        );
    }

    async fn remote_direct(&mut self, message: crate::federation::DirectWire) -> bool {
        let recipients: Vec<u64> = self
            .sessions
            .values()
            .filter(|s| !s.ended && s.resource.is_some() && s.account.as_ref() == Some(&message.to))
            .map(|s| s.id)
            .collect();
        if recipients.is_empty() {
            return false;
        }
        if !self.remember_mid(&message.mid) {
            return true;
        }
        if self.backends.host.blocked(&message.from, &message.to).await {
            return true;
        }
        let text = escape(&message.body);
        let id_attr = message
            .stanza_id
            .as_deref()
            .map(|i| format!(" id=\"{}\"", escape(i)))
            .unwrap_or_default();
        for sid in recipients {
            let (domain, to) = self
                .sessions
                .get(&sid)
                .map(|s| (s.domain.clone(), s.full_jid()))
                .unwrap_or_default();
            let from = format!("{}@{domain}", message.from_local);
            self.send(
                sid,
                &format!(
                    "<message xmlns=\"{NS_CLIENT}\" type=\"chat\" from=\"{}\" to=\"{}\"{id_attr}><body>{text}</body></message>",
                    escape(&from),
                    escape(&to)
                ),
            );
        }
        true
    }

    /// A mute set or lifted: the account's live sessions follow it at once.
    fn muted(&mut self, account: &GlobalId, mute: Option<Mute>) {
        for session in self.sessions.values_mut() {
            if session.account.as_ref() == Some(account) {
                session.muted.clone_from(&mute);
            }
        }
    }

    // ---- The task ----

    /// Shutdown: `<close/>` to every session; whatever is still open is cut after 1 s.
    async fn shutdown(&mut self) {
        type Open = (u64, Arc<AtomicBool>, Arc<tokio::sync::Notify>);
        let open: Vec<Open> = self
            .sessions
            .values()
            .map(|s| (s.id, s.out.closed.clone(), s.out.terminate.clone()))
            .collect();
        for (id, _, _) in &open {
            self.end(*id, EndReason::Shutdown).await;
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        while tokio::time::Instant::now() < deadline
            && !open.iter().all(|(_, closed, _)| closed.load(Ordering::Relaxed))
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        for (_, _, terminate) in &open {
            terminate.notify_one();
        }
    }
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix))
}

/// The authcid as a JID local part (DR checked its own account id shape here; each game has its own): 1 to
/// 128 bytes, no `@`, `/`, white space or control character.
pub fn is_authcid_shape(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_AUTHCID
        && !text
            .chars()
            .any(|c| c == '@' || c == '/' || c.is_whitespace() || c.is_control())
}

/// `Buffer.from(text, "base64")` after the shape check: 6 bits per character up to the first `=`,
/// whole bytes kept, a trailing partial one dropped.
fn decode_base64_like_node(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0u32);
    for b in text.bytes() {
        let value = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => break,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

/// The chat task: answers events one at a time and ticks every second. Returns after a shutdown.
pub async fn run(mut chat: Chat, mut events: mpsc::UnboundedReceiver<Event>) {
    let auto_tick = chat.auto_tick;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            event = events.recv() => match event {
                None => return,
                Some(Event::Admit { address, via, reply }) => {
                    let _ = reply.send(chat.admit(&address, via));
                }
                Some(Event::Connected { address, via, out, reply }) => {
                    let id = chat.connected(address, via, out);
                    let _ = reply.send(id);
                }
                Some(Event::Frame { id, text }) => chat.frame(id, &text).await,
                Some(Event::Gone { id, size }) => {
                    let now = chat.now();
                    if let Some(session) = chat.sessions.get_mut(&id) {
                        if size {
                            // ws answers an oversized frame with a close 1009 of its own
                            let address = session.address.clone();
                            tracing::warn!("chat: socket error c={id} (RangeError)");
                            chat.count_failure(&address, now);
                            chat.end(id, EndReason::Size).await;
                        } else {
                            session.open = false;
                            chat.end(id, EndReason::Socket).await;
                        }
                    }
                }
                Some(Event::Muted { account, mute }) => chat.muted(&account, mute),
                Some(Event::RemoteDirect { message, reply }) => {
                    let delivered = chat.remote_direct(message).await;
                    let _ = reply.send(delivered);
                }
                Some(Event::RemoteRoom { message }) => {
                    chat.remote_room(message).await;
                }
                Some(Event::Shutdown(done)) => {
                    chat.shutdown().await;
                    let _ = done.send(());
                    return;
                }
                Some(Event::Tick(done)) => {
                    chat.tick().await;
                    let _ = done.send(());
                }
            },
            _ = ticker.tick(), if auto_tick => chat.tick().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sasl_payloads_decode_like_buffer_from() {
        assert_eq!(decode_base64_like_node("AGEAYg=="), b"\0a\0b");
        assert_eq!(decode_base64_like_node("AGEAYg"), b"\0a\0b");
        assert_eq!(decode_base64_like_node("QQ"), b"A");
        assert_eq!(decode_base64_like_node("Q"), b"");
    }

    #[test]
    fn log_once_keeps_a_window() {
        let mut once = LogOnce::default();
        assert!(once.check("a", 1000, 0));
        assert!(!once.check("a", 1000, 999));
        assert!(once.check("a", 1000, 1000));
    }

    #[test]
    fn authcids_are_jid_local_parts() {
        for good in ["UID-1234", "0f8fad5b-d9cb-469f-a165-70867728950e", "player"] {
            assert!(is_authcid_shape(good), "{good}");
        }
        let long = "a".repeat(MAX_AUTHCID + 1);
        for bad in ["", "a@b", "a/b", "a b", "a\u{0}", long.as_str()] {
            assert!(!is_authcid_shape(bad), "{bad:?}");
        }
    }

    #[test]
    fn stamps_are_utc_with_milliseconds() {
        assert_eq!(stamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(stamp(1_793_000_000_123), "2026-10-26T07:33:20.123Z");
    }
}
