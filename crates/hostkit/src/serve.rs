// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `serve` and `start`: the server as one process (`docs/app-structure-plan.md`, RD26). The host kit
//! composes the controller's modules from host.toml and the env files `setup` writes: the Rust
//! services built into this binary run in-process ([`controller::modules`]); legacy Node processes
//! run as supervised processes when built with `paradox`.
//!
//! `start` keeps the host kit's contract (RD23): no console of its own, `REFORGED_CONTROL=stdin`,
//! the `REFORGED_SERVER_READY` line, the backend on port 61000. `serve` is the same server with the
//! REPL, or the dashboard with `--tui`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use controller::{
    Module, ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions, ServerController, ServiceLibrary, modules,
};
use service_kit::env::Env;
use tokio::sync::watch;

use crate::Outcome;
use crate::commands::start::control::{Control, SERVER_READY_LINE, controlled_by_stdin};
use crate::commands::start::{self as start_cmd, say};
use crate::error::HostError;
use crate::hostconfig::{self, HostConfig, Platform, ServerMode};
use crate::paths::Paths;
use crate::services::{Printer, ServiceSpec, Supervisor, service_environment};

/// How `serve` runs.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// `--config <file>`: host.toml, .yaml, .yml or .json; the workspace's host.toml otherwise.
    pub config_file: Option<PathBuf>,
    /// The REPL (or the TUI) on this terminal. `start`, and a run the launcher controls through
    /// stdin, have neither.
    pub interactive: bool,
    pub tui: bool,
    pub log_filter: String,
}

impl ServeOptions {
    /// `reforged-host start`: no console.
    pub fn start() -> Self {
        Self {
            config_file: None,
            interactive: false,
            tui: false,
            log_filter: "info".into(),
        }
    }
}

/// The console's `name           | line`, as the stdout layer prints the in-process services.
pub fn table_printer() -> Printer {
    Arc::new(|name: &str, line: &str| println!("{name:<14} | {}", crate::repl::plain(line)))
}

/// The dashboard's: a process's lines into its log pane as that service's, never printed over it.
fn ring_printer() -> Printer {
    Arc::new(|name: &str, line: &str| tracing::info!(target: "host", service = name, "{}", crate::repl::plain(line)))
}

/// The env file as the service would see it as a process: the system's variables, then the file.
pub fn module_env(paths: &Paths, name: &str) -> Result<Env, HostError> {
    let from_file = hostconfig::read_env_file(&start_cmd::env_file(paths, name))?;
    let pairs = service_environment(std::env::vars(), from_file.iter(), &[]);
    Ok(Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))))
}

/// A supervised process as a module: its own [`Supervisor`] on its own thread (a process tree is
/// not `Send`), ready when it answers, restarted by the supervisor when it dies, stopped with its tree.
pub fn process_module(spec: ServiceSpec, ready_timeout: Duration, printer: Printer, log_dir: PathBuf) -> Module {
    let name = spec.name.clone();
    let spec = Arc::new(spec);
    Module::new(
        name,
        Arc::new(move |mut ctx| {
            let (spec, printer, log_dir) = (spec.clone(), printer.clone(), log_dir.clone());
            Box::pin(async move {
                ctx.status().report_starting("starting");
                let stop = Arc::new(AtomicBool::new(false));
                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
                let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
                let worker_stop = stop.clone();
                let worker_spec = (*spec).clone();
                std::thread::Builder::new()
                    .name(format!("module {}", spec.name))
                    .spawn(move || {
                        let supervisor = Supervisor::new(log_dir, printer).with_cancel(worker_stop.clone());
                        let started = start_process(&supervisor, worker_spec, ready_timeout);
                        let ok = started.is_ok();
                        let _ = ready_tx.send(started);
                        while ok && !worker_stop.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        supervisor.stop_all();
                        let _ = done_tx.send(());
                    })
                    .map_err(|e| format!("could not start a thread: {e}"))?;
                match ready_rx.await {
                    Ok(Ok(())) => ctx
                        .status()
                        .report_ready(spec.ready.as_ref().map(|r| r.port()), "ready"),
                    Ok(Err(error)) => {
                        ctx.status().failed(error.clone());
                        return Err(error);
                    }
                    Err(_) => {
                        let error = format!("{} ended before it started", spec.name);
                        ctx.status().failed(error.clone());
                        return Err(error);
                    }
                }
                ctx.wait_stop().await;
                ctx.status().report_stopping("stopping");
                stop.store(true, Ordering::SeqCst);
                let _ = done_rx.await;
                ctx.status().report_stopped("stopped");
                Ok(())
            })
        }),
    )
    .ready_within(ready_timeout + Duration::from_secs(5))
}

/// Starts one process and waits for it, with the taken-port check `start` made (a taken port would
/// pass "something answers" while the service fails to listen).
pub fn start_process(supervisor: &Supervisor, spec: ServiceSpec, ready_timeout: Duration) -> Result<(), String> {
    if let Some(ready) = &spec.ready
        && start_cmd::port_in_use(ready).map_err(|e| e.to_string())?
    {
        return Err(format!(
            "port {} ({}) is already used by another program; stop that program (or a server still running from before) and start again",
            ready.port(),
            spec.name
        ));
    }
    let (name, port) = (spec.name.clone(), spec.ready.as_ref().map(|r| r.port()));
    match supervisor.start(spec, ready_timeout).map_err(|e| e.to_string())? {
        true => Ok(()),
        false => Err(format!(
            "{name} did not answer{} within {} s (see its log in {})",
            port.map(|p| format!(" on port {p}")).unwrap_or_default(),
            ready_timeout.as_secs(),
            supervisor.log_dir().display()
        )),
    }
}

