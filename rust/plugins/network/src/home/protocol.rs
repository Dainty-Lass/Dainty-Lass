//! What nodes say to each other about accounts (`Service::Home`, DN5): JSON, versioned, every field known and
//! bounded. A node asks an account's home for the home's public keys, to check or exchange a session ticket,
//! or to run a module's operation on the account.
//!
//! **The account key never crosses a node link, and no field here can hold it.** Every typed field is a
//! name, id or number that is checked when it is read; the free-form parts are [`HomeBody`] (a module's
//! operation and answer), [`WireActor`], [`RequestKey`] and [`CleanText`], whose constructors and
//! deserialisers refuse anything shaped like an `RRK_` key; and a ticket's token is a base64url string of
//! bounded length whose claims are typed (`ticket.rs`). `decode_*` and `encode_*` run [`validate`] over what
//! they carry, so a message neither leaves this node nor is believed with one in it.

use module_api::network::{GlobalId, NodeId, OperationName, SessionTicket};
use module_api::{ActorContext, ErrorKind, ModuleId};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::keys::VerifyingKey;
use super::ticket::{MAX_TOKEN, TicketError};
use crate::link::MAX_MESSAGE;

/// The home protocol's version; a request with another is refused (DD16: versioned so federation can come).
pub const HOME_VERSION: u32 = 1;

/// The most bytes an operation's body, or the module's answer, may hold once written as JSON.
pub const MAX_BODY: usize = 256 * 1024;

/// The longest message a refusal carries from a module.
pub const MAX_TEXT: usize = 512;

pub const MIN_KEY: usize = 8;
pub const MAX_KEY: usize = 64;

/// Whether `text` holds something shaped like an account key (`RRK_` and at least eight more of the characters
/// a key is written in, in any case). Looser than a real key on purpose: a false alarm costs a refused
/// message, a miss a leaked key.
pub fn looks_like_account_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at + 4 <= bytes.len() {
        if bytes[at..at + 4].eq_ignore_ascii_case(b"rrk_") {
            let tail = bytes[at + 4..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_' || **b == b'-')
                .count();
            if tail >= 8 {
                return true;
            }
        }
        at += 1;
    }
    false
}

/// Whether any string in `value`, or any key of an object in it, looks like an account key.
pub fn holds_account_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => looks_like_account_key(text),
        serde_json::Value::Array(items) => items.iter().any(holds_account_key),
        serde_json::Value::Object(map) => map
            .iter()
            .any(|(name, item)| looks_like_account_key(name) || holds_account_key(item)),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FieldError {
    #[error("over {MAX_BODY} bytes")]
    TooLarge,
    #[error("holds something shaped like an account key")]
    AccountKey,
    #[error("not what this field takes")]
    Invalid,
}

/// A module's operation body or answer: any JSON, at most [`MAX_BODY`] bytes, with no account key in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeBody(serde_json::Value);

impl HomeBody {
    pub fn new(value: serde_json::Value) -> Result<Self, FieldError> {
        if holds_account_key(&value) {
            return Err(FieldError::AccountKey);
        }
        let size = serde_json::to_vec(&value).map_or(usize::MAX, |bytes| bytes.len());
        if size > MAX_BODY {
            return Err(FieldError::TooLarge);
        }
        Ok(Self(value))
    }

    pub fn null() -> Self {
        Self(serde_json::Value::Null)
    }

    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }

    pub fn into_value(self) -> serde_json::Value {
        self.0
    }
}

impl Serialize for HomeBody {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HomeBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Who asked at the calling node, as its module's [`ActorContext`] says, for the home's change log (DD23). It
/// is the caller's claim: the log also records which node made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ActorContext", into = "ActorContext")]
pub struct WireActor(ActorContext);

impl WireActor {
    pub fn new(actor: ActorContext) -> Result<Self, FieldError> {
        if !actor.is_valid() {
            return Err(FieldError::Invalid);
        }
        let text = serde_json::to_string(&actor).map_err(|_| FieldError::Invalid)?;
        if looks_like_account_key(&text) {
            return Err(FieldError::AccountKey);
        }
        Ok(Self(actor))
    }

    pub fn actor(&self) -> &ActorContext {
        &self.0
    }
}

impl TryFrom<ActorContext> for WireActor {
    type Error = FieldError;

