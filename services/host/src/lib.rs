//! The host kit, `reforged-host` (MIGRATION_PLAN.md RD3, section 4.4): the Node kit in
//! `legacy/ReforgedHost` as one Rust binary, with the same commands and options and the same files
//! (the `.env` files; `host.json` became `host.toml`, read from a Node kit's host.json until the
//! first write), so an existing host moves over with `update`.
//!
//! RH1 is the skeleton: the command line ([`cli`]), the layout ([`paths`]), the env files
//! ([`envfile`]) and the settings with the service env files built from it ([`hostconfig`]) are
//! real. Each command is a module in [`commands`]; the other modules follow the Node kit's files and
//! name the row that fills them. Shared helpers one row writes and another calls are declared here
//! with their signatures, so the wave 2 rows (RH2-RH5) each fill only their own files.

pub mod cli;
pub mod commands;
pub mod envfile;
pub mod epicinstall;
pub mod error;
pub mod guarded;
pub mod hostconfig;
pub mod hosttoml;
pub mod paths;

// RH2
pub mod run;
pub mod services;
// RH3
pub mod doctor;
pub mod invite;
pub mod mongo;
// RH4
pub mod captures;
pub mod gamedata;
pub mod gamepack;
pub mod preset;
pub mod progression;
pub mod publish;
// Dauntless Revived's 1.4.4 servers (RD5 reversed); without the feature, refusals with the same signatures
#[cfg(feature = "legacy")]
pub mod legacy;
#[cfg(not(feature = "legacy"))]
#[path = "legacy/off.rs"]
pub mod legacy;
// RH5
pub mod linux;
pub mod selfupdate;
pub mod wine;
// The server as one process (RD26): the controller's modules, the console and the dashboard
pub mod repl;
pub mod serve;
pub mod tui;

use std::process::ExitCode;

use clap::Parser as _;
pub use error::HostError;

/// What a command returns: its exit code, or the error `main` prints.
pub type Outcome = Result<ExitCode, HostError>;

/// Runs the parsed command line: finds the checkout, then the command. A service run alone needs
/// no checkout (a container holds the binary and the workspace only).
pub fn run(cli: &cli::Cli) -> Outcome {
    use cli::Command;
    if let Command::Gateway { args }
    | Command::Allowlist { args }
    | Command::Content { args }
    | Command::Admin { args }
    | Command::Backend { args }
    | Command::Director { args } = &cli.command
    {
        return Ok(service_main(
            cli.command.name(),
            &with_admin_env(cli.command.name(), args),
        ));
    }
    // The CA and node certificates are made on the owner's machine, in or out of a server's folder
    if let Command::Network(args) = &cli.command {
        let paths = paths::locate_repo_root().ok().map(paths::Paths::new);
        return commands::network::run(args, paths.as_ref());
    }
    let root = paths::locate_repo_root()?;
    commands::dispatch(&cli.command, &paths::Paths::new(root))
}

/// `admin key ...` and `admin guarded ...` without `--env-file` read this checkout admin.env (DH2, DH6); a
/// container with no checkout names the file itself.
fn with_admin_env(name: &str, args: &[String]) -> Vec<String> {
    let mut args = args.to_vec();
    let named = args.iter().any(|a| a == "--env-file" || a.starts_with("--env-file="));
    if name == "admin"
        && args.first().is_some_and(|a| a == "key" || a == "guarded")
        && !named
        && let Ok(root) = paths::locate_repo_root()
    {
        let file = paths::Paths::new(root).config.join("admin.env");
        if file.is_file() {
            args.extend(["--env-file".to_owned(), file.display().to_string()]);
        }
    }
    args
}

/// Which services this binary runs on its own (`reforged-host <service>`).
pub const BUILT_IN_SERVICES: &[&str] = &[
    #[cfg(feature = "gateway")]
    "gateway",
    #[cfg(feature = "allowlist")]
    "allowlist",
    #[cfg(feature = "content")]
    "content",
    #[cfg(feature = "admin")]
    "admin",
    #[cfg(feature = "isles-1-14-7")]
    "backend",
    #[cfg(feature = "isles-1-14-7")]
    "director",
];

#[cfg(feature = "isles-1-14-7")]
struct StandaloneLink {
    home: module_api::network::StandaloneHome,
}

#[cfg(feature = "isles-1-14-7")]
impl module_api::HostLink for StandaloneLink {
    fn call<'a>(
        &'a self,
        module: &'a module_api::ModuleId,
        call: module_api::HostCall,
    ) -> module_api::BoxFuture<'a, Result<module_api::HostAnswer, module_api::ModuleError>> {
        Box::pin(async move {
            match call {
                module_api::HostCall::Emit(_) => Ok(module_api::HostAnswer::Done),
                module_api::HostCall::Log(record) => {
                    println!("{module} | {:?}: {}", record.level, record.message);
                    Ok(module_api::HostAnswer::Done)
                }
                module_api::HostCall::Home(call) => Ok(module_api::HostAnswer::Home(
                    module_api::network::dispatch_home(&self.home, call).await,
                )),
            }
        })
    }
}

