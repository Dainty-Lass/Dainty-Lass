// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! What each command checks before it reaches the controller (RD27 spec §2.3.4). The isolation hook guards the
//! page; these guard the backend, so they hold even if a page gets past the hook. Each returns the value the
//! controller takes, or the [`LauncherError`] the command answers with.

use protocol::text::js_trim;
use serde_json::Value;

use super::spec::{
    self, ADMIN_PLAYER_MAX, ADMIN_REASON_MAX, CHANGE_ID_MAX, COPY_TEXT_MAX, DETECTED_INDEX_MAX, EPIC_CODE_MAX,
    GAME_PATH_MAX, GAME_VERSION_MAX, GUARDED_ID_LEN, HISTORY_CURSOR_MAX, HISTORY_TIME_MAX, HISTORY_WHO_MAX,
    INVITE_FOR_MAX, INVITE_MAX, KEY_TEXT_MAX, MODULE_ID_MAX, PUBLIC_HOST_MAX, REALM_ID_MAX, REMOTE_LABEL_MAX,
    REMOTE_NODE_MAX, REMOTE_URI_MAX, REMOTE_URL_MAX, SERVER_NAME_MAX, Shape, USERNAME_MAX, fits,
};
use super::types::{
    AdminActionKind, ErrorCode, HostMultipliers, HostServerMode, HostSetupRequest, LauncherError, SettingsPatch,
};

/// Detail of `host_failed` for a bad `host_setup` form.
pub const SETUP_FORM: &str = "setup_form";
/// Detail of `host_failed` for a bad `host_set_multipliers` form.
pub const MULTIPLIERS_FORM: &str = "multipliers_form";
/// Detail of `host_failed` when the server GUI has no workspace.
pub const WORKSPACE: &str = "workspace";
/// Detail of `host_failed` for Host commands in a build without the Host page.
pub const HOST_TOOLS_UNAVAILABLE: &str = "host_tools_unavailable";

fn fail(code: ErrorCode) -> LauncherError {
    LauncherError::new(code)
}

/// `not_in_this_build`, naming the command (§2.3.2).
pub fn not_in_this_build(command: &str) -> LauncherError {
    LauncherError::with_detail(ErrorCode::NotInThisBuild, command)
}

/// `not_implemented`, naming the command: a planned command in a build that serves it (§2.3.2).
pub fn not_implemented(command: &str) -> LauncherError {
    LauncherError::with_detail(ErrorCode::NotImplemented, command)
}

/// `host_failed` with `detail`.
pub fn host_failed(detail: &str) -> LauncherError {
    LauncherError::with_detail(ErrorCode::HostFailed, detail)
}

/// A control character as the forms refuse it: U+0000–U+001F and U+007F.
pub const fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}')
}

/// `text` trimmed as JavaScript trims, when it then holds no control character and fits `max`; empty is allowed.
pub fn plain_text(text: &str, max: usize) -> Option<&str> {
    let trimmed = js_trim(text);
    (fits(trimmed, max) && !trimmed.chars().any(is_control)).then_some(trimmed)
}

/// [`plain_text`], and not empty.
pub fn required_text(text: &str, max: usize) -> Option<&str> {
    plain_text(text, max).filter(|t| !t.is_empty())
}

/// An optional field of a form: absent, null or empty after trimming is `None`; else [`plain_text`].
fn optional_text(value: Option<&Value>, max: usize) -> Result<Option<String>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => match plain_text(s, max) {
            Some("") => Ok(None),
            Some(t) => Ok(Some(t.to_owned())),
            None => Err(()),
        },
        Some(_) => Err(()),
    }
}

fn required_field(value: Option<&Value>, max: usize) -> Result<String, ()> {
    value
        .and_then(Value::as_str)
        .and_then(|s| required_text(s, max))
        .map(str::to_owned)
        .ok_or(())
}

/// `check_invite`, `submit_invite`: the invite's text.
pub fn invite_text(text: &str) -> Result<&str, LauncherError> {
    if fits(text, INVITE_MAX) {
        Ok(text)
    } else {
        Err(fail(ErrorCode::InviteInvalidFormat))
    }
}

