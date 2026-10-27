//! Realm hosting and joining tests (`realms` feature, RF7): drives `Controller::realm_host`,
//! `realm_join`, `realm_stop`, and `realm_get` against a dynamically compiled C ABI transport cdylib,
//! checks RD9 (no injection or loader `.ini` written by realms, and leftover loader files refused by
//! both `realm_host` and `play`), and verifies that the account key in `KeyStore` never reaches
//! `rifts-host`, `p2p-sessions/`, or the dedicated server launch environment.

use std::process::Command;

use rifts_host::{
    ADMISSION_PUBLIC_KEY_FILE, DefaultHostBackend, HostBackend, NativeTransportLoader, NoopServerSpawner,
    SESSION_FILES, ServerSpawner, TRANSPORT_DLL, TransportLoader, sha256_file,
};

use super::*;
use crate::bridge::types::{RealmKind, RealmOutcome, RealmState};
use crate::controller::realms::RealmsSeams;
use crate::platform::loader_session::LOADER_INI;

fn build_fake_transport_dll(out_path: &Path) {
    let dir = out_path.parent().unwrap();
    let src = dir.join("fake_launcher_transport.rs");
    let code = r#"
#![allow(non_snake_case, clippy::missing_safety_doc)]
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static NEXT_HANDLE: AtomicI32 = AtomicI32::new(1);
static IS_JOIN: AtomicBool = AtomicBool::new(false);

fn write_cstr(buf: *mut u8, size: u32, text: &[u8]) {
    if buf.is_null() || size == 0 {
        return;
    }
    let max = (size as usize).saturating_sub(1).min(text.len());
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), buf, max);
        *buf.add(max) = 0;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_AbiVersion() -> i32 { 3 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_Initialize(_err: *mut u8, _size: u32) -> i32 { 1 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_Shutdown() {}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_PeerId() -> u64 { 0xCAFE_1234_5678 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_NetworkingReady() -> i32 { 1 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_StartHost(
    _pk: *const i8, _cfg: *const i8, _cap: *const i8, _ticket: *const i8, err: *mut u8, size: u32
) -> i32 {
    write_cstr(err, size, b"v1 unsupported");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_StartJoin(
    _pk: *const i8, _cfg: *const i8, _ticket: *const i8, err: *mut u8, size: u32
) -> i32 {
    write_cstr(err, size, b"v1 unsupported");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_StartHostV2(
    _pk: *const i8, _cfg: *const i8, _cap: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 { 1 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_StartJoinV2(
    _pk: *const i8, _cfg: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 { 1 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_WaitReady(_timeout: u32, buf: *mut u8, size: u32) -> i32 {
    write_cstr(buf, size, b"HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC");
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_SessionRunning() -> i32 { 1 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_LastSessionResult() -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_StopSession() {}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_ReadDiagnostic(_buf: *mut u8, _size: u32) -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_ReadSignal(
    _to: *mut u8, _to_sz: u32, _kind: *mut u8, _kind_sz: u32, _data: *mut u8, _data_sz: u32
) -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_SubmitSignal(_from: *const i8, _kind: *const i8, _data: *const i8) -> i32 { 1 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_ReadPeerEvent(_buf: *mut u8, _size: u32) -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_HostOpen(
    _pk: *const i8, _cfg: *const i8, _cap: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 {
    IS_JOIN.store(false, Ordering::SeqCst);
    NEXT_HANDLE.fetch_add(1, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_JoinOpen(
    _pk: *const i8, _cfg: *const i8, _ticket: *const i8, _sec: *const i8, _err: *mut u8, _size: u32
) -> i32 {
    IS_JOIN.store(true, Ordering::SeqCst);
    NEXT_HANDLE.fetch_add(1, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_WaitReadyH(handle: i32, _timeout: u32, buf: *mut u8, size: u32) -> i32 {
    if handle <= 0 {
        return -1;
    }
    if IS_JOIN.load(Ordering::SeqCst) {
        write_cstr(
            buf,
            size,
            b"ROUTE_READY localHost=127.0.0.1 localPort=29100 transport=ice routePolicy=AUTOMATIC route=direct",
        );
    } else {
        write_cstr(
            buf,
            size,
            b"HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC",
        );
    }
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_SessionRunningH(handle: i32) -> i32 {
    i32::from(handle > 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_StopSessionH(handle: i32) -> i32 {
    i32::from(handle > 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_LastSessionResultH(handle: i32) -> i32 {
    if handle > 0 { 0 } else { -1 }
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_ReadSignalH(
    _handle: i32, _to: *mut u8, _to_sz: u32, _kind: *mut u8, _kind_sz: u32, _data: *mut u8, _data_sz: u32
) -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_SubmitSignalH(
    handle: i32, _from: *const i8, _kind: *const i8, _data: *const i8
) -> i32 {
    i32::from(handle > 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_ReadPeerEventH(_handle: i32, _buf: *mut u8, _size: u32) -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_ReadDiagnosticH(_handle: i32, _buf: *mut u8, _size: u32) -> i32 { 0 }

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_AddPeerH(
    handle: i32, _account_id: *const i8, _peer_id: u64, _pair_key: *const i8
) -> i32 {
    i32::from(handle > 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn MysticTransport_RemovePeerH(
    handle: i32, _account_id: *const i8, _reason: *const i8
) -> i32 {
    i32::from(handle > 0)
}
"#;
    std::fs::write(&src, code).unwrap();
    let status = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg(&src)
        .arg("-o")
        .arg(out_path)
        .status()
        .unwrap();
    assert!(status.success(), "rustc failed to compile fake transport cdylib");
    let _ = std::fs::remove_file(&src);
}

#[tokio::test]
async fn realm_commands_host_join_stop_keep_rd9_and_never_expose_account_key() {
    let hx = Harness::new();
    let secret_key = "RRK_super_secret_account_key_never_in_realms_998877";
    let mut host = Host::new("RealmsSrv");
    host.registration = "OPEN";
    host.users
        .insert(secret_key.into(), ("Slayer".into(), "user-slayer-1".into()));
    host.runtime = Some(make_runtime(1));
    let (_shared, port) = lan(host).await;

    let inv = invite(ServerMode::Private, "127.0.0.1", port, None, "OPEN", "RealmsSrv");
    assert_eq!(hx.c.submit_invite(&inv, false, false).await, ActionResult::Ok);
    assert_eq!(hx.c.use_existing_key(secret_key).await, ActionResult::Ok);

    let game_root = hx.game();
    let win64 = game_root.join("Archon").join("Binaries").join("Win64");
    let dll_path = win64.join(TRANSPORT_DLL);
    build_fake_transport_dll(&dll_path);
    std::fs::write(win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x11u8; 64]).unwrap();
    let dll_sha = sha256_file(&dll_path).unwrap();

    assert_eq!(hx.c.use_game_path(game_root.to_str().unwrap()).await, ActionResult::Ok);
    assert_eq!(hx.c.verify_game().await, ActionResult::Ok);

    let spawner = Arc::new(NoopServerSpawner::new());
    hx.c.set_realms_seams(RealmsSeams {
        loader: Arc::new(NativeTransportLoader) as Arc<dyn TransportLoader>,
        expected_sha256: Some(dll_sha),
        backend: Arc::new(DefaultHostBackend::new()) as Arc<dyn HostBackend>,
        spawner: spawner.clone() as Arc<dyn ServerSpawner>,
    });

    // Initial realm_get reports available with no sessions.
    let RealmOutcome::Ok(initial) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(initial.available);
    assert!(initial.active.is_none());
    assert!(initial.sessions.is_empty());

    // Host a hub.
    let RealmOutcome::Ok(after_hub) = hx.c.realm_host(RealmKind::Hub).await else {
        panic!("expected RealmOutcome::Ok from realm_host(Hub)");
    };
    assert_eq!(after_hub.sessions.len(), 1);
    assert_eq!(after_hub.active.as_ref().map(|s| s.kind), Some(RealmKind::Hub));
    assert_eq!(after_hub.active.as_ref().map(|s| s.state), Some(RealmState::Running));

    // Host a hunt alongside the hub.
    let RealmOutcome::Ok(after_hunt) = hx.c.realm_host(RealmKind::Hunt).await else {
        panic!("expected RealmOutcome::Ok from realm_host(Hunt)");
    };
    assert_eq!(after_hunt.sessions.len(), 2);
    assert_eq!(after_hunt.active.as_ref().map(|s| s.kind), Some(RealmKind::Hunt));

    // Join a hunt.
    let RealmOutcome::Ok(after_join) = hx.c.realm_join("01ARZ3NDEKTSV4RRFFQ69G5FAV@local").await else {
        panic!("expected RealmOutcome::Ok from realm_join");
    };
    assert_eq!(after_join.sessions.len(), 3);
    let joined = after_join.active.as_ref().unwrap();
    assert!(!joined.hosting);
    assert_eq!(joined.local_port, Some(29_100));

    // RD9: realm hosting and joining never write mystic_loader.ini or any DLL beside the loader.
    assert!(!win64.join(LOADER_INI).exists());
    assert!(hx.c.loader_leftovers(&win64).is_empty());

    // Account key isolation: neither the session files under userData/p2p-sessions/ nor the server
    // launch environment may contain the player's account key.
    let sessions_root = hx.user.path().join("p2p-sessions");
    assert!(sessions_root.is_dir());
    let mut checked_dirs = 0usize;
    for entry in std::fs::read_dir(&sessions_root).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        checked_dirs += 1;
        for file_name in SESSION_FILES {
            let content = std::fs::read_to_string(dir.join(file_name)).unwrap();
            assert!(
                !content.contains(secret_key),
                "session file {file_name} must never contain the account key"
            );
        }
    }
    assert_eq!(checked_dirs, 3);
    let env = spawner.last_environment().unwrap();
    for (k, v) in &env {
        assert!(
            !v.contains(secret_key),
            "launch variable {k} must never contain the account key"
        );
    }

    // Stop all realm sessions and verify session directories are cleaned up.
    let RealmOutcome::Ok(after_stop) = hx.c.realm_stop().await else {
        panic!("expected RealmOutcome::Ok from realm_stop");
    };
    assert!(after_stop.sessions.is_empty());
    assert!(after_stop.active.is_none());
    let remaining_dirs = std::fs::read_dir(&sessions_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .count();
    assert_eq!(
        remaining_dirs, 0,
        "all session directories must be deleted on realm_stop"
    );

    // RD9: if a leftover mystic_loader.ini sits in Win64, both realm_host and play refuse with LoaderLeftovers.
    std::fs::write(win64.join(LOADER_INI), "[loader]\ndll=Dumper-7.dll\n").unwrap();
    assert_eq!(
        hx.c.realm_host(RealmKind::Hub).await,
        RealmOutcome::Err(LauncherError {
            code: ErrorCode::LoaderLeftovers,
            detail: Some(LOADER_INI.to_owned()),
        })
    );
    assert_eq!(
        hx.c.play().await,
        ActionResult::Err(LauncherError {
            code: ErrorCode::LoaderLeftovers,
            detail: Some(LOADER_INI.to_owned()),
        })
    );
    std::fs::remove_file(win64.join(LOADER_INI)).unwrap();

    // SHA-256 mismatch on ParadoxTransport.dll refuses before loading.
    hx.c.realms_shutdown().await;
    hx.c.set_realms_seams(RealmsSeams {
        loader: Arc::new(NativeTransportLoader) as Arc<dyn TransportLoader>,
        expected_sha256: Some([0xAA; 32]),
        backend: Arc::new(DefaultHostBackend::new()) as Arc<dyn HostBackend>,
        spawner: spawner as Arc<dyn ServerSpawner>,
    });
    assert_eq!(
        hx.c.realm_host(RealmKind::Hub).await,
        RealmOutcome::Err(LauncherError {
            code: ErrorCode::RealmUnavailable,
            detail: Some("transport_hash_mismatch".to_owned()),
        })
    );

    hx.c.shutdown().await;
}