#[cfg(feature = "isles-1-14-7")]
fn parse_service_env_file(
    name: &str,
    args: &[String],
    paths: Option<&paths::Paths>,
) -> Result<Option<std::path::PathBuf>, String> {
    let mut env_file: Option<std::path::PathBuf> = None;
    let mut idx = 0;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg == "serve" && idx == 0 {
            idx += 1;
            continue;
        }
        if arg == "--env-file" {
            idx += 1;
            let Some(val) = args.get(idx) else {
                return Err("--env-file requires a path".into());
            };
            env_file = Some(std::path::PathBuf::from(val));
            idx += 1;
        } else if let Some(val) = arg.strip_prefix("--env-file=") {
            env_file = Some(std::path::PathBuf::from(val));
            idx += 1;
        } else {
            return Err(format!("unexpected argument: {arg}"));
        }
    }
    if env_file.is_none() {
        if let Some(p) = paths {
            let candidate = p.config.join(format!("{name}.env"));
            if candidate.is_file() {
                env_file = Some(candidate);
            }
        }
        if env_file.is_none() {
            let dot_env = std::path::PathBuf::from(".env");
            if dot_env.is_file() {
                env_file = Some(dot_env);
            }
        }
    }
    Ok(env_file)
}

#[cfg(feature = "isles-1-14-7")]
fn isles_service_main(name: &'static str, args: &[String]) -> ExitCode {
    use module_api::GameModule as _;

    let repo_paths = paths::locate_repo_root().ok().map(paths::Paths::new);
    let env_file = match parse_service_env_file(name, args, repo_paths.as_ref()) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("Configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    let paths = env_file
        .as_deref()
        .and_then(|file| {
            let config_dir = file.parent()?;
            if config_dir.file_name().is_some_and(|n| n == "config") {
                let ws_dir = config_dir.parent()?;
                if ws_dir.file_name().is_some_and(|n| n == ".workspace") {
                    return ws_dir.parent().map(paths::Paths::new);
                }
            }
            None
        })
        .or(repo_paths);
    let host_config = paths
        .as_ref()
        .and_then(|p| hostconfig::load_host_config(p).ok().flatten());
    if name == "backend"
        && let Some(p) = &paths
    {
        if let Some(hc) = &host_config {
            if let Err(error) = commands::update::check_unmigrated_mongo_data(p, hc) {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        } else if !p.config.join(commands::update::MONGO_MIGRATED_MARKER).exists()
            && commands::update::has_managed_mongo_data(p)
        {
            eprintln!(
                "Unmigrated MongoDB data found in {}. Run `reforged-host setup` to migrate existing player data before starting the server, or pass `reforged-host setup --fresh-start` (or `fresh_start = true` under `[database]` in host.toml) to start with an empty database without modifying MongoDB.",
                p.mongo_data.display()
            );
            return ExitCode::FAILURE;
        }
    }
    let identity =
        host_config
            .as_ref()
            .and_then(|c| c.network.as_ref())
            .map_or_else(module_api::NodeIdentity::standalone, |n| module_api::NodeIdentity {
                node: n.node.clone(),
                region: n.region.clone(),
            });

    let mut spec = isles_1_14_7::spec();
    spec.services = match name {
        "backend" => vec![std::sync::Arc::new(isles_core::module::BackendService)],
        "director" => vec![std::sync::Arc::new(isles_director::DirectorService::new(
            isles_1_14_7::DIRECTOR,
        ))],
        _ => Vec::new(),
    };
    let module = isles_core::module::IslesModule::new(spec);
    let manifest = module.manifest();
    let mut registrar = module_api::Registrar::new(manifest.clone());
    if let Err(error) = module.register(&mut registrar) {
        eprintln!("Configuration: {error}");
        return ExitCode::FAILURE;
    }
    let registration = match registrar.finish() {
        Ok(reg) => reg,
        Err(error) => {
            eprintln!("Configuration: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut config = module_api::ModuleConfig::defaults(&manifest, &registration);
    if let Some(file) = &env_file {
        config.sections.entry("backend".to_owned()).or_default().insert(
            "env_file".to_owned(),
            serde_json::Value::String(file.display().to_string()),
        );
        if let Ok(entries) = hostconfig::read_env_file(file) {
            if let Some(port) = entries.get("PORT").and_then(|v| v.parse::<u16>().ok()) {
                config
                    .ports
                    .insert(name.to_owned(), module_api::PortRange::single(port));
            }
            if let (Some(first), Some(last)) = (
                entries.get("PORT_RANGE_BEGIN").and_then(|v| v.parse::<u16>().ok()),
                entries.get("PORT_RANGE_END").and_then(|v| v.parse::<u16>().ok()),
            ) {
                config
                    .ports
                    .insert("game".to_owned(), module_api::PortRange { first, last });
            }
        }
    }
    if let Some(p) = &paths {
        let game_data = config.sections.entry("game_data".to_owned()).or_default();
        if p.backend_game_data.is_dir() {
            game_data.insert(
                "backend_dir".to_owned(),
                serde_json::Value::String(p.backend_game_data.display().to_string()),
            );
        }
        if p.director_game_data.is_dir() {
            game_data.insert(
                "director_dir".to_owned(),
                serde_json::Value::String(p.director_game_data.display().to_string()),
            );
        }
    }
    if let Some(switches) = host_config.as_ref().and_then(|c| c.switches.as_ref()) {
        let section = config.sections.entry("switches".to_owned()).or_default();
        for (key, value) in switches.module_switches() {
            section.insert(key.to_owned(), value);
        }
    }
    let ctx = module_api::ModuleContext::new(
        manifest.id,
        identity.clone(),
        std::sync::Arc::new(StandaloneLink {
            home: module_api::network::StandaloneHome { identity },
        }),
    );
    let _ = service_kit::telemetry::init_table_stdout_subscriber(tracing::level_filters::LevelFilter::INFO);
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        if let Err(error) = module.start(ctx.clone(), config).await {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        service_kit::serve::stop_requested().await;
        if let Err(error) = module.stop(ctx).await {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    })
}

/// One service with the command line its own binary had: `name` is its argv[0].
pub fn service_main(name: &str, args: &[String]) -> ExitCode {
    let argv = std::iter::once(name.to_owned()).chain(args.iter().cloned());
    match name {
        #[cfg(feature = "gateway")]
        "gateway" => gateway::cli::main_from(argv),
        #[cfg(feature = "allowlist")]
        "allowlist" => gateway::allowlist::cli::main_from(argv),
        #[cfg(feature = "content")]
        "content" => content::cli::main_from(argv),
        #[cfg(feature = "admin")]
        "admin" => admin::cli::main_from(argv),
        #[cfg(feature = "isles-1-14-7")]
        "backend" => {
            drop(argv);
            isles_service_main("backend", args)
        }
        #[cfg(feature = "isles-1-14-7")]
        "director" => {
            drop(argv);
            isles_service_main("director", args)
        }
        _ => {
            drop(argv);
            eprintln!(
                "\nError: this binary was built without the {name} service (built in: {})",
                BUILT_IN_SERVICES.join(", ")
            );
            ExitCode::FAILURE
        }
    }
}

/// `reforged-host`: the command line as the Node kit had it (help when no command is given).
pub fn host_main() -> ExitCode {
    let cli = cli::Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            // As the Node kit's fail().
            eprint!("\nError: {error}\n");
            ExitCode::FAILURE
        }
    }
}

/// Entry point of the `server-console*` binaries: `reforged-host`'s commands, with `serve` when no
/// command is given (a bare console serves).
pub fn console_main() -> ExitCode {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.len() == 1 || args.get(1).is_some_and(|a| a.to_string_lossy().starts_with('-')) {
        args.insert(1, "serve".into());
    }
    let cli = cli::Cli::parse_from(args);
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("\nError: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(test, feature = "isles-1-14-7"))]
mod tests {
    use super::*;

    #[test]
    fn built_in_services_include_backend_and_director() {
        assert!(BUILT_IN_SERVICES.contains(&"backend"));
        assert!(BUILT_IN_SERVICES.contains(&"director"));
        let cli = cli::Cli::try_parse_from([
            "reforged-host",
            "backend",
            "serve",
            "--env-file",
            "/opt/reforged/.workspace/config/backend.env",
        ])
        .unwrap();
        assert_eq!(cli.command.name(), "backend");
        let cli = cli::Cli::try_parse_from([
            "reforged-host",
            "director",
            "serve",
            "--env-file=/opt/reforged/.workspace/config/director.env",
        ])
        .unwrap();
        assert_eq!(cli.command.name(), "director");
        assert_eq!(
            parse_service_env_file(
                "backend",
                &[
                    "serve".into(),
                    "--env-file".into(),
                    "/opt/reforged/.workspace/config/backend.env".into()
                ],
                None
            )
            .unwrap()
            .as_deref(),
            Some(std::path::Path::new("/opt/reforged/.workspace/config/backend.env"))
        );
        assert!(parse_service_env_file("backend", &["--env-file".into()], None).is_err());
    }
}
