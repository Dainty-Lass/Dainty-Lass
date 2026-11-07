// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The bridge's command table: every command the page may invoke, the argument each takes, the bounds
//! on its text, and which build serves it.

use crate::core::text::{utf16_len, within_utf16};

use super::types::RequirementId;

/// The JSON shape a command's argument must have, checked by [`arg_accepts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    /// No argument: null or `{}`.
    None,
    /// `{invite}`: the pasted invite text.
    Invite,
    /// `{serverId}`: 24 lowercase hex digits.
    ServerId,
    /// `{serverId, username, inviteCode?}`.
    RegisterAccount,
    /// `{serverId, key, username?}`.
    ImportAccountKey,
    /// `{serverId}` of the key to export.
    ExportAccountKey,
    /// `{path}`: the game folder.
    GamePath,
    /// `{version}`: a game version.
    SelectGameVersion,
    /// `{version, path}`: a game version and its folder.
    GameVersionPath,
    /// `{code}`: the Epic authorization code.
    EpicCompleteSignIn,
    /// `{version, installDir}`.
    InstallGameVersion,
    /// Any JSON object of launcher settings.
    LauncherSettings,
    /// `{mode}`: the Linux compatibility layer.
    LinuxCompat,
    /// `{id}`: a [`RequirementId`].
    Requirement,
    /// `{id, path}`: a [`RequirementId`] and where it is installed.
    LocateRequirement,
    /// `{displayName, port}`.
    HostServerConfig,
    /// An optional filter object.
    HostLogQuery,
    /// `{id, approve}`: a guarded action and the host's decision.
    GuardedDecision,
    /// `{moduleId}`.
    ModuleId,
    /// `{moduleId, ...}`: settings for one module.
    ModuleConfigure,
    /// `{moduleId, command, ...}`.
    ModuleCommand,
    /// An optional history filter object.
    HistoryQuery,
    /// `{entryId}`: the history entry to roll back to.
    HistoryPreviewRollback,
    /// `{planId}`: the previewed rollback to apply.
    HistoryApplyRollback,
    /// `{realmId}`.
    RealmId,
    /// `{name, moduleId}`.
    RealmCreate,
    /// `{realmId, ...}`: settings for one realm.
    RealmConfig,
    /// An optional account filter object.
    AdminAccountsQuery,
    /// `{username}`.
    AdminAccountCreate,
    /// `{accountId, status}`.
    AdminAccountStatus,
    /// `{accountId, note?}`.
    AdminAccountNote,
    /// `{accountId}`.
    AdminAccountDelete,
    /// Optional invite settings object.
    AdminInviteCreate,
    /// `{code}`: an invite code.
    AdminInviteCode,
    /// `{sessionId}`.
    AdminSessionKick,
    /// `{message}`: text sent to every player.
    AdminBroadcast,
    /// `{url?}`.
    OptionalUrl,
}

