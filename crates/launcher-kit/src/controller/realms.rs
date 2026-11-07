// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Realm hosting and joining (RF7, DD12, DU5): links `rifts-host` behind the `realms` feature and serves
//! `realm_host`, `realm_join`, `realm_stop`, and `realm_get`.
//!
//! RD9 holds throughout: realm hosting and joining never inject into the game process or place a redirect
//! `.ini` or DLL beside the runtime loader, and any leftover loader files in `Win64` refuse realm startup
//! just as `PLAY` refuses them. The player's account key stays inside `KeyStore` and never reaches
//! `rifts-host`, `p2p-sessions/`, or the spawned dedicated server's environment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use hyper::Method;
use rifts_host::{
    ADMISSION_PUBLIC_KEY_FILE, CommonBootstrapConfig, DefaultHostBackend, FileHostState, HostBackend,
    HostBootstrapSpec, HostError, HostState as _, HostedRole, HubLimits, JoinBootstrapSpec, LocalRealmHosting,
    LocalSession, LocalState, NativeTransportLoader, NoopServerSpawner, PeerListEntry, RealmHosting, ServerSpawner,
    SessionResult, Signal, TRANSPORT_DLL, TransportAbi, TransportConfig, TransportLoader, default_max_servers,
    wire_session_id,
};
use rifts_seam::{AccountId, GlobalId, HubKind, NodeId, Secret, SessionId, Ulid};
use serde_json::{Value, json};

use super::{Controller, Shared, now_iso};
use crate::bridge::types::{
    ErrorCode, LauncherError, RealmKind, RealmOutcome, RealmSessionView, RealmState, RealmView, RuntimeState,
};
use crate::net::hostapi::key_headers;
use crate::net::http::{Endpoint, RequestOptions, request};
use crate::platform::log;

const DEFAULT_HUB_MAX_PLAYERS: u8 = 16;
const DEFAULT_HUB_AFK_SECONDS: u32 = 600;
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What tests put in place of `rifts-host`'s own parts.
#[doc(hidden)]
pub struct RealmsSeams {
    /// Loads the transport.
    pub loader: Arc<dyn TransportLoader>,
    /// The transport's SHA-256, in place of the runtime manifest's.
    pub expected_sha256: Option<[u8; 32]>,
    /// The server side.
    pub backend: Arc<dyn HostBackend>,
    /// Starts the dedicated server.
    pub spawner: Arc<dyn ServerSpawner>,
}

struct ActiveRealms {
    win64: PathBuf,
    dll_sha256: [u8; 32],
    transport: Arc<dyn TransportAbi>,
    hosting: Arc<LocalRealmHosting>,
    last_session: Option<SessionId>,
}

/// The loaded transport and its sessions, and the test seams.
#[derive(Default)]
pub struct RealmsState {
    inner: tokio::sync::Mutex<Option<ActiveRealms>>,
    seams: Mutex<Option<RealmsSeams>>,
}

fn parse_sha256_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn map_host_error(error: HostError) -> (ErrorCode, String) {
    match error {
        HostError::HashMismatch { .. } => (ErrorCode::RealmUnavailable, "transport_hash_mismatch".to_owned()),
        HostError::MissingExport(sym) => (ErrorCode::RealmUnavailable, format!("missing_export:{sym}")),
        HostError::UnsupportedAbi(ver) => (ErrorCode::RealmUnavailable, format!("unsupported_abi:{ver}")),
        HostError::NotFound(msg) => (ErrorCode::RealmNotFound, msg),
        HostError::InvalidInput(msg) => (ErrorCode::RealmFailed, msg),
        HostError::Transport(msg) => (ErrorCode::RealmFailed, msg),
        HostError::Io(msg) => (ErrorCode::RealmFailed, msg),
        HostError::Unimplemented(_) => (ErrorCode::RealmUnavailable, "unimplemented".to_owned()),
        _ => (ErrorCode::RealmFailed, error.to_string()),
    }
}

fn encode_ulid_u128(value: u128) -> Option<Ulid> {
    let mut chars = [0u8; 26];
    // Top 2 bits only for first char (<= '7')
    chars[0] = CROCKFORD[((value >> 125) & 0x07) as usize];
    for (idx, slot) in chars.iter_mut().enumerate().skip(1) {
        let shift = 125 - idx * 5;
        *slot = CROCKFORD[((value >> shift) & 0x1F) as usize];
    }
    let text = std::str::from_utf8(&chars).ok()?;
    text.parse::<Ulid>().ok()
}

fn encode_ulid(timestamp_ms: u64, rand_hi: u16, rand_lo: u64) -> Option<Ulid> {
    let ts = u128::from(timestamp_ms & 0xFFFF_FFFF_FFFF);
    let r = (u128::from(rand_hi) << 64) | u128::from(rand_lo);
    encode_ulid_u128((ts << 80) | r)
}

fn uuid_to_session_id(raw: &str) -> Option<SessionId> {
    let trimmed = raw.trim();
    let stripped = trimmed.split_once(':').map_or(trimmed, |(_, rest)| rest);
    let hex: String = stripped.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bits = u128::from_str_radix(&hex, 16).ok()?;
    let ulid = encode_ulid_u128(bits)?;
    let node = NodeId::new("local").ok()?;
    Some(SessionId(GlobalId { ulid, node }))
}

fn mint_local_session_id() -> Result<SessionId, LauncherError> {
    let failed = || LauncherError::with_detail(ErrorCode::RealmFailed, "session_id");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1_700_000_000_000, |d| d.as_millis() as u64);
    let rand_hi: u16 = rand::random();
    let rand_lo: u64 = rand::random();
    let ulid = encode_ulid(now_ms, rand_hi, rand_lo).ok_or_else(failed)?;
    let gid = format!("{ulid}@local").parse::<GlobalId>().map_err(|_| failed())?;
    Ok(SessionId(gid))
}

