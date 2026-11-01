//! "Rustic Isles 1.14.7" (DD4): game version 1.14.7, invite alias `reforged`, with realms (DD14) and the network
//! (DD16). The version's side of every RU3c hook and table; the code is isles-core's (DD8).

use std::sync::Arc;

use isles_chat::IslesChat;
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
        unless: "RUSTIC_ISLES_GAUNTLET",
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
    },
};

/// rustic-rifts' crates, stubbed until RF4-RF6, as `RUSTIC_ISLES_P2P_MODULE` allows (RF2).
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
    use std::collections::HashMap;

    use module_api::NodeId;
    use rifts_admission::{
        Admission, AdmissionError, AdmissionHunts, AdmissionRoutes, AdmissionService, AdmissionStore,
        CapabilityResolver, Es256Tickets, HostCapability, KeyId, MemoryAdmissionStore, P256KeyGenerator,
        TicketAuthority,
    };
    use rifts_guard::{
        CoreOperationsPolicy, Guard, GuardConfig, GuardParts, GuardRoutes, GuardStore, HuntSessions, NoHuntSessions,
        SqlGuardStore, StubGuardStore,
    };
    use rifts_realms::capabilities::HostCapabilities;
    use rifts_realms::control::ControlChannel;
    use rifts_realms::db::RealmsDb;
    use rifts_realms::directory::{DirectoryParts, RealmDirectory};
    use rifts_realms::host_policy::{ArtifactSources, read_artifact_policy};
    use rifts_realms::hunt::ArtifactHashes;
    use rifts_realms::ids::seam_session_id;
    use rifts_realms::leases::{Hosting, HostingParts, HostingRuntime, HostingSettings};
    use rifts_realms::lifecycle::RealmsRecovery;
    use rifts_realms::ports::{
        Clock, CoreAccounts, CoreGuardSessions, CoreOperationsView, IdSource as _, RandomIds,
        RateLimiter as RealmsRateLimiter, RealmEventSink, StderrLog, SystemClock as RealmsClock,
    };
    use rifts_realms::realm::RealmLimits;
    use rifts_realms::selection::derive_ice_peer_id;
    use rifts_realms::tiers::HostingPolicy;
    use rifts_realms::{
        RealmsHuntSessions, RealmsRoutes, RealmsRoutesParts, RealmsService, RealmsServiceParts,
        realms_enabled_or_default,
    };
    use rifts_relay::{
        Ice, IceHunts, IceService, OperationsPolicy, ProviderAnswer, ProviderHttp, ProviderRequest,
        ProviderUnreachable, RelayConfig, RelayGrants, RelayRoutes, RelayUsageCache,
    };
    use rifts_seam::{AccountId, BoxFuture, NodeId as SeamNodeId, PeerId, RiftsParts, RouteHandler, Secret};
    use store::{Database, DatabaseConfig, DatabaseOptions};

    struct NoProviderHttp;
    impl ProviderHttp for NoProviderHttp {
        fn post(&self, _request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>> {
            Box::pin(async { Err(ProviderUnreachable) })
        }
    }

    // Node's IsRateLimited (security/rateLimit.ts) over isles-core's sliding-window limiter; only /launcher/v1/realms*
    // routes throttle through RealmDirectory (p2pLauncher.ts, p2pControl.ts, p2pHostLifecycle.ts and p2pHunts.ts:80
    // apply no IsRateLimited window).
    struct IslesRateLimiter {
        inner: isles_core::areas::auth::runtime::RateLimiter,
        clock: Arc<dyn Clock>,
    }

    impl RealmsRateLimiter for IslesRateLimiter {
        fn is_limited(&self, key: &str, max: u32, window_ms: i64) -> bool {
            self.inner
                .is_limited_at(key, max as usize, window_ms, self.clock.now().0)
        }
    }

    Rifts::load(settings, || {
        let wanted = rifts_seam::Settings::get(settings, "REFORGED_DB_FILE").is_some()
            || rifts_seam::Settings::get(settings, "REFORGED_DB_ENGINE").is_some();
        let node = rifts_seam::Settings::get(settings, "REFORGED_NODE_ID")
            .and_then(|raw| NodeId::new(raw).ok())
            .or_else(|| NodeId::new("local").ok());
        let realms_db = if wanted {
            let lookup = |name: &str| rifts_seam::Settings::get(settings, name).map(str::to_owned);
            let file = rifts_seam::Settings::get(settings, "REFORGED_DB_FILE")
                .unwrap_or_default()
                .into();
            match (DatabaseConfig::from_lookup(lookup, file), node.clone()) {
                (Ok(config), Some(node_id)) => {
                    let options = DatabaseOptions::default();
                    let _ = Database::open(&config, &options, &rifts_guard::SCHEMA);
                    let db = RealmsDb::open(&config, &options, node_id).ok();
                    let _ = Database::open(&config, &options, &isles_core::db::SCHEMA);
                    db
                }
                _ => None,
            }
        } else {
            None
        };

        let realms_clock: Arc<dyn rifts_realms::ports::Clock> = Arc::new(RealmsClock);
        let realms_enabled = realms_enabled_or_default(settings);
        let mut policy = HostingPolicy::from_settings(settings);
        if rifts_seam::Settings::get(settings, "RUSTIC_ISLES_P2P_HOSTING").is_none() {
            policy.enabled = realms_enabled;
        }
        let service_node = node.unwrap_or_else(|| {
            NodeId::new("local").unwrap_or_else(|_| unreachable!("static 'local' node id is valid"))
        });
        let seam_node = SeamNodeId::new(service_node.as_str()).unwrap_or_else(|_| {
            SeamNodeId::new("local").unwrap_or_else(|_| unreachable!("static 'local' node id is valid"))
        });

        #[allow(clippy::type_complexity)]
        let (directory, control, hosting, capabilities, admission_routes, relay_routes): (
            Option<Arc<RealmDirectory>>,
            Option<Arc<ControlChannel>>,
            Option<Arc<Hosting>>,
            Option<Arc<HostCapabilities>>,
            Arc<dyn RouteHandler>,
            Arc<dyn RouteHandler>,
        ) = if let Some(ref db) = realms_db {
            let tickets = Arc::new(Es256Tickets::from_settings(settings).unwrap_or_else(|_| {
                let (pair, _, _) = P256KeyGenerator
                    .generate_with_pems(KeyId("default-k1".to_owned()))
                    .unwrap_or_else(|_| unreachable!("system RNG generates valid P-256 keypair"));
                let fallback: HashMap<String, String> = [
                    ("P2P_ADMISSION_KEY_ID".to_owned(), pair.key_id.0),
                    (
                        "P2P_ADMISSION_SIGNING_PRIVKEY_B64".to_owned(),
                        pair.signing_key.expose().to_owned(),
                    ),
                    ("P2P_ADMISSION_VERIFY_PUBKEY_B64".to_owned(), pair.verify_key),
                ]
                .into_iter()
                .collect();
                Es256Tickets::from_settings(&fallback)
                    .unwrap_or_else(|_| unreachable!("generated P-256 keypair parses"))
            }));
            let admission_store = Arc::new(MemoryAdmissionStore::default());
            let capabilities = Arc::new(HostCapabilities::new(db.clone()));

            let mut hosting_settings = HostingSettings::from_settings(settings);
            if rifts_seam::Settings::get(settings, "RUSTIC_ISLES_REALMS_HOSTING").is_none() {
                hosting_settings.realm_hosting = realms_enabled;
            }
            if rifts_seam::Settings::get(settings, "RUSTIC_ISLES_P2P_HOSTING").is_none() {
                hosting_settings.policy.enabled = realms_enabled;
            }
            if rifts_seam::Settings::get(settings, "P2P_ALLOW_ALL_ACCOUNTS").is_none()
                && rifts_seam::Settings::get(settings, "P2P_CANARY_ACCOUNTS").is_none()
                && hosting_settings.policy.trust_tier == 0
            {
                hosting_settings.policy.allow_all_accounts = true;
            }

            let settings_for_artifacts = settings.clone();
            let core_for_artifacts = Arc::clone(&core);
            let expected_artifacts = Arc::new(move || {
                if let Ok(policy) = read_artifact_policy(&settings_for_artifacts, &ArtifactSources::default()) {
                    return Ok(policy.hashes);
                }
                if core_for_artifacts.guard_mode() == rifts_seam::GuardMode::Observe {
                    let zero_hash = "0000000000000000000000000000000000000000000000000000000000000000";
                    return Ok(ArtifactHashes {
                        game_exe: rifts_seam::Settings::get(&settings_for_artifacts, "P2P_GAME_EXE_SHA256")
                            .unwrap_or("f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b")
                            .to_owned(),
                        internal_server: rifts_seam::Settings::get(
                            &settings_for_artifacts,
                            "P2P_INTERNAL_SERVER_SHA256",
                        )
                        .unwrap_or(zero_hash)
                        .to_owned(),
                        winmm_proxy: rifts_seam::Settings::get(&settings_for_artifacts, "P2P_WINMM_SHA256")
                            .unwrap_or(zero_hash)
                            .to_owned(),
                        steam_bridge: rifts_seam::Settings::get(&settings_for_artifacts, "P2P_TRANSPORT_MODULE_SHA256")
                            .or_else(|| rifts_seam::Settings::get(&settings_for_artifacts, "P2P_STEAM_BRIDGE_SHA256"))
                            .unwrap_or(zero_hash)
                            .to_owned(),
                        steam_runtime: rifts_seam::Settings::get(
                            &settings_for_artifacts,
                            "P2P_TRANSPORT_RUNTIME_SHA256",
                        )
                        .or_else(|| rifts_seam::Settings::get(&settings_for_artifacts, "P2P_STEAM_RUNTIME_SHA256"))
                        .unwrap_or(zero_hash)
                        .to_owned(),
                        admission_public_key: rifts_seam::Settings::get(
                            &settings_for_artifacts,
                            "P2P_ADMISSION_PUBLIC_XY_SHA256",
                        )
                        .unwrap_or(zero_hash)
                        .to_owned(),
                    });
                }
                Err("artifact policy feed not configured".to_owned())
            });

            let backend_url = rifts_seam::Settings::get(settings, "METAGAME_URL")
                .or_else(|| rifts_seam::Settings::get(settings, "BACKEND_PUBLIC_URL"))
                .unwrap_or("http://127.0.0.1:61000")
                .to_owned();
            let host_server_udp_port = rifts_seam::Settings::get(settings, "P2P_HOST_SERVER_UDP_PORT")
                .and_then(|raw| raw.trim().parse::<u16>().ok())
                .filter(|&p| p > 0)
                .unwrap_or(8791);

            let hosting = Arc::new(
                Hosting::new(HostingParts {
                    db: db.clone(),
                    settings: hosting_settings,
                    clock: Arc::clone(&realms_clock),
                    operations: Arc::new(CoreOperationsView(Arc::clone(&core))),
                    guard: Arc::new(CoreGuardSessions(Arc::clone(&core))),
                    recovery: Arc::new(RealmsRecovery::new(
                        db.clone(),
                        Arc::clone(&core),
                        Arc::clone(&realms_clock),
                        Arc::new(StderrLog),
                    )),
                    expected_artifacts,
                    log: Arc::new(StderrLog),
                })
                .with_runtime(HostingRuntime {
                    capabilities: Arc::clone(&capabilities),
                    tickets: Arc::clone(&tickets),
                    admission_store: Arc::clone(&admission_store) as Arc<dyn AdmissionStore>,
                    core: Arc::clone(&core),
                    node: service_node.clone(),
                    backend_url,
                    host_server_udp_port,
                    realms_enabled,
                }),
            );

            let directory = Arc::new(RealmDirectory::new(DirectoryParts {
                db: db.clone(),
                accounts: Arc::new(CoreAccounts(Arc::clone(&core))),
                events: Some(Arc::clone(&hosting) as Arc<dyn RealmEventSink>),
                clock: Arc::clone(&realms_clock),
                limiter: Arc::new(IslesRateLimiter {
                    inner: isles_core::areas::auth::runtime::RateLimiter::default(),
                    clock: Arc::clone(&realms_clock),
                }),
                ids: Arc::new(RandomIds),
                limits: RealmLimits::from_settings(settings),
                log: Arc::new(StderrLog),
            }));
            let control = Arc::new(ControlChannel::new(db.clone(), Arc::clone(&realms_clock)));

            let mut relay_config = RelayConfig::from_settings(settings);
            if relay_config.transport_key_secret.is_none() {
                let first = RandomIds.realm_id().replace('-', "");
                let second = RandomIds.realm_id().replace('-', "");
                relay_config.transport_key_secret = Some(Secret::new(format!("{first}{second}")));
            }
            let transport_secret = relay_config.transport_key_secret.clone();

            let admissions: Arc<dyn AdmissionService> = Arc::new(Admission::new(
                Arc::clone(&tickets) as Arc<dyn TicketAuthority>,
                Arc::clone(&admission_store) as Arc<dyn AdmissionStore>,
                Arc::clone(&hosting) as Arc<dyn AdmissionHunts>,
                transport_secret,
                Arc::new({
                    let c = Arc::clone(&realms_clock);
                    move || c.now()
                }),
            ));
            let cap_check = Arc::clone(&capabilities);
            let cap_clock = Arc::clone(&realms_clock);
            let cap_node = service_node.clone();
            let resolver: CapabilityResolver = Arc::new(move |header, session, scope| {
                let cap_check = Arc::clone(&cap_check);
                let now = cap_clock.now();
                let raw = header.map(str::to_owned);
                let session_str = session.to_owned();
                let node_clone = cap_node.clone();
                Box::pin(async move {
                    let rec = cap_check
                        .check(raw.as_deref(), &session_str, scope, now)
                        .await
                        .map_err(|e| match e {
                            rifts_realms::RealmsError::Refused(r) => AdmissionError::Refused(r),
                            other => AdmissionError::Refused(rifts_seam::Refusal {
                                code: "AUTH_UNAUTHORIZED".to_owned(),
                                message: other.to_string(),
                            }),
                        })?;
                    let peer_str = if rec.host_steam_id.is_empty() {
                        derive_ice_peer_id(&rec.host_account_id)
                    } else {
                        rec.host_steam_id.clone()
                    };
                    let peer_num = peer_str.parse::<u64>().unwrap_or_else(|_| {
                        rifts_admission::derive_ice_peer_id(&AccountId(rec.host_account_id.clone())).0
                    });
                    let seam_session = seam_session_id(&rec.session_id, &node_clone).ok_or_else(|| {
                        AdmissionError::Refused(rifts_seam::Refusal {
                            code: "P2P_VALIDATION_FAILED".to_owned(),
                            message: "sessionId must be a valid session identifier.".to_owned(),
                        })
                    })?;
                    Ok(HostCapability {
                        session: seam_session,
                        wire_session: rec.session_id,
                        generation: u64::try_from(rec.generation.max(1)).unwrap_or(1),
                        host_account: AccountId(rec.host_account_id),
                        host_peer: PeerId(peer_num),
                        bridge_instance_id: rec.bridge_instance_id,
                    })
                })
            });
            let admission_routes: Arc<dyn RouteHandler> =
                Arc::new(AdmissionRoutes::new(admissions, Some(resolver), seam_node.clone()));

            let grants = RelayGrants {
                config: Arc::new(relay_config),
                http: Arc::new(NoProviderHttp),
                usage: Arc::new(RelayUsageCache::default()),
            };
            let relay_service: Arc<dyn IceService> = Arc::new(Ice::new(
                Arc::clone(&hosting) as Arc<dyn IceHunts>,
                grants,
                Arc::clone(&hosting) as Arc<dyn OperationsPolicy>,
                Arc::new({
                    let c = Arc::clone(&realms_clock);
                    move || c.now()
                }),
            ));
            let relay_routes: Arc<dyn RouteHandler> = Arc::new(RelayRoutes::new(relay_service, seam_node));

            (
                Some(directory),
                Some(control),
                Some(hosting),
                Some(capabilities),
                admission_routes,
                relay_routes,
            )
        } else {
            (
                None,
                None,
                None,
                None,
                Arc::new(rifts_admission::stub_routes()),
                Arc::new(rifts_relay::stub_routes()),
            )
        };

        let (guard_store, hunt_sessions, has_store): (Arc<dyn GuardStore>, Arc<dyn HuntSessions>, bool) =
            if let Some(ref db) = realms_db {
                (
                    Arc::new(SqlGuardStore::new(db.database().clone())),
                    Arc::new(RealmsHuntSessions::new(db.clone(), Arc::clone(&realms_clock))),
                    true,
                )
            } else {
                (Arc::new(StubGuardStore), Arc::new(NoHuntSessions), false)
            };

        let guard = Arc::new(Guard::new(GuardParts {
            config: GuardConfig::from_settings(settings),
            store: guard_store,
            hunts: hunt_sessions,
            operations: Arc::new(CoreOperationsPolicy::new(Arc::clone(&core))),
            core: Arc::clone(&core),
            clock: Arc::new(rifts_guard::ports::SystemClock::default()),
            log: Arc::new(rifts_guard::ports::StderrLog),
        }));

        let realms_service = Arc::new(RealmsService::new(RealmsServiceParts {
            directory: directory.clone(),
            capabilities: capabilities.clone(),
            hosting: hosting.clone(),
            clock: Arc::clone(&realms_clock),
            core,
            policy,
            realms_enabled,
            node: service_node,
        }));
        let realms_routes = Arc::new(RealmsRoutes::new(RealmsRoutesParts {
            directory,
            control,
            hosting,
            capabilities,
            clock: realms_clock,
            realms_enabled,
        }));
        let guard_routes = Arc::new(GuardRoutes::with_store(Arc::clone(&guard), has_store));

        let routes: Vec<Arc<dyn RouteHandler>> = vec![admission_routes, relay_routes, realms_routes, guard_routes];
        RiftsParts {
            realms: Arc::clone(&realms_service) as Arc<dyn rifts_seam::RealmsHooks>,
            entry: realms_service as Arc<dyn rifts_seam::RealmsEntry>,
            guard,
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
        chat: Arc::new(IslesChat),
        rifts: rifts_with_core,
        services: vec![Arc::new(BackendService), Arc::new(DirectorService::new(DIRECTOR))],
    }
}

pub fn module() -> IslesModule {
    IslesModule::new(spec())
}