/// MongoDB (under `paradox`) as a module: `mongod` started when the kit manages it, the replica set
/// checked and initiated either way.
#[cfg(feature = "paradox")]
fn mongod_module(paths: &Paths, config: &HostConfig, printer: Printer) -> Module {
    let (paths, config) = (paths.clone(), config.clone());
    Module::new(
        "mongod",
        Arc::new(move |mut ctx| {
            let (paths, config, printer) = (paths.clone(), config.clone(), printer.clone());
            Box::pin(async move {
                ctx.status().report_starting("starting");
                let stop = Arc::new(AtomicBool::new(false));
                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
                let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
                let worker_stop = stop.clone();
                std::thread::Builder::new()
                    .name("module mongod".into())
                    .spawn(move || {
                        let supervisor = Supervisor::new(&paths.logs, printer).with_cancel(worker_stop.clone());
                        let started = start_cmd::prepare_database(&paths, &config, &supervisor, &|| {
                            worker_stop.load(Ordering::SeqCst)
                        })
                        .map_err(|e| e.to_string())
                        .and_then(|up| {
                            up.then_some(())
                                .ok_or_else(|| "stopped while MongoDB was starting".to_string())
                        });
                        let ok = started.is_ok();
                        let _ = ready_tx.send(started);
                        while ok && !worker_stop.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        supervisor.stop_all();
                        let _ = done_tx.send(());
                    })
                    .map_err(|e| format!("could not start a thread: {e}"))?;
                match ready_rx.await {
                    Ok(Ok(())) => ctx
                        .status()
                        .report_ready(Some(crate::paths::PORTS.mongo), "replica set ready"),
                    Ok(Err(error)) => {
                        ctx.status().failed(error.clone());
                        return Err(error);
                    }
                    Err(_) => {
                        let error = "mongod ended before it started".to_string();
                        ctx.status().failed(error.clone());
                        return Err(error);
                    }
                }
                ctx.wait_stop().await;
                ctx.status().report_stopping("stopping");
                stop.store(true, Ordering::SeqCst);
                let _ = done_rx.await;
                ctx.status().report_stopped("stopped");
                Ok(())
            })
        }),
    )
    .ready_within(Duration::from_secs(90))
}

/// The services `[services.<name>]` loads as libraries (RD27), paths taken from the server folder.
pub fn service_libraries(paths: &Paths, config: &HostConfig) -> Vec<ServiceLibrary> {
    let Some(services) = &config.services else {
        return Vec::new();
    };
    services
        .iter()
        .map(|(name, entry)| {
            let path = PathBuf::from(&entry.library);
            let (name, serves): (&'static str, &'static [module_api::CoreCapability]) =
                match hostconfig::services::KNOWN.iter().find(|(known, _)| known == name) {
                    Some((known, serves)) => (known, serves),
                    // Made once per start, for a name only host.toml knows
                    None => (
                        Box::leak(name.clone().into_boxed_str()),
                        entry.serves.clone().unwrap_or_default().leak(),
                    ),
                };
            ServiceLibrary {
                name,
                path: if path.is_absolute() {
                    path
                } else {
                    paths.repo.join(path)
                },
                sha256: entry.sha256,
                serves,
            }
        })
        .collect()
}

/// Whether `[services.<name>]` loads `name` as a library, in place of the compiled-in one.
pub fn library_configured(config: &HostConfig, name: &str) -> bool {
    config.services.as_ref().is_some_and(|s| s.contains_key(name))
}

/// The env a library runs on: chat's is the gateway's file, as the compiled-in chat's is; a service without a
/// file of its own gets the system's variables only.
pub fn library_env(paths: &Paths, name: &str) -> Result<Env, HostError> {
    let file = if name == "chat" { "gateway" } else { name };
    if start_cmd::env_file(paths, file).is_file() {
        return module_env(paths, file);
    }
    let pairs = service_environment(std::env::vars(), std::iter::empty::<(&str, &str)>(), &[]);
    Ok(Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))))
}

/// A service library as a module of `controller`, on its env file (RD27). The gateway and chat take their own core
/// side (routes, UDP ranges and node links; chat's host calls), as their compiled-in builds do: a library of either
/// loaded as a plain service would start without them.
pub fn add_service_library(
    controller: &mut ServerController,
    paths: &Paths,
    config: &HostConfig,
    registry: &ModuleRegistry,
    library: ServiceLibrary,
) -> Result<(), HostError> {
    use controller::service_library::gateway::{AllowlistMode, GatewayOptions, gateway_library};
    let env = library_env(paths, library.name)?;
    match library.name {
        "gateway" => {
            let allowlist = if config.mode != ServerMode::Public {
                AllowlistMode::Off
            } else if start_cmd::env_file(paths, "allowlist").is_file() {
                let env = module_env(paths, "allowlist")?;
                AllowlistMode::InProcess {
                    env: env.vars().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
                }
            } else {
                // A helper of its own guards the game ports: a module's UDP ranges must lie inside them (DF4d)
                let (first, last) = hostconfig::game_ports(config);
                AllowlistMode::External {
                    base: module_api::PortRange { first, last },
                }
            };
            let options = GatewayOptions {
                allowlist,
                ws_port: None,
            };
            controller.add(gateway_library(library, env, Some(registry), options).0);
        }
        "chat" => {
            let network =
                config
                    .network
                    .as_ref()
                    .filter(|n| !n.nodes.is_empty())
                    .map(|n| service_plugin::chat::ChatNetwork {
                        leader: n.effective_leader(),
                        peers: n.nodes.iter().map(|p| (p.id.clone(), p.region.clone())).collect(),
                    });
            let setup = service_plugin::chat::ChatSetup {
                node: node_identity(config),
                network,
            };
            // Chat keeps its tables beside the node's database and never falls back to its working folder
            let mut pairs: Vec<(String, String)> = env
                .vars()
                .filter(|(key, _)| !key.starts_with("REFORGED_DB_"))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            pairs.extend(database_env(&hostconfig::node_database_config(config, paths)?));
            let env = Env::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            controller.add(controller::service_library::chat_library(library, env, registry, setup));
        }
        _ => {
            controller.add(controller::service_library(library, env, Some(registry)));
        }
    }
    Ok(())
}

