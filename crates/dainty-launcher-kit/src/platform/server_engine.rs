// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The manager's in-process host kit (RD27 spec §4.3): what the embedded server's own crate does with a
//! workspace, behind one trait, so the controller keeps the jobs, the checks and the page's events. The blocking
//! calls run on a blocking thread; `lines` gets the host kit's raw output.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::bridge::checks::{
    HistoryFilter, RemoteEnrollRequest, RemoteExecRequest, RemoteInitRequest, RemoteInviteRequest, ServerConfigForm,
};
use crate::bridge::types::{
    DoctorReportView, HistoryOutcome, RemoteExecView, RemoteInviteView, RemoteStatusView, RevertApplyOutcome,
    RevertPlanOutcome, ServerConfigView, ServerInviteEntry,
};

type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;
/// Where a command's output goes, a line or several at a time.
pub type Lines<'a> = &'a mut dyn FnMut(&str);

/// Implemented by the manager's embedded server; every error is a one-line reason for the page.
pub trait ServerEngine: Send + Sync {
    /// One host kit command (`setup`, `invite`, `multipliers`, `gamedata`, `publish-runtime`, `update`) with its
    /// arguments, as `dainty-host` takes them. `Ok(false)`: it ran and failed.
    ///
    /// # Errors
    ///
    /// When it could not run.
    fn run_kit(&self, workspace: &Path, args: &[String], lines: Lines<'_>) -> Result<bool, String>;
    /// The doctor's checks.
    ///
    /// # Errors
    ///
    /// When the checks could not run.
    fn doctor(&self, workspace: &Path) -> Result<DoctorReportView, String>;
    /// A backup of the server's data. `Ok(false)`: it ran and failed.
    ///
    /// # Errors
    ///
    /// When it could not run.
    fn backup(&self, workspace: &Path, lines: Lines<'_>) -> Result<bool, String>;
    /// Moves a MongoDB server's data to the configured store, then verifies it.
    ///
    /// # Errors
    ///
    /// When either step fails.
    fn migrate_mongo(&self, workspace: &Path, lines: Lines<'_>) -> Result<(), String>;
    /// The server's settings.
    ///
    /// # Errors
    ///
    /// When they cannot be read.
    fn config_get(&self, workspace: &Path) -> Result<ServerConfigView, String>;
    /// Saves the form and rewrites the services' files; answers the settings after.
    ///
    /// # Errors
    ///
    /// When they cannot be saved.
    fn config_save(&self, workspace: &Path, form: &ServerConfigForm) -> Result<ServerConfigView, String>;
    /// Confirms that the next start wipes the data.
    ///
    /// # Errors
    ///
    /// When the marker cannot be written.
    fn confirm_fresh_start(&self, workspace: &Path) -> Result<ServerConfigView, String>;
    /// The running server's invite codes, as links.
    ///
    /// # Errors
    ///
    /// When the server cannot be asked.
    fn invite_list(&self, workspace: &Path) -> Result<Vec<ServerInviteEntry>, String>;
    /// This install's remote management state.
    ///
    /// # Errors
    ///
    /// When it cannot be read.
    fn remote_status(&self, workspace: &Path) -> Result<RemoteStatusView, String>;
    /// Makes this install a controller.
    ///
    /// # Errors
    ///
    /// When it cannot.
    fn remote_init(&self, workspace: &Path, request: &RemoteInitRequest) -> Result<RemoteStatusView, String>;
    /// An enrolment invite for a new node.
    ///
    /// # Errors
    ///
    /// When it cannot be made.
    fn remote_invite(&self, workspace: &Path, request: &RemoteInviteRequest) -> Result<RemoteInviteView, String>;
    /// Enrols this install with a controller.
    ///
    /// # Errors
    ///
    /// When the controller refuses or cannot be reached.
    fn remote_enroll(&self, workspace: &Path, request: &RemoteEnrollRequest) -> Result<RemoteStatusView, String>;
    /// Revokes a node.
    ///
    /// # Errors
    ///
    /// When it cannot.
    fn remote_revoke(&self, workspace: &Path, node: &str) -> Result<RemoteStatusView, String>;
    /// Leaves the controller.
    ///
    /// # Errors
    ///
    /// When it cannot.
    fn remote_leave(&self, workspace: &Path) -> Result<RemoteStatusView, String>;
    /// Queues a command on a node.
    ///
    /// # Errors
    ///
    /// When it cannot be queued.
    fn remote_exec(&self, workspace: &Path, request: &RemoteExecRequest) -> Result<RemoteExecView, String>;
    /// A page of the server's change history.
    fn history_query(&self, workspace: PathBuf, filter: HistoryFilter) -> Boxed<HistoryOutcome>;
    /// Plans a revert of the selected changes.
    fn history_dry_run(&self, workspace: PathBuf, selection: HistoryFilter) -> Boxed<RevertPlanOutcome>;
    /// Applies a planned revert by its id.
    fn history_apply(&self, workspace: PathBuf, plan: String) -> Boxed<RevertApplyOutcome>;
}