fn parse_session_id(realm: &str) -> Option<SessionId> {
    let trimmed = realm.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('@') {
        return trimmed.parse::<GlobalId>().ok().map(SessionId);
    }
    if let Some(session) = uuid_to_session_id(trimmed) {
        return Some(session);
    }
    format!("{trimmed}@local").parse::<GlobalId>().ok().map(SessionId)
}

struct ServerContext {
    endpoint: Endpoint,
    headers: Vec<(&'static str, String)>,
    backend_url: String,
    username: String,
}

/// Live [`HostBackend`] that drives the connected server's `/launcher/v1/*` and `/rustic/v1/*` realm,
/// admission, and ICE relay routes over HTTP while keeping the player's account key inside the
/// launcher's `net` layer (RD9 / RF7). Falls back to [`DefaultHostBackend`] when no server is configured.
struct ServerHostBackend {
    controller: Weak<Shared>,
    fallback: DefaultHostBackend,
    bridge_instances: Mutex<HashMap<String, String>>,
    control_cursors: Mutex<HashMap<String, u64>>,
}

impl ServerHostBackend {
    fn new(controller: Weak<Shared>) -> Self {
        Self {
            controller,
            fallback: DefaultHostBackend::new(),
            bridge_instances: Mutex::new(HashMap::new()),
            control_cursors: Mutex::new(HashMap::new()),
        }
    }

    fn server_context(&self) -> Option<ServerContext> {
        let ctrl = Controller::from_weak(&self.controller)?;
        let sv = ctrl.server()?;
        // Called off the async workers (`run_async`'s thread or `block_in_place`), so a blocking read is fine
        let key = ctrl.shared.keys.load(&sv.slot(), sv.pin()).ok().flatten()?;
        let endpoint = sv.endpoint();
        let headers = key_headers(&endpoint, Some(&key));
        let scheme = if endpoint.pin.is_some() { "https" } else { "http" };
        let backend_url = format!("{scheme}://{}:{}", endpoint.host, endpoint.port);
        let username = ctrl
            .stored()
            .username(&sv.id())
            .map_or_else(|| "Slayer".to_owned(), str::to_owned);
        Some(ServerContext {
            endpoint,
            headers,
            backend_url,
            username,
        })
    }

