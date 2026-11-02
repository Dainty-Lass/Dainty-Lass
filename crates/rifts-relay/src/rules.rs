// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Who may talk ICE with whom in a hunt or a realm hub, the per-pair keys, and the checks on the signals the relay
//! carries between them.

use base64::Engine as _;
use hmac::{Hmac, KeyInit as _, Mac as _};
use rifts_seam::{AccountId, PeerId, Secret, UnixMillis};
use serde_json::Value;
use sha2::Sha256;

use crate::wire::{js_trim, safe_integer};

pub const MAXIMUM_HUB_PEERS: usize = 64;
pub const MAX_SIGNAL_BYTES: usize = 4096;

// Umbrella spec D3: the Rustic label. The transport derives nothing itself; it takes the key from the config.
const PAIR_KEY_LABEL: &str = "rustic-ice-pair-v1";

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
    /// States in which the transports may still be connecting or connected.
    pub const fn is_ice_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::HostReady | Self::Admitting | Self::Running | Self::Draining | Self::Suspect
        )
    }

    pub const fn is_ticketable(self) -> bool {
        matches!(self, Self::HostReady | Self::Admitting | Self::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionKind {
    Hunt,
    RealmHub,
}

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

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ICE" => Some(Self::Ice),
            "STEAM_EXPERIMENTAL" => Some(Self::SteamExperimental),
            _ => None,
        }
    }
}

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

    fn parse(text: &str) -> Option<Self> {
        [Self::Description, Self::Candidate, Self::GatheringDone]
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub account: AccountId,
    pub peer: PeerId,
    pub returned_to_hub: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceRow {
    pub account: AccountId,
    pub peer: PeerId,
    pub state: PresenceState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntSnapshot {
    pub wire_session: String,
    pub generation: u64,
    pub state: HuntState,
    pub kind: SessionKind,
    pub realm: Option<String>,
    /// Frozen when the hunt was created.
    pub coop_transport: CoopTransport,
    pub host_account: AccountId,
    pub host_peer: PeerId,
    pub expires_at: UnixMillis,
    pub roster: Vec<RosterEntry>,
}

impl HuntSnapshot {
    pub fn is_hub(&self) -> bool {
        self.kind == SessionKind::RealmHub
    }

    pub fn is_realm_session(&self) -> bool {
        self.realm.is_some()
    }

    fn rostered(&self, account: &AccountId) -> Option<&RosterEntry> {
        self.roster
            .iter()
            .find(|entry| &entry.account == account && !entry.returned_to_hub)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcePeer {
    pub account: AccountId,
    pub role: IceRole,
    pub transport_key: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceSignal {
    pub generation: i64,
    pub to: AccountId,
    pub kind: SignalKind,
    pub data: String,
}

/// A hub member with only a lost presence row has no role: it must enter the hub again first.
pub fn resolve_role(hunt: &HuntSnapshot, account: &AccountId, presence: Option<&[PresenceRow]>) -> Option<IceRole> {
    if account == &hunt.host_account {
        return Some(IceRole::Host);
    }
    let joined = if hunt.is_hub() {
        presence.is_some_and(|rows| rows.iter().any(|row| &row.account == account && row.state.is_live()))
    } else {
        hunt.rostered(account).is_some()
    };
    joined.then_some(IceRole::Join)
}

pub fn active_joiners(hunt: &HuntSnapshot, presence: Option<&[PresenceRow]>) -> Vec<AccountId> {
    let mut joiners: Vec<AccountId> = Vec::new();
    if hunt.is_hub() {
        for row in presence.unwrap_or_default() {
            if joiners.len() == MAXIMUM_HUB_PEERS {
                break;
            }
            if row.state.is_live() && row.account != hunt.host_account && !joiners.contains(&row.account) {
                joiners.push(row.account.clone());
            }
        }
    } else {
        joiners.extend(
            hunt.roster
                .iter()
                .filter(|entry| entry.account != hunt.host_account && !entry.returned_to_hub)
                .map(|entry| entry.account.clone()),
        );
    }
    joiners
}

pub fn peer_id_of(hunt: &HuntSnapshot, account: &AccountId, presence: Option<&[PresenceRow]>) -> Option<PeerId> {
    if account == &hunt.host_account {
        return Some(hunt.host_peer);
    }
    if hunt.is_hub() {
        presence?.iter().find(|row| &row.account == account).map(|row| row.peer)
    } else {
        hunt.roster
            .iter()
            .find(|entry| &entry.account == account)
            .map(|entry| entry.peer)
    }
}

/// The peers `account` connects to: every active joiner for the host, the host alone for a joiner.
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
        IceRole::Join => {
            vec![IcePeer {
                account: hunt.host_account.clone(),
                role: IceRole::Host,
                transport_key: key(account),
            }]
        }
    }
}

/// The key one host and one joiner share for one hunt generation; derived on demand, never stored.
pub fn derive_pair_key(
    secret: &Secret,
    wire_session: &str,
    generation: u64,
    host: &AccountId,
    joiner: &AccountId,
) -> Secret {
    let message = format!(
        "{PAIR_KEY_LABEL}\n{wire_session}\n{generation}\n{}\n{}",
        host.0, joiner.0
    );
    let tag = match <Hmac<Sha256>>::new_from_slice(secret.expose().as_bytes()) {
        Ok(mut mac) => {
            mac.update(message.as_bytes());
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        }
        // HMAC takes a key of any length.
        Err(_) => String::new(),
    };
    Secret::new(tag)
}

fn is_signal_account(text: &str) -> bool {
    (8..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// SDP attribute lines only: printable ASCII and line breaks.
fn is_signal_text(text: &str) -> bool {
    text.len() <= MAX_SIGNAL_BYTES
        && text
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) || b == b'\r' || b == b'\n')
}

pub fn parse_ice_signal(body: &Value) -> Option<IceSignal> {
    let generation = safe_integer(body.get("generation")?).filter(|generation| *generation >= 0)?;
    let to = body.get("toAccountId")?.as_str().filter(|to| is_signal_account(to))?;
    let kind = SignalKind::parse(body.get("kind")?.as_str()?)?;
    let data = match body.get("data") {
        None | Some(Value::Null) => "",
        Some(Value::String(data)) => data.as_str(),
        Some(_) => return None,
    };
    if !is_signal_text(data) || (kind != SignalKind::GatheringDone && js_trim(data).is_empty()) {
        return None;
    }
    Some(IceSignal {
        generation,
        to: AccountId(to.to_owned()),
        kind,
        data: data.to_owned(),
    })
}

/// Host to joiner and back; never joiner to joiner, never to oneself.
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
        (Some(a), Some(b)) => (a == IceRole::Host) != (b == IceRole::Host),
        _ => false,
    }
}

fn is_relay_candidate(line: &str) -> bool {
    line.match_indices(" typ relay")
        .any(|(at, found)| matches!(line.as_bytes().get(at + found.len()), None | Some(b' ')))
}

fn is_candidate_line(line: &str) -> bool {
    line.strip_prefix("a=").unwrap_or(line).starts_with("candidate:")
}

/// Relay-only mode: drops every candidate that would reveal a host or reflexive address.
pub fn filter_relay_only(kind: SignalKind, data: &str) -> Option<String> {
    if kind == SignalKind::GatheringDone {
        return Some(data.to_owned());
    }
    let separator = if data.contains("\r\n") { "\r\n" } else { "\n" };
    let kept: Vec<&str> = data
        .split('\n')
        .map(|piece| js_trim(piece.strip_suffix('\r').unwrap_or(piece)))
        .filter(|line| !line.is_empty())
        .filter(|line| {
            if is_candidate_line(line) {
                is_relay_candidate(line)
            } else {
                kind == SignalKind::Description
                    && (line.starts_with("a=ice-ufrag:")
                        || line.starts_with("a=ice-pwd:")
                        || line.starts_with("a=ice-options:")
                        || *line == "a=end-of-candidates")
            }
        })
        .collect();
    match kind {
        SignalKind::Candidate => match kept.as_slice() {
            [line] => Some((*line).to_owned()),
            _ => None,
        },
        _ => kept
            .iter()
            .any(|line| line.starts_with("a=ice-ufrag:"))
            .then(|| kept.join(separator)),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn id(name: &str) -> AccountId {
        AccountId(name.to_owned())
    }

    pub(crate) fn hunt() -> HuntSnapshot {
        HuntSnapshot {
            wire_session: "01234567-89ab-4def-8123-456789abcdef".to_owned(),
            generation: 3,
            state: HuntState::Running,
            kind: SessionKind::Hunt,
            realm: None,
            coop_transport: CoopTransport::Ice,
            host_account: id("host-0001"),
            host_peer: PeerId(11),
            expires_at: UnixMillis(0),
            roster: vec![
                RosterEntry {
                    account: id("host-0001"),
                    peer: PeerId(11),
                    returned_to_hub: false,
                },
                RosterEntry {
                    account: id("joiner-01"),
                    peer: PeerId(12),
                    returned_to_hub: false,
                },
                RosterEntry {
                    account: id("joiner-02"),
                    peer: PeerId(13),
                    returned_to_hub: true,
                },
                RosterEntry {
                    account: id("joiner-03"),
                    peer: PeerId(14),
                    returned_to_hub: false,
                },
            ],
        }
    }

    fn secret() -> Secret {
        Secret::new("s".repeat(32))
    }

    #[test]
    fn roles_in_a_hunt() {
        let hunt = hunt();
        assert_eq!(resolve_role(&hunt, &id("host-0001"), None), Some(IceRole::Host));
        assert_eq!(resolve_role(&hunt, &id("joiner-01"), None), Some(IceRole::Join));
        assert_eq!(resolve_role(&hunt, &id("joiner-02"), None), None);
        assert_eq!(resolve_role(&hunt, &id("stranger"), None), None);
        assert_eq!(active_joiners(&hunt, None), [id("joiner-01"), id("joiner-03")]);
        assert_eq!(peer_id_of(&hunt, &id("joiner-03"), None), Some(PeerId(14)));
        assert_eq!(peer_id_of(&hunt, &id("host-0001"), None), Some(PeerId(11)));
        assert_eq!(peer_id_of(&hunt, &id("stranger"), None), None);
    }

    #[test]
    fn hub_members_come_from_presence() {
        let mut hub = hunt();
        hub.kind = SessionKind::RealmHub;
        hub.roster.clear();
        let rows = [
            PresenceRow {
                account: id("member-01"),
                peer: PeerId(21),
                state: PresenceState::Admitted,
            },
            PresenceRow {
                account: id("member-02"),
                peer: PeerId(22),
                state: PresenceState::Lost,
            },
            PresenceRow {
                account: id("member-01"),
                peer: PeerId(23),
                state: PresenceState::Ticketed,
            },
            PresenceRow {
                account: id("host-0001"),
                peer: PeerId(11),
                state: PresenceState::Admitted,
            },
            PresenceRow {
                account: id("member-03"),
                peer: PeerId(24),
                state: PresenceState::Pending,
            },
        ];
        assert_eq!(resolve_role(&hub, &id("member-01"), Some(&rows)), Some(IceRole::Join));
        assert_eq!(resolve_role(&hub, &id("member-02"), Some(&rows)), None);
        assert_eq!(resolve_role(&hub, &id("member-01"), None), None);
        assert_eq!(active_joiners(&hub, Some(&rows)), [id("member-01"), id("member-03")]);
        assert_eq!(peer_id_of(&hub, &id("member-02"), Some(&rows)), Some(PeerId(22)));
        assert_eq!(peer_id_of(&hub, &id("member-01"), None), None);

        let many: Vec<PresenceRow> = (0..70)
            .map(|n| PresenceRow {
                account: id(&format!("member-{n:03}")),
                peer: PeerId(n),
                state: PresenceState::Admitted,
            })
            .collect();
        assert_eq!(active_joiners(&hub, Some(&many)).len(), MAXIMUM_HUB_PEERS);
    }

    #[test]
    fn state_sets() {
        use HuntState::*;
        let all = [
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
        ];
        let active: Vec<_> = all.iter().copied().filter(|s| s.is_ice_active()).collect();
        assert_eq!(active, [Starting, HostReady, Admitting, Running, Draining, Suspect]);
        let ticketable: Vec<_> = all.iter().copied().filter(|s| s.is_ticketable()).collect();
        assert_eq!(ticketable, [HostReady, Admitting, Running]);
        assert_eq!(CoopTransport::parse("ICE"), Some(CoopTransport::Ice));
        assert_eq!(CoopTransport::parse("ice"), None);
        assert_eq!(CoopTransport::SteamExperimental.as_str(), "STEAM_EXPERIMENTAL");
        assert_eq!(PresenceState::Expired.as_str(), "EXPIRED");
        assert!(!PresenceState::Lost.is_live());
    }

    #[test]
    fn pair_keys() {
        let hunt = hunt();
        let host_side = ice_peers_for(&hunt, &id("host-0001"), IceRole::Host, &secret(), None);
        let joiner_side = ice_peers_for(&hunt, &id("joiner-01"), IceRole::Join, &secret(), None);
        assert_eq!(host_side.len(), 2);
        assert_eq!(host_side[0].account, id("joiner-01"));
        assert_eq!(host_side[0].role, IceRole::Join);
        assert_eq!(
            joiner_side,
            [IcePeer {
                account: id("host-0001"),
                role: IceRole::Host,
                transport_key: host_side[0].transport_key.clone()
            }]
        );
        assert_ne!(host_side[0].transport_key, host_side[1].transport_key);
        let key = derive_pair_key(&secret(), &hunt.wire_session, 3, &id("host-0001"), &id("joiner-01"));
        assert_eq!(key.expose().len(), 43);
        assert_eq!(key, host_side[0].transport_key);
        assert_ne!(
            key,
            derive_pair_key(&secret(), &hunt.wire_session, 4, &id("host-0001"), &id("joiner-01"))
        );
        assert!(!key.expose().contains(['+', '/', '=']));

        let mut mac = <Hmac<Sha256>>::new_from_slice(secret().expose().as_bytes()).unwrap();
        mac.update(format!("rustic-ice-pair-v1\n{}\n3\nhost-0001\njoiner-01", hunt.wire_session).as_bytes());
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        assert_eq!(key.expose(), expected);
    }

    #[test]
    fn signal_parsing() {
        let good = json!({"generation": 3, "toAccountId": "joiner-01", "kind": "CANDIDATE",
            "data": "a=candidate:1 1 udp 1 10.0.0.1 5000 typ relay"});
        let signal = parse_ice_signal(&good).unwrap();
        assert_eq!(
            (signal.generation, signal.to.0.as_str(), signal.kind),
            (3, "joiner-01", SignalKind::Candidate)
        );

        let done = parse_ice_signal(&json!({"generation": 0, "toAccountId": "joiner-01", "kind": "GATHERING_DONE"}));
        assert_eq!(done.unwrap().data, "");
        let null_data = json!({"generation": 1.0, "toAccountId": "joiner-01", "kind": "GATHERING_DONE", "data": null});
        assert!(parse_ice_signal(&null_data).is_some());

        let with = |field: &str, value: Value| {
            let mut body = good.clone();
            body[field] = value;
            parse_ice_signal(&body)
        };
        assert!(with("data", json!("café")).is_none());
        assert!(with("data", json!(5)).is_none());
        assert!(with("data", json!("  \r\n")).is_none());
        assert!(with("data", json!("a\tb")).is_none());
        assert!(with("data", json!("x".repeat(4097))).is_none());
        assert!(with("data", json!("x".repeat(4096))).is_some());
        assert!(with("generation", json!(-1)).is_none());
        assert!(with("generation", json!("3")).is_none());
        assert!(with("generation", json!(1.5)).is_none());
        assert!(with("toAccountId", json!("short")).is_none());
        assert!(with("toAccountId", json!("joiner_01")).is_none());
        assert!(with("toAccountId", json!("j".repeat(65))).is_none());
        assert!(with("kind", json!("candidate")).is_none());
        let mut blank = good.clone();
        blank["kind"] = json!("DESCRIPTION");
        blank["data"] = json!(" ");
        assert!(parse_ice_signal(&blank).is_none());
        assert!(parse_ice_signal(&json!([])).is_none());
    }

    #[test]
    fn routing_is_host_to_joiner_only() {
        let hunt = hunt();
        assert!(signal_route_allowed(&hunt, &id("host-0001"), &id("joiner-01"), None));
        assert!(signal_route_allowed(&hunt, &id("joiner-01"), &id("host-0001"), None));
        assert!(!signal_route_allowed(&hunt, &id("joiner-01"), &id("joiner-03"), None));
        assert!(!signal_route_allowed(&hunt, &id("host-0001"), &id("host-0001"), None));
        assert!(!signal_route_allowed(&hunt, &id("host-0001"), &id("joiner-02"), None));
        assert!(!signal_route_allowed(&hunt, &id("stranger"), &id("host-0001"), None));
    }

    const DESCRIPTION: &str = "v=0\r\na=ice-ufrag:abcd\r\na=ice-pwd:0123456789abcdefghijkl\r\na=ice-options:ice2\r\n\
        a=candidate:1 1 UDP 2122 192.168.1.2 5000 typ host\r\n\
        a=candidate:2 1 UDP 1686 203.0.113.9 5001 typ srflx raddr 192.168.1.2 rport 5000\r\n\
        a=candidate:3 1 UDP 16777215 198.51.100.7 49152 typ relay raddr 203.0.113.9 rport 5001\r\n\
        a=fingerprint:sha-256 AB\r\na=end-of-candidates\r\n";

    #[test]
    fn relay_only_filter() {
        assert_eq!(
            filter_relay_only(SignalKind::Description, DESCRIPTION).unwrap(),
            "a=ice-ufrag:abcd\r\na=ice-pwd:0123456789abcdefghijkl\r\na=ice-options:ice2\r\n\
             a=candidate:3 1 UDP 16777215 198.51.100.7 49152 typ relay raddr 203.0.113.9 rport 5001\r\n\
             a=end-of-candidates"
        );
        assert_eq!(
            filter_relay_only(SignalKind::Candidate, "a=candidate:1 1 UDP 2122 10.0.0.2 5000 typ host"),
            None
        );
        let relay = "a=candidate:3 1 UDP 1 198.51.100.7 49152 typ relay";
        assert_eq!(
            filter_relay_only(SignalKind::Candidate, &format!(" {relay} \n")).as_deref(),
            Some(relay)
        );
        assert_eq!(
            filter_relay_only(SignalKind::Candidate, "candidate:3 1 UDP 1 h 1 typ relayed"),
            None
        );
        assert_eq!(
            filter_relay_only(SignalKind::Candidate, &format!("{relay}\n{relay}")),
            None
        );
        assert_eq!(
            filter_relay_only(SignalKind::Candidate, &format!("a=ice-ufrag:x\n{relay}")).as_deref(),
            Some(relay)
        );
        assert_eq!(filter_relay_only(SignalKind::Description, relay), None);
        assert_eq!(
            filter_relay_only(SignalKind::Description, &format!("a=ice-ufrag:x\n{relay}\n")).as_deref(),
            Some(format!("a=ice-ufrag:x\n{relay}").as_str())
        );
        assert_eq!(
            filter_relay_only(SignalKind::GatheringDone, " raw ").as_deref(),
            Some(" raw ")
        );
    }
}
