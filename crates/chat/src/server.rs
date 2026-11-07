// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The chat task: one task owns every session, room and limit and handles one event at a time, as the Node chat's
//! single thread did. Login, bind, IQs, presence, whispers and offline delivery live here; rooms in [`crate::muc`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::chat::{ChatAnswer, ChatQuery, Jid as ModuleJid};
use module_api::{GlobalId, ModuleId, NodeIdentity};
use rand::Rng as _;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::federation::{DirectWire, Envelope, Federation, RoomWire};
use crate::history::{History, Mute, NewDirect};
use crate::hooks::{ChatHost, Login, LoginRefusal, default_jid, is_unsupported};
use crate::moderation::{Channel, Line, Moderation, Person, Verdict};
use crate::muc::Room;
use crate::settings::{NickCheck, Settings};
use crate::xmpp::{
    self, Bucket, DEFAULT_DOMAIN, Element, NS_BIND, NS_FRAMING, NS_PING, NS_SASL, NS_SESSION, NS_STANZAS,
    NS_STREAM_ERRORS, NS_STREAMS, escape, has_markup_declaration, local_name, log_text, parse_frame, parse_jid,
    redact_frame,
};

/// Largest WebSocket frame and message, bytes
pub const FRAME_LIMIT: usize = 32768;
/// Largest message body, chars
pub const BODY_LIMIT: usize = 2048;
/// Default unsent-output limit, bytes
pub const OUTPUT_LIMIT: usize = 256 * 1024;

/// The time source: unix milliseconds. Tests swap in a fixed clock.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// Sees every stanza handed to a connection, with the receiving session's JID local part
pub type Observer = Arc<dyn Fn(Option<&str>, &str) + Send + Sync>;

const MINUTE: i64 = 60_000;
const TEN_MINUTES: i64 = 10 * MINUTE;
const MAX_SESSIONS: usize = 512;
const MAX_UNBOUND_PER_ADDRESS: usize = 8;
const MAX_FRAMES_BEFORE_LOGIN: u32 = 4;
const LOGIN_DEADLINE: i64 = 15_000;
const BIND_DEADLINE: i64 = 10_000;
const BIND_TIMEOUTS: usize = 3;
const SASL_FAILURES: usize = 10;
const UID_WAIT: i64 = MINUTE;
const STANZA_BURST: u32 = 60;
const STANZA_REFILL_MS: f64 = 1000.0 / 30.0;
const MAX_DROPS: usize = 100;
const IDLE_BEFORE_PING: i64 = 50_000;
const PING_TIMEOUT: i64 = 100_000;
const GHOST_PING: i64 = 10_000;
const MAX_CONNECTIONS_PER_UID: usize = 3;
const MAX_BOUND_PER_UID: usize = 2;
/// IQ ids and message ids, in UTF-16 units
pub(crate) const MAX_ID: usize = 128;
const MAX_RESOURCE: usize = 256;
pub(crate) const MAX_TO: usize = 2048;
const SWEEP_EVERY: i64 = MINUTE;
const SEEN_MIDS: usize = 4096;
const LOG_ONCE_KEYS: usize = 10_000;
const MAX_PAYLOAD: usize = 12_000;
const MAX_TOKEN_UNITS: usize = 8192;
const MAX_LOGGED_NAMESPACES: usize = 32;
const REPORT_WINDOW: i64 = 30_000;

/// How a chat runs, beyond its [`crate::Settings`]: the hooks that tests and the host replace.
#[derive(Clone)]
pub struct Options {
    /// Where every time comes from.
    pub clock: Clock,
    /// Unsent output, in bytes, past which a connection stops getting stanzas.
    pub output_limit: usize,
    /// Whether the task ticks itself; tests turn it off and call `tick`.
    pub auto_tick: bool,
    /// Sees every stanza handed to a connection.
    pub observe: Option<Observer>,
    /// The dashboard's filter and chat log.
    pub moderation: Moderation,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            clock: Arc::new(now_ms),
            output_limit: OUTPUT_LIMIT,
            auto_tick: true,
            observe: None,
            moderation: Moderation::off(),
        }
    }
}

/// What the task hands a connection's writer.
#[derive(Debug)]
pub enum Out {
    /// One text frame.
    Text(String),
    /// A close frame with this code and an empty reason; nothing is written after it
    Close(u16),
    /// Drop the socket without a close frame.
    Cut,
}

/// The task's handle on one connection's writer.
#[derive(Debug, Clone)]
pub struct Writer {
    tx: mpsc::UnboundedSender<Out>,
    unsent: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

/// The connection's side: what the task hands it, and the counts the task reads.
#[derive(Debug)]
pub struct WriterEnd {
    /// What the task hands the connection, in order.
    pub rx: mpsc::UnboundedReceiver<Out>,
    unsent: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

impl Writer {
    /// A writer and the connection's end of it.
    pub fn new() -> (Self, WriterEnd) {
        let (tx, rx) = mpsc::unbounded_channel();
        let unsent = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let end = WriterEnd {
            rx,
            unsent: unsent.clone(),
            closed: closed.clone(),
        };
        (Self { tx, unsent, closed }, end)
    }

    /// Bytes handed over and not yet written (Node's `bufferedAmount`).
    pub fn unsent(&self) -> usize {
        self.unsent.load(Ordering::Relaxed)
    }

    /// Whether the connection's socket is gone.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn send(&self, out: Out) -> bool {
        self.tx.send(out).is_ok()
    }
}

impl WriterEnd {
    /// Takes `bytes` just written off the unsent total.
    pub fn written(&self, bytes: usize) {
        _ = self
            .unsent
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| Some(n.saturating_sub(bytes)));
    }

