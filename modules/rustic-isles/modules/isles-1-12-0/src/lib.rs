//! "Rustic Isles 1.12" (DD4): game version 1.12.0, unverified until someone records 1.12.0 traffic for a replay
//! (DD9, RU11). Every value here is 1.12.0's answer (the tag `dauntless-1.12.0`) or the 1.12 setting the code
//! names, as RU3c diffed them; the code is isles-core's (DD8).

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
use isles_core::tables::{StarterManifest, SwitchDefault, VersionTables};
use isles_director::{DirectorConfig, DirectorService, GameserverArgs};
use module_api::{Capabilities, PortRange};
use serde_json::{Value, json};

pub const ID: &str = "isles-1-12-0";
pub const DISPLAY_NAME: &str = "Rustic Isles 1.12";

/// The 1.12.0 sides of RU3c's hooks.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hooks;

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        NoPlayerPolicy::Off
    }

    fn game_session_payload(&self, token: &str) -> Value {
        json!({ "error_code": null, "sessionid": "SESSION_ID_LOL", "sessionToken": token })
    }

    // The 1.12 client connects unencrypted: POST /key/generate is 400 with an empty body
    fn network_encryption_keys(&self) -> bool {
        false
    }

    fn weapons_at_base_level(&self, _settings: &Settings) -> bool {
        false
    }

    fn reforge_rules(&self) -> bool {
        false
    }

    fn mastery_persistence(&self) -> bool {
        false
    }

    // The payload is the id itself
    fn huntpass_payload(&self, id: &str) -> Value {
        Value::String(id.to_owned())
    }

    fn objective_row(&self, _user_id: &str, row: &ObjectiveRow) -> Value {
        json!({ "objective_id": row.objective_id, "value": row.value, "completed_count": row.completed_count })
    }

    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::AsIs
    }

    fn survey_config(&self) -> StubReply {
        StubReply {
            status: 400,
            body: None,
        }
    }

    fn entitlements_payload(&self, _list: &[Entitlement]) -> Value {
        json!({ "code": null, "message": "OK", "payload": [] })
    }

    // Ramsgate's real hunt id too; 1.12 has no realms
    fn candidate_destination(&self, hunt_id: &str, _realm_hub: bool) -> Destination {
        Destination {
            game_mode: GameMode::Island,
            hunt_id: Some(hunt_id.to_owned()),
        }
    }
}

pub const TABLES: VersionTables = VersionTables {
    // Hard-coded in 1.12.0's launcherUpdates.ts and the Director's runtimeUpdater.ts
    target_changelist: 392819,
    game_version: "1.12.0",
    active_huntpass_id: "season19",
    recent_huntpass_seasons: &[],
    patchnotes_release_version: "1.12.0",
    bounty_item_grant_data: &[],
    store_sections: &["ladyluckstore"],
    // 1.12.0 has no funding rule; whether CURRENCY_S19_COIN takes the role is RU5's to confirm (RU3c)
    seasonal_coin: None,
    masked_journey_nodes: &[],
    starter_manifest: StarterManifest {
        bootstrap_version: "starter-1.12-v2",
        stacked: &[
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
        ],
        quick_emojis: None,
    },
    // 1.12.0 has no QoS listener, no reforge and no mastery persistence; a host can still switch them on
    switch_defaults: &[
        SwitchDefault {
            switch: QOS_PORT,
            value: "off",
        },
        SwitchDefault {
            switch: REFORGE,
            value: "off",
        },
        SwitchDefault {
            switch: MASTERY_PERSISTENCE,
            value: "off",
        },
        SwitchDefault {
            switch: BOUNTY_PERSISTENCE,
            value: "off",
        },
        SwitchDefault {
            switch: COOLDOWN_PERSISTENCE,
            value: "off",
        },
    ],
};

pub const DIRECTOR: DirectorConfig = DirectorConfig {
    target_changelist: 392819,
    args: GameserverArgs {
        standard: &["-EpicPortal", "-server", "-nullrhi", "-warp", "-RepDriverEnable"],
        log_cmds: "LogPhoenixCharacter VeryVerbose, LogOnline Verbose",
        log_cmds_setting: false,
        windows_hide: false,
    },
    trials_new_tables: false,
};

/// Realms exist only on 1.14.7 (DD14).
pub fn rifts(_settings: &Settings) -> Rifts {
    Rifts::dedicated_only()
}

pub fn rifts_with_core(
    _settings: &Settings,
    _core: Arc<dyn isles_core::areas::matchmaking::rifts_core::IslesCore>,
) -> Rifts {
    Rifts::dedicated_only()
}

pub fn version() -> IslesVersion {
    IslesVersion {
        hooks: Arc::new(Hooks),
        tables: TABLES,
        routes: RouteSet::V1120,
    }
}

pub fn spec() -> ModuleSpec {
    ModuleSpec {
        id: ID,
        display_name: DISPLAY_NAME,
        invite_aliases: &[],
        capabilities: Capabilities::default(),
        unverified: true,
        // Beside the 1.14.7 module's and the 1.4.4 stack's (61010-61015, UDP 8780-8787)
        ports: DefaultPorts {
            backend: 61020,
            director: 61021,
            game: PortRange {
                first: 8790,
                last: 8797,
            },
        },
        version: version(),
        chat: Arc::new(IslesChat {
            version: ChatVersion::V1120,
        }),
        rifts: rifts_with_core,
        services: vec![Arc::new(BackendService), Arc::new(DirectorService::new(DIRECTOR))],
    }
}

pub fn module() -> IslesModule {
    IslesModule::new(spec())
}
