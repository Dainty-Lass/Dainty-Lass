// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The pure rules (spec §3.6, §5.4-§5.6, §7, §9): hunt ids, the party a join travels with, the Go Now window, the
//! client address, the party id and the QoS plan.

use std::net::IpAddr;

use base64::Engine as _;
use module_api::GlobalId;
use rustic_isles::Settings;
use serde_json::Value;

/// The hub every return travels to.
pub const RAMSGATE_HUNT: &str = "ShatteredIsles_ReturnToRamsgate";
const ISLAND_A: &str = "ShatteredIsles_IslandA";
const DIA_MOSS_MAP: &str = "/Game/Maps/islands/1705/dia_moss_triforce";

/// `GetFallbackHuntId`: a blank hunt id stays as given unless the mode or the map names the destination.
pub fn effective_hunt_id(mode: Option<&str>, args: Option<&str>, hunt: Option<&str>) -> String {
    if let Some(hunt) = hunt.filter(|h| !service_kit::js::trim(h).is_empty()) {
        return hunt.to_owned();
    }
    if mode == Some("CITY") {
        return RAMSGATE_HUNT.to_owned();
    }
    if args.is_some_and(|a| a.contains(DIA_MOSS_MAP)) {
        return ISLAND_A.to_owned();
    }
    hunt.unwrap_or("").to_owned()
}

pub fn needs_queue(hunt: &str) -> bool {
    hunt.contains("CR19")
}

pub fn is_hub_hunt_id(hunt: &str) -> bool {
    hunt.contains("ReturnToRamsgate") || hunt.contains("TrainingDojo")
}

pub fn is_hub_destination(mode: Option<&str>, hunt: &str) -> bool {
    mode == Some("CITY") || is_hub_hunt_id(hunt)
}

pub fn is_gauntlet_hunt_id(hunt: &str) -> bool {
    hunt.to_ascii_lowercase().contains("gauntlet")
}

/// `RUSTIC_ISLES_GAUNTLET` reads `on`, trimmed, any case.
pub fn gauntlet_enabled(settings: &Settings) -> bool {
    settings
        .raw("RUSTIC_ISLES_GAUNTLET")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("on"))
}

/// The resolution's answer (`ResolveMatchmakingParty`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartyResolution {
    pub party_id: Option<String>,
    pub members: Option<Vec<String>>,
    pub revision: Option<i64>,
    pub excluded: Vec<String>,
    pub mismatch: bool,
}

/// The party as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvableParty<'a> {
    pub party_id: &'a str,
    pub members: &'a [String],
    pub revision: i64,
}

/// An ISLAND request names the party the client travels with, so a stale server party never turns a solo hunt into a
/// two-player server whose airship waits for ever; CITY omits it and keeps the authoritative party.
pub fn resolve_matchmaking_party(
    mode: Option<&str>,
    requester: &str,
    requested_party_id: Option<&str>,
    party: Option<ResolvableParty<'_>>,
    online: impl Fn(&str) -> bool,
    filter_offline: bool,
) -> PartyResolution {
    let Some(party) = party else {
        return PartyResolution::default();
    };
    let island = mode == Some("ISLAND");
    if island && requested_party_id != Some(party.party_id) {
        return PartyResolution {
            excluded: party.members.iter().filter(|m| *m != requester).cloned().collect(),
            mismatch: true,
            ..PartyResolution::default()
        };
    }
    let filter = island || filter_offline;
    let (kept, excluded): (Vec<String>, Vec<String>) = party
        .members
        .iter()
        .cloned()
        .partition(|m| !filter || m == requester || online(m));
    PartyResolution {
        party_id: Some(party.party_id.to_owned()),
        members: Some(kept),
        revision: Some(party.revision),
        excluded,
        mismatch: false,
    }
}

