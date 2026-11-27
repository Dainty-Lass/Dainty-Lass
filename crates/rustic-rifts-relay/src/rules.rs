// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Who may talk ICE with whom in a hunt or a realm hub, the per-pair keys, and the checks on the signals the relay
//! carries between them.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit as _, Mac as _};
use rustic_rifts_seam::{AccountId, PeerId, Secret, UnixMillis};
use serde_json::Value;

use crate::wire::{js_trim, safe_integer};

/// The most joiners a realm hub's host connects to.
pub const MAXIMUM_HUB_PEERS: usize = 64;
/// The longest signal payload accepted, in bytes.
pub const MAX_SIGNAL_BYTES: usize = 4096;

// Umbrella spec D3: the Rustic label. The transport derives nothing itself; it takes the key from the config.
const PAIR_KEY_LABEL: &str = "rustic-ice-pair-v1";

/// Where a player-hosted session is in its lifecycle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HuntState {
    /// Asked for; no host chosen yet.
    Requested,
    /// Choosing a host.
    HostSelecting,
    /// A host was offered the session and has not accepted yet.
    HostOffered,
    /// The host accepted and its runtime is starting.
    Starting,
    /// The host's runtime is ready for players.
    HostReady,
    /// Players are being admitted.
    Admitting,
    /// Under way with players in it.
    Running,
    /// A superseded Tier 0 hunt whose server still serves its players.
    Draining,
    /// The host's lease lapsed; waiting for it to come back.
    Suspect,
    /// The host was fenced out and may change nothing.
    Fenced,
    /// Being aborted.
    Aborting,
    /// Recovering after losing its host.
    Recovering,
    /// Ended normally.
    Completed,
    /// Ended without completing.
    Aborted,
}

impl HuntState {
    /// Whether players may be issued admission tickets in this state.
    pub const fn is_ticketable(self) -> bool {
        matches!(self, Self::HostReady | Self::Admitting | Self::Running)
    }

    /// States in which the transports may still be connecting or connected.
    pub const fn is_ice_active(self) -> bool {
        self.is_ticketable() || matches!(self, Self::Starting | Self::Draining | Self::Suspect)
    }
}

/// What a player-hosted session is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SessionKind {
    /// A hunt.
    Hunt,
    /// A realm's hub.
    RealmHub,
}

wire_enum! {
    /// How a session's co-op transport connects players.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    CoopTransport, read by pub parse {
        Ice = "ICE",
        SteamExperimental = "STEAM_EXPERIMENTAL",
    }
}

wire_enum! {
    /// Where a member's presence in a realm hub stands.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    PresenceState {
        Pending = "PENDING",
        Ticketed = "TICKETED",
        Admitted = "ADMITTED",
        Lost = "LOST",
        Left = "LEFT",
        Removed = "REMOVED",
        Expired = "EXPIRED",
    }
}

impl PresenceState {
    /// Whether the member still counts as present in the hub.
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Pending | Self::Ticketed | Self::Admitted)
    }
}

wire_enum! {
    /// Which side of a host-joiner pair a peer is.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    IceRole {
        Host = "HOST",
        Join = "JOIN",
    }
}

wire_enum! {
    /// What an ICE signal carries.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    SignalKind, read by parse {
        Description = "DESCRIPTION",
        Candidate = "CANDIDATE",
        GatheringDone = "GATHERING_DONE",
    }
}

/// One seat in a hunt's roster.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RosterEntry {
    /// The member.
    pub account: AccountId,
    /// The member's transport peer id.
    pub peer: PeerId,
    /// Whether the member went back to the hub, leaving the hunt.
    pub returned_to_hub: bool,
}

/// One member's presence in a realm hub.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresenceRow {
    /// The member.
    pub account: AccountId,
    /// The member's transport peer id.
    pub peer: PeerId,
    /// Where the member's presence stands.
    pub state: PresenceState,
}

