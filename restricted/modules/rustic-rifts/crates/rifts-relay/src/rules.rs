//! Who may set up an ICE connection with whom, the signals they exchange through the backend and the keys that
//! encrypt each host-joiner pair: `p2p/security/iceSessionRules.ts`, pure functions over the hunt as realms keeps it.

use base64::Engine as _;
use hmac::{KeyInit, Mac};
use rifts_seam::{AccountId, PeerId, Secret, UnixMillis};
use serde_json::Value;

use crate::wire::{js_trim, safe_integer};

/// `P2PHuntState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HuntState {
    Requested,
    HostSelecting,
    HostOffered,
    Starting,
    HostReady,
    Admitting,
    Running,
    Draining,
    Suspect,
    Fenced,
    Aborting,
    Recovering,
    Completed,
    Aborted,
}

impl HuntState {
    /// `ICE_ACTIVE_STATES`: participants may (re)build their connections. A drained server's members keep theirs;
    /// admission is still refused there, so no one new joins by signalling alone.
    pub const fn is_ice_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::HostReady | Self::Admitting | Self::Running | Self::Draining | Self::Suspect
        )
    }

    /// `TICKETABLE_STATES` (`p2pAdmission.ts`).
    pub const fn is_ticketable(self) -> bool {
        matches!(self, Self::HostReady | Self::Admitting | Self::Running)
    }
}

/// `sessionKind`: a hunt, or a realm's hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionKind {
    Hunt,
    RealmHub,
}

/// `operationsPolicy.coopTransport`, frozen on each hunt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoopTransport {
    Ice,
    SteamExperimental,
}

impl CoopTransport {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ice => "ICE",
            Self::SteamExperimental => "STEAM_EXPERIMENTAL",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ICE" => Some(Self::Ice),
            "STEAM_EXPERIMENTAL" => Some(Self::SteamExperimental),
            _ => None,
        }
    }
}

/// `RealmHubPresenceState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresenceState {
    Pending,
    Ticketed,
    Admitted,
    Lost,
    Left,
    Removed,
    Expired,
}

impl PresenceState {
    /// `PRESENCE_LIVE_STATES`: expected (`PENDING`, `TICKETED`) or connected (`ADMITTED`).
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Pending | Self::Ticketed | Self::Admitted)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Ticketed => "TICKETED",
            Self::Admitted => "ADMITTED",
            Self::Lost => "LOST",
            Self::Left => "LEFT",
            Self::Removed => "REMOVED",
            Self::Expired => "EXPIRED",
        }
    }
}

/// One roster entry of a hunt (`P2PHuntParticipantRecord`), as the ICE rules read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub account: AccountId,
    /// `steamId`: the transport peer id the tickets carry.
    pub peer: PeerId,
    /// `returnedToHubAt` is set: the player is back in Ramsgate.
    pub returned_to_hub: bool,
}

/// `IcePresenceRow`: a hub member, from realms' presence rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceRow {
    pub account: AccountId,
    pub peer: PeerId,
    pub state: PresenceState,
}

/// The fields of `P2PHuntSessionRecord` the relay and admission rules read. Realms (RF5) fills it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntSnapshot {
    /// Node's `sessionId` text (a UUID), which the tickets, the pair keys and every reply carry.
    pub wire_session: String,
    pub generation: u64,
    pub state: HuntState,
    pub kind: SessionKind,
    /// `realmId`: a realm's hub or hunt.
    pub realm: Option<String>,
    /// `coopTransport`, `STEAM_EXPERIMENTAL` for hunts that predate it.
    pub coop_transport: CoopTransport,
    pub host_account: AccountId,
    /// `hostSteamId`.
    pub host_peer: PeerId,
    pub expires_at: UnixMillis,
    /// Empty for a hub, whose members are presence rows.
    pub roster: Vec<RosterEntry>,
}

impl HuntSnapshot {
    pub fn is_hub(&self) -> bool {
        self.kind == SessionKind::RealmHub
    }

    /// `IsRealmSession`.
    pub fn is_realm_session(&self) -> bool {
        self.realm.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRole {
    Host,
    Join,
}

impl IceRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "HOST",
            Self::Join => "JOIN",
        }
    }
}

/// The most joiners a hub host's peer set carries (the runtime's hub ceiling).
pub const MAXIMUM_HUB_PEERS: usize = 64;
pub const MAX_SIGNAL_BYTES: usize = 4096;

