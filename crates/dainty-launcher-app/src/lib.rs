// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The launcher's MIT Tauri shell (RD27 spec §2.1, RL13): start-up, the window, the bridge's commands and the
//! controller's [`Platform`](dainty_launcher_kit::controller::Platform) over Tauri. Everything below Tauri is
//! dainty-launcher-kit's.

mod art;
mod bridge;
mod cli;
mod dialogs;
mod epic_login;
mod platform;
mod tray;
mod updater;
mod window;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dainty_epic::auth::EpicAccounts;
use dainty_launcher_kit::controller::cli::{self as command_line, CliRequest, USAGE, USAGE_EXIT};
use dainty_launcher_kit::controller::install::EpicGate;
use dainty_launcher_kit::controller::{Controller, ControllerConfig, Seams};
use dainty_launcher_kit::core::i18n::Language;
use dainty_launcher_kit::net::relay;
use dainty_launcher_kit::platform::invite_link::{self, Channel, Claim, Holder, LinkArg};
use dainty_launcher_kit::platform::keystore::epic::EpicSessionStore;
use dainty_launcher_kit::platform::keystore::legacy::{ElectronReader, LegacyReader};
use dainty_launcher_kit::platform::keystore::os::{Encryptor, platform_encryptor};
use dainty_launcher_kit::platform::paths::{DEV_PROFILE_VAR, Paths};
use dainty_launcher_kit::platform::{epic_library, epic_session::Epic, log};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::art::ArtStore;
use crate::bridge::backend::Backend;
use crate::platform::AppPlatform;
use crate::updater::Updates;
use crate::window::{CloseGuard, MAIN};

#[cfg(feature = "server-gui")]
pub use dainty_launcher_kit::platform::hostserver::EmbeddedServer;

/// Set by a launcher that restarted itself, so the new one waits for the old to exit (§2.1.2 step 1).
const PARENT_PID_VAR: &str = "DAINTY_PARENT_PID";
/// `0` turns off installing the secure-storage packages.
const AUTO_INSTALL_VAR: &str = "DAINTY_AUTO_INSTALL";
/// Set on the restarted launcher, so it never bootstraps twice.
const BOOTSTRAPPED_VAR: &str = "DAINTY_STORAGE_BOOTSTRAPPED";

/// Which app this build is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// The player's launcher.
    Client,
    /// The server manager.
    Manager,
}

impl Flavour {
    /// The flavour this build was compiled as.
    pub const fn current() -> Self {
        if cfg!(feature = "server-gui") {
            Self::Manager
        } else {
            Self::Client
        }
    }

    const fn window_title(self) -> &'static str {
        match self {
            Self::Client => "Dainty Lass Launcher",
            Self::Manager => "Dainty Lass Manager",
        }
    }

    const fn channel_id(self) -> &'static str {
        match self {
            Self::Client => invite_link::CLIENT_ID,
            Self::Manager => invite_link::MANAGER_ID,
        }
    }
}

/// What a build hands the shell beyond its features.
#[derive(Default)]
pub struct Options {
    /// The manager's in-process server (`dainty-manager-server`).
    #[cfg(feature = "server-gui")]
    pub embedded_server: Option<Arc<dyn EmbeddedServer>>,
    /// The window's and the tray's icon in place of the config's.
    pub icon: Option<tauri::image::Image<'static>>,
}

