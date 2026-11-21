// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' matchmaking and parties (RD27, written clean-room from `docs/port/spec-rustic-isles-matchmaking.md`):
//! the candidate, QoS and party routes, the in-memory queues, candidates, party instances and hub game session ids,
//! game presence, the party outbox dispatcher, the QoS pong listener, the Director over HTTP, matchmaking's answers
//! to rustic-rifts and the two module hooks core's Director invocation calls.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]
#![allow(
    clippy::result_large_err,
    reason = "a refusal is the answer itself (an axum `Response`), as in core and the other areas, so it is not boxed"
)]

pub mod candidate;
pub mod controller;
pub mod director;
pub mod hooks;
pub mod outbox;
pub mod party;
pub mod rifts_core;
pub mod routes;
pub mod rules;
pub mod startup;
pub mod state;
pub mod status;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use candidate::{CandidateSource, MatchCandidate};
pub use controller::{
    Ctx, GAUNTLET_DISABLED_REASON, JoinParty, JoinRequest, LeaveOutcome, NO_DEDICATED_SERVERS_REASON, resolve_account,
};
pub use director::{DEPLOYSERVER_MATCHMAKING_PATH, HttpDirector, read_answer};
pub use hooks::{place_hunt_players, player_offline, record_ring_instance};
pub use outbox::{ContextSink, HeadlineSink, outbox_pass};
pub use party::{INVITE_TTL_MS, Invitation, MAX_PARTY_SIZE, PLATFORM, Parties, PartyError};
pub use rifts_core::MatchmakingCore;
pub use routes::register;
pub use rules::{
    PartyResolution, QosPlan, RAMSGATE_HUNT, client_address, effective_hunt_id, gauntlet_enabled, is_gauntlet_hunt_id,
    is_hub_hunt_id, make_party_id, qos_plan, resolve_matchmaking_party,
};
pub use startup::{QOS_PONG, QosListener, startups, startups_on};
pub use state::{GamePresence, HuntActivity, MatchClock, MatchmakingState, NoHuntActivity, ring_instances_slot};
pub use status::REALM_FULL_RETRY_AFTER_SECONDS;
