// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The manager's server panels and its in-process host kit (RD27 spec §4.3): the workspace checks, the jobs and
//! the page's events here, the work in the embedded server's [`ServerEngine`].

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Controller, Outcome, failure};
use crate::bridge::checks::{
    HistoryFilter, RemoteEnrollRequest, RemoteExecRequest, RemoteInitRequest, RemoteInviteRequest, ServerConfigForm,
};
use crate::bridge::types::{
    ActionResult, ErrorCode, HistoryOutcome, HostJobKind, HostServerState, LauncherError, RemoteExecOutcome,
    RemoteInviteOutcome, RemoteStatusOutcome, RevertApplyOutcome, RevertPlanOutcome, ServerConfigOutcome,
    ServerDoctorOutcome, ServerInviteListOutcome,
};
use crate::platform::hostkit::WorkspacePaths;
use crate::platform::hostkit::parse::{clean_line, failure_line};
use crate::platform::hostserver::{CommandOutput, HostServer};
use crate::platform::server_engine::ServerEngine;

/// A command's output: cleaned, kept, appended to its log file and shown in the server panel.
struct KitLog {
    file: Option<std::fs::File>,
    lines: Vec<String>,
    server: HostServer,
    /// `invite --json` answers a secret on a JSON line, kept from the log and the panel.
    hide_json: bool,
}

impl KitLog {
    fn open(path: &Path, server: HostServer, hide_json: bool) -> Self {
        let file = path.parent().and_then(|dir| {
            std::fs::create_dir_all(dir).ok()?;
            std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
        });
        Self {
            file,
            lines: Vec::new(),
            server,
            hide_json,
        }
    }

    fn write(&mut self, line: &str) {
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{line}");
        }
    }

    fn emit(&mut self, raw: &str) {
        for part in raw.lines() {
            let clean = clean_line(part);
            if clean.is_empty() {
                continue;
            }
            if !(self.hide_json && clean.trim_start().starts_with('{')) {
                self.write(&clean);
                self.server.note(&clean);
            }
            self.lines.push(clean);
        }
    }
}

type Engine = Arc<dyn ServerEngine>;

fn no_engine() -> LauncherError {
    LauncherError::with_detail(ErrorCode::NotInThisBuild, "server_gui")
}

impl Controller {
    fn engine(&self) -> Option<Engine> {
        self.host().server.engine()
    }

    /// The workspace and the engine, or the refusal the page shows.
    fn manager_workspace(&self) -> Result<(PathBuf, Engine), LauncherError> {
        let engine = self.engine().ok_or_else(no_engine)?;
        match self.valid_workspace() {
            Some(workspace) => Ok((workspace, engine)),
            None => Err(self.refuse(self.workspace_failure())),
        }
    }

    /// One host kit command in this process, logged as a child's would be.
    pub(super) async fn run_kit_in_process(
        &self,
        workspace: &Path,
        args: Vec<String>,
        log_name: &str,
    ) -> CommandOutput {
        let Some(engine) = self.engine() else {
            let detail = "not_in_this_build".to_owned();
            return CommandOutput {
                code: 1,
                lines: vec![detail.clone()],
                failure: Some(detail),
            };
        };
        let command = args.first().cloned().unwrap_or_default();
        let server = self.host().server.clone();
        server.note(&format!("> {command}"));
        let log_file = WorkspacePaths::new(workspace).launcher_log(log_name);
        let workspace = workspace.to_path_buf();
        let ran = tokio::task::spawn_blocking(move || {
            let mut log = KitLog::open(&log_file, server, command == "invite");
            log.write(&format!(
                "\n> embedded-host {}  (in {}, {})",
                args.join(" "),
                workspace.display(),
                chrono::Utc::now().to_rfc3339()
            ));
            let ok = match engine.run_kit(&workspace, &args, &mut |raw| log.emit(raw)) {
                Ok(ok) => ok,
                Err(error) => {
                    log.emit(&format!("Error: {error}"));
                    false
                }
            };
            let code = i32::from(!ok);
            log.write(&format!("(exit code {code})"));
            let failure = if ok { None } else { failure_line(&log.lines) };
            CommandOutput {
                code,
                lines: log.lines,
                failure,
            }
        })
        .await;
        ran.unwrap_or_else(|error| CommandOutput {
            code: 1,
            lines: vec![error.to_string()],
            failure: Some(error.to_string()),
        })
    }

