//! Session tickets (DD17, DN5): what an account's home signs so another node can act for a logged-in player,
//! and how the node the ticket is for checks one.
//!
//! A ticket is `dlt1.<claims>.<signature>`, both parts base64url without padding. The claims are JSON the
//! home wrote once and the signature covers those exact bytes, so nothing is ever re-serialised before it is
//! checked. They say who issued it (`iss`, the account's home), for which node (`aud`), for whom (`sub`, the
//! account's global id), for which game (`game`), when (`iat`, `exp`, in unix ms), when the login it belongs
//! to began (`since`, kept by every renewal, so a session has an age limit) and a random id (`jti`).
//! Ed25519 over `dainty-lass/session-ticket/v1\n` and the claims.
//!
//! A ticket carries no secret: it is a signed statement that a player is logged in at their home. The
//! account key never goes into one, and `Tickets`' `Debug` shows no key.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dainty_module_api::network::{GlobalId, IdKind, NodeId, SessionTicket};
use dainty_module_api::{GameVersion, NodeIdentity};
use serde::{Deserialize, Serialize};

use super::keys::{KeyRing, MAX_KEYS_PER_NODE, SIGNATURE_LEN, SigningKey, VerifyingKey, random_hex};
use crate::link::{Clock, MAX_CLOCK_SKEW};

/// The `v` every ticket's claims carry; another is malformed.
pub const TICKET_VERSION: u32 = 1;
const TOKEN_PREFIX: &str = "dlt1";
const SIGN_CONTEXT: &[u8] = b"dainty-lass/session-ticket/v1\n";
/// The longest token read; a real one is about 700 bytes at its biggest.
pub const MAX_TOKEN: usize = 1024;
/// How long a ticket lives unless the home is told otherwise.
pub const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);
/// The longest life a ticket may claim, whatever its issuer's settings: a verifier refuses more.
pub const MAX_TTL: Duration = Duration::from_secs(15 * 60);
/// How long a login may be renewed for, counted from when it began.
pub const DEFAULT_MAX_SESSION: Duration = Duration::from_secs(6 * 3600);
/// Tickets a node remembers having admitted; past it, it refuses new ones rather than forget (fail closed).
pub const MAX_REPLAY_ENTRIES: usize = 65_536;

fn ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// Why a ticket was refused. Crosses the node link, so the home can tell the node that asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum TicketError {
    /// The token does not parse as a `dlt1` ticket.
    #[error("not a session ticket")]
    Malformed,
    /// Signed with a key this node does not hold for the issuer.
    #[error("signed with a key this node does not know")]
    UnknownKey,
    /// The signature does not verify.
    #[error("the signature is not the issuer's")]
    BadSignature,
    /// Not issued by the account's home.
    #[error("not issued by the account's home")]
    WrongIssuer,
    /// Made for another node.
    #[error("made for another node")]
    WrongAudience,
    /// Made for another game version.
    #[error("made for another game version")]
    WrongGame,
    /// Made for another account.
    #[error("made for another account")]
    WrongAccount,
    /// Issued later than this node's clock allows.
    #[error("issued in the future")]
    NotYetValid,
    /// Its `exp` has passed.
    #[error("expired")]
    Expired,
    /// Its lifetime is over [`MAX_TTL`].
    #[error("claims a longer life than a ticket may have")]
    TooLong,
    /// This node already admitted it.
    #[error("already used")]
    Replayed,
    /// Issued before this node started, so it can't tell whether it was used already
    #[error("issued before this node started; ask for a new one")]
    BeforeStart,
    /// The replay cache holds [`MAX_REPLAY_ENTRIES`]: refused rather than forgotten.
    #[error("this node cannot remember more tickets just now")]
    ReplayCacheFull,
    /// The home ended the session.
    #[error("the session was ended at its home")]
    Revoked,
    /// The login began longer ago than renewals allow.
    #[error("the login is older than a session may be renewed for")]
    SessionTooOld,
    /// The home cannot issue a ticket just now.
    #[error("this node cannot make a ticket just now")]
    Unavailable,
}

/// Persisting a ticket revocation or unban floor in `network_ticket_revocations` failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not persist ticket revocation for {account}: {reason}")]
pub struct RevocationPersistError {
    /// The account whose revocation was not saved.
    pub account: GlobalId,
    /// The store's error.
    pub reason: String,
}

/// What a ticket says, once its signature has been checked (`Tickets::verify`) or not yet (`Tickets::peek`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claims {
    /// [`TICKET_VERSION`].
    pub v: u32,
    /// The id of the key that signed it
    pub kid: String,
    /// The issuing node: the account's home.
    pub iss: NodeId,
    /// The node the ticket is for.
    pub aud: NodeId,
    /// The account it acts for.
    pub sub: GlobalId,
    /// The game version it is for.
    pub game: GameVersion,
    /// Issued at, unix milliseconds.
    pub iat: i64,
    /// Expires at, unix milliseconds.
    pub exp: i64,
    /// When the login this ticket belongs to began: the first ticket's `iat`, kept by renewals
    pub since: i64,
    /// A random 32-hex-character id, for replay checks.
    pub jti: String,
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Claims {
    fn well_formed(&self) -> bool {
        self.v == TICKET_VERSION
            && is_hex(&self.kid, 16)
            && is_hex(&self.jti, 32)
            && self.sub.kind() == IdKind::Account
            && self.iat >= 0
            && self.since >= 0
    }
}