/// Every command the page may invoke on the Rust side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    /// Which features this build serves.
    GetCapabilities,
    /// The launcher's full state.
    GetSnapshot,
    /// Parses an invite without adding its server.
    PreviewInvite,
    /// Adds the invite's server to the list.
    AddServerFromInvite,
    /// Makes a listed server the joined one.
    SelectServer,
    /// Removes a server and its stored key.
    RemoveServer,
    /// Checks a server's status.
    ProbeServer,
    /// Times the joined server's regions and reports the order to it.
    ProbeRegionPing,
    /// Registers an account on a server.
    RegisterAccount,
    /// Stores an account key the player pasted.
    ImportAccountKey,
    /// Copies a stored account key out for the player.
    ExportAccountKey,
    /// Opens a folder picker for the game.
    PickGameFolder,
    /// Sets the game folder.
    SetGamePath,
    /// Checks the game files against the server's manifest.
    VerifyGame,
    /// Game versions the player may install.
    ListGameVersions,
    /// Makes a game version the active one.
    SelectGameVersion,
    /// Opens a folder picker for a game version.
    PickGameVersionFolder,
    /// Sets a game version's folder.
    SetGameVersionPath,
    /// Checks a game version's files.
    VerifyGameVersion,
    /// Whether an Epic account is signed in.
    EpicAccountStatus,
    /// Opens Epic's sign-in page.
    EpicStartSignIn,
    /// Finishes Epic sign-in with the pasted code.
    EpicCompleteSignIn,
    /// Forgets the Epic sign-in.
    EpicSignOut,
    /// Downloads a game version through Epic.
    InstallGameVersion,
    /// Stops a running game install.
    CancelGameInstall,
    /// Starts the game for the joined server.
    LaunchGame,
    /// Stops the running game.
    StopGame,
    /// Saves launcher settings.
    UpdateSettings,
    /// Finds Proton and Wine installs.
    DetectLinuxCompat,
    /// Picks the compatibility layer used on Linux.
    SetLinuxCompat,
    /// Checks for a launcher update.
    CheckForUpdates,
    /// Opens a fixed external page in the browser.
    OpenExternalUrl,
    /// Runs the host requirement checks.
    CheckRequirements,
    /// Installs a host requirement.
    InstallRequirement,
    /// Points a host requirement at a folder.
    LocateRequirement,
    /// The embedded server's state.
    GetHostServerState,
    /// Saves the server's name and port.
    ConfigureHostServer,
    /// Starts the embedded server.
    StartHostServer,
    /// Stops the embedded server.
    StopHostServer,
    /// Restarts the embedded server.
    RestartHostServer,
    /// The server's recent log lines.
    GetHostLogs,
    /// Clears the server's logs.
    ClearHostLogs,
    /// Copies the latest invite link.
    CopyHostInvite,
    /// Guarded actions waiting for the host.
    ListGuardedExecutions,
    /// Approves or refuses a guarded action.
    DecideGuardedExecution,
    /// The server's modules.
    ListModules,
    /// Loads a module.
    LoadModule,
    /// Stops a module.
    UnloadModule,
    /// Stops and loads a module again.
    ReloadModule,
    /// Saves a module's settings.
    ConfigureModule,
    /// Runs a module command.
    RunModuleCommand,
    /// The server's change history.
    ListHistory,
    /// Previews rolling back to a history entry.
    PreviewRollback,
    /// Applies a previewed rollback.
    ApplyRollback,
    /// The server's realms.
    ListRealms,
    /// Makes a realm.
    CreateRealm,
    /// Saves a realm's settings.
    ConfigureRealm,
    /// Starts a realm.
    StartRealm,
    /// Stops a realm.
    StopRealm,
    /// Deletes a realm.
    DeleteRealm,
    /// Joins a realm.
    JoinRealm,
    /// Lists accounts.
    AdminListAccounts,
    /// Makes an account.
    AdminCreateAccount,
    /// Sets an account's status.
    AdminSetAccountStatus,
    /// Sets an account's staff note.
    AdminSetAccountNote,
    /// Issues a new key for an account.
    AdminResetAccountKey,
    /// Deletes an account.
    AdminDeleteAccount,
    /// Lists invites.
    AdminListInvites,
    /// Makes an invite.
    AdminCreateInvite,
    /// Revokes an invite.
    AdminRevokeInvite,
    /// Lists connected sessions.
    AdminListSessions,
    /// Disconnects a session.
    AdminKickSession,
    /// Sends a message to every player.
    AdminBroadcast,
}

