// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use http_body_util::Full;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use rcgen::generate_simple_self_signed;
use rustls::ServerConfig;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::*;
use crate::core::pins::sha256_hex;

struct MockRoute {
    status: u16,
    body: Vec<u8>,
    require_auth: Option<String>,
    hits: Arc<AtomicUsize>,
}

async fn start_tls_server(routes: HashMap<String, MockRoute>) -> (Endpoint, tokio::task::JoinHandle<()>) {
    let certified = generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_der = certified.cert.der().clone();
    let pin = sha256_hex(cert_der.as_ref());

    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let server_cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let routes = Arc::new(routes);

    let handle = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let routes = Arc::clone(&routes);
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(tcp).await {
                    let svc = service_fn(move |req| {
                        let routes = Arc::clone(&routes);
                        async move {
                            let path = req.uri().path().to_owned();
                            if let Some(route) = routes.get(&path) {
                                route.hits.fetch_add(1, Ordering::SeqCst);
                                if let Some(expected_auth) = &route.require_auth {
                                    let actual = req
                                        .headers()
                                        .get("authorization")
                                        .and_then(|v| v.to_str().ok())
                                        .unwrap_or("");
                                    if actual != expected_auth {
                                        return Ok::<_, hyper::Error>(
                                            Response::builder()
                                                .status(StatusCode::UNAUTHORIZED)
                                                .body(Full::new(Bytes::new()))
                                                .unwrap(),
                                        );
                                    }
                                }
                                Ok::<_, hyper::Error>(
                                    Response::builder()
                                        .status(StatusCode::from_u16(route.status).unwrap())
                                        .header("content-length", route.body.len().to_string())
                                        .body(Full::new(Bytes::from(route.body.clone())))
                                        .unwrap(),
                                )
                            } else {
                                Ok::<_, hyper::Error>(
                                    Response::builder()
                                        .status(StatusCode::NOT_FOUND)
                                        .body(Full::new(Bytes::new()))
                                        .unwrap(),
                                )
                            }
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), svc)
                        .await;
                }
            });
        }
    });

    (
        Endpoint {
            host: "127.0.0.1".to_owned(),
            port,
            pin: Some(pin),
        },
        handle,
    )
}

#[test]
fn set_id_and_trust_follow_the_electron_launcher() {
    let f1 = RuntimeFile {
        name: "winmm.dll".to_owned(),
        size: 1024,
        sha256: "b".repeat(64),
    };
    let f2 = RuntimeFile {
        name: "Rustic-Isles.dll".to_owned(),
        size: 2048,
        sha256: "a".repeat(64),
    };

    let id_forward = runtime_set_id(&[f1.clone(), f2.clone()]);
    let id_reverse = runtime_set_id(&[f2.clone(), f1.clone()]);
    assert_eq!(id_forward, id_reverse);

    let expected_lines = format!(
        "Rustic-Isles.dll:2048:{}\nwinmm.dll:1024:{}",
        "a".repeat(64),
        "b".repeat(64)
    );
    assert_eq!(id_forward, sha256_hex(expected_lines.as_bytes()));

    let mut trusted = Vec::new();
    assert!(!is_trusted(&trusted, &[f1.clone(), f2.clone()]));
    assert!(!is_trusted(&trusted, &[]));

    let accepted_id = accept(&mut trusted, &[f1.clone(), f2.clone()]);
    assert_eq!(accepted_id, id_forward);
    assert!(is_trusted(&trusted, &[f1, f2]));

    for i in 0..10u64 {
        let dummy = [RuntimeFile {
            name: "Rustic-Isles.dll".to_owned(),
            size: 100 + i,
            sha256: format!("{i:064x}"),
        }];
        accept(&mut trusted, &dummy);
    }
    assert_eq!(trusted.len(), 8);
}