/// The wire form, checked to be the only spelling of itself: the payload and signature decode and re-encode to
/// the same text, so a ticket can't be made to look different to two readers.
fn parse_token(token: &str) -> Result<(Claims, Vec<u8>, [u8; SIGNATURE_LEN]), TicketError> {
    if token.len() > MAX_TOKEN {
        return Err(TicketError::Malformed);
    }
    let mut parts = token.split('.');
    let (Some(prefix), Some(payload), Some(signature), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(TicketError::Malformed);
    };
    if prefix != TOKEN_PREFIX {
        return Err(TicketError::Malformed);
    }
    let payload_bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| TicketError::Malformed)?;
    let signature_bytes = URL_SAFE_NO_PAD.decode(signature).map_err(|_| TicketError::Malformed)?;
    let signature: [u8; SIGNATURE_LEN] = signature_bytes.try_into().map_err(|_| TicketError::Malformed)?;
    let claims: Claims = serde_json::from_slice(&payload_bytes).map_err(|_| TicketError::Malformed)?;
    if !claims.well_formed() {
        return Err(TicketError::Malformed);
    }
    Ok((claims, payload_bytes, signature))
}

/// Parses a wire token (`dlt1.<claims>.<signature>`) into a [`SessionTicket`] and the [`GameVersion`] its claims
/// name, without checking the signature yet (trusted only once passed to [`Tickets::verify`], [`Tickets::redeem`],
/// or [`dainty_module_api::network::HomeApi::admit`]).
pub fn session_ticket_from_token(token: &str) -> Result<(SessionTicket, GameVersion), TicketError> {
    let (claims, _, _) = parse_token(token)?;
    Ok((
        SessionTicket {
            account: claims.sub,
            audience: claims.aud,
            token: token.to_owned(),
        },
        claims.game,
    ))
}

fn signed_message(payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SIGN_CONTEXT.len() + payload.len());
    message.extend_from_slice(SIGN_CONTEXT);
    message.extend_from_slice(payload);
    message
}

/// What a verifier requires of a ticket beyond its signature.
#[derive(Debug, Clone, Copy)]
pub struct Expect<'a> {
    /// The node that may have issued it: the account's home
    pub issuer: &'a NodeId,
    /// The node checking it
    pub audience: &'a NodeId,
    /// The game version the checking module serves; none for a check that doesn't depend on a game
    pub game: Option<&'a GameVersion>,
    /// The account it must be for; none for any.
    pub account: Option<&'a GlobalId>,
}

/// How long tickets live and how far clocks may differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TicketConfig {
    /// How long a new ticket lives, at most [`MAX_TTL`].
    pub ttl: Duration,
    /// How long after a login began it may still be renewed.
    pub max_session: Duration,
    /// How far a peer's clock may differ from this node's.
    pub skew: Duration,
}

impl Default for TicketConfig {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_TTL,
            max_session: DEFAULT_MAX_SESSION,
            skew: MAX_CLOCK_SKEW,
        }
    }
}

struct SignerState {
    current: SigningKey,
    /// Keys replaced by a rotation, and when: still told to peers while tickets they signed can be alive
    retired: Vec<(VerifyingKey, i64)>,
}

#[derive(Default)]
struct ReplayCache {
    by_expiry: BTreeSet<(i64, String)>,
    seen: HashSet<String>,
}

impl ReplayCache {
    fn purge(&mut self, before: i64) {
        while let Some((exp, jti)) = self.by_expiry.first().cloned() {
            if exp >= before {
                break;
            }
            self.by_expiry.remove(&(exp, jti.clone()));
            self.seen.remove(&jti);
        }
    }
}

/// One node's tickets: the key it signs with, the public keys of the others, the replay memory of the tickets
/// it admitted and the sessions it ended. Shared by the home service (answering) and the networked home
/// (asking), so a node that is both the home of some accounts and the audience for others uses one.
pub struct Tickets {
    me: NodeId,
    state: RwLock<SignerState>,
    ring: KeyRing,
    clock: Arc<Clock>,
    config: TicketConfig,
    started_ms: i64,
    replay: Mutex<ReplayCache>,
    // Accounts whose sessions ended, and when: a ticket issued at or before that is dead
    revoked: Mutex<HashMap<GlobalId, i64>>,
    #[cfg(feature = "directory")]
    db: RwLock<Option<dainty_store::Database>>,
}

impl std::fmt::Debug for Tickets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tickets")
            .field("node", &self.me)
            .finish_non_exhaustive()
    }
}