impl Command {
    /// The command's name on the wire, as the page invokes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::GetCapabilities => "get_capabilities",
            Self::GetSnapshot => "get_snapshot",
            Self::PreviewInvite => "preview_invite",
            Self::AddServerFromInvite => "add_server_from_invite",
            Self::SelectServer => "select_server",
            Self::RemoveServer => "remove_server",
            Self::ProbeServer => "probe_server",
            Self::ProbeRegionPing => "probe_region_ping",
            Self::RegisterAccount => "register_account",
            Self::ImportAccountKey => "import_account_key",
            Self::ExportAccountKey => "export_account_key",
            Self::PickGameFolder => "pick_game_folder",
            Self::SetGamePath => "set_game_path",
            Self::VerifyGame => "verify_game",
            Self::ListGameVersions => "list_game_versions",
            Self::SelectGameVersion => "select_game_version",
            Self::PickGameVersionFolder => "pick_game_version_folder",
            Self::SetGameVersionPath => "set_game_version_path",
            Self::VerifyGameVersion => "verify_game_version",
            Self::EpicAccountStatus => "epic_account_status",
            Self::EpicStartSignIn => "epic_start_sign_in",
            Self::EpicCompleteSignIn => "epic_complete_sign_in",
            Self::EpicSignOut => "epic_sign_out",
            Self::InstallGameVersion => "install_game_version",
            Self::CancelGameInstall => "cancel_game_install",
            Self::LaunchGame => "launch_game",
            Self::StopGame => "stop_game",
            Self::UpdateSettings => "update_settings",
            Self::DetectLinuxCompat => "detect_linux_compat",
            Self::SetLinuxCompat => "set_linux_compat",
            Self::CheckForUpdates => "check_for_updates",
            Self::OpenExternalUrl => "open_external_url",
            Self::CheckRequirements => "check_requirements",
            Self::InstallRequirement => "install_requirement",
            Self::LocateRequirement => "locate_requirement",
            Self::GetHostServerState => "get_host_server_state",
            Self::ConfigureHostServer => "configure_host_server",
            Self::StartHostServer => "start_host_server",
            Self::StopHostServer => "stop_host_server",
            Self::RestartHostServer => "restart_host_server",
            Self::GetHostLogs => "get_host_logs",
            Self::ClearHostLogs => "clear_host_logs",
            Self::CopyHostInvite => "copy_host_invite",
            Self::ListGuardedExecutions => "list_guarded_executions",
            Self::DecideGuardedExecution => "decide_guarded_execution",
            Self::ListModules => "list_modules",
            Self::LoadModule => "load_module",
            Self::UnloadModule => "unload_module",
            Self::ReloadModule => "reload_module",
            Self::ConfigureModule => "configure_module",
            Self::RunModuleCommand => "run_module_command",
            Self::ListHistory => "list_history",
            Self::PreviewRollback => "preview_rollback",
            Self::ApplyRollback => "apply_rollback",
            Self::ListRealms => "list_realms",
            Self::CreateRealm => "create_realm",
            Self::ConfigureRealm => "configure_realm",
            Self::StartRealm => "start_realm",
            Self::StopRealm => "stop_realm",
            Self::DeleteRealm => "delete_realm",
            Self::JoinRealm => "join_realm",
            Self::AdminListAccounts => "admin_list_accounts",
            Self::AdminCreateAccount => "admin_create_account",
            Self::AdminSetAccountStatus => "admin_set_account_status",
            Self::AdminSetAccountNote => "admin_set_account_note",
            Self::AdminResetAccountKey => "admin_reset_account_key",
            Self::AdminDeleteAccount => "admin_delete_account",
            Self::AdminListInvites => "admin_list_invites",
            Self::AdminCreateInvite => "admin_create_invite",
            Self::AdminRevokeInvite => "admin_revoke_invite",
            Self::AdminListSessions => "admin_list_sessions",
            Self::AdminKickSession => "admin_kick_session",
            Self::AdminBroadcast => "admin_broadcast",
        }
    }

    /// The [`Arg`] it takes.
    pub const fn arg(self) -> Arg {
        match self {
            Self::GetCapabilities
            | Self::GetSnapshot
            | Self::PickGameFolder
            | Self::VerifyGame
            | Self::ListGameVersions
            | Self::EpicAccountStatus
            | Self::EpicStartSignIn
            | Self::EpicSignOut
            | Self::CancelGameInstall
            | Self::LaunchGame
            | Self::StopGame
            | Self::DetectLinuxCompat
            | Self::CheckForUpdates
            | Self::CheckRequirements
            | Self::GetHostServerState
            | Self::StartHostServer
            | Self::StopHostServer
            | Self::RestartHostServer
            | Self::ClearHostLogs
            | Self::CopyHostInvite
            | Self::ListGuardedExecutions
            | Self::ListModules
            | Self::ListRealms
            | Self::AdminListInvites
            | Self::AdminListSessions => Arg::None,
            Self::PreviewInvite | Self::AddServerFromInvite => Arg::Invite,
            Self::SelectServer | Self::RemoveServer | Self::ProbeServer | Self::ProbeRegionPing => Arg::ServerId,
            Self::RegisterAccount => Arg::RegisterAccount,
            Self::ImportAccountKey => Arg::ImportAccountKey,
            Self::ExportAccountKey => Arg::ExportAccountKey,
            Self::SetGamePath => Arg::GamePath,
            Self::SelectGameVersion | Self::PickGameVersionFolder | Self::VerifyGameVersion => Arg::SelectGameVersion,
            Self::SetGameVersionPath => Arg::GameVersionPath,
            Self::EpicCompleteSignIn => Arg::EpicCompleteSignIn,
            Self::InstallGameVersion => Arg::InstallGameVersion,
            Self::UpdateSettings => Arg::LauncherSettings,
            Self::SetLinuxCompat => Arg::LinuxCompat,
            Self::OpenExternalUrl => Arg::OptionalUrl,
            Self::InstallRequirement => Arg::Requirement,
            Self::LocateRequirement => Arg::LocateRequirement,
            Self::ConfigureHostServer => Arg::HostServerConfig,
            Self::GetHostLogs => Arg::HostLogQuery,
            Self::DecideGuardedExecution => Arg::GuardedDecision,
            Self::LoadModule | Self::UnloadModule | Self::ReloadModule => Arg::ModuleId,
            Self::ConfigureModule => Arg::ModuleConfigure,
            Self::RunModuleCommand => Arg::ModuleCommand,
            Self::ListHistory => Arg::HistoryQuery,
            Self::PreviewRollback => Arg::HistoryPreviewRollback,
            Self::ApplyRollback => Arg::HistoryApplyRollback,
            Self::StartRealm | Self::StopRealm | Self::DeleteRealm | Self::JoinRealm => Arg::RealmId,
            Self::CreateRealm => Arg::RealmCreate,
            Self::ConfigureRealm => Arg::RealmConfig,
            Self::AdminListAccounts => Arg::AdminAccountsQuery,
            Self::AdminCreateAccount => Arg::AdminAccountCreate,
            Self::AdminSetAccountStatus => Arg::AdminAccountStatus,
            Self::AdminSetAccountNote => Arg::AdminAccountNote,
            Self::AdminResetAccountKey | Self::AdminDeleteAccount => Arg::AdminAccountDelete,
            Self::AdminCreateInvite => Arg::AdminInviteCreate,
            Self::AdminRevokeInvite => Arg::AdminInviteCode,
            Self::AdminKickSession => Arg::AdminSessionKick,
            Self::AdminBroadcast => Arg::AdminBroadcast,
        }
    }

    /// Only host builds, with the server GUI, serve it.
    pub const fn requires_server_gui(self) -> bool {
        matches!(
            self,
            Self::CheckRequirements
                | Self::InstallRequirement
                | Self::LocateRequirement
                | Self::GetHostServerState
                | Self::ConfigureHostServer
                | Self::StartHostServer
                | Self::StopHostServer
                | Self::RestartHostServer
                | Self::GetHostLogs
                | Self::ClearHostLogs
                | Self::CopyHostInvite
                | Self::ListGuardedExecutions
                | Self::DecideGuardedExecution
                | Self::ListModules
                | Self::LoadModule
                | Self::UnloadModule
                | Self::ReloadModule
                | Self::ConfigureModule
                | Self::RunModuleCommand
                | Self::ListHistory
                | Self::PreviewRollback
                | Self::ApplyRollback
                | Self::AdminListAccounts
                | Self::AdminCreateAccount
                | Self::AdminSetAccountStatus
                | Self::AdminSetAccountNote
                | Self::AdminResetAccountKey
                | Self::AdminDeleteAccount
                | Self::AdminListInvites
                | Self::AdminCreateInvite
                | Self::AdminRevokeInvite
                | Self::AdminListSessions
                | Self::AdminKickSession
                | Self::AdminBroadcast
        )
    }

    /// Only builds with realms serve it.
    pub const fn requires_realms(self) -> bool {
        matches!(
            self,
            Self::ListRealms
                | Self::CreateRealm
                | Self::ConfigureRealm
                | Self::StartRealm
                | Self::StopRealm
                | Self::DeleteRealm
                | Self::JoinRealm
        )
    }

    /// Declared but not served by any build yet.
    pub const fn is_planned(self) -> bool {
        matches!(self, Self::OpenExternalUrl)
    }
}

