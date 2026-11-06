// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The Epic install job: `epic::download::install` with pause, resume and cancel from the page, and its progress
//! turned into the page's task progress (kind `install`) with a rate and an ETA.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use epic::download::{Control, DownloadError, Gate, Installed, Options, Progress, Stage, Target, install};

/// How far back the rate looks.
const WINDOW: Duration = Duration::from_secs(5);
/// Byte counts closer together than this share a bucket.
const BUCKET: Duration = Duration::from_millis(250);
/// The shortest span the rate divides by, so the first few chunks don't read as a burst.
const MIN_SPAN: Duration = Duration::from_secs(1);
/// How often progress is reported while the installer is quiet.
const TICK: Duration = Duration::from_millis(250);

/// The page's task progress for an install; the shell sends it with kind `install`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallProgress {
    /// `.chunk` bytes the install needs.
    pub total_bytes: u64,
    /// `.chunk` bytes fetched.
    pub done_bytes: u64,
    /// Files in the manifest.
    pub files_total: u32,
    /// Files checked.
    pub files_done: u32,
    /// Download rate; 0 unless downloading.
    pub bytes_per_second: u64,
    /// Seconds left at that rate, when it is positive.
    pub eta_seconds: Option<u64>,
    /// Always none: chunks span files, so no one file is current.
    pub current_file: Option<String>,
}

/// `progress` at `rate` bytes a second.
pub fn task_progress(progress: &Progress, rate: f64) -> InstallProgress {
    let rate = if progress.stage == Stage::Downloading && rate > 0.0 {
        rate
    } else {
        0.0
    };
    let left = progress.download_total.saturating_sub(progress.downloaded);
    InstallProgress {
        total_bytes: progress.download_total,
        done_bytes: progress.downloaded,
        files_total: u32::try_from(progress.files_total).unwrap_or(u32::MAX),
        files_done: u32::try_from(progress.files_done).unwrap_or(u32::MAX),
        bytes_per_second: rate.round() as u64,
        eta_seconds: (rate > 0.0).then(|| (left as f64 / rate).ceil() as u64),
        current_file: None,
    }
}

/// Bytes fetched over the last [`WINDOW`], in [`BUCKET`]s.
#[derive(Debug, Default)]
struct Rate {
    buckets: VecDeque<(Instant, u64)>,
    last: Option<u64>,
}

impl Rate {
    fn record(&mut self, downloaded: u64, now: Instant) {
        // The first count after a start or a reset is a baseline: resumed bytes were not fetched now
        let Some(last) = self.last.replace(downloaded) else {
            self.buckets.push_back((now, 0));
            return;
        };
        let added = downloaded.saturating_sub(last);
        match self.buckets.back_mut() {
            Some((start, bytes)) if now.saturating_duration_since(*start) < BUCKET => *bytes += added,
            _ => self.buckets.push_back((now, added)),
        }
        self.trim(now);
    }

    fn trim(&mut self, now: Instant) {
        while self
            .buckets
            .front()
            .is_some_and(|(start, _)| now.saturating_duration_since(*start) > WINDOW)
        {
            self.buckets.pop_front();
        }
    }

    fn per_second(&mut self, now: Instant) -> f64 {
        self.trim(now);
        let Some((oldest, _)) = self.buckets.front() else {
            return 0.0;
        };
        let span = now.saturating_duration_since(*oldest).max(MIN_SPAN);
        let bytes: u64 = self.buckets.iter().map(|(_, bytes)| bytes).sum();
        bytes as f64 / span.as_secs_f64()
    }

    fn reset(&mut self) {
        self.buckets.clear();
        self.last = None;
    }
}

#[derive(Debug, Default)]
struct State {
    last: Option<Progress>,
    rate: Rate,
}

/// One install, controllable from other tasks while [`InstallJob::run`] runs.
#[derive(Default)]
pub struct InstallJob {
    control: Control,
    state: Mutex<State>,
}

impl InstallJob {
    /// A job not yet run.
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops fetching; false when it was not running.
    pub fn pause(&self) -> bool {
        let applied = self.control.pause();
        if applied {
            self.state().rate.reset();
        }
        applied
    }

