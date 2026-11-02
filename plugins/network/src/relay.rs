//! Operator actions relayed between nodes over `Service::Admin` (DN11, DD16, DD23).
//!
//! When an operator kicks, times out, bans, unbans, lifts a timeout or revokes a key on any node in a
//! network, that node resolves the player's home through the directory and sends the action over
//! `Service::Admin` to the home and to every reachable peer, each carrying an idempotency [`RequestKey`].
//! The network plugin stays game-agnostic: an action is an [`OperationName`], optional [`GlobalId`]
//! account, originating admin key id (`actor_key`, never a secret key), operator id, optional second
//! operator `approved_by` (DH6), and an opaque JSON [`HomeBody`].
//!
//! Each receiving node verifies that the caller (`from`, authenticated by the node link) is a listed
//! peer, checks its idempotency cache for `(from, key)`, revokes session tickets on [`Tickets`] when
//! `revoke_tickets` is set, and hands the [`RelayedAction`] to its [`AdminAuthority`]. Neither an
//! account key (`RRK_...`) nor an admin secret key (`RRA_...`) may appear anywhere in a relay message.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use module_api::BoxFuture;
use module_api::network::{GlobalId, NodeId, OperationName};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::home::Tickets;
use crate::home::protocol::{CleanText, DecodeError, HomeBody, RequestKey, holds_account_key, looks_like_account_key};
use crate::link::{LinkError, NodeService, PeerRoster, Service};

pub const ADMIN_RELAY_VERSION: u32 = 1;

/// Maximum bytes of an encoded [`AdminRelayRequest`] or [`AdminRelayResponse`] on a node link.
pub const MAX_ADMIN_WIRE: usize = 64 * 1024;

/// Standard admin relay operation names (DN11).
pub mod operations {
    pub const KICK: &str = "admin.kick";
    pub const BAN: &str = "admin.ban";
    pub const TIMEOUT: &str = "admin.timeout";
    pub const UNBAN: &str = "admin.unban";
    pub const LIFT_TIMEOUT: &str = "admin.lift_timeout";
    pub const REVOKE_KEY: &str = "admin.revoke_key";
}

/// Refuses anything shaped like an issued admin secret key (`RRA_<id>_<secret>`), so only the public
/// key id (`owner` or 8 lower-case hex digits) ever crosses a node link.
pub fn looks_like_admin_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_key_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    for (i, window) in bytes.windows(4).enumerate() {
        if window.eq_ignore_ascii_case(b"rra_") {
            let before_ok = i == 0 || !is_key_char(bytes[i - 1]);
            let tail = bytes[i + 4..].iter().take_while(|&&b| is_key_char(b)).count();
            if before_ok && tail >= 16 {
                return true;
            }
        }
    }
    false
}

/// Whether any string inside `value` looks like an account key (`RRK_...`) or an admin secret key (`RRA_...`).
pub fn holds_secret_key(value: &Value) -> bool {
    if holds_account_key(value) {
        return true;
    }
    fn walk(value: &Value) -> bool {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
            Value::String(text) => looks_like_admin_key(text),
            Value::Array(items) => items.iter().any(walk),
            Value::Object(map) => map.iter().any(|(k, v)| looks_like_admin_key(k) || walk(v)),
        }
    }
    walk(value)
}

fn text_holds_secret(text: &str) -> bool {
    looks_like_account_key(text) || looks_like_admin_key(text)
}

/// An operator action sent over `Service::Admin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayAction {
    /// Idempotency key (`[A-Za-z0-9_-]{8,64}`): each receiver applies one key from one peer at most once.
    pub key: RequestKey,
    /// Opaque operation name, such as `admin.ban` or `admin.kick`.
    pub operation: OperationName,
    /// The player's network-wide account id when resolved through the directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<GlobalId>,
    /// Whether this action ends the account's session tickets (`ban`, `timeout`, `kick`).
    #[serde(default)]
    pub revoke_tickets: bool,
    /// The originating admin key id (`owner` or 8 lower-case hex digits; never a secret key).
    pub actor_key: CleanText,
    /// The operator id (`discord:<id>`, `host`, etc.).
    pub operator: CleanText,
    /// The second operator's key id when a guarded action ran on their approval (DH6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<CleanText>,
    /// Opaque JSON payload for the receiver's [`AdminAuthority`].
    #[serde(default)]
    pub body: HomeBody,
}

