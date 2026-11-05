//! The client side of realms (RF7), linked by DaintyLass's launcher with its realms feature (DD12): hosting and
//! joining, the session folder, and the transport it loads.
//!
//! Nothing here opens a listener. The transport binds what its `bridge.conf` says (ICE on libjuice), and the hosted
//! game server listens on 127.0.0.1 only (`PrepareListen`).

mod ffi;
pub mod files;
pub mod session;
pub mod transport;

pub use ffi::{NativeTransportLoader, sha256_file};
pub use files::{
    ADMISSION_PUBLIC_KEY_FILE, DISABLE_COOP_FLAG_FILE, FileHostState, HOST_SETTINGS_FILE, HUB_LIMITS_FILE, HostState,
    SESSION_FILES, TRANSPORT_DLL, validate_hub_limits,
};
pub use session::{
    CLEARED_VARIABLES, CapacityDecision, CommonBootstrapConfig, DIRECTOR_SERVER_SWITCHES, DefaultHostBackend, FilesFor,
    GAME_LEFT_IDLE, GameTrafficWatch, HOST_COMMIT_FLOOR_MIB, HOST_MEMORY_FLOOR_MIB, HostBackend, HostBootstrapSpec,
    HostedRole, HubLimits, IceFiles, IcePeerEntry, IceServerEntry, JoinBootstrapSpec, LAUNCH_VARIABLES,
    LaunchEnvironmentSpec, LocalRealmHosting, LocalSession, LocalState, MAX_HUB_PEERS, MAX_HUNT_PEERS,
    NoopServerSpawner, PeerDiff, PeerEvent, PeerListEntry, PeerLive, RealmHosting, RelayGrant, ServerSpawner,
    SessionLive, TransportConfig, WantedPeer, apply_peer_event, apply_peer_line, attempts_in_window,
    build_launch_environment, capacity_decision, cleanup_directory, create_session_directory, default_max_servers,
    host_server_arguments, hunt_join_leave_outcome, ice_files, is_local_admission, next_idle_since, parse_ice_ready,
    parse_peer_event, peer_diff, relay_refresh_due, relay_restart_due, restart_allowed, restart_backoff, safe_account,
    safe_config_value, safe_credential, safe_key, safe_peer_id, safe_url, sanitize_central_log_line,
    scheduled_restart_due, training_should_stop, trials_week, udp_stats_local_rx, wanted_peers, windows_command_line,
    wire_session_id, write_session_file_0600,
};
pub use transport::{
    ABI_VERSION, EXPORTS, Handle, HostOpen, JoinOpen, OPTIONAL_EXPORTS, ReadyState, SessionResult, Signal,
    TransportAbi, TransportLoader,
};

use std::path::Path;