impl Tickets {
    /// Signs with `key` and stamps tickets with `clock`; replay checks refuse tickets issued before now.
    pub fn new(me: &NodeIdentity, key: SigningKey, clock: Arc<Clock>, config: TicketConfig) -> Self {
        let tickets = Self {
            me: me.node.clone(),
            ring: KeyRing::default(),
            started_ms: clock.now_ms(),
            clock,
            config,
            state: RwLock::new(SignerState {
                current: key,
                retired: Vec::new(),
            }),
            replay: Mutex::new(ReplayCache::default()),
            revoked: Mutex::new(HashMap::new()),
            #[cfg(feature = "directory")]
            db: RwLock::new(None),
        };
        tickets.publish_own_keys();
        tickets
    }

    /// Builds [`Tickets`] backed by `db` (`network_ticket_revocations`), reloading persisted revocation floors.
    #[cfg(feature = "directory")]
    pub async fn with_database(
        me: &NodeIdentity,
        key: SigningKey,
        clock: Arc<Clock>,
        config: TicketConfig,
        db: dainty_store::Database,
    ) -> Result<Self, dainty_store::Error> {
        let tickets = Self::new(me, key, clock, config);
        tickets.attach_database(db).await?;
        Ok(tickets)
    }

    /// Attaches `db` (`network_ticket_revocations`) and reloads all persisted revocation floors.
    #[cfg(feature = "directory")]
    pub async fn attach_database(&self, db: dainty_store::Database) -> Result<(), dainty_store::Error> {
        let rows: Vec<(String, i64)> = db
            .read::<_, dainty_store::Error, _>(|conn| {
                use diesel::prelude::*;
                use diesel::sql_types::{BigInt, Text};
                #[derive(QueryableByName)]
                struct Row {
                    #[diesel(sql_type = Text)]
                    account: String,
                    #[diesel(sql_type = BigInt)]
                    revoked_at: i64,
                }
                Ok(
                    diesel::sql_query("SELECT account, revoked_at FROM network_ticket_revocations")
                        .load::<Row>(conn)?
                        .into_iter()
                        .map(|r| (r.account, r.revoked_at))
                        .collect(),
                )
            })
            .await?;
        {
            let mut revoked = self.revoked.lock().unwrap_or_else(PoisonError::into_inner);
            for (raw, floor) in rows {
                if let Ok(account) = raw.parse::<GlobalId>()
                    && account.kind() == IdKind::Account
                {
                    revoked
                        .entry(account)
                        .and_modify(|cur| *cur = (*cur).max(floor))
                        .or_insert(floor);
                }
            }
        }
        *self.db.write().unwrap_or_else(PoisonError::into_inner) = Some(db);
        Ok(())
    }

    /// This node.
    pub fn node(&self) -> &NodeId {
        &self.me
    }

    /// Ticket lifetimes and allowed skew.
    pub fn config(&self) -> &TicketConfig {
        &self.config
    }

    /// The clock tickets are stamped and checked with.
    pub fn clock(&self) -> &Arc<Clock> {
        &self.clock
    }

    fn skew_ms(&self) -> i64 {
        ms(self.config.skew)
    }

    fn publish_own_keys(&self) {
        let keys = self.public_keys();
        self.ring.replace(&self.me, keys);
    }

    /// This node's public keys, newest first: what peers fetch.
    pub fn public_keys(&self) -> Vec<VerifyingKey> {
        let now = self.clock.now_ms();
        let keep = ms(MAX_TTL) + self.skew_ms();
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        let mut keys = vec![state.current.verifying_key().clone()];
        keys.extend(
            state
                .retired
                .iter()
                .rev()
                .filter(|(_, at)| now.saturating_sub(*at) <= keep)
                .map(|(key, _)| key.clone()),
        );
        keys.truncate(MAX_KEYS_PER_NODE);
        keys
    }

    /// Signs with `key` from now on. Tickets the old key signed stay valid until they expire.
    pub fn rotate(&self, key: SigningKey) {
        {
            let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
            let old = std::mem::replace(&mut state.current, key);
            state.retired.push((old.verifying_key().clone(), self.clock.now_ms()));
            if state.retired.len() >= MAX_KEYS_PER_NODE {
                state.retired.remove(0);
            }
        }
        self.publish_own_keys();
    }

    /// What `node` told this node its public keys are. Never for this node itself: no peer chooses the key
    /// that decides whether this node's own tickets are real.
    pub fn learn(&self, node: &NodeId, keys: Vec<VerifyingKey>) -> bool {
        node != &self.me && self.ring.replace(node, keys)
    }

    /// Whether any key of `node` is known.
    pub fn knows_keys_of(&self, node: &NodeId) -> bool {
        self.ring.knows(node)
    }

    /// Whether `node`'s key `kid` is known.
    pub fn knows_key(&self, node: &NodeId, kid: &str) -> bool {
        self.ring.find(node, kid).is_some()
    }

