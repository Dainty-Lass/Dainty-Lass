//! The job around one install from Epic (DE5, DD22): Dauntless Revived's `net/downloader.rs` (DR L23, at
//! `e1c5fec`) brought back for it, as RD22's amendment says. DR's structure is kept where it fits: the shared
//! `Inner` and its locked `State`, `pause` / `resume` / `cancel` that are no-ops when they don't apply, the
//! `SpeedMeter` and the progress ticker. What DR's job did per file, the MIT `dainty_epic::download::install` does per
//! chunk (fetching, checking, writing, the resume file); this AGPL module only drives it (DD26).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use dainty_epic::download::{Control, DownloadError, Gate, Installed, Progress, Stage, Target};

use crate::bridge::types::{TaskKind, TaskProgress};

/// Bytes per second over the last few seconds (DR's, unchanged).
pub struct SpeedMeter {
    buckets: VecDeque<(Instant, u64)>,
    window: Duration,
}

impl SpeedMeter {
    pub fn new(window: Duration) -> Self {
        Self {
            buckets: VecDeque::new(),
            window,
        }
    }

    pub fn add_at(&mut self, now: Instant, bytes: u64) {
        match self.buckets.back_mut() {
            Some((at, total)) if now.duration_since(*at) < Duration::from_millis(250) => *total += bytes,
            _ => self.buckets.push_back((now, bytes)),
        }
        self.trim(now);
    }

    pub fn rate_at(&mut self, now: Instant) -> f64 {
        self.trim(now);
        let Some((first, _)) = self.buckets.front() else {
            return 0.0;
        };
        let span = now.duration_since(*first).max(Duration::from_secs(1));
        let bytes: u64 = self.buckets.iter().map(|(_, bytes)| bytes).sum();
        bytes as f64 / span.as_secs_f64()
    }

    pub fn reset(&mut self) {
        self.buckets.clear();
    }

    fn trim(&mut self, now: Instant) {
        while self
            .buckets
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > self.window)
        {
            self.buckets.pop_front();
        }
    }
}

pub type OnProgress = Arc<dyn Fn(TaskProgress) + Send + Sync>;

struct State {
    last: Option<Progress>,
    speed: SpeedMeter,
}

struct Inner {
    control: Control,
    state: Mutex<State>,
    on_progress: OnProgress,
    progress_interval: Duration,
}

#[derive(Clone)]
pub struct InstallJob {
    inner: Arc<Inner>,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn progress(&self) -> Option<TaskProgress> {
        let mut state = self.state();
        let last = state.last.clone()?;
        let rate = if last.stage == Stage::Downloading {
            state.speed.rate_at(Instant::now())
        } else {
            0.0
        };
        let left = last.download_total.saturating_sub(last.downloaded);
        Some(TaskProgress {
            kind: TaskKind::Install,
            total_bytes: last.download_total,
            done_bytes: last.downloaded,
            files_total: u32::try_from(last.files_total).unwrap_or(u32::MAX),
            files_done: u32::try_from(last.files_done).unwrap_or(u32::MAX),
            bytes_per_second: rate,
            eta_seconds: (rate > 0.0).then(|| left as f64 / rate),
            current_file: None,
        })
    }

    fn emit(&self) {
        if let Some(progress) = self.progress() {
            (self.on_progress)(progress);
        }
    }

    fn update(&self, progress: &Progress) {
        {
            let mut state = self.state();
            let before = state.last.as_ref().map_or(0, |last| last.downloaded);
            if progress.downloaded > before {
                state.speed.add_at(Instant::now(), progress.downloaded - before);
            }
            if progress.stage == Stage::Paused {
                state.speed.reset();
            }
            state.last = Some(progress.clone());
        }
        self.emit();
    }
}

impl InstallJob {
    pub fn new(on_progress: OnProgress) -> Self {
        Self {
            inner: Arc::new(Inner {
                control: Control::default(),
                state: Mutex::new(State {
                    last: None,
                    speed: SpeedMeter::new(Duration::from_secs(5)),
                }),
                on_progress,
                progress_interval: Duration::from_millis(250),
            }),
        }
    }

    pub fn pause(&self) -> bool {
        self.inner.control.pause()
    }

    pub fn resume(&self) -> bool {
        self.inner.control.resume()
    }

    pub fn cancel(&self) -> bool {
        self.inner.control.cancel()
    }

    /// The last progress seen, as the page draws it.
    pub fn progress(&self) -> Option<TaskProgress> {
        self.inner.progress()
    }

    pub fn stage(&self) -> Option<Stage> {
        self.inner.state().last.as_ref().map(|last| last.stage)
    }

    pub async fn run(
        &self,
        target: &Target<'_>,
        options: &dainty_epic::download::Options,
        gate: &impl Gate,
    ) -> Result<Installed, DownloadError> {
        let inner = &self.inner;
        // Speed and ETA move between chunks too (DR's ticker)
        let ticker = {
            let inner = inner.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(inner.progress_interval);
                loop {
                    interval.tick().await;
                    inner.emit();
                }
            })
        };
        let on_progress = |progress: &Progress| inner.update(progress);
        let result = dainty_epic::download::install(target, options, gate, &inner.control, &on_progress).await;
        ticker.abort();
        inner.emit();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speed_meter_averages_over_its_window() {
        let start = Instant::now();
        let mut meter = SpeedMeter::new(Duration::from_secs(5));
        meter.add_at(start, 1000);
        meter.add_at(start + Duration::from_millis(100), 1000);
        meter.add_at(start + Duration::from_secs(2), 2000);
        assert_eq!(meter.rate_at(start + Duration::from_secs(2)), 2000.0);
        assert_eq!(meter.rate_at(start + Duration::from_secs(10)), 0.0);
    }

    #[test]
    fn progress_reads_as_an_install_task_and_controls_apply_once() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let job = InstallJob::new(Arc::new(move |p| sink.lock().unwrap().push(p)));
        assert_eq!(job.progress(), None);
        job.inner.update(&Progress {
            stage: Stage::Downloading,
            download_total: 100,
            downloaded: 40,
            chunks_total: 10,
            chunks_done: 4,
            files_total: 3,
            files_done: 1,
            install_total: 200,
        });
        let progress = job.progress().unwrap();
        assert_eq!(progress.kind, TaskKind::Install);
        assert_eq!((progress.done_bytes, progress.total_bytes), (40, 100));
        assert_eq!((progress.files_done, progress.files_total), (1, 3));
        assert!(progress.bytes_per_second > 0.0);
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(job.pause() && !job.pause());
        assert!(job.resume() && !job.resume());
        assert!(job.cancel() && !job.resume());
    }
}