/// `switch_server`, `remove_saved_server`: a saved server's id.
pub fn server_id(id: &str) -> Result<&str, LauncherError> {
    if spec::is_server_id(id) {
        Ok(id)
    } else {
        Err(fail(ErrorCode::InviteInvalidFormat))
    }
}

/// `register`: the length only; the name's rules are the server's and `protocol::account`'s.
pub fn username(name: &str) -> Result<&str, LauncherError> {
    if fits(name, USERNAME_MAX) {
        Ok(name)
    } else {
        Err(fail(ErrorCode::UsernameInvalid))
    }
}

/// `use_existing_key`: the pasted key's length.
pub fn key_text(text: &str) -> Result<&str, LauncherError> {
    if fits(text, KEY_TEXT_MAX) {
        Ok(text)
    } else {
        Err(fail(ErrorCode::KeyInvalid))
    }
}

/// `use_game_path`: the path's length.
pub fn game_path(path: &str) -> Result<&str, LauncherError> {
    if fits(path, GAME_PATH_MAX) {
        Ok(path)
    } else {
        Err(fail(ErrorCode::FolderInvalid))
    }
}

/// `use_detected_game`, `revived_use_detected`: an integral JSON number in 0..=15.
pub fn detected_index(value: &Value) -> Result<usize, LauncherError> {
    if !spec::fits_shape(
        &Shape::Index {
            max: DETECTED_INDEX_MAX,
        },
        Some(value),
    ) {
        return Err(fail(ErrorCode::FolderInvalid));
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "0..=15, checked above"
    )]
    value
        .as_f64()
        .map(|f| f as usize)
        .ok_or_else(|| fail(ErrorCode::FolderInvalid))
}

/// A name argument (`revived_install`, `open_external`, `host_action`, `host_open`, `host_server_restart`,
/// `realm_host`): a JSON string equal to one of `names`, decoded as `T` (whose names they are).
pub fn name<T: serde::de::DeserializeOwned>(value: &Value, names: &[&str]) -> Result<T, LauncherError> {
    match value {
        Value::String(s) if names.contains(&s.as_str()) => {
            serde_json::from_value(value.clone()).map_err(|_| fail(ErrorCode::Unknown))
        }
        _ => Err(fail(ErrorCode::Unknown)),
    }
}

/// [`name`] for a list with no enum, such as the server components: the listed name itself.
pub fn listed_name(value: &Value, names: &'static [&'static str]) -> Result<&'static str, LauncherError> {
    value
        .as_str()
        .and_then(|s| names.iter().find(|n| **n == s).copied())
        .ok_or_else(|| fail(ErrorCode::Unknown))
}