/// Why the app could not start.
#[derive(Debug)]
pub enum StartError {
    /// No home folder, so no folders at all.
    NoFolders,
    /// Tauri failed.
    Tauri(tauri::Error),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFolders => f.write_str("no home folder to keep the launcher's data in"),
            Self::Tauri(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for StartError {}

impl From<tauri::Error> for StartError {
    fn from(error: tauri::Error) -> Self {
        Self::Tauri(error)
    }
}

/// The relay's port: the override when it passes §2.6.4's rule, else the default.
fn relay_port(var: Option<&str>) -> u16 {
    var.and_then(relay::port_override).unwrap_or(relay::DEFAULT_PORT)
}

/// Whether a launcher without key storage installs the secure-storage packages and restarts (§2.1.2 step 5).
fn bootstraps_storage(development_build: bool, auto_install: Option<&str>, bootstrapped: Option<&str>) -> bool {
    !development_build && auto_install != Some("0") && bootstrapped != Some("1")
}

/// Whether this run registers the invite schemes for its executable (§2.1.2 step 9).
fn registers_schemes(packaged: bool, flavour: Flavour, dev_profile: Option<&OsString>) -> bool {
    packaged && flavour == Flavour::Client && dev_profile.is_none_or(|v| v.is_empty())
}

/// Waits (at most 100 × 100 ms) until `alive` says the process that restarted us is gone.
fn wait_for_parent(mut alive: impl FnMut() -> bool, mut sleep: impl FnMut()) {
    for _ in 0..100 {
        if !alive() {
            return;
        }
        sleep();
    }
}

/// Runs the app until it exits (§2.1.2), or a command line (`play`, `export`, `servers`) to its end; the answer is
/// the process's exit code.
///
/// # Errors
///
/// When the folders cannot be resolved or Tauri cannot start.
pub fn run(mut options: Options) -> Result<i32, StartError> {
    let flavour = Flavour::current();
    // 1. The arguments, after the restarted-from launcher is gone
    let args: Vec<OsString> = std::env::args_os().collect();
    let user_args: Vec<String> = args.iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let cli: Option<CliRequest> = match command_line::parse(&user_args) {
        None => None,
        Some(Ok(request)) if request.detaches() => return Ok(cli::detach(&user_args)),
        Some(Ok(request)) => Some(request),
        Some(Err(problem)) => {
            println!("error: {problem}\n\n{USAGE}");
            return Ok(USAGE_EXIT);
        }
    };
    if cfg!(target_os = "linux")
        && let Some(pid) = std::env::var(PARENT_PID_VAR).ok().filter(|p| p.parse::<u32>().is_ok())
    {
        let proc = PathBuf::from("/proc").join(pid);
        wait_for_parent(
            || proc.exists(),
            || std::thread::sleep(std::time::Duration::from_millis(100)),
        );
    }

    // 2. One launcher per user: hand the arguments over and leave
    let channel = Channel::current(flavour.channel_id(), &args);
    // A command line hands nothing over: the open launcher only comes to the front
    let handed: &[String] = if cli.is_some() { &[] } else { &user_args };
    let holder = match tauri::async_runtime::block_on(invite_link::claim(&channel, handed)) {
        Ok(Claim::HandedOver) if cli.is_some() => {
            println!("error: the launcher is already open; close it and run the command again");
            return Ok(command_line::REFUSED_EXIT);
        }
        Ok(Claim::HandedOver) => return Ok(0),
        Ok(Claim::First(holder)) => Some(holder),
        Err(_) => None,
    };

    // 3. Logging and TLS
    log::install();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let context = tauri::generate_context!();
    let version = context.package_info().version.to_string();

    // 4. Folders
    let paths = Paths::resolve().ok_or(StartError::NoFolders)?;
    log::start_session(&paths.logs());
    log::info(format!("Dainty Lass Launcher {version} starting"));
    if holder.is_none() {
        log::warn("could not hold the single-launcher channel; a second launcher will start on its own");
    }

    // 5. Key storage
    let development_build = tauri::is_dev();
    let encryptor = platform_encryptor();
    if cli.is_none()
        && cfg!(target_os = "linux")
        && !encryptor.available()
        && bootstrap_storage(development_build, &user_args)
    {
        return Ok(0);
    }
    if let Some(request) = cli {
        let resources = resources_dir(development_build, || {
            tauri::utils::platform::resource_dir(context.package_info(), &tauri::Env::default()).ok()
        });
        let config = controller_config(paths, encryptor, resources, version, development_build);
        return Ok(cli::run(&request, config, holder));
    }

    // 6–7. The Epic ownership cache and the invite link from the arguments
    let ownership = Arc::new(epic_library::ownership(paths.user_data.clone()));
    let link = match invite_link::scan_args(&user_args) {
        LinkArg::Invite(link) => Some(link),
        LinkArg::Refused(error) => {
            log::info(format!("ignored the invite link in the arguments: {error:?}"));
            None
        }
        LinkArg::None => None,
    };

    // 8. The app
    let backend = Arc::new(Backend::new(link));
    let art = Arc::new(ArtStore::default());
    let guard = Arc::new(CloseGuard::default());
    let scheme_art = Arc::clone(&art);
    let icon = window::Icon(options.icon.take());
    let start = Start {
        flavour,
        paths,
        version,
        encryptor,
        ownership: Arc::clone(&ownership),
        backend: Arc::clone(&backend),
        art: Arc::clone(&art),
        guard: Arc::clone(&guard),
        holder,
        development_build,
        options,
    };
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Updates::default())
        .manage(icon)
        .manage(guard)
        .manage(art)
        .manage(backend)
        .manage(ownership)
        .register_asynchronous_uri_scheme_protocol(art::SCHEME, move |_, request, responder| {
            let store = Arc::clone(&scheme_art);
            let uri = request.uri().to_string();
            tauri::async_runtime::spawn(async move { responder.respond(art::respond(&store, &uri).await) });
        })
        .invoke_handler(tauri::generate_handler![
            bridge::commands::get_snapshot,
            bridge::commands::check_invite,
            bridge::commands::submit_invite,
            bridge::commands::retry_connect,
            bridge::commands::forget_server,
            bridge::commands::switch_server,
            bridge::commands::remove_saved_server,
            bridge::commands::register,
            bridge::commands::use_existing_key,
            bridge::commands::import_key_file,
            bridge::commands::save_key_backup,
            bridge::commands::dismiss_backup_offer,
            bridge::commands::choose_game_folder,
            bridge::commands::use_game_path,
            bridge::commands::use_detected_game,
            bridge::commands::verify_game,
            bridge::commands::revived_install,
            bridge::commands::revived_choose_folder,
            bridge::commands::revived_use_detected,
            bridge::commands::trust_runtime,
            bridge::commands::install_runtime,
            bridge::commands::choose_local_runtime,
            bridge::commands::clear_local_runtime,
            bridge::commands::cancel_task,
            bridge::commands::play,
            bridge::commands::leave_queue,
            bridge::commands::set_settings,
            bridge::commands::set_status_polling,
            bridge::commands::refresh_status,
            bridge::commands::get_news,
            bridge::commands::get_branding,
            bridge::commands::open_external,
            bridge::commands::open_game_folder,
            bridge::commands::open_logs_folder,
            bridge::commands::logout,
            bridge::commands::install_update,
            bridge::commands::host_get,
            bridge::commands::host_choose_workspace,
            bridge::commands::host_choose_dumper,
            bridge::commands::host_import_gamedata,
            bridge::commands::host_import_sdk_folder,
            bridge::commands::host_download_project,
            bridge::commands::host_action,
            bridge::commands::host_open,
            bridge::commands::host_server_get,
            bridge::commands::host_server_start,
            bridge::commands::host_server_stop,
            bridge::commands::host_server_restart,
            bridge::commands::host_setup,
            bridge::commands::host_create_invite,
            bridge::commands::host_update,
            bridge::commands::host_set_multipliers,
            bridge::commands::copy_text,
            bridge::commands::take_invite_link,
            bridge::commands::game_log_get,
            bridge::commands::dismiss_notice,
            bridge::commands::dismiss_error,
            bridge::commands::subscribe_progress,
            bridge::commands::epic_sign_in_start,
            bridge::commands::epic_sign_in_code,
            bridge::commands::epic_get,
            bridge::commands::epic_sign_out,
            bridge::commands::epic_library_check,
            bridge::commands::game_version_list,
            bridge::commands::game_install_start,
            bridge::commands::game_install_pause,
            bridge::commands::game_install_resume,
            bridge::commands::game_install_cancel,
            bridge::commands::game_install_get,
            bridge::commands::game_install_choose_folder,
            bridge::commands::game_install_discard,
            bridge::commands::region_ping,
            bridge::commands::module_list,
            bridge::commands::module_get,
            bridge::commands::module_run_command,
            bridge::commands::module_set_settings,
            bridge::commands::module_load,
            bridge::commands::module_unload,
            bridge::commands::realm_host,
            bridge::commands::realm_join,
            bridge::commands::realm_stop,
            bridge::commands::realm_get,
            bridge::commands::history_query,
            bridge::commands::history_revert_dry_run,
            bridge::commands::history_revert_apply,
            bridge::commands::guarded_list,
            bridge::commands::guarded_approve,
            bridge::commands::guarded_cancel,
            bridge::commands::server_doctor,
            bridge::commands::server_backup,
            bridge::commands::server_config_get,
            bridge::commands::server_config_save,
            bridge::commands::server_migrate_mongo,
            bridge::commands::server_confirm_fresh_start,
            bridge::commands::server_invite_list,
            bridge::commands::admin_action,
            bridge::commands::remote_status,
            bridge::commands::remote_init_controller,
            bridge::commands::remote_invite,
            bridge::commands::remote_enroll,
            bridge::commands::remote_revoke,
            bridge::commands::remote_leave,
            bridge::commands::remote_exec,
            bridge::commands::update_launcher,
            bridge::commands::game_uninstall_plan,
            bridge::commands::game_uninstall,
            bridge::commands::get_playtime,
            bridge::commands::crash_report_send,
            bridge::commands::crash_report_dismiss,
        ])
        .setup(move |app| {
            start.setup(app.handle())?;
            Ok(())
        })
        .build(context)?;

    // 10. The first exit request shuts the controller down first
    let exiting = AtomicBool::new(false);
    app.run(move |handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event
            && !exiting.swap(true, Ordering::SeqCst)
        {
            api.prevent_exit();
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(controller) = handle.try_state::<Controller>() {
                    controller.inner().clone().shutdown().await;
                }
                handle.exit(0);
            });
        }
    });
    Ok(0)
}

