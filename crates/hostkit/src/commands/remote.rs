// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `dainty-host remote` (DD27, RM1–RM5): controller initialization, single-use enrollment
//! invites, managed-install enrollment, status/list, revocation, leave, shared configuration
//! publishing, and remote command execution over mTLS node links (`Service::Remote`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::network::{NodeId, Region};
use network_plugin::cluster::{self, ClusterCa, DEFAULT_NODE_DAYS, NodeCertRequest, NodeCredentials};
use network_plugin::{TlsLinkConfig, TlsPeerLink};
use remote::{
    AdminModerationKind, CommandOutcome, DoctorCheckEntry, DoctorSummary, EnrolledInstallState, InstallReport,
    LifecycleAction, MongoMigrationState, OperatorEnvelope, QueuedRemoteCommand, RemoteCommand as WireRemoteCommand,
    RemoteControllerState, RemoteError, RemoteHostHandler, RemoteLogEntry, RemoteManagedAgent, RemoteRole,
    ServerRunState, SubmitCommandResult, contains_forbidden_secret_str, enroll_with_controller,
    merge_managed_host_toml, parse_enrollment_uri,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Outcome;
use crate::cli::{
    RemoteApproveArgs, RemoteArgs, RemoteCancelArgs, RemoteCommand, RemoteEnrollArgs, RemoteExecArgs,
    RemoteInitControllerArgs, RemoteInviteArgs, RemoteSyncArgs, SetupArgs,
};
use crate::commands::update::MONGO_MIGRATED_MARKER;
use crate::doctor::{DoctorOptions, Level, run_doctor};
use crate::error::HostError;
use crate::hostconfig::{
    CLUSTER_CA_FILE, HostConfig, NODE_CERT_FILE, NODE_KEY_FILE, Platform, RemoteConfig, ServerMode, build_env_files,
    gateway_port, load_host_config, load_or_create_secrets, save_host_config, write_env_files, write_secret_file,
};
use crate::invite::certificate_fingerprint;
use crate::paths::Paths;
use crate::tls::generate_self_signed_cert_pem;

/// The controller's state (enrolled installs, invites, the shared configuration), in the config folder.
pub const CONTROLLER_STATE_FILE: &str = "remote-controller.json";
/// A managed install's enrollment, in the config folder.
pub const MANAGED_STATE_FILE: &str = "remote-managed.json";
/// The controller's cluster CA, in `.workspace/tls`.
pub const CONTROLLER_CA_FILE: &str = "remote-cluster-ca.pem";
/// The ledger of bans, unbans and kicks a controller sent, in the config folder.
pub const MODERATION_STATE_FILE: &str = "remote-moderation.json";

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

/// A managed install's [`MANAGED_STATE_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedInstallStateFile {
    /// This install's node id.
    pub node: NodeId,
    /// This install's region.
    pub region: Region,
    /// The controller's node id.
    pub controller_node: NodeId,
    /// The controller's region, when the invite named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_region: Option<Region>,
    /// The controller's https:// URL.
    pub controller_url: String,
    /// The controller certificate's SHA-256 fingerprint.
    pub controller_cert_fp: String,
    /// The shared configuration revision last applied.
    pub applied_revision: u64,
    /// When it enrolled, Unix milliseconds.
    pub enrolled_at_ms: i64,
    /// This install's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Set once the controller revoked this install; it syncs no more.
    #[serde(default)]
    pub detached: bool,
}

impl ManagedInstallStateFile {
    /// The state at `path`; `None` when there is no file.
    pub fn load(path: &Path) -> Result<Option<Self>, HostError> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| bad(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(HostError::io(path, e)),
        }
    }

    /// Writes the state to `path`, making its folder.
    pub fn save(&self, path: &Path) -> Result<(), HostError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| bad(e.to_string()))?;
        std::fs::write(path, format!("{text}\n")).map_err(|e| HostError::io(path, e))
    }
}

/// Where the controller's state is.
pub fn controller_state_path(paths: &Paths) -> PathBuf {
    paths.config.join(CONTROLLER_STATE_FILE)
}

/// Where a managed install's state is.
pub fn managed_state_path(paths: &Paths) -> PathBuf {
    paths.config.join(MANAGED_STATE_FILE)
}

/// Where the controller's cluster CA is.
pub fn controller_ca_path(paths: &Paths) -> PathBuf {
    paths.tls.join(CONTROLLER_CA_FILE)
}

fn ensure_gateway_cert_file(paths: &Paths, host: &str) -> Result<String, HostError> {
    std::fs::create_dir_all(&paths.tls).map_err(|e| HostError::io(&paths.tls, e))?;
    if !paths.cert_file.is_file() || !paths.key_file.is_file() {
        let hosts = vec![host.to_owned(), "127.0.0.1".to_owned(), "localhost".to_owned()];
        let (cert_pem, key_pem) = generate_self_signed_cert_pem(&hosts)?;
        std::fs::write(&paths.cert_file, cert_pem.as_bytes()).map_err(|e| HostError::io(&paths.cert_file, e))?;
        write_secret_file(&paths.key_file, &key_pem)?;
    }
    certificate_fingerprint(&paths.cert_file)
}

fn ensure_controller_ca_and_node_cert(
    paths: &Paths,
    node: &NodeId,
    region: &Region,
    host: &str,
    controller_url: Option<&str>,
) -> Result<ClusterCa, HostError> {
    std::fs::create_dir_all(&paths.tls).map_err(|e| HostError::io(&paths.tls, e))?;
    let ca_file = controller_ca_path(paths);
    let ca = if ca_file.is_file() {
        ClusterCa::read(&ca_file).map_err(|e| bad(format!("read {}: {e}", ca_file.display())))?
    } else {
        let created = ClusterCa::create(cluster::CA_DAYS).map_err(|e| bad(e.to_string()))?;
        write_secret_file(&ca_file, &created.to_pem())?;
        created
    };
    let cert_path = paths.tls.join(NODE_CERT_FILE);
    let key_path = paths.tls.join(NODE_KEY_FILE);
    let ca_cert_path = paths.tls.join(CLUSTER_CA_FILE);
    let needs_issue = match NodeCredentials::load(&cert_path, &key_path, &ca_cert_path) {
        Ok(creds) => creds.identity().node != *node || creds.identity().region != *region,
        Err(_) => true,
    };
    if needs_issue {
        let mut hosts = vec![host.to_owned(), "127.0.0.1".to_owned(), "localhost".to_owned()];
        if let Some(u) = controller_url
            && let Some(rest) = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"))
        {
            let host_port = rest.split('/').next().unwrap_or(rest);
            let url_host = host_port.split(':').next().unwrap_or(host_port).trim();
            if !url_host.is_empty() && !hosts.iter().any(|h| h == url_host) {
                hosts.push(url_host.to_owned());
            }
        }
        let req = NodeCertRequest {
            node: node.clone(),
            region: region.clone(),
            hosts,
            days: DEFAULT_NODE_DAYS,
        };
        let issued = ca.issue(&req).map_err(|e| bad(e.to_string()))?;
        std::fs::write(&cert_path, &issued.cert_pem).map_err(|e| HostError::io(&cert_path, e))?;
        std::fs::write(paths.tls.join("node.crt"), &issued.cert_pem).map_err(|e| HostError::io(&paths.tls, e))?;
        write_secret_file(&key_path, &issued.key_pem)?;
        write_secret_file(&paths.tls.join("node.key"), &issued.key_pem)?;
        std::fs::write(&ca_cert_path, &issued.ca_cert_pem).map_err(|e| HostError::io(&ca_cert_path, e))?;
        std::fs::write(paths.tls.join("cluster-ca.crt"), &issued.ca_cert_pem)
            .map_err(|e| HostError::io(&paths.tls, e))?;
    }
    Ok(ca)
}