use rifts_seam::{AccountId, BoxFuture, Component, HubKind, PeerId, Secret, SessionId, Unimplemented, stub};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    #[error("{0}")]
    Io(String),
    #[error("transport SHA-256 mismatch")]
    HashMismatch { expected: [u8; 32], actual: [u8; 32] },
    #[error("transport is missing export {0}")]
    MissingExport(&'static str),
    #[error("transport speaks ABI version {0}; expected {ABI_VERSION}")]
    UnsupportedAbi(u32),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Transport(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Busy(String),
}

/// Every operation answers [`Unimplemented`] until RF7 (RD18).
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Host;

fn refuse<T>(operation: &'static str) -> Result<T, HostError> {
    Err(Unimplemented::new(COMPONENT, operation).into())
}

impl TransportLoader for Stub {
    fn load(&self, _: &Path, _: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError> {
        refuse("transport.load")
    }
}

impl TransportAbi for Stub {
    fn abi_version(&self) -> Result<u32, HostError> {
        refuse("transport.abi_version")
    }
    fn initialize(&self) -> Result<(), HostError> {
        refuse("transport.initialize")
    }
    fn shutdown(&self) -> Result<(), HostError> {
        refuse("transport.shutdown")
    }
    fn peer_id(&self) -> Result<PeerId, HostError> {
        refuse("transport.peer_id")
    }
    fn networking_ready(&self) -> Result<bool, HostError> {
        refuse("transport.networking_ready")
    }
    fn host_open(&self, _: &HostOpen) -> Result<Handle, HostError> {
        refuse("transport.host_open")
    }
    fn join_open(&self, _: &JoinOpen) -> Result<Handle, HostError> {
        refuse("transport.join_open")
    }
    fn wait_ready(&self, _: Handle, _: u32) -> Result<ReadyState, HostError> {
        refuse("transport.wait_ready")
    }
    fn session_running(&self, _: Handle) -> Result<bool, HostError> {
        refuse("transport.session_running")
    }
    fn stop_session(&self, _: Handle) -> Result<(), HostError> {
        refuse("transport.stop_session")
    }
    fn last_session_result(&self, _: Handle) -> Result<SessionResult, HostError> {
        refuse("transport.last_session_result")
    }
    fn read_signal(&self, _: Handle) -> Result<Option<Signal>, HostError> {
        refuse("transport.read_signal")
    }
    fn submit_signal(&self, _: Handle, _: &AccountId, _: &str, _: &str) -> Result<(), HostError> {
        refuse("transport.submit_signal")
    }
    fn read_peer_event(&self, _: Handle) -> Result<Option<String>, HostError> {
        refuse("transport.read_peer_event")
    }
    fn read_diagnostic(&self, _: Handle) -> Result<Option<String>, HostError> {
        refuse("transport.read_diagnostic")
    }
    fn add_peer(&self, _: Handle, _: &AccountId, _: PeerId, _: &Secret) -> Result<(), HostError> {
        refuse("transport.add_peer")
    }
    fn remove_peer(&self, _: Handle, _: &AccountId, _: &str) -> Result<(), HostError> {
        refuse("transport.remove_peer")
    }
    fn update_relay_credential(&self, _: Handle, _: &str, _: &Secret) -> Result<(), HostError> {
        refuse("transport.update_relay_credential")
    }
}

impl RealmHosting for Stub {
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>> {
        stub(COMPONENT, "coop_available")
    }
    fn host_hunt(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        stub(COMPONENT, "host_hunt")
    }
    fn join_hunt(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        stub(COMPONENT, "join_hunt")
    }
    fn start_hub(&self, _: AccountId, _: HubKind, _: HubLimits) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        stub(COMPONENT, "start_hub")
    }
    fn stop_hub(&self, _: AccountId, _: HubKind) -> BoxFuture<'_, Result<(), HostError>> {
        stub(COMPONENT, "stop_hub")
    }
    fn stop(&self, _: SessionId) -> BoxFuture<'_, Result<(), HostError>> {
        stub(COMPONENT, "stop")
    }
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>> {
        stub(COMPONENT, "sessions")
    }
}

impl HostState for Stub {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError> {
        refuse("state.hub_limits")
    }
    fn save_hub_limits(&self, _: &HubLimits) -> Result<(), HostError> {
        refuse("state.save_hub_limits")
    }
    fn control_cursor(&self, _: &AccountId) -> Result<Option<u64>, HostError> {
        refuse("state.control_cursor")
    }
    fn set_control_cursor(&self, _: &AccountId, _: u64) -> Result<(), HostError> {
        refuse("state.set_control_cursor")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::poll_once;
    use std::collections::{BTreeSet, HashSet};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn exports_are_the_headers() {
        let header = include_str!("../../../src/paradox_transport_api.hpp");
        let declared: BTreeSet<&str> = header
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|word| word.starts_with("MysticTransport_"))
            .collect();
        assert_eq!(declared, EXPORTS.into_iter().collect());
        assert!(OPTIONAL_EXPORTS.iter().all(|name| !declared.contains(name)));
    }

    #[test]
    fn session_results_keep_their_codes() {
        assert_eq!(SessionResult::from_code(0), SessionResult::Clean);
        assert_eq!(SessionResult::from_code(2), SessionResult::BadInput);
        assert_eq!(SessionResult::from_code(3), SessionResult::Setup);
        assert_eq!(SessionResult::from_code(4), SessionResult::RefusedAdmission);
        assert_eq!(SessionResult::from_code(5), SessionResult::LeaseFailed);
        assert_eq!(SessionResult::from_code(-1), SessionResult::UnknownHandle);
        assert_eq!(SessionResult::from_code(9), SessionResult::Other(9));
    }

    #[test]
    fn stub_refuses_everything() {
        assert_eq!(
            Stub.load(Path::new(TRANSPORT_DLL), [0; 32]).err(),
            refuse::<()>("transport.load").err()
        );
        assert_eq!(Stub.abi_version(), refuse("transport.abi_version"));
        assert_eq!(
            Stub.update_relay_credential(Handle(1), "u", &Secret::new("c".to_owned())),
            refuse("transport.update_relay_credential")
        );
        assert_eq!(poll_once(Stub.coop_available()), Some(refuse("coop_available")));
        assert_eq!(Stub.hub_limits(), refuse("state.hub_limits"));
    }

