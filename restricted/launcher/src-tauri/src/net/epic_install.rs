//! The job around one install from Epic (DE5, DD22), on launcher-kit's `net::epic_install` (RL2): its progress
//! as the page's install task.

use std::sync::Arc;

use epic::download::{DownloadError, Gate, Installed, Options, Stage, Target};
use launcher_kit::net::epic_install as kit;

use crate::bridge::types::{TaskKind, TaskProgress};

pub type OnProgress = Arc<dyn Fn(TaskProgress) + Send + Sync>;

#[derive(Clone)]
pub struct InstallJob {
    job: Arc<kit::InstallJob>,
    on_progress: OnProgress,
}

fn task(progress: kit::InstallProgress) -> TaskProgress {
    TaskProgress {
        kind: TaskKind::Install,
        total_bytes: progress.total_bytes,
        done_bytes: progress.done_bytes,
        files_total: progress.files_total,
        files_done: progress.files_done,
        bytes_per_second: progress.bytes_per_second as f64,
        eta_seconds: progress.eta_seconds.map(|seconds| seconds as f64),
        current_file: progress.current_file,
    }
}

impl InstallJob {
    pub fn new(on_progress: OnProgress) -> Self {
        Self {
            job: Arc::new(kit::InstallJob::new()),
            on_progress,
        }
    }

    pub fn pause(&self) -> bool {
        self.job.pause()
    }

    pub fn resume(&self) -> bool {
        self.job.resume()
    }

    pub fn cancel(&self) -> bool {
        self.job.cancel()
    }

    pub fn progress(&self) -> Option<TaskProgress> {
        self.job.progress().map(task)
    }

    pub fn stage(&self) -> Option<Stage> {
        self.job.last().map(|last| last.stage)
    }

    pub async fn run(
        &self,
        target: &Target<'_>,
        options: &Options,
        gate: &impl Gate,
    ) -> Result<Installed, DownloadError> {
        let on_progress = self.on_progress.clone();
        self.job
            .run(target, options, gate, &move |progress| on_progress(task(progress)))
            .await
    }
}