/// Every command, in declaration order.
pub const ALL_COMMANDS: &[Command] = &[
    Command::GetCapabilities,
    Command::GetSnapshot,
    Command::PreviewInvite,
    Command::AddServerFromInvite,
    Command::SelectServer,
    Command::RemoveServer,
    Command::ProbeServer,
    Command::ProbeRegionPing,
    Command::RegisterAccount,
    Command::ImportAccountKey,
    Command::ExportAccountKey,
    Command::PickGameFolder,
    Command::SetGamePath,
    Command::VerifyGame,
    Command::ListGameVersions,
    Command::SelectGameVersion,
    Command::PickGameVersionFolder,
    Command::SetGameVersionPath,
    Command::VerifyGameVersion,
    Command::EpicAccountStatus,
    Command::EpicStartSignIn,
    Command::EpicCompleteSignIn,
    Command::EpicSignOut,
    Command::InstallGameVersion,
    Command::CancelGameInstall,
    Command::LaunchGame,
    Command::StopGame,
    Command::UpdateSettings,
    Command::DetectLinuxCompat,
    Command::SetLinuxCompat,
    Command::CheckForUpdates,
    Command::OpenExternalUrl,
    Command::CheckRequirements,
    Command::InstallRequirement,
    Command::LocateRequirement,
    Command::GetHostServerState,
    Command::ConfigureHostServer,
    Command::StartHostServer,
    Command::StopHostServer,
    Command::RestartHostServer,
    Command::GetHostLogs,
    Command::ClearHostLogs,
    Command::CopyHostInvite,
    Command::ListGuardedExecutions,
    Command::DecideGuardedExecution,
    Command::ListModules,
    Command::LoadModule,
    Command::UnloadModule,
    Command::ReloadModule,
    Command::ConfigureModule,
    Command::RunModuleCommand,
    Command::ListHistory,
    Command::PreviewRollback,
    Command::ApplyRollback,
    Command::ListRealms,
    Command::CreateRealm,
    Command::ConfigureRealm,
    Command::StartRealm,
    Command::StopRealm,
    Command::DeleteRealm,
    Command::JoinRealm,
    Command::AdminListAccounts,
    Command::AdminCreateAccount,
    Command::AdminSetAccountStatus,
    Command::AdminSetAccountNote,
    Command::AdminResetAccountKey,
    Command::AdminDeleteAccount,
    Command::AdminListInvites,
    Command::AdminCreateInvite,
    Command::AdminRevokeInvite,
    Command::AdminListSessions,
    Command::AdminKickSession,
    Command::AdminBroadcast,
];

