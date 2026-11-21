// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The rest of the wire (RD27 spec §2.3.8): Epic and game installs, and the server manager's panels (guarded
//! actions, history, realms, doctor, server config, invites, remote installs), with their answers.

use serde::{Deserialize, Serialize, Serializer};

use super::{Failed, LauncherError, RealmKind, TaskProgress};
use crate::bridge::types::HostServerMode;

/// `{ ok: true, <field>: T } | { ok: false, error }` for a command that answers one value.
macro_rules! outcome {
    ($(#[$doc:meta])* $name:ident, $field:literal, $ty:ty) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq)]
        #[allow(clippy::large_enum_variant, reason = "built once per answer and serialised straight away")]
        pub enum $name {
            /// The command succeeded.
            Ok($ty),
            /// It failed.
            Err(LauncherError),
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeStruct;
                match self {
                    Self::Ok(value) => {
                        let mut s = serializer.serialize_struct(stringify!($name), 2)?;
                        s.serialize_field("ok", &true)?;
                        s.serialize_field($field, value)?;
                        s.end()
                    }
                    Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
                }
            }
        }

        impl From<Result<$ty, LauncherError>> for $name {
            fn from(result: Result<$ty, LauncherError>) -> Self {
                match result {
                    Ok(value) => Self::Ok(value),
                    Err(error) => Self::Err(error),
                }
            }
        }
    };
}

/// The Epic Games sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EpicView {
    /// Signed in.
    pub signed_in: bool,
    /// The Epic account's name.
    pub display_name: Option<String>,
    /// The sign-in window is open.
    pub signing_in: bool,
}

/// Where a game version's install stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    /// Nothing on disk.
    NotInstalled,
    /// A stopped install can go on.
    Resumable,
    /// Reading Epic's file list.
    Preparing,
    /// Downloading.
    Downloading,
    /// Paused by the player.
    Paused,
    /// Checking and moving files into place.
    Finishing,
    /// Done.
    Installed,
    /// Stopped by an error.
    Failed,
}

/// A game version's install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct InstallInfo {
    /// The version.
    pub version: String,
    /// Where it stands.
    pub state: InstallState,
    /// Its folder: where it is, or where it would install.
    pub folder: Option<String>,
    /// How far it has got.
    pub progress: Option<TaskProgress>,
    /// Why it failed.
    pub error: Option<LauncherError>,
}

outcome!(
    /// The `game_install_*` answer.
    InstallOutcome,
    "install",
    InstallInfo
);

/// Where a guarded action stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GuardedState {
    /// Waiting for its delay or an approval.
    Pending,
    /// Approved, not started.
    Approved,
    /// Running.
    Running,
    /// Finished.
    Done,
    /// Finished with an error.
    Failed,
    /// Cancelled.
    Canceled,
    /// Never approved in time.
    Expired,
}

/// What releases a guarded action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GuardedMode {
    /// A waiting time.
    Delay,
    /// A second person's approval.
    Approval,
}

/// A destructive action held back until released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GuardedActionInfo {
    /// Its id, 16 lower-case hex digits.
    pub id: String,
    /// What it does.
    pub kind: String,
    /// One line for the list.
    pub summary: String,
    /// Where it stands.
    pub state: GuardedState,
    /// What releases it.
    pub mode: GuardedMode,
    /// Who asked for it.
    pub initiated_by: String,
    /// The role an approver needs.
    pub needs_role: String,
    /// When it was asked for.
    pub created_at: String,
    /// When its delay ends.
    pub release_at: Option<String>,
    /// When it lapses.
    pub expires_at: String,
    /// Who approved it.
    pub approved_by: Option<String>,
    /// Who cancelled it.
    pub canceled_by: Option<String>,
    /// When it finished.
    pub ended_at: Option<String>,
    /// What it reported.
    pub result: Option<String>,
}

outcome!(
    /// The `guarded_list` answer.
    GuardedList,
    "actions",
    Vec<GuardedActionInfo>
);

outcome!(
    /// The `guarded_approve` and `guarded_cancel` answer.
    GuardedOutcome,
    "action",
    GuardedActionInfo
);

/// One recorded change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HistoryEntryView {
    /// Its id.
    pub id: String,
    /// The database.
    pub database: String,
    /// When (ISO 8601).
    pub at: String,
    /// Who made it.
    pub actor: String,
    /// The table.
    pub table: String,
    /// The row's key.
    pub key: String,
    /// Insert, update or delete.
    pub op: String,
    /// The row before, as JSON.
    pub before: Option<String>,
    /// The row after, as JSON.
    pub after: Option<String>,
}

