//! One command per `LauncherApi` method of Reforged's `preload.ts` (row L7, RA2), named after its IPC
//! channel without `rr:` (in snake_case). Each checks its arguments again, whatever the isolation hook let
//! through: the hook guards the page, these guard the backend (`ipc-validate.ts` and the handlers in
//! `main.ts`, with the same refusals). Then it hands over to the controller (RA13, RA12), whose stubs
//! answer until those rows land; this file does not change when they do. The DaintyLass commands at the
//! end (DF8) are refused by `planned` until their rows replace them.

use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use super::backend::Backend;
use super::spec::{
    Arg, COMMANDS, COPY_TEXT_MAX, DETECTED_GAME_MAX, GAME_PATH_MAX, INVITE_FOR_MAX, INVITE_MAX, KEY_TEXT_MAX,
    PUBLIC_HOST_MAX, SERVER_NAME_MAX, USERNAME_MAX, fits, is_server_id, serves,
};
use super::types::{
    ActionResult, Branding, EpicLibraryCheck, ErrorCode, ExternalTarget, GameVersionList, HostAction, HostFolder,
    HostKit, HostMultipliers, HostServerMode, HostServerView, HostSetupRequest, InstallMode, InviteCheck,
    LauncherError, MAX_MULTIPLIER, NewsItem, RegionPing, RegisterOutcome, Settings, SettingsPatch, Snapshot,
    TaskProgress,
};
use crate::controller::ControllerHandle;
use crate::core::text::{js_length, js_trim};
use crate::platform::epic_library::{self, EpicOwnership};

type Backed<'a> = State<'a, Backend>;

/// The controller, owned (async commands cannot borrow state without returning a `Result`).
fn controller(app: &AppHandle) -> ControllerHandle {
    app.state::<ControllerHandle>().inner().clone()
}

/// A name from one of the fixed lists (`pick` in `ipc-validate.ts`): only the exact strings.
fn name<T: serde::de::DeserializeOwned>(value: Value) -> Option<T> {
    value.is_string().then(|| serde_json::from_value(value).ok()).flatten()
}

/// `useDetectedGame`: an integer from 0 to 15.
fn detected_index(value: &Value) -> Option<usize> {
    let index = value.as_f64()?;
    (index.fract() == 0.0 && (0.0..=f64::from(DETECTED_GAME_MAX)).contains(&index)).then_some(index as usize)
}

/// Text the host types (a server name, who an invite is for): trimmed, at most `max` long, no control
/// characters. It goes to the host kit as one argument, never through a shell.
fn plain_text(value: &Value, max: usize) -> Option<String> {
    let text = js_trim(value.as_str()?);
    (js_length(text) <= max && !text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')).then(|| text.to_owned())
}

static PUBLIC_HOST: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?$").ok());

/// `hostSetupRequest`: the setup form, or None when any field is wrong.
fn host_setup_request(value: &Value) -> Option<HostSetupRequest> {
    let form = value.as_object()?;
    let name = plain_text(form.get("name")?, SERVER_NAME_MAX).filter(|name| !name.is_empty())?;
    let mode = match form.get("mode")?.as_str()? {
        "private" => HostServerMode::Private,
        "public" => HostServerMode::Public,
        _ => return None,
    };
    if mode == HostServerMode::Private {
        return Some(HostSetupRequest {
            name,
            mode,
            public_host: None,
        });
    }
    let host = js_trim(form.get("publicHost").and_then(Value::as_str).unwrap_or(""));
    let valid = js_length(host) <= PUBLIC_HOST_MAX && PUBLIC_HOST.as_ref().is_some_and(|re| re.is_match(host));
    valid.then(|| HostSetupRequest {
        name,
        mode,
        public_host: Some(host.to_owned()),
    })
}

/// `hostMultipliers`: Ok(None) turns them off (null, or 1 and 1), Err refuses the form.
fn host_multipliers(value: &Value) -> Result<Option<HostMultipliers>, ()> {
    if value.is_null() {
        return Ok(None);
    }
    let form = value.as_object().ok_or(())?;
    let multiplier = |field: &str| {
        let value = form.get(field).and_then(Value::as_f64).ok_or(())?;
        if value.is_finite() && (1.0..=MAX_MULTIPLIER).contains(&value) {
            Ok((value * 100.0).round() / 100.0)
        } else {
            Err(())
        }
    };
    let (xp, rewards) = (multiplier("xp")?, multiplier("rewards")?);
    Ok((xp != 1.0 || rewards != 1.0).then_some(HostMultipliers { xp, rewards }))
}

/// `inviteFor`: who an invite is for, optional. Ok(None) for nobody, Err refuses it.
fn invite_for(value: Option<&Value>) -> Result<Option<String>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(value) => plain_text(value, INVITE_FOR_MAX)
            .map(|text| (!text.is_empty()).then_some(text))
            .ok_or(()),
    }
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle) -> Snapshot {
    controller(&app).snapshot()
}