/// `ResolveIceRole`: the caller's role, or `None` when it is not (or no longer) part of the hunt.
pub fn resolve_role(hunt: &HuntSnapshot, account: &AccountId, presence: Option<&[PresenceRow]>) -> Option<IceRole> {
    if &hunt.host_account == account {
        return Some(IceRole::Host);
    }
    // A LOST hub member needs a fresh entry first
    if hunt.is_hub() {
        let live = presence.is_some_and(|rows| rows.iter().any(|row| &row.account == account && row.state.is_live()));
        return live.then_some(IceRole::Join);
    }
    hunt.roster
        .iter()
        .find(|entry| &entry.account == account)
        .filter(|entry| !entry.returned_to_hub)
        .map(|_| IceRole::Join)
}

/// `ActiveJoiners`: joiners still in the hunt, the host and players back in Ramsgate left out.
pub fn active_joiners(hunt: &HuntSnapshot, presence: Option<&[PresenceRow]>) -> Vec<AccountId> {
    if hunt.is_hub() {
        let mut accounts: Vec<AccountId> = Vec::new();
        for row in presence.unwrap_or_default() {
            if row.account != hunt.host_account && row.state.is_live() && !accounts.contains(&row.account) {
                accounts.push(row.account.clone());
            }
        }
        accounts.truncate(MAXIMUM_HUB_PEERS);
        return accounts;
    }
    hunt.roster
        .iter()
        .filter(|entry| entry.account != hunt.host_account && !entry.returned_to_hub)
        .map(|entry| entry.account.clone())
        .collect()
}

/// `DeriveIcePairKey`: 32 bytes, base64url (43 characters). Derived, not stored, so host and joiner get the same key
/// from separate requests and no other participant can read that pair's packets.
pub fn derive_pair_key(
    secret: &Secret,
    wire_session: &str,
    generation: u64,
    host: &AccountId,
    joiner: &AccountId,
) -> Secret {
    let message = format!("mysticparadox-ice-pair-v1\n{wire_session}\n{generation}\n{host}\n{joiner}");
    // HMAC takes a key of any length
    let key = match hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.expose().as_bytes()) {
        Ok(mut mac) => {
            mac.update(message.as_bytes());
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        }
        Err(_) => String::new(),
    };
    Secret::new(key)
}

/// One peer of `IcePeersFor`, before its peer id is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcePeer {
    pub account: AccountId,
    pub role: IceRole,
    pub transport_key: Secret,
}

/// `IcePeersFor`: every active joiner for the host, the host for a joiner, each with the pair's key.
pub fn ice_peers_for(
    hunt: &HuntSnapshot,
    account: &AccountId,
    role: IceRole,
    secret: &Secret,
    presence: Option<&[PresenceRow]>,
) -> Vec<IcePeer> {
    let key =
        |joiner: &AccountId| derive_pair_key(secret, &hunt.wire_session, hunt.generation, &hunt.host_account, joiner);
    match role {
        IceRole::Host => active_joiners(hunt, presence)
            .into_iter()
            .map(|joiner| IcePeer {
                transport_key: key(&joiner),
                account: joiner,
                role: IceRole::Join,
            })
            .collect(),
        IceRole::Join => vec![IcePeer {
            account: hunt.host_account.clone(),
            role: IceRole::Host,
            transport_key: key(account),
        }],
    }
}

/// `PeerIdOf`: a participant's transport peer id as frozen in the hunt (the host's is `hostSteamId`); `None` is
/// Node's empty string.
pub fn peer_id_of(hunt: &HuntSnapshot, account: &AccountId, presence: Option<&[PresenceRow]>) -> Option<PeerId> {
    if &hunt.host_account == account {
        return Some(hunt.host_peer);
    }
    if hunt.is_hub() {
        return presence?.iter().find(|row| &row.account == account).map(|row| row.peer);
    }
    hunt.roster
        .iter()
        .find(|entry| &entry.account == account)
        .map(|entry| entry.peer)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Description,
    Candidate,
    GatheringDone,
}

impl SignalKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Description => "DESCRIPTION",
            Self::Candidate => "CANDIDATE",
            Self::GatheringDone => "GATHERING_DONE",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "DESCRIPTION" => Some(Self::Description),
            "CANDIDATE" => Some(Self::Candidate),
            "GATHERING_DONE" => Some(Self::GatheringDone),
            _ => None,
        }
    }
}

/// A signal body that passed [`parse_ice_signal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceSignal {
    pub generation: i64,
    pub to: AccountId,
    pub kind: SignalKind,
    /// Up to 4096 bytes of printable ASCII and line breaks: SDP attribute lines.
    pub data: String,
}