/// A page of history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HistoryPageView {
    /// The changes, newest first.
    pub entries: Vec<HistoryEntryView>,
    /// The cursor of the next page.
    pub next: Option<String>,
}

outcome!(
    /// The `history_query` answer.
    HistoryOutcome,
    "page",
    HistoryPageView
);

/// A row a revert would write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RevertRowView {
    /// The database.
    pub database: String,
    /// The table.
    pub table: String,
    /// The row's key.
    pub key: String,
    /// Insert, update or delete.
    pub kind: String,
    /// The row now, as JSON.
    pub current: Option<String>,
    /// The row after the revert, as JSON.
    pub target: Option<String>,
}

/// A row a revert would not touch, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RevertConflictView {
    /// The database.
    pub database: String,
    /// The table.
    pub table: String,
    /// The row's key.
    pub key: String,
    /// Why.
    pub reason: String,
    /// The later change in the way.
    pub entry: Option<String>,
    /// Who made it.
    pub by: Option<String>,
    /// When.
    pub at: Option<String>,
    /// More words.
    pub detail: Option<String>,
}

/// What a revert would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevertPlanView {
    /// The plan's id, for `history_revert_apply`.
    pub id: String,
    /// Changes selected.
    pub entries: u64,
    /// Rows it would write.
    pub action_count: u64,
    /// Rows it would skip.
    pub conflict_count: u64,
    /// Rows already as they were.
    pub unchanged_count: u64,
    /// The rows it would write (perhaps cut short).
    pub actions: Vec<RevertRowView>,
    /// The rows it would skip (perhaps cut short).
    pub conflicts: Vec<RevertConflictView>,
    /// The lists were cut short.
    pub truncated: bool,
}

outcome!(
    /// The `history_revert_dry_run` answer.
    RevertPlanOutcome,
    "plan",
    RevertPlanView
);

/// Rows a revert changed in one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RevertedInView {
    /// The database.
    pub database: String,
    /// Rows changed.
    pub rows: u64,
}

/// What a revert did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevertDoneView {
    /// When (ISO 8601).
    pub at: Option<String>,
    /// Rows changed.
    pub rows_changed: u64,
    /// Per database.
    pub databases: Vec<RevertedInView>,
    /// Rows skipped (perhaps cut short).
    pub conflicts: Vec<RevertConflictView>,
    /// Rows skipped.
    pub conflict_count: u64,
    /// The list was cut short.
    pub truncated: bool,
}

outcome!(
    /// The `history_revert_apply` answer.
    RevertApplyOutcome,
    "done",
    RevertDoneView
);

/// Where a realm session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RealmState {
    /// Being set up.
    Starting,
    /// Waiting for players.
    Ready,
    /// Playing.
    Running,
    /// Over.
    Ended,
}

/// A realm session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RealmSessionView {
    /// Its id.
    pub session: String,
    /// Hub or hunt.
    pub kind: RealmKind,
    /// This PC hosts it.
    pub hosting: bool,
    /// Bumped each time it restarts.
    pub generation: u64,
    /// Where it stands.
    pub state: RealmState,
    /// The local port the game connects to.
    pub local_port: Option<u16>,
    /// How it ended.
    pub result: Option<String>,
}

/// The realms panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RealmView {
    /// The server offers realms.
    pub available: bool,
    /// The session in use.
    pub active: Option<RealmSessionView>,
    /// Every known session.
    pub sessions: Vec<RealmSessionView>,
}

outcome!(
    /// The `realm_*` answer.
    RealmOutcome,
    "realm",
    RealmView
);

/// How a doctor check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum DoctorLevel {
    /// Fine.
    Ok,
    /// Works, but should be looked at.
    Warning,
    /// Broken.
    Problem,
}

/// One doctor check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct DoctorCheckView {
    /// How it came out.
    pub level: DoctorLevel,
    /// What was checked.
    pub label: String,
    /// What was found.
    pub detail: String,
}

/// The server doctor's report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct DoctorReportView {
    /// The checks.
    pub checks: Vec<DoctorCheckView>,
    /// How many warned.
    pub warnings: u32,
    /// How many failed.
    pub problems: u32,
}

outcome!(
    /// The `server_doctor` answer.
    ServerDoctorOutcome,
    "report",
    DoctorReportView
);

/// Whether an old MongoDB database was carried over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum MongoMigrationState {
    /// There was none.
    None,
    /// Carried over.
    Migrated,
    /// The host chose not to.
    Skipped,
    /// There is one, not carried over yet.
    Unmigrated,
    /// There is one, but it could not be reached.
    Unreachable,
}

