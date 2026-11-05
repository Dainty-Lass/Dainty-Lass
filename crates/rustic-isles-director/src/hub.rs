// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One persistent hub's supervisor (spec §7.1): single-flight starts, backoff on failure, a retry timer, and a
//! stable uptime that resets the count. Times are a monotonic clock, so a wall-clock step back cannot push a down
//! hub's attempt into the future.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Instant;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};

pub const DEFAULT_BACKOFF_MS: [u64; 6] = [5_000, 15_000, 30_000, 60_000, 120_000, 300_000];
pub const DEFAULT_STABLE_UPTIME_MS: u64 = 120_000;

/// Milliseconds since the first call in this process.
pub fn monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// A hub switch: unset or blank takes the default; `0`, `false`, `no`, `off` (trimmed, any case) are off;
/// anything else is on.
pub fn switch_on(value: Option<&str>, default: bool) -> bool {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return default;
    };
    !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// A cancellable timer from [`Options::set_timer`].
pub trait Timer: Send {
    fn cancel(&self);
}

struct TaskTimer(tokio::task::JoinHandle<()>);

impl Timer for TaskTimer {
    fn cancel(&self) {
        self.0.abort();
    }
}

struct NoTimer;

impl Timer for NoTimer {
    fn cancel(&self) {}
}

pub type StartFn<T> = Arc<dyn Fn(String) -> BoxFuture<'static, Result<T, String>> + Send + Sync>;
pub type SetTimer = Arc<dyn Fn(u64, Box<dyn FnOnce() + Send>) -> Box<dyn Timer> + Send + Sync>;
pub type LogFn = Arc<dyn Fn(Level, String, Option<String>) + Send + Sync>;

/// A tokio sleep; none outside a runtime.
pub fn tokio_timer() -> SetTimer {
    Arc::new(|delay_ms, fire| match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            let task = handle.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                fire();
            });
            Box::new(TaskTimer(task)) as Box<dyn Timer>
        }
        Err(_) => Box::new(NoTimer),
    })
}

/// `tracing` at the level, the error as a structured field.
pub fn tracing_log() -> LogFn {
    Arc::new(|level, message, error| match (level, error) {
        (Level::Info, None) => tracing::info!("{message}"),
        (Level::Info, Some(error)) => tracing::info!(error, "{message}"),
        (Level::Warn, None) => tracing::warn!("{message}"),
        (Level::Warn, Some(error)) => tracing::warn!(error, "{message}"),
        (Level::Error, None) => tracing::error!("{message}"),
        (Level::Error, Some(error)) => tracing::error!(error, "{message}"),
    })
}

pub struct Options<T> {
    pub label: String,
    pub start: StartFn<T>,
    pub is_alive: Arc<dyn Fn(&T) -> bool + Send + Sync>,
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
    pub set_timer: SetTimer,
    pub log: LogFn,
    pub backoff_ms: Vec<u64>,
    pub stable_uptime_ms: u64,
}

impl<T> Options<T> {
    /// The defaults: the monotonic clock, tokio timers, `tracing`, §7.1's backoff and uptime.
    pub fn new(label: &str, start: StartFn<T>, is_alive: Arc<dyn Fn(&T) -> bool + Send + Sync>) -> Self {
        Self {
            label: label.to_owned(),
            start,
            is_alive,
            now: Arc::new(monotonic_ms),
            set_timer: tokio_timer(),
            log: tracing_log(),
            backoff_ms: DEFAULT_BACKOFF_MS.to_vec(),
            stable_uptime_ms: DEFAULT_STABLE_UPTIME_MS,
        }
    }
}

type Pending<T> = Shared<BoxFuture<'static, Option<T>>>;

struct State<T> {
    current: Option<T>,
    current_since: u64,
    pending: Option<(u64, Pending<T>)>,
    generation: u64,
    failures: u32,
    next_due: u64,
    timer: Option<Box<dyn Timer>>,
    disposed: bool,
}