#[test]
fn manifest_parses_the_runtime_and_an_optional_loader() {
    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);

    let valid = json!({
        "version": " 1.2.3 ",
        "files": [
            { "name": "winmm.dll", "size": 512, "sha256": sha_b },
            { "name": "Rustic-Isles.dll", "size": 1024, "sha256": sha_a }
        ]
    });
    let parsed = parse_runtime_manifest(&valid).expect("valid manifest");
    assert_eq!(parsed.version, "1.2.3");
    assert_eq!(parsed.build.changelist, DEFAULT_CHANGELIST);
    assert_eq!(parsed.files[0].name, "Rustic-Isles.dll");
    assert_eq!(parsed.files[1].name, "winmm.dll");

    // The loader is optional since Windows launchers inject the runtime (RD9, 2026-11-12); the runtime is not
    let runtime_only = json!({
        "version": "1.0.0",
        "files": [
            { "name": "Rustic-Isles.dll", "size": 1024, "sha256": sha_a }
        ]
    });
    assert_eq!(
        parse_runtime_manifest(&runtime_only).expect("runtime only").files.len(),
        1
    );
    let loader_only = json!({
        "version": "1.0.0",
        "files": [
            { "name": "winmm.dll", "size": 512, "sha256": sha_b }
        ]
    });
    assert!(parse_runtime_manifest(&loader_only).is_none());

    let unknown_file = json!({
        "version": "1.0.0",
        "files": [
            { "name": "Rustic-Isles.dll", "size": 1024, "sha256": sha_a },
            { "name": "other.dll", "size": 512, "sha256": sha_b }
        ]
    });
    assert!(parse_runtime_manifest(&unknown_file).is_none());

    let uppercase_hash = json!({
        "version": "1.0.0",
        "files": [
            { "name": "Rustic-Isles.dll", "size": 1024, "sha256": "A".repeat(64) },
            { "name": "winmm.dll", "size": 512, "sha256": sha_b }
        ]
    });
    assert!(parse_runtime_manifest(&uppercase_hash).is_none());

    let zero_size = json!({
        "version": "1.0.0",
        "files": [
            { "name": "Rustic-Isles.dll", "size": 0, "sha256": sha_a },
            { "name": "winmm.dll", "size": 512, "sha256": sha_b }
        ]
    });
    assert!(parse_runtime_manifest(&zero_size).is_none());

    let oversized = json!({
        "version": "1.0.0",
        "files": [
            { "name": "Rustic-Isles.dll", "size": MAX_RUNTIME_FILE_BYTES + 1, "sha256": sha_a },
            { "name": "winmm.dll", "size": 512, "sha256": sha_b }
        ]
    });
    assert!(parse_runtime_manifest(&oversized).is_none());

    let control_only_version = json!({
        "version": "\u{0000}\u{0007}",
        "files": [
            { "name": "Rustic-Isles.dll", "size": 1024, "sha256": sha_a },
            { "name": "winmm.dll", "size": 512, "sha256": sha_b }
        ]
    });
    assert!(parse_runtime_manifest(&control_only_version).is_none());

    let wide_cl = json!({
        "version": "1.0.0",
        "build": { "changelist": u64::from(u32::MAX) + 5 },
        "files": [
            { "name": "Rustic-Isles.dll", "size": 1024, "sha256": sha_a },
            { "name": "winmm.dll", "size": 512, "sha256": sha_b }
        ]
    });
    // Any safe integer is kept (spec §2.6.3)
    let parsed_cl = parse_runtime_manifest(&wide_cl).expect("valid with its changelist");
    assert_eq!(parsed_cl.build.changelist, u64::from(u32::MAX) + 5);
}