/// The server's modules in start order: the database, the allowlist (its table closes the game
/// ports before any game server listens), the backend, the Director, the content server, the admin
/// API, the gateway last. A front service runs in this process once [`rust_ported`] says so and this
/// binary has it; until then its Node build runs as a supervised process. A service `[services]` lists
/// runs as a library in its place.
pub fn compose(
    paths: &Paths,
    config: &HostConfig,
    external_allowlist: bool,
    printer: Printer,
) -> Result<ServerController, HostError> {
    use crate::paths::PORTS;
    use crate::services::Ready;
    use start_cmd::binaries::{FrontService, Programs, rust_ported};

    let public = config.mode == ServerMode::Public;
    let node_runs = cfg!(feature = "paradox");
    if !node_runs {
        crate::commands::update::check_unmigrated_mongo_data(paths, config)?;
    }
    let games = vec![protocol::invite::reforged_version().to_string()];
    let mut controller = ServerController::new(config.mode.as_str(), games);
    // Made first, so the admin API can hold it; its modules are listed after the services
    let registry = new_registry(config, &controller);
    let programs = if node_runs {
        Programs::find(paths)?
    } else {
        Programs::find_rust(paths)
    };
    let in_process = |service: FrontService| rust_ported(service) && service.built_in();
    let front_process = |service: FrontService, ready: Ready| {
        let file = start_cmd::env_file(paths, service.bin_name());
        let launch = programs.launch(service, paths, &file);
        process_module(
            start_cmd::spec_from(service.bin_name(), launch, file, ready),
            Duration::from_secs(60),
            printer.clone(),
            paths.logs.clone(),
        )
    };

    #[cfg(feature = "paradox")]
    {
        controller.add(mongod_module(paths, config, printer.clone()));
    }

    let mut libraries = service_libraries(paths, config);
    let mut take_library = |name: &str| {
        libraries
            .iter()
            .position(|l| l.name == name)
            .map(|at| libraries.remove(at))
    };
    let gateway_library = take_library("gateway");
    let chat_library = take_library("chat");
    let content_library = take_library("content");
    let hooks = crate::hooks::get();

    let mut slot: Option<crate::hooks::AnySlot> = None;
    let mut guarded = false;
    // A gateway library holds the allowlist (it serves UdpPorts)
    if public && !external_allowlist && gateway_library.is_none() {
        if in_process(FrontService::Allowlist) {
            slot = crate::hooks::get().compose_in_process_allowlist(&mut controller, paths, &registry)?;
            guarded = slot.is_some();
        } else if node_runs {
            controller.add(front_process(
                FrontService::Allowlist,
                Ready::http(PORTS.allowlist, "/status"),
            ));
        }
    }

    #[cfg(feature = "paradox")]
    {
        let node = |name: &str, dir: &Path, script: &[&str], ready: Ready| ServiceSpec {
            name: name.into(),
            cwd: dir.to_path_buf(),
            command: programs.node.clone(),
            args: vec![script.iter().collect::<PathBuf>().display().to_string()],
            env_file: Some(start_cmd::env_file(paths, name)),
            extra_env: Vec::new(),
            ready: Some(ready),
        };
        controller.add(process_module(
            node(
                "backend",
                &paths.backend_dir,
                &["build", "server.js"],
                Ready::http(PORTS.backend, "/dauntless-status"),
            ),
            Duration::from_secs(120),
            printer.clone(),
            paths.logs.clone(),
        ));
        controller.add(
            process_module(
                node(
                    "director",
                    &paths.director_dir,
                    &["dist", "server.js"],
                    Ready::http(PORTS.director, "/api/instances"),
                ),
                Duration::from_secs(60),
                printer.clone(),
                paths.logs.clone(),
            )
            .optional(),
        );
    }

    if let Some(library) = content_library {
        hooks.compose_service_library(&mut controller, paths, config, &registry, library)?;
    } else if in_process(FrontService::Content) {
        hooks.compose_in_process_content(&mut controller, paths, &registry)?;
    } else if node_runs {
        controller.add(front_process(FrontService::Content, Ready::http(PORTS.content, "/content/v1/news")).optional());
    }

    // The admin API has no Node build; it runs when setup wrote its env file (an older setup did not)
    #[cfg(feature = "admin")]
    if start_cmd::env_file(paths, "admin").is_file() {
        controller
            .add(modules::admin(&module_env(paths, "admin")?, Some(registry.clone())).map_err(HostError::message)?);
    }
    for library in libraries {
        hooks.compose_service_library(&mut controller, paths, config, &registry, library)?;
    }

    let in_process_gw = in_process(FrontService::Gateway);
    hooks.compose_in_process_network_and_gateway(
        &mut controller,
        paths,
        config,
        &registry,
        slot,
        in_process_gw,
        public,
        guarded,
    )?;
    let gateway_runs = gateway_library.is_some();
    for library in [gateway_library, chat_library].into_iter().flatten() {
        hooks.compose_service_library(&mut controller, paths, config, &registry, library)?;
    }
    if !in_process_gw && node_runs && !gateway_runs {
        let ready = match config.mode {
            ServerMode::Public => Ready::tcp("127.0.0.1", hostconfig::gateway_port(config)),
            ServerMode::Private => Ready::tcp(&config.game_address, PORTS.private_gateway),
        };
        controller.add(front_process(FrontService::Gateway, ready));
    }
    crate::commands::remote::compose_remote_module(paths, config, &registry, &mut controller)?;

    list_modules(paths, config, &registry, hooks.game_modules())?;
    controller.set_registry(registry);
    Ok(controller)
}

