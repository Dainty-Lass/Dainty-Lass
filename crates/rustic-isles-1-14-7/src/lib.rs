// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles for Dauntless 1.14.7 (RD27, written clean-room from `docs/port/spec-rustic-isles-1-14-7.md`): the
//! version's tables, hooks and Director values, the module spec, and [`VersionComposer`], which wires every
//! `rustic-isles-*` area and dainty-lass's Rustic Rifts crates into one module. The game-derived inputs that are not
//! wire facts come from `version_data.json` ([`VersionData`]).

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

mod accounts;
mod composer;
mod data;
mod hooks;
mod realms;
mod rifts;
#[cfg(feature = "test-support")]
pub mod test_support;

use std::sync::Arc;

use dainty_module_api::manifest::PatchSetCapability;
use dainty_module_api::{Capabilities, ModuleError, ModuleId, PortRange};
use rustic_isles::{
    BackendService, BountyItemGrant, DefaultPorts, IslesModule, IslesVersion, MaskedJourneyNode, ModuleSpec, RouteSet,
    SeasonalCoin, StarterManifest, VersionTables,
};
use rustic_isles_director::{DirectorConfig, DirectorService, GameserverArgs};

pub use accounts::AccountsCore;
pub use composer::{SharedCell, VersionComposer};
pub use data::{DATA_FILE, VersionData};
pub use hooks::Hooks;
pub use realms::RealmsAdapter;
pub use rifts::rifts_with_core;

/// Kept from restricted: `host.toml`, the gateway's routing and invites name it (core §15 item 13).
pub const ID: &str = "isles-1-14-7";
/// The module's name as hosts and the launcher show it.
pub const DISPLAY_NAME: &str = "Rustic Isles 1.14.7";
/// The manifest's game version answers `1.14.7` itself.
pub const INVITE_ALIASES: &[&str] = &["reforged"];
/// The backend, director, QoS and game server ports a host gets unless it sets its own.
pub const DEFAULT_PORTS: DefaultPorts = DefaultPorts {
    backend: 41_000,
    director: 41_001,
    qos: 41_010,
    game: PortRange {
        first: 8770,
        last: 8777,
    },
};

/// Wire facts the 1.14.7 client sends, asks for or reads back (spec §3.2, §4.2 class (a)), and Dainty Lass's policy
/// values (class (c)).
pub static TABLES: VersionTables = VersionTables {
    // The client's build
    target_changelist: 647_472,
    game_version: "1.14.7",
    // The client's season id
    active_huntpass_id: "season43",
    recent_huntpass_seasons: &[
        "season43", "season44", "season45", "season46", "season47", "season48", "season49", "season50",
    ],
    // The 1.14.7 runtime collapses the line
    patchnotes_release_version: "",
    // Ids the client reads; amounts from upstream tuning, dates moved forward for Dainty Lass (§14 item 16)
    bounty_item_grant_data: &[
        BountyItemGrant {
            item_id: "TOKEN_BOUNTY_DRAFT_PREMIUM",
            amount: 1,
            start: "2026-01-01T00:00:00.000Z",
            end: "2027-12-31T00:00:00.000Z",
        },
        BountyItemGrant {
            item_id: "CURRENCY_PRESTIGE",
            amount: 5,
            start: "2026-01-01T00:00:00.000Z",
            end: "2027-12-31T00:00:00.000Z",
        },
    ],
    // Section tags the client asks for
    store_sections: &[
        "ladyluckstore",
        "season_store",
        "webstore",
        "dyes",
        "hp_level_skip_rank",
        "huntpass_store",
        "exchange_vendor_slot_2",
        "exchange_vendor_slot_3",
        "fountain_daily_free_bundle",
    ],
    seasonal_coin: Some(SeasonalCoin {
        catalog_id: "CURRENCY_SEASONAL_COIN",
        max_per_claim: 1000,
    }),
    masked_journey_nodes: &[MaskedJourneyNode {
        node: "Gauntlet_Unlock",
        switch: "RUSTIC_ISLES_GAUNTLET",
    }],
    starter_manifest: StarterManifest {
        // Stored in existing rows: must stay byte-equal (§14 item 17)
        bootstrap_version: "starter-1.12-v5",
        // Wire facts (maintainer's ruling, 2026-11-05): written to a new account's stacked items in this order
        stacked_item_ids: &[
            "TOKEN_DAILY_CHALLENGE_DRAFT",
            "QI_BASIC_FLARE_DURABLE",
            "EM_INTRO_BEGINNER_01",
            "EM_END_BEGINNER_01",
            "EM_PLAYER_BEGINNER_01",
            "EM_PLAYER_BEGINNER_02",
            "EM_PLAYER_BEGINNER_03",
            "EM_PLAYER_BEGINNER_04",
            "BNC_MESH_BEGINNER_00",
            "BNC_FABRIC_BEGINNER_00",
            "BNC_SIGIL_BEGINNER_00",
            "BNC_ANIMATION_BEGINNER_00",
            "BNC_VFX_BEGINNER_00",
            "DYE_GREEN04_DURABLE",
            "DYE_BROWN07_DURABLE",
            "EJ_SYSTEM_TYPING",
            "EJ_PLAYER_BOSUN_JOY",
            "EJ_PLAYER_KAT_PRAY",
            "EJ_PLAYER_MOYRA_MUSCLE",
            "EJ_PLAYER_GREGARIO_QUESTION",
            "EJ_PLAYER_MIDDLEMAN_HMM",
            "EJ_PLAYER_PRIYANI_WINK",
            "EJ_PLAYER_ZAI_ANGRY",
            "EJ_PLAYER_ZAI_SWEAT",
        ],
        // The index is the in-game slot; 1.14.7 has the wheel, so it is never empty (characters §6.3)
        quick_emoji_wheel: &[
            "EJ_PLAYER_BOSUN_JOY",
            "EJ_PLAYER_KAT_PRAY",
            "EJ_PLAYER_MOYRA_MUSCLE",
            "EJ_PLAYER_GREGARIO_QUESTION",
            "EJ_PLAYER_MIDDLEMAN_HMM",
            "EJ_PLAYER_PRIYANI_WINK",
            "EJ_PLAYER_ZAI_ANGRY",
            "EJ_PLAYER_ZAI_SWEAT",
            "",
        ],
    },
    // The 1.14.7 QoS fix: unset is 80
    switch_defaults: &[
        ("qos_port", "80"),
        ("reforge", "on"),
        ("mastery_persistence", "on"),
        ("bounty_persistence", "on"),
        ("cooldown_persistence", "on"),
    ],
    // The captured epiceos contract
    session_payload: rustic_isles_auth::session_payload_1_14_7,
};