#[tokio::test]
async fn check_runtime_states() {
    let dir = tempfile::tempdir().unwrap();
    let win64 = dir.path().join("win64");
    let local = dir.path().join("local");
    std::fs::create_dir_all(&win64).unwrap();
    std::fs::create_dir_all(&local).unwrap();

    let mp_bytes = b"rustic isles dll bytes";
    let wm_bytes = b"winmm dll bytes";
    let manifest = RuntimeManifest::new(
        "1.0.0".to_owned(),
        DEFAULT_CHANGELIST,
        vec![
            RuntimeFile {
                name: "Rustic-Isles.dll".to_owned(),
                size: mp_bytes.len() as u64,
                sha256: sha256_hex(mp_bytes),
            },
            RuntimeFile {
                name: "winmm.dll".to_owned(),
                size: wm_bytes.len() as u64,
                sha256: sha256_hex(wm_bytes),
            },
        ],
    );

    assert_eq!(
        check_runtime(&win64, None, false, None).await,
        (RuntimeState::Unavailable, false)
    );
    assert_eq!(
        check_runtime(&win64, Some(&manifest), false, None).await,
        (RuntimeState::Untrusted, false)
    );
    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, None).await,
        (RuntimeState::Missing, false)
    );

    std::fs::write(win64.join("Rustic-Isles.dll"), b"old mp").unwrap();
    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, None).await,
        (RuntimeState::Outdated, false)
    );

    std::fs::write(win64.join("Rustic-Isles.dll"), mp_bytes).unwrap();
    std::fs::write(win64.join("winmm.dll"), wm_bytes).unwrap();
    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, None).await,
        (RuntimeState::Ok, true)
    );
    assert_eq!(check_runtime(&win64, None, false, None).await, (RuntimeState::Ok, true));

    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, Some(&local)).await,
        (RuntimeState::Local, false)
    );
    std::fs::write(local.join("Rustic-Isles.dll"), mp_bytes).unwrap();
    std::fs::write(local.join("winmm.dll"), wm_bytes).unwrap();
    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, Some(&local)).await,
        (RuntimeState::Local, true)
    );
}

#[test]
fn replace_file_moves_a_locked_dll_aside_and_puts_it_back_if_the_second_rename_fails() {
    struct MockOps {
        fail_direct: bool,
        fail_second_move: bool,
        renames: RefCell<Vec<(PathBuf, PathBuf)>>,
        removes: RefCell<Vec<PathBuf>>,
    }

    impl ReplaceOps for MockOps {
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.renames.borrow_mut().push((from.to_path_buf(), to.to_path_buf()));
            let count = self.renames.borrow().len();
            if count == 1 && self.fail_direct {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "in use"));
            }
            if count == 3 && self.fail_second_move {
                return Err(io::Error::other("second move failed"));
            }
            Ok(())
        }

        fn remove(&self, file: &Path) -> io::Result<()> {
            self.removes.borrow_mut().push(file.to_path_buf());
            Ok(())
        }
    }

    let tmp = PathBuf::from("win64/winmm.dll.rr-download");
    let target = PathBuf::from("win64/winmm.dll");

    // 1. Direct rename succeeds
    let normal_ops = MockOps {
        fail_direct: false,
        fail_second_move: false,
        renames: RefCell::new(Vec::new()),
        removes: RefCell::new(Vec::new()),
    };
    assert_eq!(replace_file(&tmp, &target, &normal_ops).unwrap(), Replaced::Replaced);
    assert_eq!(normal_ops.renames.borrow().len(), 1);

    // 2. First rename fails, aside rename + second rename succeed -> Replaced::InUse
    let in_use_ops = MockOps {
        fail_direct: true,
        fail_second_move: false,
        renames: RefCell::new(Vec::new()),
        removes: RefCell::new(Vec::new()),
    };
    assert_eq!(replace_file(&tmp, &target, &in_use_ops).unwrap(), Replaced::InUse);
    assert_eq!(in_use_ops.renames.borrow().len(), 3);
    assert_eq!(in_use_ops.removes.borrow().len(), 1);

    // 3. Second rename fails -> restores aside back to target
    let rollback_ops = MockOps {
        fail_direct: true,
        fail_second_move: true,
        renames: RefCell::new(Vec::new()),
        removes: RefCell::new(Vec::new()),
    };
    assert!(replace_file(&tmp, &target, &rollback_ops).is_err());
    let renames = rollback_ops.renames.borrow();
    assert_eq!(renames.len(), 4);
    assert_eq!(renames[3].1, target);
}