/// Whether an invite can be joined. A public invite whose certificate differs from the one this PC's
/// key for that server belongs to says so, and a private server this PC knows at another address too.
#[tauri::command]
pub fn check_invite(text: String, app: AppHandle) -> InviteCheck {
    if !fits(&text, INVITE_MAX) {
        return InviteCheck::Err(LauncherError::new(ErrorCode::InviteInvalidFormat));
    }
    controller(&app).check_invite(&text)
}

#[tauri::command]
pub async fn submit_invite(
    text: String,
    accept_new_certificate: bool,
    carry_account: bool,
    app: AppHandle,
) -> ActionResult {
    if !fits(&text, INVITE_MAX) {
        return ActionResult::err(ErrorCode::InviteInvalidFormat);
    }
    controller(&app)
        .submit_invite(&text, accept_new_certificate, carry_account)
        .await
}

#[tauri::command]
pub async fn retry_connect(app: AppHandle) -> ActionResult {
    controller(&app).connect().await
}

#[tauri::command]
pub async fn forget_server(app: AppHandle) -> ActionResult {
    controller(&app).forget_server().await
}

#[tauri::command]
pub async fn switch_server(id: String, app: AppHandle) -> ActionResult {
    if !is_server_id(&id) {
        return ActionResult::err(ErrorCode::InviteInvalidFormat);
    }
    controller(&app).switch_server(&id).await
}

#[tauri::command]
pub async fn remove_saved_server(id: String, app: AppHandle) -> ActionResult {
    if !is_server_id(&id) {
        return ActionResult::err(ErrorCode::InviteInvalidFormat);
    }
    controller(&app).remove_saved_server(&id).await
}

#[tauri::command]
pub async fn register(username: String, app: AppHandle) -> RegisterOutcome {
    if !fits(&username, USERNAME_MAX) {
        return RegisterOutcome::Err(LauncherError::new(ErrorCode::UsernameInvalid));
    }
    controller(&app).register(&username).await
}

#[tauri::command]
pub async fn use_existing_key(text: String, app: AppHandle) -> ActionResult {
    if !fits(&text, KEY_TEXT_MAX) {
        return ActionResult::err(ErrorCode::KeyInvalid);
    }
    controller(&app).use_existing_key(&text).await
}

#[tauri::command]
pub async fn import_key_file(app: AppHandle) -> ActionResult {
    controller(&app).import_key_file().await
}

#[tauri::command]
pub async fn save_key_backup(app: AppHandle) -> ActionResult {
    controller(&app).save_key_backup().await
}

#[tauri::command]
pub async fn dismiss_backup_offer(app: AppHandle) {
    controller(&app).dismiss_backup_offer().await;
}

#[tauri::command]
pub async fn choose_game_folder(app: AppHandle) -> ActionResult {
    controller(&app).choose_game_folder().await
}

#[tauri::command]
pub async fn use_game_path(game_path: String, app: AppHandle) -> ActionResult {
    if !fits(&game_path, GAME_PATH_MAX) {
        return ActionResult::err(ErrorCode::FolderInvalid);
    }
    controller(&app).use_game_path(&game_path).await
}

#[tauri::command]
pub async fn use_detected_game(index: Value, app: AppHandle) -> ActionResult {
    match detected_index(&index) {
        Some(index) => controller(&app).use_detected_game(index).await,
        None => ActionResult::err(ErrorCode::FolderInvalid),
    }
}

#[tauri::command]
pub async fn verify_game(app: AppHandle) -> ActionResult {
    controller(&app).verify_game().await
}

