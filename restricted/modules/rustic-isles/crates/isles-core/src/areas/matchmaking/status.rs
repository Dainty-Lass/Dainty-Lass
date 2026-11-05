//! `controllers/candidateStatus.ts`: the body of `GET /candidate/status`, built from the candidate. The route keeps
//! the authentication and the candidate lookup; everything the client reads is decided here. The destination (Ramsgate
//! as `CITY` with a null `huntId`, a realm's Training hub as `SHARED`) is the version's `candidate_destination` hook.

use serde_json::{Map, Value, json};

use super::controller::RAMSGATE_HUNT;
use super::state::{MatchmakingResult, Source};
use crate::http::Shared;

/// The retry hint of a realm cancelled for its host's capacity (the candidate's StatusReason is REALM_FULL).
pub const REALM_FULL_RETRY_AFTER_SECONDS: i64 = 30;

/// The HTTP status and the JSON body (none for a missing candidate, which the route answers empty).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusAnswer {
    pub status: u16,
    pub body: Option<Value>,
}

/// `candidateStatusPeriodMillis`, `status`, ... in `JSON.stringify`'s key order.
struct Body {
    map: Map<String, Value>,
}

impl Body {
    fn new(candidate_id: &str, period: i64, mode: &str, hunt_id: Value, user: &str) -> Self {
        let mut map = Map::new();
        map.insert("candidateId".into(), json!(candidate_id));
        map.insert("candidateStatusPeriodMillis".into(), json!(period));
        map.insert("gameMode".into(), json!(mode));
        map.insert("huntId".into(), hunt_id);
        map.insert("playerStates".into(), json!({ user: {} }));
        Self { map }
    }

    fn finish(mut self, status: &str, reason: Value) -> Self {
        self.map.insert("status".into(), json!(status));
        // `0.0` is a plain 0 to JSON.stringify
        self.map.insert("statusDuration".into(), json!(0));
        self.map.insert("statusReason".into(), reason);
        self
    }

    fn done(self) -> StatusAnswer {
        StatusAnswer {
            status: 200,
            body: Some(Value::Object(self.map)),
        }
    }
}

/// `req.ip` behind a loopback proxy (`trust proxy` for loopback): the nearest address in `X-Forwarded-For` that isn't
/// a trusted proxy, else the peer.
pub fn client_ip(peer: Option<std::net::IpAddr>, forwarded_for: Option<&str>) -> Option<String> {
    let peer = peer?;
    let trusted = |ip: &str| {
        ip.trim_start_matches("::ffff:")
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    };
    let mut addrs: Vec<String> = forwarded_for
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    addrs.push(peer.to_string());
    let mut at = addrs.len() - 1;
    while at > 0 && trusted(&addrs[at]) {
        at -= 1;
    }
    Some(addrs.swap_remove(at))
}

