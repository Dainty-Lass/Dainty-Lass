// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Rustic Rifts composition (spec §6): realms, admission, relay and guard over one realms database, or the
//! stubs without one. Blocking: the composer runs it off the async threads.

use std::sync::Arc;

use rifts_seam::{IslesCore, Rifts};
use rustic_isles::Settings;

/// The rifts parts over `core`, or dedicated-only when `RUSTIC_RIFTS_P2P_MODULE` says no.
#[cfg(feature = "realms")]
pub fn rifts_with_core(settings: &Settings, core: Arc<dyn IslesCore>) -> Rifts {
    Rifts::load(settings, || wired::parts(settings, core))
}

/// Built without the `realms` feature: dedicated servers only.
#[cfg(not(feature = "realms"))]
pub fn rifts_with_core(_settings: &Settings, _core: Arc<dyn IslesCore>) -> Rifts {
    Rifts::dedicated_only()
}

#[cfg(feature = "realms")]
mod wired {
    use std::path::PathBuf;
    use std::sync::Arc;

    use module_api::{NodeId, NodeIdentity};
    use rifts_admission::{
        Admission, AdmissionError, AdmissionHunts, AdmissionRoutes, CapabilityResolver, Es256Tickets, HostCapability,
        MemoryAdmissionStore, P256KeyGenerator,
    };
    use rifts_guard::{
        CoreOperationsPolicy, Guard, GuardConfig, GuardParts, GuardRoutes, NoHuntSessions, StubGuardStore,
    };
    use rifts_realms::capabilities::{HostCapabilities, generate_capability_secret};
    use rifts_realms::control::ControlChannel;
    use rifts_realms::db::RealmsDb;
    use rifts_realms::directory::{DirectoryParts, RealmDirectory};
    use rifts_realms::host_policy::{ArtifactSources, KNOWN_GAME_EXE_SHA256, read_artifact_policy};
    use rifts_realms::hunt::ArtifactHashes;
    use rifts_realms::ids::seam_session_id;
    use rifts_realms::leases::{Hosting, HostingParts, HostingRuntime, HostingSettings};
    use rifts_realms::ports::{
        Clock, CoreAccounts, CoreGuardSessions, CoreOperationsView, RandomIds, RateLimiter, RealmEventSink, StderrLog,
        SystemClock,
    };
    use rifts_realms::realm::RealmLimits;
    use rifts_realms::tiers::HostingPolicy;
    use rifts_realms::{RealmsError, RealmsRoutes, RealmsRoutesParts, RealmsService, RealmsServiceParts};
    use rifts_relay::{Ice, IceHunts, ProviderAnswer, ProviderHttp, ProviderRequest, ProviderUnreachable};
    use rifts_relay::{RelayConfig, RelayGrants, RelayRoutes, RelayUsageCache};
    use rifts_seam::{
        AccountId, BoxFuture, CapabilityScope, GuardMode, IslesCore, PeerId, Refusal, RiftsParts, RouteHandler,
        UnixMillis,
    };
    use rustic_isles::Settings;
    use store::{DatabaseConfig, DatabaseOptions};

