//! The manager's in-process host kit (B3, RM4, DH3b): what `controller/host.rs`'s server-GUI tail did with
//! restricted's `host` crate, behind launcher-kit's `ServerEngine`. launcher-kit keeps the jobs, the checks, the
//! logs and the page's events; this file only calls the host kit (spec §3.6: it links `host`).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::ExitCode;

use launcher_kit::bridge::checks::{
    HistoryFilter, RemoteEnrollRequest, RemoteExecRequest, RemoteInitRequest, RemoteInviteRequest, ServerConfigForm,
};
use launcher_kit::bridge::types::{
    DoctorCheckView, DoctorLevel, DoctorReportView, HistoryOutcome, HostServerMode, ManagedInstallView,
    MongoMigrationState, MongoMigrationView, RemoteExecView, RemoteInviteView, RemoteStatusView, RevertApplyOutcome,
    RevertPlanOutcome, ServerConfigView, ServerInviteEntry,
};
use launcher_kit::platform::server_engine::{Lines, ServerEngine};
use serde_json::Value;

type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The host kit linked into the manager.
pub struct Engine;

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn succeeded(code: ExitCode) -> bool {
    code == ExitCode::SUCCESS
}

/// `--flag value` and `--flag` pairs, as `dainty-host` takes them.
struct Args<'a> {
    list: &'a [String],
    at: usize,
}

impl<'a> Args<'a> {
    fn new(list: &'a [String]) -> Self {
        Self { list, at: 1 }
    }

    /// The next flag, and its value when the flag takes one and one follows.
    fn next(&mut self, takes_value: impl Fn(&str) -> bool) -> Option<(&'a str, Option<&'a str>)> {
        let flag = self.list.get(self.at)?.as_str();
        self.at += 1;
        if takes_value(flag)
            && let Some(value) = self.list.get(self.at)
        {
            self.at += 1;
            return Some((flag, Some(value.as_str())));
        }
        Some((flag, None))
    }
}

fn run_setup(paths: &host::paths::Paths, list: &[String], lines: Lines<'_>) -> Result<ExitCode, host::HostError> {
    let mut args = host::cli::SetupArgs::default();
    let mut flags = Args::new(list);
    while let Some((flag, value)) = flags.next(|f| matches!(f, "--mode" | "--name" | "--host" | "--game")) {
        let value = value.map(str::to_owned);
        match flag {
            "--mode" if value.is_some() => args.mode = value,
            "--name" if value.is_some() => args.name = value,
            "--host" if value.is_some() => args.host = value,
            "--game" if value.is_some() => args.game = value,
            "--fresh-start" => args.fresh_start = true,
            _ => {}
        }
    }
    host::commands::setup::run_with(paths, &args, &InProcessSetupEnv, lines)
}

fn run_invite(paths: &host::paths::Paths, list: &[String], lines: Lines<'_>) -> Result<ExitCode, host::HostError> {
    let mut args = host::cli::InviteArgs {
        uses: "1".to_owned(),
        ..Default::default()
    };
    let mut flags = Args::new(list);
    while let Some((flag, value)) = flags.next(|f| f == "--for") {
        match (flag, value) {
            ("--json", _) => args.json = true,
            ("--for", Some(name)) => args.for_whom = Some(name.to_owned()),
            _ => {}
        }
    }
    host::commands::invite::run_with(paths, &args, host::paths::PORTS.backend, lines)
}

fn run_multipliers(paths: &host::paths::Paths, list: &[String], lines: Lines<'_>) -> Result<ExitCode, host::HostError> {
    let mut args = host::cli::MultipliersArgs::default();
    let mut flags = Args::new(list);
    while let Some((flag, value)) = flags.next(|f| matches!(f, "--xp" | "--rewards")) {
        match (flag, value) {
            ("--off", _) => args.off = true,
            ("--xp", Some(v)) => args.xp = Some(v.to_owned()),
            ("--rewards", Some(v)) => args.rewards = Some(v.to_owned()),
            _ => {}
        }
    }
    // The embedded server has no checkout to commit to
    let commit = |_repo: &Path| -> Result<String, host::HostError> { Ok(String::new()) };
    host::commands::multipliers::run_with(paths, &args, host::hostconfig::Platform::current(), &commit, lines)
}