/// Dauntless 1.4.4 (legacy edition): `install` or `repair`, nothing else.
#[tauri::command]
pub async fn revived_install(mode: Value, app: AppHandle) -> ActionResult {
    match name::<InstallMode>(mode) {
        Some(mode) => controller(&app).revived_install(mode).await,
        None => ActionResult::err(ErrorCode::Unknown),
    }
}

#[tauri::command]
pub async fn revived_choose_folder(app: AppHandle) -> ActionResult {
    controller(&app).revived_choose_folder().await
}

#[tauri::command]
pub async fn revived_use_detected(index: Value, app: AppHandle) -> ActionResult {
    match detected_index(&index) {
        Some(index) => controller(&app).revived_use_detected(index as u32).await,
        None => ActionResult::err(ErrorCode::FolderInvalid),
    }
}

#[tauri::command]
pub async fn trust_runtime(app: AppHandle) -> ActionResult {
    controller(&app).trust_runtime().await
}

#[tauri::command]
pub async fn install_runtime(app: AppHandle) -> ActionResult {
    controller(&app).install_runtime().await
}

#[tauri::command]
pub async fn choose_local_runtime(app: AppHandle) -> ActionResult {
    controller(&app).choose_local_runtime().await
}

#[tauri::command]
pub async fn clear_local_runtime(app: AppHandle) -> ActionResult {
    controller(&app).clear_local_runtime().await
}

#[tauri::command]
pub fn cancel_task(app: AppHandle) {
    controller(&app).cancel_task();
}

#[tauri::command]
pub async fn play(app: AppHandle) -> ActionResult {
    controller(&app).play().await
}

/// An unknown field or a wrong value refuses the whole patch, which then changes nothing, as today.
#[tauri::command]
pub async fn set_settings(patch: Value, app: AppHandle) -> Settings {
    let patch = serde_json::from_value::<SettingsPatch>(patch).unwrap_or_default();
    controller(&app).set_settings(patch).await
}

#[tauri::command]
pub fn set_status_polling(on: bool, app: AppHandle) {
    controller(&app).set_status_polling(on);
}

#[tauri::command]
pub async fn refresh_status(app: AppHandle) {
    controller(&app).poll_status().await;
}

#[tauri::command]
pub fn get_news(app: AppHandle) -> Vec<NewsItem> {
    controller(&app).news()
}

#[tauri::command]
pub fn get_branding(app: AppHandle) -> Branding {
    controller(&app).branding()
}

#[tauri::command]
pub fn open_external(target: Value, app: AppHandle) -> ActionResult {
    match name::<ExternalTarget>(target) {
        Some(target) => controller(&app).open_external(target),
        None => ActionResult::err(ErrorCode::Unknown),
    }
}

#[tauri::command]
pub fn open_game_folder(app: AppHandle) {
    controller(&app).open_game_folder();
}

#[tauri::command]
pub fn open_logs_folder(app: AppHandle) {
    controller(&app).open_logs_folder();
}

#[tauri::command]
pub async fn logout(app: AppHandle) -> ActionResult {
    controller(&app).logout().await
}

/// Installs the update the player was offered (L46); nothing when none waits or a task or the game runs.
#[tauri::command]
pub async fn install_update(app: AppHandle) {
    let controller = controller(&app);
    crate::platform::updater::install(app, controller).await;
}

/// The page asks when it opens, after an action and on "Check again": the requirements are looked at
/// afresh.
#[tauri::command]
pub async fn host_get(app: AppHandle) -> HostKit {
    controller(&app).host_kit().await
}

#[tauri::command]
pub async fn host_choose_workspace(app: AppHandle) -> ActionResult {
    controller(&app).host_choose_workspace().await
}

#[tauri::command]
pub async fn host_choose_dumper(app: AppHandle) -> ActionResult {
    controller(&app).host_choose_dumper().await
}

#[tauri::command]
pub async fn host_import_gamedata(app: AppHandle) -> ActionResult {
    controller(&app).host_import_gamedata().await
}

#[tauri::command]
pub async fn host_import_sdk_folder(app: AppHandle) -> ActionResult {
    controller(&app).host_import_sdk_folder().await
}

