// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The firewall sync: brings the firewall to the desired list, never more often than once per interval, folding
//! changes that arrive during a run or within the interval into one later run, and backing off after failures.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dainty_module_api::BoxFuture;
use tokio::sync::Notify;
use tokio::time::Instant;

/// Brings the firewall to a list of addresses.
pub type Apply = Arc<dyn Fn(Vec<String>) -> BoxFuture<'static, Result<(), String>> + Send + Sync>;

/// Told every run's result, before the run is marked finished.
pub type OnResult = Arc<dyn Fn(&ApplyResult) + Send + Sync>;

const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// How one firewall run went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    /// Whether the firewall now matches.
    pub ok: bool,
    /// The addresses it applied.
    pub ips: Vec<String>,
    /// Milliseconds since the epoch, when the run started
    pub at: i64,
    /// How long it took, in milliseconds.
    pub ms: u64,
    /// Why it failed.
    pub error: Option<String>,
}

#[derive(Default)]
struct State {
    desired: Vec<String>,
    /// Unknown at start and after a failed run, so the next run always applies
    applied: Option<Vec<String>>,
    last_start: Option<Instant>,
    failures: u32,
    stopped: bool,
    running: bool,
    runs: u64,
    last: Option<ApplyResult>,
    epoch: u64,
    urgent: bool,
}

impl State {
    fn differs(&self) -> bool {
        self.applied.as_ref() != Some(&self.desired)
    }
}

struct Inner {
    apply: Apply,
    on_result: OnResult,
    interval: Duration,
    state: Mutex<State>,
    wake: Notify,
    run_lock: tokio::sync::Mutex<()>,
}

/// Brings the firewall to the desired address list in the background, at most once per interval, backing off on failure.
#[derive(Clone)]
pub struct FirewallSync(Arc<Inner>);