/// The command with this wire name, if any.
pub const fn command(name: &str) -> Option<Command> {
    let mut i = 0;
    while i < ALL_COMMANDS.len() {
        let cmd = ALL_COMMANDS[i];
        if str_eq(cmd.name(), name) {
            return Some(cmd);
        }
        i += 1;
    }
    None
}

const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Longest invite text, in UTF-16 units.
pub const INVITE_MAX_LEN: usize = 2048;
/// Length of a server id: 24 hex digits.
pub const SERVER_ID_LEN: usize = 24;
/// Shortest username, in UTF-16 units.
pub const USERNAME_MIN_LEN: usize = 3;
/// Longest username, in UTF-16 units.
pub const USERNAME_MAX_LEN: usize = 32;
/// Longest invite code, in UTF-16 units.
pub const INVITE_CODE_MAX_LEN: usize = 64;
/// Longest account key, in UTF-16 units.
pub const KEY_MAX_LEN: usize = 512;
/// Longest path, in UTF-16 units.
pub const PATH_MAX_LEN: usize = 4096;
/// Longest game version string, in UTF-16 units.
pub const GAME_VERSION_MAX_LEN: usize = 64;
/// Longest Epic authorization code, in UTF-16 units.
pub const EPIC_AUTH_CODE_MAX_LEN: usize = 512;
/// Longest URL, in UTF-16 units.
pub const URL_MAX_LEN: usize = 2048;
/// Longest server name, in UTF-16 units.
pub const HOST_NAME_MAX_LEN: usize = 128;
/// Longest module id, in UTF-16 units.
pub const MODULE_ID_MAX_LEN: usize = 64;
/// Longest module command name, in UTF-16 units.
pub const MODULE_COMMAND_MAX_LEN: usize = 64;
/// Longest realm id, in UTF-16 units.
pub const REALM_ID_MAX_LEN: usize = 64;
/// Longest realm name, in UTF-16 units.
pub const REALM_NAME_MAX_LEN: usize = 64;
/// Longest account note, in UTF-16 units.
pub const NOTE_MAX_LEN: usize = 512;
/// Longest broadcast message, in UTF-16 units.
pub const BROADCAST_MAX_LEN: usize = 1024;

/// Which shell the bridge is compiled into; it decides the commands served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// The player launcher: realms, no server GUI.
    Player,
    /// The server manager without realms.
    Host,
    /// The server manager with realms too.
    Full,
}

impl Build {
    /// Build name the page reads from `get_capabilities`.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Host => "host",
            Self::Full => "full",
        }
    }

    /// Whether it serves the server GUI commands.
    pub const fn server_gui(self) -> bool {
        matches!(self, Self::Host | Self::Full)
    }

    /// Whether it serves the realm commands.
    pub const fn realms(self) -> bool {
        matches!(self, Self::Player | Self::Full)
    }
}