/// `set_settings`: decoded strictly; anything not a valid patch is the empty patch, which changes nothing.
pub fn settings_patch(value: &Value) -> SettingsPatch {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// `host_setup`: the server form. Private mode drops `publicHost`.
pub fn host_setup(value: &Value) -> Result<HostSetupRequest, LauncherError> {
    let bad = || host_failed(SETUP_FORM);
    let form = value.as_object().ok_or_else(bad)?;
    let name = form
        .get("name")
        .and_then(Value::as_str)
        .and_then(|s| required_text(s, SERVER_NAME_MAX))
        .ok_or_else(bad)?;
    let mode = match form.get("mode").and_then(Value::as_str) {
        Some("private") => HostServerMode::Private,
        Some("public") => HostServerMode::Public,
        _ => return Err(bad()),
    };
    let public_host = match mode {
        HostServerMode::Private => None,
        HostServerMode::Public => {
            let host = form
                .get("publicHost")
                .and_then(Value::as_str)
                .map(js_trim)
                .ok_or_else(bad)?;
            if !fits(host, PUBLIC_HOST_MAX) || !is_host_name(host) {
                return Err(bad());
            }
            Some(host.to_owned())
        }
    };
    Ok(HostSetupRequest {
        name: name.to_owned(),
        mode,
        public_host,
    })
}

/// `^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?$`.
fn is_host_name(host: &str) -> bool {
    let bytes = host.as_bytes();
    let alnum = |b: &u8| b.is_ascii_alphanumeric();
    match bytes {
        [] => false,
        [only] => alnum(only),
        [first, middle @ .., last] => {
            alnum(first)
                && alnum(last)
                && middle.len() <= 251
                && middle.iter().all(|b| alnum(b) || *b == b'.' || *b == b'-')
        }
    }
}

/// `host_set_multipliers`: null, or both equal to 1 after rounding, is off (`None`).
pub fn multipliers(value: &Value) -> Result<Option<HostMultipliers>, LauncherError> {
    let bad = || host_failed(MULTIPLIERS_FORM);
    if value.is_null() {
        return Ok(None);
    }
    let form = value.as_object().ok_or_else(bad)?;
    let read = |key: &str| {
        form.get(key)
            .and_then(Value::as_f64)
            .filter(|f| f.is_finite() && (1.0..=super::types::MAX_MULTIPLIER).contains(f))
            // As `Math.round(x * 100) / 100`; the values are positive, so half-up and half-away agree.
            .map(|f| (f * 100.0).round() / 100.0)
            .ok_or_else(bad)
    };
    let set = HostMultipliers {
        xp: read("xp")?,
        rewards: read("rewards")?,
    };
    #[allow(clippy::float_cmp, reason = "exactly 1 after rounding is the off value")]
    let off = set.xp == 1.0 && set.rewards == 1.0;
    Ok((!off).then_some(set))
}

/// `host_create_invite`: absent, null or `""` is nobody; else non-empty plain text ≤40.
pub fn invite_for(value: Option<&Value>) -> Result<Option<String>, LauncherError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => required_text(s, INVITE_FOR_MAX)
            .map(|t| Some(t.to_owned()))
            .ok_or_else(|| fail(ErrorCode::Unknown)),
        Some(_) => Err(fail(ErrorCode::Unknown)),
    }
}

/// `copy_text`: whether the text may go on the clipboard.
pub fn copy_text(text: &str) -> bool {
    fits(text, COPY_TEXT_MAX)
}

/// `epic_sign_in_code`: the code's length.
pub fn epic_code(code: &str) -> Result<&str, LauncherError> {
    if fits(code, EPIC_CODE_MAX) {
        Ok(code)
    } else {
        Err(fail(ErrorCode::EpicCodeInvalid))
    }
}

/// `game_install_*`: a non-empty version ≤64.
pub fn game_version(version: &str) -> Result<&str, LauncherError> {
    if !version.is_empty() && fits(version, GAME_VERSION_MAX) {
        Ok(version)
    } else {
        Err(fail(ErrorCode::Unknown))
    }
}

/// `realm_join`: the realm id, trimmed.
pub fn realm_id(realm: &str) -> Result<&str, LauncherError> {
    required_text(realm, REALM_ID_MAX).ok_or_else(|| fail(ErrorCode::Unknown))
}

/// A history filter or revert selection, each field trimmed and empty as absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    /// Changes to this account.
    pub account: Option<String>,
    /// Changes by this actor.
    pub actor: Option<String>,
    /// From this time.
    pub since: Option<String>,
    /// Up to this time.
    pub until: Option<String>,
    /// The page to read (`history_query` only).
    pub cursor: Option<String>,
}

fn history(value: &Value, command: &str) -> Result<HistoryFilter, LauncherError> {
    let bad = || fail(ErrorCode::Unknown);
    let fields = match spec::command(command).map(|c| c.args) {
        Some(
            [
                spec::Field {
                    shape: Shape::Object { fields, .. },
                    ..
                },
            ],
        ) => *fields,
        _ => return Err(bad()),
    };
    if !spec::fits_fields(fields, value) {
        return Err(bad());
    }
    let field = |key: &str, max: usize| optional_text(value.get(key), max).map_err(|()| bad());
    Ok(HistoryFilter {
        account: field("account", HISTORY_WHO_MAX)?,
        actor: field("actor", HISTORY_WHO_MAX)?,
        since: field("since", HISTORY_TIME_MAX)?,
        until: field("until", HISTORY_TIME_MAX)?,
        cursor: field("cursor", HISTORY_CURSOR_MAX)?,
    })
}

