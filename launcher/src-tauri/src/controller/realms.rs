//! Realm hosting and joining (RF7, DD12, DU5): links `rifts-host` behind the `realms` feature and serves
//! `realm_host`, `realm_join`, `realm_stop`, and `realm_get`.
//!
//! RD9 holds throughout: realm hosting and joining never inject into the game process or place a redirect
//! `.ini` or DLL beside the runtime loader, and any leftover loader files in `Win64` refuse realm startup
//! just as `PLAY` refuses them. The player's account key stays inside `KeyStore` and never reaches
//! `rifts-host`, `p2p-sessions/`, or the spawned dedicated server's environment.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rifts_host::{
    ADMISSION_PUBLIC_KEY_FILE, DefaultHostBackend, FileHostState, HostBackend, HostError, HostState as _, HostedRole,
    HubLimits, LocalRealmHosting, LocalSession, LocalState, NativeTransportLoader, NoopServerSpawner, RealmHosting,
    ServerSpawner, SessionResult, TRANSPORT_DLL, TransportAbi, TransportLoader, default_max_servers,
};
use rifts_seam::{AccountId, GlobalId, HubKind, SessionId, Ulid};

use super::{Controller, err, lock, under};
use crate::bridge::types::{
    ErrorCode, LauncherError, RealmKind, RealmOutcome, RealmSessionView, RealmState, RealmView, RuntimeState,
};
use crate::core::pins::WIN64_RELATIVE_DIR;
use crate::platform::log;

const DEFAULT_HUB_MAX_PLAYERS: u8 = 16;
const DEFAULT_HUB_AFK_SECONDS: u32 = 600;
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[cfg_attr(not(test), allow(dead_code, reason = "controller tests install realms seams"))]
pub struct RealmsSeams {
    pub loader: Arc<dyn TransportLoader>,
    pub expected_sha256: Option<[u8; 32]>,
    pub backend: Arc<dyn HostBackend>,
    pub spawner: Arc<dyn ServerSpawner>,
}

struct ActiveRealms {
    win64: PathBuf,
    dll_sha256: [u8; 32],
    transport: Arc<dyn TransportAbi>,
    hosting: Arc<LocalRealmHosting>,
    last_session: Option<SessionId>,
}

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

fn encode_ulid(timestamp_ms: u64, rand_hi: u16, rand_lo: u64) -> Option<Ulid> {
    let ts = u128::from(timestamp_ms & 0xFFFF_FFFF_FFFF);
    let r = (u128::from(rand_hi) << 64) | u128::from(rand_lo);
    let value = (ts << 80) | r;
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

fn mint_local_session_id() -> Result<SessionId, LauncherError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1_700_000_000_000, |d| d.as_millis() as u64);
    let rand_hi: u16 = rand::random();
    let rand_lo: u64 = rand::random();
    let ulid = encode_ulid(now_ms, rand_hi, rand_lo)
        .ok_or_else(|| err(ErrorCode::RealmFailed, Some("session_id".to_owned())))?;
    let gid = format!("{ulid}@local")
        .parse::<GlobalId>()
        .map_err(|_| err(ErrorCode::RealmFailed, Some("session_id".to_owned())))?;
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
    format!("{trimmed}@local").parse::<GlobalId>().ok().map(SessionId)
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
        generation: u32::try_from(local.generation).unwrap_or(u32::MAX),
        state,
        local_port,
        result,
    }
}

impl Controller {
    /// Installs test seams for `rifts-host` (transport loader, pinned SHA-256, backend and server spawner).
    #[cfg(test)]
    pub fn set_realms_seams(&self, seams: RealmsSeams) {
        *lock(&self.realms.seams) = Some(seams);
    }

