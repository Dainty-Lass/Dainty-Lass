// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' module skeleton (RD27, written clean-room from `docs/port/spec-rustic-isles-core.md`): how a
//! request enters, the route table in Express's match order, the pipeline, the state areas share, settings,
//! start-up and stop, the game data root and loader, the replay clock and ids, and the composer seam that
//! restricted's areas (and later the area crates) plug into. It hosts `rustic-isles-auth`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// A refusal is the answer itself (an axum `Response`), as in rustic-isles-auth, so it is not boxed
#![allow(clippy::result_large_err)]

pub mod body;
pub mod composer;
pub mod dedicated;
pub mod game_data;
pub mod handlers;
pub mod ids;
pub mod module;
pub mod observe;
pub mod pattern;
pub mod pipeline;
pub mod replay;
pub mod routes;
pub mod season;
pub mod settings;
pub mod shared;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod version;

pub use body::{BODY_LIMIT, read_body, read_body_parts, read_body_parts_limit};
pub use composer::{
    BareComposer, ComposeInputs, Composer, Composition, ModuleHooks, NoHooks, Started, StartedListener, Startup,
};
pub use dedicated::{DedicatedServers, Launch, LaunchRequest, LiveHub, LiveHubStatus, Unconfigured};
pub use game_data::{BACKEND_FILES, DIRECTOR_FILES, DIRECTOR_TRIALS_FILES, GameDataRoot, LoadError, director_files};
pub use handlers::{Call, Handler, Handlers, MountError};
pub use ids::{AccountId, AccountIdError, UserId};
pub use module::{
    BackendService, DefaultPorts, HOST_ONLY_PREFIXES, IslesModule, LISTENING_EVENT, ModuleSpec, PortService,
};
pub use observe::{NoObserver, Observation, Observer};
pub use pattern::{BadParam, Params, Pattern, PatternError};
pub use pipeline::{REQUEST_ID_HEADER, app, not_implemented};
pub use routes::{Group, Method, RouteDef, RouteSet, Scope, Service, Side, When};
pub use settings::{OPTIONAL_SWITCHES, SWITCHES, Settings, SettingsError, SharedSwitch};
pub use shared::{
    AdminAuthenticator, AdminCaller, ContextHome, IslesAuth, Presence, Shared, SharedParts, Slots, admin_refusal,
};
pub use version::{
    BountyItemGrant, Destination, Entitlement, GameMode, IslesVersion, MaskedJourneyNode, NoPlayerPolicy, ObjectiveRow,
    ProgressionConfigRule, SeasonalCoin, StarterManifest, StubReply, VersionHooks, VersionTables,
};