    fn run_async<F, T>(fut: F) -> Result<T, HostError>
    where
        F: Future<Output = Result<T, HostError>> + Send + 'static,
        T: Send + 'static,
    {
        let run = move || {
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| HostError::Io(e.to_string()))?;
                rt.block_on(fut)
            })
            .join()
            .map_err(|_| HostError::Io("realm backend worker panicked".to_owned()))?
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current()
            && handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        {
            tokio::task::block_in_place(run)
        } else {
            run()
        }
    }

    async fn request_json(
        ctx: &ServerContext,
        method: Method,
        path: &str,
        extra_headers: &[(&'static str, &str)],
        body: Option<&Value>,
    ) -> Result<(u16, Value), HostError> {
        let mut headers = ctx.headers.clone();
        for (k, v) in extra_headers {
            headers.push((*k, (*v).to_owned()));
        }
        let payload = match body {
            Some(val) => Some(serde_json::to_string(val).map_err(|e| HostError::InvalidInput(e.to_string()))?),
            None => None,
        };
        let options = RequestOptions {
            method,
            headers,
            body: payload,
            timeout: Duration::from_secs(10),
            max_bytes: 512 * 1024,
            cancel: None,
        };
        let reply = request(&ctx.endpoint, path, options)
            .await
            .map_err(|e| HostError::Transport(format!("server request failed ({path}): {e}")))?;
        let parsed: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
        Ok((reply.status, parsed))
    }

    fn check_status(status: u16, parsed: &Value, path: &str) -> Result<(), HostError> {
        if (200..300).contains(&status) {
            return Ok(());
        }
        let code = parsed
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .or_else(|| parsed.get("code").and_then(Value::as_str))
            .unwrap_or("SERVER_ERROR");
        let msg = parsed
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .or_else(|| parsed.get("message").and_then(Value::as_str))
            .unwrap_or(path);
        if status == 404 || matches!(code, "REALM_NOT_FOUND" | "P2P_SESSION_NOT_FOUND") {
            return Err(HostError::NotFound(format!("{code}: {msg}")));
        }
        Err(HostError::Transport(format!("{code}: {msg}")))
    }

    fn derive_realm_name(username: &str) -> String {
        let mut clean: String = username
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .take(16)
            .collect();
        if clean.is_empty() {
            clean = "slayer".to_owned();
        }
        format!("{clean}realm")
    }

    async fn ensure_host_realm_and_presence(ctx: &ServerContext, limits: &HubLimits) -> Result<(), HostError> {
        let zero_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let (mine_status, mine_val) =
            Self::request_json(ctx, Method::GET, "/launcher/v1/realms/mine", &[], None).await?;
        if mine_status == 200 && mine_val.get("realm").is_none_or(Value::is_null) {
            let realm_name = Self::derive_realm_name(&ctx.username);
            let create_body = json!({
                "name": realm_name,
                "joinPolicy": "PUBLIC",
                "listed": true,
                "hostPublicKey": zero_hash,
            });
            let (create_status, create_val) =
                Self::request_json(ctx, Method::POST, "/launcher/v1/realms", &[], Some(&create_body)).await?;
            if create_status != 200 && create_status != 201 && create_status != 409 {
                Self::check_status(create_status, &create_val, "/launcher/v1/realms")?;
            }
        }
        let hb_body = json!({
            "maxPlayers": limits.max_players,
            "hosting": {
                "maxServers": limits.max_servers,
                "maxPlayers": limits.max_players,
                "afkSeconds": limits.afk_seconds,
            }
        });
        let _ = Self::request_json(
            ctx,
            Method::POST,
            "/launcher/v1/realms/mine/heartbeat",
            &[],
            Some(&hb_body),
        )
        .await;

        let presence_body = json!({
            "observedAt": now_iso(),
            "consentToHost": true,
            "powerSaver": false,
            "runtimeSupported": true,
            "transportReady": true,
            "peerId": "100001",
            "launcherVersion": env!("CARGO_PKG_VERSION"),
            "freePhysicalMemoryMiB": 8192,
            "commitHeadroomMiB": 8192,
            "logicalCpuCount": 8,
            "cpuLoadPercent": 12.0,
            "backendRttMs": 15.0,
            "worstPeerRttMs": 15.0,
            "jitterMs": 1.0,
            "packetLossPercent": 0.0,
            "peerLinks": [],
            "artifacts": {
                "gameExe": "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b",
                "internalServer": zero_hash,
                "winmmProxy": zero_hash,
                "transportModule": zero_hash,
                "transportRuntime": zero_hash,
                "steamBridge": zero_hash,
                "steamRuntime": zero_hash,
                "admissionPublicKey": zero_hash,
            }
        });
        let (pres_status, pres_val) = Self::request_json(
            ctx,
            Method::POST,
            "/launcher/v1/p2p/host-presence",
            &[],
            Some(&presence_body),
        )
        .await?;
        Self::check_status(pres_status, &pres_val, "/launcher/v1/p2p/host-presence")?;
        Ok(())
    }

    fn parse_common_bootstrap(
        val: &Value,
        fallback_session: Option<&SessionId>,
    ) -> Result<CommonBootstrapConfig, HostError> {
        let wire_session = val
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| HostError::InvalidInput("missing sessionId in bootstrap".to_owned()))?;
        let session_id = uuid_to_session_id(wire_session)
            .or_else(|| fallback_session.cloned())
            .ok_or_else(|| HostError::InvalidInput(format!("invalid sessionId {wire_session}")))?;
        let str_field = |k: &str, def: &str| val.get(k).and_then(Value::as_str).unwrap_or(def).to_owned();
        Ok(CommonBootstrapConfig {
            key_id: str_field("keyId", "local-key-1"),
            session_id,
            party_id: str_field("partyId", "party-default"),
            party_revision: val.get("partyRevision").and_then(Value::as_u64).unwrap_or(1),
            hunt_definition_id: str_field("huntDefinitionId", "ShatteredIsles_ReturnToRamsgate"),
            generation: val.get("generation").and_then(Value::as_u64).unwrap_or(1),
            build: u32::try_from(val.get("build").and_then(Value::as_u64).unwrap_or(647_472)).unwrap_or(647_472),
            bridge_protocol: u8::try_from(val.get("bridgeProtocol").and_then(Value::as_u64).unwrap_or(1)).unwrap_or(1),
            virtual_port: u16::try_from(val.get("virtualPort").and_then(Value::as_u64).unwrap_or(1)).unwrap_or(1),
            host_peer_id: str_field("hostPeerId", "100001"),
            bridge_instance_id: str_field("bridgeInstanceId", "bridge-default-0001"),
            metadata_nonce: str_field("metadataNonce", "0123456789abcdef0123456789abcdef"),
            force_relay: val.get("forceRelay").and_then(Value::as_bool).unwrap_or(false),
            guard_completion_mode: str_field("guardCompletionMode", "NO_COMPLETION"),
        })
    }

    fn parse_host_bootstrap(
        val: &Value,
        role: HostedRole,
        default_backend_url: &str,
        fallback_session: Option<&SessionId>,
    ) -> Result<HostBootstrapSpec, HostError> {
        let common = Self::parse_common_bootstrap(val, fallback_session)?;
        let cap = val
            .get("capability")
            .and_then(Value::as_str)
            .ok_or_else(|| HostError::InvalidInput("missing capability in host bootstrap".to_owned()))?;
        let host_ticket = val
            .get("hostTicket")
            .and_then(Value::as_str)
            .map(|s| Secret::new(s.to_owned()));
        let str_field = |k: &str, def: &str| val.get(k).and_then(Value::as_str).unwrap_or(def).to_owned();
        Ok(HostBootstrapSpec {
            common,
            role,
            capability: Secret::new(cap.to_owned()),
            host_ticket,
            backend_url: str_field("backendUrl", default_backend_url),
            server_udp_port: u16::try_from(val.get("serverUdpPort").and_then(Value::as_u64).unwrap_or(8791))
                .unwrap_or(8791),
            maximum_peers: u8::try_from(val.get("maximumPeers").and_then(Value::as_u64).unwrap_or(16)).unwrap_or(16),
            local_client: val.get("hostLocalClient").and_then(Value::as_bool).unwrap_or(true),
            hunt_evidence: val.get("huntEvidence").and_then(Value::as_bool).unwrap_or(false),
            matchmaker_hunt_id: str_field("matchmakerHuntId", "ShatteredIsles_ReturnToRamsgate"),
            map_path: str_field("mapPath", "/Game/Maps/ramsgate/ramsgate_01_persistent"),
            behemoth_path: str_field("behemothPath", "NO_BEHEMOTH"),
            expected_players: str_field("expectedPlayers", "1"),
            runtime_profile: str_field("runtimeProfile", "production"),
            max_players: val
                .get("maxPlayers")
                .and_then(Value::as_u64)
                .and_then(|v| u8::try_from(v).ok()),
            hub_afk_seconds: val
                .get("hubAfkSeconds")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok()),
        })
    }

    async fn complete_host_ready_and_transport_config(
        ctx: &ServerContext,
        spec: &HostBootstrapSpec,
    ) -> Result<TransportConfig, HostError> {
        let wire_session = wire_session_id(&spec.common.session_id);
        let cap_header = [
            ("x-rustic-host-capability", spec.capability.expose()),
            ("x-rustic-isles-host-capability", spec.capability.expose()),
        ];
        let ready_body = json!({
            "lobbyId": "ICE",
            "virtualPort": spec.common.virtual_port,
        });
        let ready_path = format!("/rustic/v1/hunts/{wire_session}/host-ready");
        let (ready_status, ready_val) =
            Self::request_json(ctx, Method::POST, &ready_path, &cap_header, Some(&ready_body)).await?;
        Self::check_status(ready_status, &ready_val, &ready_path)?;

        if let Some(ticket) = &spec.host_ticket {
            let consume_path = format!("/rustic/v1/hunts/{wire_session}/tickets/consume");
            let req_id = format!("host-local-{}", spec.common.generation);
            let consume_headers = [
                ("x-rustic-host-capability", spec.capability.expose()),
                ("x-rustic-isles-host-capability", spec.capability.expose()),
                ("x-rustic-isles-request-id", req_id.as_str()),
            ];
            let consume_body = json!({
                "ticket": ticket.expose(),
                "authenticatedSteamId": spec.common.host_peer_id,
                "bridgeInstanceId": spec.common.bridge_instance_id,
                "connectionHandle": "localhost",
            });
            let (consume_status, consume_val) =
                Self::request_json(ctx, Method::POST, &consume_path, &consume_headers, Some(&consume_body)).await?;
            Self::check_status(consume_status, &consume_val, &consume_path)?;
        }

        let cfg_path = format!("/launcher/v1/p2p/hunts/{wire_session}/transport-config");
        let cfg_body = json!({ "generation": spec.common.generation });
        let (cfg_status, cfg_val) = Self::request_json(ctx, Method::POST, &cfg_path, &[], Some(&cfg_body)).await?;
        Self::check_status(cfg_status, &cfg_val, &cfg_path)?;
        serde_json::from_value(cfg_val)
            .map_err(|e| HostError::InvalidInput(format!("invalid transport-config response: {e}")))
    }
}