/// What [`HubSupervisor::ensure`] does before it awaits anything.
enum Begin<T> {
    Done(Option<T>),
    Wait(Pending<T>),
}

pub struct HubSupervisor<T: Clone + PartialEq + Send + Sync + 'static> {
    options: Options<T>,
    state: Mutex<State<T>>,
    me: Weak<Self>,
}

fn seconds(ms: u64) -> String {
    service_kit::jsjson::number(ms as f64 / 1000.0)
}

impl<T: Clone + PartialEq + Send + Sync + 'static> HubSupervisor<T> {
    pub fn new(options: Options<T>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            options,
            state: Mutex::new(State {
                current: None,
                current_since: 0,
                pending: None,
                generation: 0,
                failures: 0,
                next_due: 0,
                timer: None,
                disposed: false,
            }),
            me: me.clone(),
        })
    }

    pub fn label(&self) -> &str {
        &self.options.label
    }

    fn state(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> u64 {
        (self.options.now)()
    }

    fn alive(&self, server: &T) -> bool {
        (self.options.is_alive)(server)
    }

    /// A log call that panics never breaks supervision.
    fn log(&self, level: Level, message: String, error: Option<String>) {
        let log = &self.options.log;
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| log(level, message, error)));
    }

    fn backoff(&self, failures: u32) -> u64 {
        let list = &self.options.backoff_ms;
        let at = (failures as usize).min(list.len()).saturating_sub(1);
        list.get(at).copied().unwrap_or(0)
    }

    /// The current hub if it is alive, else a shared start, else none (before the due time, or a failed start).
    pub async fn ensure(&self, reason: &str) -> Option<T> {
        match self.begin(reason) {
            Begin::Done(answer) => answer,
            Begin::Wait(pending) => pending.await,
        }
    }

    /// Starts what `ensure` would without waiting for it.
    pub fn ensure_in_background(&self, reason: &str) {
        let _ = self.begin(reason);
    }

    fn begin(&self, reason: &str) -> Begin<T> {
        let mut state = self.state();
        if let Some(current) = state.current.clone() {
            if self.alive(&current) {
                return Begin::Done(Some(current));
            }
            self.mark_gone_locked(&mut state, &current, &format!("{reason}: process gone"));
        }
        if let Some((_, pending)) = &state.pending {
            return Begin::Wait(pending.clone());
        }
        if state.disposed {
            return Begin::Done(None);
        }
        if self.now() < state.next_due {
            self.arm_locked(&mut state);
            return Begin::Done(None);
        }
        if let Some(timer) = state.timer.take() {
            timer.cancel();
        }
        let Some(me) = self.me.upgrade() else {
            return Begin::Done(None);
        };
        state.generation += 1;
        let generation = state.generation;
        let reason = reason.to_owned();
        let attempt = state.failures + 1;
        let task = async move { me.run_start(generation, reason, attempt).await };
        // Its own task, so it finishes whether or not anyone awaits it; the lock is held until `pending` is set
        let pending = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let join = handle.spawn(task);
                async move { join.await.ok().flatten() }.boxed().shared()
            }
            Err(_) => task.boxed().shared(),
        };
        state.pending = Some((generation, pending.clone()));
        Begin::Wait(pending)
    }

    async fn run_start(self: Arc<Self>, generation: u64, reason: String, attempt: u32) -> Option<T> {
        let label = &self.options.label;
        self.log(
            Level::Info,
            format!("[HubSupervisor] {label} starting ({reason}, attempt {attempt})"),
            None,
        );
        let result = (self.options.start)(reason.clone()).await;
        let mut state = self.state();
        if state.pending.as_ref().is_some_and(|(g, _)| *g == generation) {
            state.pending = None;
        }
        match result {
            Err(error) => {
                self.failure_locked(&mut state, "start failed", Some(error));
                None
            }
            Ok(server) if !self.alive(&server) => {
                self.failure_locked(&mut state, "exited right after reporting ready", None);
                None
            }
            Ok(server) => {
                state.current = Some(server.clone());
                state.current_since = self.now();
                state.next_due = 0;
                let failures = state.failures;
                drop(state);
                self.log(
                    Level::Info,
                    format!("[HubSupervisor] {label} up ({reason}; failures so far {failures})"),
                    None,
                );
                Some(server)
            }
        }
    }

    fn failure_locked(&self, state: &mut State<T>, what: &str, error: Option<String>) {
        state.failures += 1;
        let delay = self.backoff(state.failures);
        state.next_due = self.now().saturating_add(delay);
        self.arm_locked(state);
        self.log(
            Level::Error,
            format!(
                "[HubSupervisor] {} {what} (failure {}); retrying in {}s",
                self.options.label,
                state.failures,
                seconds(delay)
            ),
            error,
        );
    }

    /// Only the current server; a late exit of another is ignored. True when it acted.
    fn mark_gone_locked(&self, state: &mut State<T>, server: &T, reason: &str) -> bool {
        if state.current.as_ref() != Some(server) {
            return false;
        }
        state.current = None;
        if state.pending.is_some() {
            return true;
        }
        let now = self.now();
        let up = now.saturating_sub(state.current_since);
        let up_s = service_kit::jsjson::number((up as f64 / 1000.0).round());
        let label = &self.options.label;
        if up >= self.options.stable_uptime_ms {
            state.failures = 0;
            state.next_due = now;
            self.log(
                Level::Warn,
                format!("[HubSupervisor] {label} exited after {up_s}s up ({reason}) - restarting now"),
                None,
            );
        } else {
            state.failures += 1;
            let delay = self.backoff(state.failures);
            state.next_due = now.saturating_add(delay);
            self.log(
                Level::Warn,
                format!(
                    "[HubSupervisor] {label} exited {up_s}s after ready ({reason}; failure {}) - restart in {}s",
                    state.failures,
                    seconds(delay)
                ),
                None,
            );
        }
        true
    }

    fn arm_locked(&self, state: &mut State<T>) {
        if state.disposed || state.timer.is_some() || state.pending.is_some() || state.current.is_some() {
            return;
        }
        let delay = state.next_due.saturating_sub(self.now());
        let me = self.me.clone();
        let fire = Box::new(move || {
            if let Some(me) = me.upgrade() {
                me.state().timer = None;
                me.ensure_in_background("retry");
            }
        });
        state.timer = Some((self.options.set_timer)(delay, fire));
    }

    /// A stable exit restarts at once, an early one waits for its timer.
    pub async fn on_exit(&self, server: &T, reason: &str) -> Option<T> {
        let gone = {
            let mut state = self.state();
            self.mark_gone_locked(&mut state, server, reason)
        };
        if gone {
            self.ensure(&format!("exit: {reason}")).await
        } else {
            None
        }
    }

    /// Forgets the current server with no failure, backoff or timer; whether it acted.
    pub fn release(&self, server: &T, reason: &str) -> bool {
        let mut state = self.state();
        if state.current.as_ref() != Some(server) {
            return false;
        }
        state.current = None;
        state.failures = 0;
        state.next_due = 0;
        if let Some(timer) = state.timer.take() {
            timer.cancel();
        }
        drop(state);
        self.log(
            Level::Info,
            format!(
                "[HubSupervisor] {} released ({reason}) - started again on the next request",
                self.options.label
            ),
            None,
        );
        true
    }

    /// The watchdog's pass.
    pub async fn tick(&self) {
        let current = self.state().current.clone();
        match current {
            Some(server) if !self.alive(&server) => {
                let gone = {
                    let mut state = self.state();
                    self.mark_gone_locked(&mut state, &server, "watchdog: process gone")
                };
                if gone {
                    self.ensure_in_background("exit: watchdog: process gone");
                }
            }
            Some(_) => {
                let mut state = self.state();
                let up = self.now().saturating_sub(state.current_since);
                if state.failures > 0 && up >= self.options.stable_uptime_ms {
                    state.failures = 0;
                    drop(state);
                    self.log(
                        Level::Info,
                        format!(
                            "[HubSupervisor] {} stable for {}s - failure count reset",
                            self.options.label,
                            service_kit::jsjson::number((up as f64 / 1000.0).round())
                        ),
                        None,
                    );
                }
            }
            None => {
                if self.state().pending.is_none() {
                    self.ensure_in_background("watchdog");
                }
            }
        }
    }

    pub fn dispose(&self) {
        let mut state = self.state();
        state.disposed = true;
        if let Some(timer) = state.timer.take() {
            timer.cancel();
        }
    }

    pub fn current(&self) -> Option<T> {
        self.state().current.clone()
    }

    pub fn pending(&self) -> bool {
        self.state().pending.is_some()
    }

    pub fn failure_count(&self) -> u32 {
        self.state().failures
    }

    pub fn next_attempt_due_at(&self) -> u64 {
        self.state().next_due
    }

    pub fn retry_armed(&self) -> bool {
        self.state().timer.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    use super::*;

    #[derive(Clone, Debug)]
    struct Hub(u64, Arc<AtomicBool>);

    impl PartialEq for Hub {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }

    type FakeTimerEntry = (u64, Option<Box<dyn FnOnce() + Send>>, Arc<AtomicBool>);

    #[derive(Default)]
    struct Rig {
        clock: AtomicU64,
        starts: AtomicUsize,
        reasons: Mutex<Vec<String>>,
        fail: AtomicBool,
        dead_on_arrival: AtomicBool,
        timers: Mutex<Vec<FakeTimerEntry>>,
    }

    struct FakeTimer(Arc<AtomicBool>);

    impl Timer for FakeTimer {
        fn cancel(&self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl Rig {
        fn supervisor(self: &Arc<Self>) -> Arc<HubSupervisor<Hub>> {
            let rig = Arc::clone(self);
            let start: StartFn<Hub> = Arc::new(move |reason| {
                let rig = Arc::clone(&rig);
                Box::pin(async move {
                    tokio::task::yield_now().await;
                    let n = rig.starts.fetch_add(1, Ordering::SeqCst) as u64;
                    rig.reasons.lock().unwrap().push(reason);
                    if rig.fail.load(Ordering::SeqCst) {
                        return Err("boom".to_owned());
                    }
                    let alive = !rig.dead_on_arrival.load(Ordering::SeqCst);
                    Ok(Hub(n, Arc::new(AtomicBool::new(alive))))
                })
            });
            let mut options = Options::new("ramsgate", start, Arc::new(|hub: &Hub| hub.1.load(Ordering::SeqCst)));
            let clock = Arc::clone(self);
            options.now = Arc::new(move || clock.clock.load(Ordering::SeqCst));
            let timers = Arc::clone(self);
            options.set_timer = Arc::new(move |delay, fire| {
                let cancelled = Arc::new(AtomicBool::new(false));
                let due = timers.clock.load(Ordering::SeqCst) + delay;
                timers.timers.lock().unwrap().push((due, Some(fire), cancelled.clone()));
                Box::new(FakeTimer(cancelled))
            });
            options.log = Arc::new(|_, _, _| {});
            HubSupervisor::new(options)
        }

        fn advance(&self, ms: u64) {
            let now = self.clock.fetch_add(ms, Ordering::SeqCst) + ms;
            let due: Vec<Box<dyn FnOnce() + Send>> = self
                .timers
                .lock()
                .unwrap()
                .iter_mut()
                .filter(|(at, fire, cancelled)| *at <= now && fire.is_some() && !cancelled.load(Ordering::SeqCst))
                .filter_map(|(_, fire, _)| fire.take())
                .collect();
            for fire in due {
                fire();
            }
        }

        fn starts(&self) -> usize {
            self.starts.load(Ordering::SeqCst)
        }
    }

    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_failed_boot_is_retried_by_its_timer() {
        let rig = Arc::new(Rig::default());
        rig.fail.store(true, Ordering::SeqCst);
        let hub = rig.supervisor();
        assert!(hub.ensure("boot").await.is_none());
        assert_eq!((hub.failure_count(), hub.next_attempt_due_at()), (1, 5000));
        assert!(hub.retry_armed());
        rig.fail.store(false, Ordering::SeqCst);
        rig.advance(5000);
        settle().await;
        assert!(hub.current().is_some());
        assert_eq!(rig.reasons.lock().unwrap().as_slice(), ["boot", "retry"]);
    }

    #[tokio::test]
    async fn backoff_delays() {
        let rig = Arc::new(Rig::default());
        rig.fail.store(true, Ordering::SeqCst);
        let hub = rig.supervisor();
        let mut delays = Vec::new();
        for _ in 0..8 {
            let now = rig.clock.load(Ordering::SeqCst);
            hub.ensure("boot").await;
            delays.push(hub.next_attempt_due_at() - now);
            rig.clock.store(hub.next_attempt_due_at(), Ordering::SeqCst);
        }
        assert_eq!(delays, [5000, 15000, 30000, 60000, 120000, 300000, 300000, 300000]);
    }

    #[tokio::test]
    async fn concurrent_ensures_share_one_start() {
        let rig = Arc::new(Rig::default());
        let hub = rig.supervisor();
        let (a, b) = tokio::join!(hub.ensure("boot"), hub.ensure("request"));
        assert_eq!(a, b);
        assert_eq!(rig.starts(), 1);
    }

    #[tokio::test]
    async fn exits_stable_and_early() {
        let rig = Arc::new(Rig::default());
        let hub = rig.supervisor();
        let first = hub.ensure("boot").await.unwrap();
        rig.advance(DEFAULT_STABLE_UPTIME_MS);
        first.1.store(false, Ordering::SeqCst);
        let second = hub.on_exit(&first, "exit code=0").await.unwrap();
        assert_eq!((hub.failure_count(), hub.retry_armed()), (0, false));
        assert_eq!(rig.reasons.lock().unwrap()[1], "exit: exit code=0");
        // A crash loop waits, and a request reuses the armed timer
        second.1.store(false, Ordering::SeqCst);
        assert!(hub.on_exit(&second, "exit code=1").await.is_none());
        assert_eq!(hub.failure_count(), 1);
        assert!(hub.retry_armed());
        assert!(hub.ensure("request").await.is_none());
        assert_eq!(rig.timers.lock().unwrap().len(), 1);
        // A non-current exit is ignored
        assert!(hub.on_exit(&first, "late").await.is_none());
        assert_eq!(hub.failure_count(), 1);
        rig.advance(5000);
        settle().await;
        assert!(hub.current().is_some());
        assert_eq!(rig.starts(), 3);
    }

    #[tokio::test]
    async fn a_failed_restart_after_a_stable_crash_is_retried() {
        let rig = Arc::new(Rig::default());
        let hub = rig.supervisor();
        let first = hub.ensure("boot").await.unwrap();
        rig.advance(DEFAULT_STABLE_UPTIME_MS);
        first.1.store(false, Ordering::SeqCst);
        rig.fail.store(true, Ordering::SeqCst);
        assert!(hub.on_exit(&first, "exit code=0").await.is_none());
        assert!(hub.retry_armed());
        rig.fail.store(false, Ordering::SeqCst);
        rig.advance(5000);
        settle().await;
        assert!(hub.current().is_some());
    }

    #[tokio::test]
    async fn watchdog_and_exit_in_one_pass_restart_once() {
        let rig = Arc::new(Rig::default());
        let hub = rig.supervisor();
        let first = hub.ensure("boot").await.unwrap();
        rig.advance(DEFAULT_STABLE_UPTIME_MS);
        first.1.store(false, Ordering::SeqCst);
        let (exit, ()) = tokio::join!(hub.on_exit(&first, "exit code=0"), hub.tick());
        settle().await;
        assert!(exit.is_some() || hub.current().is_some());
        assert_eq!(rig.starts(), 2);
        hub.tick().await;
        hub.tick().await;
        settle().await;
        assert_eq!(rig.starts(), 2);
    }

    #[tokio::test]
    async fn tick_retries_when_due_and_never_doubles() {
        let rig = Arc::new(Rig::default());
        rig.fail.store(true, Ordering::SeqCst);
        let hub = rig.supervisor();
        hub.ensure("boot").await;
        hub.tick().await;
        settle().await;
        assert_eq!(rig.starts(), 1);
        rig.fail.store(false, Ordering::SeqCst);
        rig.clock.store(5000, Ordering::SeqCst);
        hub.tick().await;
        hub.tick().await;
        settle().await;
        assert_eq!(rig.starts(), 2);
        assert!(hub.current().is_some());
    }

    #[tokio::test]
    async fn an_exit_during_a_start_and_a_dead_start() {
        let rig = Arc::new(Rig::default());
        let hub = rig.supervisor();
        let first = hub.ensure("boot").await.unwrap();
        first.1.store(false, Ordering::SeqCst);
        // ensure marks it gone while no start is in flight: a failure; now one in flight and an old exit
        rig.dead_on_arrival.store(true, Ordering::SeqCst);
        rig.advance(DEFAULT_STABLE_UPTIME_MS);
        assert!(hub.ensure("request").await.is_none());
        assert_eq!(hub.failure_count(), 1);
        assert!(hub.on_exit(&first, "exit").await.is_none());
        assert_eq!(hub.failure_count(), 1);
    }

    #[tokio::test]
    async fn released_hubs() {
        let rig = Arc::new(Rig::default());
        let hub = rig.supervisor();
        let first = hub.ensure("boot").await.unwrap();
        assert!(hub.release(&first, "idle"));
        assert!(!hub.release(&first, "idle"));
        assert_eq!(
            (hub.failure_count(), hub.retry_armed(), hub.current()),
            (0, false, None)
        );
        hub.ensure("request").await.unwrap();
        assert_eq!(rig.starts(), 2);
    }

    #[tokio::test]
    async fn a_shared_gate_never_starts_two_at_once() {
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        let busy = Arc::new(AtomicBool::new(false));
        let overlap = Arc::new(AtomicBool::new(false));
        let make = |label: &str| {
            let (gate, busy, overlap) = (gate.clone(), busy.clone(), overlap.clone());
            let start: StartFn<u32> = Arc::new(move |_| {
                let (gate, busy, overlap) = (gate.clone(), busy.clone(), overlap.clone());
                Box::pin(async move {
                    let _held = gate.lock().await;
                    if busy.swap(true, Ordering::SeqCst) {
                        overlap.store(true, Ordering::SeqCst);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    busy.store(false, Ordering::SeqCst);
                    Ok(1)
                })
            });
            let mut options = Options::new(label, start, Arc::new(|_: &u32| true));
            options.log = Arc::new(|_, _, _| {});
            HubSupervisor::new(options)
        };
        let (a, b) = (make("ramsgate"), make("training_dojo"));
        let _ = tokio::join!(a.ensure("boot"), b.ensure("boot"));
        assert!(!overlap.load(Ordering::SeqCst));
    }

    #[test]
    fn switches_and_clock() {
        assert!(switch_on(None, true));
        assert!(!switch_on(Some(""), false));
        assert!(!switch_on(Some("  "), false));
        for off in ["0", "false", "no", "off", " OFF ", "False"] {
            assert!(!switch_on(Some(off), true), "{off}");
        }
        for on in ["1", "on", "yes", "true", "anything"] {
            assert!(switch_on(Some(on), false), "{on}");
        }
        let a = monotonic_ms();
        assert!(monotonic_ms() >= a);
    }
}
