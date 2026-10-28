//! `serve` and `start`: the server as one process (`docs/app-structure-plan.md`, RD26). The host kit
//! composes the controller's modules from host.toml and the env files `setup` writes: the Rust
//! services built into this binary run in-process ([`controller::modules`]); MongoDB and Mystic
//! Paradox's backend and Director run as supervised processes until the RB rows (RD17).
//!
//! `start` keeps the Node kit's contract (RD23): no console of its own, `REFORGED_CONTROL=stdin`,
//! the `REFORGED_SERVER_READY` line, the backend on port 61000. `serve` is the same server with the
//! REPL, or the dashboard with `--tui`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use controller::{Module, ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions, ServerController, modules};
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
    /// `reforged-host start`: the Node kit's contract, no console.
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

/// MongoDB (until RB14) as a module: `mongod` started when the kit manages it, the replica set
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

/// The server's modules in start order: the database, the allowlist (its table closes the game
/// ports before any game server listens), the backend, the Director, the content server, the admin
/// API, the gateway last. A front service runs in this process once [`rust_ported`] says so and this
/// binary has it; until then its Node build runs as a supervised process, as `start` always ran it
/// (flipping the switch is the front-door cutover, RS5). The Node processes come with `paradox`.
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
    let reborn = config.edition.runs_reborn();
    let node_runs = cfg!(feature = "paradox") && reborn;
    let games = config.edition.versions().iter().map(ToString::to_string).collect();
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
    if reborn {
        controller.add(mongod_module(paths, config, printer.clone()));
    }

    #[cfg(any(feature = "gateway", feature = "allowlist"))]
    #[cfg_attr(not(feature = "allowlist"), allow(unused_mut))]
    let mut slot: Option<modules::AllowlistSlot> = None;
    // Whether this process guards the UDP ranges of game modules (DF4d)
    #[cfg(feature = "allowlist")]
    let mut guarded = false;
    if reborn && public && !external_allowlist {
        if in_process(FrontService::Allowlist) {
            #[cfg(feature = "allowlist")]
            {
                let helper = modules::allowlist_slot();
                controller.add(
                    modules::allowlist(&module_env(paths, "allowlist")?, helper.clone(), Some(registry.clone()))
                        .map_err(HostError::message)?,
                );
                slot = Some(helper);
                guarded = true;
            }
        } else if node_runs {
            controller.add(front_process(
                FrontService::Allowlist,
                Ready::http(PORTS.allowlist, "/status"),
            ));
        }
    }

    #[cfg(feature = "paradox")]
    if reborn {
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

    if !reborn {
    } else if in_process(FrontService::Content) {
        #[cfg(feature = "content")]
        controller
            .add(modules::content(&module_env(paths, "content")?, Some(registry.clone())).map_err(HostError::message)?);
    } else if node_runs {
        controller.add(front_process(FrontService::Content, Ready::http(PORTS.content, "/content/v1/news")).optional());
    }

    // The admin API has no Node build; it runs when setup wrote its env file (an older setup did not)
    #[cfg(feature = "admin")]
    if reborn && start_cmd::env_file(paths, "admin").is_file() {
        controller
            .add(modules::admin(&module_env(paths, "admin")?, Some(registry.clone())).map_err(HostError::message)?);
    }

    #[cfg(feature = "gateway")]
    if let Some(network) = &config.network
        && !network.nodes.is_empty()
    {
        let cert = paths.tls.join(hostconfig::NODE_CERT_FILE);
        let key = paths.tls.join(hostconfig::NODE_KEY_FILE);
        let ca = paths.tls.join(hostconfig::CLUSTER_CA_FILE);
        let credentials = network_plugin::cluster::NodeCredentials::load(&cert, &key, &ca)
            .map_err(|e| HostError::message(format!("network credentials in {}: {e}", paths.tls.display())))?;
        let said = credentials.identity();
        if said.node != network.node || said.region != network.region {
            return Err(HostError::message(format!(
                "the node certificate in {} is for {} in {}, but host.toml says {} in {}",
                paths.tls.display(),
                said.node,
                said.region,
                network.node,
                network.region
            )));
        }
        std::fs::create_dir_all(&paths.tls).map_err(|e| HostError::io(&paths.tls, e))?;
        let key_path = paths.tls.join(hostconfig::SESSION_KEY_FILE);
        let signing_key = network_plugin::home::SigningKey::load_or_create(&key_path)
            .map_err(|e| HostError::message(format!("network session key {}: {e}", key_path.display())))?;
        let database = hostconfig::node_database_config(config, paths)?;
        let game = config
            .edition
            .versions()
            .first()
            .cloned()
            .unwrap_or_else(protocol::invite::reforged_version);
        let peer_log_dir = Some(paths.workspace.join("peer-logs"));
        let ship_to_peer = config
            .history
            .as_ref()
            .and_then(|h| h.ship_to_peer.clone())
            .map(|peer| (peer, paths.config.join("history-peer-state.json")));
        controller.add(
            modules::network(modules::NetworkOptions {
                node: said.clone(),
                leader: network.effective_leader(),
                peers: network.nodes.clone(),
                credentials,
                signing_key,
                database,
                game,
                registry: registry.clone(),
                allowlist: slot.clone(),
                replication_interval: Duration::from_secs(5),
                peer_log_dir,
                ship_to_peer,
            })
            .map_err(HostError::message)?,
        );
    }

    if !reborn {
    } else if in_process(FrontService::Gateway) {
        #[cfg(feature = "gateway")]
        controller.add(
            modules::gateway(&module_env(paths, "gateway")?, slot.clone(), Some(registry.clone()))
                .map_err(HostError::message)?,
        );
    } else if node_runs {
        let ready = match config.mode {
            ServerMode::Public => Ready::tcp("127.0.0.1", hostconfig::gateway_port(config)),
            ServerMode::Private => Ready::tcp(&config.game_address, PORTS.private_gateway),
        };
        controller.add(front_process(FrontService::Gateway, ready));
    }

    // The 1.4.4 stack (alone, or beside reborn on its own ports, RD26): every service a supervised
    // process, the metagame and the deploy server being binaries of their own
    if config.edition.runs_legacy() {
        for planned in crate::legacy::legacy_service_plan(paths, config, external_allowlist, &programs)? {
            let module = process_module(planned.spec, planned.ready_timeout, printer.clone(), paths.logs.clone())
                .for_edition(protocol::invite::revived_version().to_string());
            controller.add(if planned.required { module } else { module.optional() });
        }
    }

    #[cfg(any(feature = "gateway", feature = "allowlist"))]
    let _ = &slot;
    // A public server guarded by another process (the Node helper, Docker's allowlist) cannot take a module's
    // ports; without the plugin they would simply be open
    #[cfg(feature = "allowlist")]
    if public && !guarded {
        modules::allowlist_absent(&registry);
    }
    list_modules(paths, config, &registry, controller::modules::game_modules())?;
    controller.set_registry(registry);
    Ok(controller)
}