#[tokio::test]
async fn install_runtime_downloads_checks_replaces_and_writes_the_marker() {
    let mp_bytes = b"rustic isles fresh content";
    let wm_bytes = b"winmm already current content";
    let manifest = RuntimeManifest::new(
        "2.0.0".to_owned(),
        DEFAULT_CHANGELIST,
        vec![
            RuntimeFile {
                name: "Rustic-Isles.dll".to_owned(),
                size: mp_bytes.len() as u64,
                sha256: sha256_hex(mp_bytes),
            },
            RuntimeFile {
                name: "winmm.dll".to_owned(),
                size: wm_bytes.len() as u64,
                sha256: sha256_hex(wm_bytes),
            },
        ],
    );

    let mp_hits = Arc::new(AtomicUsize::new(0));
    let wm_hits = Arc::new(AtomicUsize::new(0));
    let mut routes = HashMap::new();
    routes.insert(
        format!("{RUNTIME_FILES_BASE_PATH}/Rustic-Isles.dll"),
        MockRoute {
            status: 200,
            body: mp_bytes.to_vec(),
            require_auth: Some("Bearer test-key".to_owned()),
            hits: Arc::clone(&mp_hits),
        },
    );
    routes.insert(
        format!("{RUNTIME_FILES_BASE_PATH}/winmm.dll"),
        MockRoute {
            status: 200,
            body: wm_bytes.to_vec(),
            require_auth: Some("Bearer test-key".to_owned()),
            hits: Arc::clone(&wm_hits),
        },
    );

    let (endpoint, handle) = start_tls_server(routes).await;
    let dir = tempfile::tempdir().unwrap();
    let win64 = dir.path().join("win64");
    std::fs::create_dir_all(&win64).unwrap();

    // Pre-populate winmm.dll so it is skipped
    std::fs::write(win64.join("winmm.dll"), wm_bytes).unwrap();
    // Pre-populate an old aside file so sweep_aside cleans it
    let old_aside = win64.join("Rustic-Isles.dll.rr-old-12345");
    std::fs::write(&old_aside, b"old").unwrap();

    let res = install_runtime(InstallRequest {
        endpoint: &endpoint,
        headers: vec![("authorization", "Bearer test-key".to_owned())],
        manifest: &manifest,
        win64_dir: &win64,
        cancel: None,
        on_progress: None,
    })
    .await
    .unwrap();

    assert!(res.replaced_in_use.is_empty());
    assert!(!old_aside.exists());
    assert_eq!(mp_hits.load(Ordering::SeqCst), 1);
    assert_eq!(wm_hits.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(win64.join("Rustic-Isles.dll")).unwrap(), mp_bytes);
    assert!(win64.join(RUNTIME_MARKER_FILE).is_file());

    handle.abort();
}

#[tokio::test]
async fn install_runtime_takes_a_runtime_a_host_lists_under_its_name_from_before_the_rename() {
    let dll = b"runtime the host still lists by its old name";
    let wm_bytes = b"winmm already current content";
    let manifest = parse_runtime_manifest(&json!({
        "version": "1.9.0",
        "files": [
            { "name": "MysticParadox.dll", "size": dll.len(), "sha256": sha256_hex(dll) },
            { "name": "winmm.dll", "size": wm_bytes.len(), "sha256": sha256_hex(wm_bytes) }
        ]
    }))
    .unwrap();
    assert_eq!(manifest.files[0].name, "MysticParadox.dll");

    let hits = Arc::new(AtomicUsize::new(0));
    let mut routes = HashMap::new();
    routes.insert(
        format!("{RUNTIME_FILES_BASE_PATH}/MysticParadox.dll"),
        MockRoute {
            status: 200,
            body: dll.to_vec(),
            require_auth: None,
            hits: Arc::clone(&hits),
        },
    );
    let (endpoint, handle) = start_tls_server(routes).await;
    let dir = tempfile::tempdir().unwrap();
    let win64 = dir.path().join("win64");
    std::fs::create_dir_all(&win64).unwrap();
    std::fs::write(win64.join("winmm.dll"), wm_bytes).unwrap();
    std::fs::write(win64.join("MysticParadox.dll"), b"an older runtime").unwrap();

    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, None).await,
        (RuntimeState::Outdated, false)
    );
    install_runtime(InstallRequest {
        endpoint: &endpoint,
        headers: Vec::new(),
        manifest: &manifest,
        win64_dir: &win64,
        cancel: None,
        on_progress: None,
    })
    .await
    .unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(win64.join("Rustic-Isles.dll")).unwrap(), dll);
    assert!(!win64.join("MysticParadox.dll").exists());
    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, None).await,
        (RuntimeState::Ok, true)
    );

    // A game folder an older launcher filled is current without a download
    std::fs::rename(win64.join("Rustic-Isles.dll"), win64.join("MysticParadox.dll")).unwrap();
    assert_eq!(
        check_runtime(&win64, Some(&manifest), true, None).await,
        (RuntimeState::Ok, true)
    );

    handle.abort();
}