/// JavaScript's `Number` of a trimmed text, for the characters a delay may hold; anything else is NaN.
pub fn js_number(raw: &str) -> f64 {
    let text = raw.trim();
    if text.is_empty() {
        return 0.0;
    }
    let numeric = text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-' | b'e' | b'E'));
    if !numeric {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// `ReadDelayMs`: floored, and only a finite value above 0 counts, so `0` reads as the default (spec §15 item 6).
pub fn read_delay_ms(raw: Option<&str>, default: i64) -> i64 {
    let value = raw.map_or(f64::NAN, js_number).floor();
    if value.is_finite() && value > 0.0 {
        // Saturates on a huge value
        value as i64
    } else {
        default
    }
}

/// An integer in `[low, high_exclusive)` from the next replay UUID's 128 bits; `low` when the span is empty.
pub fn draw(low: i64, high_exclusive: i64) -> i64 {
    let span = i128::from(high_exclusive) - i128::from(low);
    if span <= 0 {
        return low;
    }
    let bits = uuid_bits(&rustic_isles::replay::next_uuid());
    let offset = (bits % span as u128) as i128;
    i64::try_from(i128::from(low) + offset).unwrap_or(low)
}

fn uuid_bits(uuid: &str) -> u128 {
    let hex: String = uuid.chars().filter(char::is_ascii_hexdigit).collect();
    u128::from_str_radix(&hex, 16).unwrap_or(0)
}

/// Which window a release is drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseKind {
    Hub,
    Hunt,
}

/// The Go Now settings, read once per draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoNowWindow {
    pub min: i64,
    pub max: i64,
    pub per_joiner: i64,
    pub cap: i64,
}

impl GoNowWindow {
    pub fn from_settings(settings: &Settings, kind: ReleaseKind) -> Self {
        let read = |name: &str, default: i64| read_delay_ms(settings.raw(name), default);
        let (min, max) = match kind {
            ReleaseKind::Hub => (
                read("HUB_GO_NOW_MIN_DELAY_MS", 0),
                read("HUB_GO_NOW_MAX_DELAY_MS", 1000),
            ),
            ReleaseKind::Hunt => (
                read("HUNT_GO_NOW_MIN_DELAY_MS", 10_000),
                read("HUNT_GO_NOW_MAX_DELAY_MS", 20_000),
            ),
        };
        Self {
            min,
            max,
            per_joiner: read("GO_NOW_LOAD_PADDING_PER_JOINER_MS", 1000),
            cap: read("GO_NOW_LOAD_PADDING_MAX_MS", 20_000),
        }
    }

    /// `CreateGoNowReleaseAt`: hubs only spread a burst of returning players; a hunt's window masks its server's
    /// spin-up. Padding grows with the releases still in flight.
    pub fn release_at(&self, now: i64, in_flight: usize) -> i64 {
        let in_flight = i64::try_from(in_flight).unwrap_or(i64::MAX);
        let padding = in_flight.saturating_mul(self.per_joiner).min(self.cap);
        let low = self.min.min(self.max);
        let high = low.max(self.max).saturating_add(padding);
        now.saturating_add(draw(low, high.saturating_add(1)))
    }
}

fn is_loopback(entry: &str) -> bool {
    let bare = entry.strip_prefix("::ffff:").unwrap_or(entry);
    bare.parse::<IpAddr>().is_ok_and(|ip| match ip {
        IpAddr::V4(v4) => v4.octets()[0] == 127,
        IpAddr::V6(v6) => v6.is_loopback(),
    })
}

/// The backend's own reading for matchmaking (spec §3.6, §15 item 9): from the right, the first entry that is not
/// loopback; the leftmost when all are.
pub fn client_address(peer: Option<&str>, forwarded: Option<&str>) -> Option<String> {
    let peer = peer?;
    let mut entries: Vec<&str> = forwarded
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .collect();
    entries.push(peer);
    let found = entries.iter().rev().find(|e| !is_loopback(e)).or(entries.first());
    found.map(|e| (*e).to_owned())
}

/// Where the client travels: a client behind the hub's own public address cannot count on NAT hairpin for UDP, so
/// a local client goes to loopback.
pub fn travel_host(client: Option<&str>, candidate_host: &str, ring_assigned: bool) -> (String, bool) {
    let address = client.map(|a| a.strip_prefix("::ffff:").unwrap_or(a)).unwrap_or("");
    let loopback_client = address == "127.0.0.1" || address == "::1";
    let local = !address.is_empty()
        && !candidate_host.is_empty()
        && (address == candidate_host || (loopback_client && (!ring_assigned || candidate_host == "127.0.0.1")));
    if local {
        ("127.0.0.1".to_owned(), true)
    } else {
        (candidate_host.to_owned(), false)
    }
}

/// `MakePartyId`: chat's room spelling, `_`, and the build id in padded base64.
pub fn make_party_id(id: &GlobalId, build_id: &str) -> String {
    let build = base64::engine::general_purpose::STANDARD.encode(build_id);
    format!("{}_{build}", rustic_isles_chat::ids::room_id_text(id))
}