/// Wire names of the commands only the server GUI serves.
pub const SERVER_GUI_COMMANDS: &[&str] = &[
    "check_requirements",
    "install_requirement",
    "locate_requirement",
    "get_host_server_state",
    "configure_host_server",
    "start_host_server",
    "stop_host_server",
    "restart_host_server",
    "get_host_logs",
    "clear_host_logs",
    "copy_host_invite",
    "list_guarded_executions",
    "decide_guarded_execution",
    "list_modules",
    "load_module",
    "unload_module",
    "reload_module",
    "configure_module",
    "run_module_command",
    "list_history",
    "preview_rollback",
    "apply_rollback",
    "admin_list_accounts",
    "admin_create_account",
    "admin_set_account_status",
    "admin_set_account_note",
    "admin_reset_account_key",
    "admin_delete_account",
    "admin_list_invites",
    "admin_create_invite",
    "admin_revoke_invite",
    "admin_list_sessions",
    "admin_kick_session",
    "admin_broadcast",
];

/// Wire names of the realm commands.
pub const REALM_COMMANDS: &[&str] = &[
    "list_realms",
    "create_realm",
    "configure_realm",
    "start_realm",
    "stop_realm",
    "delete_realm",
    "join_realm",
];

/// Wire names of the commands no build serves yet.
pub const PLANNED: &[&str] = &["open_external_url"];

/// Wire names of every command, in declaration order.
pub const COMMANDS: &[&str] = &[
    "get_capabilities",
    "get_snapshot",
    "preview_invite",
    "add_server_from_invite",
    "select_server",
    "remove_server",
    "probe_server",
    "probe_region_ping",
    "register_account",
    "import_account_key",
    "export_account_key",
    "pick_game_folder",
    "set_game_path",
    "verify_game",
    "list_game_versions",
    "select_game_version",
    "pick_game_version_folder",
    "set_game_version_path",
    "verify_game_version",
    "epic_account_status",
    "epic_start_sign_in",
    "epic_complete_sign_in",
    "epic_sign_out",
    "install_game_version",
    "cancel_game_install",
    "launch_game",
    "stop_game",
    "update_settings",
    "detect_linux_compat",
    "set_linux_compat",
    "check_for_updates",
    "open_external_url",
    "check_requirements",
    "install_requirement",
    "locate_requirement",
    "get_host_server_state",
    "configure_host_server",
    "start_host_server",
    "stop_host_server",
    "restart_host_server",
    "get_host_logs",
    "clear_host_logs",
    "copy_host_invite",
    "list_guarded_executions",
    "decide_guarded_execution",
    "list_modules",
    "load_module",
    "unload_module",
    "reload_module",
    "configure_module",
    "run_module_command",
    "list_history",
    "preview_rollback",
    "apply_rollback",
    "list_realms",
    "create_realm",
    "configure_realm",
    "start_realm",
    "stop_realm",
    "delete_realm",
    "join_realm",
    "admin_list_accounts",
    "admin_create_account",
    "admin_set_account_status",
    "admin_set_account_note",
    "admin_reset_account_key",
    "admin_delete_account",
    "admin_list_invites",
    "admin_create_invite",
    "admin_revoke_invite",
    "admin_list_sessions",
    "admin_kick_session",
    "admin_broadcast",
];

/// Names of the events Rust emits to the page.
pub const EVENTS: &[&str] = &[
    "snapshot",
    "game_status",
    "verify_progress",
    "install_progress",
    "server_connection",
    "requirements_updated",
    "host_server_updated",
    "host_log",
    "guarded_updated",
    "modules_updated",
    "realms_updated",
    "notice",
];

/// The build for the features compiled in.
pub fn build(server_gui: bool, realms: bool) -> Build {
    match (server_gui, realms) {
        (false, _) => Build::Player,
        (true, false) => Build::Host,
        (true, true) => Build::Full,
    }
}

/// Whether build `b` serves `cmd`.
pub const fn serves(b: Build, cmd: Command) -> bool {
    if cmd.is_planned() {
        return false;
    }
    if cmd.requires_server_gui() && !b.server_gui() {
        return false;
    }
    if cmd.requires_realms() && !b.realms() {
        return false;
    }
    true
}

/// `s` is not blank and fits in `max_utf16` UTF-16 units, as JavaScript counts length.
pub fn fits(s: &str, max_utf16: usize) -> bool {
    let trimmed = s.trim();
    !trimmed.is_empty() && within_utf16(s, max_utf16)
}