    fn build_fake_transport_cdylib(
        dir: &Path,
        name: &str,
        abi_version: i32,
        include_remove_peer: bool,
        include_optional_update_relay: bool,
    ) -> PathBuf {
        let src_path = dir.join(format!("{name}.rs"));
        let dll_ext = if cfg!(windows) {
            "dll"
        } else if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };
        let out_path = dir.join(format!("{name}.{dll_ext}"));

        let remove_peer_fn = if include_remove_peer {
            r#"
#[no_mangle]
pub extern "C" fn MysticTransport_RemovePeerH(handle: i32, _account_id: *const i8, _reason: *const i8) -> i32 {
    if handle > 0 { 1 } else { 0 }
}
"#
        } else {
            ""
        };

        let optional_fn = if include_optional_update_relay {
            r#"
#[no_mangle]
pub extern "C" fn MysticTransport_UpdateRelayCredentialH(handle: i32, _user: *const i8, _cred: *const i8) -> i32 {
    if handle == 99 { 0 } else if handle > 0 { 1 } else { 0 }
}
"#
        } else {
            ""
        };

        let code = format!(
            r#"
use std::sync::atomic::{{AtomicBool, AtomicI32, Ordering}};

static NEXT_HANDLE: AtomicI32 = AtomicI32::new(1);
static RUNNING_1: AtomicBool = AtomicBool::new(false);
static RUNNING_2: AtomicBool = AtomicBool::new(false);
static SIGNAL_PENDING: AtomicBool = AtomicBool::new(false);
static EVENT_PENDING: AtomicBool = AtomicBool::new(false);
static DIAG_PENDING: AtomicBool = AtomicBool::new(false);
static IS_JOIN_2: AtomicBool = AtomicBool::new(false);

fn write_cstr(buf: *mut u8, size: u32, text: &[u8]) {{
    if buf.is_null() || size == 0 {{
        return;
    }}
    let max = (size as usize).saturating_sub(1).min(text.len());
    unsafe {{
        std::ptr::copy_nonoverlapping(text.as_ptr(), buf, max);
        *buf.add(max) = 0;
    }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_AbiVersion() -> i32 {{
    {abi_version}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_Initialize(_err: *mut u8, _size: u32) -> i32 {{
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_Shutdown() {{
    RUNNING_1.store(false, Ordering::SeqCst);
    RUNNING_2.store(false, Ordering::SeqCst);
}}

#[no_mangle]
pub extern "C" fn MysticTransport_PeerId() -> u64 {{
    0xDEAD_BEEF_CAFE_1234
}}

#[no_mangle]
pub extern "C" fn MysticTransport_NetworkingReady() -> i32 {{
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_StartHost(
    _pk: *const i8, _cfg: *const i8, _cap: *const i8, _ticket: *const i8, err: *mut u8, size: u32
) -> i32 {{
    write_cstr(err, size, b"v1 unsupported");
    0
}}

#[no_mangle]
pub extern "C" fn MysticTransport_StartJoin(
    _pk: *const i8, _cfg: *const i8, _ticket: *const i8, err: *mut u8, size: u32
) -> i32 {{
    write_cstr(err, size, b"v1 unsupported");
    0
}}

#[no_mangle]
pub extern "C" fn MysticTransport_StartHostV2(
    _pk: *const i8, _cfg: *const i8, _cap: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 {{
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_StartJoinV2(
    _pk: *const i8, _cfg: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 {{
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_WaitReady(_timeout: u32, buf: *mut u8, size: u32) -> i32 {{
    write_cstr(buf, size, b"HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC");
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_SessionRunning() -> i32 {{
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_LastSessionResult() -> i32 {{
    0
}}

#[no_mangle]
pub extern "C" fn MysticTransport_StopSession() {{}}

#[no_mangle]
pub extern "C" fn MysticTransport_ReadDiagnostic(_buf: *mut u8, _size: u32) -> i32 {{
    0
}}

#[no_mangle]
pub extern "C" fn MysticTransport_ReadSignal(
    _to: *mut u8, _to_sz: u32, _kind: *mut u8, _kind_sz: u32, _data: *mut u8, _data_sz: u32
) -> i32 {{
    0
}}

#[no_mangle]
pub extern "C" fn MysticTransport_SubmitSignal(_from: *const i8, _kind: *const i8, _data: *const i8) -> i32 {{
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_ReadPeerEvent(_buf: *mut u8, _size: u32) -> i32 {{
    0
}}

#[no_mangle]
pub extern "C" fn MysticTransport_HostOpen(
    _pk: *const i8, _cfg: *const i8, _cap: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 {{
    let h = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    RUNNING_1.store(true, Ordering::SeqCst);
    SIGNAL_PENDING.store(true, Ordering::SeqCst);
    EVENT_PENDING.store(true, Ordering::SeqCst);
    DIAG_PENDING.store(true, Ordering::SeqCst);
    IS_JOIN_2.store(false, Ordering::SeqCst);
    h
}}

#[no_mangle]
pub extern "C" fn MysticTransport_JoinOpen(
    _pk: *const i8, _cfg: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 {{
    let h = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    RUNNING_2.store(true, Ordering::SeqCst);
    IS_JOIN_2.store(true, Ordering::SeqCst);
    h
}}

#[no_mangle]
pub extern "C" fn MysticTransport_WaitReadyH(handle: i32, _timeout: u32, buf: *mut u8, size: u32) -> i32 {{
    if handle <= 0 {{
        return -1;
    }}
    if IS_JOIN_2.load(Ordering::SeqCst) {{
        write_cstr(buf, size, b"ROUTE_READY localHost=127.0.0.1 localPort=29100 transport=ice routePolicy=AUTOMATIC route=direct");
    }} else {{
        write_cstr(buf, size, b"HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC");
    }}
    1
}}

#[no_mangle]
pub extern "C" fn MysticTransport_SessionRunningH(handle: i32) -> i32 {{
    if handle > 0 && (RUNNING_1.load(Ordering::SeqCst) || RUNNING_2.load(Ordering::SeqCst)) {{ 1 }} else {{ 0 }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_StopSessionH(handle: i32) -> i32 {{
    if handle > 0 {{
        RUNNING_1.store(false, Ordering::SeqCst);
        RUNNING_2.store(false, Ordering::SeqCst);
        1
    }} else {{
        0
    }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_LastSessionResultH(handle: i32) -> i32 {{
    if handle > 0 {{ 0 }} else {{ -1 }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_ReadSignalH(
    handle: i32, to: *mut u8, to_sz: u32, kind: *mut u8, kind_sz: u32, data: *mut u8, data_sz: u32
) -> i32 {{
    if handle > 0 && SIGNAL_PENDING.swap(false, Ordering::SeqCst) {{
        write_cstr(to, to_sz, b"peer-acct-1");
        write_cstr(kind, kind_sz, b"DESCRIPTION");
        write_cstr(data, data_sz, b"v=0\r\na=ice-ufrag:test\r\n");
        1
    }} else {{
        0
    }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_SubmitSignalH(handle: i32, _from: *const i8, _kind: *const i8, _data: *const i8) -> i32 {{
    if handle > 0 {{ 1 }} else {{ 0 }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_ReadPeerEventH(handle: i32, buf: *mut u8, size: u32) -> i32 {{
    if handle > 0 && EVENT_PENDING.swap(false, Ordering::SeqCst) {{
        write_cstr(buf, size, b"PEER_ADMITTED account=peer-acct-1 route=direct");
        1
    }} else {{
        0
    }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_ReadDiagnosticH(handle: i32, buf: *mut u8, size: u32) -> i32 {{
    if handle > 0 && DIAG_PENDING.swap(false, Ordering::SeqCst) {{
        write_cstr(buf, size, b"ADMISSION_ACCEPTED role=host peer=100001 source=local");
        1
    }} else {{
        0
    }}
}}

#[no_mangle]
pub extern "C" fn MysticTransport_AddPeerH(handle: i32, _account: *const i8, peer_id: u64, _key: *const i8) -> i32 {{
    if handle > 0 && peer_id > 0 {{ 1 }} else {{ 0 }}
}}

{remove_peer_fn}
{optional_fn}
"#
        );

        assert!(fs::write(&src_path, code).is_ok());
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let status = Command::new(rustc)
            .arg("--crate-type=cdylib")
            .arg("--edition=2021")
            .arg("-O")
            .arg(&src_path)
            .arg("-o")
            .arg(&out_path)
            .status();
        assert!(
            status.is_ok_and(|s| s.success()),
            "failed to compile fake transport cdylib"
        );
        out_path
    }

    #[test]
    fn transport_loader_verifies_sha256_exports_abi_and_calls_cdylib() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir");
        };

        // 1. Missing export is refused with MissingExport.
        let missing_dll = build_fake_transport_cdylib(tmp.path(), "fake_missing", 3, false, false);
        let Ok(missing_sha) = sha256_file(&missing_dll) else {
            panic!("sha256");
        };
        assert_eq!(
            NativeTransportLoader.load(&missing_dll, missing_sha).err(),
            Some(HostError::MissingExport("MysticTransport_RemovePeerH"))
        );

        // 2. Wrong ABI version is refused with UnsupportedAbi.
        let bad_abi_dll = build_fake_transport_cdylib(tmp.path(), "fake_abi2", 2, true, false);
        let Ok(bad_abi_sha) = sha256_file(&bad_abi_dll) else {
            panic!("sha256");
        };
        assert_eq!(
            NativeTransportLoader.load(&bad_abi_dll, bad_abi_sha).err(),
            Some(HostError::UnsupportedAbi(2))
        );

        // 3. Full ABI 3 cdylib with optional export present: hash mismatch refused before load.
        let valid_dll = build_fake_transport_cdylib(tmp.path(), "fake_valid", 3, true, true);
        let Ok(valid_sha) = sha256_file(&valid_dll) else {
            panic!("sha256");
        };
        let wrong_sha = [0xAA; 32];
        assert_eq!(
            NativeTransportLoader.load(&valid_dll, wrong_sha).err(),
            Some(HostError::HashMismatch {
                expected: wrong_sha,
                actual: valid_sha,
            })
        );

        // 4. Correct hash loads and exercises all TransportAbi methods.
        let Ok(transport) = NativeTransportLoader.load(&valid_dll, valid_sha) else {
            panic!("load valid transport");
        };
        assert_eq!(transport.abi_version(), Ok(3));
        assert_eq!(transport.initialize(), Ok(()));
        assert_eq!(transport.peer_id(), Ok(PeerId(0xDEAD_BEEF_CAFE_1234)));
        assert_eq!(transport.networking_ready(), Ok(true));

        let open = HostOpen {
            public_key: tmp.path().join("pub.xy"),
            config: tmp.path().join("bridge.conf"),
            capability: tmp.path().join("capability.secret"),
            host_ticket: Some(tmp.path().join("ticket.secret")),
            secrets: tmp.path().join("transport.secret"),
        };
        let Ok(handle) = transport.host_open(&open) else {
            panic!("host_open");
        };
        assert!(handle.0 > 0);
        assert_eq!(
            transport.wait_ready(handle, 100),
            Ok(ReadyState::Ready(
                "HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC".to_owned()
            ))
        );
        assert_eq!(transport.session_running(handle), Ok(true));
        assert_eq!(
            transport.read_signal(handle),
            Ok(Some(Signal {
                to_account: AccountId("peer-acct-1".to_owned()),
                kind: "DESCRIPTION".to_owned(),
                data: "v=0\r\na=ice-ufrag:test\r\n".to_owned(),
            }))
        );
        assert_eq!(transport.read_signal(handle), Ok(None));
        assert_eq!(
            transport.submit_signal(
                handle,
                &AccountId("peer-acct-1".to_owned()),
                "CANDIDATE",
                "a=candidate:1"
            ),
            Ok(())
        );
        assert_eq!(
            transport.read_peer_event(handle),
            Ok(Some("PEER_ADMITTED account=peer-acct-1 route=direct".to_owned()))
        );
        assert_eq!(
            transport.read_diagnostic(handle),
            Ok(Some("ADMISSION_ACCEPTED role=host peer=100001 source=local".to_owned()))
        );
        assert_eq!(
            transport.add_peer(
                handle,
                &AccountId("peer-acct-1".to_owned()),
                PeerId(200002),
                &Secret::new("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned())
            ),
            Ok(())
        );
        assert_eq!(
            transport.remove_peer(handle, &AccountId("peer-acct-1".to_owned()), "not_expected"),
            Ok(())
        );
        assert_eq!(
            transport.update_relay_credential(handle, "turn-user", &Secret::new("turn-pass".to_owned())),
            Ok(())
        );
        assert!(
            transport
                .update_relay_credential(Handle(99), "turn-user", &Secret::new("turn-pass".to_owned()))
                .is_err()
        );
        assert_eq!(transport.last_session_result(handle), Ok(SessionResult::Clean));
        assert_eq!(transport.stop_session(handle), Ok(()));
        assert_eq!(transport.session_running(handle), Ok(false));
        assert_eq!(transport.shutdown(), Ok(()));
    }

    #[test]
    fn real_paradox_transport_dll_loads_when_built() {
        let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/Release/ParadoxTransport.dll");
        if !dll.is_file() {
            return;
        }
        let Ok(expected) = sha256_file(&dll) else {
            panic!("sha256 of ParadoxTransport.dll");
        };
        let Ok(transport) = NativeTransportLoader.load(&dll, expected) else {
            panic!("load ParadoxTransport.dll");
        };
        assert_eq!(transport.abi_version(), Ok(ABI_VERSION));
        assert_eq!(transport.initialize(), Ok(()));
        let Ok(peer) = transport.peer_id() else {
            panic!("peer_id");
        };
        assert_ne!(peer.0, 0);
        assert_eq!(transport.networking_ready(), Ok(true));
        // Optional export is absent on ParadoxTransport.dll, so update_relay_credential is a safe no-op.
        assert_eq!(
            transport.update_relay_credential(Handle(1), "user", &Secret::new("secret".to_owned())),
            Ok(())
        );
        assert_eq!(transport.shutdown(), Ok(()));
    }

    #[test]
    fn host_state_persists_limits_and_cursors_and_preserves_corrupt_files_as_bad() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir");
        };
        let state = FileHostState::for_server(tmp.path(), "srv-1");
        assert_eq!(state.hub_limits(), Ok(None));

        let limits = HubLimits {
            max_players: 24,
            max_servers: 4,
            afk_seconds: 600,
        };
        assert_eq!(state.save_hub_limits(&limits), Ok(()));
        assert_eq!(state.hub_limits(), Ok(Some(limits)));

        // Out-of-range limits are refused.
        assert!(
            state
                .save_hub_limits(&HubLimits {
                    max_players: 0,
                    max_servers: 4,
                    afk_seconds: 600,
                })
                .is_err()
        );

        // Corrupt realm-host-limits.json is set aside as realm-host-limits.json.bad on next save.
        let limits_file = tmp.path().join(HUB_LIMITS_FILE);
        assert!(fs::write(&limits_file, "{not-json").is_ok());
        assert_eq!(state.hub_limits(), Ok(None));
        assert_eq!(state.save_hub_limits(&limits), Ok(()));
        assert!(tmp.path().join("realm-host-limits.json.bad").is_file());
        assert_eq!(state.hub_limits(), Ok(Some(limits)));

        // Control cursors are per account and advance monotonically.
        let acct_a = AccountId("acct-a".to_owned());
        let acct_b = AccountId("acct-b".to_owned());
        assert_eq!(state.control_cursor(&acct_a), Ok(None));
        assert_eq!(state.set_control_cursor(&acct_a, 10), Ok(()));
        assert_eq!(state.control_cursor(&acct_a), Ok(Some(10)));
        assert_eq!(state.set_control_cursor(&acct_a, 5), Ok(()));
        assert_eq!(state.control_cursor(&acct_a), Ok(Some(10)));
        assert_eq!(state.set_control_cursor(&acct_b, 3), Ok(()));
        assert_eq!(state.control_cursor(&acct_b), Ok(Some(3)));
        assert_eq!(state.control_cursor(&acct_a), Ok(Some(10)));
    }

    #[test]
    fn realm_hosting_creates_0600_session_files_builds_launch_variables_and_cleans_up_on_stop() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir");
        };
        let dll = build_fake_transport_cdylib(tmp.path(), "fake_hosting", 3, true, false);
        let Ok(sha) = sha256_file(&dll) else {
            panic!("sha256");
        };
        let Ok(transport) = NativeTransportLoader.load(&dll, sha) else {
            panic!("load");
        };
        let transport: Arc<dyn TransportAbi> = Arc::from(transport);
        let app_local = tmp.path().join("local");
        let game_dir = tmp.path().join("game");
        assert!(fs::create_dir_all(&app_local).is_ok());
        assert!(fs::create_dir_all(&game_dir).is_ok());
        assert!(fs::write(game_dir.join(ADMISSION_PUBLIC_KEY_FILE), [0u8; 64]).is_ok());

        let spawner = Arc::new(NoopServerSpawner::new());
        let backend: Arc<dyn HostBackend> = Arc::new(DefaultHostBackend::new());
        let hosting = LocalRealmHosting::with_backend_and_spawner(
            &app_local,
            &game_dir,
            Arc::clone(&transport),
            Arc::clone(&backend),
            spawner.clone(),
        );

        assert_eq!(poll_once(hosting.coop_available()), Some(Ok(true)));

        // Flag file turns co-op off.
        let flag = app_local.join(DISABLE_COOP_FLAG_FILE);
        assert!(fs::write(&flag, b"1").is_ok());
        assert_eq!(poll_once(hosting.coop_available()), Some(Ok(false)));
        let _ = fs::remove_file(&flag);
        assert_eq!(poll_once(hosting.coop_available()), Some(Ok(true)));

        // Start a hub session.
        let account = AccountId("host-acct-1".to_owned());
        let limits = HubLimits {
            max_players: 16,
            max_servers: 2,
            afk_seconds: 300,
        };
        let Some(Ok(hub_session)) = poll_once(hosting.start_hub(account.clone(), HubKind::Ramsgate, limits)) else {
            panic!("start_hub");
        };
        assert_eq!(hub_session.role, HostedRole::Hub(HubKind::Ramsgate));
        assert!(hub_session.hosting);
        assert_eq!(hub_session.state, LocalState::Running);

        // Session directory exists under p2p-sessions/ with all 4 SESSION_FILES.
        let Some(hub_dir) = hosting.session_directory(&hub_session.session) else {
            panic!("session_directory");
        };
        assert!(hub_dir.starts_with(app_local.join("p2p-sessions")));
        for file_name in SESSION_FILES {
            let file_path = hub_dir.join(file_name);
            assert!(file_path.is_file(), "missing session file {file_name}");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = fs::metadata(&file_path)
                    .map(|m| m.permissions().mode() & 0o777)
                    .unwrap_or(0);
                assert_eq!(mode, 0o600, "{file_name} must be 0600");
            }
        }

        // Launch environment has LAUNCH_VARIABLES and strips CLEARED_VARIABLES.
        let Some(env) = spawner.last_environment() else {
            panic!("last_environment");
        };
        assert_eq!(
            env.get("MYSTICPARADOX_AUTH_MODE").map(String::as_str),
            Some("HUNT_CAPABILITY")
        );
        assert_eq!(
            env.get("MYSTICPARADOX_BIND_ADDRESS").map(String::as_str),
            Some("127.0.0.1")
        );
        assert_eq!(env.get("MYSTICPARADOX_HOSTED_ROLE").map(String::as_str), Some("HUB"));
        assert_eq!(env.get("MYSTICPARADOX_MAX_PLAYERS").map(String::as_str), Some("16"));
        assert_eq!(
            env.get("MYSTICPARADOX_HUB_AFK_SECONDS").map(String::as_str),
            Some("300")
        );
        for cleared in CLEARED_VARIABLES {
            assert!(!env.contains_key(cleared));
        }

        // Host a hunt and join a hunt alongside the hub.
        let Ok(hunt_gid) = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse() else {
            panic!("parse hunt gid");
        };
        let hunt_id = SessionId(hunt_gid);
        let Some(Ok(hosted_hunt)) = poll_once(hosting.host_hunt(account.clone(), hunt_id.clone())) else {
            panic!("host_hunt");
        };
        assert_eq!(hosted_hunt.role, HostedRole::Hunt);
        assert!(hosted_hunt.hosting);
        let Some(hunt_dir) = hosting.session_directory(&hunt_id) else {
            panic!("hunt_dir");
        };
        assert!(hunt_dir.is_dir());

        // Trials hunt sets MYSTICPARADOX_TRIALS_WEEK.
        let Some(hunt_env) = hosting.session_launch_environment(&hunt_id) else {
            panic!("hunt_env");
        };
        assert_eq!(
            hunt_env.get("MYSTICPARADOX_TRIALS_WEEK").map(String::as_str),
            Some("001")
        );
        for var in LAUNCH_VARIABLES {
            if !matches!(var, "MYSTICPARADOX_MAX_PLAYERS" | "MYSTICPARADOX_HUB_AFK_SECONDS") {
                assert!(hunt_env.contains_key(var), "missing launch variable {var}");
            }
        }

        let Ok(join_gid) = "01ARZ3NDEKTSV4RRFFQ69G5FAW@local".parse() else {
            panic!("parse join gid");
        };
        let join_id = SessionId(join_gid);
        let Some(Ok(joined_hunt)) = poll_once(hosting.join_hunt(AccountId("joiner-1".to_owned()), join_id.clone()))
        else {
            panic!("join_hunt");
        };
        assert!(!joined_hunt.hosting);
        assert_eq!(hosting.session_local_port(&join_id), Some(29_100));
        let Some(join_dir) = hosting.session_directory(&join_id) else {
            panic!("join_dir");
        };
        assert!(join_dir.is_dir());

        let Some(Ok(active)) = poll_once(hosting.sessions()) else {
            panic!("sessions");
        };
        assert_eq!(active.len(), 3);

        // Stop the hub, the hosted hunt, and the joined hunt; verify every session directory is deleted.
        assert_eq!(poll_once(hosting.stop_hub(account, HubKind::Ramsgate)), Some(Ok(())));
        assert!(!hub_dir.exists(), "hub session folder must be deleted on stop_hub");

        assert_eq!(poll_once(hosting.stop(hunt_id)), Some(Ok(())));
        assert!(!hunt_dir.exists(), "hunt session folder must be deleted on stop");

        assert_eq!(poll_once(hosting.stop(join_id)), Some(Ok(())));
        assert!(!join_dir.exists(), "join session folder must be deleted on stop");

        assert_eq!(poll_once(hosting.sessions()), Some(Ok(Vec::new())));
    }

    #[test]
    fn pure_rules_for_peers_signals_traffic_and_capacity() {
        let mut live = SessionLive::default();
        apply_peer_line(
            &mut live,
            "ADMISSION_ACCEPTED role=host peer=100001 source=local",
            "2026-10-27T00:00:00Z",
        );
        assert_eq!(live.local_since.as_deref(), Some("2026-10-27T00:00:00Z"));
        apply_peer_line(
            &mut live,
            "PEER_ADMITTED account=p1 route=relay",
            "2026-10-27T00:00:01Z",
        );
        assert_eq!(live.peers.get("p1").and_then(|p| p.route.as_deref()), Some("relay"));
        apply_peer_line(&mut live, "PEER_LOST account=p1 reason=timeout", "2026-10-27T00:00:02Z");
        assert!(!live.peers.contains_key("p1"));

        let entries = vec![
            PeerListEntry {
                account_id: "p1".to_owned(),
                peer_id: "200001".to_owned(),
                transport_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(),
            },
            PeerListEntry {
                account_id: "host".to_owned(),
                peer_id: "100001".to_owned(),
                transport_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(),
            },
            PeerListEntry {
                account_id: "bad!".to_owned(),
                peer_id: "0".to_owned(),
                transport_key: "short".to_owned(),
            },
        ];
        let (wanted, refused) = wanted_peers(&entries, Some("host"));
        assert_eq!(wanted.len(), 1);
        assert_eq!(refused, 2);
        let mut linked = HashSet::new();
        linked.insert("stale".to_owned());
        let diff = peer_diff(&linked, &wanted);
        assert_eq!(diff.add.len(), 1);
        assert_eq!(diff.remove, vec!["stale".to_owned()]);

        let start = Instant::now();
        let mut watch = GameTrafficWatch::new(start);
        watch.observe(
            "UDP_STATS role=join path=remote localRx=10",
            start + Duration::from_secs(1),
        );
        assert_eq!(
            hunt_join_leave_outcome(start, &watch, start + Duration::from_secs(3)),
            None
        );
        assert_eq!(
            hunt_join_leave_outcome(start, &watch, start + Duration::from_secs(8)),
            Some("game_left")
        );

        assert_eq!(default_max_servers(16), 6);
        assert_eq!(default_max_servers(2), 1);
        assert_eq!(capacity_decision(4096, 1, 2), CapacityDecision::Accept);
        assert_eq!(
            capacity_decision(2048, 1, 2),
            CapacityDecision::Refuse { reason: "HOST_BUSY" }
        );
        assert_eq!(
            sanitize_central_log_line("p2p ticket=secret123 capability=cap456 ok=1"),
            "p2p ticket=[redacted] capability=[redacted] ok=1"
        );
    }
}