/// `^[A-Za-z0-9-]{8,64}$`
fn signal_account(value: &str) -> bool {
    (8..=64).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `ParseIceSignal`: `None` when a field is missing, malformed or too large.
pub fn parse_ice_signal(body: &Value) -> Option<IceSignal> {
    let generation = body.get("generation").and_then(safe_integer).filter(|g| *g >= 0)?;
    let to = body
        .get("toAccountId")
        .and_then(Value::as_str)
        .filter(|to| signal_account(to))?;
    let kind = body.get("kind").and_then(Value::as_str).and_then(SignalKind::parse)?;
    // `body.data ?? ""`
    let data = match body.get("data") {
        None | Some(Value::Null) => "",
        Some(Value::String(data)) => data.as_str(),
        Some(_) => return None,
    };
    let printable = data
        .bytes()
        .all(|b| (0x20..=0x7e).contains(&b) || b == b'\r' || b == b'\n');
    if data.len() > MAX_SIGNAL_BYTES || !printable {
        return None;
    }
    if kind != SignalKind::GatheringDone && js_trim(data).is_empty() {
        return None;
    }
    Some(IceSignal {
        generation,
        to: AccountId(to.to_owned()),
        kind,
        data: data.to_owned(),
    })
}

/// `SignalRouteAllowed`: only between the host and a joiner, never between two joiners or to oneself.
pub fn signal_route_allowed(
    hunt: &HuntSnapshot,
    from: &AccountId,
    to: &AccountId,
    presence: Option<&[PresenceRow]>,
) -> bool {
    if from == to {
        return false;
    }
    match (resolve_role(hunt, from, presence), resolve_role(hunt, to, presence)) {
        (Some(from), Some(to)) => (from == IceRole::Host) != (to == IceRole::Host),
        _ => false,
    }
}

/// `^(?:a=)?candidate:`
fn candidate_line(line: &str) -> bool {
    line.strip_prefix("a=").unwrap_or(line).starts_with("candidate:")
}

/// `/ typ relay(?: |$)/`
fn relay_candidate(line: &str) -> bool {
    line.match_indices(" typ relay")
        .any(|(at, found)| matches!(line.as_bytes().get(at + found.len()), None | Some(b' ')))
}

/// `^(?:a=ice-ufrag:|a=ice-pwd:|a=ice-options:|a=end-of-candidates$)`
fn relay_only_keep(line: &str) -> bool {
    line.starts_with("a=ice-ufrag:")
        || line.starts_with("a=ice-pwd:")
        || line.starts_with("a=ice-options:")
        || line == "a=end-of-candidates"
}

/// `FilterRelayOnly`: with relay-only on, a signal keeps only relay candidates and the ICE credentials, so neither
/// side learns the other's address even with a modified client. `None` when nothing is left to deliver.
pub fn filter_relay_only(kind: SignalKind, data: &str) -> Option<String> {
    if kind == SignalKind::GatheringDone {
        return Some(data.to_owned());
    }
    let separator = if data.contains("\r\n") { "\r\n" } else { "\n" };
    let kept: Vec<&str> = data
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .map(js_trim)
        .filter(|line| {
            if line.is_empty() {
                return false;
            }
            if candidate_line(line) {
                return relay_candidate(line);
            }
            kind == SignalKind::Description && relay_only_keep(line)
        })
        .collect();
    match kind {
        SignalKind::Candidate => match kept.as_slice() {
            [only] => Some((*only).to_owned()),
            _ => None,
        },
        _ => kept
            .iter()
            .any(|line| line.starts_with("a=ice-ufrag:"))
            .then(|| kept.join(separator)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const HOST: &str = "host-0000-account";
    pub(crate) const JOIN_A: &str = "join-aaaa-account";
    pub(crate) const JOIN_B: &str = "join-bbbb-account";
    pub(crate) const HOME: &str = "join-home-account";

    pub(crate) fn account(id: &str) -> AccountId {
        AccountId(id.to_owned())
    }

    fn secret() -> Secret {
        Secret::new("s".repeat(40))
    }

    pub(crate) fn hunt() -> HuntSnapshot {
        let entry = |id: &str, peer: u64, returned_to_hub: bool| RosterEntry {
            account: account(id),
            peer: PeerId(peer),
            returned_to_hub,
        };
        HuntSnapshot {
            wire_session: "session-1".to_owned(),
            generation: 3,
            state: HuntState::Running,
            kind: SessionKind::Hunt,
            realm: None,
            coop_transport: CoopTransport::Ice,
            host_account: account(HOST),
            host_peer: PeerId(1001),
            expires_at: UnixMillis(1_791_892_800_000), // 2026-10-13T12:00:00.000Z
            roster: vec![
                entry(HOST, 1001, false),
                entry(JOIN_A, 1002, false),
                entry(JOIN_B, 1003, false),
                entry(HOME, 1004, true),
            ],
        }
    }

    // iceSessionRules.test.ts "roles: the host, joiners still in the hunt, and nobody else"
    #[test]
    fn roles_are_the_host_joiners_still_in_the_hunt_and_nobody_else() {
        let hunt = hunt();
        assert_eq!(resolve_role(&hunt, &account(HOST), None), Some(IceRole::Host));
        assert_eq!(resolve_role(&hunt, &account(JOIN_A), None), Some(IceRole::Join));
        assert_eq!(resolve_role(&hunt, &account(JOIN_B), None), Some(IceRole::Join)); // lost players may rejoin
        assert_eq!(resolve_role(&hunt, &account(HOME), None), None); // back in Ramsgate
        assert_eq!(resolve_role(&hunt, &account("stranger-account"), None), None);
        assert!(HuntState::Running.is_ice_active());
        assert!(!HuntState::Completed.is_ice_active());
        assert!(!HuntState::Aborting.is_ice_active());
    }

    #[test]
    fn a_drained_hunts_members_keep_their_connections() {
        assert!(HuntState::Draining.is_ice_active());
        let drained = HuntSnapshot {
            state: HuntState::Draining,
            ..hunt()
        };
        assert_eq!(resolve_role(&drained, &account(JOIN_A), None), Some(IceRole::Join));
        assert!(!HuntState::Fenced.is_ice_active());
        assert!(!HuntState::Recovering.is_ice_active());
        assert!(!HuntState::Draining.is_ticketable());
    }

    #[test]
    fn pair_keys_match_for_host_and_joiner_and_differ_between_pairs_and_generations() {
        let hunt = hunt();
        let host_peers = ice_peers_for(&hunt, &account(HOST), IceRole::Host, &secret(), None);
        let accounts: Vec<&str> = host_peers.iter().map(|peer| peer.account.0.as_str()).collect();
        assert_eq!(accounts, [JOIN_A, JOIN_B]);
        let join_peers = ice_peers_for(&hunt, &account(JOIN_A), IceRole::Join, &secret(), None);
        assert_eq!(
            join_peers
                .iter()
                .map(|peer| (peer.account.0.as_str(), peer.role))
                .collect::<Vec<_>>(),
            [(HOST, IceRole::Host)]
        );
        assert_eq!(join_peers[0].transport_key, host_peers[0].transport_key);
        assert_ne!(host_peers[0].transport_key, host_peers[1].transport_key);
        assert_ne!(
            derive_pair_key(&secret(), "session-1", 4, &account(HOST), &account(JOIN_A)),
            host_peers[0].transport_key
        );
        let key = host_peers[0].transport_key.expose();
        assert_eq!(key.len(), 43);
        assert_eq!(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(key)
                .unwrap()
                .len(),
            32
        );
    }

    #[test]
    fn signals_are_well_formed_bodies_only() {
        let valid = serde_json::json!({ "generation": 3, "toAccountId": HOST, "kind": "CANDIDATE",
            "data": "a=candidate:1 1 UDP 1 203.0.113.5 50000 typ srflx" });
        assert_eq!(
            parse_ice_signal(&valid),
            Some(IceSignal {
                generation: 3,
                to: account(HOST),
                kind: SignalKind::Candidate,
                data: "a=candidate:1 1 UDP 1 203.0.113.5 50000 typ srflx".to_owned(),
            })
        );
        let with = |key: &str, value: Value| {
            let mut body = valid.clone();
            body[key] = value;
            parse_ice_signal(&body)
        };
        assert_eq!(with("kind", "OFFER".into()), None);
        assert_eq!(with("generation", "3".into()), None);
        assert_eq!(with("toAccountId", "../x".into()), None);
        assert_eq!(with("data", "x".repeat(4097).into()), None);
        assert_eq!(with("data", "café".into()), None);
        assert_eq!(with("data", 5.into()), None);
        let mut description = valid.clone();
        description["kind"] = "DESCRIPTION".into();
        description["data"] = "  ".into();
        assert_eq!(parse_ice_signal(&description), None);
        let done = serde_json::json!({ "generation": 3, "toAccountId": HOST, "kind": "GATHERING_DONE" });
        assert_eq!(parse_ice_signal(&done).map(|signal| signal.data), Some(String::new()));
        assert_eq!(with("generation", (-1).into()), None);
    }

    #[test]
    fn signals_go_only_between_the_host_and_a_joiner() {
        let hunt = hunt();
        let allowed = |from: &str, to: &str| signal_route_allowed(&hunt, &account(from), &account(to), None);
        assert!(allowed(HOST, JOIN_A));
        assert!(allowed(JOIN_A, HOST));
        assert!(!allowed(JOIN_A, JOIN_B));
        assert!(!allowed(HOST, HOST));
        assert!(!allowed(HOST, HOME));
        assert!(!allowed("stranger-account", HOST));
    }

    #[test]
    fn relay_only_keeps_relay_candidates_and_the_ice_credentials_never_an_address() {
        let description = [
            "c=IN IP4 192.168.1.20",
            "a=ice-ufrag:abcd",
            "a=ice-pwd:0123456789abcdef0123456789",
            "a=ice-options:ice2,trickle",
            "a=candidate:1 1 UDP 2122317823 192.168.1.20 50000 typ host",
            "a=candidate:2 1 UDP 1686052607 203.0.113.5 50000 typ srflx raddr 192.168.1.20 rport 50000",
            "a=candidate:3 1 UDP 41885439 104.30.1.1 3478 typ relay raddr 203.0.113.5 rport 50000",
            "a=end-of-candidates",
        ]
        .join("\r\n");
        let kept = filter_relay_only(SignalKind::Description, &description).unwrap();
        assert_eq!(
            kept.split("\r\n").collect::<Vec<_>>(),
            [
                "a=ice-ufrag:abcd",
                "a=ice-pwd:0123456789abcdef0123456789",
                "a=ice-options:ice2,trickle",
                "a=candidate:3 1 UDP 41885439 104.30.1.1 3478 typ relay raddr 203.0.113.5 rport 50000",
                "a=end-of-candidates"
            ]
        );
        assert_eq!(
            filter_relay_only(
                SignalKind::Candidate,
                "a=candidate:1 1 UDP 2122317823 192.168.1.20 50000 typ host"
            ),
            None
        );
        assert_eq!(
            filter_relay_only(
                SignalKind::Candidate,
                "a=candidate:3 1 UDP 41885439 104.30.1.1 3478 typ relay"
            )
            .as_deref(),
            Some("a=candidate:3 1 UDP 41885439 104.30.1.1 3478 typ relay")
        );
        assert_eq!(
            filter_relay_only(
                SignalKind::Description,
                "a=candidate:3 1 UDP 1 104.30.1.1 3478 typ relay"
            ),
            None
        );
        assert_eq!(filter_relay_only(SignalKind::GatheringDone, "").as_deref(), Some(""));
        // "typ relayed" is not a relay candidate
        assert_eq!(
            filter_relay_only(SignalKind::Candidate, "candidate:3 1 UDP 1 1.2.3.4 9 typ relayed"),
            None
        );
    }

    #[test]
    fn hub_members_come_from_presence_rows() {
        let hub = HuntSnapshot {
            kind: SessionKind::RealmHub,
            roster: Vec::new(),
            realm: Some("realm-1".to_owned()),
            ..hunt()
        };
        let row = |id: &str, peer: u64, state: PresenceState| PresenceRow {
            account: account(id),
            peer: PeerId(peer),
            state,
        };
        let rows = vec![
            row(HOST, 1001, PresenceState::Admitted),
            row(JOIN_A, 2002, PresenceState::Ticketed),
            row(JOIN_B, 2003, PresenceState::Lost),
            row(JOIN_A, 2002, PresenceState::Admitted),
        ];
        assert_eq!(resolve_role(&hub, &account(JOIN_A), Some(&rows)), Some(IceRole::Join));
        assert_eq!(resolve_role(&hub, &account(JOIN_B), Some(&rows)), None);
        assert_eq!(resolve_role(&hub, &account(JOIN_A), None), None);
        assert_eq!(active_joiners(&hub, Some(&rows)), [account(JOIN_A)]);
        assert_eq!(peer_id_of(&hub, &account(JOIN_A), Some(&rows)), Some(PeerId(2002)));
        assert_eq!(peer_id_of(&hub, &account(HOST), Some(&rows)), Some(PeerId(1001)));
        assert_eq!(peer_id_of(&hunt(), &account("nobody-here"), None), None);
    }
}