#[tauri::command]
pub async fn host_download_project(app: AppHandle) -> ActionResult {
    controller(&app).host_download_project().await
}

#[tauri::command]
pub async fn host_action(action: Value, app: AppHandle) -> ActionResult {
    match name::<HostAction>(action) {
        Some(action) => controller(&app).host_action(action).await,
        None => ActionResult::err(ErrorCode::Unknown),
    }
}

#[tauri::command]
pub fn host_open(folder: Value, app: AppHandle) -> ActionResult {
    match name::<HostFolder>(folder) {
        Some(folder) => controller(&app).host_open(folder),
        None => ActionResult::err(ErrorCode::Unknown),
    }
}

#[tauri::command]
pub fn host_server_get(app: AppHandle) -> HostServerView {
    controller(&app).host_server_view()
}

#[tauri::command]
pub async fn host_server_start(app: AppHandle) -> ActionResult {
    controller(&app).host_server_start().await
}

#[tauri::command]
pub async fn host_server_stop(app: AppHandle) -> ActionResult {
    controller(&app).host_server_stop().await
}

#[tauri::command]
pub async fn host_server_restart(name: Value, app: AppHandle) -> ActionResult {
    match name
        .as_str()
        .filter(|n| crate::bridge::spec::HOST_SERVER_COMPONENTS.contains(n))
    {
        Some(name) => controller(&app).host_server_restart(name).await,
        None => ActionResult::err(ErrorCode::Unknown),
    }
}

#[tauri::command]
pub async fn host_setup(request: Value, app: AppHandle) -> ActionResult {
    match host_setup_request(&request) {
        Some(request) => controller(&app).host_setup(request).await,
        None => ActionResult::Err(LauncherError::with_detail(ErrorCode::HostFailed, "setup_form")),
    }
}

#[tauri::command]
pub async fn host_create_invite(for_name: Option<Value>, app: AppHandle) -> ActionResult {
    match invite_for(for_name.as_ref()) {
        Ok(for_name) => controller(&app).host_create_invite(for_name).await,
        Err(()) => ActionResult::err(ErrorCode::Unknown),
    }
}

#[tauri::command]
pub async fn host_update(app: AppHandle) -> ActionResult {
    controller(&app).host_update().await
}

#[tauri::command]
pub async fn host_set_multipliers(multipliers: Value, app: AppHandle) -> ActionResult {
    match host_multipliers(&multipliers) {
        Ok(multipliers) => controller(&app).host_set_multipliers(multipliers).await,
        Err(()) => ActionResult::Err(LauncherError::with_detail(ErrorCode::HostFailed, "multipliers_form")),
    }
}

/// The page may not use the clipboard itself (RD22): an invite link is copied here. True when copied.
#[tauri::command]
pub fn copy_text(text: String) -> bool {
    fits(&text, COPY_TEXT_MAX) && crate::platform::clipboard::write_text(&text).is_ok()
}

#[tauri::command]
pub fn take_invite_link(backend: Backed<'_>) -> Option<String> {
    backend.take_invite_link()
}

#[tauri::command]
pub fn dismiss_notice(app: AppHandle) {
    controller(&app).set_notice(None);
}

#[tauri::command]
pub fn dismiss_error(app: AppHandle) {
    controller(&app).dismiss_error();
}

#[tauri::command]
pub fn subscribe_progress(on_progress: Channel<TaskProgress>, backend: Backed<'_>) {
    backend.subscribe_progress(on_progress);
}

/// An argument against its shape in the spec, as the hook checks it (`fits` in `isolation/hook.js`): the
/// planned commands' check until the rows that fill them bring their own.
fn arg_accepts(arg: &Arg, value: Option<&Value>) -> bool {
    match (arg, value) {
        (Arg::Text { max }, Some(Value::String(text))) => fits(text, *max),
        (Arg::OptionalText { .. }, None | Some(Value::Null)) => true,
        (Arg::OptionalText { max }, Some(Value::String(text))) => fits(text, *max),
        (Arg::Flag, Some(Value::Bool(_))) => true,
        (Arg::Index { max }, Some(value)) => value
            .as_f64()
            .is_some_and(|index| index.fract() == 0.0 && (0.0..=f64::from(*max)).contains(&index)),
        (Arg::Name { names }, Some(Value::String(name))) => names.contains(&name.as_str()),
        (Arg::Number { min, max }, Some(value)) => value
            .as_f64()
            .is_some_and(|number| number.is_finite() && (*min..=*max).contains(&number)),
        (Arg::Object { nullable, .. }, Some(Value::Null)) => *nullable,
        (Arg::Object { fields, .. }, Some(Value::Object(object))) => {
            object.keys().all(|key| fields.iter().any(|(name, _)| name == key))
                && fields.iter().all(|(name, arg)| arg_accepts(arg, object.get(*name)))
        }
        _ => false,
    }
}