    /// Stops any active realm sessions and shuts down the loaded transport on launcher exit, logout or server switch.
    pub(super) async fn realms_shutdown(&self) {
        let active = self.realms.inner.lock().await.take();
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
        let s = self.s();
        let raw = s
            .server
            .as_ref()
            .and_then(|sv| s.usernames.get(&sv.id()).cloned())
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

    fn game_win64(&self) -> Option<PathBuf> {
        let root = self.st().game.root.clone()?;
        let win64 = under(&root, WIN64_RELATIVE_DIR);
        win64.is_dir().then_some(win64)
    }

    fn expected_transport_sha256(&self, dll_path: &Path) -> Result<[u8; 32], LauncherError> {
        if let Some(sha) = lock(&self.realms.seams).as_ref().and_then(|s| s.expected_sha256) {
            return Ok(sha);
        }
        let (manifest_sha, local_runtime) = {
            let st = self.st();
            let sha = st
                .runtime_manifest
                .as_ref()
                .and_then(|m| {
                    m.files
                        .iter()
                        .find(|f| f.name.eq_ignore_ascii_case(TRANSPORT_DLL))
                        .map(|f| f.sha256.clone())
                })
                .and_then(|hex| parse_sha256_hex(&hex));
            (sha, st.runtime.state == RuntimeState::Local)
        };
        if let Some(sha) = manifest_sha {
            return Ok(sha);
        }
        if local_runtime {
            return rifts_host::sha256_file(dll_path).map_err(|error| {
                let (code, detail) = map_host_error(error);
                self.fail(code, Some(detail))
            });
        }
        Err(self.fail(ErrorCode::RealmUnavailable, Some("transport_untrusted".to_owned())))
    }

    async fn ensure_realms_ready(&self) -> Result<(), LauncherError> {
        if self.st().shutting_down {
            return Err(err(ErrorCode::Cancelled, None));
        }
        let Some(win64) = self.game_win64() else {
            return Err(self.fail(ErrorCode::GameFolderNotFound, None));
        };
        // RD9: never start or join a realm while a loader redirect ini or Host tools DLL sits in Win64.
        let leftovers = self.loader_leftovers(&win64);
        if !leftovers.is_empty() {
            log::warn(&format!(
                "realm refused: beside the runtime loader: {}",
                leftovers.join(", ")
            ));
            return Err(self.fail(ErrorCode::LoaderLeftovers, Some(leftovers.join(", "))));
        }
        let dll_path = win64.join(TRANSPORT_DLL);
        if !dll_path.is_file() {
            return Err(self.fail(ErrorCode::RealmUnavailable, Some("transport_missing".to_owned())));
        }
        let pubkey_path = win64.join(ADMISSION_PUBLIC_KEY_FILE);
        if !pubkey_path.is_file() {
            return Err(self.fail(ErrorCode::RealmUnavailable, Some("admission_key_missing".to_owned())));
        }
        let expected_sha256 = self.expected_transport_sha256(&dll_path)?;

        let mut guard = self.realms.inner.lock().await;
        if let Some(active) = guard.as_ref()
            && active.win64 == win64
            && active.dll_sha256 == expected_sha256
        {
            return Ok(());
        }

        let (loader, backend, spawner) = {
            let seams = lock(&self.realms.seams);
            match seams.as_ref() {
                Some(s) => (Arc::clone(&s.loader), Arc::clone(&s.backend), Arc::clone(&s.spawner)),
                None => (
                    Arc::new(NativeTransportLoader) as Arc<dyn TransportLoader>,
                    Arc::new(DefaultHostBackend::new()) as Arc<dyn HostBackend>,
                    Arc::new(NoopServerSpawner::new()) as Arc<dyn ServerSpawner>,
                ),
            }
        };

        let boxed_transport = loader.load(&dll_path, expected_sha256).map_err(|error| {
            let (code, detail) = map_host_error(error);
            self.fail(code, Some(detail))
        })?;
        let transport: Arc<dyn TransportAbi> = Arc::from(boxed_transport);
        transport.initialize().map_err(|error| {
            let (code, detail) = map_host_error(error);
            self.fail(code, Some(detail))
        })?;

        let hosting = Arc::new(LocalRealmHosting::with_backend_and_spawner(
            &self.c.user_data,
            &win64,
            Arc::clone(&transport),
            backend,
            spawner,
        ));
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
        let guard = self.realms.inner.lock().await;
        let Some(active) = guard.as_ref() else {
            let available = self.game_win64().is_some_and(|win64| {
                win64.join(TRANSPORT_DLL).is_file()
                    && win64.join(ADMISSION_PUBLIC_KEY_FILE).is_file()
                    && !self.c.user_data.join(rifts_host::DISABLE_COOP_FLAG_FILE).is_file()
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
            self.fail(code, Some(detail))
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

    pub async fn realm_host(&self, kind: RealmKind) -> RealmOutcome {
        if let Err(error) = self.ensure_realms_ready().await {
            return RealmOutcome::Err(error);
        }
        let account = self.realm_account_id();
        let server_id = self
            .s()
            .server
            .as_ref()
            .map_or_else(|| "local".to_owned(), |sv| format!("{}:{}", sv.host, sv.port));
        let state_files = FileHostState::for_server(&self.c.user_data, &server_id);
        let limits = state_files.hub_limits().ok().flatten().unwrap_or(HubLimits {
            max_players: DEFAULT_HUB_MAX_PLAYERS,
            max_servers: default_max_servers(4),
            afk_seconds: DEFAULT_HUB_AFK_SECONDS,
        });

        let hosting = {
            let guard = self.realms.inner.lock().await;
            let Some(active) = guard.as_ref() else {
                return RealmOutcome::Err(self.fail(ErrorCode::RealmUnavailable, None));
            };
            Arc::clone(&active.hosting)
        };

        match hosting.coop_available().await {
            Ok(true) => {}
            Ok(false) => {
                return RealmOutcome::Err(self.fail(ErrorCode::RealmUnavailable, Some("coop_disabled".to_owned())));
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                return RealmOutcome::Err(self.fail(code, Some(detail)));
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
                if let Some(active) = self.realms.inner.lock().await.as_mut() {
                    active.last_session = Some(local.session);
                }
                match self.build_realm_view().await {
                    Ok(view) => RealmOutcome::Ok(view),
                    Err(error) => RealmOutcome::Err(error),
                }
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                RealmOutcome::Err(self.fail(code, Some(detail)))
            }
        }
    }

    pub async fn realm_join(&self, realm: &str) -> RealmOutcome {
        let Some(session_id) = parse_session_id(realm) else {
            return RealmOutcome::Err(self.fail(ErrorCode::RealmNotFound, Some(realm.to_owned())));
        };
        if let Err(error) = self.ensure_realms_ready().await {
            return RealmOutcome::Err(error);
        }
        let account = self.realm_account_id();
        let hosting = {
            let guard = self.realms.inner.lock().await;
            let Some(active) = guard.as_ref() else {
                return RealmOutcome::Err(self.fail(ErrorCode::RealmUnavailable, None));
            };
            Arc::clone(&active.hosting)
        };

        match hosting.coop_available().await {
            Ok(true) => {}
            Ok(false) => {
                return RealmOutcome::Err(self.fail(ErrorCode::RealmUnavailable, Some("coop_disabled".to_owned())));
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                return RealmOutcome::Err(self.fail(code, Some(detail)));
            }
        }

        match hosting.join_hunt(account, session_id.clone()).await {
            Ok(_) => {
                if let Some(active) = self.realms.inner.lock().await.as_mut() {
                    active.last_session = Some(session_id);
                }
                match self.build_realm_view().await {
                    Ok(view) => RealmOutcome::Ok(view),
                    Err(error) => RealmOutcome::Err(error),
                }
            }
            Err(error) => {
                let (code, detail) = map_host_error(error);
                RealmOutcome::Err(self.fail(code, Some(detail)))
            }
        }
    }

    pub async fn realm_stop(&self) -> RealmOutcome {
        let (hosting, account) = {
            let mut guard = self.realms.inner.lock().await;
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
                return RealmOutcome::Err(self.fail(code, Some(detail)));
            }
        };

        for session in sessions {
            let res = match session.role {
                HostedRole::Hub(hub) if session.hosting => hosting.stop_hub(account.clone(), hub).await,
                _ => hosting.stop(session.session).await,
            };
            if let Err(error) = res {
                let (code, detail) = map_host_error(error);
                return RealmOutcome::Err(self.fail(code, Some(detail)));
            }
        }

        match self.build_realm_view().await {
            Ok(view) => RealmOutcome::Ok(view),
            Err(error) => RealmOutcome::Err(error),
        }
    }

    pub async fn realm_get(&self) -> RealmOutcome {
        match self.build_realm_view().await {
            Ok(view) => RealmOutcome::Ok(view),
            Err(error) => RealmOutcome::Err(error),
        }
    }
}
