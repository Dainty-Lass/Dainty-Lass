// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The page's 100 commands (RD27 spec §2.3.1), in the table's order. Each checks its arguments again with
//! dainty-launcher-kit's `bridge::checks` (the isolation hook guards the page, these guard the backend), then calls the
//! controller.

#![allow(clippy::needless_pass_by_value, reason = "Tauri hands commands owned arguments")]

use std::sync::Arc;

use dainty_launcher_kit::bridge::checks;
use dainty_launcher_kit::bridge::spec::{
    EXTERNAL_TARGETS, HOST_ACTIONS, HOST_FOLDERS, HOST_SERVER_COMPONENTS, INSTALL_MODES, REALM_KINDS,
};
use dainty_launcher_kit::bridge::types::{
    ActionResult, AdminOutcome, Branding, EpicLibraryCheck, EpicView, ExternalTarget, GameVersionList, GuardedList,
    GuardedOutcome, HistoryOutcome, HostAction, HostFolder, HostKit, HostServerView, InstallMode, InstallOutcome,
    InviteCheck, ModuleCommandOutcome, ModuleListOutcome, ModuleOutcome, NewsItem, RealmKind, RealmOutcome, RegionPing,
    RegisterOutcome, RemoteExecOutcome, RemoteInviteOutcome, RemoteStatusOutcome, RevertApplyOutcome,
    RevertPlanOutcome, ServerConfigOutcome, ServerDoctorOutcome, ServerInviteListOutcome, Settings, Snapshot,
    TaskProgress,
};
use dainty_launcher_kit::controller::Controller;
use dainty_launcher_kit::controller::install::EpicGate;
use dainty_launcher_kit::platform::{clipboard, epic_library, epic_session::Epic};
use serde::Serialize;
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::backend::Backend;
use crate::{epic_login, updater};

/// The error side of the asynchronous commands: Tauri wants a `Result` from those that borrow state, and none
/// of them fails as a call (failures are answers).
#[derive(Debug, Serialize)]
pub enum Never {}

type Answer<T> = Result<T, Never>;
type Ctl<'a> = State<'a, Controller>;

#[tauri::command]
pub fn get_snapshot(c: Ctl<'_>) -> Snapshot {
    c.snapshot()
}