/// `history_query`'s filter.
pub fn history_filter(value: &Value) -> Result<HistoryFilter, LauncherError> {
    history(value, "history_query")
}

/// `history_revert_dry_run`'s selection (no cursor).
pub fn revert_selection(value: &Value) -> Result<HistoryFilter, LauncherError> {
    history(value, "history_revert_dry_run")
}

/// `history_revert_apply`: the plan's id.
pub fn revert_plan(plan: &str) -> Result<&str, LauncherError> {
    if !plan.is_empty() && fits(plan, CHANGE_ID_MAX) {
        Ok(plan)
    } else {
        Err(fail(ErrorCode::Unknown))
    }
}

/// `guarded_approve`, `guarded_cancel`: 16 lower-case hex digits.
pub fn guarded_id(id: &str) -> Result<&str, LauncherError> {
    if spec::is_lower_hex(id, GUARDED_ID_LEN) {
        Ok(id)
    } else {
        Err(fail(ErrorCode::Unknown))
    }
}

/// The checked `server_config_save` form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfigForm {
    /// One of [`spec::REGISTRATION_MODES`].
    pub registration: &'static str,
    /// One of [`spec::DATABASE_ENGINES`].
    pub database: &'static str,
    /// Most hubs at once.
    pub max_hubs: u32,
    /// Most hunts at once.
    pub max_hunts: u32,
    /// Wipe the data on the next start.
    pub fresh_start: bool,
}

fn object_fits(value: &Value, command: &str) -> bool {
    spec::command(command).is_some_and(|c| match c.args {
        [spec::Field { shape, .. }] => spec::fits_shape(shape, Some(value)),
        _ => false,
    })
}

/// `server_config_save`.
pub fn server_config(value: &Value) -> Result<ServerConfigForm, LauncherError> {
    let bad = || fail(ErrorCode::Unknown);
    if !object_fits(value, "server_config_save") {
        return Err(bad());
    }
    let count = |key: &str| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the shape bounds it"
        )]
        value.get(key).and_then(Value::as_f64).map(|f| f as u32).ok_or_else(bad)
    };
    let pick =
        |key: &str, list: &'static [&'static str]| value.get(key).ok_or_else(bad).and_then(|v| listed_name(v, list));
    Ok(ServerConfigForm {
        registration: pick("registration", spec::REGISTRATION_MODES)?,
        database: pick("database", spec::DATABASE_ENGINES)?,
        max_hubs: count("maxHubs")?,
        max_hunts: count("maxHunts")?,
        fresh_start: value.get("freshStart").and_then(Value::as_bool).ok_or_else(bad)?,
    })
}

/// The checked `admin_action` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminRequest {
    /// What to do.
    pub action: AdminActionKind,
    /// To whom.
    pub player: String,
    /// Why.
    pub reason: Option<String>,
    /// Until when (timeouts and bans).
    pub until: Option<String>,
    /// In which module.
    pub module: Option<String>,
}

/// `admin_action`.
pub fn admin_request(value: &Value) -> Result<AdminRequest, LauncherError> {
    let bad = |()| fail(ErrorCode::Unknown);
    if !object_fits(value, "admin_action") {
        return Err(bad(()));
    }
    Ok(AdminRequest {
        action: name(value.get("action").unwrap_or(&Value::Null), spec::ADMIN_ACTIONS)?,
        player: required_field(value.get("player"), ADMIN_PLAYER_MAX).map_err(bad)?,
        reason: optional_text(value.get("reason"), ADMIN_REASON_MAX).map_err(bad)?,
        until: optional_text(value.get("until"), HISTORY_TIME_MAX).map_err(bad)?,
        module: optional_text(value.get("module"), MODULE_ID_MAX).map_err(bad)?,
    })
}