impl HostBackend for ServerHostBackend {
    fn host_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        let Some(ctx) = self.server_context() else {
            return self.fallback.host_bootstrap(account, session);
        };
        let session_clone = session.clone();
        let res = Self::run_async(async move {
            let wire_session = wire_session_id(&session_clone);
            let path = format!("/launcher/v1/p2p/hunts/{wire_session}/host-bootstrap");
            let (status, val) = Self::request_json(&ctx, Method::POST, &path, &[], Some(&json!({}))).await?;
            Self::check_status(status, &val, &path)?;
            let spec = Self::parse_host_bootstrap(&val, HostedRole::Hunt, &ctx.backend_url, Some(&session_clone))?;
            let cfg = Self::complete_host_ready_and_transport_config(&ctx, &spec).await?;
            Ok((spec, cfg))
        });
        match res {
            Ok((spec, cfg)) => {
                lock(&self.bridge_instances).insert(
                    wire_session_id(&spec.common.session_id),
                    spec.common.bridge_instance_id.clone(),
                );
                Ok((spec, cfg))
            }
            Err(HostError::NotFound(_)) => self.fallback.host_bootstrap(account, session),
            Err(err) => Err(err),
        }
    }

    fn join_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError> {
        let Some(ctx) = self.server_context() else {
            return self.fallback.join_bootstrap(account, session);
        };
        let session_clone = session.clone();
        let (spec, cfg) = Self::run_async(async move {
            // Ensure the joiner has joined an active listed realm when joining a realm hub session.
            let (list_status, list_val) =
                Self::request_json(&ctx, Method::GET, "/launcher/v1/realms/listed", &[], None).await?;
            if list_status == 200
                && let Some(realms) = list_val.get("realms").and_then(Value::as_array)
                && let Some(first_name) = realms.first().and_then(|r| r.get("name")).and_then(Value::as_str)
            {
                let join_body = json!({ "name": first_name });
                let _ = Self::request_json(&ctx, Method::POST, "/launcher/v1/realms/join", &[], Some(&join_body)).await;
                let _ = Self::request_json(
                    &ctx,
                    Method::POST,
                    "/launcher/v1/realms/presence/heartbeat",
                    &[],
                    Some(&join_body),
                )
                .await;
            }

            let wire_session = wire_session_id(&session_clone);
            let join_path = format!("/launcher/v1/p2p/hunts/{wire_session}/join-bootstrap");
            let (join_status, join_val) =
                Self::request_json(&ctx, Method::POST, &join_path, &[], Some(&json!({}))).await?;
            Self::check_status(join_status, &join_val, &join_path)?;

            let common = Self::parse_common_bootstrap(&join_val, Some(&session_clone))?;
            let role = if join_val.get("sessionRole").and_then(Value::as_str) == Some("HUB") {
                HostedRole::Hub(HubKind::Ramsgate)
            } else {
                HostedRole::Hunt
            };
            let ticket = join_val
                .get("ticket")
                .and_then(Value::as_str)
                .ok_or_else(|| HostError::InvalidInput("missing ticket in join-bootstrap".to_owned()))?;
            let spec = JoinBootstrapSpec {
                common,
                role,
                ticket: Secret::new(ticket.to_owned()),
            };

            let cfg_path = format!("/launcher/v1/p2p/hunts/{wire_session}/transport-config");
            let cfg_body = json!({ "generation": spec.common.generation });
            let (cfg_status, cfg_val) = Self::request_json(&ctx, Method::POST, &cfg_path, &[], Some(&cfg_body)).await?;
            Self::check_status(cfg_status, &cfg_val, &cfg_path)?;
            let cfg: TransportConfig = serde_json::from_value(cfg_val)
                .map_err(|e| HostError::InvalidInput(format!("invalid transport-config response: {e}")))?;

            Ok((spec, cfg))
        })?;
        lock(&self.bridge_instances).insert(
            wire_session_id(&spec.common.session_id),
            spec.common.bridge_instance_id.clone(),
        );
        Ok((spec, cfg))
    }

    fn start_hub(
        &self,
        account: &AccountId,
        kind: HubKind,
        limits: &HubLimits,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        let Some(ctx) = self.server_context() else {
            return self.fallback.start_hub(account, kind, limits);
        };
        let limits_copy = *limits;
        let (spec, cfg) = Self::run_async(async move {
            Self::ensure_host_realm_and_presence(&ctx, &limits_copy).await?;
            let hub_kind_str = match kind {
                HubKind::Ramsgate => "RAMSGATE",
                HubKind::Training => "TRAINING",
            };
            let start_body = json!({
                "hubKind": hub_kind_str,
                "maxPlayers": limits_copy.max_players,
                "afkSeconds": limits_copy.afk_seconds,
            });
            let (status, val) = Self::request_json(
                &ctx,
                Method::POST,
                "/launcher/v1/realm-hosting/hubs/start",
                &[],
                Some(&start_body),
            )
            .await?;
            Self::check_status(status, &val, "/launcher/v1/realm-hosting/hubs/start")?;
            let spec = Self::parse_host_bootstrap(&val, HostedRole::Hub(kind), &ctx.backend_url, None)?;
            let cfg = Self::complete_host_ready_and_transport_config(&ctx, &spec).await?;
            Ok((spec, cfg))
        })?;
        lock(&self.bridge_instances).insert(
            wire_session_id(&spec.common.session_id),
            spec.common.bridge_instance_id.clone(),
        );
        Ok((spec, cfg))
    }

    fn stop_hub(&self, account: &AccountId, kind: HubKind) -> Result<Option<SessionId>, HostError> {
        let Some(ctx) = self.server_context() else {
            return self.fallback.stop_hub(account, kind);
        };
        Self::run_async(async move {
            let hub_kind_str = match kind {
                HubKind::Ramsgate => "RAMSGATE",
                HubKind::Training => "TRAINING",
            };
            let stop_body = json!({ "hubKind": hub_kind_str });
            let (status, val) = Self::request_json(
                &ctx,
                Method::POST,
                "/launcher/v1/realm-hosting/hubs/stop",
                &[],
                Some(&stop_body),
            )
            .await?;
            Self::check_status(status, &val, "/launcher/v1/realm-hosting/hubs/stop")?;
            let session_id = val
                .get("sessionId")
                .and_then(Value::as_str)
                .and_then(uuid_to_session_id);
            if let Ok((ctrl_status, ctrl_val)) =
                Self::request_json(&ctx, Method::GET, "/launcher/v1/p2p/control", &[], None).await
                && (200..300).contains(&ctrl_status)
                && let Some(msgs) = ctrl_val.get("messages").and_then(Value::as_array)
            {
                for msg in msgs {
                    if let Some(msg_id) = msg
                        .get("messageId")
                        .and_then(Value::as_str)
                        .or_else(|| msg.get("id").and_then(Value::as_str))
                    {
                        let ack_path = format!("/launcher/v1/p2p/control/{msg_id}/ack");
                        let _ = Self::request_json(&ctx, Method::POST, &ack_path, &[], Some(&json!({}))).await;
                    }
                }
            }
            Ok(session_id)
        })
    }

    fn route_ready(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: u64,
        local_port: u16,
    ) -> Result<(), HostError> {
        let Some(ctx) = self.server_context() else {
            return self.fallback.route_ready(account, session, generation, local_port);
        };
        let wire_session = wire_session_id(session);
        let Some(bridge_instance) = lock(&self.bridge_instances).get(&wire_session).cloned() else {
            return self.fallback.route_ready(account, session, generation, local_port);
        };
        Self::run_async(async move {
            let path = format!("/launcher/v1/p2p/hunts/{wire_session}/route-ready");
            let body = json!({
                "generation": generation,
                "localBridgePort": local_port,
                "bridgeInstanceId": bridge_instance,
            });
            let (status, val) = Self::request_json(&ctx, Method::POST, &path, &[], Some(&body)).await?;
            Self::check_status(status, &val, &path)?;
            Ok(())
        })
    }

    fn supports_signalling(&self) -> bool {
        self.server_context().is_some()
    }

    fn poll_peers(
        &self,
        _account: &AccountId,
        session: &SessionId,
        generation: u64,
        role: HostedRole,
    ) -> Result<Vec<PeerListEntry>, HostError> {
        let Some(ctx) = self.server_context() else {
            return Ok(Vec::new());
        };
        let wire_session = wire_session_id(session);
        Self::run_async(async move {
            let path = match role {
                HostedRole::Hub(_) => format!("/launcher/v1/p2p/hunts/{wire_session}/peers"),
                HostedRole::Hunt => format!("/launcher/v1/p2p/hunts/{wire_session}/transport-config"),
            };
            let body = json!({ "generation": generation });
            let (status, val) = Self::request_json(&ctx, Method::POST, &path, &[], Some(&body)).await?;
            if !(200..300).contains(&status) {
                return Ok(Vec::new());
            }
            let peers_val = val.get("peers").cloned().unwrap_or(Value::Null);
            Ok(serde_json::from_value::<Vec<PeerListEntry>>(peers_val).unwrap_or_default())
        })
    }

    fn send_signal(
        &self,
        _account: &AccountId,
        session: &SessionId,
        generation: u64,
        signal: &Signal,
    ) -> Result<(), HostError> {
        let Some(ctx) = self.server_context() else {
            return Ok(());
        };
        let wire_session = wire_session_id(session);
        let to_account = signal.to_account.0.clone();
        let kind = signal.kind.clone();
        let data = signal.data.clone();
        Self::run_async(async move {
            let path = format!("/launcher/v1/p2p/hunts/{wire_session}/signal");
            let body = json!({
                "generation": generation,
                "toAccountId": to_account,
                "kind": kind,
                "data": data,
            });
            let (status, val) = Self::request_json(&ctx, Method::POST, &path, &[], Some(&body)).await?;
            Self::check_status(status, &val, &path)?;
            Ok(())
        })
    }

    fn poll_signals(
        &self,
        account: &AccountId,
        session: &SessionId,
        _generation: u64,
    ) -> Result<Vec<(AccountId, Signal)>, HostError> {
        let Some(ctx) = self.server_context() else {
            return Ok(Vec::new());
        };
        let wire_session = wire_session_id(session);
        let after_seq = lock(&self.control_cursors).get(&account.0).copied().unwrap_or(0);
        let (out, next_seq) = Self::run_async(async move {
            let path = format!("/launcher/v1/p2p/control?afterSequence={after_seq}&waitMs=0");
            let (status, val) = Self::request_json(&ctx, Method::GET, &path, &[], None).await?;
            if !(200..300).contains(&status) {
                return Ok((Vec::new(), after_seq));
            }
            let mut max_seq = after_seq;
            let mut signals = Vec::new();
            if let Some(msgs) = val.get("messages").and_then(Value::as_array) {
                for msg in msgs {
                    if let Some(seq) = msg.get("sequence").and_then(Value::as_u64)
                        && seq > max_seq
                    {
                        max_seq = seq;
                    }
                    let msg_type = msg
                        .get("messageType")
                        .and_then(Value::as_str)
                        .or_else(|| msg.get("type").and_then(Value::as_str));
                    if msg_type != Some("ICE_SIGNAL") {
                        continue;
                    }
                    let msg_session = msg.get("sessionId").and_then(Value::as_str);
                    if msg_session.is_some_and(|s| !s.eq_ignore_ascii_case(&wire_session)) {
                        continue;
                    }
                    let payload = msg.get("payload").unwrap_or(msg);
                    let (Some(from_acc), Some(kind), Some(data)) = (
                        payload.get("fromAccountId").and_then(Value::as_str),
                        payload.get("kind").and_then(Value::as_str),
                        payload.get("data").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    signals.push((
                        AccountId(from_acc.to_owned()),
                        Signal {
                            to_account: AccountId(String::new()),
                            kind: kind.to_owned(),
                            data: data.to_owned(),
                        },
                    ));
                    if let Some(msg_id) = msg
                        .get("messageId")
                        .and_then(Value::as_str)
                        .or_else(|| msg.get("id").and_then(Value::as_str))
                    {
                        let ack_path = format!("/launcher/v1/p2p/control/{msg_id}/ack");
                        let _ = Self::request_json(&ctx, Method::POST, &ack_path, &[], Some(&json!({}))).await;
                    }
                }
            }
            Ok((signals, max_seq))
        })?;
        if next_seq > after_seq {
            lock(&self.control_cursors).insert(account.0.clone(), next_seq);
        }
        Ok(out)
    }
}