    /// The socket is dropped: what shutdown waits for.
    pub fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

/// Whatever the chat task is asked.
#[derive(Debug)]
pub enum Event {
    /// Whether a new connection from `address` may open. Answered on `reply`.
    Admit {
        /// The player's address.
        address: IpAddr,
        /// Gets the answer.
        reply: oneshot::Sender<bool>,
    },
    /// A WebSocket opened. Answered on `reply` with the session id.
    Open {
        /// The player's address.
        address: IpAddr,
        /// `gateway` when the gateway named the address, `direct` otherwise.
        via: &'static str,
        /// Where the task writes to the connection.
        writer: Writer,
        /// Gets the session id.
        reply: oneshot::Sender<u64>,
    },
    /// A text frame from a session.
    Frame {
        /// The session id.
        session: u64,
        /// The frame as received.
        text: String,
    },
    /// A session's socket closed.
    Gone {
        /// The session id.
        session: u64,
        /// Whether it closed on a frame over [`FRAME_LIMIT`], which counts as a failed login.
        too_large: bool,
    },
    /// A mute was set or lifted: live sessions of `account` follow it.
    Muted {
        /// The account.
        account: GlobalId,
        /// The mute, or none when lifted.
        mute: Option<Mute>,
    },
    /// A direct message from another node for a session here. Answered with whether one took it.
    RemoteDirect {
        /// The message.
        message: DirectWire,
        /// Gets whether a session took it.
        reply: oneshot::Sender<bool>,
    },
    /// A room message from another node for the occupants here.
    RemoteRoom {
        /// The message.
        message: RoomWire,
    },
    /// Run the periodic work now: pings, sweeps, federation's outbox.
    Tick {
        /// Signalled when the tick is done.
        done: oneshot::Sender<()>,
    },
    /// End every session and stop the task.
    Shutdown,
}

/// Milliseconds since the epoch: 0 before it, saturating.
pub fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

/// `n` random bytes as lowercase hex.
pub fn random_hex(n: usize) -> String {
    let mut bytes = vec![0_u8; n];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// XEP-0203's stamp, as JavaScript's `toISOString`.
pub fn stamp(at: i64) -> String {
    // JavaScript's Date range
    const MAX_DATE: i64 = 8_640_000_000_000_000;
    let time = if (-MAX_DATE..=MAX_DATE).contains(&at) {
        chrono::DateTime::from_timestamp_millis(at)
    } else {
        None
    };
    time.unwrap_or_default().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// A SASL authcid that can be a JID local part.
pub fn is_authcid_shape(text: &str) -> bool {
    (1..=128).contains(&text.len())
        && !text
            .chars()
            .any(|c| c == '@' || c == '/' || c.is_whitespace() || c.is_control())
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `Buffer.from(text, "base64")`: `-` and `_` count as `+` and `/`, the first other char ends it, and a trailing
/// partial byte is dropped.
pub(crate) fn node_base64(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for b in text.bytes() {
        let value = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => break,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

/// The three fields of a SASL PLAIN payload, when it is well formed.
pub(crate) fn decode_plain(payload: &str) -> Option<(String, String)> {
    let payload = service_kit::js::trim(payload);
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return None;
    }
    let data = payload.trim_end_matches('=');
    if payload.len() - data.len() > 2
        || data.is_empty()
        || !data
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return None;
    }
    let decoded = String::from_utf8_lossy(&node_base64(payload)).into_owned();
    let fields: Vec<&str> = decoded.split('\0').collect();
    let [authzid, authcid, token] = fields.as_slice() else {
        return None;
    };
    let ok = (authzid.is_empty() || authzid == authcid)
        && is_authcid_shape(authcid)
        && !token.is_empty()
        && utf16_len(token) <= MAX_TOKEN_UNITS;
    ok.then(|| ((*authcid).to_owned(), (*token).to_owned()))
}

/// "Log now?" per key and window.
#[derive(Default)]
pub(crate) struct LogOnce {
    seen: HashMap<String, (i64, u64)>,
    order: BTreeMap<(i64, u64), String>,
    next: u64,
}

impl LogOnce {
    pub(crate) fn check(&mut self, key: &str, window: i64, now: i64) -> bool {
        if let Some(&(at, _)) = self.seen.get(key)
            && now - at < window
        {
            return false;
        }
        if let Some(old) = self.seen.remove(key) {
            self.order.remove(&old);
        }
        self.next += 1;
        self.seen.insert(key.to_owned(), (now, self.next));
        self.order.insert((now, self.next), key.to_owned());
        while self.seen.len() > LOG_ONCE_KEYS {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.seen.remove(&oldest);
        }
        true
    }

    fn forget(&mut self, now: i64) {
        while let Some((&(at, seq), _)) = self.order.first_key_value() {
            if now - at < TEN_MINUTES {
                break;
            }
            if let Some(key) = self.order.remove(&(at, seq)) {
                self.seen.remove(&key);
            }
        }
    }
}

/// The last message ids handled.
#[derive(Default)]
struct Mids {
    order: VecDeque<String>,
    seen: HashSet<String>,
}

impl Mids {
    /// Whether it was new.
    fn remember(&mut self, mid: &str) -> bool {
        if mid.is_empty() {
            return true;
        }
        if !self.seen.insert(mid.to_owned()) {
            return false;
        }
        self.order.push_back(mid.to_owned());
        if self.order.len() > SEEN_MIDS
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndReason {
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

pub(crate) struct Session {
    pub(crate) id: u64,
    writer: Writer,
    /// Still open for sending
    open: bool,
    address: IpAddr,
    opened: i64,
    pub(crate) domain: String,
    pub(crate) uid: Option<String>,
    pub(crate) account: Option<GlobalId>,
    pub(crate) module: Option<ModuleId>,
    pub(crate) name: Option<String>,
    login_at: i64,
    pub(crate) resource: Option<String>,
    pub(crate) ended: bool,
    refused_login: bool,
    backlogged: bool,
    frames_before_login: u32,
    last_inbound: i64,
    ping: Option<(String, i64)>,
    stanzas: Bucket,
    pub(crate) joins: Bucket,
    pub(crate) messages: Bucket,
    drops: VecDeque<i64>,
    last_drop_log: Option<i64>,
    pub(crate) last_limit_log: Option<i64>,
    logged_namespaces: Vec<String>,
    /// Kept as the Node chat kept it; nothing is echoed
    #[allow(dead_code, reason = "kept as the Node chat kept it; nothing reads it")]
    available: bool,
    /// `(room JID as the client spelled it, nickname)`, in join order
    pub(crate) rooms: Vec<(String, String)>,
    pub(crate) mute: Option<Mute>,
}

impl Session {
    pub(crate) fn full_jid(&self) -> String {
        format!(
            "{}@{}/{}",
            self.uid.as_deref().unwrap_or("undefined"),
            self.domain,
            self.resource.as_deref().unwrap_or("undefined")
        )
    }

    pub(crate) fn muted(&self, now: i64) -> bool {
        self.mute.as_ref().is_some_and(|m| m.holds(now))
    }

    fn uid_or_dash(&self) -> &str {
        self.uid.as_deref().unwrap_or("-")
    }
}

/// Everything the task owns.
pub(crate) struct Chat {
    pub(crate) settings: Settings,
    pub(crate) node: NodeIdentity,
    pub(crate) host: Arc<dyn ChatHost>,
    pub(crate) federation: Arc<dyn Federation>,
    pub(crate) history: History,
    pub(crate) options: Options,
    pub(crate) modules: Arc<Mutex<BTreeSet<ModuleId>>>,
    largest_backlog: Arc<AtomicUsize>,
    pub(crate) sessions: BTreeMap<u64, Session>,
    /// Bound sessions per uid, oldest first
    pub(crate) bound: HashMap<String, Vec<u64>>,
    next_session: u64,
    next_ping: u64,
    mids: Mids,
    pub(crate) log_once: LogOnce,
    sasl_failures: HashMap<IpAddr, Vec<i64>>,
    address_wait: HashMap<IpAddr, i64>,
    uid_wait: HashMap<String, i64>,
    bind_timeouts: HashMap<String, Vec<i64>>,
    pub(crate) rooms: Vec<Room>,
    last_sweep: Option<i64>,
}

pub(crate) struct Parts {
    pub(crate) settings: Settings,
    pub(crate) node: NodeIdentity,
    pub(crate) host: Arc<dyn ChatHost>,
    pub(crate) federation: Arc<dyn Federation>,
    pub(crate) history: History,
    pub(crate) options: Options,
    pub(crate) modules: Arc<Mutex<BTreeSet<ModuleId>>>,
    pub(crate) largest_backlog: Arc<AtomicUsize>,
}

fn failure(condition: &str) -> String {
    format!(r#"<failure xmlns="{NS_SASL}"><{condition}/></failure>"#)
}

fn id_attr(id: Option<&str>) -> String {
    id.map(|id| format!(r#" id="{}""#, escape(id))).unwrap_or_default()
}

impl Chat {
    pub(crate) fn new(parts: Parts) -> Self {
        Self {
            settings: parts.settings,
            node: parts.node,
            host: parts.host,
            federation: parts.federation,
            history: parts.history,
            options: parts.options,
            modules: parts.modules,
            largest_backlog: parts.largest_backlog,
            sessions: BTreeMap::new(),
            bound: HashMap::new(),
            next_session: 1,
            next_ping: 1,
            mids: Mids::default(),
            log_once: LogOnce::default(),
            sasl_failures: HashMap::new(),
            address_wait: HashMap::new(),
            uid_wait: HashMap::new(),
            bind_timeouts: HashMap::new(),
            rooms: Vec::new(),
            last_sweep: None,
        }
    }

    pub(crate) fn now(&self) -> i64 {
        (self.options.clock)()
    }

    /// The task: events in order, a tick a second when asked for, until shutdown or every sender is gone.
    pub(crate) async fn run(mut self, mut events: mpsc::UnboundedReceiver<Event>) {
        let second = Duration::from_secs(1);
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + second, second);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let auto_tick = self.options.auto_tick;
        loop {
            tokio::select! {
                biased;
                event = events.recv() => match event {
                    None => break,
                    Some(Event::Shutdown) => {
                        self.shutdown().await;
                        break;
                    }
                    Some(event) => self.handle(event).await,
                },
                _ = ticker.tick(), if auto_tick => self.tick().await,
            }
        }
    }

    async fn handle(&mut self, event: Event) {
        match event {
            Event::Admit { address, reply } => {
                let admitted = self.admit(address);
                _ = reply.send(admitted);
            }
            Event::Open {
                address,
                via,
                writer,
                reply,
            } => {
                let id = self.open(address, via, writer);
                if reply.send(id).is_err() {
                    self.gone(id, false).await;
                }
            }
            Event::Frame { session, text } => self.frame(session, &text).await,
            Event::Gone { session, too_large } => self.gone(session, too_large).await,
            Event::Muted { account, mute } => {
                for session in self.sessions.values_mut() {
                    if session.account.as_ref() == Some(&account) {
                        session.mute = mute.clone();
                    }
                }
            }
            Event::RemoteDirect { message, reply } => {
                let delivered = self.remote_direct(message).await;
                _ = reply.send(delivered);
            }
            Event::RemoteRoom { message } => self.remote_room(message).await,
            Event::Tick { done } => {
                self.tick().await;
                _ = done.send(());
            }
            Event::Shutdown => self.shutdown().await,
        }
    }

    pub(crate) fn answers_chat(&self, module: Option<&ModuleId>) -> bool {
        module.is_some_and(|m| self.modules.lock().unwrap_or_else(PoisonError::into_inner).contains(m))
    }

    fn admit(&mut self, address: IpAddr) -> bool {
        let now = self.now();
        let cause = if self.sessions.len() >= MAX_SESSIONS {
            Some("too many connections")
        } else if self
            .sessions
            .values()
            .filter(|s| s.address == address && !s.ended && s.resource.is_none())
            .count()
            >= MAX_UNBOUND_PER_ADDRESS
        {
            Some("too many logins in progress from this address")
        } else {
            None
        };
        let Some(cause) = cause else { return true };
        if self.log_once.check(&format!("admit|{address}"), TEN_MINUTES, now) {
            warn!("chat: connection refused from={address} reason={cause}");
        }
        false
    }

    fn open(&mut self, address: IpAddr, via: &'static str, writer: Writer) -> u64 {
        let now = self.now();
        let id = self.next_session;
        self.next_session += 1;
        self.sessions.insert(
            id,
            Session {
                id,
                writer,
                open: true,
                address,
                opened: now,
                domain: DEFAULT_DOMAIN.to_owned(),
                uid: None,
                account: None,
                module: None,
                name: None,
                login_at: 0,
                resource: None,
                ended: false,
                refused_login: false,
                backlogged: false,
                frames_before_login: 0,
                last_inbound: now,
                ping: None,
                stanzas: Bucket::new(STANZA_BURST, now),
                joins: Bucket::new(crate::muc::JOIN_BURST, now),
                messages: Bucket::new(crate::muc::MESSAGE_BURST, now),
                drops: VecDeque::new(),
                last_drop_log: None,
                last_limit_log: None,
                logged_namespaces: Vec::new(),
                available: false,
                rooms: Vec::new(),
                mute: None,
            },
        );
        info!("chat: connect c={id} from={address} via={via}");
        id
    }

    /// Hands a stanza to a session's writer, unless it is gone or backed up.
    pub(crate) fn send(&mut self, id: u64, stanza: &str) {
        let limit = self.options.output_limit;
        let trace = self.settings.trace;
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if !session.open {
            return;
        }
        if !session.ended && (session.backlogged || session.writer.unsent() > limit) {
            session.backlogged = true;
            return;
        }
        if trace {
            info!("chat: out c={id} {}", redact_frame(stanza));
        }
        if let Some(observe) = &self.options.observe {
            observe(session.uid.as_deref(), stanza);
        }
        let unsent = session.writer.unsent.fetch_add(stanza.len(), Ordering::Relaxed) + stanza.len();
        self.largest_backlog.fetch_max(unsent, Ordering::Relaxed);
        if !session.writer.send(Out::Text(stanza.to_owned())) {
            session.open = false;
        }
    }

    /// Ends a session once, telling its client as the reason says.
    pub(crate) async fn end(&mut self, id: u64, reason: EndReason) {
        let now = self.now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.ended {
            return;
        }
        session.ended = true;
        let leave = if reason == EndReason::Replaced {
            crate::muc::Leave::Replaced
        } else {
            crate::muc::Leave::Disconnect
        };
        self.leave_all(id, leave);
        let Some(session) = self.sessions.get(&id) else { return };
        let (uid, account, bound) = (session.uid.clone(), session.account.clone(), session.resource.is_some());
        if bound && let Some(uid) = &uid {
            let mut last = false;
            if let Some(list) = self.bound.get_mut(uid) {
                list.retain(|&s| s != id);
                last = list.is_empty();
            }
            if last {
                self.bound.remove(uid);
                if let Some(account) = &account {
                    self.federation.presence(account, uid, false).await;
                    self.host.presence(account, uid, false).await;
                }
            }
            if matches!(
                reason,
                EndReason::Replaced | EndReason::Abuse | EndReason::Size | EndReason::Backlog
            ) {
                self.uid_wait.insert(uid.clone(), now + UID_WAIT);
            }
        }
        let Some(session) = self.sessions.get(&id) else { return };
        #[allow(clippy::cast_precision_loss, reason = "a session age in ms, far below 2^53")]
        let age = ((now - session.opened) as f64 / 1000.0).round();
        info!(
            "chat: closed c={id} uid={} reason={} after={age}s",
            session.uid_or_dash(),
            reason.as_str()
        );
        let (open, writer) = (session.open, session.writer.clone());
        if open {
            match reason {
                EndReason::Close | EndReason::Shutdown | EndReason::Replaced => {
                    self.send(id, &format!(r#"<close xmlns="{NS_FRAMING}"/>"#));
                    writer.send(Out::Close(1000));
                }
                EndReason::Size => _ = writer.send(Out::Close(1009)),
                _ => _ = writer.send(Out::Cut),
            }
        }
        self.sessions.remove(&id);
    }

    async fn gone(&mut self, id: u64, too_large: bool) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if too_large {
            warn!("chat: frame too large c={id} uid={}", session.uid_or_dash());
            let address = session.address;
            let now = self.now();
            self.sasl_failure(address, now);
            self.end(id, EndReason::Size).await;
        } else {
            session.open = false;
            self.end(id, EndReason::Socket).await;
        }
    }

    fn sasl_failure(&mut self, address: IpAddr, now: i64) {
        let times = self.sasl_failures.entry(address).or_default();
        times.retain(|&at| now - at < TEN_MINUTES);
        times.push(now);
        if times.len() >= SASL_FAILURES {
            self.address_wait.insert(address, now + TEN_MINUTES);
        }
    }

    pub(crate) async fn drop_stanza(&mut self, id: u64, reason: &str) {
        let now = self.now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.drops.push_back(now);
        while session.drops.front().is_some_and(|&at| now - at >= MINUTE) {
            session.drops.pop_front();
        }
        if session.last_drop_log.is_none_or(|at| now - at >= MINUTE) {
            session.last_drop_log = Some(now);
            info!("chat: dropped c={id} uid={} reason={reason}", session.uid_or_dash());
        }
        if session.drops.len() > MAX_DROPS {
            self.end(id, EndReason::Abuse).await;
        }
    }

    async fn frame(&mut self, id: u64, text: &str) {
        let now = self.now();
        let limit = self.options.output_limit;
        let trace = self.settings.trace;
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.ended {
            return;
        }
        session.last_inbound = now;
        session.ping = None;
        if trace {
            info!("chat: in c={id} {}", redact_frame(text));
        }
        if session.backlogged || session.writer.unsent() > limit {
            self.end(id, EndReason::Backlog).await;
            return;
        }
        let logged_in = session.uid.is_some();
        if !logged_in {
            session.frames_before_login += 1;
            if session.frames_before_login > MAX_FRAMES_BEFORE_LOGIN {
                let address = session.address;
                self.sasl_failure(address, now);
                self.end(id, EndReason::Refused).await;
                return;
            }
        } else if !session.stanzas.take(STANZA_BURST, STANZA_REFILL_MS, now) {
            self.drop_stanza(id, "rate").await;
            return;
        }
        if has_markup_declaration(text) {
            if logged_in {
                self.drop_stanza(id, "parse").await;
            } else {
                self.send(
                    id,
                    &format!(
                        r#"<stream:error xmlns:stream="{NS_STREAMS}"><restricted-xml xmlns="{NS_STREAM_ERRORS}"/></stream:error>"#
                    ),
                );
                self.end(id, EndReason::Refused).await;
            }
            return;
        }
        let Some(root) = parse_frame(text) else {
            if logged_in {
                self.drop_stanza(id, "parse").await;
            } else {
                self.end(id, EndReason::Refused).await;
            }
            return;
        };
        let name = local_name(&root.name);
        match name.as_str() {
            "open" => return self.stream_open(id, &root),
            "close" => return self.end(id, EndReason::Close).await,
            _ => {}
        }
        if !logged_in {
            return self.before_login(id, &name, &root).await;
        }
        if name == "iq" {
            return self.iq(id, &root).await;
        }
        let bound = self.sessions.get(&id).is_some_and(|s| s.resource.is_some());
        if !bound {
            return self.drop_stanza(id, "unknown").await;
        }
        match name.as_str() {
            "presence" => self.presence(id, &root).await,
            "message" => self.message(id, &root).await,
            _ => self.drop_stanza(id, "unknown").await,
        }
    }

    fn stream_open(&mut self, id: u64, root: &Element) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.resource.is_none() {
            match root.attr("to").filter(|to| xmpp::is_xmpp_domain(to)) {
                Some(to) => session.domain = to.to_owned(),
                None if session.uid.is_none() => session.domain = DEFAULT_DOMAIN.to_owned(),
                None => {}
            }
        }
        let open = format!(
            r#"<open xmlns="{NS_FRAMING}" from="{}" id="{}" version="1.0" xml:lang="en"/>"#,
            escape(&session.domain),
            random_hex(8)
        );
        let features = if session.uid.is_some() {
            format!(r#"<stream:features xmlns:stream="{NS_STREAMS}"><bind xmlns="{NS_BIND}"/></stream:features>"#)
        } else {
            format!(
                r#"<stream:features xmlns:stream="{NS_STREAMS}"><mechanisms xmlns="{NS_SASL}"><mechanism>PLAIN</mechanism></mechanisms></stream:features>"#
            )
        };
        self.send(id, &open);
        self.send(id, &features);
    }

    async fn before_login(&mut self, id: u64, name: &str, root: &Element) {
        match name {
            "auth" => self.sasl(id, root).await,
            "iq" => {
                let iq_id = root.attr("id").filter(|i| !i.is_empty() && utf16_len(i) <= MAX_ID);
                self.send(
                    id,
                    &format!(
                        r#"<iq xmlns="jabber:client" type="error"{}><error type="auth"><not-authorized xmlns="{NS_STANZAS}"/></error></iq>"#,
                        id_attr(iq_id)
                    ),
                );
                self.end(id, EndReason::Refused).await;
            }
            _ => self.end(id, EndReason::Refused).await,
        }
    }

    fn refuse_login(&mut self, id: u64, condition: &str, reason: &str, uid: Option<&str>) {
        let now = self.now();
        self.send(id, &failure(condition));
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.refused_login = true;
        let address = session.address;
        if reason != "throttled" && reason != "unavailable" {
            self.sasl_failure(address, now);
        }
        let who = uid.map_or_else(|| address.to_string(), str::to_owned);
        if self.log_once.check(&format!("login|{who}|{reason}"), TEN_MINUTES, now) {
            info!(
                "chat: login refused c={id} uid={} from={address} reason={reason}",
                uid.map_or_else(|| String::from("-"), |u| log_text(u, 128))
            );
        }
    }

    fn throttled_address(&self, address: IpAddr, now: i64) -> bool {
        self.address_wait.get(&address).is_some_and(|&until| now < until)
    }

    fn throttled_uid(&self, uid: &str, now: i64) -> bool {
        self.uid_wait.get(uid).is_some_and(|&until| now < until)
    }

    async fn sasl(&mut self, id: u64, auth: &Element) {
        let now = self.now();
        let Some(session) = self.sessions.get(&id) else { return };
        if session.refused_login {
            return self.end(id, EndReason::Refused).await;
        }
        let (address, domain) = (session.address, session.domain.clone());
        if self.throttled_address(address, now) {
            return self.refuse_login(id, "temporary-auth-failure", "throttled", None);
        }
        if auth.attr("mechanism") != Some("PLAIN") {
            return self.refuse_login(id, "invalid-mechanism", "bad-mechanism", None);
        }
        let Some((authcid, token)) = decode_plain(&auth.text()) else {
            return self.refuse_login(id, "not-authorized", "bad-format", None);
        };
        if self.throttled_uid(&authcid, now) {
            return self.refuse_login(id, "temporary-auth-failure", "throttled", Some(&authcid));
        }
        let login = Login {
            authcid: &authcid,
            token: &token,
            domain: &domain,
        };
        let host = self.host.clone();
        let player = match host.login(login).await {
            Ok(player) => player,
            Err(refusal) => {
                let condition = if refusal == LoginRefusal::Unavailable {
                    "temporary-auth-failure"
                } else {
                    "not-authorized"
                };
                return self.refuse_login(id, condition, refusal.as_str(), Some(&authcid));
            }
        };
        let Some(jid) = self.jid_of(&player.module, &player.account, &authcid, &domain).await else {
            return self.refuse_login(id, "temporary-auth-failure", "unavailable", Some(&authcid));
        };
        if jid.local() != authcid {
            return self.refuse_login(id, "not-authorized", "uid-mismatch", Some(&authcid));
        }
        let account_text = player.account.to_string();
        let mute = match self.history.mute_of(&account_text).await {
            Ok(Some(mute)) => Some(mute),
            Ok(None) => self.federation.mute_of(&player.account).await,
            Err(e) => {
                error!("chat: mute lookup failed c={id} uid={authcid}: {e}");
                return self.refuse_login(id, "temporary-auth-failure", "unavailable", Some(&authcid));
            }
        };
        if let Err(e) = self
            .history
            .remember_account(&authcid, &account_text, player.module.as_str(), jid.domain(), now)
            .await
        {
            warn!("chat: could not remember the account c={id} uid={authcid}: {e}");
        }
        let others: Vec<(u64, bool)> = self
            .sessions
            .values()
            .filter(|s| s.id != id && !s.ended && s.uid.as_deref() == Some(authcid.as_str()))
            .map(|s| (s.id, s.resource.is_some()))
            .collect();
        let excess = (others.len() + 1).saturating_sub(MAX_CONNECTIONS_PER_UID);
        let unbound: Vec<u64> = others.iter().filter(|(_, bound)| !bound).map(|(s, _)| *s).collect();
        for other in unbound.into_iter().take(excess) {
            self.end(other, EndReason::Replaced).await;
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.uid = Some(authcid.clone());
        session.account = Some(player.account);
        session.module = Some(player.module);
        session.name = Some(player.name);
        session.login_at = now;
        session.mute = mute;
        info!("chat: login ok c={id} uid={authcid}");
        self.send(id, &format!(r#"<success xmlns="{NS_SASL}"/>"#));
    }

    async fn jid_of(&self, module: &ModuleId, account: &GlobalId, authcid: &str, domain: &str) -> Option<ModuleJid> {
        if !self.answers_chat(Some(module)) {
            return default_jid(authcid, domain);
        }
        let query = ChatQuery::Jid {
            account: account.clone(),
        };
        match self.host.ask(module, query).await {
            Ok(ChatAnswer::Jid(jid)) => Some(jid),
            Ok(_) => None,
            Err(e) if is_unsupported(&e) => default_jid(authcid, domain),
            Err(e) => {
                warn!("chat: {module} gave no JID: {e}");
                None
            }
        }
    }

    async fn iq(&mut self, id: u64, root: &Element) {
        let kind = root.attr("type").unwrap_or_default();
        if kind == "result" || kind == "error" {
            return;
        }
        let iq_id = root.attr("id").unwrap_or_default().to_owned();
        if !(kind == "get" || kind == "set") || utf16_len(&iq_id) > MAX_ID {
            return self.drop_stanza(id, "unknown").await;
        }
        if kind == "set"
            && let Some(bind) = root.child("bind")
        {
            let resource = xmpp::text_of(bind.child("resource"));
            return self.bind(id, &iq_id, resource).await;
        }
        let namespace = root
            .elements()
            .next()
            .and_then(|e| e.attr("xmlns"))
            .unwrap_or_default()
            .to_owned();
        if namespace != NS_PING
            && namespace != NS_SESSION
            && let Some(session) = self.sessions.get_mut(&id)
            && session.logged_namespaces.len() < MAX_LOGGED_NAMESPACES
        {
            let shown: String = namespace.chars().take(80).collect();
            if !session.logged_namespaces.contains(&shown) {
                debug!(
                    "chat: iq c={id} uid={} ns={}",
                    session.uid_or_dash(),
                    log_text(&shown, 80)
                );
                session.logged_namespaces.push(shown);
            }
        }
        self.send(
            id,
            &format!(r#"<iq xmlns="jabber:client" type="result" id="{}"/>"#, escape(&iq_id)),
        );
    }

    fn bind_result(iq_id: &str, full: &str) -> String {
        format!(
            r#"<iq xmlns="jabber:client" type="result" id="{}"><bind xmlns="{NS_BIND}"><jid>{}</jid></bind></iq>"#,
            escape(iq_id),
            escape(full)
        )
    }

    async fn bind(&mut self, id: u64, iq_id: &str, requested: String) {
        let now = self.now();
        let Some(session) = self.sessions.get(&id) else { return };
        if session.resource.is_some() {
            let answer = Self::bind_result(iq_id, &session.full_jid());
            return self.send(id, &answer);
        }
        if utf16_len(&requested) > MAX_RESOURCE || xmpp::has_control_character(&requested) {
            let answer = format!(
                r#"<iq xmlns="jabber:client" type="error" id="{}"><error type="modify"><bad-request xmlns="{NS_STANZAS}"/></error></iq>"#,
                escape(iq_id)
            );
            return self.send(id, &answer);
        }
        let resource = if requested.is_empty() {
            format!("srv-{}", random_hex(16))
        } else {
            requested
        };
        let Some(uid) = session.uid.clone() else { return };
        let account = session.account.clone();
        let same: Vec<u64> = self
            .bound_of(&uid)
            .into_iter()
            .filter(|s| {
                self.sessions
                    .get(s)
                    .is_some_and(|s| s.resource.as_deref() == Some(&resource))
            })
            .collect();
        for other in same {
            self.end(other, EndReason::Replaced).await;
        }
        let bound = self.bound_of(&uid);
        if bound.len() >= MAX_BOUND_PER_UID {
            // Least recently heard from; the oldest bound first among equals
            let quiet = bound
                .iter()
                .enumerate()
                .filter_map(|(order, s)| self.sessions.get(s).map(|s| (s.last_inbound, order, s.id)))
                .min()
                .map(|(_, _, s)| s);
            if let Some(quiet) = quiet {
                self.end(quiet, EndReason::Replaced).await;
            }
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.resource = Some(resource.clone());
        let domain = session.domain.clone();
        let full = session.full_jid();
        let list = self.bound.entry(uid.clone()).or_default();
        list.push(id);
        info!(
            "chat: bound c={id} uid={uid} resource={} domain={} sessions={}",
            log_text(&resource, 80),
            log_text(&domain, 80),
            list.len()
        );
        self.send(id, &Self::bind_result(iq_id, &full));
        for other in self.bound_of(&uid) {
            if other == id {
                continue;
            }
            let ping_due = self
                .sessions
                .get(&other)
                .is_some_and(|s| s.ping.as_ref().is_none_or(|(_, deadline)| *deadline > now + GHOST_PING));
            if ping_due {
                self.ping(other, GHOST_PING, now);
            }
        }
        if let Some(account) = &account {
            self.federation.presence(account, &uid, true).await;
            self.host.presence(account, &uid, true).await;
        }
        self.deliver_waiting(id).await;
    }

    pub(crate) fn bound_of(&self, uid: &str) -> Vec<u64> {
        self.bound.get(uid).cloned().unwrap_or_default()
    }

    fn ping(&mut self, id: u64, timeout: i64, now: i64) {
        let ping_id = format!("sp{}", self.next_ping);
        self.next_ping += 1;
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        session.ping = Some((ping_id.clone(), now + timeout));
        let stanza = format!(
            r#"<iq xmlns="jabber:client" type="get" id="{ping_id}" from="{}" to="{}"><ping xmlns="{NS_PING}"/></iq>"#,
            escape(&session.domain),
            escape(&session.full_jid())
        );
        self.send(id, &stanza);
    }

    async fn presence(&mut self, id: u64, root: &Element) {
        let kind = root.attr("type");
        let Some(to) = root.attr("to") else {
            match kind {
                Some("unavailable") => {
                    self.leave_all(id, crate::muc::Leave::Left);
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
        let Some(jid) = (utf16_len(to) <= MAX_TO).then(|| parse_jid(to)).flatten() else {
            debug!("chat: presence with a bad to c={id}");
            return;
        };
        let Some(session) = self.sessions.get(&id) else { return };
        let domain = jid.domain.to_ascii_lowercase();
        let for_room = crate::muc::is_muc_domain(&session.domain, &jid.domain)
            || domain.starts_with("muc.")
            || domain.starts_with("conference.")
            || root.child("x").and_then(|x| x.attr("xmlns")) == Some(xmpp::NS_MUC);
        if !for_room {
            debug!("chat: directed presence dropped c={id}");
            return;
        }
        match kind {
            None => self.join(id, jid).await,
            Some("unavailable") => self.leave(id, &jid),
            Some(_) => {}
        }
    }

    async fn message(&mut self, id: u64, root: &Element) {
        let to = root.attr("to").unwrap_or_default();
        let jid = (utf16_len(to) <= MAX_TO).then(|| parse_jid(to)).flatten();
        let kind = root.attr("type");
        let Some(domain) = self.sessions.get(&id).map(|s| s.domain.clone()) else {
            return;
        };
        match (kind, jid) {
            (Some("groupchat"), Some(jid)) => self.room_message(id, root, jid).await,
            (None | Some("chat" | "normal"), Some(jid)) if !crate::muc::is_muc_domain(&domain, &jid.domain) => {
                self.whisper(id, root, &jid.local, jid.resource.as_deref()).await;
            }
            _ => {
                let shown: String = kind.unwrap_or("undefined").chars().take(16).collect();
                debug!("chat: message ignored c={id} type={}", log_text(&shown, 16));
            }
        }
    }

    /// The account a JID local part names: a bound session's, chat's own record, the host's, the network's.
    pub(crate) async fn resolve_local(&self, local: &str) -> Option<GlobalId> {
        if let Some(account) = self
            .bound_of(local)
            .first()
            .and_then(|s| self.sessions.get(s))
            .and_then(|s| s.account.clone())
        {
            return Some(account);
        }
        match self.history.account_of(local).await {
            Ok(Some(text)) => {
                if let Ok(account) = text.parse() {
                    return Some(account);
                }
            }
            Ok(None) => {}
            Err(e) => warn!("chat: account lookup failed: {e}"),
        }
        if let Some(account) = self.host.account_of(local).await {
            return Some(account);
        }
        self.federation.resolve(local).await
    }

    fn whisper_refused(sender: &str, target: &str, length: usize, reason: &str) {
        info!("chat: whisper refused uid={sender} to={target} length={length} reason={reason}");
    }

    async fn whisper(&mut self, id: u64, root: &Element, target: &str, resource: Option<&str>) {
        let now = self.now();
        let body = xmpp::text_of(root.child("body"));
        let length = body.chars().count();
        let message_id = root.attr("id").filter(|i| utf16_len(i) <= MAX_ID).map(str::to_owned);
        let shown_target = if is_authcid_shape(target) { target } else { "?" };
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let (Some(uid), Some(sender)) = (session.uid.clone(), session.account.clone()) else {
            return;
        };
        if !session.messages.take(crate::muc::MESSAGE_BURST, 1000.0, now) {
            if session.last_limit_log.is_none_or(|at| now - at >= MINUTE) {
                session.last_limit_log = Some(now);
                Self::whisper_refused(&uid, shown_target, length, "limit");
            }
            return;
        }
        if length == 0 || length > BODY_LIMIT {
            return Self::whisper_refused(&uid, shown_target, length, "size");
        }
        if let Some(command) = crate::moderation::parse_report(&body) {
            let from = session.domain.clone();
            return self.report(id, command, "chat", from, None).await;
        }
        if target == uid {
            return Self::whisper_refused(&uid, shown_target, length, "self");
        }
        if session.muted(now) {
            return Self::whisper_refused(&uid, shown_target, length, "muted");
        }
        if !is_authcid_shape(target) {
            return Self::whisper_refused(&uid, shown_target, length, "unknown");
        }
        let (module, name) = (session.module.clone(), session.name.clone());
        let Some(recipient) = self.resolve_local(target).await else {
            return Self::whisper_refused(&uid, shown_target, length, "unknown");
        };
        if self.whisper_blocked(module.as_ref(), &sender, &recipient).await {
            return Self::whisper_refused(&uid, shown_target, length, "blocked");
        }
        let mid = format!("dm-{}", random_hex(16));
        let rated = self.options.moderation.rate(&body, Channel::Direct);
        let (verdict, shown) = (rated.verdict, rated.shown);
        self.options.moderation.record(Line {
            id: mid.clone(),
            at: now,
            channel: Channel::Direct.as_str(),
            room_id: None,
            sender: Person {
                user_id: uid.clone(),
                name: name.clone().unwrap_or_else(|| uid.clone()),
            },
            recipient: Some(Person {
                user_id: target.to_owned(),
                name: target.to_owned(),
            }),
            body,
            verdict,
        });
        if verdict == Verdict::Blocked {
            return Self::whisper_refused(&uid, shown_target, length, "filter");
        }
        let mut recipients: Vec<u64> = self
            .bound_of(target)
            .into_iter()
            .filter(|s| {
                self.sessions
                    .get(s)
                    .is_some_and(|s| s.account.as_ref() == Some(&recipient))
            })
            .collect();
        if let Some(resource) = resource {
            let exact: Vec<u64> = recipients
                .iter()
                .copied()
                .filter(|s| {
                    self.sessions
                        .get(s)
                        .is_some_and(|s| s.resource.as_deref() == Some(resource))
                })
                .collect();
            if !exact.is_empty() {
                recipients = exact;
            }
        }
        let delivered = !recipients.is_empty();
        if delivered {
            self.mids.remember(&mid);
        }
        let elsewhere = self.federation.is_federated() && recipient.owner() != &self.node.node;
        let delivered_at = (delivered || elsewhere).then_some(now);
        let stored = NewDirect {
            sender: sender.to_string(),
            sender_local: uid.clone(),
            recipient: recipient.to_string(),
            body: shown.clone(),
            stanza_id: message_id.clone(),
            at: now,
            delivered_at,
            mid: Some(mid.clone()),
        };
        if let Err(e) = self.history.add_direct_once(stored).await {
            error!("chat: could not store a whisper uid={uid}: {e}");
        }
        let from = self.sessions.get(&id).map(Session::full_jid).unwrap_or_default();
        for r in &recipients {
            let Some(to) = self.sessions.get(r).map(Session::full_jid) else {
                continue;
            };
            let stanza = format!(
                r#"<message xmlns="jabber:client" type="chat" from="{}" to="{}"{}><body>{}</body></message>"#,
                escape(&from),
                escape(&to),
                id_attr(message_id.as_deref()),
                escape(&shown)
            );
            self.send(*r, &stanza);
        }
        info!(
            "chat: whisper uid={uid} to={shown_target} length={length} delivered={}",
            recipients.len()
        );
        let envelope = Envelope::Direct {
            mid,
            from: sender,
            from_local: uid,
            to: recipient,
            body: shown,
            id: message_id,
            at: now,
            delivered,
        };
        if let Err(e) = self.federation.publish(envelope).await {
            warn!("chat: whisper not federated: {e}");
        }
    }

    async fn whisper_blocked(&self, module: Option<&ModuleId>, sender: &GlobalId, recipient: &GlobalId) -> bool {
        if self.host.blocked(sender, recipient).await {
            return true;
        }
        let Some(module) = module.filter(|m| self.answers_chat(Some(m))) else {
            return false;
        };
        let query = ChatQuery::MayWhisper {
            from: sender.clone(),
            to: recipient.clone(),
        };
        matches!(self.host.ask(module, query).await, Ok(ChatAnswer::MayWhisper(false)))
    }

    /// A `/report` command: answered to the sender alone, never delivered.
    pub(crate) async fn report(
        &mut self,
        id: u64,
        command: Result<(String, &'static str, String), ()>,
        kind: &str,
        from: String,
        room_key: Option<String>,
    ) {
        let now = self.now();
        let Some(session) = self.sessions.get(&id) else { return };
        let uid = session.uid.clone().unwrap_or_default();
        let name = session.name.clone().unwrap_or_else(|| uid.clone());
        let answer = match command {
            Err(_) => "Usage: /report <player> [category] <what happened>",
            Ok(_) if !self.options.moderation.is_on() => "Reports are not set up on this server.",
            Ok(_) if !self.log_once.check(&format!("report|{uid}"), REPORT_WINDOW, now) => {
                "Please wait a little before sending another report."
            }
            Ok((target, category, description)) => {
                let target_uid = self
                    .sessions
                    .values()
                    .find(|s| {
                        s.uid.as_deref() == Some(target.as_str())
                            || s.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&target))
                    })
                    .and_then(|s| s.uid.clone());
                let report = crate::moderation::Report {
                    id: format!("report-{}", random_hex(16)),
                    source: "game",
                    reporter: Person {
                        user_id: uid.clone(),
                        name,
                    },
                    target: crate::moderation::TargetPerson {
                        user_id: target_uid,
                        name: target,
                    },
                    category,
                    description,
                    context: crate::moderation::ReportContext {
                        room_id: room_key,
                        at: now,
                    },
                };
                let queued = self.options.moderation.report(report);
                info!(
                    "chat: report c={id} uid={uid} category={} queued={queued}",
                    log_text(category, 32)
                );
                if queued {
                    "Report sent. Thank you."
                } else {
                    "The report could not be sent. Please try again later."
                }
            }
        };
        let Some(session) = self.sessions.get(&id) else { return };
        let stanza = format!(
            r#"<message xmlns="jabber:client" type="{kind}" from="{}" to="{}"><body>{}</body></message>"#,
            escape(&from),
            escape(&session.full_jid()),
            escape(answer)
        );
        self.send(id, &stanza);
    }

    fn waiting_stanza(
        from_local: &str,
        domain: &str,
        to: &str,
        stanza_id: Option<&str>,
        body: &str,
        at: i64,
    ) -> String {
        format!(
            r#"<message xmlns="jabber:client" type="chat" from="{}@{}" to="{}"{}><body>{}</body><delay xmlns="{}" from="{}" stamp="{}"/></message>"#,
            escape(from_local),
            escape(domain),
            escape(to),
            id_attr(stanza_id),
            escape(body),
            xmpp::NS_DELAY,
            escape(domain),
            stamp(at)
        )
    }

    /// Direct messages that waited for this session's account, at its bind.
    async fn deliver_waiting(&mut self, id: u64) {
        let now = self.now();
        let Some(session) = self.sessions.get(&id) else { return };
        let Some(account) = session.account.clone() else { return };
        let (domain, full, uid) = (
            session.domain.clone(),
            session.full_jid(),
            session.uid_or_dash().to_owned(),
        );
        let mut total = 0;
        let account_text = account.to_string();
        match self
            .history
            .pending_for(&account_text, self.settings.offline_batch)
            .await
        {
            Ok(waiting) => {
                let mut ids = Vec::with_capacity(waiting.len());
                for message in waiting {
                    ids.push(message.id);
                    if let Some(mid) = &message.mid
                        && !self.mids.remember(mid)
                    {
                        continue;
                    }
                    let stanza = Self::waiting_stanza(
                        &message.sender_local,
                        &domain,
                        &full,
                        message.stanza_id.as_deref(),
                        &message.body,
                        message.at,
                    );
                    self.send(id, &stanza);
                    total += 1;
                }
                if !ids.is_empty()
                    && let Err(e) = self.history.mark_delivered(&account_text, ids, now).await
                {
                    error!("chat: could not mark messages delivered uid={uid}: {e}");
                }
            }
            Err(e) => error!("chat: could not read waiting messages uid={uid}: {e}"),
        }
        if self.federation.is_federated() && account.owner() != &self.node.node {
            let pulled = self
                .federation
                .pull_waiting(&account, self.settings.offline_batch)
                .await;
            let mut mids = Vec::with_capacity(pulled.len());
            let mut stanzas = Vec::new();
            for message in pulled {
                mids.push(message.mid.clone());
                if !self.mids.remember(&message.mid) {
                    continue;
                }
                let stored = NewDirect {
                    sender: message.from.to_string(),
                    sender_local: message.from_local.clone(),
                    recipient: message.to.to_string(),
                    body: message.body.clone(),
                    stanza_id: message.stanza_id.clone(),
                    at: message.at,
                    delivered_at: Some(now),
                    mid: Some(message.mid.clone()),
                };
                if let Err(e) = self.history.add_direct_once(stored).await {
                    error!("chat: could not store a pulled message uid={uid}: {e}");
                }
                stanzas.push(Self::waiting_stanza(
                    &message.from_local,
                    &domain,
                    &full,
                    message.stanza_id.as_deref(),
                    &message.body,
                    message.at,
                ));
            }
            if !mids.is_empty() {
                self.federation.ack_waiting(&account, mids, now).await;
            }
            for stanza in stanzas {
                self.send(id, &stanza);
                total += 1;
            }
        }
        if total > 0 {
            info!("chat: offline messages delivered c={id} uid={uid} count={total}");
        }
    }

    async fn remote_direct(&mut self, message: DirectWire) -> bool {
        let recipients: Vec<u64> = self
            .sessions
            .values()
            .filter(|s| !s.ended && s.resource.is_some() && s.account.as_ref() == Some(&message.to))
            .map(|s| s.id)
            .collect();
        if recipients.is_empty() {
            return false;
        }
        if !self.mids.remember(&message.mid) {
            return true;
        }
        if self.host.blocked(&message.from, &message.to).await {
            return true;
        }
        for r in recipients {
            let Some(session) = self.sessions.get(&r) else { continue };
            let stanza = format!(
                r#"<message xmlns="jabber:client" type="chat" from="{}@{}" to="{}"{}><body>{}</body></message>"#,
                escape(&message.from_local),
                escape(&session.domain),
                escape(&session.full_jid()),
                id_attr(message.stanza_id.as_deref()),
                escape(&message.body)
            );
            self.send(r, &stanza);
        }
        true
    }

    pub(crate) fn remember_mid(&mut self, mid: &str) -> bool {
        self.mids.remember(mid)
    }

    pub(crate) async fn tick(&mut self) {
        let now = self.now();
        let limit = self.options.output_limit;
        let ids: Vec<u64> = self.sessions.keys().copied().collect();
        for id in ids {
            let Some(session) = self.sessions.get(&id) else {
                continue;
            };
            if session.ended {
                continue;
            }
            if session.backlogged || session.writer.unsent() > limit {
                self.end(id, EndReason::Backlog).await;
            } else if session.uid.is_none() {
                if now - session.opened >= LOGIN_DEADLINE {
                    self.end(id, EndReason::Timeout).await;
                }
            } else if session.resource.is_none() {
                if now - session.login_at >= BIND_DEADLINE {
                    let uid = session.uid.clone().unwrap_or_default();
                    let times = self.bind_timeouts.entry(uid.clone()).or_default();
                    times.retain(|&at| now - at < TEN_MINUTES);
                    times.push(now);
                    if times.len() >= BIND_TIMEOUTS {
                        self.uid_wait.insert(uid, now + UID_WAIT);
                    }
                    self.end(id, EndReason::Timeout).await;
                }
            } else {
                match session.ping {
                    Some((_, deadline)) => {
                        if now >= deadline {
                            self.end(id, EndReason::PingTimeout).await;
                        }
                    }
                    None => {
                        if now - session.last_inbound >= IDLE_BEFORE_PING {
                            self.ping(id, PING_TIMEOUT, now);
                        }
                    }
                }
            }
        }
        if self.last_sweep.is_none_or(|at| now - at >= SWEEP_EVERY) {
            self.last_sweep = Some(now);
            self.sweep().await;
        }
        self.federation.tick(now).await;
        self.address_wait.retain(|_, &mut until| now < until);
        self.uid_wait.retain(|_, &mut until| now < until);
        self.log_once.forget(now);
        self.bind_timeouts
            .retain(|_, times| times.iter().any(|&at| now - at < TEN_MINUTES));
        self.sasl_failures.retain(|_, times| {
            times.retain(|&at| now - at < TEN_MINUTES);
            !times.is_empty()
        });
    }

    async fn shutdown(&mut self) {
        let writers: Vec<Writer> = self.sessions.values().map(|s| s.writer.clone()).collect();
        let ids: Vec<u64> = self.sessions.keys().copied().collect();
        for id in ids {
            self.end(id, EndReason::Shutdown).await;
        }
        let waited = tokio::time::Instant::now();
        while writers.iter().any(|w| !w.is_closed()) && waited.elapsed() < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for writer in writers {
            writer.send(Out::Cut);
        }
    }

    /// The nick check's setting, for rooms.
    pub(crate) fn nick_check(&self) -> NickCheck {
        self.settings.nick_check
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
    use base64::Engine as _;

    use super::*;

    #[test]
    fn sasl_payloads_decode_like_buffer_from() {
        assert_eq!(node_base64("AGEAYg=="), b"\0a\0b");
        assert_eq!(node_base64("AGEAYg"), b"\0a\0b");
        assert_eq!(node_base64("QQ"), b"A");
        assert_eq!(node_base64("Q"), b"");
        assert_eq!(node_base64("-_8"), node_base64("+/8"));
        assert_eq!(node_base64("QUJD$QUJD"), b"ABC");

        let plain = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
        assert_eq!(
            decode_plain(&plain("\0UID-1\0tok")),
            Some((String::from("UID-1"), String::from("tok")))
        );
        assert_eq!(
            decode_plain(&format!("  {}\n", plain("UID-1\0UID-1\0tok"))),
            Some((String::from("UID-1"), String::from("tok")))
        );
        for bad in [
            plain("other\0UID-1\0tok"),
            plain("\0UID-1\0"),
            plain("\0a@b\0tok"),
            plain("\0UID-1\0tok\0x"),
            plain("\0UID-1"),
            String::new(),
            String::from("==="),
            String::from("AGEAYg==="),
            String::from("AG-AYg"),
            "A".repeat(12_004),
            plain(&format!("\0u\0{}", "t".repeat(8193))),
        ] {
            assert_eq!(decode_plain(&bad), None, "{bad:.40}");
        }
    }

    #[test]
    fn log_once_keeps_a_window() {
        let mut once = LogOnce::default();
        assert!(once.check("a", 1000, 0));
        assert!(!once.check("a", 1000, 999));
        assert!(once.check("a", 1000, 1000));
        assert!(once.check("b", 1000, 0));
        once.forget(TEN_MINUTES + 1000);
        assert!(once.seen.is_empty());
        for i in 0..(LOG_ONCE_KEYS + 5) {
            assert!(once.check(&i.to_string(), 1000, 0));
        }
        assert_eq!(once.seen.len(), LOG_ONCE_KEYS);
        assert!(!once.seen.contains_key("0"));
    }

    #[test]
    fn authcids_are_jid_local_parts() {
        for good in ["UID-1234", "6f1c2c1e-6a2b-4c4e-9b51-1d2f3e4a5b6c", "player"] {
            assert!(is_authcid_shape(good), "{good}");
        }
        for bad in ["", "a@b", "a/b", "a b", "a\0", "a\u{a0}b", &"x".repeat(129)] {
            assert!(!is_authcid_shape(bad), "{bad:?}");
        }
        assert!(is_authcid_shape(&"x".repeat(128)));
    }

    #[test]
    fn stamps_are_utc_with_milliseconds() {
        assert_eq!(stamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(stamp(1_793_000_000_123), "2026-10-26T07:33:20.123Z");
        assert_eq!(stamp(i64::MAX), "1970-01-01T00:00:00.000Z");
        assert_eq!(random_hex(8).len(), 16);
        assert!(
            random_hex(16)
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert!(now_ms() > 1_700_000_000_000);
    }

    #[test]
    fn seen_mids_are_bounded() {
        let mut mids = Mids::default();
        assert!(mids.remember(""));
        assert!(mids.remember(""));
        assert!(mids.remember("a"));
        assert!(!mids.remember("a"));
        for i in 0..SEEN_MIDS {
            mids.remember(&i.to_string());
        }
        assert!(mids.remember("a"));
        assert_eq!(mids.order.len(), SEEN_MIDS);
    }
}