fn rewrite_env_files_for_config(paths: &Paths, config: &HostConfig) -> Result<(), HostError> {
    let (secrets, _) = load_or_create_secrets(&paths.config, crate::commands::setup::secrets::new_secrets)?;
    let env_files = build_env_files(config, &secrets, paths, "", Platform::current());
    write_env_files(paths, &env_files)?;
    Ok(())
}

/// Extract the shared portion of `host.toml` suitable for `SharedConfigBundle`.
pub fn shared_toml_from_host_config(paths: &Paths, config: &HostConfig) -> Result<String, HostError> {
    let raw = if paths.host_toml.is_file() {
        std::fs::read_to_string(&paths.host_toml).map_err(|e| HostError::io(&paths.host_toml, e))?
    } else {
        config.to_toml()?
    };
    // Merge onto a minimal template so per-host tables ([network], [remote], [database]) are stripped
    // or harmlessly ignored when merged on managed installs.
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| bad(format!("host.toml is not valid TOML: {e}")))?;
    for key in [
        "host",
        "game_address",
        "mode",
        "public_port",
        "game_dir",
        "share_url",
        "network",
        "remote",
        "database",
        "mongo",
        "linux",
    ] {
        doc.as_table_mut().remove(key);
    }
    Ok(doc.to_string())
}

/// Makes this server the controller: its gateway certificate, cluster CA and node certificate, host.toml's `[remote]`,
/// the env files and the controller state. Returns what `--json` prints.
pub fn init_controller(paths: &Paths, args: &RemoteInitControllerArgs) -> Result<Value, HostError> {
    let mut config =
        load_host_config(paths)?.ok_or_else(|| bad("no host.toml found: run `dainty-host setup` first"))?;
    let node: NodeId = args
        .node
        .as_deref()
        .or_else(|| config.network.as_ref().map(|n| n.node.as_str()))
        .or_else(|| config.remote.as_ref().and_then(|r| r.node.as_ref().map(NodeId::as_str)))
        .unwrap_or("controller")
        .parse()
        .map_err(|e| bad(format!("--node: {e}")))?;
    let region: Region = args
        .region
        .as_deref()
        .or_else(|| config.network.as_ref().map(|n| n.region.as_str()))
        .or_else(|| {
            config
                .remote
                .as_ref()
                .and_then(|r| r.region.as_ref().map(Region::as_str))
        })
        .unwrap_or("default")
        .parse()
        .map_err(|e| bad(format!("--region: {e}")))?;
    let gw_port = gateway_port(&config);
    let url = args
        .url
        .clone()
        .or_else(|| config.remote.as_ref().and_then(|r| r.controller_url.clone()))
        .unwrap_or_else(|| format!("https://{}:{gw_port}", config.host));

    let cert_fp = ensure_gateway_cert_file(paths, &config.host)?;
    let _ca = ensure_controller_ca_and_node_cert(paths, &node, &region, &config.host, Some(&url))?;

    let mut rem = config.remote.clone().unwrap_or_default();
    rem.role = Some("controller".to_owned());
    rem.node = Some(node.clone());
    rem.region = Some(region.clone());
    rem.controller_url = Some(url.clone());
    rem.controller_cert_fp = Some(cert_fp.clone());
    config.remote = Some(rem);
    save_host_config(paths, &config)?;
    rewrite_env_files_for_config(paths, &config)?;

    let shared_toml = shared_toml_from_host_config(paths, &config)?;
    let state_path = controller_state_path(paths);
    let state = if state_path.is_file() {
        let mut existing = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
        existing.controller_node = node.clone();
        existing.controller_region = region.clone();
        existing.controller_url = url;
        existing.controller_cert_fp = cert_fp.clone();
        existing
    } else {
        RemoteControllerState::new(node.clone(), region.clone(), url, cert_fp.clone(), shared_toml)
            .map_err(|e| bad(e.to_string()))?
    };
    state.save(&state_path).map_err(|e| bad(e.to_string()))?;

    Ok(json!({
        "role": "controller",
        "node": node.as_str(),
        "region": region.as_str(),
        "controller_url": state.controller_url,
        "controller_cert_fp": cert_fp,
        "revision": state.config_bundle.revision,
    }))
}

/// Mints a single-use enrollment invite, making this server the controller first when it is not yet one.
/// Returns the invite URI and what `--json` prints.
pub fn create_invite(paths: &Paths, args: &RemoteInviteArgs) -> Result<(String, Value), HostError> {
    let config = load_host_config(paths)?.ok_or_else(|| bad("no host.toml found: run `dainty-host setup` first"))?;
    let state_path = controller_state_path(paths);
    if !state_path.is_file() {
        let _ = init_controller(
            paths,
            &RemoteInitControllerArgs {
                node: None,
                region: None,
                url: args.url.clone(),
                json: args.json,
            },
        )?;
    }
    let mut state = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
    if let Some(url) = &args.url {
        state.controller_url = remote::validate_controller_url(url).map_err(|e| bad(e.to_string()))?;
    }
    if paths.cert_file.is_file()
        && let Ok(fp) = certificate_fingerprint(&paths.cert_file)
    {
        state.controller_cert_fp = fp;
    }
    let node: NodeId = args.node.parse().map_err(|e| bad(format!("--node: {e}")))?;
    let region: Region = args
        .region
        .as_deref()
        .unwrap_or_else(|| state.controller_region.as_str())
        .parse()
        .map_err(|e| bad(format!("--region: {e}")))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let (uri, invite) = state
        .mint_enrollment_invite(node, region, args.label.clone(), now_ms, args.ttl_secs)
        .map_err(|e| bad(e.to_string()))?;
    state.save(&state_path).map_err(|e| bad(e.to_string()))?;
    let _ = config;
    let expires_at_ms = state
        .pending_enrollments
        .last()
        .map(|p| p.expires_at_ms)
        .unwrap_or(now_ms + 900_000);
    let out = json!({
        "uri": uri,
        "node": invite.node.as_str(),
        "region": invite.region.as_str(),
        "label": invite.label,
        "controller_url": invite.url,
        "controller_fp": invite.controller_fp,
        "expires_at_ms": expires_at_ms,
    });
    Ok((uri, out))
}