/// `CandidateStatusAnswerFor`.
pub async fn candidate_status_answer(
    shared: &Shared,
    user: &str,
    result: Option<&MatchmakingResult>,
    client_ip: Option<&str>,
) -> StatusAnswer {
    let state = &shared.matchmaking;
    let now = state.clock.now_ms();
    let Some(result) = result else {
        tracing::error!("UserId {user} was not found in the MatchmakingMap");
        return StatusAnswer {
            status: 404,
            body: None,
        };
    };
    let realm_hub = result.source == Some(Source::RealmHub);
    let destination = shared.version.hooks.candidate_destination(&result.hunt_id, realm_hub);
    let mode = destination.game_mode.as_str();
    let client_hunt = destination.hunt_id.as_deref().map_or(Value::Null, |id| json!(id));
    let period = result.status_period_millis(now);
    let ramsgate = result.hunt_id == RAMSGATE_HUNT;
    if result.canceled {
        let reason = result.status_reason.as_deref().unwrap_or("HOST_ROUTE_FAILED");
        tracing::info!("Canceling matchmaking for user {user}: {reason}");
        let mut body =
            Body::new(&result.candidate_id, period, mode, Value::Null, user).finish("CANCELED", json!(reason));
        if reason == "REALM_FULL" {
            body.map
                .insert("retryAfterSeconds".into(), json!(REALM_FULL_RETRY_AFTER_SECONDS));
        }
        return body.done();
    }
    if !result.ready {
        tracing::info!("MM not ready yet!");
        // A realm's Ramsgate is huntId null while it is pending too, as it is once it runs
        let hunt = if realm_hub { client_hunt } else { json!(result.hunt_id) };
        return Body::new(&result.candidate_id, period, mode, hunt, user)
            .finish("MATCHING", Value::Null)
            .done();
    }

    // The hub advertises this machine's PUBLIC address; a client behind the same public address must reach it over
    // loopback, since NAT hairpin for the game's UDP port is not a given. The client's own HTTPS requests do reach the
    // public domain, so its source address equals the advertised host exactly when it is behind the same address.
    let advertised = result.host.as_str();
    let address = client_ip.unwrap_or_default().trim_start_matches("::ffff:");
    let ring_assigned =
        result.game_session_id.starts_with("hunt:") || result.game_session_id.starts_with("hub_instance:");
    let local = !address.is_empty()
        && !advertised.is_empty()
        && (address == advertised
            || ((!ring_assigned || advertised == "127.0.0.1") && (address == "127.0.0.1" || address == "::1")));
    let travel_host = if local { "127.0.0.1" } else { advertised };

    // Before answering with a hub address, confirm the hub is STILL serving: a cached candidate is not proof. A hub
    // that can't be verified holds the client in the MATCHING shape it already polls on. A realm hub is the member's
    // own loopback port, so it isn't checked against the DeployServer.
    let mut live_port = result.port;
    let ring_assigned_hub = result.game_session_id.starts_with("hub_instance:");
    if ramsgate && !realm_hub && !ring_assigned_hub {
        let live = state.deploy.get().hub_status().await;
        match live.and_then(|status| status.ramsgate).filter(|hub| hub.ready) {
            None => {
                tracing::warn!(
                    "Ramsgate candidate for {user} is stale (hub not verified as serving) - holding the client in MATCHING"
                );
                return Body::new(&result.candidate_id, 1000, "CITY", Value::Null, user)
                    .finish("MATCHING", Value::Null)
                    .done();
            }
            Some(hub) => {
                if hub.port != result.port {
                    tracing::warn!(
                        "Ramsgate port changed ({} -> {}); serving the live port",
                        result.port,
                        hub.port
                    );
                }
                live_port = hub.port;
            }
        }
    }
    tracing::info!(
        "Telling user {user} to travel to {travel_host}:{live_port} candidateId={} local={} verified={}",
        result.candidate_id,
        i32::from(local),
        if realm_hub { "realm" } else { "1" }
    );
    let mut body = Body::new(&result.candidate_id, period, mode, client_hunt, user);
    body.map.insert(
        "serverInfo".into(),
        json!({
            "buildId": shared.version.tables.build_id(),
            "gameSessionId": result.game_session_id,
            "host": travel_host,
            "port": live_port,
        }),
    );
    body.finish("IN_PROGRESS", Value::Null).done()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_address_is_the_peer_unless_a_loopback_proxy_forwards_one() {
        let ip = |text: &str| text.parse().ok();
        assert_eq!(
            client_ip(ip("203.0.113.7"), Some("198.51.100.1")).as_deref(),
            Some("203.0.113.7")
        );
        assert_eq!(
            client_ip(ip("127.0.0.1"), Some("198.51.100.1")).as_deref(),
            Some("198.51.100.1")
        );
        assert_eq!(
            client_ip(ip("127.0.0.1"), Some("10.0.0.1, 198.51.100.1")).as_deref(),
            Some("198.51.100.1")
        );
        assert_eq!(client_ip(ip("127.0.0.1"), None).as_deref(), Some("127.0.0.1"));
        assert_eq!(client_ip(ip("::1"), Some("127.0.0.1")).as_deref(), Some("127.0.0.1"));
        assert_eq!(client_ip(None, Some("1.1.1.1")), None);
    }
}