    /// Fetches again; false when it was not paused, which includes after a cancel.
    pub fn resume(&self) -> bool {
        self.control.resume()
    }

    /// Stops for good; false when it already was.
    pub fn cancel(&self) -> bool {
        self.control.cancel()
    }

    /// The installer's last report, as it gave it.
    pub fn last(&self) -> Option<Progress> {
        self.state().last.clone()
    }

    /// The last report as task progress, at the current rate.
    pub fn progress(&self) -> Option<InstallProgress> {
        self.progress_at(Instant::now())
    }

    fn progress_at(&self, now: Instant) -> Option<InstallProgress> {
        let mut state = self.state();
        let rate = state.rate.per_second(now);
        state.last.as_ref().map(|last| task_progress(last, rate))
    }

    fn observe(&self, progress: &Progress, now: Instant) {
        let mut state = self.state();
        state.rate.record(progress.downloaded, now);
        state.last = Some(progress.clone());
    }

    /// Installs `target`, calling `report` on every update from the installer and every 250 ms until it ends,
    /// then once more.
    pub async fn run(
        &self,
        target: &Target<'_>,
        options: &Options,
        gate: &impl Gate,
        report: &(dyn Fn(InstallProgress) + Send + Sync),
    ) -> Result<Installed, DownloadError> {
        let on_progress = |progress: &Progress| {
            self.observe(progress, Instant::now());
            if let Some(task) = self.progress() {
                report(task);
            }
        };
        let installing = install(target, options, gate, &self.control, &on_progress);
        tokio::pin!(installing);
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let result = loop {
            tokio::select! {
                result = &mut installing => break result,
                _ = ticker.tick() => {
                    if let Some(task) = self.progress() {
                        report(task);
                    }
                }
            }
        };
        if let Some(task) = self.progress() {
            report(task);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::path::Path;
    use std::sync::Arc;

    use epic::download::Refusal;
    use epic::testing::{Host, cdn, version};

    use super::*;

    struct Allowed;

    impl Gate for Allowed {
        async fn require(&self) -> Result<(), Refusal> {
            Ok(())
        }
    }

    fn progress(stage: Stage, downloaded: u64) -> Progress {
        Progress {
            stage,
            download_total: 10_000,
            downloaded,
            chunks_total: 4,
            chunks_done: 1,
            files_total: 7,
            files_done: 2,
            install_total: 20_000,
        }
    }

    #[test]
    fn each_control_applies_once_and_cancel_ends_resume() {
        let job = InstallJob::new();
        assert!(!job.resume());
        assert!(job.pause());
        assert!(!job.pause());
        assert!(job.resume());
        assert!(!job.resume());
        assert!(job.pause());
        assert!(job.cancel());
        assert!(!job.cancel());
        assert!(!job.resume());
        assert!(!job.pause());
    }

    #[test]
    fn progress_carries_the_download_totals_with_a_rate_only_while_downloading() {
        let task = task_progress(&progress(Stage::Downloading, 4_000), 1_500.0);
        assert_eq!(
            task,
            InstallProgress {
                total_bytes: 10_000,
                done_bytes: 4_000,
                files_total: 7,
                files_done: 2,
                bytes_per_second: 1_500,
                eta_seconds: Some(4),
                current_file: None,
            }
        );
        for stage in [Stage::Preparing, Stage::Paused, Stage::Finishing, Stage::Done] {
            let task = task_progress(&progress(stage, 4_000), 1_500.0);
            assert_eq!((task.bytes_per_second, task.eta_seconds), (0, None), "{stage:?}");
        }
        assert_eq!(task_progress(&progress(Stage::Downloading, 0), 0.0).eta_seconds, None);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn file_counts_saturate() {
        let mut huge = progress(Stage::Downloading, 0);
        huge.files_total = usize::MAX;
        huge.files_done = u32::MAX as usize + 1;
        let task = task_progress(&huge, 0.0);
        assert_eq!((task.files_total, task.files_done), (u32::MAX, u32::MAX));
    }

    #[test]
    fn the_rate_spans_at_least_a_second_and_groups_quarter_seconds() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut rate = Rate::default();
        // A resumed install starts part way: that is a baseline, not a burst
        rate.record(5_000, at(0));
        assert_eq!(rate.per_second(at(0)), 0.0);
        rate.record(5_500, at(100));
        rate.record(6_000, at(200));
        assert_eq!(rate.buckets.len(), 1);
        assert_eq!(rate.per_second(at(500)), 1_000.0);
        rate.record(8_000, at(300));
        assert_eq!(rate.buckets.len(), 2);
        assert_eq!(rate.per_second(at(2_000)), 1_500.0);
    }

    #[test]
    fn the_rate_forgets_what_is_older_than_five_seconds() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut rate = Rate::default();
        rate.record(0, at(0));
        for s in 1..=8 {
            rate.record(s * 1_000, at(s));
        }
        // At 8 s only the buckets started at 3..=8 s remain: 6,000 bytes over 5 s
        assert_eq!(rate.per_second(at(8)), 1_200.0);
        assert_eq!(rate.per_second(at(20)), 0.0);
    }

    #[test]
    fn a_pause_resets_the_rate() {
        let job = InstallJob::new();
        let start = Instant::now();
        job.observe(&progress(Stage::Downloading, 0), start);
        job.observe(&progress(Stage::Downloading, 3_000), start + Duration::from_secs(1));
        let now = start + Duration::from_secs(2);
        assert_eq!(job.progress_at(now).unwrap().bytes_per_second, 1_500);
        assert!(job.pause());
        assert_eq!(job.progress_at(now).unwrap().bytes_per_second, 0);
        // The first count after the pause is again a baseline
        job.observe(&progress(Stage::Downloading, 3_000), now);
        job.observe(&progress(Stage::Downloading, 4_000), now + Duration::from_secs(2));
        assert_eq!(
            job.progress_at(now + Duration::from_secs(2)).unwrap().bytes_per_second,
            500
        );
    }

    fn options(cdn: &epic::testing::Cdn) -> Options {
        let mut options = Options::new(Arc::new(|_: &Path| Ok(u64::MAX)));
        options.bases = cdn.bases.clone();
        options.tls = Some(cdn.tls.clone());
        options.backoff = Arc::new(|_| Duration::ZERO);
        options.timeout = Duration::from_secs(10);
        options
    }

    fn target<'a>(v: &'a epic::testing::Version, dir: &Path) -> Target<'a> {
        Target {
            manifest: &v.manifest,
            manifest_sha1: v.pin,
            folder: dir.join("games").join("1.14.7"),
            executable: None,
            resume_file: dir.join("1.14.7.resume"),
        }
    }

    #[tokio::test]
    async fn a_run_reports_until_done_and_answers_the_installers_result() {
        let v = version();
        let cdn = cdn(&[Host::Good], v.files.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let job = InstallJob::new();
        let reports = Mutex::new(Vec::new());
        let report = |task: InstallProgress| reports.lock().unwrap().push(task);
        let installed = job
            .run(&target(&v, dir.path()), &options(&cdn), &Allowed, &report)
            .await
            .unwrap();
        assert_eq!(installed.files, 5);
        assert_eq!(job.last().unwrap().stage, Stage::Done);
        let reports = reports.into_inner().unwrap();
        let last = reports.last().unwrap();
        assert_eq!(last.done_bytes, last.total_bytes);
        assert_eq!((last.files_done, last.files_total), (5, 5));
        assert_eq!((last.bytes_per_second, last.eta_seconds), (0, None));
        assert!(reports.iter().all(|task| task.current_file.is_none()));
    }

    #[tokio::test]
    async fn a_cancelled_job_answers_the_installers_cancel() {
        let v = version();
        let cdn = cdn(&[Host::Stall(1)], v.files.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let job = Arc::new(InstallJob::new());
        let canceller = {
            let job = Arc::clone(&job);
            tokio::spawn(async move {
                while job.last().is_none_or(|p| p.downloaded == 0) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                assert!(job.cancel());
            })
        };
        let result = job
            .run(&target(&v, dir.path()), &options(&cdn), &Allowed, &|_| ())
            .await;
        assert_eq!(result, Err(DownloadError::Cancelled));
        canceller.await.unwrap();
    }
}