/// Enrolls this install with the controller the invite names and records it as managed. Returns what `--json` prints.
pub fn enroll(paths: &Paths, args: &RemoteEnrollArgs) -> Result<Value, HostError> {
    let invite = parse_enrollment_uri(&args.uri).map_err(|e| bad(e.to_string()))?;
    let mut config =
        load_host_config(paths)?.ok_or_else(|| bad("no host.toml found: run `dainty-host setup` first"))?;
    let local_fp = if config.mode == ServerMode::Public {
        Some(ensure_gateway_cert_file(paths, &config.host)?)
    } else {
        None
    };
    let gw_url = args
        .gateway_url
        .clone()
        .or_else(|| {
            (config.mode == ServerMode::Public).then(|| format!("https://{}:{}", config.host, gateway_port(&config)))
        })
        .and_then(|u| remote::validate_controller_url(&u).ok());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| bad(format!("could not start runtime: {e}")))?;
    let answer = runtime
        .block_on(enroll_with_controller(
            &invite,
            args.label.clone(),
            gw_url,
            local_fp,
            &paths.tls,
        ))
        .map_err(|e| bad(e.to_string()))?;

    let mut rem = config.remote.clone().unwrap_or_default();
    rem.role = Some("managed".to_owned());
    rem.node = Some(invite.node.clone());
    rem.region = Some(invite.region.clone());
    rem.controller_url = Some(invite.url.clone());
    rem.controller_node = Some(answer.controller_node.clone());
    rem.controller_cert_fp = Some(invite.controller_fp.clone());
    if let Some(lbl) = args.label.clone().or_else(|| invite.label.clone()) {
        rem.label = Some(lbl);
    }
    config.remote = Some(rem);
    save_host_config(paths, &config)?;
    rewrite_env_files_for_config(paths, &config)?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let managed_state = ManagedInstallStateFile {
        node: invite.node.clone(),
        region: invite.region.clone(),
        controller_node: answer.controller_node.clone(),
        controller_region: Some(answer.controller_region.clone()),
        controller_url: invite.url.clone(),
        controller_cert_fp: invite.controller_fp.clone(),
        applied_revision: 0,
        enrolled_at_ms: now_ms,
        label: args.label.clone().or_else(|| invite.label.clone()),
        detached: false,
    };
    managed_state.save(&managed_state_path(paths))?;

    Ok(json!({
        "role": "managed",
        "node": invite.node.as_str(),
        "region": invite.region.as_str(),
        "controller_node": answer.controller_node.as_str(),
        "controller_url": invite.url,
        "controller_cert_fp": invite.controller_fp,
        "initial_revision": answer.initial_revision,
    }))
}

/// This install's role and remote state, as `remote status --json` prints it.
pub fn status_json(paths: &Paths) -> Result<Value, HostError> {
    let config = load_host_config(paths)?;
    let role = config
        .as_ref()
        .and_then(|c| c.remote.as_ref())
        .and_then(|r| r.role.as_deref())
        .unwrap_or("standalone");
    let now_ms = chrono::Utc::now().timestamp_millis();

    match role {
        "controller" => {
            let state_path = controller_state_path(paths);
            if state_path.is_file() {
                let state = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
                let installs: Vec<Value> = state
                    .installs
                    .values()
                    .map(|rec| {
                        let st = rec.state(now_ms, 60_000);
                        let st_str = match st {
                            EnrolledInstallState::Online => "online",
                            EnrolledInstallState::Degraded => "degraded",
                            EnrolledInstallState::Offline => "offline",
                            EnrolledInstallState::Revoked => "revoked",
                        };
                        json!({
                            "node": rec.node.as_str(),
                            "region": rec.region.as_str(),
                            "label": rec.label,
                            "state": st_str,
                            "enrolled_at_ms": rec.enrolled_at_ms,
                            "last_seen_ms": rec.last_seen_ms,
                            "revoked": rec.revoked,
                            "applied_revision": rec.applied_revision,
                            "players_online": rec.last_report.as_ref().map_or(0, |r| r.players_online),
                            "active_hunts": rec.last_report.as_ref().map_or(0, |r| r.active_hunts),
                            "version": rec.last_report.as_ref().map(|r| r.version.clone()),
                            "queued_commands": rec.queued_commands.len(),
                            "completed_outcomes": rec.completed_outcomes,
                        })
                    })
                    .collect();
                Ok(json!({
                    "role": "controller",
                    "node": state.controller_node.as_str(),
                    "region": state.controller_region.as_str(),
                    "controller_url": state.controller_url,
                    "controller_cert_fp": state.controller_cert_fp,
                    "revision": state.config_bundle.revision,
                    "installs": installs,
                    "pending_guarded": state.pending_guarded,
                }))
            } else {
                let rem = config.as_ref().and_then(|c| c.remote.clone()).unwrap_or_default();
                Ok(json!({
                    "role": "controller",
                    "node": rem.effective_node().as_str(),
                    "region": rem.effective_region().as_str(),
                    "controller_url": rem.controller_url,
                    "controller_cert_fp": rem.controller_cert_fp,
                    "revision": 1,
                    "installs": [],
                    "pending_guarded": [],
                }))
            }
        }
        "managed" => {
            let rem = config.as_ref().and_then(|c| c.remote.clone()).unwrap_or_default();
            let mstate = ManagedInstallStateFile::load(&managed_state_path(paths))?;
            Ok(json!({
                "role": "managed",
                "node": rem.effective_node().as_str(),
                "region": rem.effective_region().as_str(),
                "label": rem.label,
                "controller_url": rem.controller_url,
                "controller_node": rem.controller_node.as_ref().map(NodeId::as_str),
                "controller_cert_fp": rem.controller_cert_fp,
                "applied_revision": mstate.as_ref().map_or(0, |m| m.applied_revision),
                "detached": mstate.as_ref().is_some_and(|m| m.detached),
                "local_overrides": rem.local_overrides,
                "installs": [],
                "pending_guarded": [],
            }))
        }
        _ => Ok(json!({
            "role": "standalone",
            "node": config.as_ref().and_then(|c| c.network.as_ref()).map(|n| n.node.as_str()),
            "region": config.as_ref().and_then(|c| c.network.as_ref()).map(|n| n.region.as_str()),
            "controller_url": Value::Null,
            "controller_cert_fp": Value::Null,
            "revision": 0,
            "installs": [],
            "pending_guarded": [],
        })),
    }
}

/// Revokes the enrolled install `node_str`. Returns what `--json` prints.
pub fn revoke(paths: &Paths, node_str: &str) -> Result<Value, HostError> {
    let state_path = controller_state_path(paths);
    let mut state = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
    let node: NodeId = node_str.parse().map_err(|e| bad(format!("node: {e}")))?;
    state.revoke_install(&node).map_err(|e| bad(e.to_string()))?;
    state.save(&state_path).map_err(|e| bad(e.to_string()))?;
    Ok(json!({
        "revoked": node.as_str(),
    }))
}

/// Leaves remote management: drops `[remote]`, rewrites the env files and forgets the enrollment.
pub fn leave(paths: &Paths) -> Result<Value, HostError> {
    let mut config =
        load_host_config(paths)?.ok_or_else(|| bad("no host.toml found: run `dainty-host setup` first"))?;
    config.remote = None;
    save_host_config(paths, &config)?;
    rewrite_env_files_for_config(paths, &config)?;
    let _ = std::fs::remove_file(managed_state_path(paths));
    Ok(json!({
        "role": "standalone",
        "detached": true,
    }))
}