/// A command registered ahead of the row that fills it (DF8, `spec::PLANNED`): arguments of the wrong
/// shape are refused as `unknown`, as the other commands refuse them, then the call is refused as
/// `not_in_this_build` or `not_implemented` with the command's name as the detail. The page shows the code.
fn planned(command: &str, args: &[&Value]) -> ActionResult {
    let shapes = COMMANDS
        .iter()
        .find(|listed| listed.name == command)
        .map(|listed| listed.args);
    let fit = shapes.is_some_and(|shapes| {
        shapes.len() == args.len()
            && shapes
                .iter()
                .zip(args)
                .all(|((_, arg), value)| arg_accepts(arg, Some(value)))
    });
    if !fit {
        return ActionResult::err(ErrorCode::Unknown);
    }
    let code = if serves(command) {
        ErrorCode::NotImplemented
    } else {
        ErrorCode::NotInThisBuild
    };
    ActionResult::Err(LauncherError::with_detail(code, command))
}

#[tauri::command]
pub fn epic_sign_in_start() -> ActionResult {
    planned("epic_sign_in_start", &[])
}

#[tauri::command]
pub fn epic_sign_in_code(code: Value) -> ActionResult {
    planned("epic_sign_in_code", &[&code])
}

#[tauri::command]
pub fn epic_get() -> ActionResult {
    planned("epic_get", &[])
}

#[tauri::command]
pub fn epic_sign_out() -> ActionResult {
    planned("epic_sign_out", &[])
}

/// The signed-in Epic session, from the keystore. DE2 keeps it there; until it does, nobody is signed in.
fn epic_session(app: &AppHandle) -> Option<epic::auth::Session> {
    let _ = app;
    None
}

/// Whether the signed-in Epic account owns Dauntless (DE3), cached a while. The session never crosses.
#[tauri::command]
pub async fn epic_library_check(app: AppHandle) -> EpicLibraryCheck {
    let ownership = app.state::<Arc<EpicOwnership>>().inner().clone();
    let session = epic_session(&app);
    epic_library::check(&ownership, session.as_ref()).await
}

/// The installable versions (DE4): the compiled-in client profiles and the joined servers', each Epic
/// build with its sizes from its manifest.
#[tauri::command]
pub async fn game_version_list(app: AppHandle) -> GameVersionList {
    controller(&app).game_version_list().await
}

#[tauri::command]
pub fn game_install_start(version: Value) -> ActionResult {
    planned("game_install_start", &[&version])
}

#[tauri::command]
pub fn game_install_pause(version: Value) -> ActionResult {
    planned("game_install_pause", &[&version])
}

#[tauri::command]
pub fn game_install_resume(version: Value) -> ActionResult {
    planned("game_install_resume", &[&version])
}

#[tauri::command]
pub fn game_install_cancel(version: Value) -> ActionResult {
    planned("game_install_cancel", &[&version])
}

#[tauri::command]
pub fn game_install_get(version: Value) -> ActionResult {
    planned("game_install_get", &[&version])
}

/// The joined server's regions, nearest first, reported to it as well (DN7).
#[tauri::command]
pub async fn region_ping(app: AppHandle) -> RegionPing {
    controller(&app).region_ping().await
}

#[tauri::command]
pub fn module_list() -> ActionResult {
    planned("module_list", &[])
}

#[tauri::command]
pub fn module_get(module: Value) -> ActionResult {
    planned("module_get", &[&module])
}

#[tauri::command]
pub fn module_run_command(module: Value, command: Value, input: Value) -> ActionResult {
    planned("module_run_command", &[&module, &command, &input])
}