    fn try_from(actor: ActorContext) -> Result<Self, FieldError> {
        Self::new(actor)
    }
}

impl From<WireActor> for ActorContext {
    fn from(actor: WireActor) -> Self {
        actor.0
    }
}

/// Names one request, so the home runs it once however often it is sent: 8 to 64 characters of `A-Za-z0-9_-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RequestKey(String);

impl RequestKey {
    pub fn new(text: impl Into<String>) -> Result<Self, FieldError> {
        let text = text.into();
        let ok = (MIN_KEY..=MAX_KEY).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !ok {
            return Err(FieldError::Invalid);
        }
        if looks_like_account_key(&text) {
            return Err(FieldError::AccountKey);
        }
        Ok(Self(text))
    }

    pub fn generate() -> Result<Self, FieldError> {
        let text = super::keys::random_hex::<16>().map_err(|_| FieldError::Invalid)?;
        Self::new(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RequestKey {
    type Error = FieldError;

    fn try_from(text: String) -> Result<Self, FieldError> {
        Self::new(text)
    }
}

impl From<RequestKey> for String {
    fn from(key: RequestKey) -> String {
        key.0
    }
}

/// A short line of free text a module's refusal carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CleanText(String);

impl CleanText {
    pub fn new(text: impl Into<String>) -> Result<Self, FieldError> {
        let text = text.into();
        if text.len() > MAX_TEXT || text.chars().any(char::is_control) {
            return Err(FieldError::Invalid);
        }
        if looks_like_account_key(&text) {
            return Err(FieldError::AccountKey);
        }
        Ok(Self(text))
    }

    /// `text` cut to fit, or a stand-in if it holds anything a field refuses: the sending side's way to say
    /// what failed without ever failing to say it.
    pub fn lossy(text: &str) -> Self {
        let cut: String = text.chars().filter(|c| !c.is_control()).take(MAX_TEXT / 4).collect();
        Self::new(cut).unwrap_or_else(|_| Self(String::from("the module failed")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CleanText {
    type Error = FieldError;

    fn try_from(text: String) -> Result<Self, FieldError> {
        Self::new(text)
    }
}

impl From<CleanText> for String {
    fn from(text: CleanText) -> String {
        text.0
    }
}

/// A module's operation to run on an account at its home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeCall {
    /// A ticket the account's home made for the calling node and the module's game
    pub ticket: SessionTicket,
    /// The module whose backend runs it at the home
    pub module: ModuleId,
    pub operation: OperationName,
    /// Must be the ticket's account
    pub account: GlobalId,
    pub actor: WireActor,
    pub key: RequestKey,
    pub body: HomeBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeRequestCall {
    /// The home's public keys, for checking tickets it made
    PublicKeys,
    /// Does the home still stand behind this ticket
    Check {
        ticket: SessionTicket,
    },
    /// A ticket for `audience` in return for a ticket the home made for the caller: how a node hands a player
    /// on, and with the caller as audience how it renews its own
    Exchange {
        ticket: SessionTicket,
        audience: NodeId,
    },
    Run(Box<HomeCall>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeRequest {
    pub version: u32,
    pub call: HomeRequestCall,
}

/// What the home says of a session it stands behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInfo {
    pub account: GlobalId,
    /// When the ticket checked expires, in unix ms
    pub expires_at: i64,
    /// When the login began, in unix ms
    pub since: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeRefusal {
    #[error("ticket refused: {0}")]
    Ticket(TicketError),
    /// The account isn't homed on the node asked
    #[error("not the account's home")]
    NotHome,
    #[error("the home runs no such module")]
    NoModule,
    /// The request disagrees with itself (an account that isn't the ticket's) or names an unknown node
    #[error("not a request the home can answer")]
    BadRequest,
    /// A request with this key is running; ask again for its answer
    #[error("that request is still running")]
    InFlight,
    #[error("the home is busy")]
    Busy,
    /// The module ran it and it failed
    #[error("the module failed: {message:?}")]
    Failed { kind: ErrorKind, message: CleanText },
    #[error("the answer was too large")]
    TooLarge,
    #[error("home protocol {supported} only")]
    UnsupportedVersion { supported: u32 },
    #[error("not a home message")]
    Malformed,
    #[error("the home can't answer just now")]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeReply {
    PublicKeys { node: NodeId, keys: Vec<VerifyingKey> },
    Session(SessionInfo),
    Ticket(SessionTicket),
    Done { body: HomeBody },
    Refused(HomeRefusal),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeResponse {
    pub version: u32,
    pub reply: HomeReply,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("a node-link message is over {MAX_MESSAGE} bytes")]
    TooLarge,
    #[error("home protocol {0}")]
    Version(u32),
    #[error("{0}")]
    Json(String),
    #[error("{0}")]
    Field(String),
}

fn ticket_ok(ticket: &SessionTicket) -> Result<(), DecodeError> {
    let ok = ticket.token.len() <= MAX_TOKEN
        && ticket
            .token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(DecodeError::Field(String::from("not a session ticket")))
    }
}

/// What the types alone don't settle: tickets are bounded base64url text, a keys list is no longer than a node
/// has keys, and a call's account is its ticket's. Run by `encode_*` and `decode_*`.
pub trait Validate {
    fn validate(&self) -> Result<(), DecodeError>;
}

impl Validate for HomeRequest {
    fn validate(&self) -> Result<(), DecodeError> {
        match &self.call {
            HomeRequestCall::PublicKeys => Ok(()),
            HomeRequestCall::Check { ticket } | HomeRequestCall::Exchange { ticket, .. } => ticket_ok(ticket),
            HomeRequestCall::Run(call) => ticket_ok(&call.ticket),
        }
    }
}

impl Validate for HomeResponse {
    fn validate(&self) -> Result<(), DecodeError> {
        match &self.reply {
            HomeReply::Ticket(ticket) => ticket_ok(ticket),
            HomeReply::PublicKeys { keys, .. } if keys.is_empty() || keys.len() > super::keys::MAX_KEYS_PER_NODE => {
                Err(DecodeError::Field(String::from("a node has one to four keys")))
            }
            _ => Ok(()),
        }
    }
}

fn decode<T: DeserializeOwned + Validate>(bytes: &[u8], version_of: impl Fn(&T) -> u32) -> Result<T, DecodeError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(DecodeError::TooLarge);
    }
    #[derive(Deserialize)]
    struct Versioned {
        version: u32,
    }
    match serde_json::from_slice::<T>(bytes) {
        Ok(message) if version_of(&message) == HOME_VERSION => {
            message.validate()?;
            Ok(message)
        }
        Ok(message) => Err(DecodeError::Version(version_of(&message))),
        // A newer peer's message may not parse as this version's: say it's the version, not the shape
        Err(error) => match serde_json::from_slice::<Versioned>(bytes) {
            Ok(Versioned { version }) if version != HOME_VERSION => Err(DecodeError::Version(version)),
            _ => Err(DecodeError::Json(error.to_string())),
        },
    }
}

pub fn decode_request(bytes: &[u8]) -> Result<HomeRequest, DecodeError> {
    decode(bytes, |r: &HomeRequest| r.version)
}

pub fn decode_response(bytes: &[u8]) -> Result<HomeResponse, DecodeError> {
    decode(bytes, |r: &HomeResponse| r.version)
}

fn encode<T: Serialize + Validate>(message: &T) -> Result<Vec<u8>, DecodeError> {
    message.validate()?;
    let bytes = serde_json::to_vec(message).map_err(|e| DecodeError::Json(e.to_string()))?;
    if bytes.len() > MAX_MESSAGE {
        return Err(DecodeError::TooLarge);
    }
    Ok(bytes)
}

pub fn encode_request(message: &HomeRequest) -> Result<Vec<u8>, DecodeError> {
    encode(message)
}

pub fn encode_response(message: &HomeResponse) -> Result<Vec<u8>, DecodeError> {
    encode(message)
}

impl HomeRequest {
    pub fn new(call: HomeRequestCall) -> Self {
        Self {
            version: HOME_VERSION,
            call,
        }
    }
}

impl HomeResponse {
    pub fn new(reply: HomeReply) -> Self {
        Self {
            version: HOME_VERSION,
            reply,
        }
    }

    pub fn refused(refusal: HomeRefusal) -> Self {
        Self::new(HomeReply::Refused(refusal))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use module_api::Actor;

    use super::*;
    use crate::home::keys::SigningKey;

    fn account() -> GlobalId {
        "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@germany-1".parse().unwrap()
    }

    fn ticket() -> SessionTicket {
        SessionTicket {
            account: account(),
            audience: "oregon-1".parse().unwrap(),
            token: String::from("dlt1.e30.c2ln"),
        }
    }

    fn call() -> HomeCall {
        HomeCall {
            ticket: ticket(),
            module: "rustic-isles".parse().unwrap(),
            operation: "inventory.write".parse().unwrap(),
            account: account(),
            actor: WireActor::new(ActorContext::new(Actor::Player { account: account() })).unwrap(),
            key: RequestKey::new("0123456789abcdef").unwrap(),
            body: HomeBody::new(serde_json::json!({"items": [{"id": 7, "count": 2}]})).unwrap(),
        }
    }

    /// One of every message a node can send or answer with.
    fn every_message() -> (Vec<HomeRequest>, Vec<HomeResponse>) {
        let key = SigningKey::generate().unwrap();
        let requests = vec![
            HomeRequest::new(HomeRequestCall::PublicKeys),
            HomeRequest::new(HomeRequestCall::Check { ticket: ticket() }),
            HomeRequest::new(HomeRequestCall::Exchange {
                ticket: ticket(),
                audience: "uk-1".parse().unwrap(),
            }),
            HomeRequest::new(HomeRequestCall::Run(Box::new(call()))),
        ];
        let mut responses = vec![
            HomeResponse::new(HomeReply::PublicKeys {
                node: "germany-1".parse().unwrap(),
                keys: vec![key.verifying_key().clone()],
            }),
            HomeResponse::new(HomeReply::Session(SessionInfo {
                account: account(),
                expires_at: 1_790_000_300_000,
                since: 1_790_000_000_000,
            })),
            HomeResponse::new(HomeReply::Ticket(ticket())),
            HomeResponse::new(HomeReply::Done {
                body: HomeBody::new(serde_json::json!({"ok": true})).unwrap(),
            }),
        ];
        let refusals = [
            HomeRefusal::Ticket(TicketError::Expired),
            HomeRefusal::NotHome,
            HomeRefusal::NoModule,
            HomeRefusal::BadRequest,
            HomeRefusal::InFlight,
            HomeRefusal::Busy,
            HomeRefusal::Failed {
                kind: ErrorKind::Failed,
                message: CleanText::new("no such item").unwrap(),
            },
            HomeRefusal::TooLarge,
            HomeRefusal::UnsupportedVersion { supported: 1 },
            HomeRefusal::Malformed,
            HomeRefusal::Unavailable,
        ];
        responses.extend(refusals.into_iter().map(HomeResponse::refused));
        (requests, responses)
    }

    #[test]
    fn every_message_round_trips_and_has_one_spelling() {
        let (requests, responses) = every_message();
        for request in requests {
            let bytes = encode_request(&request).unwrap();
            assert_eq!(decode_request(&bytes).unwrap(), request);
            assert_eq!(encode_request(&decode_request(&bytes).unwrap()).unwrap(), bytes);
        }
        for response in responses {
            let bytes = encode_response(&response).unwrap();
            assert_eq!(decode_response(&bytes).unwrap(), response);
            assert_eq!(encode_response(&decode_response(&bytes).unwrap()).unwrap(), bytes);
        }
        let json =
            String::from_utf8(encode_request(&HomeRequest::new(HomeRequestCall::Check { ticket: ticket() })).unwrap())
                .unwrap();
        assert_eq!(
            json,
            r#"{"version":1,"call":{"check":{"ticket":{"account":"account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@germany-1","audience":"oregon-1","token":"dlt1.e30.c2ln"}}}}"#
        );
    }

    #[test]
    fn no_message_type_can_carry_an_account_key() {
        let key = format!("RRK_{}", "ab".repeat(24));
        // Every message, written out: nothing in them looks like a key
        let (requests, responses) = every_message();
        for request in &requests {
            assert!(!looks_like_account_key(&serde_json::to_string(request).unwrap()));
        }
        for response in &responses {
            assert!(!looks_like_account_key(&serde_json::to_string(response).unwrap()));
        }
        // Each free-form field refuses one, spelled any way
        for spelled in [
            key.clone(),
            key.to_lowercase(),
            format!("x {key} y"),
            format!("RRK_{}", "Z".repeat(8)),
        ] {
            assert_eq!(
                HomeBody::new(serde_json::json!(spelled.clone())),
                Err(FieldError::AccountKey)
            );
            assert_eq!(
                HomeBody::new(serde_json::json!({"a": [spelled.clone()]})),
                Err(FieldError::AccountKey)
            );
            assert_eq!(
                HomeBody::new(serde_json::json!({ spelled.clone(): 1 })),
                Err(FieldError::AccountKey)
            );
            assert!(RequestKey::new(spelled.clone()).is_err());
            assert!(CleanText::new(spelled.clone()).is_err());
            let actor = ActorContext::new(Actor::Admin { key: spelled.clone() });
            assert!(WireActor::new(actor).is_err());
            let request = ActorContext::new(Actor::System).with_request(spelled.clone());
            assert!(WireActor::new(request).is_err());
        }
        // ...and so does reading one off the wire, not only building one
        let mut message = serde_json::to_value(HomeRequest::new(HomeRequestCall::Run(Box::new(call())))).unwrap();
        message["call"]["run"]["body"] = serde_json::json!({ "note": key });
        let bytes = serde_json::to_vec(&message).unwrap();
        assert!(matches!(decode_request(&bytes), Err(DecodeError::Json(_))));
        let mut message = serde_json::to_value(HomeResponse::new(HomeReply::Done { body: HomeBody::null() })).unwrap();
        message["reply"]["done"]["body"] = serde_json::json!([key]);
        assert!(matches!(
            decode_response(&serde_json::to_vec(&message).unwrap()),
            Err(DecodeError::Json(_))
        ));
        // Fields with a fixed shape have nowhere to put one: ids, names and the ticket are checked as read
        for text in [format!("account:{key}@germany-1"), key.clone()] {
            assert!(text.parse::<GlobalId>().is_err());
        }
        assert!(key.parse::<NodeId>().is_err() && key.parse::<ModuleId>().is_err());
        assert!(key.parse::<OperationName>().is_err());
        let bad_ticket = SessionTicket {
            token: format!("dlt1.{key}.{key} "),
            ..ticket()
        };
        assert!(matches!(
            encode_request(&HomeRequest::new(HomeRequestCall::Check { ticket: bad_ticket })),
            Err(DecodeError::Field(_))
        ));
        // The sender's own constructor for a refusal never fails on a module's text
        assert_eq!(
            CleanText::lossy(&format!("failed for {key}")).as_str(),
            "the module failed"
        );
        assert_eq!(CleanText::lossy("bad\nnews").as_str(), "badnews");
    }

    #[test]
    fn unknown_fields_versions_and_sizes_are_refused() {
        assert_eq!(
            decode_request(br#"{"version":2,"call":"public_keys"}"#),
            Err(DecodeError::Version(2))
        );
        assert_eq!(
            decode_request(br#"{"version":2,"call":{"future":{}}}"#),
            Err(DecodeError::Version(2))
        );
        assert!(matches!(
            decode_request(br#"{"version":1,"call":"public_keys","extra":1}"#),
            Err(DecodeError::Json(_))
        ));
        assert!(matches!(
            decode_request(br#"{"version":1,"call":{"check":{"ticket":{"account":"account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@a","audience":"b","token":"t","extra":1}}}}"#),
            Err(DecodeError::Json(_))
        ));
        assert!(matches!(decode_request(b"{}"), Err(DecodeError::Json(_))));
        assert!(matches!(decode_request(b"\xff\xfe"), Err(DecodeError::Json(_))));
        let big = vec![b' '; MAX_MESSAGE + 1];
        assert_eq!(decode_request(&big), Err(DecodeError::TooLarge));
        assert_eq!(decode_response(&big), Err(DecodeError::TooLarge));
        // A body over its own bound is refused although the message fits
        let body = serde_json::json!({"blob": "x".repeat(MAX_BODY)});
        assert_eq!(HomeBody::new(body), Err(FieldError::TooLarge));
        // Names and keys are checked as they are read
        for field in [
            ("operation", "Inventory.Write"),
            ("module", "Rustic_Isles"),
            ("key", "short"),
        ] {
            let mut message = serde_json::to_value(HomeRequest::new(HomeRequestCall::Run(Box::new(call())))).unwrap();
            message["call"]["run"][field.0] = serde_json::json!(field.1);
            assert!(matches!(
                decode_request(&serde_json::to_vec(&message).unwrap()),
                Err(DecodeError::Json(_))
            ));
        }
        // Too many keys
        let keys: Vec<_> = (0..5)
            .map(|_| SigningKey::generate().unwrap().verifying_key().clone())
            .collect();
        let long = HomeResponse::new(HomeReply::PublicKeys {
            node: "germany-1".parse().unwrap(),
            keys,
        });
        assert!(matches!(encode_response(&long), Err(DecodeError::Field(_))));
    }
}