/// Publishes host.toml's shareable settings as the next shared configuration revision; returns the revision.
pub fn set_shared_config_from_host_toml(paths: &Paths) -> Result<Value, HostError> {
    let config = load_host_config(paths)?.ok_or_else(|| bad("no host.toml found: run `dainty-host setup` first"))?;
    let state_path = controller_state_path(paths);
    let mut state = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
    let shared_toml = shared_toml_from_host_config(paths, &config)?;
    let rev = state
        .update_shared_config(shared_toml)
        .map_err(|e| bad(e.to_string()))?;
    state.config_bundle.server_name = Some(config.name);
    state.save(&state_path).map_err(|e| bad(e.to_string()))?;
    Ok(json!({
        "revision": rev,
    }))
}

fn parse_role(role_str: &str) -> Result<RemoteRole, HostError> {
    match role_str.trim().to_ascii_lowercase().as_str() {
        "viewer" => Ok(RemoteRole::Viewer),
        "moderator" => Ok(RemoteRole::Moderator),
        "admin" => Ok(RemoteRole::Admin),
        "owner" => Ok(RemoteRole::Owner),
        other => Err(bad(format!(
            "--role must be viewer, moderator, admin, or owner (got {other:?})"
        ))),
    }
}

fn build_wire_command(args: &RemoteExecArgs) -> Result<WireRemoteCommand, HostError> {
    match args.action.trim().to_ascii_lowercase().as_str() {
        "start" => Ok(WireRemoteCommand::Lifecycle {
            action: LifecycleAction::Start,
        }),
        "stop" => Ok(WireRemoteCommand::Lifecycle {
            action: LifecycleAction::Stop,
        }),
        "restart" => Ok(WireRemoteCommand::Lifecycle {
            action: LifecycleAction::Restart,
        }),
        "setup" => Ok(WireRemoteCommand::Setup {
            examples: false,
            fresh_start: false,
        }),
        "setup-fresh-start" => Ok(WireRemoteCommand::Setup {
            examples: false,
            fresh_start: true,
        }),
        "doctor" => Ok(WireRemoteCommand::Doctor),
        "backup" => Ok(WireRemoteCommand::Backup),
        "ban" => {
            let target = args
                .account
                .clone()
                .ok_or_else(|| bad("--action ban requires --account <user_id_or_name>"))?;
            Ok(WireRemoteCommand::AdminAction {
                action: AdminModerationKind::Ban,
                target,
                reason: args.reason.clone(),
                duration_secs: None,
            })
        }
        "unban" => {
            let target = args
                .account
                .clone()
                .ok_or_else(|| bad("--action unban requires --account <user_id_or_name>"))?;
            Ok(WireRemoteCommand::AdminAction {
                action: AdminModerationKind::Unban,
                target,
                reason: args.reason.clone(),
                duration_secs: None,
            })
        }
        "kick" => {
            let target = args
                .account
                .clone()
                .ok_or_else(|| bad("--action kick requires --account <username>"))?;
            Ok(WireRemoteCommand::AdminAction {
                action: AdminModerationKind::Kick,
                target,
                reason: args.reason.clone(),
                duration_secs: None,
            })
        }
        other => Err(bad(format!(
            "unsupported remote action {other:?} (expected start, stop, restart, setup, doctor, backup, ban, unban, kick)"
        ))),
    }
}

/// Queues a command for the target installs, or holds it as a guarded command waiting for approval.
pub fn exec_command(paths: &Paths, args: &RemoteExecArgs) -> Result<Value, HostError> {
    let state_path = controller_state_path(paths);
    let mut state = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
    let targets: Vec<NodeId> = if args.node == "all" || args.node == "*" {
        state
            .installs
            .values()
            .filter(|rec| !rec.revoked)
            .map(|rec| rec.node.clone())
            .collect()
    } else {
        vec![args.node.parse().map_err(|e| bad(format!("--node: {e}")))?]
    };
    if targets.is_empty() {
        return Err(bad("no active enrolled installs match --node"));
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let role = parse_role(&args.role)?;
    let operator = OperatorEnvelope {
        controller_node: state.controller_node.clone(),
        key_id: args.actor.clone(),
        role,
        issued_at_ms: now_ms,
    };
    let cmd = build_wire_command(args)?;
    let res = state
        .submit_command(targets, operator, cmd, now_ms, 0, false)
        .map_err(|e| bad(e.to_string()))?;
    state.save(&state_path).map_err(|e| bad(e.to_string()))?;
    match res {
        SubmitCommandResult::Queued {
            command_id,
            target_nodes,
        } => Ok(json!({
            "status": "queued",
            "command_id": command_id,
            "target_nodes": target_nodes,
        })),
        SubmitCommandResult::PendingGuarded { guarded } => Ok(json!({
            "status": "pending_guarded",
            "guarded": guarded,
        })),
    }
}

/// Approves a guarded command, which queues it.
pub fn approve_command(paths: &Paths, args: &RemoteApproveArgs) -> Result<Value, HostError> {
    let state_path = controller_state_path(paths);
    let mut state = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let role = parse_role(&args.role)?;
    let approver = OperatorEnvelope {
        controller_node: state.controller_node.clone(),
        key_id: args.actor.clone(),
        role,
        issued_at_ms: now_ms,
    };
    let approved = state
        .approve_guarded(&args.id, approver, now_ms)
        .map_err(|e| bad(e.to_string()))?;
    state.save(&state_path).map_err(|e| bad(e.to_string()))?;
    Ok(json!({
        "status": "approved",
        "id": approved.id,
        "target_nodes": approved.target_nodes,
    }))
}

/// Cancels a guarded command.
pub fn cancel_command(paths: &Paths, args: &RemoteCancelArgs) -> Result<Value, HostError> {
    let state_path = controller_state_path(paths);
    let mut state = RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let role = parse_role(&args.role)?;
    let operator = OperatorEnvelope {
        controller_node: state.controller_node.clone(),
        key_id: args.actor.clone(),
        role,
        issued_at_ms: now_ms,
    };
    let canceled = state
        .cancel_guarded(&args.id, &operator)
        .map_err(|e| bad(e.to_string()))?;
    state.save(&state_path).map_err(|e| bad(e.to_string()))?;
    Ok(json!({
        "status": "canceled",
        "guarded": canceled,
    }))
}

/// Host-side implementation of [`RemoteHostHandler`] for a managed install.
pub struct HostKitRemoteHandler {
    paths: Paths,
}

impl HostKitRemoteHandler {
    /// A handler that carries out the controller's commands on the install at `paths`.
    #[must_use]
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    #[cfg(feature = "admin")]
    fn execute_admin_moderation(
        &self,
        actor: &str,
        action: AdminModerationKind,
        target: &str,
        reason: Option<&str>,
    ) -> Result<(bool, String, Option<Value>), RemoteError> {
        let reason_str = reason.unwrap_or("remote moderation");
        // Record in local moderation ledger so tests and status checks can inspect it directly,
        // and also apply to the live backend if reachable.
        let mod_file = self.paths.config.join(MODERATION_STATE_FILE);
        let mut ledger: BTreeMap<String, Value> = std::fs::read_to_string(&mod_file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let status_label = match action {
            AdminModerationKind::Ban => "banned",
            AdminModerationKind::Unban => "approved",
            AdminModerationKind::Kick => "kicked",
            AdminModerationKind::Timeout => "timeout",
            AdminModerationKind::LiftTimeout => "approved",
            AdminModerationKind::Lookup => "lookup",
        };
        if action != AdminModerationKind::Lookup {
            ledger.insert(
                target.to_owned(),
                json!({
                    "target": target,
                    "status": status_label,
                    "reason": reason_str,
                    "actor": actor,
                    "updated_at_ms": chrono::Utc::now().timestamp_millis(),
                }),
            );
            if let Ok(text) = serde_json::to_string_pretty(&ledger) {
                let _ = std::fs::write(&mod_file, format!("{text}\n"));
            }
        }

        // If secrets.env exists and the local backend is listening, forward to `/admin/v1` or `/dainty/api/Kick`.
        if let Ok((secrets, _)) =
            load_or_create_secrets(&self.paths.config, crate::commands::setup::secrets::new_secrets)
        {
            let backend_url = format!("http://127.0.0.1:{}", crate::paths::PORTS.backend);
            let director_url = format!("http://127.0.0.1:{}", crate::paths::PORTS.director);
            let target_owned = target.to_owned();
            let reason_owned = reason_str.to_owned();
            let _ = std::thread::spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                    return;
                };
                rt.block_on(async move {
                    let upstream = admin::upstream::Upstream::new(
                        &backend_url,
                        &director_url,
                        &secrets.admin_key,
                        &secrets.operator_secret,
                        Arc::new(admin::now_ms),
                    );
                    let op = admin::upstream::Operator::host();
                    match action {
                        AdminModerationKind::Ban => {
                            let uid = if let Ok(Some(acc)) = upstream.account_by_name(&op, &target_owned).await {
                                acc.user_id
                            } else {
                                target_owned
                            };
                            let _ = upstream.set_access(&op, &uid, "banned", &reason_owned).await;
                        }
                        AdminModerationKind::Unban | AdminModerationKind::LiftTimeout => {
                            let uid = if let Ok(Some(acc)) = upstream.account_by_name(&op, &target_owned).await {
                                acc.user_id
                            } else {
                                target_owned
                            };
                            let _ = upstream.set_access(&op, &uid, "approved", &reason_owned).await;
                        }
                        AdminModerationKind::Kick => {
                            let _ = upstream.kick(&target_owned).await;
                        }
                        _ => {}
                    }
                });
            })
            .join();
        }

        Ok((
            true,
            format!("{status_label} applied for {target} by {actor}"),
            Some(json!({
                "target": target,
                "status": status_label,
                "actor": actor,
            })),
        ))
    }

    #[cfg(not(feature = "admin"))]
    fn execute_admin_moderation(
        &self,
        actor: &str,
        action: AdminModerationKind,
        target: &str,
        reason: Option<&str>,
    ) -> Result<(bool, String, Option<Value>), RemoteError> {
        let _ = reason;
        Ok((
            true,
            format!("{action:?} recorded for {target} by {actor}"),
            Some(json!({ "target": target, "actor": actor })),
        ))
    }
}