#[tauri::command]
pub fn module_set_settings(module: Value, values: Value) -> ActionResult {
    planned("module_set_settings", &[&module, &values])
}

#[tauri::command]
pub fn module_load(module: Value) -> ActionResult {
    planned("module_load", &[&module])
}

#[tauri::command]
pub fn module_unload(module: Value) -> ActionResult {
    planned("module_unload", &[&module])
}

#[tauri::command]
pub fn realm_host(kind: Value) -> ActionResult {
    planned("realm_host", &[&kind])
}

#[tauri::command]
pub fn realm_join(realm: Value) -> ActionResult {
    planned("realm_join", &[&realm])
}

#[tauri::command]
pub fn realm_stop() -> ActionResult {
    planned("realm_stop", &[])
}

#[tauri::command]
pub fn realm_get() -> ActionResult {
    planned("realm_get", &[])
}

#[tauri::command]
pub fn history_query(filter: Value) -> ActionResult {
    planned("history_query", &[&filter])
}

#[tauri::command]
pub fn history_revert_dry_run(selection: Value) -> ActionResult {
    planned("history_revert_dry_run", &[&selection])
}

#[tauri::command]
pub fn history_revert_apply(plan: Value) -> ActionResult {
    planned("history_revert_apply", &[&plan])
}

#[tauri::command]
pub fn guarded_list() -> ActionResult {
    planned("guarded_list", &[])
}

#[tauri::command]
pub fn guarded_approve(id: Value) -> ActionResult {
    planned("guarded_approve", &[&id])
}