/// `s` is a server id: 24 lowercase hex digits.
pub fn is_server_id(s: &str) -> bool {
    s.len() == SERVER_ID_LEN && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `value` has the shape `arg` requires.
pub fn arg_accepts(arg: Arg, value: &serde_json::Value) -> bool {
    match arg {
        Arg::None => value.is_null() || value.as_object().is_some_and(|o| o.is_empty()),
        Arg::Invite => get_str(value, "invite").is_some_and(|s| fits(s, INVITE_MAX_LEN)),
        Arg::ServerId => get_str(value, "serverId").is_some_and(is_server_id),
        Arg::RegisterAccount => {
            get_str(value, "serverId").is_some_and(is_server_id)
                && get_str(value, "username").is_some_and(|u| {
                    let len = utf16_len(u.trim());
                    (USERNAME_MIN_LEN..=USERNAME_MAX_LEN).contains(&len)
                })
                && opt_str(value, "inviteCode", INVITE_CODE_MAX_LEN)
        }
        Arg::ImportAccountKey => {
            get_str(value, "serverId").is_some_and(is_server_id)
                && get_str(value, "key").is_some_and(|k| fits(k, KEY_MAX_LEN))
                && opt_str(value, "username", USERNAME_MAX_LEN)
        }
        Arg::ExportAccountKey => get_str(value, "serverId").is_some_and(is_server_id),
        Arg::GamePath => get_str(value, "path").is_some_and(|p| fits(p, PATH_MAX_LEN)),
        Arg::SelectGameVersion => get_str(value, "version").is_some_and(|v| fits(v, GAME_VERSION_MAX_LEN)),
        Arg::GameVersionPath => {
            get_str(value, "version").is_some_and(|v| fits(v, GAME_VERSION_MAX_LEN))
                && get_str(value, "path").is_some_and(|p| fits(p, PATH_MAX_LEN))
        }
        Arg::EpicCompleteSignIn => get_str(value, "code").is_some_and(|c| fits(c, EPIC_AUTH_CODE_MAX_LEN)),
        Arg::InstallGameVersion => {
            get_str(value, "version").is_some_and(|v| fits(v, GAME_VERSION_MAX_LEN))
                && get_str(value, "installDir").is_some_and(|p| fits(p, PATH_MAX_LEN))
        }
        Arg::LauncherSettings => value.is_object(),
        Arg::LinuxCompat => get_str(value, "mode").is_some_and(|m| fits(m, 64)),
        Arg::Requirement => get_str(value, "id").is_some_and(|id| {
            serde_json::from_value::<RequirementId>(serde_json::Value::String(id.to_owned())).is_ok()
        }),
        Arg::LocateRequirement => {
            get_str(value, "id").is_some_and(|id| {
                serde_json::from_value::<RequirementId>(serde_json::Value::String(id.to_owned())).is_ok()
            }) && get_str(value, "path").is_some_and(|p| fits(p, PATH_MAX_LEN))
        }
        Arg::HostServerConfig => {
            get_str(value, "displayName").is_some_and(|n| fits(n, HOST_NAME_MAX_LEN))
                && value
                    .get("port")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|p| (1..=65535).contains(&p))
        }
        Arg::HostLogQuery => value.is_null() || value.is_object(),
        Arg::GuardedDecision => {
            get_str(value, "id").is_some_and(|id| fits(id, 128))
                && value.get("approve").and_then(serde_json::Value::as_bool).is_some()
        }
        Arg::ModuleId => get_str(value, "moduleId").is_some_and(|id| fits(id, MODULE_ID_MAX_LEN)),
        Arg::ModuleConfigure => get_str(value, "moduleId").is_some_and(|id| fits(id, MODULE_ID_MAX_LEN)),
        Arg::ModuleCommand => {
            get_str(value, "moduleId").is_some_and(|id| fits(id, MODULE_ID_MAX_LEN))
                && get_str(value, "command").is_some_and(|c| fits(c, MODULE_COMMAND_MAX_LEN))
        }
        Arg::HistoryQuery => value.is_null() || value.is_object(),
        Arg::HistoryPreviewRollback => get_str(value, "entryId").is_some_and(|id| fits(id, 128)),
        Arg::HistoryApplyRollback => get_str(value, "planId").is_some_and(|id| fits(id, 128)),
        Arg::RealmId => get_str(value, "realmId").is_some_and(|id| fits(id, REALM_ID_MAX_LEN)),
        Arg::RealmCreate => {
            get_str(value, "name").is_some_and(|n| fits(n, REALM_NAME_MAX_LEN))
                && get_str(value, "moduleId").is_some_and(|m| fits(m, MODULE_ID_MAX_LEN))
        }
        Arg::RealmConfig => get_str(value, "realmId").is_some_and(|id| fits(id, REALM_ID_MAX_LEN)),
        Arg::AdminAccountsQuery => value.is_null() || value.is_object(),
        Arg::AdminAccountCreate => get_str(value, "username").is_some_and(|u| {
            let len = utf16_len(u.trim());
            (USERNAME_MIN_LEN..=USERNAME_MAX_LEN).contains(&len)
        }),
        Arg::AdminAccountStatus => {
            get_str(value, "accountId").is_some_and(|id| fits(id, 128))
                && get_str(value, "status").is_some_and(|s| fits(s, 32))
        }
        Arg::AdminAccountNote => {
            get_str(value, "accountId").is_some_and(|id| fits(id, 128)) && opt_str(value, "note", NOTE_MAX_LEN)
        }
        Arg::AdminAccountDelete => get_str(value, "accountId").is_some_and(|id| fits(id, 128)),
        Arg::AdminInviteCreate => value.is_null() || value.is_object(),
        Arg::AdminInviteCode => get_str(value, "code").is_some_and(|c| fits(c, INVITE_CODE_MAX_LEN)),
        Arg::AdminSessionKick => get_str(value, "sessionId").is_some_and(|id| fits(id, 128)),
        Arg::AdminBroadcast => get_str(value, "message").is_some_and(|m| fits(m, BROADCAST_MAX_LEN)),
        Arg::OptionalUrl => opt_str(value, "url", URL_MAX_LEN),
    }
}