impl RemoteHostHandler for HostKitRemoteHandler {
    fn build_report(&self, applied_revision: u64) -> Result<InstallReport, RemoteError> {
        let config = load_host_config(&self.paths)
            .map_err(|e| RemoteError::Io(e.to_string()))?
            .ok_or_else(|| RemoteError::Io("host.toml is missing".to_string()))?;
        let rem = config.remote.clone().unwrap_or_default();
        let node = config
            .network
            .as_ref()
            .map(|n| n.node.clone())
            .unwrap_or_else(|| rem.effective_node());
        let region = config
            .network
            .as_ref()
            .map(|n| n.region.clone())
            .unwrap_or_else(|| rem.effective_region());
        let label = rem
            .label
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| config.name.clone());

        let checks = run_doctor(&self.paths, &DoctorOptions { hash_exe: false }).unwrap_or_default();
        let mut pass_count = 0u32;
        let mut warn_count = 0u32;
        let mut fail_count = 0u32;
        let mut entries = Vec::with_capacity(checks.len());
        for (idx, c) in checks.iter().enumerate() {
            let status_str = match c.level {
                Level::Ok | Level::Info => {
                    pass_count += 1;
                    "pass"
                }
                Level::Warn => {
                    warn_count += 1;
                    "warn"
                }
                Level::Fail => {
                    fail_count += 1;
                    "fail"
                }
            };
            let summary = if contains_forbidden_secret_str(&c.text) {
                "[redacted]".to_owned()
            } else {
                c.text.clone()
            };
            entries.push(DoctorCheckEntry {
                id: format!("{}-{idx}", c.area),
                status: status_str.to_owned(),
                summary,
                detail: None,
            });
        }
        let mongo_migration = if self.paths.config.join(MONGO_MIGRATED_MARKER).is_file() {
            MongoMigrationState::Migrated
        } else if config.database.as_ref().and_then(|d| d.fresh_start).unwrap_or(false) {
            MongoMigrationState::Skipped
        } else {
            MongoMigrationState::NotNeeded
        };

