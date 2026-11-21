// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The progress channel's throttle (RD27 spec §2.3.5). A download reports far more often than the page can
//! draw, so only the values worth drawing are sent.

use std::time::{Duration, Instant};

use super::types::{TaskKind, TaskProgress};

/// Least time between two values of the same task.
pub const MIN_INTERVAL: Duration = Duration::from_millis(100);

/// Decides which progress values reach the page.
#[derive(Debug, Clone, Default)]
pub struct Throttle {
    last: Option<(Instant, TaskKind)>,
}

impl Throttle {
    /// A throttle that has sent nothing.
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Whether `progress`, reported at `now`, is sent: the first value, a value [`MIN_INTERVAL`] after the last
    /// one sent, a value of another task, or a final one. A value that passes counts as sent.
    pub fn pass(&mut self, progress: &TaskProgress, now: Instant) -> bool {
        let send = match self.last {
            None => true,
            Some((at, kind)) => {
                kind != progress.kind || now.saturating_duration_since(at) >= MIN_INTERVAL || progress.is_final()
            }
        };
        if send {
            self.last = Some((now, progress.kind));
        }
        send
    }
}

/// The page's progress subscription: a sink and its throttle. Nothing is sent before [`Self::subscribe`].
#[derive(Debug)]
pub struct ProgressChannel<S> {
    sink: Option<S>,
    throttle: Throttle,
}

impl<S> Default for ProgressChannel<S> {
    fn default() -> Self {
        Self {
            sink: None,
            throttle: Throttle::new(),
        }
    }
}

impl<S> ProgressChannel<S> {
    /// No subscription yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the page's sink, replacing an earlier one. A reloaded page gets the next value at once.
    pub fn subscribe(&mut self, sink: S) {
        self.sink = Some(sink);
        self.throttle = Throttle::new();
    }

    /// Whether a page has subscribed.
    pub const fn is_subscribed(&self) -> bool {
        self.sink.is_some()
    }

    /// The sink, when `progress` at `now` should be sent to it.
    pub fn offer(&mut self, progress: &TaskProgress, now: Instant) -> Option<&S> {
        let sink = self.sink.as_ref()?;
        self.throttle.pass(progress, now).then_some(sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(kind: TaskKind, done: u64) -> TaskProgress {
        TaskProgress {
            kind,
            total_bytes: 100,
            done_bytes: done,
            files_total: 2,
            files_done: u32::from(done >= 100) * 2,
            bytes_per_second: 0,
            eta_seconds: None,
            current_file: None,
        }
    }

    #[test]
    fn sends_first_spaced_switched_and_final_values() {
        let start = Instant::now();
        let mut throttle = Throttle::new();
        assert!(throttle.pass(&progress(TaskKind::Download, 1), start));
        assert!(!throttle.pass(&progress(TaskKind::Download, 2), start + Duration::from_millis(99)));
        assert!(throttle.pass(&progress(TaskKind::Download, 3), start + Duration::from_millis(100)));
        assert!(throttle.pass(&progress(TaskKind::Verify, 3), start + Duration::from_millis(101)));
        assert!(!throttle.pass(&progress(TaskKind::Verify, 4), start + Duration::from_millis(102)));
        assert!(throttle.pass(&progress(TaskKind::Verify, 100), start + Duration::from_millis(103)));
    }

    #[test]
    fn half_final_is_not_final() {
        let mut p = progress(TaskKind::Install, 100);
        p.files_done = 1;
        assert!(!p.is_final());
        assert!(progress(TaskKind::Install, 100).is_final());
    }

    #[test]
    fn nothing_before_a_subscription() {
        let now = Instant::now();
        let mut channel = ProgressChannel::<u8>::new();
        assert!(channel.offer(&progress(TaskKind::Runtime, 1), now).is_none());
        channel.subscribe(1);
        assert_eq!(channel.offer(&progress(TaskKind::Runtime, 1), now), Some(&1));
        assert!(channel.offer(&progress(TaskKind::Runtime, 2), now).is_none());
        channel.subscribe(2);
        assert_eq!(channel.offer(&progress(TaskKind::Runtime, 3), now), Some(&2));
    }
}