/// The MongoDB migration's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct MongoMigrationView {
    /// Where it stands.
    pub state: MongoMigrationState,
    /// The database found.
    pub source: Option<String>,
    /// More words.
    pub detail: Option<String>,
}

/// The server's config, as the manager edits it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerConfigView {
    /// A config exists.
    pub configured: bool,
    /// The server's name.
    pub name: String,
    /// Private or public.
    pub mode: HostServerMode,
    /// The public address.
    pub public_host: Option<String>,
    /// Who may register.
    pub registration: String,
    /// The database engine.
    pub database: String,
    /// Most hubs at once.
    pub max_hubs: u32,
    /// Most hunts at once.
    pub max_hunts: u32,
    /// The next start wipes the data.
    pub fresh_start: bool,
    /// The MongoDB migration.
    pub mongo_migration: MongoMigrationView,
    /// The config file's path.
    pub config_file: Option<String>,
}

outcome!(
    /// The `server_config_get` and `server_config_save` answer.
    ServerConfigOutcome,
    "config",
    ServerConfigView
);

/// An invite the server holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerInviteEntry {
    /// Its code.
    pub code: String,
    /// The link to hand out. A secret: never logged.
    pub link: String,
    /// Who it is meant for.
    #[serde(rename = "for")]
    pub for_name: Option<String>,
    /// Uses left.
    pub uses_remaining: u32,
    /// It never runs out.
    pub infinite_uses: bool,
    /// When it was made.
    pub created_at: String,
}

outcome!(
    /// The `server_invite_list` answer.
    ServerInviteListOutcome,
    "invites",
    Vec<ServerInviteEntry>
);

/// An install this controller manages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ManagedInstallView {
    /// Its node id.
    pub node: String,
    /// Its region.
    pub region: String,
    /// The host's label for it.
    pub label: Option<String>,
    /// Its state.
    pub state: String,
    /// When it enrolled (Unix ms).
    pub enrolled_at_ms: u64,
    /// When it last reported (Unix ms).
    pub last_seen_ms: u64,
    /// Its access was taken away.
    pub revoked: bool,
    /// The config revision it runs.
    pub applied_revision: u64,
    /// Players on it.
    pub players_online: u32,
    /// Hunts running on it.
    pub active_hunts: u32,
    /// Its version.
    pub version: Option<String>,
    /// Commands waiting for it.
    pub queued_commands: u32,
}

/// This install's place in a fleet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatusView {
    /// Controller, node or neither.
    pub role: String,
    /// Its node id.
    pub node: Option<String>,
    /// Its region.
    pub region: Option<String>,
    /// The controller's address.
    pub controller_url: Option<String>,
    /// The controller certificate's SHA-256.
    pub controller_cert_fp: Option<String>,
    /// The controller's node id.
    pub controller_node: Option<String>,
    /// The config revision.
    pub revision: u64,
    /// Invites not used yet.
    pub pending_enrollments: u32,
    /// Managed installs, for a controller.
    pub installs: Vec<ManagedInstallView>,
    /// The node left its controller.
    pub detached: bool,
}

outcome!(
    /// The `remote_status`, `remote_init_controller`, `remote_enroll`, `remote_revoke` and `remote_leave` answer.
    RemoteStatusOutcome,
    "status",
    RemoteStatusView
);

/// An enrolment invite for a new node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RemoteInviteView {
    /// The URI to give the node. A secret: never logged.
    pub uri: String,
    /// The node id it is for.
    pub node: String,
    /// The node's region.
    pub region: String,
    /// The host's label.
    pub label: Option<String>,
    /// The controller's address.
    pub controller_url: String,
    /// The controller certificate's SHA-256.
    pub controller_fp: String,
    /// When it lapses (Unix ms).
    pub expires_at_ms: u64,
}

outcome!(
    /// The `remote_invite` answer.
    RemoteInviteOutcome,
    "invite",
    RemoteInviteView
);

/// A command queued for a managed install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RemoteExecView {
    /// Its id.
    pub command_id: String,
    /// The node.
    pub node: String,
    /// What it asks.
    pub action: String,
    /// Where it stands.
    pub status: String,
}

outcome!(
    /// The `remote_exec` answer.
    RemoteExecOutcome,
    "result",
    RemoteExecView
);

/// What uninstalling a version deletes, for the page's confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UninstallPlan {
    /// The version.
    pub version: String,
    /// Its folder.
    pub folder: String,
    /// Bytes freed.
    pub bytes: u64,
    /// The install made the folder, so all of it goes; otherwise only the game's files and the launcher's own.
    pub whole_folder: bool,
}

outcome!(
    /// The `game_uninstall_plan` answer.
    UninstallOutcome,
    "plan",
    UninstallPlan
);