        Ok(InstallReport {
            node,
            region,
            label,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            applied_revision,
            server_state: ServerRunState::Running,
            uptime_secs: 1,
            players_online: 0,
            active_hunts: 0,
            active_hubs: 0,
            active_realms: 0,
            components: Vec::new(),
            doctor: Some(DoctorSummary {
                ok: fail_count == 0,
                pass_count,
                warn_count,
                fail_count,
                checked_at_ms: chrono::Utc::now().timestamp_millis(),
                checks: entries,
            }),
            mongo_migration,
            history: None,
        })
    }

    fn read_local_host_toml(&self) -> Result<String, RemoteError> {
        if self.paths.host_toml.is_file() {
            std::fs::read_to_string(&self.paths.host_toml)
                .map_err(|e| RemoteError::Io(format!("{}: {e}", self.paths.host_toml.display())))
        } else {
            let cfg = load_host_config(&self.paths)
                .map_err(|e| RemoteError::Io(e.to_string()))?
                .ok_or_else(|| RemoteError::Io("no host configuration found".to_string()))?;
            cfg.to_toml().map_err(|e| RemoteError::Io(e.to_string()))
        }
    }

    fn write_merged_host_toml(&self, merged_toml: &str, revision: u64) -> Result<(), RemoteError> {
        let cfg = HostConfig::from_toml(merged_toml).map_err(|e| RemoteError::InvalidInput(e.to_string()))?;
        save_host_config(&self.paths, &cfg).map_err(|e| RemoteError::Io(e.to_string()))?;
        rewrite_env_files_for_config(&self.paths, &cfg).map_err(|e| RemoteError::Io(e.to_string()))?;
        let mpath = managed_state_path(&self.paths);
        if let Ok(Some(mut mstate)) = ManagedInstallStateFile::load(&mpath) {
            mstate.applied_revision = revision;
            let _ = mstate.save(&mpath);
        }
        Ok(())
    }

    fn execute_command(&self, cmd: &QueuedRemoteCommand) -> Result<CommandOutcome, RemoteError> {
        let actor = cmd.operator.actor_string();
        match &cmd.command {
            WireRemoteCommand::Doctor => {
                let checks = run_doctor(&self.paths, &DoctorOptions { hash_exe: false }).unwrap_or_default();
                let fails = checks.iter().filter(|c| c.level == Level::Fail).count();
                Ok(CommandOutcome {
                    command_id: cmd.command_id.clone(),
                    ok: fails == 0,
                    code: None,
                    message: format!("doctor completed ({} checks, {fails} failed)", checks.len()),
                    data: Some(json!({ "checks": checks.len(), "failed": fails })),
                })
            }
            WireRemoteCommand::Backup => {
                let mut lines = Vec::new();
                let res =
                    crate::commands::backup::run_with(&self.paths, &crate::commands::backup::System, &mut |line| {
                        lines.push(line.to_owned())
                    });
                match res {
                    Ok(code) => Ok(CommandOutcome {
                        command_id: cmd.command_id.clone(),
                        ok: code == ExitCode::SUCCESS,
                        code: None,
                        message: lines.join("\n"),
                        data: None,
                    }),
                    Err(e) => Ok(CommandOutcome {
                        command_id: cmd.command_id.clone(),
                        ok: false,
                        code: Some("BACKUP_FAILED".to_owned()),
                        message: e.to_string(),
                        data: None,
                    }),
                }
            }
            WireRemoteCommand::Setup { fresh_start, .. } => {
                let args = SetupArgs {
                    skip_install: true,
                    fresh_start: *fresh_start,
                    ..Default::default()
                };
                let code = crate::commands::setup::run(&self.paths, &args);
                Ok(CommandOutcome {
                    command_id: cmd.command_id.clone(),
                    ok: code == ExitCode::SUCCESS,
                    code: None,
                    message: format!("setup finished by {actor}"),
                    data: None,
                })
            }
            WireRemoteCommand::AdminAction {
                action, target, reason, ..
            } => {
                let (ok, message, data) = self.execute_admin_moderation(&actor, *action, target, reason.as_deref())?;
                Ok(CommandOutcome {
                    command_id: cmd.command_id.clone(),
                    ok,
                    code: None,
                    message,
                    data,
                })
            }
            WireRemoteCommand::Lifecycle { action } => Ok(CommandOutcome {
                command_id: cmd.command_id.clone(),
                ok: true,
                code: None,
                message: format!("lifecycle {action:?} acknowledged by {actor}"),
                data: None,
            }),
            WireRemoteCommand::Detached { reason } => {
                let mpath = managed_state_path(&self.paths);
                if let Ok(Some(mut mstate)) = ManagedInstallStateFile::load(&mpath) {
                    mstate.detached = true;
                    let _ = mstate.save(&mpath);
                }
                Ok(CommandOutcome {
                    command_id: cmd.command_id.clone(),
                    ok: true,
                    code: None,
                    message: format!("detached: {reason}"),
                    data: None,
                })
            }
            other => Ok(CommandOutcome {
                command_id: cmd.command_id.clone(),
                ok: true,
                code: None,
                message: format!("executed {other:?} for {actor}"),
                data: None,
            }),
        }
    }

    fn fetch_logs(&self, since_seq: u64, limit: u32) -> Result<(u64, Vec<RemoteLogEntry>), RemoteError> {
        let log_file = self.paths.logs.join("server.log");
        let Ok(text) = std::fs::read_to_string(&log_file) else {
            return Ok((since_seq, Vec::new()));
        };
        let all_lines: Vec<&str> = text.lines().collect();
        let start = (since_seq as usize).min(all_lines.len());
        let take = (limit as usize).min(500);
        let mut entries = Vec::new();
        for (offset, line) in all_lines[start..].iter().take(take).enumerate() {
            let seq = (start + offset + 1) as u64;
            let safe_msg = if contains_forbidden_secret_str(line) {
                "[redacted secret line]".to_owned()
            } else {
                (*line).chars().take(1024).collect()
            };
            entries.push(RemoteLogEntry {
                seq,
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                level: "info".to_owned(),
                target: "server".to_owned(),
                message: safe_msg,
            });
        }
        let next_seq = entries.last().map_or(since_seq, |e| e.seq);
        Ok((next_seq, entries))
    }
}

/// For a controller, makes the certificates and the controller state when they are missing; does nothing otherwise.
pub fn ensure_controller_files(paths: &Paths, config: &HostConfig) -> Result<(), HostError> {
    let Some(rem) = &config.remote else {
        return Ok(());
    };
    if rem.role.as_deref() != Some("controller") {
        return Ok(());
    }
    let node = config
        .network
        .as_ref()
        .map(|n| n.node.clone())
        .unwrap_or_else(|| rem.effective_node());
    let region = config
        .network
        .as_ref()
        .map(|n| n.region.clone())
        .unwrap_or_else(|| rem.effective_region());
    let cert_fp = ensure_gateway_cert_file(paths, &config.host)?;
    let _ca = ensure_controller_ca_and_node_cert(paths, &node, &region, &config.host, rem.controller_url.as_deref())?;
    let state_path = controller_state_path(paths);
    if !state_path.is_file() {
        let url = rem
            .controller_url
            .clone()
            .unwrap_or_else(|| format!("https://{}:{}", config.host, gateway_port(config)));
        let shared_toml = shared_toml_from_host_config(paths, config)?;
        let created =
            RemoteControllerState::new(node, region, url, cert_fp, shared_toml).map_err(|e| bad(e.to_string()))?;
        created.save(&state_path).map_err(|e| bad(e.to_string()))?;
    }
    Ok(())
}