/// The QoS listener's plan: the public port, or none with the problem when the value is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QosPlan {
    pub port: Option<u16>,
    pub problem: Option<String>,
}

/// `QOS_PORT`: blank is the version's default, `off` or `0` starts none (the host kit's spelling when the launcher's
/// relay answers the pings), digits from 1 to 65535 are a port.
pub fn qos_plan(raw: Option<&str>, default: &str) -> QosPlan {
    let given = raw.map(str::trim).filter(|v| !v.is_empty());
    let value = given.unwrap_or_else(|| default.trim());
    let value = if value.is_empty() { "80" } else { value };
    if value.eq_ignore_ascii_case("off") || value == "0" {
        return QosPlan {
            port: None,
            problem: None,
        };
    }
    let port = value
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| value.parse::<u16>().ok())
        .flatten()
        .filter(|p| *p > 0);
    match port {
        Some(port) => QosPlan {
            port: Some(port),
            problem: None,
        },
        None => QosPlan {
            port: None,
            problem: Some(format!("QOS_PORT='{value}' is not a port number or 'off'")),
        },
    }
}

/// A client text for a log line: cut to 120 characters, control characters escaped (a departure, spec §15 item 13).
pub fn log_text(text: &str) -> String {
    text.chars()
        .take(120)
        .flat_map(|c| {
            let escaped: Vec<char> = if c.is_control() {
                c.escape_default().collect()
            } else {
                vec![c]
            };
            escaped
        })
        .collect()
}

/// A body field's shape in the join log: a string's first 40 characters quoted, a boolean or number as JSON.
pub fn field_shape(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => {
            let cut: String = text.chars().take(40).collect();
            service_kit::jsjson::stringify(&Value::String(cut))
        }
        Some(value @ (Value::Bool(_) | Value::Number(_))) => service_kit::jsjson::stringify(value),
        _ => "absent".to_owned(),
    }
}

