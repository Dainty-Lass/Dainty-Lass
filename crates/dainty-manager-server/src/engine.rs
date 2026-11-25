// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The in-process host kit (spec-tauri-shell §5): what `dainty-host` does with a workspace, called as a library.
//! Errors go back as their text; the launcher kit decides what the page shows.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use dainty_host::HostError;
use dainty_host::cli::{
    GamedataArgs, InviteArgs, MultipliersArgs, RemoteEnrollArgs, RemoteExecArgs, RemoteInitControllerArgs,
    RemoteInviteArgs, SetupArgs,
};
use dainty_host::commands::setup::SetupEnv;
use dainty_host::commands::setup::layout::Layout;
use dainty_host::hostconfig::{
    DatabaseSettings, HostConfig, Platform, Registration, ServerMode, build_env_files, load_host_config,
    load_or_create_secrets, new_secrets, save_host_config, service_ports, write_env_files,
};
use dainty_host::paths::Paths;
use dainty_host::preset::PresetApplied;
use dainty_launcher_kit::bridge::checks::{
    HistoryFilter, RemoteEnrollRequest, RemoteExecRequest, RemoteInitRequest, RemoteInviteRequest, ServerConfigForm,
};
use dainty_launcher_kit::bridge::types::{
    DoctorCheckView, DoctorLevel, DoctorReportView, HistoryOutcome, HostServerMode, ManagedInstallView, RemoteExecView,
    RemoteInviteView, RemoteStatusView, RevertApplyOutcome, RevertPlanOutcome, ServerConfigView, ServerInviteEntry,
};
use dainty_launcher_kit::platform::server_engine::{Lines, ServerEngine};
use serde_json::Value;

use crate::Boxed;

/// Setup's stand-in for a Tailscale address when none is found (T-3, as the restricted build did).
const TAILSCALE_FALLBACK: &str = "100.64.0.1";
/// The program setup's firewall rules name when this executable's path is unknown.
const MANAGER_PROGRAM: &str = "dainty-manager";

/// Flags read from a kit command's arguments, in order: each with its value, if it took one.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flags(Vec<(String, Option<String>)>);

impl Flags {
    /// Reads `args` (the subcommand already taken off): a `valued` flag takes the next item when one follows,
    /// an `optional` one only when the next is not a flag, a `switch` none; unknown items are skipped.
    pub fn read(args: &[String], switches: &[&str], valued: &[&str], optional: &[&str]) -> Self {
        let mut out = Vec::new();
        let mut rest = args.iter().peekable();
        while let Some(arg) = rest.next() {
            let name = arg.as_str();
            if valued.contains(&name) {
                out.push((arg.clone(), rest.next().cloned()));
            } else if optional.contains(&name) {
                let value = rest.next_if(|next| !next.starts_with("--")).cloned();
                out.push((arg.clone(), value));
            } else if switches.contains(&name) {
                out.push((arg.clone(), None));
            }
        }
        Self(out)
    }

    /// Whether `name` was given.
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(flag, _)| flag == name)
    }

    /// The last value `name` was given.
    pub fn value(&self, name: &str) -> Option<String> {
        self.0
            .iter()
            .rev()
            .find(|(flag, _)| flag == name)
            .and_then(|(_, v)| v.clone())
    }
}

fn text(error: HostError) -> String {
    error.to_string()
}

fn succeeded(result: Result<ExitCode, HostError>) -> Result<bool, String> {
    result.map(|code| code == ExitCode::SUCCESS).map_err(text)
}

/// Setup's environment inside the manager: the services are in this process, so nothing is installed.
pub struct InProcessSetupEnv;

impl SetupEnv for InProcessSetupEnv {
    fn platform(&self) -> Platform {
        Platform::current()
    }

    fn is_root(&self) -> Result<bool, HostError> {
        dainty_host::linux::is_root()
    }

    fn tailscale_ipv4(&self) -> Result<Option<String>, HostError> {
        Ok(Some(
            dainty_host::doctor::tailscale_ipv4().unwrap_or_else(|| TAILSCALE_FALLBACK.to_owned()),
        ))
    }