/// The launcher's resources: the brand folder in a development build, else the bundle's.
fn resources_dir(development_build: bool, bundled: impl FnOnce() -> Option<PathBuf>) -> PathBuf {
    if development_build {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../brand/launcher")
    } else {
        bundled().unwrap_or_default()
    }
}

/// The controller's config, the same for the window and a command line (§2.1.2 step 9).
fn controller_config(
    paths: Paths,
    encryptor: Arc<dyn Encryptor>,
    resources: PathBuf,
    version: String,
    development_build: bool,
) -> ControllerConfig {
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_owned());
    let legacy: Option<Arc<dyn LegacyReader>> =
        ElectronReader::supported().then(|| Arc::new(ElectronReader::new(&paths.user_data)) as _);
    let mut config = ControllerConfig::new(paths, encryptor);
    config.resources = resources;
    config.relay_port = relay_port(std::env::var(relay::PORT_VAR).ok().as_deref());
    config.os_language = Language::from_system(&locale);
    config.os_locale = locale;
    config.legacy_reader = legacy;
    config.version = version;
    config.packaged = !development_build;
    config.development_build = development_build;
    config
}

/// Linux without a Secret Service: installs it and starts again, unless told not to. True when this process
/// should exit because the restarted one carries on.
fn bootstrap_storage(development_build: bool, user_args: &[String]) -> bool {
    log::warn("no Secret Service: account keys cannot be stored");
    let var = |name| std::env::var(name).ok();
    if !bootstraps_storage(
        development_build,
        var(AUTO_INSTALL_VAR).as_deref(),
        var(BOOTSTRAPPED_VAR).as_deref(),
    ) {
        return false;
    }
    let env = dainty_launcher_kit::platform::linux_runtime::current_env();
    let Some(plan) = dainty_launcher_kit::platform::linux_runtime::install_plan(
        &env,
        dainty_launcher_kit::platform::linux_runtime::DependencyKind::SecureStorage,
    ) else {
        log::warn("no package manager plan for secure storage; continuing without key storage");
        return false;
    };
    log::info(format!("installing secure storage: {}", plan.display_command));
    if let Err(error) = tauri::async_runtime::block_on(dainty_launcher_kit::platform::linux_runtime::run_install(
        &plan, &env.vars,
    )) {
        log::warn(format!("could not install secure storage: {error}"));
        return false;
    }
    let restarted = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .args(user_args)
            .env(BOOTSTRAPPED_VAR, "1")
            .env(PARENT_PID_VAR, std::process::id().to_string())
            .spawn()
    });
    match restarted {
        Ok(_) => true,
        Err(error) => {
            log::warn(format!("could not restart after installing secure storage: {error}"));
            false
        }
    }
}