    /// A job whose engine call writes lines.
    async fn engine_job<F>(&self, kind: HostJobKind, log_name: &str, work: F) -> ActionResult
    where
        F: FnOnce(&dyn ServerEngine, &Path, &mut KitLog) -> Outcome + Send + 'static,
    {
        // As busy as any other job, but no banner: the page shows the running one
        if self.job_running() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let (workspace, engine) = match self.manager_workspace() {
            Ok(found) => found,
            Err(error) => return ActionResult::Err(error),
        };
        let log_file = WorkspacePaths::new(&workspace).launcher_log(log_name);
        if !self.begin_job(kind, None, Some(&log_file)).await {
            return ActionResult::err(ErrorCode::Busy);
        }
        let server = self.host().server.clone();
        let ran = tokio::task::spawn_blocking(move || {
            let mut log = KitLog::open(&log_file, server, false);
            work(engine.as_ref(), &workspace, &mut log)
        })
        .await;
        let outcome = ran.unwrap_or_else(|error| Err(Some(error.to_string())));
        self.end_job(kind, outcome).await
    }

    /// Runs `work` on a blocking thread with the workspace; a failure is a `host_failed` the page shows.
    async fn engine_call<T, F>(&self, emit: bool, work: F) -> Result<T, LauncherError>
    where
        T: Send + 'static,
        F: FnOnce(&dyn ServerEngine, &Path) -> Result<T, String> + Send + 'static,
    {
        let (workspace, engine) = self.manager_workspace()?;
        let ran = tokio::task::spawn_blocking(move || work(engine.as_ref(), &workspace)).await;
        let result = ran.unwrap_or_else(|error| Err(error.to_string()));
        if emit {
            self.emit_kit().await;
        }
        result.map_err(|why| self.refuse(failure(&why)))
    }

    /// `server_doctor`.
    pub async fn server_doctor(&self) -> ServerDoctorOutcome {
        self.engine_call(false, |engine, ws| engine.doctor(ws)).await.into()
    }

    /// `server_backup`, as a job.
    pub async fn server_backup(&self) -> ActionResult {
        self.engine_job(HostJobKind::Backup, "backup", |engine, ws, log| {
            match engine.backup(ws, &mut |raw| log.emit(raw)) {
                Ok(true) => Ok(log.lines.last().cloned()),
                Ok(false) => Err(Some(log.lines.last().cloned().unwrap_or_else(|| "backup".into()))),
                Err(error) => {
                    log.emit(&format!("Error: {error}"));
                    Err(Some(error))
                }
            }
        })
        .await
    }

    /// `server_config_get`.
    pub async fn server_config_get(&self) -> ServerConfigOutcome {
        self.engine_call(false, |engine, ws| engine.config_get(ws)).await.into()
    }

    /// `server_config_save`.
    pub async fn server_config_save(&self, form: ServerConfigForm) -> ServerConfigOutcome {
        self.engine_call(true, move |engine, ws| engine.config_save(ws, &form))
            .await
            .into()
    }

    /// `server_migrate_mongo`, as a job, with the server stopped.
    pub async fn server_migrate_mongo(&self) -> ActionResult {
        if self.engine().is_some() && !self.job_running() && self.server_runs() {
            return ActionResult::Err(self.refuse(failure("server_running")));
        }
        self.engine_job(
            HostJobKind::MigrateMongo,
            "migrate-mongo",
            |engine, ws, log| match engine.migrate_mongo(ws, &mut |raw| log.emit(raw)) {
                Ok(()) => Ok(log.lines.last().cloned()),
                Err(error) => {
                    log.emit(&format!("Error: {error}"));
                    Err(Some(error))
                }
            },
        )
        .await
    }