/// `store`'s settings as the `REFORGED_DB_*` variables a service reads them from, as setup writes them.
fn database_env(database: &store::DatabaseConfig) -> Vec<(String, String)> {
    match database {
        store::DatabaseConfig::Sqlite { file } => vec![
            ("REFORGED_DB_ENGINE".into(), "sqlite".into()),
            ("REFORGED_DB_FILE".into(), file.to_string_lossy().into_owned()),
        ],
        store::DatabaseConfig::Postgres(pg) => {
            let mut out = vec![
                ("REFORGED_DB_ENGINE".into(), "postgres".into()),
                ("REFORGED_DB_HOST".into(), pg.host.clone()),
                ("REFORGED_DB_PORT".into(), pg.port.to_string()),
                ("REFORGED_DB_USER".into(), pg.user.clone()),
                ("REFORGED_DB_NAME".into(), pg.database.clone()),
                ("REFORGED_DB_PASSWORD".into(), pg.password.clone()),
            ];
            if pg.allow_remote {
                out.push(("REFORGED_DB_ALLOW_REMOTE".into(), "1".into()));
            }
            out
        }
    }
}

/// This node's identity: its network's, or its remote management's, or a standalone server's.
fn node_identity(config: &HostConfig) -> module_api::NodeIdentity {
    use module_api::NodeIdentity;
    if let Some(network) = &config.network {
        NodeIdentity {
            node: network.node.clone(),
            region: network.region.clone(),
        }
    } else if let Some(remote) = &config.remote
        && matches!(remote.role.as_deref(), Some("controller" | "managed"))
    {
        NodeIdentity {
            node: remote.effective_node(),
            region: remote.effective_region(),
        }
    } else {
        NodeIdentity::standalone()
    }
}

/// The module registry (DF3): the server's own ports and the versions its own stacks serve are taken, and
/// every child gets only the system's variables, as `start` gives its services.
pub fn new_registry(config: &HostConfig, controller: &ServerController) -> ModuleRegistry {
    use module_api::PortRange;
    use module_api::manifest::Transport;

    let identity = node_identity(config);
    let mut options = RegistryOptions::standalone();
    options.home = Arc::new(module_api::network::StandaloneHome {
        identity: identity.clone(),
    });
    options.node = identity;
    options.child.env = Some(service_environment(
        std::env::vars(),
        std::iter::empty::<(&str, &str)>(),
        &[],
    ));
    let mut tcp: Vec<u16> = hostconfig::RESERVED_PORTS.to_vec();
    tcp.push(hostconfig::gateway_port(config));
    if !cfg!(feature = "paradox") {
        tcp.retain(|port| *port != crate::paths::PORTS.backend && *port != crate::paths::PORTS.director);
    }
    let udp = if cfg!(feature = "paradox") {
        vec![hostconfig::game_ports(config)]
    } else {
        Vec::new()
    };
    options.reserved_ports = tcp
        .into_iter()
        .map(|port| (Transport::Tcp, PortRange::single(port)))
        .chain(
            udp.into_iter()
                .map(|(first, last)| (Transport::Udp, PortRange { first, last })),
        )
        .collect();
    // The Node backend, when it runs, holds 1.14.7 itself
    options.taken_versions = if cfg!(feature = "paradox") {
        let version = protocol::invite::reforged_version();
        let by = format!("this server's own {version} stack");
        vec![(version, by)]
    } else {
        Vec::new()
    };
    ModuleRegistry::new(options, controller.status_registry().clone())
}