    /// A ticket for `audience` to act for `account`, made by this node as its home. `since` is when the login
    /// began (none: now); a login older than the session limit gets nothing more.
    pub fn issue(
        &self,
        account: &GlobalId,
        audience: &NodeId,
        game: &GameVersion,
        since: Option<i64>,
    ) -> Result<SessionTicket, TicketError> {
        if account.kind() != IdKind::Account {
            return Err(TicketError::WrongAccount);
        }
        let now = self.clock.now_ms();
        if since.is_some_and(|login_since| self.is_revoked_at(account, login_since)) {
            return Err(TicketError::Revoked);
        }
        let since = since.unwrap_or(now);
        if now.saturating_sub(since) > ms(self.config.max_session) {
            return Err(TicketError::SessionTooOld);
        }
        let ttl = self.config.ttl.min(MAX_TTL);
        let jti = random_hex::<16>().map_err(|_| TicketError::Unavailable)?;
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        let claims = Claims {
            v: TICKET_VERSION,
            kid: state.current.kid().to_owned(),
            iss: self.me.clone(),
            aud: audience.clone(),
            sub: account.clone(),
            game: game.clone(),
            iat: now,
            exp: now.saturating_add(ms(ttl)),
            since: since.min(now),
            jti,
        };
        let payload = serde_json::to_vec(&claims).map_err(|_| TicketError::Unavailable)?;
        let signature = state.current.sign(&signed_message(&payload));
        let token = format!(
            "{TOKEN_PREFIX}.{}.{}",
            URL_SAFE_NO_PAD.encode(&payload),
            URL_SAFE_NO_PAD.encode(signature)
        );
        Ok(SessionTicket {
            account: account.clone(),
            audience: audience.clone(),
            token,
        })
    }

    /// What a ticket says, with nothing checked but its shape and that its outer fields match its claims: to
    /// find the account's home and the key to look for. Trust nothing in it until `verify`.
    pub fn peek(&self, ticket: &SessionTicket) -> Result<Claims, TicketError> {
        let (claims, _, _) = parse_token(&ticket.token)?;
        if claims.sub != ticket.account || claims.aud != ticket.audience {
            return Err(TicketError::Malformed);
        }
        Ok(claims)
    }

    /// Everything about a ticket except having been seen before: shape, the issuer's signature, issuer,
    /// audience, game, account, and times with the clock skew allowed. A ticket this node issued is also
    /// refused once its session was ended or is older than a session may be.
    pub fn verify(&self, ticket: &SessionTicket, expect: &Expect<'_>) -> Result<Claims, TicketError> {
        let (claims, payload, signature) = parse_token(&ticket.token)?;
        if claims.sub != ticket.account || claims.aud != ticket.audience {
            return Err(TicketError::Malformed);
        }
        let key = self
            .ring
            .find(&claims.iss, &claims.kid)
            .ok_or(TicketError::UnknownKey)?;
        if !key.verify(&signed_message(&payload), &signature) {
            return Err(TicketError::BadSignature);
        }
        // Signed by a key that node told us is its own: now the claims can be believed
        if &claims.iss != expect.issuer {
            return Err(TicketError::WrongIssuer);
        }
        if &claims.aud != expect.audience {
            return Err(TicketError::WrongAudience);
        }
        if expect.game.is_some_and(|game| game != &claims.game) {
            return Err(TicketError::WrongGame);
        }
        if expect.account.is_some_and(|account| account != &claims.sub) {
            return Err(TicketError::WrongAccount);
        }
        let (now, skew) = (self.clock.now_ms(), self.skew_ms());
        if claims.exp <= claims.iat || claims.since > claims.iat || claims.exp - claims.iat > ms(MAX_TTL) {
            return Err(TicketError::TooLong);
        }
        if claims.iat.saturating_sub(skew) > now {
            return Err(TicketError::NotYetValid);
        }
        if now.saturating_sub(skew) >= claims.exp {
            return Err(TicketError::Expired);
        }
        if claims.iss == self.me && claims.iat.saturating_sub(claims.since) > ms(self.config.max_session) {
            return Err(TicketError::SessionTooOld);
        }
        {
            let revoked = self.revoked.lock().unwrap_or_else(PoisonError::into_inner);
            if revoked
                .get(&claims.sub)
                .is_some_and(|floor| claims.iat <= *floor || claims.since <= *floor)
            {
                return Err(TicketError::Revoked);
            }
        }
        Ok(claims)
    }

    /// `verify`, and the ticket is used up: a second `redeem` of it is `Replayed`, and so is one this node
    /// started too late to tell about. This is what admits a player.
    pub fn redeem(&self, ticket: &SessionTicket, expect: &Expect<'_>) -> Result<Claims, TicketError> {
        let claims = self.verify(ticket, expect)?;
        if claims.iat < self.started_ms {
            return Err(TicketError::BeforeStart);
        }
        let mut cache = self.replay.lock().unwrap_or_else(PoisonError::into_inner);
        cache.purge(self.clock.now_ms().saturating_sub(self.skew_ms()));
        if cache.seen.contains(&claims.jti) {
            return Err(TicketError::Replayed);
        }
        if cache.seen.len() >= MAX_REPLAY_ENTRIES {
            return Err(TicketError::ReplayCacheFull);
        }
        cache.seen.insert(claims.jti.clone());
        cache.by_expiry.insert((claims.exp, claims.jti.clone()));
        Ok(claims)
    }