/// What the relay needs to know of a hunt or hub session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HuntSnapshot {
    /// The session id as the wire spells it.
    pub wire_session: String,
    /// The session's current generation.
    pub generation: u64,
    /// Where the session is in its lifecycle.
    pub state: HuntState,
    /// Hunt or hub.
    pub kind: SessionKind,
    /// The realm the session belongs to, if any.
    pub realm: Option<String>,
    /// Frozen when the hunt was created.
    pub coop_transport: CoopTransport,
    /// The player hosting the session.
    pub host_account: AccountId,
    /// The host's transport peer id.
    pub host_peer: PeerId,
    /// When the session expires.
    pub expires_at: UnixMillis,
    /// The hunt's seats; empty for a hub, whose members are in its presence rows.
    pub roster: Vec<RosterEntry>,
}

impl HuntSnapshot {
    /// Whether the session is a realm hub.
    pub fn is_hub(&self) -> bool {
        matches!(self.kind, SessionKind::RealmHub)
    }

    /// Whether the session belongs to a realm.
    pub fn is_realm_session(&self) -> bool {
        self.realm.is_some()
    }

    /// Roster seats still in the hunt (a member who went back to the hub has left it).
    fn seated(&self) -> impl Iterator<Item = &RosterEntry> {
        self.roster.iter().filter(|seat| !seat.returned_to_hub)
    }
}

/// One peer a transport connects to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IcePeer {
    /// The peer's account.
    pub account: AccountId,
    /// The peer's role towards the caller.
    pub role: IceRole,
    /// The key the caller and this peer share.
    pub transport_key: Secret,
}

/// An ICE signal one peer sends another through the relay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IceSignal {
    /// The generation the signal is for.
    pub generation: i64,
    /// The account it is addressed to.
    pub to: AccountId,
    /// What it carries.
    pub kind: SignalKind,
    /// The payload: SDP text, empty for [`SignalKind::GatheringDone`].
    pub data: String,
}

fn live_rows(presence: Option<&[PresenceRow]>) -> impl Iterator<Item = &PresenceRow> {
    presence.unwrap_or_default().iter().filter(|row| row.state.is_live())
}

/// A hub member with only a lost presence row has no role: it must enter the hub again first.
pub fn resolve_role(hunt: &HuntSnapshot, account: &AccountId, presence: Option<&[PresenceRow]>) -> Option<IceRole> {
    if *account == hunt.host_account {
        Some(IceRole::Host)
    } else if hunt.is_hub() {
        live_rows(presence)
            .any(|row| row.account == *account)
            .then_some(IceRole::Join)
    } else {
        hunt.seated()
            .any(|seat| seat.account == *account)
            .then_some(IceRole::Join)
    }
}

/// The joiners the host connects to: the hunt's seated members, or at most [`MAXIMUM_HUB_PEERS`] live hub members.
pub fn active_joiners(hunt: &HuntSnapshot, presence: Option<&[PresenceRow]>) -> Vec<AccountId> {
    let not_host = |account: &&AccountId| **account != hunt.host_account;
    if !hunt.is_hub() {
        return hunt
            .seated()
            .map(|seat| &seat.account)
            .filter(not_host)
            .cloned()
            .collect();
    }
    let mut members: Vec<AccountId> = Vec::with_capacity(MAXIMUM_HUB_PEERS);
    for account in live_rows(presence).map(|row| &row.account).filter(not_host) {
        if members.len() == MAXIMUM_HUB_PEERS {
            break;
        }
        if !members.contains(account) {
            members.push(account.clone());
        }
    }
    members
}