    fn find_on_path(&self, name: &str) -> Result<Option<String>, HostError> {
        dainty_host::wine::find_on_path(&[name])
    }

    fn source_commit(&self, repo: &Path) -> Result<String, HostError> {
        Ok(dainty_host::selfupdate::read_manifest(repo)
            .ok()
            .flatten()
            .map(|manifest| manifest.short_commit().to_owned())
            .unwrap_or_default())
    }

    fn apply_preset(&self, paths: &Paths) -> Result<Option<PresetApplied>, HostError> {
        dainty_host::preset::apply_preset(paths, false)
    }

    fn install_services(&self, _: &Layout, _: &Path, _: &mut dyn FnMut(&str)) -> Result<(), HostError> {
        Ok(())
    }

    fn make_certificate(&self, _: &Layout, hosts: &[String], tls: &Path, _: &Path) -> Result<(), HostError> {
        let mut args = vec!["make-cert".to_owned()];
        for host in hosts {
            args.extend(["--host".to_owned(), host.clone()]);
        }
        args.extend([
            "--out".to_owned(),
            tls.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ]);
        if dainty_host::service_main("gateway", &args) == ExitCode::SUCCESS {
            Ok(())
        } else {
            Err(HostError::message("could not make the gateway certificate"))
        }
    }

    fn gateway_program(&self) -> String {
        std::env::current_exe()
            .map(|exe| exe.to_string_lossy().into_owned())
            .unwrap_or_else(|_| MANAGER_PROGRAM.to_owned())
    }

    fn require_checkout_sources(&self) -> bool {
        false
    }
}

/// The host kit in this process. Stateless: each call reads the workspace afresh.
pub struct Engine;