/// The game modules and plugins `[modules]` lists: the compiled-in ones it names (all of them when it
/// names none), and the external ones with their SHA-256, paths taken from the server folder.
pub fn list_modules(
    paths: &Paths,
    config: &HostConfig,
    registry: &ModuleRegistry,
    compiled: Vec<controller::ModuleFactory>,
) -> Result<(), HostError> {
    let modules = config.modules.clone().unwrap_or_default();
    let explicit_compiled = modules.compiled.is_some();
    let mut settings = modules.settings.clone();
    let mut entry = |id: module_api::ModuleId, source: ModuleSource, autoload: bool| {
        let mut mod_settings = settings.remove(&id).unwrap_or_default();
        if id.as_str() == "isles-1-14-7" {
            let backend_env = start_cmd::env_file(paths, "backend");
            if backend_env.is_file() {
                mod_settings
                    .entry("backend".to_owned())
                    .or_default()
                    .entry("env_file".to_owned())
                    .or_insert_with(|| serde_json::Value::String(backend_env.display().to_string()));
            }
            if paths.backend_game_data.is_dir() || paths.director_game_data.is_dir() {
                let game_data = mod_settings.entry("game_data".to_owned()).or_default();
                if paths.backend_game_data.is_dir() {
                    game_data
                        .entry("backend_dir".to_owned())
                        .or_insert_with(|| serde_json::Value::String(paths.backend_game_data.display().to_string()));
                }
                if paths.director_game_data.is_dir() {
                    game_data
                        .entry("director_dir".to_owned())
                        .or_insert_with(|| serde_json::Value::String(paths.director_game_data.display().to_string()));
                }
            }
            if let Some(switches) = &config.switches {
                let section = mod_settings.entry("switches".to_owned()).or_default();
                for (key, value) in switches.module_switches() {
                    section.entry(key.to_owned()).or_insert(value);
                }
            }
        }
        registry
            .add_entry(ModuleEntry {
                id,
                source,
                autoload,
                settings: mod_settings,
            })
            .map_err(|e| HostError::message(format!("[modules]: {e}")))
    };
    let mut built: Vec<(module_api::ModuleId, controller::ModuleFactory)> = compiled
        .into_iter()
        .map(|factory| (factory().manifest().id, factory))
        .collect();
    if let Some(wanted) = &modules.compiled {
        for id in wanted {
            if !built.iter().any(|(built_id, _)| built_id == id) {
                say(&format!(
                    "- [modules] names {id}, which this build was not compiled with: skipped"
                ));
            }
        }
        built.retain(|(id, _)| wanted.contains(id));
    }
    for (id, factory) in built {
        let autoload = if explicit_compiled {
            true
        } else {
            match id.as_str() {
                "isles-1-14-7" => !cfg!(feature = "paradox"),
                _ => true,
            }
        };
        entry(id, ModuleSource::CompiledIn(factory), autoload)?;
    }
    for external in &modules.external {
        let path = PathBuf::from(&external.path);
        let path = if path.is_absolute() {
            path
        } else {
            paths.repo.join(path)
        };
        let source = ModuleSource::External(module_api::ExternalModule {
            path,
            sha256: external.sha256,
            args: external.args.clone(),
            expected: Some(external.id.clone()),
        });
        entry(external.id.clone(), source, external.autoload)?;
    }
    for id in settings.keys() {
        say(&format!(
            "- [modules.{id}] sets a module [modules] does not list: ignored"
        ));
    }
    Ok(())
}

/// What an empty build says instead of serving.
pub fn empty_build_message() -> String {
    format!(
        "This build has no service modules (built with: {}; available: {}). The controller runs and serves nothing: build with the default features, or --features {}.",
        if modules::BUILT_IN.is_empty() {
            "none".to_string()
        } else {
            modules::BUILT_IN.join(", ")
        },
        modules::ALL.join(", "),
        modules::ALL.join(",")
    )
}

/// The commit the env files name: the release manifest's, else git's, else nothing.
fn source_commit(repo: &Path) -> String {
    if let Ok(Some(release)) = crate::selfupdate::read_manifest(repo) {
        return release.short_commit().to_owned();
    }
    crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(repo))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// What `setup` writes and a server cannot run without, made here when missing so a server whose
/// host.toml was written by hand starts all the same: the secrets (kept once made), the services'
/// env files (all of them, when any of the four is missing) and, in public mode, the gateway's
/// certificate (never a second one: every invite's `fp` is its hash).
pub fn ensure_server_files(paths: &Paths, config: &HostConfig) -> Result<Vec<PathBuf>, HostError> {
    let (secrets, created) =
        hostconfig::load_or_create_secrets(&paths.config, crate::commands::setup::secrets::new_secrets)?;
    if created {
        say(&format!(
            "- secrets made in {} (kept from now on)",
            paths.config.join(hostconfig::SECRET_FILE).display()
        ));
    }
    let missing = |names: &[String]| names.iter().any(|name| !start_cmd::env_file(paths, name).is_file());
    let reborn_files = ["backend", "director", "content", "gateway"].map(str::to_owned);
    let mut written = Vec::new();
    if missing(&reborn_files) {
        for dir in [
            &paths.runtime_publish,
            &paths.branding,
            &paths.gameserver_logs,
            &paths.mongo_data,
        ] {
            std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
        }
        let files = hostconfig::build_env_files(
            config,
            &secrets,
            paths,
            &source_commit(&paths.repo),
            Platform::current(),
        );
        written = hostconfig::write_env_files(paths, &files)?;
        say(&format!("- env files written in {}", paths.config.display()));
    }
    if config.mode == ServerMode::Public && !paths.cert_file.exists() {
        crate::hooks::get().ensure_gateway_cert(paths, config)?;
    }
    crate::commands::remote::ensure_controller_files(paths, config)?;
    if let Some(rem) = &config.remote
        && matches!(rem.role.as_deref(), Some("controller" | "managed"))
    {
        let gw_env = start_cmd::env_file(paths, "gateway");
        let needs_node_env = !gw_env.is_file()
            || hostconfig::read_env_file(&gw_env)
                .map(|m| m.get("GATEWAY_NODE_ID").is_none())
                .unwrap_or(true);
        if needs_node_env {
            let files = hostconfig::build_env_files(
                config,
                &secrets,
                paths,
                &source_commit(&paths.repo),
                Platform::current(),
            );
            written = hostconfig::write_env_files(paths, &files)?;
        }
    }
    Ok(written)
}