fn run_gamedata(paths: &host::paths::Paths, list: &[String], lines: Lines<'_>) -> Result<ExitCode, host::HostError> {
    let mut args = host::cli::GamedataArgs::default();
    let mut at = 1;
    while let Some(flag) = list.get(at) {
        at += 1;
        match flag.as_str() {
            "--examples" => args.examples = true,
            // `--export` takes an optional folder
            "--export" => match list.get(at).filter(|next| !next.starts_with("--")) {
                Some(folder) => {
                    args.export = Some(folder.clone());
                    at += 1;
                }
                None => args.export = Some(String::new()),
            },
            "--import" => {
                if let Some(pack) = list.get(at) {
                    args.import = Some(pack.clone());
                    at += 1;
                }
            }
            _ => {}
        }
    }
    if !paths.backend_dir.exists() && !paths.host_toml.exists() {
        let _ = host::gamedata::seed_embedded_examples(paths);
    }
    host::commands::gamedata::run_with(paths, &args, lines)
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn string(value: &Value, key: &str) -> String {
    str_field(value, key).unwrap_or_default()
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn small(value: &Value, key: &str) -> u32 {
    u32::try_from(number(value, key)).unwrap_or(u32::MAX)
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn read_remote_status_view(ws: &Path) -> Result<RemoteStatusView, String> {
    let paths = host::paths::Paths::new(ws);
    let val = host::commands::remote::status_json(&paths).map_err(text)?;
    let revision = val
        .get("revision")
        .or_else(|| val.get("applied_revision"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let installs = val
        .get("installs")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|item| ManagedInstallView {
                    node: string(item, "node"),
                    region: string(item, "region"),
                    label: str_field(item, "label"),
                    state: str_field(item, "state").unwrap_or_else(|| "offline".to_owned()),
                    enrolled_at_ms: number(item, "enrolled_at_ms"),
                    last_seen_ms: number(item, "last_seen_ms"),
                    revoked: flag(item, "revoked"),
                    applied_revision: number(item, "applied_revision"),
                    players_online: small(item, "players_online"),
                    active_hunts: small(item, "active_hunts"),
                    version: str_field(item, "version"),
                    queued_commands: small(item, "queued_commands"),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(RemoteStatusView {
        role: str_field(&val, "role").unwrap_or_else(|| "standalone".to_owned()),
        node: str_field(&val, "node"),
        region: str_field(&val, "region"),
        controller_url: str_field(&val, "controller_url"),
        controller_cert_fp: str_field(&val, "controller_cert_fp"),
        controller_node: str_field(&val, "controller_node"),
        revision,
        pending_enrollments: small(&val, "pending_enrollments"),
        installs,
        detached: flag(&val, "detached"),
    })
}

fn read_server_config_view(ws: &Path) -> Result<ServerConfigView, String> {
    use host::commands::update::MongoMigrationStatus;

    let paths = host::paths::Paths::new(ws);
    let config = host::hostconfig::load_host_config(&paths).map_err(text)?;
    let (state, source) = match host::commands::update::inspect_mongo_migration(&paths) {
        MongoMigrationStatus::None => (MongoMigrationState::None, None),
        MongoMigrationStatus::Migrated => (MongoMigrationState::Migrated, None),
        MongoMigrationStatus::Skipped => (MongoMigrationState::Skipped, None),
        MongoMigrationStatus::Unmigrated { source } => (MongoMigrationState::Unmigrated, Some(source)),
    };
    let mongo_migration = MongoMigrationView {
        state,
        source,
        detail: None,
    };
    let config_file = paths
        .host_toml
        .is_file()
        .then(|| paths.host_toml.to_string_lossy().into_owned());
    let Some(c) = config else {
        return Ok(ServerConfigView {
            configured: false,
            name: String::new(),
            mode: HostServerMode::Private,
            public_host: None,
            registration: "INVITECODE".into(),
            database: "sqlite".into(),
            max_hubs: 1,
            max_hunts: 3,
            fresh_start: false,
            mongo_migration,
            config_file,
        });
    };
    let mode = match c.mode {
        host::hostconfig::ServerMode::Private => HostServerMode::Private,
        host::hostconfig::ServerMode::Public => HostServerMode::Public,
    };
    let public_host = (c.mode == host::hostconfig::ServerMode::Public).then(|| c.host.clone());
    let database = c
        .database
        .as_ref()
        .and_then(|d| d.engine.clone())
        .unwrap_or_else(|| "sqlite".into());
    let fresh_start = c.database.as_ref().and_then(|d| d.fresh_start).unwrap_or(false)
        || matches!(mongo_migration.state, MongoMigrationState::Skipped);
    Ok(ServerConfigView {
        configured: true,
        name: c.name,
        mode,
        public_host,
        registration: c.registration.as_str().to_owned(),
        database,
        max_hubs: 1,
        max_hunts: c.max_hunt_servers,
        fresh_start,
        mongo_migration,
        config_file,
    })
}

impl ServerEngine for Engine {
    fn run_kit(&self, workspace: &Path, args: &[String], lines: Lines<'_>) -> Result<bool, String> {
        let paths = host::paths::Paths::new(workspace);
        let outcome = match args.first().map(String::as_str).unwrap_or_default() {
            "setup" => run_setup(&paths, args, lines),
            "invite" => run_invite(&paths, args, lines),
            "multipliers" => run_multipliers(&paths, args, lines),
            "gamedata" => run_gamedata(&paths, args, lines),
            "publish-runtime" => {
                host::commands::publish_runtime::run_with(&paths, &host::cli::PublishRuntimeArgs::default(), lines)
            }
            // The manager updates itself, not a checkout
            "update" => {
                lines(r#"{"ok":true,"behind":0,"head":"embedded"}"#);
                Ok(ExitCode::SUCCESS)
            }
            other => Err(host::HostError::message(format!("unsupported command: {other}"))),
        };
        outcome.map(succeeded).map_err(text)
    }

    fn doctor(&self, workspace: &Path) -> Result<DoctorReportView, String> {
        let paths = host::paths::Paths::new(workspace);
        let checks =
            host::doctor::run_doctor(&paths, &host::doctor::DoctorOptions { hash_exe: false }).map_err(text)?;
        let (mut warnings, mut problems) = (0u32, 0u32);
        let checks = checks
            .into_iter()
            .map(|c| {
                let level = match c.level {
                    host::doctor::Level::Ok | host::doctor::Level::Info => DoctorLevel::Ok,
                    host::doctor::Level::Warn => {
                        warnings += 1;
                        DoctorLevel::Warning
                    }
                    host::doctor::Level::Fail => {
                        problems += 1;
                        DoctorLevel::Problem
                    }
                };
                let detail = match c.fix {
                    Some(fix) => format!("{} — {}", c.text, fix),
                    None => c.text,
                };
                DoctorCheckView {
                    level,
                    label: c.area.to_owned(),
                    detail,
                }
            })
            .collect();
        Ok(DoctorReportView {
            checks,
            warnings,
            problems,
        })
    }

    fn backup(&self, workspace: &Path, lines: Lines<'_>) -> Result<bool, String> {
        let paths = host::paths::Paths::new(workspace);
        host::commands::backup::run_with(&paths, &host::commands::backup::System, lines)
            .map(succeeded)
            .map_err(text)
    }

    fn migrate_mongo(&self, workspace: &Path, lines: Lines<'_>) -> Result<(), String> {
        let paths = host::paths::Paths::new(workspace);
        host::hostconfig::load_host_config(&paths)
            .map_err(text)?
            .ok_or_else(|| "not_configured".to_owned())?;
        // As a host would by hand: verify writes the marker start waits for, and only with no difference
        let run = |command| host::cli::DbArgs { command };
        host::commands::db::run_with(
            &paths,
            &run(host::cli::DbCommand::Migrate(host::cli::DbMigrateArgs::default())),
            lines,
        )
        .map_err(text)?;
        host::commands::db::run_with(
            &paths,
            &run(host::cli::DbCommand::Verify(host::cli::DbVerifyArgs::default())),
            lines,
        )
        .map(drop)
        .map_err(text)
    }

    fn config_get(&self, workspace: &Path) -> Result<ServerConfigView, String> {
        read_server_config_view(workspace)
    }

    fn config_save(&self, workspace: &Path, form: &ServerConfigForm) -> Result<ServerConfigView, String> {
        let paths = host::paths::Paths::new(workspace);
        let mut config = host::hostconfig::load_host_config(&paths)
            .map_err(text)?
            .ok_or_else(|| "not_configured".to_owned())?;
        let registration = if form.registration == "INVITE" {
            "INVITECODE"
        } else {
            form.registration
        };
        config.registration =
            host::hostconfig::Registration::parse(registration).ok_or_else(|| "registration".to_owned())?;
        config.max_hunt_servers = form.max_hunts.clamp(1, 128);
        let db = config.database.get_or_insert_with(Default::default);
        db.engine = Some(form.database.to_owned());
        db.fresh_start = form.fresh_start.then_some(true);
        host::hostconfig::save_host_config(&paths, &config).map_err(text)?;
        if form.fresh_start {
            host::commands::update::confirm_fresh_start(&paths).map_err(text)?;
        } else {
            let marker = paths.config.join(host::commands::update::MONGO_MIGRATED_MARKER);
            if host::commands::update::is_fresh_start_marker(&marker) {
                let _ = std::fs::remove_file(&marker);
            }
        }
        if let Ok((secrets, _)) =
            host::hostconfig::load_or_create_secrets(&paths.config, host::commands::setup::secrets::new_secrets)
        {
            let envs =
                host::hostconfig::build_env_files(&config, &secrets, &paths, "", host::hostconfig::Platform::current());
            let _ = host::hostconfig::write_env_files(&paths, &envs);
        }
        if config
            .remote
            .as_ref()
            .is_some_and(|r| r.role.as_deref() == Some("controller"))
        {
            let _ = host::commands::remote::set_shared_config_from_host_toml(&paths);
        }
        read_server_config_view(workspace)
    }

    fn confirm_fresh_start(&self, workspace: &Path) -> Result<ServerConfigView, String> {
        host::commands::update::confirm_fresh_start(&host::paths::Paths::new(workspace)).map_err(text)?;
        read_server_config_view(workspace)
    }

    fn invite_list(&self, workspace: &Path) -> Result<Vec<ServerInviteEntry>, String> {
        let paths = host::paths::Paths::new(workspace);
        let config = host::hostconfig::load_host_config(&paths)
            .map_err(text)?
            .ok_or_else(|| "not_configured".to_owned())?;
        let (secrets, _) =
            host::hostconfig::load_or_create_secrets(&paths.config, host::commands::setup::secrets::new_secrets)
                .map_err(text)?;
        let fp = match config.mode {
            host::hostconfig::ServerMode::Public => {
                Some(host::invite::certificate_fingerprint(&paths.cert_file).map_err(text)?)
            }
            host::hostconfig::ServerMode::Private => None,
        };
        let items = host::invite::list_invite_codes(host::paths::PORTS.backend, &secrets.admin_key).map_err(text)?;
        Ok(items
            .iter()
            .filter_map(|item| {
                let code = item.get("code").and_then(Value::as_str)?;
                Some(ServerInviteEntry {
                    code: code.to_owned(),
                    link: host::invite::invite_for(&config, code, fp.as_deref()).unwrap_or_else(|_| code.to_owned()),
                    for_name: str_field(item, "note").filter(|note| !note.is_empty()),
                    uses_remaining: small(item, "usesRemaining"),
                    infinite_uses: flag(item, "infiniteUses"),
                    created_at: string(item, "createdAt"),
                })
            })
            .collect())
    }

    fn remote_status(&self, workspace: &Path) -> Result<RemoteStatusView, String> {
        read_remote_status_view(workspace)
    }

    fn remote_init(&self, workspace: &Path, request: &RemoteInitRequest) -> Result<RemoteStatusView, String> {
        let args = host::cli::RemoteInitControllerArgs {
            node: request.node.clone(),
            region: request.region.clone(),
            url: request.url.clone(),
            json: true,
        };
        host::commands::remote::init_controller(&host::paths::Paths::new(workspace), &args).map_err(text)?;
        read_remote_status_view(workspace)
    }

    fn remote_invite(&self, workspace: &Path, request: &RemoteInviteRequest) -> Result<RemoteInviteView, String> {
        let args = host::cli::RemoteInviteArgs {
            node: request.node.clone(),
            region: request.region.clone(),
            label: request.label.clone(),
            url: request.url.clone(),
            ttl_secs: None,
            json: true,
        };
        let (_uri, val) =
            host::commands::remote::create_invite(&host::paths::Paths::new(workspace), &args).map_err(text)?;
        Ok(RemoteInviteView {
            uri: string(&val, "uri"),
            node: string(&val, "node"),
            region: string(&val, "region"),
            label: str_field(&val, "label"),
            controller_url: string(&val, "controller_url"),
            controller_fp: string(&val, "controller_fp"),
            expires_at_ms: number(&val, "expires_at_ms"),
        })
    }

    fn remote_enroll(&self, workspace: &Path, request: &RemoteEnrollRequest) -> Result<RemoteStatusView, String> {
        let args = host::cli::RemoteEnrollArgs {
            uri: request.uri.clone(),
            label: request.label.clone(),
            gateway_url: None,
            json: true,
        };
        host::commands::remote::enroll(&host::paths::Paths::new(workspace), &args).map_err(text)?;
        read_remote_status_view(workspace)
    }

    fn remote_revoke(&self, workspace: &Path, node: &str) -> Result<RemoteStatusView, String> {
        host::commands::remote::revoke(&host::paths::Paths::new(workspace), node).map_err(text)?;
        read_remote_status_view(workspace)
    }

    fn remote_leave(&self, workspace: &Path) -> Result<RemoteStatusView, String> {
        host::commands::remote::leave(&host::paths::Paths::new(workspace)).map_err(text)?;
        read_remote_status_view(workspace)
    }

    fn remote_exec(&self, workspace: &Path, request: &RemoteExecRequest) -> Result<RemoteExecView, String> {
        let args = host::cli::RemoteExecArgs {
            node: request.node.clone(),
            action: request.action.to_owned(),
            account: request.target.clone(),
            reason: request.reason.clone(),
            for_whom: request.target.clone(),
            role: "owner".to_owned(),
            actor: "manager-gui".to_owned(),
            json: true,
        };
        let val = host::commands::remote::exec_command(&host::paths::Paths::new(workspace), &args).map_err(text)?;
        Ok(RemoteExecView {
            command_id: string(&val, "command_id"),
            node: string(&val, "node"),
            action: string(&val, "action"),
            status: string(&val, "status"),
        })
    }

    fn history_query(&self, workspace: PathBuf, filter: HistoryFilter) -> Boxed<HistoryOutcome> {
        Box::pin(async move { crate::server_history::query(&workspace, filter).await })
    }

    fn history_dry_run(&self, workspace: PathBuf, selection: HistoryFilter) -> Boxed<RevertPlanOutcome> {
        Box::pin(async move { crate::server_history::dry_run(&workspace, selection).await })
    }

    fn history_apply(&self, workspace: PathBuf, plan: String) -> Boxed<RevertApplyOutcome> {
        Box::pin(async move { crate::server_history::apply(&workspace, &plan).await })
    }
}

struct InProcessSetupEnv;

impl host::commands::setup::SetupEnv for InProcessSetupEnv {
    fn platform(&self) -> host::hostconfig::Platform {
        host::hostconfig::Platform::current()
    }

    fn is_root(&self) -> Result<bool, host::HostError> {
        host::linux::is_root()
    }

    fn tailscale_ipv4(&self) -> Result<Option<String>, host::HostError> {
        Ok(host::doctor::tailscale_ipv4().or_else(|| Some("100.64.0.1".to_owned())))
    }

    fn find_mongod(&self) -> Result<Option<String>, host::HostError> {
        Ok(None)
    }

    fn find_on_path(&self, name: &str) -> Result<Option<String>, host::HostError> {
        host::wine::find_on_path(&[name])
    }

    fn source_commit(&self, repo: &Path) -> Result<String, host::HostError> {
        if let Some(release) = host::selfupdate::read_manifest(repo)? {
            return Ok(release.short_commit().to_owned());
        }
        Ok(String::new())
    }

    fn apply_preset(&self, paths: &host::paths::Paths) -> Result<Option<host::preset::PresetApplied>, host::HostError> {
        host::preset::apply_preset(paths, false)
    }

    fn install_services(
        &self,
        _layout: &host::commands::setup::paradox::Layout,
        _log_file: &Path,
        _out: &mut dyn FnMut(&str),
    ) -> Result<(), host::HostError> {
        Ok(())
    }

    fn make_certificate(
        &self,
        _layout: &host::commands::setup::paradox::Layout,
        hosts: &[String],
        tls: &Path,
        _log_file: &Path,
    ) -> Result<(), host::HostError> {
        let mut args = vec!["make-cert".to_owned()];
        for host in hosts {
            args.extend(["--host".to_owned(), host.clone()]);
        }
        args.extend([
            "--out".to_owned(),
            tls.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ]);
        if !succeeded(host::service_main("gateway", &args)) {
            return Err(host::HostError::message("could not make the gateway certificate"));
        }
        Ok(())
    }

    fn gateway_program(&self) -> String {
        std::env::current_exe().map_or_else(|_| "dainty-manager".to_owned(), |p| p.display().to_string())
    }

    fn require_checkout_sources(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn flags_take_their_values_and_skip_what_they_do_not_know() {
        let args = list(&["invite", "--json", "--for", "Ana", "--odd"]);
        let mut flags = Args::new(&args);
        let mut seen = Vec::new();
        while let Some(pair) = flags.next(|f| f == "--for") {
            seen.push(pair);
        }
        assert_eq!(seen, [("--json", None), ("--for", Some("Ana")), ("--odd", None)]);

        // A value flag at the end has nothing to take
        let args = list(&["setup", "--name"]);
        let mut flags = Args::new(&args);
        assert_eq!(flags.next(|f| f == "--name"), Some(("--name", None)));
        assert_eq!(flags.next(|_| true), None);
    }
}