    /// `server_confirm_fresh_start`.
    pub async fn server_confirm_fresh_start(&self) -> ServerConfigOutcome {
        self.engine_call(true, |engine, ws| engine.confirm_fresh_start(ws))
            .await
            .into()
    }

    /// `server_invite_list`: only while the server runs.
    pub async fn server_invite_list(&self) -> ServerInviteListOutcome {
        if self.engine().is_some()
            && self.valid_workspace().is_some()
            && !matches!(
                self.server_state(),
                HostServerState::Running | HostServerState::External
            )
        {
            return ServerInviteListOutcome::Err(self.refuse(LauncherError::new(ErrorCode::ServerStopped)));
        }
        self.engine_call(false, |engine, ws| engine.invite_list(ws))
            .await
            .into()
    }

    /// `remote_status`.
    pub async fn remote_status(&self) -> RemoteStatusOutcome {
        self.engine_call(false, |engine, ws| engine.remote_status(ws))
            .await
            .into()
    }

    /// `remote_init_controller`.
    pub async fn remote_init_controller(&self, request: RemoteInitRequest) -> RemoteStatusOutcome {
        self.engine_call(true, move |engine, ws| engine.remote_init(ws, &request))
            .await
            .into()
    }

    /// `remote_invite`.
    pub async fn remote_invite(&self, request: RemoteInviteRequest) -> RemoteInviteOutcome {
        self.engine_call(false, move |engine, ws| engine.remote_invite(ws, &request))
            .await
            .into()
    }

    /// `remote_enroll`.
    pub async fn remote_enroll(&self, request: RemoteEnrollRequest) -> RemoteStatusOutcome {
        self.engine_call(true, move |engine, ws| engine.remote_enroll(ws, &request))
            .await
            .into()
    }

    /// `remote_revoke`.
    pub async fn remote_revoke(&self, node: String) -> RemoteStatusOutcome {
        self.engine_call(false, move |engine, ws| engine.remote_revoke(ws, &node))
            .await
            .into()
    }

    /// `remote_leave`.
    pub async fn remote_leave(&self) -> RemoteStatusOutcome {
        self.engine_call(true, |engine, ws| engine.remote_leave(ws))
            .await
            .into()
    }

    /// `remote_exec`.
    pub async fn remote_exec(&self, request: RemoteExecRequest) -> RemoteExecOutcome {
        self.engine_call(false, move |engine, ws| engine.remote_exec(ws, &request))
            .await
            .into()
    }

    /// The history commands' workspace; their refusal names `workspace` and sets no banner.
    fn history_workspace(&self) -> Result<(PathBuf, Engine), LauncherError> {
        let engine = self.engine().ok_or_else(no_engine)?;
        self.valid_workspace()
            .map(|workspace| (workspace, engine))
            .ok_or_else(|| LauncherError::with_detail(ErrorCode::HostFailed, "workspace"))
    }

    /// `history_query`.
    pub async fn history_query(&self, filter: HistoryFilter) -> HistoryOutcome {
        match self.history_workspace() {
            Ok((workspace, engine)) => engine.history_query(workspace, filter).await,
            Err(error) => HistoryOutcome::Err(error),
        }
    }

    /// `history_revert_dry_run`.
    pub async fn history_revert_dry_run(&self, selection: HistoryFilter) -> RevertPlanOutcome {
        match self.history_workspace() {
            Ok((workspace, engine)) => engine.history_dry_run(workspace, selection).await,
            Err(error) => RevertPlanOutcome::Err(error),
        }
    }

    /// `history_revert_apply`.
    pub async fn history_revert_apply(&self, plan: String) -> RevertApplyOutcome {
        match self.history_workspace() {
            Ok((workspace, engine)) => engine.history_apply(workspace, plan).await,
            Err(error) => RevertApplyOutcome::Err(error),
        }
    }
}