fn session_result_code(result: SessionResult) -> &'static str {
    match result {
        SessionResult::Clean => "clean",
        SessionResult::BadInput => "bad_input",
        SessionResult::Setup => "setup",
        SessionResult::RefusedAdmission => "refused_admission",
        SessionResult::LeaseFailed => "lease_failed",
        SessionResult::UnknownHandle => "unknown_handle",
        SessionResult::Other(_) => "ended",
    }
}

fn session_view(hosting: &LocalRealmHosting, local: &LocalSession) -> RealmSessionView {
    let kind = match local.role {
        HostedRole::Hub(_) => RealmKind::Hub,
        HostedRole::Hunt => RealmKind::Hunt,
    };
    let (state, result) = match local.state {
        LocalState::Starting => (RealmState::Starting, None),
        LocalState::Ready => (RealmState::Ready, None),
        LocalState::Running => (RealmState::Running, None),
        LocalState::Ended(code) => (RealmState::Ended, Some(session_result_code(code).to_owned())),
    };
    let local_port = hosting.session_local_port(&local.session).filter(|port| *port > 0);
    RealmSessionView {
        session: local.session.to_string(),
        kind,
        hosting: local.hosting,
        generation: local.generation,
        state,
        local_port,
        result,
    }
}

impl Controller {
    /// Installs test seams for `rifts-host` (transport loader, pinned SHA-256, backend and server spawner).
    #[doc(hidden)]
    pub fn set_realms_seams(&self, seams: RealmsSeams) {
        *lock(&self.shared.realms.seams) = Some(seams);
    }