/// The settings to run from: `--config <file>` in any of the three formats, else the workspace's.
fn load_config(paths: &Paths, config_file: Option<&Path>) -> Result<HostConfig, HostError> {
    match config_file {
        Some(file) => {
            if let Some(cfg) = hostconfig::read_host_config(file)? {
                return Ok(cfg);
            }
            if !cfg!(feature = "paradox")
                && !crate::commands::update::mongo_migrated_marker(paths).is_file()
                && crate::commands::update::has_unmigrated_mongo_dir(paths)
            {
                return Err(HostError::message(format!(
                    "unmigrated MongoDB data was found in {}: move it with `reforged-host db migrate` and `db verify` (or `dainty-legacy import reforged <checkout>`), or pass --fresh-start to setup to start on a fresh database",
                    paths.mongo_data.display()
                )));
            }
            Err(HostError::message(format!("{} does not exist", file.display())))
        }
        None => start_cmd::load_config(paths),
    }
}

fn external_allowlist(config: &HostConfig) -> bool {
    cfg!(target_os = "linux")
        && config.mode == ServerMode::Public
        && std::env::var("REFORGED_ALLOWLIST").is_ok_and(|v| v == "external")
}

/// The checks before anything starts, the files `setup` would have written, the modules composed.
fn prepare(
    paths: &Paths,
    config: &HostConfig,
    external: bool,
    printer: Printer,
) -> Result<ServerController, HostError> {
    crate::commands::update::check_unmigrated_mongo_data(paths, config)?;
    ensure_server_files(paths, config)?;
    start_cmd::preflight_with(paths, config, external)?;
    compose(paths, config, external, printer)
}

/// The fingerprint the snapshot shows, in public mode.
fn fingerprint(paths: &Paths, config: &HostConfig) -> Option<String> {
    (config.mode == ServerMode::Public)
        .then(|| crate::invite::certificate_fingerprint(&paths.cert_file).ok())
        .flatten()
}

/// `serve` and `start`.
pub fn run(paths: &Paths, options: &ServeOptions) -> Outcome {
    let config = load_config(paths, options.config_file.as_deref())?;
    let external = external_allowlist(&config);
    let controlled = controlled_by_stdin();
    let interactive = options.interactive && !controlled;
    let level = service_kit::telemetry::parse_level_filter(&options.log_filter)
        .unwrap_or(tracing::level_filters::LevelFilter::INFO);
    let (tui_ring, reload_handle) = if interactive && options.tui {
        let (ring, handle) =
            service_kit::telemetry::init_tui_subscriber(level, 2000, Some(paths.logs.join("server.log")));
        (Some(ring), Some(handle))
    } else {
        (None, Some(service_kit::telemetry::init_table_stdout_subscriber(level)))
    };
    let control = Control::begin(controlled)?;
    if cfg!(target_os = "linux") {
        start_cmd::linux_wine(paths, &config)?;
        if external && !start_cmd::linux_external_allowlist(&control, &config)? {
            return Ok(ExitCode::SUCCESS);
        }
    }
    let printer = if tui_ring.is_some() {
        ring_printer()
    } else {
        table_printer()
    };
    let mut controller = prepare(paths, &config, external, printer)?;
    controller.set_invite_info(None, fingerprint(paths, &config));
    if controller.is_empty() {
        say(&empty_build_message());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::message(format!("could not start the runtime: {e}")))?;
    runtime.block_on(async move {
        let (stop_tx, stop_rx) = watch::channel(false);
        {
            let control = control.clone();
            std::thread::Builder::new()
                .name("stop".into())
                .spawn(move || {
                    control.wait();
                    let _ = stop_tx.send(true);
                })
                .map_err(|e| HostError::message(format!("could not start a thread: {e}")))?;
        }
        if let Err(error) = controller.start_all().await {
            say(&format!("\n{error}"));
            controller.shutdown().await;
            return Err(HostError::message(error));
        }
        if control.asked() {
            controller.shutdown().await;
            return Ok(ExitCode::SUCCESS);
        }
        say("");
        say(&start_cmd::up_line(&config));
        if controlled {
            say(SERVER_READY_LINE);
        } else if interactive {
            say("Type `help` for the console's commands; `stop` or Ctrl+C stops everything.");
        } else {
            say("Make an invite in another terminal: reforged-host invite --for \"Alex\"");
            say("Ctrl+C stops everything.");
        }
        if let Some(ring) = tui_ring {
            if let Err(error) = crate::tui::run_tui(&mut controller, ring, reload_handle, Some(paths), stop_rx).await {
                say(&format!("TUI: {error}"));
            }
        } else if interactive {
            crate::repl::run_repl(&mut controller, reload_handle, Some(paths), stop_rx).await;
        } else {
            let mut stop_rx = stop_rx;
            let _ = stop_rx.wait_for(|stopped| *stopped).await;
        }
        say("\nStopping...");
        controller.shutdown().await;
        say("Stopped.");
        Ok(ExitCode::SUCCESS)
    })
}

/// The server inside another program (the launcher's server GUI): checked and composed, its processes'
/// output through `printer`, no signals. It blocks (setup's files, the 1.4.4 migrations): call it off
/// the async threads, then `start_all`; the caller stops it with `shutdown`.
pub fn prepare_embedded(paths: &Paths, printer: Printer) -> Result<ServerController, String> {
    let config = start_cmd::load_config(paths).map_err(|e| e.to_string())?;
    let external = external_allowlist(&config);
    let controller = prepare(paths, &config, external, printer).map_err(|e| e.to_string())?;
    controller.set_invite_info(None, fingerprint(paths, &config));
    Ok(controller)
}

