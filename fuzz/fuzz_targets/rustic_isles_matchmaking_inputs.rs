//! What Rustic Isles' matchmaking reads from players and the realms side (spec-rustic-isles-matchmaking §16): arbitrary
//! bytes as a join body, an `x-forwarded-for` value with a peer, a candidate's JSON, a `QOS_PORT` value and a build
//! id. Nothing panics; a candidate that reads writes back and reads again the same; the client address is one of the
//! entries; a QoS port is 1-65535 or none; a party id is the room spelling, `_` and padded base64.
#![no_main]

use libfuzzer_sys::fuzz_target;
use dainty_module_api::{GlobalId, IdKind, NodeId};
use rustic_isles_matchmaking::rules::{body_keys, field_shape, log_text};
use rustic_isles_matchmaking::{MatchCandidate, client_address, effective_hunt_id, make_party_id, qos_plan};

fn is_padded_base64(text: &str) -> bool {
    let body = text.trim_end_matches('=');
    text.len() % 4 == 0
        && text.len() - body.len() <= 2
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

fuzz_target!(|data: &[u8]| {
    let mut parts = data.split(|b| *b == 0xff);
    let mut next = || String::from_utf8_lossy(parts.next().unwrap_or(&[])).into_owned();
    let (body, forwarded, peer, candidate, qos, build) = (next(), next(), next(), next(), next(), next());

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
        let text = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
        let hunt = effective_hunt_id(text("gameMode"), text("gameArgs"), text("playerHuntId"));
        let _ = log_text(&hunt);
        let _ = field_shape(value.get("gameType"));
        assert!(body_keys(Some(&value)).len() <= 12 * 40 + 11);
    }

    if let Some(address) = client_address(Some(&peer), Some(&forwarded)) {
        let entries: Vec<&str> = forwarded
            .split(',')
            .map(str::trim)
            .chain(std::iter::once(peer.as_str()))
            .collect();
        assert!(entries.contains(&address.as_str()), "{address:?}");
    }

    if let Some(read) = MatchCandidate::from_json_text(&candidate) {
        assert_eq!(MatchCandidate::from_json_text(&read.to_json_text()), Some(read));
    }

    let plan = qos_plan(Some(&qos), "80");
    assert!(plan.port.is_none_or(|p| p > 0));
    assert!(plan.port.is_none() || plan.problem.is_none());

    let node = NodeId::new("fuzz").unwrap();
    let id = GlobalId::new(IdKind::Party, &node);
    let party = make_party_id(&id, &build);
    let room = format!("{}.fuzz_", id.ulid());
    let suffix = party.strip_prefix(&room).expect("the room spelling and `_` first");
    assert!(is_padded_base64(suffix), "{suffix}");
});
