// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Test support (spec §14.2): a fake Director, a bare module state with the 1.14.7 destination rule, and a stand-in
//! auth layer naming the user by a test header.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::Request;
use dainty_module_api::BoxFuture;
use rustic_isles::testing::{SAMPLE_TABLES, SampleHooks};
use rustic_isles::{
    DedicatedServers, Destination, Entitlement, GameMode, Handlers, IslesVersion, Launch, LaunchRequest, LiveHub,
    LiveHubStatus, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, RouteSet, Settings, Shared, SharedParts, Slots,
    StubReply, VersionHooks,
};
use rustic_isles_auth::AuthData;
use rustic_isles_chat::HubSessions;
use rustic_isles_store::IslesStore;
use serde_json::Value;

use crate::rules::RAMSGATE_HUNT;
use crate::state::MatchmakingState;

/// The header the stand-in auth layer reads the caller's userId from.
pub const TEST_USER_HEADER: &str = "x-test-user";

/// What the fake answers a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeLaunch {
    /// Launches, ready at this address.
    Online {
        /// The server's address.
        host: String,
        /// The server's port.
        port: i64,
    },
    /// Refuses the launch, with its reason.
    Refused(Option<String>),
    /// The launch call fails with this error.
    Fails(String),
}

/// A Director in memory.
pub struct FakeDirector {
    /// Whether a dedicated server is configured.
    pub configured: bool,
    answer: Mutex<FakeLaunch>,
    ramsgate: Mutex<Option<LiveHub>>,
    hub_asks: AtomicUsize,
    launches: Mutex<Vec<LaunchRequest>>,
}

impl FakeDirector {
    /// A configured fake whose launches come up at `host:port`.
    pub fn online(host: &str, port: i64) -> Arc<Self> {
        Self::answering(FakeLaunch::Online {
            host: host.to_owned(),
            port,
        })
    }

    /// A configured fake answering every launch with `answer`.
    pub fn answering(answer: FakeLaunch) -> Arc<Self> {
        Arc::new(Self {
            configured: true,
            answer: Mutex::new(answer),
            ramsgate: Mutex::new(None),
            hub_asks: AtomicUsize::new(0),
            launches: Mutex::new(Vec::new()),
        })
    }

    /// Changes the answer later launches get.
    pub fn set_answer(&self, answer: FakeLaunch) {
        *self.answer.lock().unwrap_or_else(PoisonError::into_inner) = answer;
    }

    /// Sets the live Ramsgate hub the fake reports, or none.
    pub fn set_ramsgate(&self, hub: Option<LiveHub>) {
        *self.ramsgate.lock().unwrap_or_else(PoisonError::into_inner) = hub;
    }

    /// How many times the hub status was asked for.
    pub fn hub_asks(&self) -> usize {
        self.hub_asks.load(Ordering::SeqCst)
    }

    /// Every launch asked for, in order.
    pub fn launches(&self) -> Vec<LaunchRequest> {
        self.launches.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl DedicatedServers for FakeDirector {
    fn configured(&self) -> bool {
        self.configured
    }

    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>> {
        self.launches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let answer = self.answer.lock().unwrap_or_else(PoisonError::into_inner).clone();
        Box::pin(async move {
            match answer {
                FakeLaunch::Online { host, port } => Ok(Launch::Online { host, port }),
                FakeLaunch::Refused(reason) => Ok(Launch::Refused { reason }),
                FakeLaunch::Fails(text) => Err(text),
            }
        })
    }

    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>> {
        self.hub_asks.fetch_add(1, Ordering::SeqCst);
        let ramsgate = self.ramsgate.lock().unwrap_or_else(PoisonError::into_inner).clone();
        Box::pin(async move {
            Some(LiveHubStatus {
                ramsgate,
                training: None,
            })
        })
    }
}

/// Core's sample hooks with 1.14.7's candidate destination (spec-rustic-isles-1-14-7 §3.3.1).
pub struct DestinationHooks;

impl VersionHooks for DestinationHooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        SampleHooks.no_player_policy()
    }

    fn weapons_at_base_level(&self, settings: &Settings) -> bool {
        SampleHooks.weapons_at_base_level(settings)
    }