#[test]
fn manifest_refuses_both_names_of_the_runtime() {
    let sha = "a".repeat(64);
    let both = json!({
        "version": "1.0",
        "files": [
            { "name": "MysticParadox.dll", "size": 1, "sha256": sha },
            { "name": "Rustic-Isles.dll", "size": 1, "sha256": sha }
        ]
    });
    assert!(parse_runtime_manifest(&both).is_none());
}

#[tokio::test]
async fn install_runtime_refuses_a_wrong_hash_a_401_and_a_wrong_pin() {
    let mp_bytes = b"expected rustic isles bytes";
    let wm_bytes = b"expected winmm bytes";
    let manifest = RuntimeManifest::new(
        "1.0.0".to_owned(),
        DEFAULT_CHANGELIST,
        vec![
            RuntimeFile {
                name: "Rustic-Isles.dll".to_owned(),
                size: mp_bytes.len() as u64,
                sha256: sha256_hex(mp_bytes),
            },
            RuntimeFile {
                name: "winmm.dll".to_owned(),
                size: wm_bytes.len() as u64,
                sha256: sha256_hex(wm_bytes),
            },
        ],
    );

    let mut routes = HashMap::new();
    routes.insert(
        format!("{RUNTIME_FILES_BASE_PATH}/Rustic-Isles.dll"),
        MockRoute {
            status: 200,
            body: b"tampered rustic isles bytes".to_vec(),
            require_auth: Some("Bearer valid-key".to_owned()),
            hits: Arc::new(AtomicUsize::new(0)),
        },
    );
    routes.insert(
        format!("{RUNTIME_FILES_BASE_PATH}/winmm.dll"),
        MockRoute {
            status: 200,
            body: wm_bytes.to_vec(),
            require_auth: Some("Bearer valid-key".to_owned()),
            hits: Arc::new(AtomicUsize::new(0)),
        },
    );

    let (endpoint, handle) = start_tls_server(routes).await;
    let dir = tempfile::tempdir().unwrap();
    let win64 = dir.path().join("win64");
    std::fs::create_dir_all(&win64).unwrap();

    // 1. 401 Unauthorized -> KeyRejected
    let err_401 = install_runtime(InstallRequest {
        endpoint: &endpoint,
        headers: vec![("authorization", "Bearer wrong-key".to_owned())],
        manifest: &manifest,
        win64_dir: &win64,
        cancel: None,
        on_progress: None,
    })
    .await
    .unwrap_err();
    assert_eq!(err_401.code, RuntimeErrorCode::KeyRejected);
    assert!(!win64.join("Rustic-Isles.dll.rr-download").exists());

    // 2. Wrong hash -> Different
    let err_hash = install_runtime(InstallRequest {
        endpoint: &endpoint,
        headers: vec![("authorization", "Bearer valid-key".to_owned())],
        manifest: &manifest,
        win64_dir: &win64,
        cancel: None,
        on_progress: None,
    })
    .await
    .unwrap_err();
    assert_eq!(err_hash.code, RuntimeErrorCode::Different);
    assert!(!win64.join("Rustic-Isles.dll.rr-download").exists());

    // 3. Wrong pin -> CertMismatch
    let wrong_pin_endpoint = Endpoint {
        host: endpoint.host.clone(),
        port: endpoint.port,
        pin: Some("f".repeat(64)),
    };
    let err_pin = install_runtime(InstallRequest {
        endpoint: &wrong_pin_endpoint,
        headers: vec![("authorization", "Bearer valid-key".to_owned())],
        manifest: &manifest,
        win64_dir: &win64,
        cancel: None,
        on_progress: None,
    })
    .await
    .unwrap_err();
    assert_eq!(err_pin.code, RuntimeErrorCode::CertMismatch);
    assert!(!win64.join("Rustic-Isles.dll.rr-download").exists());

    handle.abort();
}
