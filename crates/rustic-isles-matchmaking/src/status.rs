// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The status answer (spec §3.6, `candidateStatus.ts`).

use serde_json::{Map, Value};

use crate::candidate::MatchCandidate;
use crate::controller::Ctx;
use crate::rules::{self, RAMSGATE_HUNT};

/// The realm-full retry hint.
pub const REALM_FULL_RETRY_AFTER_SECONDS: i64 = 30;
const HOST_ROUTE_FAILED: &str = "HOST_ROUTE_FAILED";

/// A status and its JSON body, or a bare status.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusAnswer {
    /// The HTTP status.
    pub status: u16,
    /// The JSON body; `None` answers the bare status.
    pub body: Option<Value>,
}

struct Body<'a> {
    candidate: &'a MatchCandidate,
    user: &'a str,
    period: i64,
    game_mode: &'a str,
    hunt_id: Option<&'a str>,
}

impl Body<'_> {
    fn write(&self, server_info: Option<Value>, status: &str, reason: Option<&str>, retry: Option<i64>) -> Value {
        let mut out = Map::new();
        out.insert("candidateId".into(), self.candidate.candidate_id.clone().into());
        out.insert("candidateStatusPeriodMillis".into(), self.period.into());
        out.insert("gameMode".into(), self.game_mode.into());
        out.insert("huntId".into(), self.hunt_id.map_or(Value::Null, Value::from));
        let mut states = Map::new();
        states.insert(self.user.to_owned(), Value::Object(Map::new()));
        out.insert("playerStates".into(), Value::Object(states));
        if let Some(info) = server_info {
            out.insert("serverInfo".into(), info);
        }
        out.insert("status".into(), status.into());
        out.insert("statusDuration".into(), 0.into());
        out.insert("statusReason".into(), reason.map_or(Value::Null, Value::from));
        if let Some(retry) = retry {
            out.insert("retryAfterSeconds".into(), retry.into());
        }
        Value::Object(out)
    }
}

impl Ctx {
    /// The answer for a candidate from the poll (client-visible, or the realms side's as it is).
    pub async fn status_answer(&self, user: &str, candidate: Option<&MatchCandidate>) -> StatusAnswer {
        let Some(candidate) = candidate else {
            tracing::error!("UserId {user} was not found in the MatchmakingMap");
            return StatusAnswer {
                status: 404,
                body: None,
            };
        };
        let now = self.now();
        let realm_hub = candidate.is_realm_hub();
        let destination = self
            .shared
            .version()
            .hooks
            .candidate_destination(&candidate.hunt_id, realm_hub);
        let client_hunt = destination.hunt_id.as_deref();
        let body = Body {
            candidate,
            user,
            period: candidate.status_period_millis(now),
            game_mode: destination.game_mode.as_str(),
            hunt_id: client_hunt,
        };
        let ok = |value: Value| StatusAnswer {
            status: 200,
            body: Some(value),
        };
        if candidate.canceled {
            let reason = candidate.status_reason.as_deref().unwrap_or(HOST_ROUTE_FAILED);
            tracing::info!("Canceling matchmaking for user {user}: {reason}");
            let retry = (reason == "REALM_FULL").then_some(REALM_FULL_RETRY_AFTER_SECONDS);
            let cancelled = Body { hunt_id: None, ..body };
            return ok(cancelled.write(None, "CANCELED", Some(reason), retry));
        }
        if !candidate.ready {
            tracing::info!("MM not ready yet!");
            let hunt = if realm_hub {
                client_hunt
            } else {
                Some(candidate.hunt_id.as_str())
            };
            return ok(Body { hunt_id: hunt, ..body }.write(None, "MATCHING", None, None));
        }
        let mut port = candidate.port;
        let verify =
            candidate.hunt_id == RAMSGATE_HUNT && !realm_hub && !candidate.game_session_id.starts_with("hub_instance:");
        if verify {
            match self.live_ramsgate_port().await {
                Some(live) => {
                    if live != port {
                        tracing::warn!("Ramsgate port changed ({port} -> {live}); serving the live port");
                    }
                    port = live;
                }
                None => {
                    tracing::warn!(
                        "Ramsgate candidate for {user} is stale (hub not verified as serving) - holding the client in MATCHING"
                    );
                    let held = Body {
                        period: 1000,
                        game_mode: "CITY",
                        hunt_id: None,
                        ..body
                    };
                    return ok(held.write(None, "MATCHING", None, None));
                }
            }
        }
        let (host, local) = rules::travel_host(self.client.as_deref(), &candidate.host, candidate.is_ring_assigned());
        let mut info = Map::new();
        info.insert("buildId".into(), self.shared.version().tables.build_id().into());
        info.insert("gameSessionId".into(), candidate.game_session_id.clone().into());
        info.insert("host".into(), host.clone().into());
        info.insert("port".into(), port.into());
        tracing::info!(
            "Telling user {user} to travel to {host}:{port} candidateId={} local={} verified={}",
            candidate.candidate_id,
            u8::from(local),
            if realm_hub { "realm" } else { "1" }
        );
        ok(body.write(Some(Value::Object(info)), "IN_PROGRESS", None, None))
    }

    /// The Director's ready Ramsgate port; without a Director the hub is never verified.
    async fn live_ramsgate_port(&self) -> Option<i64> {
        let servers = self.shared.dedicated_servers();
        if !servers.configured() {
            return None;
        }
        let hub = servers.hub_status().await?.ramsgate?;
        hub.ready.then_some(hub.port)
    }
}