    fn reforge_rules(&self) -> bool {
        SampleHooks.reforge_rules()
    }

    fn mastery_persistence(&self) -> bool {
        SampleHooks.mastery_persistence()
    }

    fn huntpass_payload(&self, id: &str) -> Value {
        SampleHooks.huntpass_payload(id)
    }

    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        SampleHooks.objective_row(user_id, row)
    }

    fn progression_config(&self) -> ProgressionConfigRule {
        SampleHooks.progression_config()
    }

    fn survey_config(&self) -> StubReply {
        SampleHooks.survey_config()
    }

    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        SampleHooks.entitlements_payload(list)
    }

    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        let (game_mode, hunt_id) = if hunt_id == RAMSGATE_HUNT {
            (GameMode::City, None)
        } else if hunt_id.contains("TrainingDojo") && realm_hub {
            (GameMode::Shared, Some(hunt_id.to_owned()))
        } else {
            (GameMode::Island, Some(hunt_id.to_owned()))
        };
        Destination { game_mode, hunt_id }
    }
}

/// Core's sample version with 1.14.7's candidate destination.
pub fn test_version() -> IslesVersion {
    IslesVersion {
        hooks: Arc::new(DestinationHooks),
        tables: &SAMPLE_TABLES,
        routes: RouteSet::V1147,
    }
}

/// A bare module with `MATCHMAKING_MODE=DEPLOYSERVER` under `settings`, this crate's handlers, and the state, the
/// store and a hub-session registry in its slots.
pub struct TestModule {
    /// The module's shared state.
    pub shared: Arc<Shared>,
    /// Matchmaking's state, also in a slot.
    pub state: Arc<MatchmakingState>,
    /// The hub-session registry, also in a slot.
    pub hubs: Arc<HubSessions>,
    /// This crate's handlers.
    pub handlers: Handlers,
}

impl TestModule {
    /// A test module with rifts serving dedicated servers only.
    pub fn new(store: Option<IslesStore>, settings: Settings) -> Self {
        Self::with_rifts(store, settings, rustic_rifts_seam::Rifts::dedicated_only())
    }

    /// A test module over the given rifts.
    pub fn with_rifts(store: Option<IslesStore>, settings: Settings, rifts: rustic_rifts_seam::Rifts) -> Self {
        let state = Arc::new(MatchmakingState::new());
        let hubs = Arc::new(HubSessions::new());
        let mut slots = Slots::default()
            .with(Arc::clone(&state))
            .with(Arc::clone(&hubs))
            .with(Arc::new(crate::ring_instances_slot(&state)));
        if let Some(store) = store {
            slots.insert(Arc::new(store));
        }
        let mut handlers = Handlers::new();
        crate::register(&mut handlers);
        let mut parts = SharedParts::bare(test_version());
        parts.settings = Settings::from_pairs([("MATCHMAKING_MODE", "DEPLOYSERVER")]).overlay(&settings);
        parts.slots = slots;
        parts.rifts = rifts;
        parts.handlers = handlers.clone();
        #[allow(
            clippy::expect_used,
            reason = "test fixture over the crate's own sample data; a panic is how a test fails"
        )]
        let shared = Shared::new(parts).expect("the sample version's game version parses");
        state.bind_shared(&shared);
        Self {
            shared,
            state,
            hubs,
            handlers,
        }
    }

    /// The backend app behind the stand-in auth layer.
    pub fn app(&self) -> axum::Router {
        #[allow(
            clippy::expect_used,
            reason = "test fixture over the crate's own sample data; a panic is how a test fails"
        )]
        let app = rustic_isles::app(Arc::clone(&self.shared), rustic_isles::Service::Backend, &self.handlers)
            .expect("this crate's rows are in the table");
        with_test_auth(app)
    }
}

/// Puts `AuthData` naming [`TEST_USER_HEADER`]'s user in the extensions; without the header game auth runs.
pub fn with_test_auth(app: axum::Router) -> axum::Router {
    app.layer(axum::middleware::map_request(|mut request: Request| async move {
        let user = request
            .headers()
            .get(TEST_USER_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        if let Some(user) = user {
            request.extensions_mut().insert(AuthData {
                user_id: Some(user),
                ..AuthData::default()
            });
        }
        request
    }))
}