    const DEFAULT_BACKEND_URL: &str = "http://127.0.0.1:61000";
    const DEFAULT_HOST_UDP_PORT: u16 = 8791;
    const GENERATED_KEY_ID: &str = "default-k1";
    const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    /// rifts-seam's view: `raw`, so a set-but-empty value counts (§14 item 10).
    fn get<'a>(settings: &'a Settings, name: &str) -> Option<&'a str> {
        rifts_seam::Settings::get(settings, name)
    }

    fn node(settings: &Settings) -> NodeId {
        get(settings, "DAINTY_NODE_ID")
            .and_then(|text| NodeId::new(text.trim()).ok())
            .unwrap_or_else(|| NodeIdentity::standalone().node)
    }

    fn standalone_seam_node() -> rifts_seam::NodeId {
        match rifts_seam::NodeId::new(NodeIdentity::standalone().node.as_str()) {
            Ok(node) => node,
            // `local` is a valid name in both id types
            Err(_) => unreachable!("the standalone node name parses"),
        }
    }

    /// No line is logged for a failed open: the stubs answer (§14 item 11).
    fn open_db(settings: &Settings, node: &NodeId) -> Option<RealmsDb> {
        let wanted = get(settings, "DAINTY_DB_FILE").is_some() || get(settings, "DAINTY_DB_ENGINE").is_some();
        if !wanted {
            return None;
        }
        let file = PathBuf::from(get(settings, "DAINTY_DB_FILE").unwrap_or(""));
        let config = DatabaseConfig::from_lookup(|name| get(settings, name).map(str::to_owned), file).ok()?;
        RealmsDb::open(&config, &DatabaseOptions::default(), node.clone()).ok()
    }

    /// The configured key set, else a fresh pair in memory only (§14 item 4).
    fn tickets(settings: &Settings) -> Option<Es256Tickets> {
        if let Ok(tickets) = Es256Tickets::from_settings(settings) {
            return Some(tickets);
        }
        let key_id = rifts_admission::KeyId(GENERATED_KEY_ID.to_owned());
        let (pair, _, _) = P256KeyGenerator.generate_with_pems(key_id).ok()?;
        let generated = settings
            .clone()
            .forced(rifts_admission::keys::KEY_ID_SETTING, GENERATED_KEY_ID)
            .forced(rifts_admission::keys::SIGNING_KEY_SETTING, pair.signing_key.expose())
            .forced(rifts_admission::keys::VERIFY_KEY_SETTING, &pair.verify_key);
        Es256Tickets::from_settings(&generated).ok()
    }

    /// The realm directory's own limiter at the system clock, not the module's auth limiter (§14 item 13).
    struct DirectoryLimiter(rustic_isles_auth::RateLimiter);

    impl RateLimiter for DirectoryLimiter {
        fn is_limited(&self, key: &str, max: u32, window_ms: i64) -> bool {
            let max = usize::try_from(max).unwrap_or(usize::MAX);
            self.0.is_limited_at(key, max, window_ms, SystemClock.now().0)
        }
    }

    /// No provider is ever reachable, so no Cloudflare credential is minted (§14 item 6).
    struct Unreachable;

    impl ProviderHttp for Unreachable {
        fn post(&self, _: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>> {
            Box::pin(std::future::ready(Err(ProviderUnreachable)))
        }
    }

    fn hash(settings: &Settings, names: &[&str]) -> Option<String> {
        names
            .iter()
            .find_map(|name| get(settings, name).map(str::trim).filter(|v| !v.is_empty()))
            .map(str::to_owned)
    }

    /// The pinned policy, else in Observe mode the settings with zeros for unset roles (§14 item 7).
    fn expected_artifacts(settings: &Settings, core: &dyn IslesCore) -> Result<ArtifactHashes, String> {
        if let Ok(policy) = read_artifact_policy(settings, &ArtifactSources::default()) {
            return Ok(policy.hashes);
        }
        if core.guard_mode() != GuardMode::Observe {
            return Err("artifact policy feed not configured".to_owned());
        }
        let or_zero = |names: &[&str]| hash(settings, names).unwrap_or_else(|| ZEROS.to_owned());
        let known_exe = KNOWN_GAME_EXE_SHA256.first().map(|(_, sha)| (*sha).to_owned());
        Ok(ArtifactHashes {
            game_exe: hash(settings, &["P2P_GAME_EXE_SHA256"])
                .or(known_exe)
                .unwrap_or_else(|| ZEROS.to_owned()),
            internal_server: or_zero(&["P2P_INTERNAL_SERVER_SHA256"]),
            winmm_proxy: or_zero(&["P2P_WINMM_SHA256"]),
            steam_bridge: or_zero(&["P2P_TRANSPORT_MODULE_SHA256", "P2P_STEAM_BRIDGE_SHA256"]),
            steam_runtime: or_zero(&["P2P_TRANSPORT_RUNTIME_SHA256", "P2P_STEAM_RUNTIME_SHA256"]),
            admission_public_key: or_zero(&["P2P_ADMISSION_PUBLIC_XY_SHA256"]),
        })
    }

    fn backend_url(settings: &Settings) -> String {
        ["METAGAME_URL", "BACKEND_PUBLIC_URL"]
            .iter()
            .find_map(|name| get(settings, name).map(str::trim).filter(|v| !v.is_empty()))
            .unwrap_or(DEFAULT_BACKEND_URL)
            .to_owned()
    }

    fn host_udp_port(settings: &Settings) -> u16 {
        get(settings, "P2P_HOST_SERVER_UDP_PORT")
            .and_then(|v| v.trim().parse::<u16>().ok())
            .filter(|port| *port > 0)
            .unwrap_or(DEFAULT_HOST_UDP_PORT)
    }

    fn refusal(code: &str, message: impl Into<String>) -> AdmissionError {
        AdmissionError::Refused(Refusal {
            code: code.to_owned(),
            message: message.into(),
        })
    }

    /// The capability header checked for routes the HTTP layer left anonymous; another error's text is echoed as
    /// the refusal's message (§14 item 9).
    fn resolver(capabilities: Arc<HostCapabilities>, node: NodeId) -> CapabilityResolver {
        Arc::new(move |header, session, scope: CapabilityScope| {
            let (capabilities, node) = (Arc::clone(&capabilities), node.clone());
            let (header, session) = (header.map(str::to_owned), session.to_owned());
            Box::pin(async move {
                let now = SystemClock.now();
                let record = match capabilities.check(header.as_deref(), &session, scope, now).await {
                    Ok(record) => record,
                    Err(RealmsError::Refused(refused)) => return Err(AdmissionError::Refused(refused)),
                    Err(other) => return Err(refusal("AUTH_UNAUTHORIZED", other.to_string())),
                };
                let host_account = AccountId(record.host_account_id.clone());
                let host_peer = record
                    .host_steam_id
                    .parse::<u64>()
                    .ok()
                    .filter(|_| !record.host_steam_id.is_empty())
                    .or_else(|| {
                        rifts_realms::selection::derive_ice_peer_id(&record.host_account_id)
                            .parse::<u64>()
                            .ok()
                    })
                    .map_or_else(|| rifts_admission::derive_ice_peer_id(&host_account), PeerId);
                let Some(seam_session) = seam_session_id(&record.session_id, &node) else {
                    return Err(refusal(
                        "P2P_VALIDATION_FAILED",
                        "sessionId must be a valid session identifier.",
                    ));
                };
                Ok(HostCapability {
                    session: seam_session,
                    wire_session: record.session_id.clone(),
                    generation: u64::try_from(record.generation.max(1)).unwrap_or(1),
                    host_account,
                    host_peer,
                    bridge_instance_id: record.bridge_instance_id,
                })
            })
        })
    }

    pub(super) fn parts(settings: &Settings, core: Arc<dyn IslesCore>) -> RiftsParts {
        let node = node(settings);
        // The admission and relay routes name nodes in rifts-seam's own type
        let seam_node = rifts_seam::NodeId::new(node.as_str()).unwrap_or_else(|_| standalone_seam_node());
        let db = open_db(settings, &node);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let realms_enabled = rifts_realms::realms_enabled_or_default(settings);
        let mut policy = HostingPolicy::from_settings(settings);
        // The hosting switches give way to the realms switch unless they are set (§14 item 1)
        if get(settings, "RUSTIC_RIFTS_P2P_HOSTING").is_none() {
            policy.enabled = realms_enabled;
        }
        let tickets = db.as_ref().and_then(|_| tickets(settings));
        let mut routes: Vec<Arc<dyn RouteHandler>> = Vec::new();
        let (mut directory, mut control, mut hosting, mut capabilities) = (None, None, None, None);
        match (&db, tickets) {
            (Some(db), Some(tickets)) => {
                let tickets = Arc::new(tickets);
                let admission_store = Arc::new(MemoryAdmissionStore::default());
                let caps = Arc::new(HostCapabilities::new(db.clone()));
                let mut hosting_settings = HostingSettings::from_settings(settings);
                if get(settings, "RUSTIC_RIFTS_REALMS_HOSTING").is_none() {
                    hosting_settings.realm_hosting = realms_enabled;
                }
                if get(settings, "RUSTIC_RIFTS_P2P_HOSTING").is_none() {
                    hosting_settings.policy.enabled = realms_enabled;
                }
                // Open hosting by default (§14 item 2)
                let listed =
                    get(settings, "P2P_ALLOW_ALL_ACCOUNTS").is_some() || get(settings, "P2P_CANARY_ACCOUNTS").is_some();
                if !listed && hosting_settings.policy.trust_tier == 0 {
                    hosting_settings.policy.allow_all_accounts = true;
                }
                let artifacts = {
                    let (settings, core) = (settings.clone(), Arc::clone(&core));
                    Arc::new(move || expected_artifacts(&settings, &*core))
                };
                let hosted = Hosting::new(HostingParts {
                    db: db.clone(),
                    settings: hosting_settings,
                    clock: Arc::clone(&clock),
                    operations: Arc::new(CoreOperationsView(Arc::clone(&core))),
                    guard: Arc::new(CoreGuardSessions(Arc::clone(&core))),
                    recovery: Arc::new(rifts_realms::RealmsRecovery::new(
                        db.clone(),
                        Arc::clone(&core),
                        Arc::clone(&clock),
                        Arc::new(StderrLog),
                    )),
                    expected_artifacts: artifacts,
                    log: Arc::new(StderrLog),
                })
                .with_runtime(HostingRuntime {
                    capabilities: Arc::clone(&caps),
                    admission_store: admission_store.clone(),
                    tickets: Arc::clone(&tickets),
                    core: Arc::clone(&core),
                    backend_url: backend_url(settings),
                    host_server_udp_port: host_udp_port(settings),
                    realms_enabled,
                    node: node.clone(),
                });
                let hosted = Arc::new(hosted);
                let realm_directory = RealmDirectory::new(DirectoryParts {
                    db: db.clone(),
                    accounts: Arc::new(CoreAccounts(Arc::clone(&core))),
                    events: Some(Arc::clone(&hosted) as Arc<dyn RealmEventSink>),
                    clock: Arc::clone(&clock),
                    limiter: Arc::new(DirectoryLimiter(rustic_isles_auth::RateLimiter::new())),
                    ids: Arc::new(RandomIds),
                    limits: RealmLimits::from_settings(settings),
                    log: Arc::new(StderrLog),
                });
                let mut relay_config = RelayConfig::from_settings(settings);
                // Random per start: pair keys change at each restart (§14 item 5)
                let transport_secret = relay_config
                    .transport_key_secret
                    .clone()
                    .unwrap_or_else(|| generate_capability_secret().0);
                relay_config.transport_key_secret = Some(transport_secret.clone());
                let since = Arc::clone(&clock);
                let now: Arc<dyn Fn() -> UnixMillis + Send + Sync> = Arc::new(move || since.now());
                let admission = Admission::new(
                    tickets,
                    admission_store,
                    Arc::clone(&hosted) as Arc<dyn AdmissionHunts>,
                    Some(transport_secret),
                    Arc::clone(&now),
                );
                routes.push(Arc::new(AdmissionRoutes::new(
                    Arc::new(admission),
                    Some(resolver(Arc::clone(&caps), node.clone())),
                    seam_node.clone(),
                )));
                let grants = RelayGrants {
                    config: Arc::new(relay_config),
                    http: Arc::new(Unreachable),
                    usage: Arc::new(RelayUsageCache::default()),
                };
                let ice = Ice::new(
                    Arc::clone(&hosted) as Arc<dyn IceHunts>,
                    grants,
                    Arc::clone(&hosted) as Arc<dyn rifts_relay::OperationsPolicy>,
                    now,
                );
                routes.push(Arc::new(RelayRoutes::new(Arc::new(ice), seam_node.clone())));
                directory = Some(Arc::new(realm_directory));
                control = Some(Arc::new(ControlChannel::new(db.clone(), Arc::clone(&clock))));
                hosting = Some(hosted);
                capabilities = Some(caps);
            }
            _ => {
                routes.push(Arc::new(rifts_admission::stub_routes()));
                routes.push(Arc::new(rifts_relay::stub_routes()));
            }
        }
        let with_db = directory.is_some();
        let (guard_store, hunts): (Arc<dyn rifts_guard::GuardStore>, Arc<dyn rifts_guard::HuntSessions>) =
            match (&db, &capabilities) {
                (Some(db), Some(caps)) if with_db => (
                    Arc::new(db.guard_store()),
                    Arc::new(caps.hunt_sessions(Arc::clone(&clock))),
                ),
                _ => (Arc::new(StubGuardStore), Arc::new(NoHuntSessions)),
            };
        let guard = Arc::new(Guard::new(GuardParts {
            config: GuardConfig::from_settings(settings),
            store: guard_store,
            hunts,
            operations: Arc::new(CoreOperationsPolicy::new(Arc::clone(&core))),
            core: Arc::clone(&core),
            clock: Arc::new(rifts_guard::SystemClock::default()),
            log: Arc::new(rifts_guard::StderrLog),
        }));
        let service = Arc::new(RealmsService::new(RealmsServiceParts {
            directory: directory.clone(),
            capabilities: capabilities.clone(),
            hosting: hosting.clone(),
            clock: Arc::clone(&clock),
            core,
            policy,
            realms_enabled,
            node,
        }));
        routes.push(Arc::new(RealmsRoutes::new(RealmsRoutesParts {
            directory,
            control,
            hosting,
            capabilities,
            clock,
            realms_enabled,
        })));
        routes.push(Arc::new(GuardRoutes::with_store(Arc::clone(&guard), with_db)));
        RiftsParts {
            realms: service.clone(),
            entry: service,
            guard,
            routes,
        }
    }
}