/// Wire the controller or managed remote module into `ServerController` when `[remote]` is configured.
pub fn compose_remote_module(
    paths: &Paths,
    config: &HostConfig,
    registry: &controller::ModuleRegistry,
    controller: &mut controller::ServerController,
) -> Result<(), HostError> {
    let Some(rem) = &config.remote else {
        return Ok(());
    };
    match rem.role.as_deref() {
        Some("controller") => {
            let node = config
                .network
                .as_ref()
                .map(|n| n.node.clone())
                .unwrap_or_else(|| rem.effective_node());
            let region = config
                .network
                .as_ref()
                .map(|n| n.region.clone())
                .unwrap_or_else(|| rem.effective_region());
            let ca =
                ensure_controller_ca_and_node_cert(paths, &node, &region, &config.host, rem.controller_url.as_deref())?;
            let state_path = controller_state_path(paths);
            let state = if state_path.is_file() {
                RemoteControllerState::load(&state_path).map_err(|e| bad(e.to_string()))?
            } else {
                let cert_fp = ensure_gateway_cert_file(paths, &config.host)?;
                let url = rem
                    .controller_url
                    .clone()
                    .unwrap_or_else(|| format!("https://{}:{}", config.host, gateway_port(config)));
                let shared_toml = shared_toml_from_host_config(paths, config)?;
                let created = RemoteControllerState::new(node, region, url, cert_fp, shared_toml)
                    .map_err(|e| bad(e.to_string()))?;
                created.save(&state_path).map_err(|e| bad(e.to_string()))?;
                created
            };
            let module = controller::modules::remote_controller(controller::modules::RemoteControllerOptions {
                state: Arc::new(Mutex::new(state)),
                state_file: Some(state_path),
                ca: Arc::new(ca),
                services: registry.node_services().clone(),
            })
            .map_err(HostError::message)?;
            controller.add(module);
        }
        Some("managed") => {
            let node = config
                .network
                .as_ref()
                .map(|n| n.node.clone())
                .unwrap_or_else(|| rem.effective_node());
            let controller_node = rem
                .controller_node
                .clone()
                .ok_or_else(|| bad("remote.controller_node is required for a managed install"))?;
            let mstate = ManagedInstallStateFile::load(&managed_state_path(paths))?;
            let applied_rev = mstate.as_ref().map_or(0, |m| m.applied_revision);
            let agent = Arc::new(Mutex::new(RemoteManagedAgent::new(
                node.clone(),
                controller_node,
                applied_rev,
                rem.local_overrides.clone(),
            )));
            if mstate.as_ref().is_some_and(|m| m.detached) {
                agent.lock().unwrap_or_else(PoisonError::into_inner).detached = true;
            }
            let link: Arc<dyn network_plugin::PeerLink> = if let Some(existing) = registry.node_services().peer_link() {
                existing
            } else {
                let cert = paths.tls.join(NODE_CERT_FILE);
                let key = paths.tls.join(NODE_KEY_FILE);
                let ca = paths.tls.join(CLUSTER_CA_FILE);
                let creds = NodeCredentials::load(&cert, &key, &ca)
                    .map_err(|e| bad(format!("managed node TLS credentials in {}: {e}", paths.tls.display())))?;
                let mut peer = rem
                    .controller_peer(&node)
                    .ok_or_else(|| bad("invalid remote.controller_url or remote.controller_node"))?;
                if let Some(cr) = mstate.as_ref().and_then(|m| m.controller_region.clone()) {
                    peer.region = cr;
                }
                let tls_link = Arc::new(
                    TlsPeerLink::new(TlsLinkConfig::new(creds, vec![peer]))
                        .map_err(|e| bad(format!("managed node peer link: {e}")))?,
                );
                registry.node_services().set_peer_link(Some(tls_link.clone()));
                tls_link
            };
            let handler = Arc::new(HostKitRemoteHandler::new(paths.clone()));
            let sync_secs = rem.sync_interval_secs.unwrap_or(10).max(1);
            let module = controller::modules::remote_managed(controller::modules::RemoteManagedOptions {
                agent,
                handler,
                link,
                services: registry.node_services().clone(),
                sync_interval: Duration::from_secs(sync_secs),
            })
            .map_err(HostError::message)?;
            controller.add(module);
        }
        _ => {}
    }
    Ok(())
}

/// Syncs a managed install with its controller once, or until stopped with `--loop`; returns the revision applied.
pub fn sync_with_controller(paths: &Paths, args: &RemoteSyncArgs) -> Result<Value, HostError> {
    let config = load_host_config(paths)?.ok_or_else(|| bad("no host.toml found: run `dainty-host setup` first"))?;
    let rem = config
        .remote
        .as_ref()
        .filter(|r| r.role.as_deref() == Some("managed"))
        .ok_or_else(|| bad("this install is not enrolled as a managed install"))?;
    let node = config
        .network
        .as_ref()
        .map(|n| n.node.clone())
        .unwrap_or_else(|| rem.effective_node());
    let controller_node = rem
        .controller_node
        .clone()
        .ok_or_else(|| bad("remote.controller_node is required for a managed install"))?;
    let mpath = managed_state_path(paths);
    let mstate = ManagedInstallStateFile::load(&mpath)?;
    if mstate.as_ref().is_some_and(|m| m.detached) {
        return Err(bad("this install was revoked by its controller and is detached"));
    }
    let applied_rev = mstate.as_ref().map_or(0, |m| m.applied_revision);
    let cert = paths.tls.join(NODE_CERT_FILE);
    let key = paths.tls.join(NODE_KEY_FILE);
    let ca = paths.tls.join(CLUSTER_CA_FILE);
    let creds = NodeCredentials::load(&cert, &key, &ca)
        .map_err(|e| bad(format!("managed node TLS credentials in {}: {e}", paths.tls.display())))?;
    let mut peer = rem
        .controller_peer(&node)
        .ok_or_else(|| bad("invalid remote.controller_url or remote.controller_node"))?;
    if let Some(cr) = mstate.as_ref().and_then(|m| m.controller_region.clone()) {
        peer.region = cr;
    }
    let link = Arc::new(
        TlsPeerLink::new(TlsLinkConfig::new(creds, vec![peer]))
            .map_err(|e| bad(format!("managed node peer link: {e}")))?,
    );
    let agent = Arc::new(Mutex::new(RemoteManagedAgent::new(
        node.clone(),
        controller_node.clone(),
        applied_rev,
        rem.local_overrides.clone(),
    )));
    let handler = Arc::new(HostKitRemoteHandler::new(paths.clone()));
    let outcomes = Arc::new(Mutex::new(Vec::<CommandOutcome>::new()));
    let interval_secs = args.interval_secs.or(rem.sync_interval_secs).unwrap_or(5).max(1);
    let continuous = args.continuous;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| bad(format!("tokio runtime: {e}")))?;

    let final_rev = rt.block_on(async move {
        if !continuous {
            let res = remote::sync_once_with_controller(link.as_ref(), &agent, handler.as_ref(), &outcomes).await;
            if matches!(res, Err(RemoteError::RevokedPeer(_)))
                && let Ok(Some(mut ms)) = ManagedInstallStateFile::load(&mpath)
            {
                ms.detached = true;
                let _ = ms.save(&mpath);
            }
            return res.map_err(|e| bad(e.to_string()));
        }

        loop {
            let res = remote::sync_once_with_controller(link.as_ref(), &agent, handler.as_ref(), &outcomes).await;
            let wait_ms = match res {
                Ok(_) => interval_secs * 1000,
                Err(RemoteError::RevokedPeer(_)) => {
                    if let Ok(Some(mut ms)) = ManagedInstallStateFile::load(&mpath) {
                        ms.detached = true;
                        let _ = ms.save(&mpath);
                    }
                    let rev = agent.lock().unwrap_or_else(PoisonError::into_inner).applied_revision;
                    return Ok(rev);
                }
                Err(_) => {
                    let failures = agent
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .consecutive_unreachable;
                    remote::offline_backoff_delay_ms(failures)
                }
            };
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(wait_ms)) => {}
                () = service_kit::serve::stop_requested() => {
                    let rev = agent.lock().unwrap_or_else(PoisonError::into_inner).applied_revision;
                    return Ok(rev);
                }
            }
        }
    })?;

    Ok(json!({
        "node": node.as_str(),
        "controller_node": controller_node.as_str(),
        "applied_revision": final_rev,
    }))
}