impl FirewallSync {
    /// Starts the scheduler on the current runtime.
    pub fn new(apply: Apply, on_result: OnResult, interval: Duration) -> Self {
        let sync = Self(Arc::new(Inner {
            apply,
            on_result,
            interval,
            state: Mutex::default(),
            wake: Notify::new(),
            run_lock: tokio::sync::Mutex::new(()),
        }));
        let scheduler = sync.clone();
        tokio::spawn(async move { scheduler.schedule().await });
        sync
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the desired list and wakes the scheduler; ignored once finished.
    pub fn request(&self, list: Vec<String>) {
        let mut state = self.lock();
        if state.stopped {
            return;
        }
        state.desired = list;
        drop(state);
        self.0.wake.notify_one();
    }

    /// The same list again at once: a module's ranges changed and its servers are about to start.
    pub fn reapply(&self) {
        let mut state = self.lock();
        if state.stopped {
            return;
        }
        state.epoch += 1;
        state.applied = None;
        state.urgent = true;
        drop(state);
        self.0.wake.notify_one();
    }

    /// Running, or applied differs from desired.
    pub fn pending(&self) -> bool {
        let state = self.lock();
        state.running || state.differs()
    }

    /// The last run's result.
    pub fn last_result(&self) -> Option<ApplyResult> {
        self.lock().last.clone()
    }

    /// Runs so far.
    pub fn runs(&self) -> u64 {
        self.lock().runs
    }

    /// Waits, polling every 10 ms, until nothing is pending or the sync stopped.
    pub async fn settled(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        loop {
            {
                let state = self.lock();
                if state.stopped || !(state.running || state.differs()) {
                    return true;
                }
            }
            if Instant::now() >= until {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn delay(&self, state: &State) -> Duration {
        let backoff = match state.failures {
            0 => Duration::ZERO,
            n => self.0.interval.saturating_mul(1u32 << (n - 1).min(16)).min(MAX_BACKOFF),
        };
        let spacing = match (state.urgent, state.last_start) {
            (false, Some(start)) => self.0.interval.saturating_sub(start.elapsed()),
            _ => Duration::ZERO,
        };
        backoff.max(spacing)
    }

    async fn schedule(&self) {
        loop {
            loop {
                let wait = self.0.wake.notified();
                {
                    let state = self.lock();
                    if state.stopped {
                        return;
                    }
                    if state.differs() {
                        break;
                    }
                }
                wait.await;
            }
            let mut deadline = {
                let state = self.lock();
                Instant::now() + self.delay(&state)
            };
            loop {
                let wait = self.0.wake.notified();
                tokio::select! {
                    () = tokio::time::sleep_until(deadline) => break,
                    () = wait => {
                        let state = self.lock();
                        // Only an urgent request recomputes the delay; others leave the timer alone
                        if state.urgent {
                            deadline = Instant::now() + self.delay(&state);
                        }
                    }
                }
            }
            let _run = self.0.run_lock.lock().await;
            if self.lock().stopped {
                return;
            }
            self.run_once().await;
        }
    }

    async fn run_once(&self) {
        let (list, epoch) = {
            let mut state = self.lock();
            state.running = true;
            state.urgent = false;
            state.last_start = Some(Instant::now());
            (state.desired.clone(), state.epoch)
        };
        let at = crate::now_ms();
        let started = Instant::now();
        let outcome = (self.0.apply)(list.clone()).await;
        let result = ApplyResult {
            ok: outcome.is_ok(),
            ips: list.clone(),
            at,
            ms: started.elapsed().as_millis() as u64,
            error: outcome.err(),
        };
        {
            let mut state = self.lock();
            if result.ok {
                state.applied = (state.epoch == epoch).then_some(list);
                state.failures = 0;
            } else {
                state.applied = None;
                state.failures += 1;
            }
            state.runs += 1;
            state.last = Some(result.clone());
        }
        (self.0.on_result)(&result);
        self.lock().running = false;
    }

    /// Shutdown: stops, waits for a run in progress, then applies `list` once unless it is already applied.
    pub async fn finish(&self, list: Vec<String>) -> Option<ApplyResult> {
        self.lock().stopped = true;
        self.0.wake.notify_one();
        let _run = self.0.run_lock.lock().await;
        {
            let mut state = self.lock();
            if state.applied.as_ref() == Some(&list) {
                return None;
            }
            state.desired = list;
        }
        self.run_once().await;
        self.last_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Log = Arc<Mutex<Vec<(Vec<String>, Instant)>>>;

    struct Fake {
        calls: Log,
        results: Arc<Mutex<Vec<bool>>>,
    }

    fn fake(fail_first: usize, run_time: Duration) -> (FirewallSync, Fake) {
        let calls: Log = Arc::default();
        let results: Arc<Mutex<Vec<bool>>> = Arc::default();
        let (c, r) = (calls.clone(), results.clone());
        let apply: Apply = Arc::new(move |list| {
            let c = c.clone();
            Box::pin(async move {
                let n = {
                    let mut calls = c.lock().unwrap();
                    calls.push((list, Instant::now()));
                    calls.len()
                };
                tokio::time::sleep(run_time).await;
                if n <= fail_first { Err("no".into()) } else { Ok(()) }
            })
        });
        let on_result: OnResult = Arc::new(move |result| r.lock().unwrap().push(result.ok));
        (
            FirewallSync::new(apply, on_result, Duration::from_millis(3000)),
            Fake { calls, results },
        )
    }

    fn ips(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn first_change_applies_at_once_and_later_ones_fold() {
        let (sync, fake) = fake(0, Duration::from_millis(100));
        let start = Instant::now();
        sync.request(ips(&["1.1.1.1"]));
        tokio::time::sleep(Duration::from_millis(50)).await;
        sync.request(ips(&["1.1.1.1", "2.2.2.2"]));
        tokio::time::sleep(Duration::from_millis(500)).await;
        sync.request(ips(&["1.1.1.1", "2.2.2.2", "3.3.3.3"]));
        assert!(sync.settled(Duration::from_secs(10)).await);
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(calls[0].1 - start, Duration::ZERO);
        assert_eq!(calls[1].0, ips(&["1.1.1.1", "2.2.2.2", "3.3.3.3"]));
        assert!(calls[1].1 - calls[0].1 >= Duration::from_millis(3000));
        // An equal list causes no run
        sync.request(ips(&["1.1.1.1", "2.2.2.2", "3.3.3.3"]));
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(fake.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_are_retried_with_backoff_and_reported() {
        let (sync, fake) = fake(1, Duration::ZERO);
        sync.request(ips(&["1.1.1.1"]));
        assert!(sync.settled(Duration::from_secs(30)).await);
        assert_eq!(*fake.results.lock().unwrap(), vec![false, true]);
        let calls = fake.calls.lock().unwrap().clone();
        assert!(calls[1].1 - calls[0].1 >= Duration::from_millis(3000));
    }

    #[tokio::test(start_paused = true)]
    async fn reapply_runs_the_same_list_at_once_and_finish_closes() {
        let (sync, fake) = fake(0, Duration::ZERO);
        sync.request(ips(&["1.1.1.1"]));
        assert!(sync.settled(Duration::from_secs(5)).await);
        tokio::time::sleep(Duration::from_millis(10)).await;
        let before = Instant::now();
        sync.reapply();
        assert!(sync.settled(Duration::from_secs(5)).await);
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].1 - before < Duration::from_millis(100));
        let closed = sync.finish(Vec::new()).await.unwrap();
        assert!(closed.ok && closed.ips.is_empty());
        sync.request(ips(&["9.9.9.9"]));
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(fake.calls.lock().unwrap().len(), 3);
        assert!(sync.finish(Vec::new()).await.is_none());
    }
}