fn get_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}

fn opt_str(value: &serde_json::Value, key: &str, max_utf16: usize) -> bool {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => true,
        Some(v) => v.as_str().is_some_and(|s| within_utf16(s, max_utf16)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::collections::HashSet;

    use super::*;

    #[test]
    fn limits_count_like_javascript() {
        assert!(fits("hello", 5));
        assert!(!fits("   ", 5));
        // Astral symbol takes 2 UTF-16 code units.
        assert!(fits("a\u{1F600}", 3));
        assert!(!fits("a\u{1F600}", 2));
    }

    #[test]
    fn server_ids_are_24_hex() {
        assert!(is_server_id("0123456789abcdef01234567"));
        assert!(!is_server_id("0123456789ABCDEF01234567"));
        assert!(!is_server_id("0123456789abcdef0123456"));
        assert!(!is_server_id("0123456789abcdef012345678"));
        assert!(!is_server_id("0123456789abcdef0123456g"));
    }

    #[test]
    fn names_are_unique() {
        let mut seen = HashSet::new();
        for &cmd in COMMANDS {
            assert!(seen.insert(cmd), "duplicate command name: {cmd}");
        }
        let mut seen_events = HashSet::new();
        for &ev in EVENTS {
            assert!(seen_events.insert(ev), "duplicate event name: {ev}");
        }
    }

    #[test]
    fn name_lists_are_the_enums() {
        assert_eq!(COMMANDS.len(), ALL_COMMANDS.len());
        for (i, &cmd) in ALL_COMMANDS.iter().enumerate() {
            assert_eq!(cmd.name(), COMMANDS[i]);
            assert_eq!(command(COMMANDS[i]), Some(cmd));
        }
        assert_eq!(command("nonexistent_command"), None);
    }

    #[test]
    fn planned_and_server_gui_commands_are_listed() {
        for &name in SERVER_GUI_COMMANDS {
            let cmd = command(name).expect("server gui command must exist");
            assert!(cmd.requires_server_gui());
        }
        for &name in REALM_COMMANDS {
            let cmd = command(name).expect("realm command must exist");
            assert!(cmd.requires_realms());
        }
        for &name in PLANNED {
            let cmd = command(name).expect("planned command must exist");
            assert!(cmd.is_planned());
            assert!(!serves(Build::Full, cmd));
        }
    }

    #[test]
    fn each_command_names_its_build() {
        for &cmd in ALL_COMMANDS {
            let _ = cmd.arg();
            if cmd.is_planned() {
                assert!(!serves(Build::Player, cmd));
                assert!(!serves(Build::Host, cmd));
                assert!(!serves(Build::Full, cmd));
            } else if cmd.requires_server_gui() {
                assert!(!serves(Build::Player, cmd));
                assert!(serves(Build::Host, cmd));
                assert!(serves(Build::Full, cmd));
            } else if cmd.requires_realms() {
                assert!(serves(Build::Player, cmd));
                assert!(!serves(Build::Host, cmd));
                assert!(serves(Build::Full, cmd));
            } else {
                assert!(serves(Build::Player, cmd));
                assert!(serves(Build::Host, cmd));
                assert!(serves(Build::Full, cmd));
            }
        }
    }
}
