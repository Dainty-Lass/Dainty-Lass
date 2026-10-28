//! "Rustic Isles 1.14.7" (DD4): game version 1.14.7, invite alias `reforged`, with realms (DD14) and the network
//! (DD16). The version's side of every RU3c hook and table; the code is isles-core's (DD8).

use std::sync::Arc;

use isles_chat::{ChatVersion, IslesChat};
use isles_core::IslesVersion;
use isles_core::hooks::{
    Destination, Entitlement, GameMode, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, StubReply, VersionHooks,
};
use isles_core::http::Rifts;
use isles_core::module::{BackendService, DefaultPorts, IslesModule, ModuleSpec};
use isles_core::routes::RouteSet;
use isles_core::settings::{
    BOUNTY_PERSISTENCE, COOLDOWN_PERSISTENCE, MASTERY_PERSISTENCE, QOS_PORT, REFORGE, Settings,
};
use isles_core::tables::{
    ItemGrantWindow, MaskedJourneyNode, SeasonalCoin, StarterManifest, SwitchDefault, VersionTables,
};
use isles_director::{DirectorConfig, DirectorService, GameserverArgs};
use module_api::manifest::PatchSetCapability;
use module_api::{Capabilities, PortRange};
use serde_json::{Value, json};

pub const ID: &str = "isles-1-14-7";
pub const DISPLAY_NAME: &str = "Rustic Isles 1.14.7";

/// The 1.14.7 sides of RU3c's hooks.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hooks;

/// Ramsgate is a null `huntId` with gameMode `CITY` (`controllers/candidateStatus.ts`, GetStatus.md).
const RAMSGATE_HUNT: &str = "ShatteredIsles_ReturnToRamsgate";

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        NoPlayerPolicy::ConfiguredAccount
    }

    // The captured epiceos contract: the token as the session id, under both spellings
    fn game_session_payload(&self, token: &str) -> Value {
        json!({ "error_code": null, "sessionid": token, "sessiontoken": token, "sessionToken": token })
    }

    fn network_encryption_keys(&self) -> bool {
        true
    }

    // weaponBaseLevel.ts: only KEEP_WEAPON_ITEM_LEVELS=true keeps the stored levels
    fn weapons_at_base_level(&self, settings: &Settings) -> bool {
        !settings
            .raw("KEEP_WEAPON_ITEM_LEVELS")
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("true")
    }

    fn reforge_rules(&self) -> bool {
        true
    }

    fn mastery_persistence(&self) -> bool {
        true
    }

    fn huntpass_payload(&self, id: &str) -> Value {
        json!({ "huntpass_id": id, "season": id })
    }

    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        json!({
            "phx_account_id": user_id,
            "objective_id": row.objective_id,
            "progress": row.value,
            "completed_count": row.completed_count,
            "created_date": row.created_date,
            "last_modified_date": row.last_modified_date,
        })
    }

    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::RollingHuntpassWindows
    }

    fn survey_config(&self) -> StubReply {
        StubReply {
            status: 200,
            body: Some(json!({ "code": null, "message": "OK", "payload": { "Triggers": [] } })),
        }
    }

    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        let entitlements: Vec<Value> = list
            .iter()
            .map(|e| json!({ "activatedDate": e.activated_date, "duration": e.duration, "name": e.name }))
            .collect();
        json!({ "entitlements": entitlements })
    }

    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        if hunt_id == RAMSGATE_HUNT {
            Destination {
                game_mode: GameMode::City,
                hunt_id: None,
            }
        } else if realm_hub && hunt_id.contains("TrainingDojo") {
            Destination {
                game_mode: GameMode::Shared,
                hunt_id: Some(hunt_id.to_owned()),
            }
        } else {
            Destination {
                game_mode: GameMode::Island,
                hunt_id: Some(hunt_id.to_owned()),
            }
        }
    }
}

const STARTER_STACKED: &[&str] = &[
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
];

const QUICK_EMOJIS: &[&str] = &[
    "EJ_PLAYER_BOSUN_JOY",
    "EJ_PLAYER_KAT_PRAY",
    "EJ_PLAYER_MOYRA_MUSCLE",
    "EJ_PLAYER_GREGARIO_QUESTION",
    "EJ_PLAYER_MIDDLEMAN_HMM",
    "EJ_PLAYER_PRIYANI_WINK",
    "EJ_PLAYER_ZAI_ANGRY",
    "EJ_PLAYER_ZAI_SWEAT",
    "",
];