#[tauri::command]
pub fn check_invite(c: Ctl<'_>, text: String) -> InviteCheck {
    match checks::invite_text(&text) {
        Ok(text) => c.check_invite(text),
        Err(error) => InviteCheck::Err(error),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn submit_invite(
    c: Ctl<'_>,
    text: String,
    accept_new_certificate: bool,
    carry_account: bool,
) -> Answer<ActionResult> {
    Ok(match checks::invite_text(&text) {
        Ok(text) => c.submit_invite(text, accept_new_certificate, carry_account).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn retry_connect(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.connect().await)
}

#[tauri::command]
pub async fn forget_server(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.forget_server().await)
}

#[tauri::command]
pub async fn switch_server(c: Ctl<'_>, id: String) -> Answer<ActionResult> {
    Ok(match checks::server_id(&id) {
        Ok(id) => c.switch_server(id).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn remove_saved_server(c: Ctl<'_>, id: String) -> Answer<ActionResult> {
    Ok(match checks::server_id(&id) {
        Ok(id) => c.remove_saved_server(id).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn register(c: Ctl<'_>, username: String) -> Answer<RegisterOutcome> {
    Ok(match checks::username(&username) {
        Ok(name) => c.register(name).await,
        Err(error) => RegisterOutcome::Err(error),
    })
}

#[tauri::command]
pub async fn use_existing_key(c: Ctl<'_>, text: String) -> Answer<ActionResult> {
    Ok(match checks::key_text(&text) {
        Ok(text) => c.use_existing_key(text).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn import_key_file(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.import_key_file().await)
}

#[tauri::command]
pub async fn save_key_backup(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.save_key_backup().await)
}

#[tauri::command]
pub async fn dismiss_backup_offer(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.dismiss_backup_offer().await)
}

#[tauri::command]
pub async fn choose_game_folder(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.choose_game_folder().await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn use_game_path(c: Ctl<'_>, game_path: String) -> Answer<ActionResult> {
    Ok(match checks::game_path(&game_path) {
        Ok(path) => c.use_game_path(path).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn use_detected_game(c: Ctl<'_>, index: Value) -> Answer<ActionResult> {
    Ok(match checks::detected_index(&index) {
        Ok(index) => c.use_detected_game(index).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn verify_game(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.verify_game().await)
}

#[tauri::command]
pub async fn revived_install(c: Ctl<'_>, mode: Value) -> Answer<ActionResult> {
    Ok(match checks::name::<InstallMode>(&mode, INSTALL_MODES) {
        Ok(mode) => c.revived_install(mode).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn revived_choose_folder(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.revived_choose_folder().await)
}

#[tauri::command]
pub async fn revived_use_detected(c: Ctl<'_>, index: Value) -> Answer<ActionResult> {
    Ok(match checks::detected_index(&index).map(u32::try_from) {
        Ok(Ok(index)) => c.revived_use_detected(index).await,
        Ok(Err(_)) => ActionResult::err(dainty_launcher_kit::bridge::types::ErrorCode::FolderInvalid),
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn trust_runtime(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.trust_runtime().await)
}

#[tauri::command]
pub async fn install_runtime(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.install_runtime().await)
}

#[tauri::command]
pub async fn choose_local_runtime(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.choose_local_runtime().await)
}

#[tauri::command]
pub async fn clear_local_runtime(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.clear_local_runtime().await)
}

#[tauri::command]
pub fn cancel_task(c: Ctl<'_>) {
    c.cancel_task();
}

#[tauri::command]
pub async fn play(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.play().await)
}

#[tauri::command]
pub fn leave_queue(c: Ctl<'_>) {
    c.leave_queue();
}

#[tauri::command]
pub async fn set_settings(c: Ctl<'_>, patch: Value) -> Answer<Settings> {
    Ok(c.set_settings(checks::settings_patch(&patch)).await)
}

// Async so it runs on Tauri's Tokio runtime: a sync command runs on the main thread, where the poll's spawn panics
#[tauri::command]
pub async fn set_status_polling(c: Ctl<'_>, on: bool) -> Answer<()> {
    c.set_status_polling(on);
    Ok(())
}

#[tauri::command]
pub async fn refresh_status(c: Ctl<'_>) -> Answer<()> {
    c.refresh_status().await;
    Ok(())
}

#[tauri::command]
pub fn get_news(c: Ctl<'_>) -> Vec<NewsItem> {
    c.news()
}

#[tauri::command]
pub fn get_branding(c: Ctl<'_>) -> Branding {
    c.branding()
}

#[tauri::command]
pub fn open_external(c: Ctl<'_>, target: Value) -> ActionResult {
    match checks::name::<ExternalTarget>(&target, EXTERNAL_TARGETS) {
        Ok(target) => c.open_external(target),
        Err(error) => ActionResult::Err(error),
    }
}

#[tauri::command]
pub fn open_game_folder(c: Ctl<'_>) {
    c.open_game_folder();
}

#[tauri::command]
pub fn open_logs_folder(c: Ctl<'_>) {
    c.open_logs_folder();
}

#[tauri::command]
pub async fn logout(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.logout().await)
}

#[tauri::command]
pub async fn install_update(app: AppHandle, c: Ctl<'_>) -> Answer<()> {
    updater::install(&app, &c).await;
    Ok(())
}

#[tauri::command]
pub async fn host_get(c: Ctl<'_>) -> Answer<HostKit> {
    Ok(c.host_kit().await)
}

#[tauri::command]
pub async fn host_choose_workspace(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_choose_workspace().await)
}

#[tauri::command]
pub async fn host_choose_dumper(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_choose_dumper().await)
}

#[tauri::command]
pub async fn host_import_gamedata(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_import_gamedata().await)
}

#[tauri::command]
pub async fn host_import_sdk_folder(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_import_sdk_folder().await)
}

#[tauri::command]
pub async fn host_download_project(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_download_project().await)
}

#[tauri::command]
pub async fn host_action(c: Ctl<'_>, action: Value) -> Answer<ActionResult> {
    Ok(match checks::name::<HostAction>(&action, HOST_ACTIONS) {
        Ok(action) => c.host_action(action).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub fn host_open(c: Ctl<'_>, folder: Value) -> ActionResult {
    match checks::name::<HostFolder>(&folder, HOST_FOLDERS) {
        Ok(folder) => c.host_open(folder),
        Err(error) => ActionResult::Err(error),
    }
}

#[tauri::command]
pub fn host_server_get(c: Ctl<'_>) -> HostServerView {
    c.host_server_view()
}

#[tauri::command]
pub async fn host_server_start(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_server_start().await)
}

#[tauri::command]
pub async fn host_server_stop(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_server_stop().await)
}

#[tauri::command]
pub async fn host_server_restart(c: Ctl<'_>, name: Value) -> Answer<ActionResult> {
    Ok(match checks::listed_name(&name, HOST_SERVER_COMPONENTS) {
        Ok(name) => c.host_server_restart(name).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn host_setup(c: Ctl<'_>, request: Value) -> Answer<ActionResult> {
    Ok(match checks::host_setup(&request) {
        Ok(request) => c.host_setup(request).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn host_create_invite(c: Ctl<'_>, for_name: Option<Value>) -> Answer<ActionResult> {
    Ok(match checks::invite_for(for_name.as_ref()) {
        Ok(name) => c.host_create_invite(name).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn host_update(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.host_update().await)
}

#[tauri::command]
pub async fn host_set_multipliers(c: Ctl<'_>, multipliers: Value) -> Answer<ActionResult> {
    Ok(match checks::multipliers(&multipliers) {
        Ok(set) => c.host_set_multipliers(set).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub fn copy_text(text: String) -> bool {
    checks::copy_text(&text) && clipboard::write_text(&text).is_ok()
}

#[tauri::command]
pub fn take_invite_link(backend: State<'_, Arc<Backend>>) -> Option<String> {
    backend.take_invite_link()
}

#[tauri::command]
pub fn game_log_get(c: Ctl<'_>) -> Vec<String> {
    c.game_log()
}

#[tauri::command]
pub fn dismiss_notice(c: Ctl<'_>) {
    c.dismiss_notice();
}

#[tauri::command]
pub fn dismiss_error(c: Ctl<'_>) {
    c.dismiss_error();
}

#[tauri::command(rename_all = "camelCase")]
pub fn subscribe_progress(backend: State<'_, Arc<Backend>>, on_progress: Channel<TaskProgress>) {
    backend.subscribe(on_progress);
}

#[tauri::command]
pub async fn epic_sign_in_start(app: AppHandle, c: Ctl<'_>, epic: State<'_, Arc<Epic>>) -> Answer<ActionResult> {
    Ok(epic_login::sign_in(&app, Arc::clone(&epic), c.language()).await)
}

#[tauri::command]
pub async fn epic_sign_in_code(app: AppHandle, epic: State<'_, Arc<Epic>>, code: String) -> Answer<ActionResult> {
    let code = match checks::epic_code(&code) {
        Ok(code) => code,
        Err(error) => return Ok(ActionResult::Err(error)),
    };
    let result = epic.sign_in_pasted(code).await;
    if matches!(result, ActionResult::Ok) {
        epic_login::close(&app);
    }
    Ok(result)
}

#[tauri::command]
pub fn epic_get(epic: State<'_, Arc<Epic>>) -> EpicView {
    epic.view()
}

#[tauri::command]
pub async fn epic_sign_out(epic: State<'_, Arc<Epic>>) -> Answer<ActionResult> {
    Ok(epic.sign_out().await)
}

#[tauri::command]
pub async fn epic_library_check(gate: State<'_, Arc<EpicGate>>) -> Answer<EpicLibraryCheck> {
    Ok(match gate.epic.fresh_session().await {
        Ok(session) => epic_library::check(&gate.ownership, session.as_ref()).await,
        Err(error) => EpicLibraryCheck::Err(error),
    })
}

#[tauri::command]
pub async fn game_version_list(c: Ctl<'_>) -> Answer<GameVersionList> {
    Ok(c.game_version_list().await)
}

#[tauri::command]
pub async fn game_install_start(c: Ctl<'_>, gate: State<'_, Arc<EpicGate>>, version: String) -> Answer<InstallOutcome> {
    Ok(match checks::game_version(&version) {
        Ok(version) => c.game_install_start(version, Arc::clone(&gate)).await,
        Err(error) => InstallOutcome::Err(error),
    })
}

#[tauri::command]
pub fn game_install_pause(c: Ctl<'_>, version: String) -> ActionResult {
    match checks::game_version(&version) {
        Ok(version) => c.game_install_pause(version),
        Err(error) => ActionResult::Err(error),
    }
}

#[tauri::command]
pub fn game_install_resume(c: Ctl<'_>, version: String) -> ActionResult {
    match checks::game_version(&version) {
        Ok(version) => c.game_install_resume(version),
        Err(error) => ActionResult::Err(error),
    }
}

#[tauri::command]
pub fn game_install_cancel(c: Ctl<'_>, version: String) -> ActionResult {
    match checks::game_version(&version) {
        Ok(version) => c.game_install_cancel(version),
        Err(error) => ActionResult::Err(error),
    }
}

#[tauri::command]
pub fn game_install_get(c: Ctl<'_>, version: String) -> InstallOutcome {
    match checks::game_version(&version) {
        Ok(version) => c.game_install_get(version),
        Err(error) => InstallOutcome::Err(error),
    }
}

#[tauri::command]
pub async fn game_install_choose_folder(c: Ctl<'_>) -> Answer<ActionResult> {
    Ok(c.game_install_choose_folder().await)
}

#[tauri::command]
pub async fn game_install_discard(c: Ctl<'_>, version: String) -> Answer<ActionResult> {
    Ok(match checks::game_version(&version) {
        Ok(version) => c.game_install_discard(version).await,
        Err(error) => ActionResult::Err(error),
    })
}

#[tauri::command]
pub async fn region_ping(c: Ctl<'_>) -> Answer<RegionPing> {
    Ok(c.region_ping().await)
}

#[tauri::command]
pub async fn module_list(backend: State<'_, Arc<Backend>>) -> Answer<ModuleListOutcome> {
    Ok(match backend.panel() {
        Some(panel) => panel.list().await,
        None => ModuleListOutcome::Err(checks::not_in_this_build("module_list")),
    })
}

#[tauri::command]
pub async fn module_get(backend: State<'_, Arc<Backend>>, module: String) -> Answer<ModuleOutcome> {
    Ok(match backend.panel() {
        Some(panel) => panel.get(module).await,
        None => ModuleOutcome::Err(checks::not_in_this_build("module_get")),
    })
}

#[tauri::command]
pub async fn module_run_command(
    backend: State<'_, Arc<Backend>>,
    module: String,
    command: String,
    input: String,
) -> Answer<ModuleCommandOutcome> {
    Ok(match backend.panel() {
        Some(panel) => panel.run(module, command, input).await,
        None => ModuleCommandOutcome::Err(checks::not_in_this_build("module_run_command")),
    })
}

#[tauri::command]
pub async fn module_set_settings(
    backend: State<'_, Arc<Backend>>,
    module: String,
    values: String,
) -> Answer<ModuleOutcome> {
    Ok(match backend.panel() {
        Some(panel) => panel.set_settings(module, values).await,
        None => ModuleOutcome::Err(checks::not_in_this_build("module_set_settings")),
    })
}

#[tauri::command]
pub async fn module_load(backend: State<'_, Arc<Backend>>, module: String) -> Answer<ModuleOutcome> {
    Ok(match backend.panel() {
        Some(panel) => panel.load(module).await,
        None => ModuleOutcome::Err(checks::not_in_this_build("module_load")),
    })
}

#[tauri::command]
pub async fn module_unload(backend: State<'_, Arc<Backend>>, module: String) -> Answer<ModuleOutcome> {
    Ok(match backend.panel() {
        Some(panel) => panel.unload(module).await,
        None => ModuleOutcome::Err(checks::not_in_this_build("module_unload")),
    })
}

#[tauri::command]
pub async fn realm_host(c: Ctl<'_>, kind: Value) -> Answer<RealmOutcome> {
    Ok(match checks::name::<RealmKind>(&kind, REALM_KINDS) {
        Ok(kind) => c.realm_host(kind).await,
        Err(error) => RealmOutcome::Err(error),
    })
}

#[tauri::command]
pub async fn realm_join(c: Ctl<'_>, realm: String) -> Answer<RealmOutcome> {
    Ok(match checks::realm_id(&realm) {
        Ok(realm) => c.realm_join(realm).await,
        Err(error) => RealmOutcome::Err(error),
    })
}

#[tauri::command]
pub async fn realm_stop(c: Ctl<'_>) -> Answer<RealmOutcome> {
    Ok(c.realm_stop().await)
}

#[tauri::command]
pub async fn realm_get(c: Ctl<'_>) -> Answer<RealmOutcome> {
    Ok(c.realm_get().await)
}

/// The manager's commands: checked, then the controller's in-process host kit; refused in any other build.
macro_rules! manager {
    ($command:literal, $outcome:ident, [$($arg:ident),*], |$c:ident| $body:expr) => {{
        #[cfg(feature = "server-gui")]
        {
            Ok($body)
        }
        #[cfg(not(feature = "server-gui"))]
        {
            let _ = (&$c, $(&$arg),*);
            Ok($outcome::Err(checks::not_in_this_build($command)))
        }
    }};
}

/// An argument check's refusal as the command's answer.
#[cfg(feature = "server-gui")]
macro_rules! checked {
    ($check:expr, $outcome:ident) => {
        match $check {
            Ok(value) => value,
            Err(error) => return Ok($outcome::Err(error)),
        }
    };
}

#[tauri::command]
pub async fn history_query(c: Ctl<'_>, filter: Value) -> Answer<HistoryOutcome> {
    manager!("history_query", HistoryOutcome, [filter], |c| {
        let filter = checked!(checks::history_filter(&filter), HistoryOutcome);
        c.history_query(filter).await
    })
}

#[tauri::command]
pub async fn history_revert_dry_run(c: Ctl<'_>, selection: Value) -> Answer<RevertPlanOutcome> {
    manager!("history_revert_dry_run", RevertPlanOutcome, [selection], |c| {
        let selection = checked!(checks::revert_selection(&selection), RevertPlanOutcome);
        c.history_revert_dry_run(selection).await
    })
}

#[tauri::command]
pub async fn history_revert_apply(c: Ctl<'_>, plan: String) -> Answer<RevertApplyOutcome> {
    manager!("history_revert_apply", RevertApplyOutcome, [plan], |c| {
        let plan = checked!(checks::revert_plan(&plan), RevertApplyOutcome).to_owned();
        c.history_revert_apply(plan).await
    })
}

#[tauri::command]
pub async fn guarded_list(c: Ctl<'_>) -> Answer<GuardedList> {
    Ok(c.guarded_list().await)
}

#[tauri::command]
pub async fn guarded_approve(c: Ctl<'_>, id: String) -> Answer<GuardedOutcome> {
    Ok(match checks::guarded_id(&id) {
        Ok(id) => c.guarded_approve(id).await,
        Err(error) => GuardedOutcome::Err(error),
    })
}

#[tauri::command]
pub async fn guarded_cancel(c: Ctl<'_>, id: String) -> Answer<GuardedOutcome> {
    Ok(match checks::guarded_id(&id) {
        Ok(id) => c.guarded_cancel(id).await,
        Err(error) => GuardedOutcome::Err(error),
    })
}

#[tauri::command]
pub async fn server_doctor(c: Ctl<'_>) -> Answer<ServerDoctorOutcome> {
    manager!("server_doctor", ServerDoctorOutcome, [], |c| c.server_doctor().await)
}

#[tauri::command]
pub async fn server_backup(c: Ctl<'_>) -> Answer<ActionResult> {
    manager!("server_backup", ActionResult, [], |c| c.server_backup().await)
}

#[tauri::command]
pub async fn server_config_get(c: Ctl<'_>) -> Answer<ServerConfigOutcome> {
    manager!("server_config_get", ServerConfigOutcome, [], |c| c
        .server_config_get()
        .await)
}

#[tauri::command]
pub async fn server_config_save(c: Ctl<'_>, config: Value) -> Answer<ServerConfigOutcome> {
    manager!("server_config_save", ServerConfigOutcome, [config], |c| {
        let form = checked!(checks::server_config(&config), ServerConfigOutcome);
        c.server_config_save(form).await
    })
}

#[tauri::command]
pub async fn server_migrate_mongo(c: Ctl<'_>) -> Answer<ActionResult> {
    manager!("server_migrate_mongo", ActionResult, [], |c| c
        .server_migrate_mongo()
        .await)
}

#[tauri::command]
pub async fn server_confirm_fresh_start(c: Ctl<'_>) -> Answer<ServerConfigOutcome> {
    manager!("server_confirm_fresh_start", ServerConfigOutcome, [], |c| {
        c.server_confirm_fresh_start().await
    })
}

#[tauri::command]
pub async fn server_invite_list(c: Ctl<'_>) -> Answer<ServerInviteListOutcome> {
    manager!("server_invite_list", ServerInviteListOutcome, [], |c| c
        .server_invite_list()
        .await)
}

#[tauri::command]
pub async fn admin_action(backend: State<'_, Arc<Backend>>, request: Value) -> Answer<AdminOutcome> {
    let Some(panel) = backend.panel() else {
        return Ok(AdminOutcome::Err(checks::not_in_this_build("admin_action")));
    };
    Ok(match checks::admin_request(&request) {
        Ok(r) => panel.admin(r.action, r.player, r.reason, r.until, r.module).await,
        Err(error) => AdminOutcome::Err(error),
    })
}

#[tauri::command]
pub async fn remote_status(c: Ctl<'_>) -> Answer<RemoteStatusOutcome> {
    manager!("remote_status", RemoteStatusOutcome, [], |c| c.remote_status().await)
}

#[tauri::command]
pub async fn remote_init_controller(c: Ctl<'_>, request: Value) -> Answer<RemoteStatusOutcome> {
    manager!("remote_init_controller", RemoteStatusOutcome, [request], |c| {
        let request = checked!(checks::remote_init(&request), RemoteStatusOutcome);
        c.remote_init_controller(request).await
    })
}

#[tauri::command]
pub async fn remote_invite(c: Ctl<'_>, request: Value) -> Answer<RemoteInviteOutcome> {
    manager!("remote_invite", RemoteInviteOutcome, [request], |c| {
        let request = checked!(checks::remote_invite(&request), RemoteInviteOutcome);
        c.remote_invite(request).await
    })
}

#[tauri::command]
pub async fn remote_enroll(c: Ctl<'_>, request: Value) -> Answer<RemoteStatusOutcome> {
    manager!("remote_enroll", RemoteStatusOutcome, [request], |c| {
        let request = checked!(checks::remote_enroll(&request), RemoteStatusOutcome);
        c.remote_enroll(request).await
    })
}

#[tauri::command]
pub async fn remote_revoke(c: Ctl<'_>, node: String) -> Answer<RemoteStatusOutcome> {
    manager!("remote_revoke", RemoteStatusOutcome, [node], |c| {
        let node = checked!(checks::remote_node(&node), RemoteStatusOutcome).to_owned();
        c.remote_revoke(node).await
    })
}

#[tauri::command]
pub async fn remote_leave(c: Ctl<'_>) -> Answer<RemoteStatusOutcome> {
    manager!("remote_leave", RemoteStatusOutcome, [], |c| c.remote_leave().await)
}

#[tauri::command]
pub async fn remote_exec(c: Ctl<'_>, request: Value) -> Answer<RemoteExecOutcome> {
    manager!("remote_exec", RemoteExecOutcome, [request], |c| {
        let request = checked!(checks::remote_exec(&request), RemoteExecOutcome);
        c.remote_exec(request).await
    })
}