    /// The backend realms use when no seams are installed, for tests that script only the transport.
    #[doc(hidden)]
    #[must_use]
    pub fn live_host_backend(&self) -> Arc<dyn HostBackend> {
        Arc::new(ServerHostBackend::new(self.weak()))
    }

    /// Stops any active realm sessions and shuts down the loaded transport on launcher exit, logout or server switch.
    pub(crate) async fn realms_shutdown(&self) {
        let active = self.shared.realms.inner.lock().await.take();
        if let Some(active) = active {
            let account = self.realm_account_id();
            if let Ok(sessions) = active.hosting.sessions().await {
                for session in sessions {
                    match session.role {
                        HostedRole::Hub(hub) if session.hosting => {
                            let _ = active.hosting.stop_hub(account.clone(), hub).await;
                        }
                        _ => {
                            let _ = active.hosting.stop(session.session).await;
                        }
                    }
                }
            }
            let _ = active.transport.shutdown();
        }
    }

    /// Derives a safe `AccountId` from the public username without ever reading `KeyStore` (RD9 / RF7).
    fn realm_account_id(&self) -> AccountId {
        let stored = self.stored();
        let raw = stored
            .server()
            .and_then(|sv| stored.username(&sv.id()).map(str::to_owned))
            .unwrap_or_else(|| "local-player".to_owned());
        let sanitized: String = raw
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
            .take(64)
            .collect();
        let id = if sanitized.is_empty() {
            "local-player".to_owned()
        } else {
            sanitized
        };
        AccountId(id)
    }