impl RelayAction {
    fn check_no_secrets(&self) -> Result<(), DecodeError> {
        if text_holds_secret(self.actor_key.as_str())
            || text_holds_secret(self.operator.as_str())
            || self
                .approved_by
                .as_ref()
                .is_some_and(|by| text_holds_secret(by.as_str()))
            || holds_secret_key(self.body.value())
        {
            return Err(DecodeError::Field(String::from(
                "holds something shaped like an account or admin secret key",
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminRelayRequest {
    pub version: u32,
    pub action: RelayAction,
}

impl AdminRelayRequest {
    pub fn new(action: RelayAction) -> Self {
        Self {
            version: ADMIN_RELAY_VERSION,
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "reason", deny_unknown_fields)]
pub enum AdminRelayRefusal {
    #[error("admin relay protocol {supported} is required")]
    UnsupportedVersion { supported: u32 },
    #[error("the admin relay message could not be read")]
    Malformed,
    #[error("the admin relay message is larger than the link allows")]
    TooLarge,
    #[error("the caller is not a peer this node lists")]
    UnknownPeer,
    #[error("the request key was already used for a different action")]
    KeyReused,
    #[error("that request is already running here")]
    InFlight,
    #[error("this node is busy with too many relayed admin actions")]
    Busy,
    #[error("{message}")]
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<CleanText>,
        message: CleanText,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", deny_unknown_fields)]
pub enum AdminRelayReply {
    Applied {
        #[serde(default)]
        body: HomeBody,
    },
    Refused {
        refusal: AdminRelayRefusal,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminRelayResponse {
    pub version: u32,
    pub reply: AdminRelayReply,
}

impl AdminRelayResponse {
    pub fn new(reply: AdminRelayReply) -> Self {
        Self {
            version: ADMIN_RELAY_VERSION,
            reply,
        }
    }
}

pub fn encode_request(request: &AdminRelayRequest) -> Result<Vec<u8>, DecodeError> {
    if request.version != ADMIN_RELAY_VERSION {
        return Err(DecodeError::Version(request.version));
    }
    request.action.check_no_secrets()?;
    let bytes = serde_json::to_vec(request).map_err(|_| DecodeError::Json("could not serialise".into()))?;
    if bytes.len() > MAX_ADMIN_WIRE {
        return Err(DecodeError::TooLarge);
    }
    Ok(bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<AdminRelayRequest, DecodeError> {
    if bytes.len() > MAX_ADMIN_WIRE {
        return Err(DecodeError::TooLarge);
    }
    #[derive(Deserialize)]
    struct VersionOnly {
        version: u32,
    }
    let header: VersionOnly = serde_json::from_slice(bytes).map_err(|error| DecodeError::Json(error.to_string()))?;
    if header.version != ADMIN_RELAY_VERSION {
        return Err(DecodeError::Version(header.version));
    }
    let request: AdminRelayRequest =
        serde_json::from_slice(bytes).map_err(|error| DecodeError::Json(error.to_string()))?;
    request.action.check_no_secrets()?;
    Ok(request)
}

pub fn encode_response(response: &AdminRelayResponse) -> Result<Vec<u8>, DecodeError> {
    if response.version != ADMIN_RELAY_VERSION {
        return Err(DecodeError::Version(response.version));
    }
    if let AdminRelayReply::Applied { body } = &response.reply
        && holds_secret_key(body.value())
    {
        return Err(DecodeError::Field(String::from(
            "holds something shaped like an account or admin secret key",
        )));
    }
    let bytes = serde_json::to_vec(response).map_err(|_| DecodeError::Json("could not serialise".into()))?;
    if bytes.len() > MAX_ADMIN_WIRE {
        return Err(DecodeError::TooLarge);
    }
    Ok(bytes)
}

pub fn decode_response(bytes: &[u8]) -> Result<AdminRelayResponse, DecodeError> {
    if bytes.len() > MAX_ADMIN_WIRE {
        return Err(DecodeError::TooLarge);
    }
    #[derive(Deserialize)]
    struct VersionOnly {
        version: u32,
    }
    let header: VersionOnly = serde_json::from_slice(bytes).map_err(|error| DecodeError::Json(error.to_string()))?;
    if header.version != ADMIN_RELAY_VERSION {
        return Err(DecodeError::Version(header.version));
    }
    let response: AdminRelayResponse =
        serde_json::from_slice(bytes).map_err(|error| DecodeError::Json(error.to_string()))?;
    if let AdminRelayReply::Applied { body } = &response.reply
        && holds_secret_key(body.value())
    {
        return Err(DecodeError::Field(String::from(
            "holds something shaped like an account or admin secret key",
        )));
    }
    Ok(response)
}

// ------------------------------------------------------------------ service side

/// An action arriving from a listed peer (`via_node`) to be applied by the local [`AdminAuthority`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayedAction {
    pub via_node: NodeId,
    pub key: RequestKey,
    pub operation: OperationName,
    pub account: Option<GlobalId>,
    pub revoke_tickets: bool,
    /// The originating admin key id (`owner` or 8 lower-case hex digits). The local actor is `admin:<actor_key>`.
    pub actor_key: String,
    pub operator: String,
    pub approved_by: Option<String>,
    pub body: Value,
}

/// An error returned by [`AdminAuthority::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminRelayError {
    pub code: Option<String>,
    pub message: String,
}

impl AdminRelayError {
    pub fn new(code: Option<impl Into<String>>, message: impl Into<String>) -> Self {
        Self {
            code: code.map(Into::into),
            message: message.into(),
        }
    }
}

/// The local admin service that applies a [`RelayedAction`] received over `Service::Admin`.
pub trait AdminAuthority: Send + Sync {
    fn apply<'a>(&'a self, action: RelayedAction) -> BoxFuture<'a, Result<Value, AdminRelayError>>;
}

#[derive(Debug, Clone, Copy)]
pub struct AdminRelayConfig {
    pub call_timeout: Duration,
    pub max_in_flight: usize,
    pub dedup_entries: usize,
    pub dedup_bytes: usize,
}

impl Default for AdminRelayConfig {
    fn default() -> Self {
        Self {
            call_timeout: Duration::from_secs(20),
            max_in_flight: 64,
            dedup_entries: 1024,
            dedup_bytes: 8 << 20,
        }
    }
}

enum Slot {
    Running {
        since: Instant,
        digest: [u8; 32],
    },
    Done {
        digest: [u8; 32],
        body: Value,
        bytes: usize,
    },
}

enum Begin {
    Fresh,
    Running,
    Done(Value),
    Reused,
}

#[derive(Default)]
struct Dedup {
    slots: HashMap<(NodeId, RequestKey), Slot>,
    done: VecDeque<(NodeId, RequestKey)>,
    bytes: usize,
}

impl Dedup {
    fn begin(&mut self, id: &(NodeId, RequestKey), digest: [u8; 32], stale: Duration) -> Begin {
        match self.slots.get(id) {
            Some(Slot::Done { digest: have, body, .. }) => {
                if *have == digest {
                    Begin::Done(body.clone())
                } else {
                    Begin::Reused
                }
            }
            Some(Slot::Running { since, digest: have }) if since.elapsed() < stale => {
                if *have == digest {
                    Begin::Running
                } else {
                    Begin::Reused
                }
            }
            _ => {
                self.slots.insert(
                    id.clone(),
                    Slot::Running {
                        since: Instant::now(),
                        digest,
                    },
                );
                Begin::Fresh
            }
        }
    }

    fn finish(&mut self, id: &(NodeId, RequestKey), result: Option<(Value, [u8; 32])>, entries: usize, cap: usize) {
        match result {
            None => {
                if matches!(self.slots.get(id), Some(Slot::Running { .. })) {
                    self.slots.remove(id);
                }
            }
            Some((body, digest)) => {
                let bytes = serde_json::to_vec(&body).map_or(0, |b| b.len()) + 128;
                self.slots.insert(id.clone(), Slot::Done { digest, body, bytes });
                self.done.push_back(id.clone());
                self.bytes += bytes;
                while self.done.len() > entries || self.bytes > cap {
                    let Some(old) = self.done.pop_front() else { break };
                    if let Some(Slot::Done { bytes, .. }) = self.slots.remove(&old) {
                        self.bytes = self.bytes.saturating_sub(bytes);
                    }
                }
            }
        }
    }
}

fn digest_of(action: &RelayAction) -> [u8; 32] {
    let mut hash = Sha256::new();
    let account = action.account.as_ref().map(ToString::to_string).unwrap_or_default();
    let approved_by = action.approved_by.as_ref().map_or("", CleanText::as_str);
    let parts: [&[u8]; 6] = [
        action.operation.as_str().as_bytes(),
        account.as_bytes(),
        &[u8::from(action.revoke_tickets)],
        action.actor_key.as_str().as_bytes(),
        action.operator.as_str().as_bytes(),
        approved_by.as_bytes(),
    ];
    for part in parts {
        hash.update(part);
        hash.update([0]);
    }
    hash.update(serde_json::to_vec(action.body.value()).unwrap_or_default());
    hash.finalize().into()
}

struct Inner {
    me: NodeId,
    peers: RwLock<BTreeSet<NodeId>>,
    roster: RwLock<Option<Arc<dyn PeerRoster>>>,
    tickets: RwLock<Option<Arc<Tickets>>>,
    authority: Arc<dyn AdminAuthority>,
    config: AdminRelayConfig,
    dedup: Arc<Mutex<Dedup>>,
    permits: Arc<Semaphore>,
}

/// The answering side of `Service::Admin`.
#[derive(Clone)]
pub struct AdminRelayService {
    inner: Arc<Inner>,
}

impl AdminRelayService {
    pub fn new(
        me: NodeId,
        peers: impl IntoIterator<Item = NodeId>,
        tickets: Option<Arc<Tickets>>,
        authority: Arc<dyn AdminAuthority>,
        config: AdminRelayConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                permits: Arc::new(Semaphore::new(config.max_in_flight)),
                me,
                peers: RwLock::new(peers.into_iter().collect()),
                roster: RwLock::new(None),
                tickets: RwLock::new(tickets),
                authority,
                config,
                dedup: Arc::new(Mutex::new(Dedup::default())),
            }),
        }
    }

    pub fn with_roster(self, roster: Arc<dyn PeerRoster>) -> Self {
        *self.inner.roster.write().unwrap_or_else(PoisonError::into_inner) = Some(roster);
        self
    }

    pub fn set_roster(&self, roster: Option<Arc<dyn PeerRoster>>) {
        *self.inner.roster.write().unwrap_or_else(PoisonError::into_inner) = roster;
    }

    pub fn set_tickets(&self, tickets: Option<Arc<Tickets>>) {
        *self.inner.tickets.write().unwrap_or_else(PoisonError::into_inner) = tickets;
    }

    pub fn set_peers(&self, peers: impl IntoIterator<Item = NodeId>) {
        *self.inner.peers.write().unwrap_or_else(PoisonError::into_inner) = peers.into_iter().collect();
    }

    fn is_peer(&self, from: &NodeId) -> bool {
        if from == &self.inner.me {
            return false;
        }
        if self
            .inner
            .peers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(from)
        {
            return true;
        }
        self.inner
            .roster
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|r| r.peers().iter().any(|p| p == from))
    }

    async fn apply_ticket_effect(&self, action: &RelayAction) -> Result<(), crate::home::RevocationPersistError> {
        let Some(account) = &action.account else {
            return Ok(());
        };
        let Some(tickets) = self
            .inner
            .tickets
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        else {
            return Ok(());
        };
        if action.revoke_tickets {
            tickets.revoke(account).await?;
        } else if matches!(action.operation.as_str(), operations::UNBAN | operations::LIFT_TIMEOUT) {
            tickets.allow(account).await?;
        }
        Ok(())
    }

    /// Handles one decoded [`RelayAction`] from `from` (authenticated by the link).
    pub async fn handle(&self, from: &NodeId, action: RelayAction) -> AdminRelayReply {
        if !self.is_peer(from) {
            return AdminRelayReply::Refused {
                refusal: AdminRelayRefusal::UnknownPeer,
            };
        }
        let Ok(permit) = self.inner.permits.clone().try_acquire_owned() else {
            return AdminRelayReply::Refused {
                refusal: AdminRelayRefusal::Busy,
            };
        };
        let id = (from.clone(), action.key.clone());
        let digest = digest_of(&action);
        let begin = self.inner.dedup.lock().unwrap_or_else(PoisonError::into_inner).begin(
            &id,
            digest,
            self.inner.config.call_timeout * 5,
        );
        match begin {
            Begin::Done(body) => {
                // Re-apply ticket revocation idempotently in case tickets were rotated or re-checked
                if let Err(error) = self.apply_ticket_effect(&action).await {
                    return AdminRelayReply::Refused {
                        refusal: AdminRelayRefusal::Failed {
                            code: Some(CleanText::lossy("STATE_NOT_SAVED")),
                            message: CleanText::lossy(&error.to_string()),
                        },
                    };
                }
                return match HomeBody::new(body) {
                    Ok(body) => AdminRelayReply::Applied { body },
                    Err(_) => AdminRelayReply::Refused {
                        refusal: AdminRelayRefusal::TooLarge,
                    },
                };
            }
            Begin::Running => {
                return AdminRelayReply::Refused {
                    refusal: AdminRelayRefusal::InFlight,
                };
            }
            Begin::Reused => {
                return AdminRelayReply::Refused {
                    refusal: AdminRelayRefusal::KeyReused,
                };
            }
            Begin::Fresh => {}
        }

        // Only revoke or allow tickets once the authority confirms the action succeeded
        let tick_action = action.clone();
        let relayed = RelayedAction {
            via_node: from.clone(),
            key: action.key,
            operation: action.operation,
            account: action.account,
            revoke_tickets: action.revoke_tickets,
            actor_key: action.actor_key.into_string(),
            operator: action.operator.into_string(),
            approved_by: action.approved_by.map(CleanText::into_string),
            body: action.body.into_value(),
        };
        let authority = self.inner.authority.clone();
        let kept_in = self.inner.dedup.clone();
        let (entries, cap) = (self.inner.config.dedup_entries, self.inner.config.dedup_bytes);
        let task = tokio::spawn(async move {
            let _permit = permit;
            let result = authority.apply(relayed).await;
            let kept = match &result {
                Ok(body) => Some((body.clone(), digest)),
                Err(_) => None,
            };
            kept_in
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .finish(&id, kept, entries, cap);
            result
        });
        let result = match tokio::time::timeout(self.inner.config.call_timeout, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                tracing::error!(peer = %from, %error, "an admin relay operation panicked");
                return AdminRelayReply::Refused {
                    refusal: AdminRelayRefusal::Failed {
                        code: None,
                        message: CleanText::lossy("admin relay operation failed"),
                    },
                };
            }
            Err(_) => {
                return AdminRelayReply::Refused {
                    refusal: AdminRelayRefusal::InFlight,
                };
            }
        };
        match result {
            Ok(body) => {
                if let Err(error) = self.apply_ticket_effect(&tick_action).await {
                    return AdminRelayReply::Refused {
                        refusal: AdminRelayRefusal::Failed {
                            code: Some(CleanText::lossy("STATE_NOT_SAVED")),
                            message: CleanText::lossy(&error.to_string()),
                        },
                    };
                }
                match HomeBody::new(body) {
                    Ok(body) => AdminRelayReply::Applied { body },
                    Err(_) => AdminRelayReply::Refused {
                        refusal: AdminRelayRefusal::TooLarge,
                    },
                }
            }
            Err(error) => AdminRelayReply::Refused {
                refusal: AdminRelayRefusal::Failed {
                    code: error.code.as_deref().map(CleanText::lossy),
                    message: CleanText::lossy(&error.message),
                },
            },
        }
    }
}

impl NodeService for AdminRelayService {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Admin {
                return Err(LinkError::NoService {
                    node: self.inner.me.clone(),
                    service,
                });
            }
            let reply = match decode_request(&body) {
                Ok(request) => self.handle(from, request.action).await,
                Err(DecodeError::Version(_)) => AdminRelayReply::Refused {
                    refusal: AdminRelayRefusal::UnsupportedVersion {
                        supported: ADMIN_RELAY_VERSION,
                    },
                },
                Err(DecodeError::TooLarge) => AdminRelayReply::Refused {
                    refusal: AdminRelayRefusal::TooLarge,
                },
                Err(_) => AdminRelayReply::Refused {
                    refusal: AdminRelayRefusal::Malformed,
                },
            };
            encode_response(&AdminRelayResponse::new(reply)).map_err(|_| LinkError::TooLarge)
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::atomic::{AtomicUsize, Ordering};

    use module_api::NodeIdentity;
    use module_api::network::IdKind;
    use serde_json::json;

    use super::*;
    use crate::home::{Expect, SigningKey, TicketConfig, TicketError};
    use crate::link::Clock;

    struct RecordingAuthority {
        calls: Mutex<Vec<RelayedAction>>,
        count: AtomicUsize,
    }

    impl RecordingAuthority {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                count: AtomicUsize::new(0),
            }
        }
    }

    impl AdminAuthority for RecordingAuthority {
        fn apply<'a>(&'a self, action: RelayedAction) -> BoxFuture<'a, Result<Value, AdminRelayError>> {
            Box::pin(async move {
                let n = self.count.fetch_add(1, Ordering::SeqCst) + 1;
                self.calls.lock().unwrap().push(action);
                Ok(json!({ "applied": n }))
            })
        }
    }