    /// Ends every session of `account` that began at or before now: tickets issued so far stop working here,
    /// and can't be renewed. A login after this gets new ones. Persisted in `network_ticket_revocations` when
    /// a database is attached (with bounded retry on transient write contention) so a restart does not
    /// reinstate banned sessions.
    pub async fn revoke(&self, account: &GlobalId) -> Result<(), RevocationPersistError> {
        let now = self.clock.now_ms();
        #[cfg_attr(
            not(feature = "directory"),
            allow(unused_variables, reason = "only the directory persists it")
        )]
        let floor = {
            let mut revoked = self.revoked.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = revoked.entry(account.clone()).or_insert(now);
            *entry = (*entry).max(now);
            *entry
        };
        #[cfg(feature = "directory")]
        {
            const MAX_PERSIST_ATTEMPTS: u32 = 3;
            let db = self.db.read().unwrap_or_else(PoisonError::into_inner).clone();
            if let Some(db) = db {
                let mut last_error: Option<String> = None;
                for attempt in 1..=MAX_PERSIST_ATTEMPTS {
                    let account_str = account.to_string();
                    let write_res: Result<(), dainty_store::Error> = db
                        .write(dainty_store::Actor::System("ticket-revocation".into()), move |tx| {
                            use diesel::prelude::*;
                            use diesel::sql_types::{BigInt, Text};
                            let sql = dainty_store::portable::upsert(
                                "network_ticket_revocations",
                                &["account", "revoked_at"],
                                &["account"],
                                &["revoked_at"],
                            );
                            diesel::sql_query(sql)
                                .bind::<Text, _>(account_str)
                                .bind::<BigInt, _>(floor)
                                .execute(tx.conn())?;
                            Ok(())
                        })
                        .await;
                    match write_res {
                        Ok(()) => {
                            last_error = None;
                            break;
                        }
                        Err(error) => {
                            last_error = Some(error.to_string());
                            if attempt < MAX_PERSIST_ATTEMPTS {
                                tokio::time::sleep(Duration::from_millis(25 * u64::from(attempt))).await;
                            }
                        }
                    }
                }
                if let Some(reason) = last_error {
                    tracing::error!(
                        account = %account,
                        attempts = MAX_PERSIST_ATTEMPTS,
                        error = %reason,
                        "could not persist ticket revocation floor"
                    );
                    return Err(RevocationPersistError {
                        account: account.clone(),
                        reason,
                    });
                }
            }
        }
        Ok(())
    }

    /// Clears an earlier [`Self::revoke`] for `account` (after an unban or lifted timeout).
    pub async fn allow(&self, account: &GlobalId) -> Result<(), RevocationPersistError> {
        #[cfg_attr(
            not(feature = "directory"),
            allow(unused_variables, reason = "only the directory persists it")
        )]
        let previous_floor = self
            .revoked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account);
        #[cfg(feature = "directory")]
        {
            const MAX_PERSIST_ATTEMPTS: u32 = 3;
            let db = self.db.read().unwrap_or_else(PoisonError::into_inner).clone();
            if let Some(db) = db {
                let mut last_error: Option<String> = None;
                for attempt in 1..=MAX_PERSIST_ATTEMPTS {
                    let account_str = account.to_string();
                    let write_res: Result<(), dainty_store::Error> = db
                        .write(dainty_store::Actor::System("ticket-revocation".into()), move |tx| {
                            use diesel::prelude::*;
                            use diesel::sql_types::Text;
                            diesel::sql_query("DELETE FROM network_ticket_revocations WHERE account = $1")
                                .bind::<Text, _>(account_str)
                                .execute(tx.conn())?;
                            Ok(())
                        })
                        .await;
                    match write_res {
                        Ok(()) => {
                            last_error = None;
                            break;
                        }
                        Err(error) => {
                            last_error = Some(error.to_string());
                            if attempt < MAX_PERSIST_ATTEMPTS {
                                tokio::time::sleep(Duration::from_millis(25 * u64::from(attempt))).await;
                            }
                        }
                    }
                }
                if let Some(reason) = last_error {
                    if let Some(prev) = previous_floor {
                        let mut revoked = self.revoked.lock().unwrap_or_else(PoisonError::into_inner);
                        let entry = revoked.entry(account.clone()).or_insert(prev);
                        *entry = (*entry).max(prev);
                    }
                    tracing::error!(
                        account = %account,
                        attempts = MAX_PERSIST_ATTEMPTS,
                        error = %reason,
                        "could not clear persisted ticket revocation floor"
                    );
                    return Err(RevocationPersistError {
                        account: account.clone(),
                        reason,
                    });
                }
            }
        }
        Ok(())
    }

    /// Whether a session of `account` that started at `at` (ms) has been revoked here.
    pub fn is_revoked_at(&self, account: &GlobalId, at: i64) -> bool {
        self.revoked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .is_some_and(|floor| at <= *floor)
    }

    /// Replay entries held, for tests.
    pub fn replay_len(&self) -> usize {
        self.replay.lock().unwrap_or_else(PoisonError::into_inner).seen.len()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    fn identity(node: &str) -> NodeIdentity {
        NodeIdentity {
            node: node.parse().unwrap(),
            region: "oregon".parse().unwrap(),
        }
    }

    fn node(name: &str, clock: &Arc<Clock>) -> Tickets {
        Tickets::new(
            &identity(name),
            SigningKey::generate().unwrap(),
            clock.clone(),
            TicketConfig::default(),
        )
    }

    /// A home, an audience that knows the home's keys, and an account.
    struct Setup {
        clock: Arc<Clock>,
        home: Tickets,
        audience: Tickets,
        account: GlobalId,
        game: GameVersion,
    }

    fn setup() -> Setup {
        let clock = Arc::new(Clock::default());
        let home = node("germany-1", &clock);
        let audience = node("oregon-1", &clock);
        audience.learn(home.node(), home.public_keys());
        home.learn(audience.node(), audience.public_keys());
        Setup {
            account: GlobalId::new(IdKind::Account, home.node()),
            game: "1.14.7".parse().unwrap(),
            clock,
            home,
            audience,
        }
    }

    impl Setup {
        fn expect(&self) -> Expect<'_> {
            Expect {
                issuer: self.home.node(),
                audience: self.audience.node(),
                game: Some(&self.game),
                account: Some(&self.account),
            }
        }

        fn ticket(&self) -> SessionTicket {
            self.home
                .issue(&self.account, self.audience.node(), &self.game, None)
                .unwrap()
        }
    }

    #[test]
    fn a_ticket_is_signed_by_the_home_and_checked_by_its_audience() {
        let s = setup();
        let ticket = s.ticket();
        assert!(ticket.token.starts_with("dlt1.") && ticket.token.len() < MAX_TOKEN);
        let claims = s.audience.verify(&ticket, &s.expect()).unwrap();
        assert_eq!(claims.iss, *s.home.node());
        assert_eq!(claims.aud, *s.audience.node());
        assert_eq!(claims.sub, s.account);
        assert_eq!(claims.game, s.game);
        assert_eq!(claims.exp - claims.iat, 300_000);
        assert_eq!(claims.since, claims.iat);
        assert_eq!(s.audience.peek(&ticket).unwrap(), claims);
        // The home accepts its own, naming the node that presents it as the audience
        let mut at_home = s.expect();
        at_home.audience = s.audience.node();
        s.home.verify(&ticket, &at_home).unwrap();
    }

    #[test]
    fn a_ticket_nobody_but_the_homes_key_signed_is_refused() {
        let s = setup();
        let ticket = s.ticket();
        let (head, rest) = ticket.token.split_once('.').unwrap();
        let (payload, signature) = rest.split_once('.').unwrap();
        // Edited claims
        let mut claims: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        claims["exp"] = serde_json::json!(claims["exp"].as_i64().unwrap() + 3_600_000);
        let edited = SessionTicket {
            token: format!(
                "{head}.{}.{signature}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
            ),
            ..ticket.clone()
        };
        assert_eq!(s.audience.verify(&edited, &s.expect()), Err(TicketError::BadSignature));
        // A flipped signature
        let mut sig = URL_SAFE_NO_PAD.decode(signature).unwrap();
        sig[0] ^= 1;
        let flipped = SessionTicket {
            token: format!("{head}.{payload}.{}", URL_SAFE_NO_PAD.encode(sig)),
            ..ticket.clone()
        };
        assert_eq!(s.audience.verify(&flipped, &s.expect()), Err(TicketError::BadSignature));
        // Another node signing as the home: its key id isn't one the home told us
        let foreign = node("uk-1", &s.clock);
        let forged = foreign.issue(&s.account, s.audience.node(), &s.game, None).unwrap();
        assert_eq!(s.audience.verify(&forged, &s.expect()), Err(TicketError::UnknownKey));
        // ...even if it names the home as issuer by editing the claims and keeping its own key id
        let (_, rest) = forged.token.split_once('.').unwrap();
        let (payload, signature) = rest.split_once('.').unwrap();
        let mut claims: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        claims["iss"] = serde_json::json!(s.home.node().as_str());
        let relabelled = SessionTicket {
            token: format!(
                "dlt1.{}.{signature}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
            ),
            ..forged
        };
        assert_eq!(
            s.audience.verify(&relabelled, &s.expect()),
            Err(TicketError::UnknownKey)
        );
        // A peer can't replace the keys this node trusts for the home by claiming to be it
        assert!(!s.home.learn(s.home.node(), foreign.public_keys()));
    }

    #[test]
    fn a_ticket_for_another_node_account_game_or_issuer_is_refused() {
        let s = setup();
        let ticket = s.ticket();
        let other_node: NodeId = "uk-1".parse().unwrap();
        let mut expect = s.expect();
        expect.audience = &other_node;
        assert_eq!(s.audience.verify(&ticket, &expect), Err(TicketError::WrongAudience));
        let game: GameVersion = "1.12.0".parse().unwrap();
        let mut expect = s.expect();
        expect.game = Some(&game);
        assert_eq!(s.audience.verify(&ticket, &expect), Err(TicketError::WrongGame));
        let account = GlobalId::new(IdKind::Account, s.home.node());
        let mut expect = s.expect();
        expect.account = Some(&account);
        assert_eq!(s.audience.verify(&ticket, &expect), Err(TicketError::WrongAccount));
        let mut expect = s.expect();
        expect.issuer = &other_node;
        assert_eq!(s.audience.verify(&ticket, &expect), Err(TicketError::WrongIssuer));
        // The outer fields must say what the claims say
        let lie = SessionTicket {
            audience: other_node,
            ..ticket.clone()
        };
        assert_eq!(s.audience.verify(&lie, &s.expect()), Err(TicketError::Malformed));
        let lie = SessionTicket {
            account: account.clone(),
            ..ticket
        };
        assert_eq!(s.audience.verify(&lie, &s.expect()), Err(TicketError::Malformed));
        // An id of another kind is no account
        let hunt = GlobalId::new(IdKind::Hunt, s.home.node());
        assert_eq!(
            s.home.issue(&hunt, s.audience.node(), &s.game, None),
            Err(TicketError::WrongAccount)
        );
    }

    #[test]
    fn tickets_expire_and_clocks_may_differ_a_little() {
        let s = setup();
        let ticket = s.ticket();
        // Four minutes on: fine. Five minutes and 29 seconds: the skew allowance. Past it: expired
        s.clock.skew_by(240_000);
        s.audience.verify(&ticket, &s.expect()).unwrap();
        s.clock.skew_by(329_000);
        s.audience.verify(&ticket, &s.expect()).unwrap();
        s.clock.skew_by(331_000);
        assert_eq!(s.audience.verify(&ticket, &s.expect()), Err(TicketError::Expired));
        // A ticket from a clock far ahead is not yet valid; a few seconds ahead is fine
        s.clock.skew_by(0);
        let early = s.home.issue(&s.account, s.audience.node(), &s.game, None).unwrap();
        s.clock.skew_by(-20_000);
        s.audience.verify(&early, &s.expect()).unwrap();
        s.clock.skew_by(-40_000);
        assert_eq!(s.audience.verify(&early, &s.expect()), Err(TicketError::NotYetValid));
    }

    #[test]
    fn a_ticket_is_admitted_once() {
        let s = setup();
        let ticket = s.ticket();
        s.audience.redeem(&ticket, &s.expect()).unwrap();
        assert_eq!(s.audience.replay_len(), 1);
        assert_eq!(s.audience.redeem(&ticket, &s.expect()), Err(TicketError::Replayed));
        // A second ticket for the same player is its own
        let another = s.ticket();
        s.audience.redeem(&another, &s.expect()).unwrap();
        // Verifying alone doesn't use a ticket up
        let third = s.ticket();
        s.audience.verify(&third, &s.expect()).unwrap();
        s.audience.verify(&third, &s.expect()).unwrap();
        s.audience.redeem(&third, &s.expect()).unwrap();
        // The memory is dropped once a ticket couldn't be accepted anyway
        s.clock.skew_by(400_000);
        let fresh = s.ticket();
        s.audience.redeem(&fresh, &s.expect()).unwrap();
        assert_eq!(s.audience.replay_len(), 1);
    }

    #[test]
    fn a_node_that_restarted_refuses_tickets_it_cannot_know_were_used() {
        let s = setup();
        let old = s.ticket();
        s.clock.skew_by(5_000);
        let restarted = node("oregon-1", &s.clock);
        restarted.learn(s.home.node(), s.home.public_keys());
        assert_eq!(restarted.redeem(&old, &s.expect()), Err(TicketError::BeforeStart));
        // Verifying (no use) is not affected, and a ticket made after the start is admitted
        restarted.verify(&old, &s.expect()).unwrap();
        s.clock.skew_by(6_000);
        let new = s.ticket();
        restarted.redeem(&new, &s.expect()).unwrap();
    }

    #[test]
    fn a_full_replay_memory_fails_closed() {
        let s = setup();
        {
            let mut cache = s.audience.replay.lock().unwrap();
            for n in 0..MAX_REPLAY_ENTRIES {
                cache.seen.insert(format!("{n:032x}"));
            }
        }
        assert_eq!(
            s.audience.redeem(&s.ticket(), &s.expect()),
            Err(TicketError::ReplayCacheFull)
        );
    }

    #[tokio::test]
    async fn an_ended_session_and_an_old_login_stop_working_at_the_home() {
        let s = setup();
        let mut at_home = s.expect();
        at_home.audience = s.audience.node();
        let ticket = s.ticket();
        s.home.verify(&ticket, &at_home).unwrap();
        s.clock.skew_by(1_000);
        s.home.revoke(&s.account).await.unwrap();
        assert_eq!(s.home.verify(&ticket, &at_home), Err(TicketError::Revoked));
        // The audience can't know, but a ticket made after the revocation is good
        s.audience.verify(&ticket, &s.expect()).unwrap();
        s.clock.skew_by(2_000);
        let new = s.ticket();
        s.home.verify(&new, &at_home).unwrap();
        // A login older than the limit is not renewed
        let started = s.clock.now_ms();
        s.clock.skew_by(2_000 + 6 * 3_600_000 + 1_000);
        assert_eq!(
            s.home.issue(&s.account, s.audience.node(), &s.game, Some(started)),
            Err(TicketError::SessionTooOld)
        );
    }

    #[test]
    fn a_rotated_key_signs_new_tickets_and_the_old_ones_live_on() {
        let s = setup();
        let before = s.ticket();
        let old_kid = s.home.public_keys()[0].kid().to_owned();
        s.home.rotate(SigningKey::generate().unwrap());
        let keys = s.home.public_keys();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0].kid(), old_kid);
        let after = s.ticket();
        // The audience has only the old key until it asks again
        s.audience.verify(&before, &s.expect()).unwrap();
        assert_eq!(s.audience.verify(&after, &s.expect()), Err(TicketError::UnknownKey));
        assert!(s.audience.learn(s.home.node(), keys));
        s.audience.verify(&after, &s.expect()).unwrap();
        s.audience.verify(&before, &s.expect()).unwrap();
        // The retired key stops being told once no ticket it signed can be alive
        s.clock.skew_by(1_000_000);
        assert_eq!(s.home.public_keys().len(), 1);
    }

    #[test]
    fn nonsense_is_malformed_not_a_panic() {
        let s = setup();
        let account = s.account.clone();
        let audience = s.audience.node().clone();
        let with = |token: &str| SessionTicket {
            account: account.clone(),
            audience: audience.clone(),
            token: token.to_owned(),
        };
        let good = s.ticket().token;
        let (_, rest) = good.split_once('.').unwrap();
        for token in [
            "",
            ".",
            "..",
            "dlt1",
            "dlt1..",
            "dlt1.a.b",
            "dlt2.a.b",
            "dlt1.!!!.???",
            &format!("dlt1.{rest}.extra"),
            &format!("dlt0.{rest}"),
            &"a".repeat(MAX_TOKEN + 1),
        ] {
            assert_eq!(
                s.audience.verify(&with(token), &s.expect()),
                Err(TicketError::Malformed),
                "{token}"
            );
        }
        // Padding or other spellings of the same bytes are not the same ticket
        let padded = format!("dlt1.{rest}=");
        assert_eq!(
            s.audience.verify(&with(&padded), &s.expect()),
            Err(TicketError::Malformed)
        );
    }

    #[test]
    fn tickets_hold_no_account_key_and_debug_shows_no_key() {
        let s = setup();
        let ticket = s.ticket();
        let payload = URL_SAFE_NO_PAD.decode(ticket.token.split('.').nth(1).unwrap()).unwrap();
        let text = String::from_utf8(payload).unwrap();
        assert!(!text.to_ascii_lowercase().contains("rrk_"));
        let shown = format!("{:?}", s.home);
        assert!(shown.contains("germany-1") && !shown.contains(s.home.public_keys()[0].kid()));
    }

    #[cfg(feature = "directory")]
    #[tokio::test]
    async fn revocation_persistence_failure_is_returned_and_fails_closed() {
        let s = setup();
        let dir = tempfile::tempdir().unwrap();
        let db = dainty_store::Database::open(
            &dainty_store::DatabaseConfig::Sqlite {
                file: dir.path().join("tickets.db"),
            },
            &dainty_store::DatabaseOptions::default(),
            &crate::directory::SCHEMA,
        )
        .unwrap();
        s.home.attach_database(db.clone()).await.unwrap();

        let ticket_iat = s.clock.now_ms();
        let ticket = s.ticket();
        s.home.revoke(&s.account).await.unwrap();
        assert!(s.home.is_revoked_at(&s.account, ticket_iat));

        // Drop the network_ticket_revocations table to simulate a database write failure.
        db.write(dainty_store::Actor::System("test".into()), |tx| {
            use diesel::connection::SimpleConnection as _;
            tx.conn().batch_execute("DROP TABLE network_ticket_revocations")?;
            Ok::<_, dainty_store::Error>(())
        })
        .await
        .unwrap();

        // `allow` fails visibly after bounded retries and keeps the account revoked in memory (fail closed).
        let err = s.home.allow(&s.account).await.unwrap_err();
        assert_eq!(err.account, s.account);
        assert!(s.home.is_revoked_at(&s.account, ticket_iat));
        let at_home = Expect {
            issuer: s.home.node(),
            audience: s.audience.node(),
            game: Some(&s.game),
            account: Some(&s.account),
        };
        assert_eq!(s.home.verify(&ticket, &at_home), Err(TicketError::Revoked));

        // `revoke` on another account also fails visibly after retries while revoking in memory.
        let other = GlobalId::new(IdKind::Account, s.home.node());
        let before_other_revoke = s.clock.now_ms();
        let err = s.home.revoke(&other).await.unwrap_err();
        assert_eq!(err.account, other);
        assert!(s.home.is_revoked_at(&other, before_other_revoke));
    }
}