/// What start-up hands the setup step.
struct Start {
    flavour: Flavour,
    paths: Paths,
    version: String,
    encryptor: Arc<dyn Encryptor>,
    ownership: Arc<epic_library::EpicOwnership>,
    backend: Arc<Backend>,
    art: Arc<ArtStore>,
    guard: Arc<CloseGuard>,
    holder: Option<Holder>,
    development_build: bool,
    #[cfg_attr(
        not(feature = "server-gui"),
        allow(dead_code, reason = "only the manager takes options")
    )]
    options: Options,
}

impl Start {
    /// §2.1.2 step 9.
    fn setup(self, app: &AppHandle) -> Result<(), StartError> {
        let packaged = !self.development_build;
        if let Some(holder) = self.holder {
            serve_second_launchers(app.clone(), Arc::clone(&self.backend), holder);
        }
        if registers_schemes(packaged, self.flavour, std::env::var_os(DEV_PROFILE_VAR).as_ref()) {
            tauri::async_runtime::spawn(async {
                if let Err(error) = invite_link::register_schemes().await {
                    log::warn(format!("could not register the invite links: {error}"));
                }
            });
        }

        let epic = Arc::new(Epic::new(
            EpicSessionStore::new(&self.paths.keys(), Arc::clone(&self.encryptor)),
            Arc::new(EpicAccounts),
        ));
        let gate = Arc::new(EpicGate {
            epic: Arc::clone(&epic),
            ownership: self.ownership,
        });

        let resources = resources_dir(self.development_build, || app.path().resource_dir().ok());
        let user_data = self.paths.user_data.clone();
        let config = controller_config(
            self.paths,
            self.encryptor,
            resources,
            self.version,
            self.development_build,
        );

        let platform = Arc::new(AppPlatform::new(app.clone(), self.art, Arc::clone(&self.backend)));
        let controller = Controller::new(config, platform, Seams::default());
        self.guard.attach(controller.clone());
        #[cfg(feature = "server-gui")]
        if let Some(server) = self.options.embedded_server {
            if let Some(panel) = server.modules() {
                self.backend.set_panel(panel);
            }
            controller.set_embedded_server(server);
        }
        app.manage(controller.clone());
        app.manage(epic);
        app.manage(gate);

        window::create(app, self.flavour, Some(user_data.join("WebView2")))?;
        if packaged {
            updater::start(app, controller.clone());
        }
        tauri::async_runtime::spawn(async move { controller.init().await });
        Ok(())
    }
}