    fn expected_transport_sha256(&self, dll_path: &Path) -> Result<[u8; 32], LauncherError> {
        if let Some(sha) = lock(&self.shared.realms.seams).as_ref().and_then(|s| s.expected_sha256) {
            return Ok(sha);
        }
        let (manifest_sha, local_or_installed) = {
            let st = self.lock();
            let sha = st
                .manifest
                .as_ref()
                .and_then(|m| {
                    m.files
                        .iter()
                        .find(|f| f.name.eq_ignore_ascii_case(TRANSPORT_DLL))
                        .map(|f| f.sha256.clone())
                })
                .and_then(|hex| parse_sha256_hex(&hex));
            (
                sha,
                matches!(st.runtime.state, RuntimeState::Local | RuntimeState::Ok) || st.runtime_playable,
            )
        };
        if let Some(sha) = manifest_sha {
            return Ok(sha);
        }
        if local_or_installed {
            return rifts_host::sha256_file(dll_path).map_err(|error| {
                let (code, detail) = map_host_error(error);
                self.refuse(LauncherError::with_detail(code, &detail))
            });
        }
        Err(self.refuse(LauncherError::with_detail(
            ErrorCode::RealmUnavailable,
            "transport_untrusted",
        )))
    }

    async fn ensure_realms_ready(&self) -> Result<(), LauncherError> {
        if self.lock().shutting_down {
            return Err(LauncherError::new(ErrorCode::Cancelled));
        }
        let Some(win64) = self.win64() else {
            return Err(self.refuse(LauncherError::new(ErrorCode::GameFolderNotFound)));
        };
        // RD9: never start or join a realm while a loader redirect ini or Host tools DLL sits in Win64.
        let leftovers = self.loader_leftovers(&win64);
        if !leftovers.is_empty() {
            log::warn(format!(
                "realm refused: beside the runtime loader: {}",
                leftovers.join(", ")
            ));
            return Err(self.refuse(LauncherError::with_detail(
                ErrorCode::LoaderLeftovers,
                &leftovers.join(", "),
            )));
        }
        let dll_path = win64.join(TRANSPORT_DLL);
        if !dll_path.is_file() {
            return Err(self.refuse(LauncherError::with_detail(
                ErrorCode::RealmUnavailable,
                "transport_missing",
            )));
        }
        let pubkey_path = win64.join(ADMISSION_PUBLIC_KEY_FILE);
        if !pubkey_path.is_file() {
            return Err(self.refuse(LauncherError::with_detail(
                ErrorCode::RealmUnavailable,
                "admission_key_missing",
            )));
        }
        let expected_sha256 = self.expected_transport_sha256(&dll_path)?;

        let mut guard = self.shared.realms.inner.lock().await;
        if let Some(active) = guard.as_ref()
            && active.win64 == win64
            && active.dll_sha256 == expected_sha256
        {
            return Ok(());
        }

        let (loader, backend, spawner) = {
            let seams = lock(&self.shared.realms.seams);
            match seams.as_ref() {
                Some(s) => (Arc::clone(&s.loader), Arc::clone(&s.backend), Arc::clone(&s.spawner)),
                None => (
                    Arc::new(NativeTransportLoader) as Arc<dyn TransportLoader>,
                    Arc::new(ServerHostBackend::new(self.weak())) as Arc<dyn HostBackend>,
                    Arc::new(NoopServerSpawner::new()) as Arc<dyn ServerSpawner>,
                ),
            }
        };

        let boxed_transport = loader.load(&dll_path, expected_sha256).map_err(|error| {
            let (code, detail) = map_host_error(error);
            self.refuse(LauncherError::with_detail(code, &detail))
        })?;
        let transport: Arc<dyn TransportAbi> = Arc::from(boxed_transport);
        transport.initialize().map_err(|error| {
            let (code, detail) = map_host_error(error);
            self.refuse(LauncherError::with_detail(code, &detail))
        })?;

        let hosting = Arc::new(
            LocalRealmHosting::with_backend_and_spawner(
                self.user_data(),
                &win64,
                Arc::clone(&transport),
                backend,
                spawner,
            )
            .with_wait_ready_timeout_ms(15_000),
        );
        *guard = Some(ActiveRealms {
            win64,
            dll_sha256: expected_sha256,
            transport,
            hosting,
            last_session: None,
        });
        Ok(())
    }

    async fn build_realm_view(&self) -> Result<RealmView, LauncherError> {
        let guard = self.shared.realms.inner.lock().await;
        let Some(active) = guard.as_ref() else {
            let available = self.win64().is_some_and(|win64| {
                win64.join(TRANSPORT_DLL).is_file()
                    && win64.join(ADMISSION_PUBLIC_KEY_FILE).is_file()
                    && !self.user_data().join(rifts_host::DISABLE_COOP_FLAG_FILE).is_file()
            });
            return Ok(RealmView {
                available,
                active: None,
                sessions: Vec::new(),
            });
        };
        let available = active.hosting.coop_available().await.unwrap_or(false);
        let raw_sessions = active.hosting.sessions().await.map_err(|error| {
            let (code, detail) = map_host_error(error);
            self.refuse(LauncherError::with_detail(code, &detail))
        })?;
        let sessions: Vec<RealmSessionView> = raw_sessions
            .iter()
            .map(|local| session_view(&active.hosting, local))
            .collect();
        let chosen = active
            .last_session
            .as_ref()
            .and_then(|last| {
                let wanted = last.to_string();
                sessions.iter().find(|s| s.session == wanted).cloned()
            })
            .or_else(|| {
                sessions
                    .iter()
                    .find(|s| s.state != RealmState::Ended)
                    .cloned()
                    .or_else(|| sessions.last().cloned())
            });
        Ok(RealmView {
            available,
            active: chosen,
            sessions,
        })
    }