#[tauri::command]
pub fn guarded_cancel(id: Value) -> ActionResult {
    planned("guarded_cancel", &[&id])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn names_are_only_the_exact_strings() {
        assert_eq!(name::<HostFolder>(json!("dumps")), Some(HostFolder::Dumps));
        assert_eq!(
            name::<ExternalTarget>(json!("dumper7_zip")),
            Some(ExternalTarget::Dumper7Zip)
        );
        for bad in [
            json!("Dumps"),
            json!("dumps "),
            json!("__proto__"),
            json!("https://example.com"),
            json!(7),
            json!(null),
            json!(["dumps"]),
        ] {
            assert_eq!(name::<HostFolder>(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn detected_games_by_index_below_16() {
        assert_eq!(detected_index(&json!(0)), Some(0));
        assert_eq!(detected_index(&json!(15)), Some(15));
        assert_eq!(detected_index(&json!(15.0)), Some(15));
        for bad in [json!(16), json!(-1), json!(1.5), json!("1"), json!(null)] {
            assert_eq!(detected_index(&bad), None, "{bad}");
        }
    }

    #[test]
    fn the_setup_form_as_ipc_validate_checks_it() {
        let private = host_setup_request(&json!({ "name": "  Friends  ", "mode": "private", "publicHost": "x" }));
        assert_eq!(
            private,
            Some(HostSetupRequest {
                name: "Friends".into(),
                mode: HostServerMode::Private,
                public_host: None
            })
        );
        let public =
            host_setup_request(&json!({ "name": "Hunts", "mode": "public", "publicHost": " play.example.org " }));
        assert_eq!(
            public.and_then(|form| form.public_host),
            Some("play.example.org".into())
        );
        let long_host = format!("a{}a", "b".repeat(251));
        assert!(host_setup_request(&json!({ "name": "x", "mode": "public", "publicHost": long_host })).is_some());
        for bad in [
            json!({ "name": "", "mode": "private", "publicHost": null }),
            json!({ "name": "   ", "mode": "private", "publicHost": null }),
            json!({ "name": "a".repeat(61), "mode": "private", "publicHost": null }),
            json!({ "name": "a\u{7}b", "mode": "private", "publicHost": null }),
            json!({ "name": "x", "mode": "lan", "publicHost": null }),
            json!({ "name": "x", "mode": "public", "publicHost": null }),
            json!({ "name": "x", "mode": "public", "publicHost": "-bad.example" }),
            json!({ "name": "x", "mode": "public", "publicHost": "a b" }),
            json!({ "name": "x", "mode": "public", "publicHost": format!("a{}a", "b".repeat(252)) }),
            json!([]),
            json!(null),
        ] {
            assert_eq!(host_setup_request(&bad), None, "{bad}");
        }
    }

    #[test]
    fn multipliers_from_1_to_100_rounded_and_1_1_is_off() {
        assert_eq!(host_multipliers(&json!(null)), Ok(None));
        assert_eq!(host_multipliers(&json!({ "xp": 1, "rewards": 1 })), Ok(None));
        assert_eq!(
            host_multipliers(&json!({ "xp": 2.345, "rewards": 100 })),
            Ok(Some(HostMultipliers {
                xp: 2.35,
                rewards: 100.0
            }))
        );
        for bad in [
            json!({ "xp": 0.5, "rewards": 1 }),
            json!({ "xp": 101, "rewards": 1 }),
            json!({ "xp": "2", "rewards": 1 }),
            json!({ "xp": 2 }),
            json!([]),
            json!("off"),
        ] {
            assert_eq!(host_multipliers(&bad), Err(()), "{bad}");
        }
    }

    #[test]
    fn who_an_invite_is_for() {
        assert_eq!(invite_for(None), Ok(None));
        assert_eq!(invite_for(Some(&json!(null))), Ok(None));
        assert_eq!(invite_for(Some(&json!(""))), Ok(None));
        assert_eq!(invite_for(Some(&json!("   "))), Ok(None));
        assert_eq!(invite_for(Some(&json!(" Sam "))), Ok(Some("Sam".into())));
        assert_eq!(invite_for(Some(&json!("a".repeat(41)))), Err(()));
        assert_eq!(invite_for(Some(&json!("a\nb"))), Err(()));
        assert_eq!(invite_for(Some(&json!(7))), Err(()));
    }

    #[test]
    fn copying_keeps_the_4096_limit() {
        assert!(!copy_text("x".repeat(COPY_TEXT_MAX + 1)));
    }

    /// A value of the right shape for each argument, as the page would send it.
    fn good(arg: &Arg) -> Value {
        match arg {
            Arg::Text { .. } | Arg::OptionalText { .. } => json!("a"),
            Arg::Flag => json!(true),
            Arg::Index { max } => json!(max),
            Arg::Name { names } => json!(names[0]),
            Arg::Number { min, .. } => json!(min),
            Arg::Object { fields, .. } => Value::Object(
                fields
                    .iter()
                    .map(|(name, arg)| ((*name).to_owned(), good(arg)))
                    .collect(),
            ),
            Arg::SettingsPatch | Arg::Channel => json!(null),
        }
    }

    #[test]
    fn planned_commands_are_refused_with_a_code() {
        for name in crate::bridge::spec::PLANNED {
            let shapes = COMMANDS.iter().find(|command| command.name == *name).unwrap().args;
            let values: Vec<Value> = shapes.iter().map(|(_, arg)| good(arg)).collect();
            let args: Vec<&Value> = values.iter().collect();
            let code = if serves(name) {
                ErrorCode::NotImplemented
            } else {
                ErrorCode::NotInThisBuild
            };
            assert_eq!(
                serde_json::to_value(planned(name, &args)).unwrap(),
                json!({ "ok": false, "error": { "code": serde_json::to_value(code).unwrap(), "detail": name } }),
                "{name}"
            );
        }
    }

    #[test]
    fn planned_commands_check_their_arguments() {
        let unknown = ActionResult::err(ErrorCode::Unknown);
        let long = json!("a".repeat(crate::bridge::spec::EPIC_CODE_MAX + 1));
        assert_eq!(planned("epic_sign_in_code", &[&long]), unknown);
        assert_eq!(planned("epic_sign_in_code", &[&json!(7)]), unknown);
        assert_eq!(planned("epic_sign_in_code", &[]), unknown);
        assert_eq!(planned("realm_host", &[&json!("raid")]), unknown);
        assert_eq!(planned("no_such_command", &[]), unknown);
        let filter = json!({ "actor": "key_01", "since": "2026-10-01T00:00:00Z" });
        assert_ne!(planned("history_query", &[&filter]), unknown);
        for bad in [
            json!({ "actor": "key_01", "table": "accounts" }),
            json!({ "actor": 7 }),
            json!(null),
            json!([]),
        ] {
            assert_eq!(planned("history_query", &[&bad]), unknown, "{bad}");
        }
    }
}