/// The checked `remote_init_controller` request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteInitRequest {
    /// This controller's node id.
    pub node: Option<String>,
    /// Its region.
    pub region: Option<String>,
    /// The address nodes reach it at.
    pub url: Option<String>,
}

/// `remote_init_controller`.
pub fn remote_init(value: &Value) -> Result<RemoteInitRequest, LauncherError> {
    let bad = |()| fail(ErrorCode::Unknown);
    if !object_fits(value, "remote_init_controller") {
        return Err(bad(()));
    }
    Ok(RemoteInitRequest {
        node: optional_text(value.get("node"), REMOTE_NODE_MAX).map_err(bad)?,
        region: optional_text(value.get("region"), REMOTE_NODE_MAX).map_err(bad)?,
        url: optional_text(value.get("url"), REMOTE_URL_MAX).map_err(bad)?,
    })
}

/// The checked `remote_invite` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInviteRequest {
    /// The new node's id.
    pub node: String,
    /// Its region.
    pub region: Option<String>,
    /// The host's label for it.
    pub label: Option<String>,
    /// The controller's address to put in the invite.
    pub url: Option<String>,
}

/// `remote_invite`.
pub fn remote_invite(value: &Value) -> Result<RemoteInviteRequest, LauncherError> {
    let bad = |()| fail(ErrorCode::Unknown);
    if !object_fits(value, "remote_invite") {
        return Err(bad(()));
    }
    Ok(RemoteInviteRequest {
        node: required_field(value.get("node"), REMOTE_NODE_MAX).map_err(bad)?,
        region: optional_text(value.get("region"), REMOTE_NODE_MAX).map_err(bad)?,
        label: optional_text(value.get("label"), REMOTE_LABEL_MAX).map_err(bad)?,
        url: optional_text(value.get("url"), REMOTE_URL_MAX).map_err(bad)?,
    })
}

/// The checked `remote_enroll` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEnrollRequest {
    /// The enrolment URI. A secret: never logged.
    pub uri: String,
    /// The host's label for this install.
    pub label: Option<String>,
}

/// `remote_enroll`.
pub fn remote_enroll(value: &Value) -> Result<RemoteEnrollRequest, LauncherError> {
    let bad = |()| fail(ErrorCode::Unknown);
    if !object_fits(value, "remote_enroll") {
        return Err(bad(()));
    }
    Ok(RemoteEnrollRequest {
        uri: required_field(value.get("uri"), REMOTE_URI_MAX).map_err(bad)?,
        label: optional_text(value.get("label"), REMOTE_LABEL_MAX).map_err(bad)?,
    })
}

/// `remote_revoke`: the node, required plain text ≤64.
pub fn remote_node(node: &str) -> Result<&str, LauncherError> {
    required_text(node, REMOTE_NODE_MAX).ok_or_else(|| fail(ErrorCode::Unknown))
}

/// The checked `remote_exec` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteExecRequest {
    /// The node.
    pub node: String,
    /// One of [`spec::REMOTE_EXEC_ACTIONS`].
    pub action: &'static str,
    /// The player or guarded action it is about.
    pub target: Option<String>,
    /// Why.
    pub reason: Option<String>,
}