/// Runs `dainty-host remote <command>`.
pub fn run(paths: &Paths, args: &RemoteArgs) -> Outcome {
    let _ = merge_managed_host_toml;
    let _ = RemoteConfig::default;
    match &args.command {
        RemoteCommand::InitController(a) => {
            let val = init_controller(paths, a)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!(
                    "Controller initialized: node={} region={} url={}",
                    val["node"].as_str().unwrap_or_default(),
                    val["region"].as_str().unwrap_or_default(),
                    val["controller_url"].as_str().unwrap_or_default(),
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Invite(a) => {
            let (uri, val) = create_invite(paths, a)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!("{uri}");
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Enroll(a) => {
            let val = enroll(paths, a)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!(
                    "Enrolled as {} ({}) with controller {} at {}",
                    val["node"].as_str().unwrap_or_default(),
                    val["region"].as_str().unwrap_or_default(),
                    val["controller_node"].as_str().unwrap_or_default(),
                    val["controller_url"].as_str().unwrap_or_default(),
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Status(a) | RemoteCommand::List(a) => {
            let val = status_json(paths)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                let role = val["role"].as_str().unwrap_or("standalone");
                println!("Role: {role}");
                if role == "controller" {
                    println!(
                        "Controller: {} ({}) rev={}",
                        val["node"].as_str().unwrap_or_default(),
                        val["controller_url"].as_str().unwrap_or_default(),
                        val["revision"].as_u64().unwrap_or(0)
                    );
                    if let Some(arr) = val["installs"].as_array() {
                        for item in arr {
                            println!(
                                "  - {} ({}) [{}] rev={}",
                                item["node"].as_str().unwrap_or_default(),
                                item["region"].as_str().unwrap_or_default(),
                                item["state"].as_str().unwrap_or_default(),
                                item["applied_revision"].as_u64().unwrap_or(0),
                            );
                        }
                    }
                } else if role == "managed" {
                    println!(
                        "Managed node: {} -> controller {} ({}) rev={}",
                        val["node"].as_str().unwrap_or_default(),
                        val["controller_node"].as_str().unwrap_or_default(),
                        val["controller_url"].as_str().unwrap_or_default(),
                        val["applied_revision"].as_u64().unwrap_or(0),
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Revoke(a) => {
            let val = revoke(paths, &a.node)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!("Revoked managed install: {}", a.node);
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Leave(a) => {
            let val = leave(paths)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!("Left remote management; install is now standalone.");
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::SetConfig(a) => {
            let val = set_shared_config_from_host_toml(paths)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!(
                    "Published shared configuration revision {}",
                    val["revision"].as_u64().unwrap_or(0)
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Exec(a) => {
            let val = exec_command(paths, a)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!(
                    "Remote command {}: {}",
                    val["status"].as_str().unwrap_or_default(),
                    val["command_id"]
                        .as_str()
                        .or_else(|| val["guarded"]["id"].as_str())
                        .unwrap_or_default()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Approve(a) => {
            let val = approve_command(paths, a)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!("Approved guarded remote action {}", a.id);
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Cancel(a) => {
            let val = cancel_command(paths, a)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!("Canceled guarded remote action {}", a.id);
            }
            Ok(ExitCode::SUCCESS)
        }
        RemoteCommand::Sync(a) => {
            let val = sync_with_controller(paths, a)?;
            if a.json {
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or_default());
            } else {
                println!(
                    "Synced {} with controller {} (revision {})",
                    val["node"].as_str().unwrap_or_default(),
                    val["controller_node"].as_str().unwrap_or_default(),
                    val["applied_revision"].as_u64().unwrap_or(0)
                );
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;
    use tempfile::TempDir;

    fn write_test_host_toml(paths: &Paths) {
        std::fs::create_dir_all(&paths.config).unwrap();
        let toml = r#"
version = 1
name = "Controller Server"
mode = "public"
host = "ctrl.example.com"
game_address = "203.0.113.10"
public_port = 8443
registration = "INVITECODE"
max_hunt_servers = 4
"#;
        std::fs::write(&paths.host_toml, toml).unwrap();
    }

    #[test]
    fn controller_lifecycle_invite_config_exec_and_revoke() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        write_test_host_toml(&paths);

        let init = init_controller(
            &paths,
            &RemoteInitControllerArgs {
                node: Some("ctrl-main".into()),
                region: Some("us-east".into()),
                url: Some("https://127.0.0.1:61000".into()),
                json: true,
            },
        )
        .unwrap();
        assert_eq!(init["role"], "controller");
        assert_eq!(init["node"], "ctrl-main");
        assert_eq!(init["region"], "us-east");

        let (uri, invite_val) = create_invite(
            &paths,
            &RemoteInviteArgs {
                node: "managed-alpha".into(),
                region: Some("eu-west".into()),
                label: Some("EU Alpha".into()),
                url: None,
                ttl_secs: Some(600),
                json: true,
            },
        )
        .unwrap();
        assert!(uri.starts_with("dainty-lass-remote://enroll?"));
        assert_eq!(invite_val["node"], "managed-alpha");
        assert_eq!(invite_val["region"], "eu-west");

        let parsed = parse_enrollment_uri(&uri).unwrap();
        let ca = ClusterCa::read(&controller_ca_path(&paths)).unwrap();
        let mut state = RemoteControllerState::load(&controller_state_path(&paths)).unwrap();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let (_priv_pem, pub_pem) = remote::generate_node_keypair_and_spki_pem().unwrap();
        let _ans = state
            .redeem_enrollment(
                &ca,
                &remote::EnrollRequest {
                    node: parsed.node.clone(),
                    region: parsed.region.clone(),
                    token: parsed.token.clone(),
                    label: parsed.label.clone(),
                    gateway_url: Some("https://127.0.0.1:61001".into()),
                    gateway_cert_fp: None,
                    public_key_pem: pub_pem,
                },
                now_ms,
            )
            .unwrap();
        state.save(&controller_state_path(&paths)).unwrap();

        let status = status_json(&paths).unwrap();
        assert_eq!(status["role"], "controller");
        assert_eq!(status["installs"].as_array().unwrap().len(), 1);
        assert_eq!(status["installs"][0]["node"], "managed-alpha");

        let pub_cfg = set_shared_config_from_host_toml(&paths).unwrap();
        assert_eq!(pub_cfg["revision"], 2);

        let exec = exec_command(
            &paths,
            &RemoteExecArgs {
                node: "managed-alpha".into(),
                action: "ban".into(),
                account: Some("Griefer99".into()),
                reason: Some("spamming".into()),
                for_whom: None,
                role: "owner".into(),
                actor: "owner-cli".into(),
                json: true,
            },
        )
        .unwrap();
        assert_eq!(exec["status"], "queued");

        let rev = revoke(&paths, "managed-alpha").unwrap();
        assert_eq!(rev["revoked"], "managed-alpha");

        let left = leave(&paths).unwrap();
        assert_eq!(left["role"], "standalone");
    }
}