/// The account's transport peer id in this session, if it has one.
pub fn peer_id_of(hunt: &HuntSnapshot, account: &AccountId, presence: Option<&[PresenceRow]>) -> Option<PeerId> {
    if *account == hunt.host_account {
        Some(hunt.host_peer)
    } else if hunt.is_hub() {
        let row = presence?.iter().find(|row| row.account == *account)?;
        Some(row.peer)
    } else {
        let seat = hunt.roster.iter().find(|seat| seat.account == *account)?;
        Some(seat.peer)
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
    let paired_with = |joiner: AccountId, peer: AccountId, role: IceRole| IcePeer {
        transport_key: derive_pair_key(secret, &hunt.wire_session, hunt.generation, &hunt.host_account, &joiner),
        account: peer,
        role,
    };
    if role == IceRole::Join {
        return vec![paired_with(account.clone(), hunt.host_account.clone(), IceRole::Host)];
    }
    active_joiners(hunt, presence)
        .into_iter()
        .map(|joiner| paired_with(joiner.clone(), joiner, IceRole::Join))
        .collect()
}

/// The key one host and one joiner share for one hunt generation; derived on demand, never stored.
pub fn derive_pair_key(
    secret: &Secret,
    wire_session: &str,
    generation: u64,
    host: &AccountId,
    joiner: &AccountId,
) -> Secret {
    let generation = generation.to_string();
    let lines = [PAIR_KEY_LABEL, wire_session, &generation, &host.0, &joiner.0];
    // HMAC takes a key of any length, so the empty fallback is never reached.
    let tag = <Hmac<sha2::Sha256>>::new_from_slice(secret.expose().as_bytes()).map_or_else(
        |_| String::new(),
        |mac| URL_SAFE_NO_PAD.encode(mac.chain_update(lines.join("\n")).finalize().into_bytes()),
    );
    Secret::new(tag)
}

/// Account ids a signal may be addressed to: 8 to 64 letters, digits and dashes.
fn addressable(text: &str) -> bool {
    matches!(text.len(), 8..=64) && text.chars().all(|c| c == '-' || c.is_ascii_alphanumeric())
}

/// SDP attribute lines only: printable ASCII and line breaks.
fn sdp_safe(text: &str) -> bool {
    text.len() <= MAX_SIGNAL_BYTES && text.chars().all(|c| matches!(c, ' '..='~' | '\r' | '\n'))
}

/// Reads an ICE signal from a request body; `None` when any field is missing or invalid.
pub fn parse_ice_signal(body: &Value) -> Option<IceSignal> {
    let generation = safe_integer(body.get("generation")?)?;
    let to = body.get("toAccountId")?.as_str()?;
    let kind = SignalKind::parse(body.get("kind")?.as_str()?)?;
    let data = match body.get("data") {
        Some(Value::String(text)) => text.as_str(),
        Some(Value::Null) | None => "",
        Some(_) => return None,
    };
    let blank_allowed = kind == SignalKind::GatheringDone;
    let valid = generation >= 0 && addressable(to) && sdp_safe(data) && (blank_allowed || !js_trim(data).is_empty());
    valid.then(|| IceSignal {
        to: AccountId(to.to_owned()),
        data: data.to_owned(),
        generation,
        kind,
    })
}

/// Host to joiner and back; never joiner to joiner, never to oneself.
pub fn signal_route_allowed(
    hunt: &HuntSnapshot,
    from: &AccountId,
    to: &AccountId,
    presence: Option<&[PresenceRow]>,
) -> bool {
    let roles = [from, to].map(|account| resolve_role(hunt, account, presence));
    from != to
        && matches!(
            roles,
            [Some(IceRole::Host), Some(IceRole::Join)] | [Some(IceRole::Join), Some(IceRole::Host)]
        )
}

/// A candidate whose type is `relay`: ` typ relay` ending the line or followed by a space.
fn relayed(line: &str) -> bool {
    const MARK: &str = " typ relay";
    line.match_indices(MARK)
        .map(|(at, _)| &line[at + MARK.len()..])
        .any(|after| after.is_empty() || after.starts_with(' '))
}

fn candidate(line: &str) -> bool {
    line.strip_prefix("a=").unwrap_or(line).starts_with("candidate:")
}

/// The description lines relay-only mode passes besides candidates: ICE credentials and options.
fn ice_attribute(line: &str) -> bool {
    ["a=ice-ufrag:", "a=ice-pwd:", "a=ice-options:"]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        || line == "a=end-of-candidates"
}

/// Relay-only mode: drops every candidate that would reveal a host or reflexive address.
pub fn filter_relay_only(kind: SignalKind, data: &str) -> Option<String> {
    // No transport reads this kind's data, so none is passed on: it could carry any candidate line.
    if kind == SignalKind::GatheringDone {
        return Some(String::new());
    }
    let mut kept: Vec<&str> = Vec::new();
    for raw in data.split('\n') {
        let line = js_trim(raw.strip_suffix('\r').unwrap_or(raw));
        let keep = if candidate(line) {
            relayed(line)
        } else {
            kind == SignalKind::Description && ice_attribute(line)
        };
        if keep && !line.is_empty() {
            kept.push(line);
        }
    }
    if kind == SignalKind::Candidate {
        return match kept[..] {
            [only] => Some(only.to_owned()),
            _ => None,
        };
    }
    let has_credentials = kept.iter().any(|line| line.starts_with("a=ice-ufrag:"));
    let line_break = if data.contains("\r\n") { "\r\n" } else { "\n" };
    has_credentials.then(|| kept.join(line_break))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sha2::Sha256;

    use super::*;

    fn id(name: &str) -> AccountId {
        AccountId(name.to_owned())
    }

    fn seat(name: &str, peer: u64, returned_to_hub: bool) -> RosterEntry {
        RosterEntry {
            peer: PeerId(peer),
            account: id(name),
            returned_to_hub,
        }
    }

    fn row(name: &str, peer: u64, state: PresenceState) -> PresenceRow {
        PresenceRow {
            state,
            peer: PeerId(peer),
            account: id(name),
        }
    }

    pub(crate) fn hunt() -> HuntSnapshot {
        HuntSnapshot {
            roster: vec![
                seat("host-0001", 11, false),
                seat("joiner-01", 12, false),
                seat("joiner-02", 13, true),
                seat("joiner-03", 14, false),
            ],
            host_account: id("host-0001"),
            host_peer: PeerId(11),
            wire_session: "01234567-89ab-4def-8123-456789abcdef".to_owned(),
            generation: 3,
            state: HuntState::Running,
            kind: SessionKind::Hunt,
            coop_transport: CoopTransport::Ice,
            realm: None,
            expires_at: UnixMillis(0),
        }
    }

    fn secret() -> Secret {
        Secret::new("s".repeat(32))
    }

    #[test]
    fn roles_in_a_hunt() {
        let hunt = hunt();
        let role = |name: &str| resolve_role(&hunt, &id(name), None);
        assert_eq!(
            ["host-0001", "joiner-01", "joiner-02", "stranger"].map(role),
            [Some(IceRole::Host), Some(IceRole::Join), None, None]
        );
        assert_eq!(active_joiners(&hunt, None), [id("joiner-01"), id("joiner-03")]);
        let peer = |name: &str| peer_id_of(&hunt, &id(name), None);
        assert_eq!(
            ["joiner-03", "host-0001", "stranger"].map(peer),
            [Some(PeerId(14)), Some(PeerId(11)), None]
        );
    }

    #[test]
    fn hub_members_come_from_presence() {
        let hub = HuntSnapshot {
            kind: SessionKind::RealmHub,
            roster: Vec::new(),
            ..hunt()
        };
        let rows = [
            row("member-01", 21, PresenceState::Admitted),
            row("member-02", 22, PresenceState::Lost),
            row("member-01", 23, PresenceState::Ticketed),
            row("host-0001", 11, PresenceState::Admitted),
            row("member-03", 24, PresenceState::Pending),
        ];
        assert_eq!(resolve_role(&hub, &id("member-01"), Some(&rows)), Some(IceRole::Join));
        assert_eq!(resolve_role(&hub, &id("member-02"), Some(&rows)), None);
        assert_eq!(resolve_role(&hub, &id("member-01"), None), None);
        assert_eq!(active_joiners(&hub, Some(&rows)), [id("member-01"), id("member-03")]);
        assert_eq!(peer_id_of(&hub, &id("member-02"), Some(&rows)), Some(PeerId(22)));
        assert_eq!(peer_id_of(&hub, &id("member-01"), None), None);

        let crowd: Vec<PresenceRow> = (0..70)
            .map(|n| row(&format!("member-{n:03}"), n, PresenceState::Admitted))
            .collect();
        assert_eq!(active_joiners(&hub, Some(&crowd)).len(), MAXIMUM_HUB_PEERS);
    }

    #[test]
    fn state_sets() {
        use HuntState::*;
        let every = [
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
        let pick = |test: fn(HuntState) -> bool| every.into_iter().filter(|&state| test(state)).collect::<Vec<_>>();
        assert_eq!(
            pick(HuntState::is_ice_active),
            [Starting, HostReady, Admitting, Running, Draining, Suspect]
        );
        assert_eq!(pick(HuntState::is_ticketable), [HostReady, Admitting, Running]);
        assert_eq!(
            (CoopTransport::parse("ICE"), CoopTransport::parse("ice")),
            (Some(CoopTransport::Ice), None)
        );
        assert_eq!(CoopTransport::SteamExperimental.as_str(), "STEAM_EXPERIMENTAL");
        assert_eq!(PresenceState::Expired.as_str(), "EXPIRED");
        assert!(!PresenceState::Lost.is_live());
    }

    #[test]
    fn pair_keys() {
        let hunt = hunt();
        let host_side = ice_peers_for(&hunt, &id("host-0001"), IceRole::Host, &secret(), None);
        let joiner_side = ice_peers_for(&hunt, &id("joiner-01"), IceRole::Join, &secret(), None);
        let first = &host_side[0];
        assert_eq!(
            (host_side.len(), &first.account, first.role),
            (2, &id("joiner-01"), IceRole::Join)
        );
        let expected_joiner_side = IcePeer {
            account: id("host-0001"),
            role: IceRole::Host,
            transport_key: first.transport_key.clone(),
        };
        assert_eq!(joiner_side, [expected_joiner_side]);
        assert_ne!(first.transport_key, host_side[1].transport_key);

        let key_at = |generation| {
            derive_pair_key(
                &secret(),
                &hunt.wire_session,
                generation,
                &id("host-0001"),
                &id("joiner-01"),
            )
        };
        let key = key_at(3);
        assert_eq!(key.expose().len(), 43);
        assert_eq!(key, first.transport_key);
        assert_ne!(key, key_at(4));
        assert!(!key.expose().contains(['+', '/', '=']));

        let message = format!("rustic-ice-pair-v1\n{}\n3\nhost-0001\njoiner-01", hunt.wire_session);
        let mac = <Hmac<Sha256>>::new_from_slice(secret().expose().as_bytes())
            .unwrap()
            .chain_update(message.as_bytes());
        assert_eq!(key.expose(), URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()));
    }

    #[test]
    fn signal_parsing() {
        let good = json!({"generation": 3, "toAccountId": "joiner-01", "kind": "CANDIDATE",
            "data": "a=candidate:1 1 udp 1 10.0.0.1 5000 typ relay"});
        let parsed = parse_ice_signal(&good).unwrap();
        assert_eq!(
            (parsed.generation, parsed.to.0.as_str(), parsed.kind),
            (3, "joiner-01", SignalKind::Candidate)
        );

        let done = parse_ice_signal(&json!({"generation": 0, "toAccountId": "joiner-01", "kind": "GATHERING_DONE"}));
        assert_eq!(done.unwrap().data, "");
        let null_data = json!({"generation": 1.0, "toAccountId": "joiner-01", "kind": "GATHERING_DONE", "data": null});
        assert!(parse_ice_signal(&null_data).is_some());

        let changed = |edits: &[(&str, Value)]| {
            let mut body = good.clone();
            for (field, value) in edits {
                body[*field] = value.clone();
            }
            parse_ice_signal(&body)
        };
        let refused = [
            ("data", json!("café")),
            ("data", json!(5)),
            ("data", json!("  \r\n")),
            ("data", json!("a\tb")),
            ("data", json!("x".repeat(4097))),
            ("generation", json!(-1)),
            ("generation", json!("3")),
            ("generation", json!(1.5)),
            ("toAccountId", json!("short")),
            ("toAccountId", json!("joiner_01")),
            ("toAccountId", json!("j".repeat(65))),
            ("kind", json!("candidate")),
        ];
        for (field, value) in refused {
            assert!(changed(&[(field, value.clone())]).is_none(), "{field} = {value}");
        }
        assert!(changed(&[("data", json!("x".repeat(4096)))]).is_some());
        assert!(changed(&[("kind", json!("DESCRIPTION")), ("data", json!(" "))]).is_none());
        assert!(parse_ice_signal(&json!([])).is_none());
    }

    #[test]
    fn routing_is_host_to_joiner_only() {
        let hunt = hunt();
        let allowed = |from: &str, to: &str| signal_route_allowed(&hunt, &id(from), &id(to), None);
        for (from, to, expected) in [
            ("host-0001", "joiner-01", true),
            ("joiner-01", "host-0001", true),
            ("joiner-01", "joiner-03", false),
            ("host-0001", "host-0001", false),
            ("host-0001", "joiner-02", false),
            ("stranger", "host-0001", false),
        ] {
            assert_eq!(allowed(from, to), expected, "{from} -> {to}");
        }
    }

    const DESCRIPTION: &str = "v=0\r\na=ice-ufrag:abcd\r\na=ice-pwd:0123456789abcdefghijkl\r\na=ice-options:ice2\r\n\
        a=candidate:1 1 UDP 2122 192.168.1.2 5000 typ host\r\n\
        a=candidate:2 1 UDP 1686 203.0.113.9 5001 typ srflx raddr 192.168.1.2 rport 5000\r\n\
        a=candidate:3 1 UDP 16777215 198.51.100.7 49152 typ relay raddr 203.0.113.9 rport 5001\r\n\
        a=fingerprint:sha-256 AB\r\na=end-of-candidates\r\n";

    #[test]
    fn relay_only_filter() {
        let relay = "a=candidate:3 1 UDP 1 198.51.100.7 49152 typ relay";
        let description_kept = "a=ice-ufrag:abcd\r\na=ice-pwd:0123456789abcdefghijkl\r\na=ice-options:ice2\r\n\
             a=candidate:3 1 UDP 16777215 198.51.100.7 49152 typ relay raddr 203.0.113.9 rport 5001\r\n\
             a=end-of-candidates";
        let cases: [(SignalKind, String, Option<String>); 10] = [
            (
                SignalKind::Description,
                DESCRIPTION.to_owned(),
                Some(description_kept.to_owned()),
            ),
            (
                SignalKind::Candidate,
                "a=candidate:1 1 UDP 2122 10.0.0.2 5000 typ host".to_owned(),
                None,
            ),
            (SignalKind::Candidate, format!(" {relay} \n"), Some(relay.to_owned())),
            (
                SignalKind::Candidate,
                "candidate:3 1 UDP 1 h 1 typ relayed".to_owned(),
                None,
            ),
            (SignalKind::Candidate, format!("{relay}\n{relay}"), None),
            (
                SignalKind::Candidate,
                format!("a=ice-ufrag:x\n{relay}"),
                Some(relay.to_owned()),
            ),
            (SignalKind::Description, relay.to_owned(), None),
            (
                SignalKind::Description,
                format!("a=ice-ufrag:x\n{relay}\n"),
                Some(format!("a=ice-ufrag:x\n{relay}")),
            ),
            (SignalKind::GatheringDone, " raw ".to_owned(), Some(String::new())),
            (
                SignalKind::GatheringDone,
                "candidate:1 1 udp 1 192.168.1.9 5000 typ host".to_owned(),
                Some(String::new()),
            ),
        ];
        for (kind, data, expected) in cases {
            assert_eq!(filter_relay_only(kind, &data), expected, "{kind:?} {data:?}");
        }
    }
}