    fn sample_action(account: Option<GlobalId>, key: &str, reason: &str) -> RelayAction {
        RelayAction {
            key: RequestKey::new(key).unwrap(),
            operation: operations::BAN.parse().unwrap(),
            account,
            revoke_tickets: true,
            actor_key: CleanText::new("ab12cd34").unwrap(),
            operator: CleanText::new("discord:99").unwrap(),
            approved_by: Some(CleanText::new("owner").unwrap()),
            body: HomeBody::new(json!({ "player": "Fox", "reason": reason })).unwrap(),
        }
    }

    #[test]
    fn wire_round_trip_and_secret_key_refusal() {
        let oregon: NodeId = "oregon-1".parse().unwrap();
        let fox = GlobalId::new(IdKind::Account, &oregon);
        let action = sample_action(Some(fox), "req-key-0001", "griefing hunts");
        let bytes = encode_request(&AdminRelayRequest::new(action.clone())).unwrap();
        let decoded = decode_request(&bytes).unwrap();
        assert_eq!(decoded.action, action);

        // Neither an account key (RRK_) nor an admin secret key (RRA_) may cross a node link
        for secret in [
            "RRK_0123456789abcdef0123456789abcdef",
            "RRA_ab12cd34_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ] {
            let raw = serde_json::to_vec(&json!({
                "version": ADMIN_RELAY_VERSION,
                "action": {
                    "key": "req-key-0001",
                    "operation": "admin.ban",
                    "revoke_tickets": true,
                    "actor_key": secret,
                    "operator": "host",
                    "body": {}
                }
            }))
            .unwrap();
            assert!(decode_request(&raw).is_err());
            let raw_body = serde_json::to_vec(&json!({
                "version": ADMIN_RELAY_VERSION,
                "action": {
                    "key": "req-key-0001",
                    "operation": "admin.ban",
                    "revoke_tickets": true,
                    "actor_key": "ab12cd34",
                    "operator": "host",
                    "body": { "leaked": secret }
                }
            }))
            .unwrap();
            assert!(decode_request(&raw_body).is_err());
        }
    }