/// `server-minimal`: `serve` without a console. `--config <file>`, `--root <dir>` (the folder that
/// holds `.workspace/`; `--workspace` means the same), `--log-level <filter>`.
pub fn minimal_main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut options = ServeOptions::start();
    let mut root: Option<PathBuf> = None;
    let mut idx = 0;
    while idx < args.len() {
        let value = |idx: &mut usize| -> Option<String> {
            *idx += 1;
            args.get(*idx).cloned()
        };
        match args[idx].as_str() {
            "--config" | "-c" => match value(&mut idx) {
                Some(file) => options.config_file = Some(PathBuf::from(file)),
                None => {
                    eprintln!("--config needs a file (.toml, .yaml, .yml or .json)");
                    return ExitCode::from(2);
                }
            },
            "--root" | "--workspace" | "-w" => match value(&mut idx) {
                Some(dir) => root = Some(PathBuf::from(dir)),
                None => {
                    eprintln!("--root needs a folder");
                    return ExitCode::from(2);
                }
            },
            "--log-level" => match value(&mut idx) {
                Some(level) => options.log_filter = level,
                None => {
                    eprintln!("--log-level needs a level");
                    return ExitCode::from(2);
                }
            },
            "--help" | "-h" => {
                println!(
                    "Usage: dainty-lass-server-minimal [OPTIONS]\n  -c, --config <FILE>      host.toml, .yaml, .yml or .json (the workspace's otherwise)\n      --root <DIR>         The server folder (holds .workspace/); found from this binary otherwise\n      --log-level <LEVEL>  error, warn, info, debug or trace (default: info)"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
        idx += 1;
    }
    let root = match root {
        Some(root) => root,
        None => match crate::paths::locate_repo_root() {
            Ok(root) => root,
            Err(error) => {
                eprintln!("\nError: {error}");
                return ExitCode::FAILURE;
            }
        },
    };
    match run(&Paths::new(root), &options) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("\nError: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostconfig::{DEFAULT_MONGO_URI, FileForm, MongoConfig, Registration};

    fn config(mode: ServerMode) -> HostConfig {
        HostConfig {
            version: 1,
            mode,
            host: "100.101.102.103".into(),
            game_address: "100.101.102.103".into(),
            name: "Friday Hunts".into(),
            game_dir: None,
            registration: Registration::parse("INVITECODE").unwrap(),
            share_url: None,
            mongo: MongoConfig {
                uri: DEFAULT_MONGO_URI.into(),
                manage: true,
                mongod: None,
            },
            max_hunt_servers: 3,
            multipliers: None,
            public_port: None,
            dashboard_url: None,
            performance: None,
            linux: None,
            database: None,
            guard: None,
            history: None,
            modules: None,
            services: None,
            env: None,
            network: None,
            remote: None,
            switches: Default::default(),
            file: FileForm::default(),
        }
    }

    #[test]
    fn chat_gets_the_dashboard_only_when_host_toml_names_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        ensure_server_files(&paths, &config(ServerMode::Private)).unwrap();
        let env = module_env(&paths, "gateway").unwrap();
        assert_eq!(env.get("CHAT_DASHBOARD_URL"), None);

        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let mut c = config(ServerMode::Private);
        c.dashboard_url = Some("https://dash.example".into());
        ensure_server_files(&paths, &c).unwrap();
        let env = module_env(&paths, "gateway").unwrap();
        assert_eq!(env.get("CHAT_DASHBOARD_URL"), Some("https://dash.example"));
        let secrets = std::fs::read_to_string(paths.config.join(hostconfig::SECRET_FILE)).unwrap();
        let key = env.get("CHAT_DASHBOARD_INGEST_KEY").unwrap();
        assert_eq!(key.len(), 64);
        assert!(secrets.contains(&format!("DASHBOARD_INGEST_KEY={key}")));
    }

    #[test]
    fn writes_what_setup_writes_only_when_it_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let c = config(ServerMode::Private);
        let written = ensure_server_files(&paths, &c).unwrap();
        assert!(written.iter().any(|f| f.ends_with("gateway.env")), "{written:?}");
        assert!(paths.config.join(hostconfig::SECRET_FILE).is_file());
        let secrets_before = std::fs::read_to_string(paths.config.join(hostconfig::SECRET_FILE)).unwrap();
        std::fs::write(start_cmd::env_file(&paths, "gateway"), "GATEWAY_PORT=1\n").unwrap();
        let again = ensure_server_files(&paths, &c).unwrap();
        assert!(again.is_empty(), "every file is there: nothing rewritten");
        assert_eq!(
            std::fs::read_to_string(start_cmd::env_file(&paths, "gateway")).unwrap(),
            "GATEWAY_PORT=1\n"
        );
        assert_eq!(
            std::fs::read_to_string(paths.config.join(hostconfig::SECRET_FILE)).unwrap(),
            secrets_before,
            "secrets are made once"
        );
    }

    #[test]
    fn the_env_files_are_the_ones_the_modules_read() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        ensure_server_files(&paths, &config(ServerMode::Private)).unwrap();
        let env = module_env(&paths, "gateway").unwrap();
        assert!(env.get("GATEWAY_SECRET").is_some_and(|s| s.len() == 64));
        let content = module_env(&paths, "content").unwrap();
        assert_eq!(content.get("GATEWAY_SECRET"), None, "each module sees its own file");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn modules_come_from_host_toml_beside_the_servers_own_stack() {
        use module_api::stub::StubModule;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let mut c = config(ServerMode::Private);
        let digest = "ab".repeat(32);
        let toml = format!(
            "[modules]\ncompiled = [\"stub\"]\n[[modules.external]]\nid = \"far\"\npath = \"modules/far.exe\"\n\
             sha256 = \"{digest}\"\nautoload = false\n[modules.stub.stub]\ngreeting = \"hi\"\n"
        );
        let doc = crate::hosttoml::document(&toml).unwrap();
        c.modules = Some(hostconfig::ModulesConfig::from_json(&doc["modules"]).unwrap());
        let controller = ServerController::new("private", Vec::new());
        let stub: controller::ModuleFactory =
            Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn module_api::GameModule>);
        let registry = new_registry(&c, &controller);
        list_modules(&paths, &c, &registry, vec![stub]).unwrap();
        let listed: Vec<(String, bool)> = registry
            .list()
            .into_iter()
            .map(|m| (m.id.to_string(), m.autoload))
            .collect();
        assert_eq!(listed, [("far".to_string(), false), ("stub".to_string(), true)]);
        registry.load(&"stub".parse().unwrap()).await.unwrap();
        let echo = module_api::CommandRequest {
            actor: module_api::stub::console(),
            name: "echo".into(),
            args: Vec::new(),
        };
        assert_eq!(
            registry.command(&"stub".parse().unwrap(), echo).await.unwrap().text,
            "hi"
        );
        // An external module's path is the server folder's, and nothing runs that isn't there
        let far = registry.load(&"far".parse().unwrap()).await.unwrap_err().to_string();
        assert!(far.contains(&dir.path().join("modules").display().to_string()), "{far}");
        registry.unload_all().await;
        // The 1.14.7 stack this server runs takes its version: no module may claim it
        let mut taken = config(ServerMode::Private);
        taken.modules = None;
        let isles: controller::ModuleFactory = Arc::new(|| {
            struct Isles(StubModule);
            impl module_api::GameModule for Isles {
                fn manifest(&self) -> module_api::ModuleManifest {
                    module_api::ModuleManifest {
                        id: "isles-1-14-7".parse().unwrap(),
                        game_version: protocol::invite::reforged_version(),
                        ..self.0.manifest()
                    }
                }
                fn register(&self, r: &mut module_api::Registrar) -> Result<(), module_api::ModuleError> {
                    self.0.register(r)
                }
                fn start(
                    &self,
                    ctx: module_api::ModuleContext,
                    config: module_api::ModuleConfig,
                ) -> module_api::BoxFuture<'_, Result<(), module_api::ModuleError>> {
                    self.0.start(ctx, config)
                }
                fn stop(
                    &self,
                    ctx: module_api::ModuleContext,
                ) -> module_api::BoxFuture<'_, Result<(), module_api::ModuleError>> {
                    self.0.stop(ctx)
                }
                fn status(&self) -> module_api::ModuleStatus {
                    self.0.status()
                }
            }
            Arc::new(Isles(StubModule::new())) as Arc<dyn module_api::GameModule>
        });
        let registry = new_registry(&taken, &controller);
        list_modules(&paths, &taken, &registry, vec![isles]).unwrap();
        if cfg!(feature = "paradox") {
            let refused = registry
                .load(&"isles-1-14-7".parse().unwrap())
                .await
                .unwrap_err()
                .to_string();
            assert!(refused.contains("this server's own 1.14.7 stack"), "{refused}");
        } else {
            registry.load(&"isles-1-14-7".parse().unwrap()).await.unwrap();
            assert_eq!(
                registry.info(&"isles-1-14-7".parse().unwrap()).unwrap().state,
                controller::ModuleState::Ready
            );
            registry.unload_all().await;
        }
    }

    #[test]
    fn service_libraries_come_from_host_toml_by_name() {
        use module_api::CoreCapability;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let mut c = config(ServerMode::Private);
        assert!(service_libraries(&paths, &c).is_empty());
        let d = "ab".repeat(32);
        let doc = crate::hosttoml::document(&format!(
            "[services.gateway]\nlibrary = \"crates/gateway.dll\"\nsha256 = \"{d}\"\n\
             [services.chat]\nlibrary = \"crates/chat.dll\"\nsha256 = \"{d}\"\n\
             [services.echo]\nlibrary = \"plugins/echo.dll\"\nsha256 = \"{d}\"\nserves = [\"content_files\"]\n"
        ))
        .unwrap();
        c.services = Some(hostconfig::services::from_json(&doc["services"]).unwrap());
        let libraries: Vec<(&str, PathBuf, Vec<CoreCapability>)> = service_libraries(&paths, &c)
            .into_iter()
            .map(|l| (l.name, l.path, l.serves.to_vec()))
            .collect();
        assert_eq!(
            libraries,
            [
                ("chat", dir.path().join("crates/chat.dll"), vec![CoreCapability::Chat]),
                (
                    "echo",
                    dir.path().join("plugins/echo.dll"),
                    vec![CoreCapability::ContentFiles]
                ),
                (
                    "gateway",
                    dir.path().join("crates/gateway.dll"),
                    vec![CoreCapability::Routes, CoreCapability::UdpPorts]
                ),
            ]
        );
        assert!(library_configured(&c, "chat") && !library_configured(&c, "content"));
    }

    #[test]
    fn an_empty_build_says_so() {
        let text = empty_build_message();
        assert!(text.contains("admin"), "{text}");
    }
}