/// `remote_exec`.
pub fn remote_exec(value: &Value) -> Result<RemoteExecRequest, LauncherError> {
    let bad = |()| fail(ErrorCode::Unknown);
    if !object_fits(value, "remote_exec") {
        return Err(bad(()));
    }
    Ok(RemoteExecRequest {
        node: required_field(value.get("node"), REMOTE_NODE_MAX).map_err(bad)?,
        action: listed_name(value.get("action").unwrap_or(&Value::Null), spec::REMOTE_EXEC_ACTIONS)?,
        target: optional_text(value.get("target"), ADMIN_PLAYER_MAX).map_err(bad)?,
        reason: optional_text(value.get("reason"), ADMIN_REASON_MAX).map_err(bad)?,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use serde_json::json;

    use super::*;
    use crate::bridge::types::{ExternalTarget, GraphicsPreset, InstallMode};

    fn code<T: std::fmt::Debug>(result: Result<T, LauncherError>) -> ErrorCode {
        result.unwrap_err().code
    }

    #[test]
    fn lengths_pick_their_codes() {
        assert_eq!(code(invite_text(&"a".repeat(2049))), ErrorCode::InviteInvalidFormat);
        assert!(invite_text(&"a".repeat(2048)).is_ok());
        assert_eq!(code(server_id("x")), ErrorCode::InviteInvalidFormat);
        assert_eq!(code(username(&"a".repeat(33))), ErrorCode::UsernameInvalid);
        assert_eq!(code(key_text(&"a".repeat(8193))), ErrorCode::KeyInvalid);
        assert_eq!(code(game_path(&"a".repeat(261))), ErrorCode::FolderInvalid);
        assert_eq!(code(epic_code(&"a".repeat(1025))), ErrorCode::EpicCodeInvalid);
        assert_eq!(code(game_version("")), ErrorCode::Unknown);
        assert!(copy_text(&"a".repeat(4096)) && !copy_text(&"a".repeat(4097)));
    }

    #[test]
    fn detected_index_must_be_integral() {
        assert_eq!(detected_index(&json!(15)).unwrap(), 15);
        assert_eq!(detected_index(&json!(2.0)).unwrap(), 2);
        for bad in [json!(16), json!(-1), json!(1.5), json!("1"), json!(null)] {
            assert_eq!(code(detected_index(&bad)), ErrorCode::FolderInvalid, "{bad}");
        }
    }

    #[test]
    fn names_must_be_listed() {
        let target: ExternalTarget = name(&json!("mongodb"), spec::EXTERNAL_TARGETS).unwrap();
        assert_eq!(target, ExternalTarget::Mongodb);
        let mode: InstallMode = name(&json!("repair"), spec::INSTALL_MODES).unwrap();
        assert_eq!(mode, InstallMode::Repair);
        assert_eq!(
            code(name::<InstallMode>(&json!("Repair"), spec::INSTALL_MODES)),
            ErrorCode::Unknown
        );
        assert_eq!(
            code(name::<InstallMode>(&json!(1), spec::INSTALL_MODES)),
            ErrorCode::Unknown
        );
        assert_eq!(
            listed_name(&json!("legacy-gateway"), spec::HOST_SERVER_COMPONENTS).unwrap(),
            "legacy-gateway"
        );
        assert_eq!(
            code(listed_name(&json!("x"), spec::HOST_SERVER_COMPONENTS)),
            ErrorCode::Unknown
        );
    }

    #[test]
    fn a_bad_patch_changes_nothing() {
        let patch = settings_patch(&json!({"graphics": 4, "language": "pt-BR", "closeToTray": true}));
        assert_eq!(patch.graphics, GraphicsPreset::new(4));
        assert!(patch.close_to_tray == Some(true) && patch.windowed.is_none());
        assert_eq!(settings_patch(&json!({"graphics": 5})), SettingsPatch::default());
        assert_eq!(settings_patch(&json!({"other": true})), SettingsPatch::default());
        assert_eq!(settings_patch(&json!({"windowed": "yes"})), SettingsPatch::default());
    }

    #[test]
    fn setup_form() {
        let private = host_setup(&json!({"name": "  Isles ", "mode": "private", "publicHost": "x y"})).unwrap();
        assert_eq!(private.name, "Isles");
        assert_eq!(private.public_host, None);
        let public =
            host_setup(&json!({"name": "Isles", "mode": "public", "publicHost": " a.example-1.org "})).unwrap();
        assert_eq!(public.public_host.as_deref(), Some("a.example-1.org"));
        for bad in [
            json!(null),
            json!({"name": "", "mode": "private"}),
            json!({"name": "a\u{7}b", "mode": "private"}),
            json!({"name": "x".repeat(61), "mode": "private"}),
            json!({"name": "Isles", "mode": "lan"}),
            json!({"name": "Isles", "mode": "public"}),
            json!({"name": "Isles", "mode": "public", "publicHost": "-a.org"}),
            json!({"name": "Isles", "mode": "public", "publicHost": "a.org."}),
        ] {
            let error = host_setup(&bad).unwrap_err();
            assert_eq!(error, host_failed(SETUP_FORM), "{bad}");
        }
        assert!(host_setup(&json!({"name": "I", "mode": "public", "publicHost": "a"})).is_ok());
    }

    #[test]
    fn multipliers_round_and_turn_off() {
        assert_eq!(multipliers(&json!(null)).unwrap(), None);
        assert_eq!(multipliers(&json!({"xp": 1.004, "rewards": 1})).unwrap(), None);
        let set = multipliers(&json!({"xp": 2.345, "rewards": 100})).unwrap().unwrap();
        assert!((set.xp - 2.35).abs() < 1e-9 && (set.rewards - 100.0).abs() < 1e-9);
        for bad in [
            json!({"xp": 0.5, "rewards": 1}),
            json!({"xp": 2}),
            json!({"xp": "2", "rewards": 2}),
            json!(3),
        ] {
            assert_eq!(multipliers(&bad).unwrap_err(), host_failed(MULTIPLIERS_FORM), "{bad}");
        }
    }

    #[test]
    fn invite_for_and_realms() {
        assert_eq!(invite_for(None).unwrap(), None);
        assert_eq!(invite_for(Some(&json!(""))).unwrap(), None);
        assert_eq!(invite_for(Some(&json!(" Ann "))).unwrap().as_deref(), Some("Ann"));
        assert_eq!(code(invite_for(Some(&json!("  ")))), ErrorCode::Unknown);
        assert_eq!(code(invite_for(Some(&json!("x".repeat(41))))), ErrorCode::Unknown);
        assert_eq!(realm_id(" r1 ").unwrap(), "r1");
        assert_eq!(code(realm_id("r\n1")), ErrorCode::Unknown);
    }

    #[test]
    fn history_forms() {
        let filter = history_filter(&json!({"account": " a ", "actor": "", "cursor": "c"})).unwrap();
        assert_eq!(filter.account.as_deref(), Some("a"));
        assert_eq!(filter.actor, None);
        assert_eq!(filter.cursor.as_deref(), Some("c"));
        assert_eq!(code(revert_selection(&json!({"cursor": "c"}))), ErrorCode::Unknown);
        assert_eq!(
            code(history_filter(&json!({"since": "x".repeat(41)}))),
            ErrorCode::Unknown
        );
        assert_eq!(code(revert_plan("")), ErrorCode::Unknown);
        assert!(guarded_id("0123456789abcdef").is_ok());
        assert_eq!(code(guarded_id("0123456789ABCDEF")), ErrorCode::Unknown);
    }

    #[test]
    fn server_gui_forms() {
        let config = server_config(&json!({
            "registration": "INVITE", "database": "postgres", "maxHubs": 64, "maxHunts": 0, "freshStart": false
        }))
        .unwrap();
        assert_eq!((config.registration, config.max_hubs), ("INVITE", 64));
        assert!(server_config(&json!({"registration": "OPEN"})).is_err());
        let admin = admin_request(&json!({"action": "ban", "player": " p ", "reason": ""})).unwrap();
        assert_eq!(
            (admin.action, admin.player.as_str(), admin.reason),
            (AdminActionKind::Ban, "p", None)
        );
        assert!(admin_request(&json!({"action": "ban", "player": " "})).is_err());
        assert_eq!(remote_init(&json!({})).unwrap(), RemoteInitRequest::default());
        assert!(remote_invite(&json!({"node": "n", "url": "x".repeat(513)})).is_err());
        assert_eq!(remote_enroll(&json!({"uri": "u"})).unwrap().uri, "u");
        let exec = remote_exec(&json!({"node": "n", "action": "announce", "reason": "hi"})).unwrap();
        assert_eq!(exec.action, "announce");
        assert!(remote_exec(&json!({"node": "n", "action": "reboot"})).is_err());
        assert_eq!(code(remote_node("")), ErrorCode::Unknown);
    }
}