/// The module registry (DF3): the server's own ports and the versions its own stacks serve are taken, and
/// every child gets only the system's variables, as `start` gives its services.
pub fn new_registry(config: &HostConfig, controller: &ServerController) -> ModuleRegistry {
    use module_api::manifest::Transport;
    use module_api::{NodeIdentity, PortRange};

    let identity = config
        .network
        .as_ref()
        .map_or_else(NodeIdentity::standalone, |network| NodeIdentity {
            node: network.node.clone(),
            region: network.region.clone(),
        });
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
    let mut udp = vec![hostconfig::game_ports(config)];
    if config.edition.runs_legacy() {
        tcp.push(hostconfig::legacy_gateway_port(config));
        udp.push(hostconfig::legacy_game_ports(config));
    }
    options.reserved_ports = tcp
        .into_iter()
        .map(|port| (Transport::Tcp, PortRange::single(port)))
        .chain(
            udp.into_iter()
                .map(|(first, last)| (Transport::Udp, PortRange { first, last })),
        )
        .collect();
    options.taken_versions = config
        .edition
        .versions()
        .into_iter()
        .map(|version| {
            let by = format!(
                "this server's own {version} stack (edition = \"{}\")",
                config.edition.as_str()
            );
            (version, by)
        })
        .collect();
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
    for feature in controller::modules::UNLINKED {
        say(&format!(
            "- built with {feature}, whose module crate is not linked yet: it has nothing to load"
        ));
    }
    let modules = config.modules.clone().unwrap_or_default();
    let mut settings = modules.settings.clone();
    let mut entry = |id: module_api::ModuleId, source: ModuleSource, autoload: bool| {
        let mut mod_settings = settings.remove(&id).unwrap_or_default();
        if let Some(switches) = &config.switches
            && matches!(id.as_str(), "isles-1-14-7" | "isles-1-12-0")
        {
            let section = mod_settings.entry("switches".to_owned()).or_default();
            for (key, value) in switches.module_switches() {
                section.entry(key.to_owned()).or_insert(value);
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
        entry(id, ModuleSource::CompiledIn(factory), true)?;
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
    if config.edition.runs_reborn() && missing(&reborn_files) {
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
    if config.edition.runs_legacy() && missing(&crate::legacy::env_file_names(config)) {
        written.extend(crate::legacy::write_env_files(
            config,
            &secrets,
            paths,
            &source_commit(&paths.repo),
            Platform::current(),
        )?);
        say(&format!("- 1.4.4 env files written in {}", paths.config.display()));
    }
    #[cfg(feature = "gateway")]
    if config.mode == ServerMode::Public && !paths.cert_file.exists() {
        gateway::cert::load_or_generate(&paths.cert_file, &paths.key_file, std::slice::from_ref(&config.host))
            .map_err(HostError::message)?;
        say(&format!(
            "- gateway certificate made in {} (fingerprint {})",
            paths.tls.display(),
            crate::invite::certificate_fingerprint(&paths.cert_file)?
        ));
    }
    Ok(written)
}

/// The settings to run from: `--config <file>` in any of the three formats, else the workspace's.
fn load_config(paths: &Paths, config_file: Option<&Path>) -> Result<HostConfig, HostError> {
    match config_file {
        Some(file) => hostconfig::read_host_config(file)?
            .ok_or_else(|| HostError::message(format!("{} does not exist", file.display()))),
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
    hostconfig::check_edition(config)?;
    ensure_server_files(paths, config)?;
    if config.edition.runs_reborn() {
        start_cmd::preflight_with(paths, config, external)?;
    } else {
        start_cmd::host_checks(paths, config, external)?;
    }
    if config.edition.runs_legacy() {
        crate::legacy::prepare(paths, config)?;
    }
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
            edition: hostconfig::Edition::Reborn,
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
            legacy_game_dir: None,
            legacy_public_port: None,
            performance: None,
            linux: None,
            database: None,
            guard: None,
            history: None,
            modules: None,
            network: None,
            switches: Default::default(),
            file: FileForm::default(),
        }
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
        let refused = registry
            .load(&"isles-1-14-7".parse().unwrap())
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("this server's own 1.14.7 stack"), "{refused}");
    }

    #[test]
    fn an_empty_build_says_so() {
        let text = empty_build_message();
        assert!(text.contains("gateway, allowlist, content, admin"), "{text}");
    }
}