    #[tokio::test]
    async fn service_verifies_peer_deduplicates_and_revokes_tickets() {
        let clock = Arc::new(Clock::default());
        let germany_id = NodeIdentity {
            node: "germany-1".parse().unwrap(),
            region: "germany".parse().unwrap(),
        };
        let oregon: NodeId = "oregon-1".parse().unwrap();
        let stranger: NodeId = "stranger-1".parse().unwrap();
        let game = "1.14.7".parse().unwrap();
        let tickets = Arc::new(Tickets::new(
            &germany_id,
            SigningKey::generate().unwrap(),
            clock,
            TicketConfig::default(),
        ));
        let fox = GlobalId::new(IdKind::Account, &germany_id.node);
        let ticket = tickets.issue(&fox, &oregon, &game, None).unwrap();
        let expect = Expect {
            issuer: &germany_id.node,
            audience: &oregon,
            game: Some(&game),
            account: Some(&fox),
        };
        assert!(tickets.verify(&ticket, &expect).is_ok());

        let authority = Arc::new(RecordingAuthority::new());
        let service = AdminRelayService::new(
            germany_id.node.clone(),
            [oregon.clone()],
            Some(tickets.clone()),
            authority.clone(),
            AdminRelayConfig::default(),
        );

        let action = sample_action(Some(fox.clone()), "ban-fox-0001", "cheating");
        // An unlisted node is refused and changes nothing
        assert_eq!(
            service.handle(&stranger, action.clone()).await,
            AdminRelayReply::Refused {
                refusal: AdminRelayRefusal::UnknownPeer,
            }
        );
        assert_eq!(authority.count.load(Ordering::SeqCst), 0);
        assert!(tickets.verify(&ticket, &expect).is_ok());

        // Listed peer applies once and revokes the player's tickets
        let first = service.handle(&oregon, action.clone()).await;
        assert_eq!(
            first,
            AdminRelayReply::Applied {
                body: HomeBody::new(json!({ "applied": 1 })).unwrap(),
            }
        );
        assert_eq!(authority.count.load(Ordering::SeqCst), 1);
        assert_eq!(tickets.verify(&ticket, &expect), Err(TicketError::Revoked));

        // Replaying the same key returns the cached reply without running a second time
        let second = service.handle(&oregon, action).await;
        assert_eq!(second, first);
        assert_eq!(authority.count.load(Ordering::SeqCst), 1);

        // Reusing the same key for a different payload is refused
        let different = sample_action(Some(fox), "ban-fox-0001", "different reason");
        assert_eq!(
            service.handle(&oregon, different).await,
            AdminRelayReply::Refused {
                refusal: AdminRelayRefusal::KeyReused,
            }
        );
    }
}