pub const TABLES: VersionTables = VersionTables {
    target_changelist: 647472,
    game_version: "1.14.7",
    active_huntpass_id: "season43",
    recent_huntpass_seasons: &[
        "season43", "season44", "season45", "season46", "season47", "season48", "season49", "season50",
    ],
    // The 1.14.7 runtime collapses the line
    patchnotes_release_version: "",
    // tuning.ts's BountyGrantWindows: windows covering now, or the bounty component finds no season
    bounty_item_grant_data: &[
        ItemGrantWindow {
            item_id: "TOKEN_BOUNTY_DRAFT_PREMIUM",
            amount: 1,
            start: "2026-01-01T00:00:00.000Z",
            end: "2027-12-31T00:00:00.000Z",
        },
        ItemGrantWindow {
            item_id: "CURRENCY_PRESTIGE",
            amount: 5,
            start: "2026-01-01T00:00:00.000Z",
            end: "2027-12-31T00:00:00.000Z",
        },
    ],
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
        unless: "MYSTICPARADOX_GAUNTLET",
    }],
    starter_manifest: StarterManifest {
        bootstrap_version: "starter-1.12-v5",
        stacked: STARTER_STACKED,
        quick_emojis: Some(QUICK_EMOJIS),
    },
    switch_defaults: &[
        // listenerPlan.ts: unset is 80 (the 1.14.7 QoS fix)
        SwitchDefault {
            switch: QOS_PORT,
            value: "80",
        },
        SwitchDefault {
            switch: REFORGE,
            value: "on",
        },
        SwitchDefault {
            switch: MASTERY_PERSISTENCE,
            value: "on",
        },
        SwitchDefault {
            switch: BOUNTY_PERSISTENCE,
            value: "on",
        },
        SwitchDefault {
            switch: COOLDOWN_PERSISTENCE,
            value: "on",
        },
    ],
};

pub const DIRECTOR: DirectorConfig = DirectorConfig {
    target_changelist: 647472,
    args: GameserverArgs {
        standard: &[
            "-EpicPortal",
            "-server",
            "-nullrhi",
            "-warp",
            "-nosound",
            "-RepDriverEnable",
        ],
        log_cmds: "LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off",
        log_cmds_setting: true,
        windows_hide: true,
    },
    trials_new_tables: true,
};

/// rustic-rifts' crates, stubbed until RF4-RF6, as `MYSTICPARADOX_P2P_MODULE` allows (RF2).
#[cfg(feature = "realms")]
pub fn rifts(settings: &Settings) -> Rifts {
    rifts_with_core(settings, Arc::new(rifts_seam::NoCore))
}

/// Built without realms: Node's `DEDICATED_ONLY`.
#[cfg(not(feature = "realms"))]
pub fn rifts(_settings: &Settings) -> Rifts {
    Rifts::dedicated_only()
}

#[cfg(feature = "realms")]
pub fn rifts_with_core(settings: &Settings, core: Arc<dyn rifts_seam::IslesCore>) -> Rifts {
    use rifts_seam::{RiftsParts, RouteHandler};
    let _ = core;
    Rifts::load(settings, || {
        let routes: Vec<Arc<dyn RouteHandler>> = vec![
            Arc::new(rifts_admission::stub_routes()),
            Arc::new(rifts_relay::stub_routes()),
            Arc::new(rifts_realms::stub_routes()),
            Arc::new(rifts_guard::stub_routes()),
        ];
        RiftsParts {
            realms: Arc::new(rifts_realms::Stub),
            entry: Arc::new(rifts_realms::Stub),
            guard: Arc::new(rifts_guard::Stub),
            routes,
        }
    })
}

#[cfg(not(feature = "realms"))]
pub fn rifts_with_core(_settings: &Settings, _core: Arc<dyn rifts_seam::IslesCore>) -> Rifts {
    Rifts::dedicated_only()
}

pub fn version() -> IslesVersion {
    IslesVersion {
        hooks: Arc::new(Hooks),
        tables: TABLES,
        routes: RouteSet::V1147,
    }
}

pub fn spec() -> ModuleSpec {
    ModuleSpec {
        id: ID,
        display_name: DISPLAY_NAME,
        invite_aliases: &["reforged"],
        capabilities: Capabilities {
            realms: cfg!(feature = "realms"),
            network: true,
            chat: true,
            // rustic-freeza's patch set registers against it (DD25, RZ2)
            patch_set: Some(PatchSetCapability::Accepts),
            theme: None,
        },
        unverified: false,
        ports: DefaultPorts {
            backend: 61000,
            director: 61001,
            game: PortRange {
                first: 8770,
                last: 8777,
            },
        },
        version: version(),
        chat: Arc::new(IslesChat {
            version: ChatVersion::V1147,
        }),
        rifts: rifts_with_core,
        services: vec![Arc::new(BackendService), Arc::new(DirectorService::new(DIRECTOR))],
    }
}

pub fn module() -> IslesModule {
    IslesModule::new(spec())
}