/// 1.14.7's game server switches and default `-LogCmds` (director §12.4).
pub const DIRECTOR: DirectorConfig = DirectorConfig::new(
    647_472,
    GameserverArgs::new(
        &[
            "-EpicPortal",
            "-server",
            "-nullrhi",
            "-warp",
            "-nosound",
            "-RepDriverEnable",
        ],
        "LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off",
    ),
);

/// `realms` follows the feature; rustic-freeza registers its patch set against this module (DD25).
pub fn capabilities() -> Capabilities {
    Capabilities {
        realms: cfg!(feature = "realms"),
        network: true,
        chat: true,
        patch_set: Some(PatchSetCapability::Accepts),
        theme: None,
    }
}

/// 1.14.7's hooks, tables and route set.
pub fn version() -> IslesVersion {
    IslesVersion {
        hooks: Arc::new(Hooks),
        tables: &TABLES,
        routes: RouteSet::V1147,
    }
}

fn spec_over(composer: VersionComposer) -> Result<ModuleSpec, ModuleError> {
    let id = ModuleId::new(ID).map_err(|e| ModuleError::failed(format!("module id {ID}: {e}")))?;
    Ok(ModuleSpec {
        id,
        display_name: DISPLAY_NAME.to_owned(),
        invite_aliases: INVITE_ALIASES.iter().map(|alias| (*alias).to_owned()).collect(),
        capabilities: capabilities(),
        unverified: false,
        ports: DEFAULT_PORTS,
        version: version(),
        services: vec![Arc::new(BackendService), Arc::new(DirectorService::new(DIRECTOR))],
        composer: Arc::new(composer),
    })
}

/// The module spec, its composer reading `version_data.json`.
pub fn spec() -> Result<ModuleSpec, ModuleError> {
    spec_over(VersionComposer::reading_file())
}

/// The same with the version data given (restricted's compiled-in host).
pub fn spec_with(data: VersionData) -> Result<ModuleSpec, ModuleError> {
    spec_over(VersionComposer::with_data(data))
}

/// The module, its spec reading `version_data.json`.
pub fn module() -> Result<IslesModule, ModuleError> {
    IslesModule::new(spec()?)
}