/// A second launcher's arguments: bring the window back and pass on its link.
fn serve_second_launchers(app: AppHandle, backend: Arc<Backend>, mut holder: Holder) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Ok(args) = holder.receive().await else {
                continue;
            };
            tray::restore(&app);
            if let LinkArg::Invite(link) = invite_link::scan_args(&args) {
                backend.set_invite_link(link);
                let _ = app.emit_to(MAIN, "invite-link", ());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;

    /// The handler list, read from this file as written, in order.
    fn handler_names() -> Vec<String> {
        let source = include_str!("lib.rs");
        let start = source.find("generate_handler![").unwrap();
        let list = &source[start..];
        let list = &list[..list.find("])").unwrap()];
        list.lines()
            .filter_map(|line| line.trim().strip_prefix("bridge::commands::"))
            .map(|name| name.trim_end_matches(',').to_owned())
            .collect()
    }

    #[test]
    fn handler_list_is_the_command_table() {
        let table: Vec<&str> = dainty_launcher_kit::bridge::spec::COMMANDS
            .iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(table.len(), 109);
        assert_eq!(handler_names(), table);
    }

    #[test]
    fn the_relay_port_override_follows_its_rule() {
        assert_eq!(relay_port(None), 61100);
        assert_eq!(relay_port(Some("not a port")), 61100);
        assert_eq!(
            relay_port(Some("61234")),
            relay::port_override("61234").unwrap_or(61100)
        );
    }

    #[test]
    fn storage_bootstrap_gates() {
        assert!(bootstraps_storage(false, None, None));
        assert!(bootstraps_storage(false, Some("1"), Some("0")));
        assert!(!bootstraps_storage(true, None, None));
        assert!(!bootstraps_storage(false, Some("0"), None));
        assert!(!bootstraps_storage(false, None, Some("1")));
    }

    #[test]
    fn only_a_packaged_client_without_a_profile_registers_schemes() {
        let profile = OsString::from("C:/profile");
        let empty = OsString::new();
        assert!(registers_schemes(true, Flavour::Client, None));
        assert!(registers_schemes(true, Flavour::Client, Some(&empty)));
        assert!(!registers_schemes(true, Flavour::Client, Some(&profile)));
        assert!(!registers_schemes(false, Flavour::Client, None));
        assert!(!registers_schemes(true, Flavour::Manager, None));
    }

    #[test]
    fn waits_for_the_parent_at_most_ten_seconds() {
        let (mut checks, mut sleeps) = (0, 0);
        wait_for_parent(
            || {
                checks += 1;
                true
            },
            || sleeps += 1,
        );
        assert_eq!((checks, sleeps), (100, 100));

        let mut left = 3;
        let mut slept = 0;
        wait_for_parent(
            || {
                left -= 1;
                left > 0
            },
            || slept += 1,
        );
        assert_eq!(slept, 2);
    }

    #[test]
    fn the_flavour_names_its_window_and_channel() {
        assert_eq!(Flavour::Client.window_title(), "Dainty Lass Launcher");
        assert_eq!(Flavour::Manager.window_title(), "Dainty Lass Manager");
        assert_eq!(Flavour::Client.channel_id(), invite_link::CLIENT_ID);
        assert_eq!(Flavour::Manager.channel_id(), invite_link::MANAGER_ID);
        assert_eq!(Flavour::current() == Flavour::Manager, cfg!(feature = "server-gui"));
    }

    #[test]
    fn the_version_is_the_launchers_package() {
        let package: serde_json::Value = serde_json::from_str(include_str!("../../../launcher/package.json")).unwrap();
        assert_eq!(package["version"], dainty_launcher_kit::VERSION);
        let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        assert_eq!(context.package_info().version.to_string(), dainty_launcher_kit::VERSION);
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["version"], "../../launcher/package.json");
    }

    #[test]
    fn the_capability_grants_the_command_set_and_nothing_else() {
        let capability: serde_json::Value = serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        assert_eq!(capability["identifier"], "main");
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        let granted: Vec<&str> = capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        assert_eq!(
            granted,
            [
                "app-commands",
                "core:event:default",
                "core:window:allow-minimize",
                "core:window:allow-toggle-maximize",
                "core:window:allow-internal-toggle-maximize",
                "core:window:allow-is-maximized",
                "core:window:allow-start-dragging",
                "core:window:allow-close",
            ]
        );
    }

    #[test]
    fn the_desktop_entry_takes_the_invite_schemes() {
        let entry = include_str!("../linux/launcher.desktop");
        let schemes: String = dainty_protocol::invite::invite_schemes()
            .iter()
            .map(|s| format!("x-scheme-handler/{s};"))
            .collect();
        assert!(
            entry.lines().any(|line| line == format!("MimeType={schemes}")),
            "{schemes}"
        );
        assert!(entry.contains("StartupWMClass={{exec}}"));
    }
}