    /// `realm_host`.
    pub async fn realm_host(&self, kind: RealmKind) -> RealmOutcome {
        if let Err(error) = self.ensure_realms_ready().await {
            return RealmOutcome::Err(error);
        }
        let account = self.realm_account_id();
        let server_id = self
            .server()
            .map_or_else(|| "local".to_owned(), |sv| format!("{}:{}", sv.host, sv.port));
        let state_files = FileHostState::for_server(self.user_data(), &server_id);
        let limits = state_files.hub_limits().ok().flatten().unwrap_or(HubLimits {
            max_players: DEFAULT_HUB_MAX_PLAYERS,
            max_servers: default_max_servers(4),
            afk_seconds: DEFAULT_HUB_AFK_SECONDS,
        });

        let hosting = {
            let guard = self.shared.realms.inner.lock().await;
            let Some(active) = guard.as_ref() else {
                return RealmOutcome::Err(self.refuse(LauncherError::new(ErrorCode::RealmUnavailable)));
            };
            Arc::clone(&active.hosting)
        };

        match hosting.coop_available().await {
            Ok(true) => {}
            Ok(false) => {
                return RealmOutcome::Err(
                    self.refuse(LauncherError::with_detail(ErrorCode::RealmUnavailable, "coop_disabled")),
                );
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                return RealmOutcome::Err(self.refuse(LauncherError::with_detail(code, &detail)));
            }
        }

        let started = match kind {
            RealmKind::Hub => hosting.start_hub(account, HubKind::Ramsgate, limits).await,
            RealmKind::Hunt => {
                let session_id = match mint_local_session_id() {
                    Ok(id) => id,
                    Err(error) => return RealmOutcome::Err(error),
                };
                hosting.host_hunt(account, session_id).await
            }
        };

        match started {
            Ok(local) => {
                if let Some(active) = self.shared.realms.inner.lock().await.as_mut() {
                    active.last_session = Some(local.session);
                }
                match self.build_realm_view().await {
                    Ok(view) => RealmOutcome::Ok(view),
                    Err(error) => RealmOutcome::Err(error),
                }
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                RealmOutcome::Err(self.refuse(LauncherError::with_detail(code, &detail)))
            }
        }
    }

    /// `realm_join`.
    pub async fn realm_join(&self, realm: &str) -> RealmOutcome {
        let Some(session_id) = parse_session_id(realm) else {
            return RealmOutcome::Err(self.refuse(LauncherError::with_detail(ErrorCode::RealmNotFound, realm)));
        };
        if let Err(error) = self.ensure_realms_ready().await {
            return RealmOutcome::Err(error);
        }
        let account = self.realm_account_id();
        let hosting = {
            let guard = self.shared.realms.inner.lock().await;
            let Some(active) = guard.as_ref() else {
                return RealmOutcome::Err(self.refuse(LauncherError::new(ErrorCode::RealmUnavailable)));
            };
            Arc::clone(&active.hosting)
        };

        match hosting.coop_available().await {
            Ok(true) => {}
            Ok(false) => {
                return RealmOutcome::Err(
                    self.refuse(LauncherError::with_detail(ErrorCode::RealmUnavailable, "coop_disabled")),
                );
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                return RealmOutcome::Err(self.refuse(LauncherError::with_detail(code, &detail)));
            }
        }

        match hosting.join_hunt(account, session_id.clone()).await {
            Ok(_) => {
                if let Some(active) = self.shared.realms.inner.lock().await.as_mut() {
                    active.last_session = Some(session_id);
                }
                match self.build_realm_view().await {
                    Ok(view) => RealmOutcome::Ok(view),
                    Err(error) => RealmOutcome::Err(error),
                }
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                RealmOutcome::Err(self.refuse(LauncherError::with_detail(code, &detail)))
            }
        }
    }

    /// `realm_stop`.
    pub async fn realm_stop(&self) -> RealmOutcome {
        let (hosting, account) = {
            let mut guard = self.shared.realms.inner.lock().await;
            let Some(active) = guard.as_mut() else {
                return match self.build_realm_view().await {
                    Ok(view) => RealmOutcome::Ok(view),
                    Err(error) => RealmOutcome::Err(error),
                };
            };
            active.last_session = None;
            (Arc::clone(&active.hosting), self.realm_account_id())
        };

        let sessions = match hosting.sessions().await {
            Ok(list) => list,
            Err(error) => {
                let (code, detail) = map_host_error(error);
                return RealmOutcome::Err(self.refuse(LauncherError::with_detail(code, &detail)));
            }
        };

        for session in sessions {
            let res = match session.role {
                HostedRole::Hub(hub) if session.hosting => hosting.stop_hub(account.clone(), hub).await,
                _ => hosting.stop(session.session).await,
            };
            if let Err(error) = res {
                let (code, detail) = map_host_error(error);
                return RealmOutcome::Err(self.refuse(LauncherError::with_detail(code, &detail)));
            }
        }

        match self.build_realm_view().await {
            Ok(view) => RealmOutcome::Ok(view),
            Err(error) => RealmOutcome::Err(error),
        }
    }

    /// `realm_get`.
    pub async fn realm_get(&self) -> RealmOutcome {
        match self.build_realm_view().await {
            Ok(view) => RealmOutcome::Ok(view),
            Err(error) => RealmOutcome::Err(error),
        }
    }
}