/// The leave's logged key names: the first 12, each cut to 40 characters, anything unusual replaced by `?`.
pub fn body_keys(body: Option<&Value>) -> String {
    let Some(Value::Object(object)) = body else {
        return String::new();
    };
    object
        .keys()
        .take(12)
        .map(|key| {
            key.chars()
                .take(40)
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                        c
                    } else {
                        '?'
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use module_api::{IdKind, NodeId};

    #[test]
    fn effective_hunt_ids() {
        let e = |mode, args, hunt| effective_hunt_id(mode, args, hunt);
        assert_eq!(e(Some("CITY"), None, None), RAMSGATE_HUNT);
        assert_eq!(e(Some("CITY"), None, Some("  ")), RAMSGATE_HUNT);
        assert_eq!(e(Some("ISLAND"), None, Some("X")), "X");
        let args = format!("?map={DIA_MOSS_MAP}");
        assert_eq!(e(Some("ISLAND"), Some(&args), None), ISLAND_A);
        assert_eq!(e(Some("ISLAND"), None, Some("  ")), "  ");
        assert_eq!(e(None, None, None), "");
    }

    #[test]
    fn hubs_and_queues() {
        assert!(is_hub_destination(Some("CITY"), "anything"));
        assert!(is_hub_destination(Some("ISLAND"), "ShatteredIsles_TrainingDojo"));
        assert!(!is_hub_destination(Some("ISLAND"), ISLAND_A));
        assert!(needs_queue("CR19_PlayerHunt_X") && !needs_queue(ISLAND_A));
    }

    #[test]
    fn gauntlet() {
        for on in ["on", " ON ", "On"] {
            assert!(
                gauntlet_enabled(&Settings::from_pairs([("RUSTIC_ISLES_GAUNTLET", on)])),
                "{on}"
            );
        }
        for off in ["", "off", "true", "1", "yes", "onn"] {
            assert!(
                !gauntlet_enabled(&Settings::from_pairs([("RUSTIC_ISLES_GAUNTLET", off)])),
                "{off}"
            );
        }
        assert!(!gauntlet_enabled(&Settings::default()));
        assert!(is_gauntlet_hunt_id("Gauntlet_Season") && is_gauntlet_hunt_id("x_gauntlet"));
        for not in ["Arena_Elite", "Trials_Island_01", RAMSGATE_HUNT] {
            assert!(!is_gauntlet_hunt_id(not));
        }
    }

    fn members() -> Vec<String> {
        ["leader", "online-member", "offline-member"].map(String::from).to_vec()
    }

    #[test]
    fn party_resolution() {
        let members = members();
        let party = || {
            Some(ResolvableParty {
                party_id: "party-live",
                members: &members,
                revision: 7,
            })
        };
        let online = |m: &str| m == "online-member";
        let island = resolve_matchmaking_party(Some("ISLAND"), "leader", Some("party-live"), party(), online, false);
        assert_eq!(island.members, Some(vec!["leader".into(), "online-member".into()]));
        assert_eq!(
            (island.excluded, island.revision, island.mismatch),
            (vec!["offline-member".into()], Some(7), false)
        );
        let stale = resolve_matchmaking_party(Some("ISLAND"), "leader", None, party(), online, false);
        assert_eq!((stale.party_id, stale.members, stale.mismatch), (None, None, true));
        assert_eq!(stale.excluded, ["online-member", "offline-member"]);
        let city = resolve_matchmaking_party(Some("CITY"), "leader", None, party(), online, false);
        assert_eq!(city.members, Some(members.clone()));
        assert!(city.excluded.is_empty());
        let none = resolve_matchmaking_party(Some("ISLAND"), "leader", None, None, online, false);
        assert_eq!(none, PartyResolution::default());
        let shared = resolve_matchmaking_party(Some("SHARED"), "leader", None, party(), online, false);
        assert_eq!(shared.members.map(|m| m.len()), Some(3));
        let filtered = resolve_matchmaking_party(Some("SHARED"), "leader", None, party(), online, true);
        assert_eq!(filtered.members.map(|m| m.len()), Some(2));
        let flagged = resolve_matchmaking_party(Some("ISLAND"), "leader", None, party(), online, true);
        assert!(flagged.mismatch);
    }

    #[test]
    fn delays() {
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number(""), 0.0);
        assert!(js_number("abc").is_nan() && js_number("inf").is_nan());
        assert_eq!(js_number("1e3"), 1000.0);
        assert_eq!(read_delay_ms(Some("2500.9"), 5), 2500);
        for default in [Some("0"), Some("-3"), Some("x"), None] {
            assert_eq!(read_delay_ms(default, 5), 5, "{default:?}");
        }
    }

    #[test]
    fn draws() {
        for _ in 0..200 {
            let n = draw(10, 15);
            assert!((10..15).contains(&n), "{n}");
        }
        assert_eq!(draw(4, 4), 4);
        let window = GoNowWindow {
            min: 0,
            max: 1000,
            per_joiner: 1000,
            cap: 20_000,
        };
        for _ in 0..100 {
            let at = window.release_at(100, 30);
            assert!((100..=100 + 21_000).contains(&at), "{at}");
        }
        let fixed = GoNowWindow {
            min: 50,
            max: 10,
            per_joiner: 0,
            cap: 0,
        };
        assert!((10..=50).contains(&(fixed.release_at(0, 3))));
    }

    #[test]
    fn window_settings() {
        let s = Settings::from_pairs([("HUB_GO_NOW_MAX_DELAY_MS", "40"), ("GO_NOW_LOAD_PADDING_MAX_MS", "1")]);
        let hub = GoNowWindow::from_settings(&s, ReleaseKind::Hub);
        assert_eq!((hub.min, hub.max, hub.per_joiner, hub.cap), (0, 40, 1000, 1));
        let hunt = GoNowWindow::from_settings(&Settings::default(), ReleaseKind::Hunt);
        assert_eq!(
            (hunt.min, hunt.max, hunt.per_joiner, hunt.cap),
            (10_000, 20_000, 1000, 20_000)
        );
    }

    #[test]
    fn client_addresses() {
        let a = |peer: Option<&str>, xff: Option<&str>| client_address(peer, xff);
        assert_eq!(
            a(Some("203.0.113.7"), Some("198.51.100.1")).as_deref(),
            Some("203.0.113.7")
        );
        assert_eq!(
            a(Some("127.0.0.1"), Some("198.51.100.1")).as_deref(),
            Some("198.51.100.1")
        );
        assert_eq!(
            a(Some("127.0.0.1"), Some("10.0.0.1, 198.51.100.1")).as_deref(),
            Some("198.51.100.1")
        );
        assert_eq!(a(Some("127.0.0.1"), None).as_deref(), Some("127.0.0.1"));
        assert_eq!(a(Some("::1"), Some("127.0.0.1")).as_deref(), Some("127.0.0.1"));
        assert_eq!(
            a(Some("::ffff:127.0.0.1"), Some(" , 198.51.100.2 ,")).as_deref(),
            Some("198.51.100.2")
        );
        assert_eq!(a(None, Some("198.51.100.1")), None);
    }

    #[test]
    fn travel_hosts() {
        let hub = "198.51.100.9";
        for client in ["198.51.100.9", "::ffff:198.51.100.9", "127.0.0.1", "::1"] {
            assert_eq!(
                travel_host(Some(client), hub, false),
                ("127.0.0.1".into(), true),
                "{client}"
            );
        }
        assert_eq!(travel_host(Some("203.0.113.7"), hub, false), (hub.into(), false));
        assert_eq!(travel_host(None, hub, false), (hub.into(), false));
        assert_eq!(travel_host(Some("127.0.0.1"), hub, true), (hub.into(), false));
        assert_eq!(
            travel_host(Some("127.0.0.1"), "127.0.0.1", true),
            ("127.0.0.1".into(), true)
        );
        assert_eq!(travel_host(Some("127.0.0.1"), "", false), (String::new(), false));
    }

    #[test]
    fn party_ids() {
        let id = GlobalId::new(IdKind::Party, &NodeId::new("oregon-1").unwrap());
        let party_id = make_party_id(&id, "647472");
        assert_eq!(party_id, format!("{}.oregon-1_NjQ3NDcy", id.ulid()));
        let local = NodeId::new("local").unwrap();
        assert_eq!(
            rustic_isles_chat::ids::instance_id(&party_id, IdKind::Party, &local),
            Some(id.clone())
        );
        assert!(make_party_id(&id, "").ends_with('_'));
        let b64 = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
            ("392819_1.12.0", "MzkyODE5XzEuMTIuMA=="),
        ];
        for (text, encoded) in cases {
            assert_eq!(b64(text), encoded);
            assert!(make_party_id(&id, text).ends_with(&format!("_{encoded}")));
        }
    }

    #[test]
    fn qos_plans() {
        let p = |raw: Option<&str>| qos_plan(raw, "80");
        assert_eq!(p(None).port, Some(80));
        assert_eq!(p(Some("  ")).port, Some(80));
        assert_eq!(
            p(Some("OFF")),
            QosPlan {
                port: None,
                problem: None
            }
        );
        assert_eq!(p(Some("8080")).port, Some(8080));
        assert_eq!(
            p(Some("0")),
            QosPlan {
                port: None,
                problem: None
            }
        );
        assert_eq!(
            p(Some("70000")).problem.as_deref(),
            Some("QOS_PORT='70000' is not a port number or 'off'")
        );
        assert_eq!(p(Some(" http ")).port, None);
        assert_eq!(
            p(Some(" http ")).problem.as_deref(),
            Some("QOS_PORT='http' is not a port number or 'off'")
        );
        assert_eq!(qos_plan(None, "8080").port, Some(8080));
        assert_eq!(qos_plan(None, "").port, Some(80));
    }

    #[test]
    fn log_texts() {
        assert_eq!(log_text("a\nb\u{1b}"), "a\\nb\\u{1b}");
        assert_eq!(log_text(&"x".repeat(300)).len(), 120);
        assert_eq!(
            field_shape(Some(&Value::from("y".repeat(50)))),
            format!("\"{}\"", "y".repeat(40))
        );
        assert_eq!(field_shape(Some(&Value::from(true))), "true");
        assert_eq!(field_shape(Some(&Value::from(2))), "2");
        assert_eq!(field_shape(Some(&Value::Null)), "absent");
        assert_eq!(field_shape(None), "absent");
        let body: Value = serde_json::from_str(r#"{"a b":1,"ok_.-":2}"#).unwrap();
        assert_eq!(body_keys(Some(&body)), "a?b,ok_.-");
        let many: serde_json::Map<String, Value> = (0..20).map(|i| (format!("k{i:02}"), Value::Null)).collect();
        assert_eq!(body_keys(Some(&Value::Object(many))).split(',').count(), 12);
        assert_eq!(body_keys(Some(&Value::from(1))), "");
    }
}