impl Engine {
    fn kit(paths: &Paths, args: &[String], lines: Lines<'_>) -> Result<bool, String> {
        let Some((command, rest)) = args.split_first() else {
            return Err("unsupported command: ".to_owned());
        };
        match command.as_str() {
            "setup" => {
                let flags = Flags::read(rest, &["--fresh-start"], &["--mode", "--name", "--host", "--game"], &[]);
                let args = SetupArgs {
                    mode: flags.value("--mode"),
                    name: flags.value("--name"),
                    host: flags.value("--host"),
                    game: flags.value("--game"),
                    fresh_start: flags.has("--fresh-start"),
                    ..SetupArgs::default()
                };
                succeeded(dainty_host::commands::setup::run_with(
                    paths,
                    &args,
                    &InProcessSetupEnv,
                    lines,
                ))
            }
            "invite" => {
                let flags = Flags::read(rest, &["--json"], &["--for"], &[]);
                let args = InviteArgs {
                    for_whom: flags.value("--for"),
                    uses: "1".to_owned(),
                    unlimited: false,
                    list: false,
                    json: flags.has("--json"),
                    game: None,
                };
                let port = dainty_host::commands::invite::backend_port(paths);
                succeeded(dainty_host::commands::invite::run_with(paths, &args, port, lines))
            }
            "multipliers" => {
                let flags = Flags::read(rest, &["--off"], &["--xp", "--rewards"], &[]);
                let args = MultipliersArgs {
                    xp: flags.value("--xp"),
                    rewards: flags.value("--rewards"),
                    off: flags.has("--off"),
                    ..MultipliersArgs::default()
                };
                // No checkout to name a commit of
                let commit = |_: &Path| Ok(String::new());
                succeeded(dainty_host::commands::multipliers::run_with(
                    paths,
                    &args,
                    Platform::current(),
                    &commit,
                    lines,
                ))
            }
            "gamedata" => {
                let flags = Flags::read(rest, &["--examples"], &["--import"], &["--export"]);
                let args = GamedataArgs {
                    examples: flags.has("--examples"),
                    export: flags
                        .has("--export")
                        .then(|| flags.value("--export").unwrap_or_default()),
                    import: flags.value("--import"),
                    ..GamedataArgs::default()
                };
                if !paths.backend_game_data.exists() && !paths.settings_toml.exists() {
                    // A fresh manager has no data to start from; a failure here shows in the command's own
                    let _ = dainty_host::gamedata::seed_embedded_examples(paths);
                }
                succeeded(dainty_host::commands::gamedata::run_with(paths, &args, lines))
            }
            "publish-runtime" => succeeded(dainty_host::commands::publish_runtime::run_with(
                paths,
                &Default::default(),
                lines,
            )),
            // The manager updates itself, not a checkout
            "update" => {
                lines(r#"{"ok":true,"behind":0,"head":"embedded"}"#);
                Ok(true)
            }
            other => Err(format!("unsupported command: {other}")),
        }
    }
}

fn config_view(paths: &Paths) -> Result<ServerConfigView, String> {
    let config_file = paths
        .settings_toml
        .is_file()
        .then(|| paths.settings_toml.to_string_lossy().into_owned());
    let Some(config) = load_host_config(paths).map_err(text)? else {
        return Ok(ServerConfigView {
            configured: false,
            name: String::new(),
            mode: HostServerMode::Private,
            public_host: None,
            registration: "INVITECODE".to_owned(),
            database: "sqlite".to_owned(),
            max_hubs: 1,
            max_hunts: 3,
            fresh_start: false,
            config_file,
        });
    };
    let database = config.database.clone().unwrap_or_default();
    let public = config.mode == ServerMode::Public;
    Ok(ServerConfigView {
        configured: true,
        name: config.name.clone(),
        mode: if public {
            HostServerMode::Public
        } else {
            HostServerMode::Private
        },
        public_host: public.then(|| config.host.clone()),
        registration: config.registration.as_str().to_owned(),
        database: database.engine.unwrap_or_else(|| "sqlite".to_owned()),
        max_hubs: 1,
        max_hunts: config.max_hunt_servers,
        fresh_start: database.fresh_start == Some(true),
        config_file,
    })
}

fn configured(paths: &Paths) -> Result<HostConfig, String> {
    load_host_config(paths)
        .map_err(text)?
        .ok_or_else(|| "not_configured".to_owned())
}

fn get_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn get_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn get_u32(value: &Value, key: &str) -> u32 {
    let n = match value.get(key) {
        // A list is counted (`pending_enrollments` may be the invites themselves)
        Some(Value::Array(items)) => items.len() as u64,
        Some(other) => other.as_u64().unwrap_or(0),
        None => 0,
    };
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn get_bool(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// `remote status --json` as the page reads it (§5.7).
pub fn remote_status_view(status: &Value) -> RemoteStatusView {
    let installs = status
        .get("installs")
        .and_then(Value::as_array)
        .map(|installs| {
            installs
                .iter()
                .map(|install| ManagedInstallView {
                    node: get_str(install, "node").unwrap_or_default(),
                    region: get_str(install, "region").unwrap_or_default(),
                    label: get_str(install, "label"),
                    state: get_str(install, "state").unwrap_or_else(|| "offline".to_owned()),
                    enrolled_at_ms: get_u64(install, "enrolled_at_ms").unwrap_or(0),
                    last_seen_ms: get_u64(install, "last_seen_ms").unwrap_or(0),
                    revoked: get_bool(install, "revoked"),
                    applied_revision: get_u64(install, "applied_revision").unwrap_or(0),
                    players_online: get_u32(install, "players_online"),
                    active_hunts: get_u32(install, "active_hunts"),
                    version: get_str(install, "version"),
                    queued_commands: get_u32(install, "queued_commands"),
                })
                .collect()
        })
        .unwrap_or_default();
    RemoteStatusView {
        role: get_str(status, "role").unwrap_or_else(|| "standalone".to_owned()),
        node: get_str(status, "node"),
        region: get_str(status, "region"),
        controller_url: get_str(status, "controller_url"),
        controller_cert_fp: get_str(status, "controller_cert_fp"),
        controller_node: get_str(status, "controller_node"),
        revision: get_u64(status, "revision")
            .or_else(|| get_u64(status, "applied_revision"))
            .unwrap_or(0),
        pending_enrollments: get_u32(status, "pending_enrollments"),
        installs,
        detached: get_bool(status, "detached"),
    }
}

fn remote_status(paths: &Paths) -> Result<RemoteStatusView, String> {
    dainty_host::commands::remote::status_json(paths)
        .map(|status| remote_status_view(&status))
        .map_err(text)
}

impl ServerEngine for Engine {
    fn run_kit(&self, workspace: &Path, args: &[String], lines: Lines<'_>) -> Result<bool, String> {
        dainty_host::ensure_hooks();
        let paths = Paths::new(workspace);
        // Setup, the example data and the generators read the version data
        crate::version_data::install_shipped(&paths)?;
        Self::kit(&paths, args, lines)
    }

    fn doctor(&self, workspace: &Path) -> Result<DoctorReportView, String> {
        use dainty_host::doctor::{DoctorOptions, Level, run_doctor};
        let checks = run_doctor(&Paths::new(workspace), &DoctorOptions { hash_exe: false }).map_err(text)?;
        let mut report = DoctorReportView {
            checks: Vec::new(),
            warnings: 0,
            problems: 0,
        };
        for check in checks {
            let level = match check.level {
                Level::Ok | Level::Info => DoctorLevel::Ok,
                Level::Warn => {
                    report.warnings += 1;
                    DoctorLevel::Warning
                }
                Level::Fail => {
                    report.problems += 1;
                    DoctorLevel::Problem
                }
            };
            let detail = match check.fix {
                Some(fix) => format!("{} \u{2014} {fix}", check.text),
                None => check.text,
            };
            report.checks.push(DoctorCheckView {
                level,
                label: check.area.to_owned(),
                detail,
            });
        }
        Ok(report)
    }

    fn backup(&self, workspace: &Path, lines: Lines<'_>) -> Result<bool, String> {
        dainty_host::ensure_hooks();
        let paths = Paths::new(workspace);
        succeeded(dainty_host::commands::backup::run_with(
            &paths,
            &dainty_host::commands::backup::System,
            lines,
        ))
    }

    fn config_get(&self, workspace: &Path) -> Result<ServerConfigView, String> {
        config_view(&Paths::new(workspace))
    }

    fn config_save(&self, workspace: &Path, form: &ServerConfigForm) -> Result<ServerConfigView, String> {
        let paths = Paths::new(workspace);
        let mut config = configured(&paths)?;
        let registration = match form.registration {
            "INVITE" => "INVITECODE",
            other => other,
        };
        config.registration = Registration::parse(registration).ok_or_else(|| "registration".to_owned())?;
        config.max_hunt_servers = form.max_hunts.clamp(1, 128);
        let database = config.database.get_or_insert_with(DatabaseSettings::default);
        database.engine = Some(form.database.to_owned());
        database.fresh_start = form.fresh_start.then_some(true);
        save_host_config(&paths, &config).map_err(text)?;
        // Best effort: the next start rewrites the services' files anyway
        if let Ok((secrets, _)) = load_or_create_secrets(&paths.config, new_secrets) {
            let files = build_env_files(&config, &secrets, &paths, "", Platform::current());
            let _ = write_env_files(&paths, &files);
        }
        if config.remote.as_ref().and_then(|r| r.role.as_deref()) == Some("controller") {
            let _ = dainty_host::commands::remote::set_shared_config_from_settings_toml(&paths);
        }
        config_view(&paths)
    }

    fn invite_list(&self, workspace: &Path) -> Result<Vec<ServerInviteEntry>, String> {
        use dainty_host::invite::{certificate_fingerprint, invite_for, list_invite_codes};
        let paths = Paths::new(workspace);
        let config = configured(&paths)?;
        let (secrets, _) = load_or_create_secrets(&paths.config, new_secrets).map_err(text)?;
        let fingerprint = match config.mode {
            ServerMode::Public => Some(certificate_fingerprint(&paths.cert_file).map_err(text)?),
            ServerMode::Private => None,
        };
        let codes = list_invite_codes(service_ports(&config).backend, &secrets.admin_key).map_err(text)?;
        Ok(codes
            .iter()
            .filter_map(|item| {
                let code = get_str(item, "code")?;
                Some(ServerInviteEntry {
                    link: invite_for(&config, &code, fingerprint.as_deref()).unwrap_or_else(|_| code.clone()),
                    for_name: get_str(item, "note").filter(|note| !note.is_empty()),
                    uses_remaining: get_u32(item, "usesRemaining"),
                    infinite_uses: get_bool(item, "infiniteUses"),
                    created_at: get_str(item, "createdAt").unwrap_or_default(),
                    code,
                })
            })
            .collect())
    }

    fn remote_status(&self, workspace: &Path) -> Result<RemoteStatusView, String> {
        remote_status(&Paths::new(workspace))
    }

    fn remote_init(&self, workspace: &Path, request: &RemoteInitRequest) -> Result<RemoteStatusView, String> {
        let paths = Paths::new(workspace);
        let args = RemoteInitControllerArgs {
            node: request.node.clone(),
            region: request.region.clone(),
            url: request.url.clone(),
            json: true,
        };
        dainty_host::commands::remote::init_controller(&paths, &args).map_err(text)?;
        remote_status(&paths)
    }

    fn remote_invite(&self, workspace: &Path, request: &RemoteInviteRequest) -> Result<RemoteInviteView, String> {
        let args = RemoteInviteArgs {
            node: request.node.clone(),
            region: request.region.clone(),
            label: request.label.clone(),
            ttl_secs: None,
            url: request.url.clone(),
            json: true,
        };
        let (_, out) = dainty_host::commands::remote::create_invite(&Paths::new(workspace), &args).map_err(text)?;
        Ok(RemoteInviteView {
            uri: get_str(&out, "uri").unwrap_or_default(),
            node: get_str(&out, "node").unwrap_or_default(),
            region: get_str(&out, "region").unwrap_or_default(),
            label: get_str(&out, "label"),
            controller_url: get_str(&out, "controller_url").unwrap_or_default(),
            controller_fp: get_str(&out, "controller_fp").unwrap_or_default(),
            expires_at_ms: get_u64(&out, "expires_at_ms").unwrap_or(0),
        })
    }

    fn remote_enroll(&self, workspace: &Path, request: &RemoteEnrollRequest) -> Result<RemoteStatusView, String> {
        let paths = Paths::new(workspace);
        let args = RemoteEnrollArgs {
            uri: request.uri.clone(),
            label: request.label.clone(),
            gateway_url: None,
            json: true,
        };
        dainty_host::commands::remote::enroll(&paths, &args).map_err(text)?;
        remote_status(&paths)
    }

    fn remote_revoke(&self, workspace: &Path, node: &str) -> Result<RemoteStatusView, String> {
        let paths = Paths::new(workspace);
        dainty_host::commands::remote::revoke(&paths, node).map_err(text)?;
        remote_status(&paths)
    }

    fn remote_leave(&self, workspace: &Path) -> Result<RemoteStatusView, String> {
        let paths = Paths::new(workspace);
        dainty_host::commands::remote::leave(&paths).map_err(text)?;
        remote_status(&paths)
    }

    fn remote_exec(&self, workspace: &Path, request: &RemoteExecRequest) -> Result<RemoteExecView, String> {
        let args = RemoteExecArgs {
            node: request.node.clone(),
            action: request.action.to_owned(),
            account: request.target.clone(),
            reason: request.reason.clone(),
            for_whom: request.target.clone(),
            role: "owner".to_owned(),
            actor: "manager-gui".to_owned(),
            json: true,
        };
        let out = dainty_host::commands::remote::exec_command(&Paths::new(workspace), &args).map_err(text)?;
        Ok(RemoteExecView {
            command_id: get_str(&out, "command_id").unwrap_or_default(),
            node: get_str(&out, "node").unwrap_or_else(|| request.node.clone()),
            action: get_str(&out, "action").unwrap_or_else(|| request.action.to_owned()),
            status: get_str(&out, "status").unwrap_or_default(),
        })
    }

    fn history_query(&self, workspace: PathBuf, filter: HistoryFilter) -> Boxed<HistoryOutcome> {
        Box::pin(async move { crate::history::query(&workspace, &filter).await.into() })
    }

    fn history_dry_run(&self, workspace: PathBuf, selection: HistoryFilter) -> Boxed<RevertPlanOutcome> {
        Box::pin(async move { crate::history::dry_run(&workspace, &selection).await.into() })
    }

    fn history_apply(&self, workspace: PathBuf, plan: String) -> Boxed<RevertApplyOutcome> {
        Box::pin(async move { crate::history::apply(&workspace, &plan).await.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn unknown_flags_are_skipped_and_values_taken() {
        let flags = Flags::read(
            &args(&["--name", "Isle", "--color", "blue", "--fresh-start", "--host", "a.b"]),
            &["--fresh-start"],
            &["--name", "--host"],
            &[],
        );
        assert_eq!(flags.value("--name").as_deref(), Some("Isle"));
        assert_eq!(flags.value("--host").as_deref(), Some("a.b"));
        assert!(flags.has("--fresh-start"));
        // `--color` is skipped, its value with it as a stray item
        assert!(!flags.has("--color"));
        assert!(!flags.has("blue"));
    }

    #[test]
    fn a_value_flag_at_the_end_takes_nothing() {
        let flags = Flags::read(&args(&["--json", "--for"]), &["--json"], &["--for"], &[]);
        assert!(flags.has("--for"));
        assert_eq!(flags.value("--for"), None);
        assert!(flags.has("--json"));
        // A value flag takes the next item, whatever it looks like
        let flags = Flags::read(&args(&["--for", "--json"]), &["--json"], &["--for"], &[]);
        assert_eq!(flags.value("--for").as_deref(), Some("--json"));
        assert!(!flags.has("--json"));
    }

    #[test]
    fn an_optional_value_is_taken_only_when_it_is_not_a_flag() {
        let read = |items: &[&str]| Flags::read(&args(items), &["--examples"], &["--import"], &["--export"]);
        assert_eq!(read(&["--export", "out"]).value("--export").as_deref(), Some("out"));
        let bare = read(&["--export", "--examples"]);
        assert!(bare.has("--export") && bare.value("--export").is_none() && bare.has("--examples"));
        let end = read(&["--export"]);
        assert!(end.has("--export") && end.value("--export").is_none());
    }

    #[test]
    fn update_is_the_managers_own_and_unknown_commands_are_refused() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let mut lines = Vec::new();
        let ran = Engine.run_kit(dir.path(), &args(&["update", "--yes"]), &mut |l| {
            lines.push(l.to_owned())
        });
        assert_eq!(ran, Ok(true));
        assert_eq!(lines, [r#"{"ok":true,"behind":0,"head":"embedded"}"#]);
        assert_eq!(
            Engine.run_kit(dir.path(), &args(&["start"]), &mut |_| {}),
            Err("unsupported command: start".to_owned())
        );
    }

    #[test]
    fn remote_status_takes_defaults_for_what_is_missing() {
        let view = remote_status_view(&serde_json::json!({
            "role": "managed",
            "applied_revision": 7,
            "installs": [{ "node": "n1", "players_online": 5_000_000_000_u64 }],
        }));
        assert_eq!((view.role.as_str(), view.revision), ("managed", 7));
        assert_eq!(view.installs[0].state, "offline");
        assert_eq!(view.installs[0].players_online, u32::MAX);
        assert_eq!(remote_status_view(&serde_json::json!({})).role, "standalone");
    }
}
