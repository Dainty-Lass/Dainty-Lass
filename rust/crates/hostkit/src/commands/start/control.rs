// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Shutdown signal and stdin control watcher for `reforged-host start` and `reforged-host serve`.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::Duration;

use crate::error::HostError;

/// Environment variable set to `"stdin"` when a parent process (such as the Electron launcher)
/// controls `reforged-host` over standard input.
pub const CONTROL_ENV: &str = "REFORGED_CONTROL";

/// Marker line printed to stdout once all required services are ready.
pub const SERVER_READY_LINE: &str = "REFORGED_SERVER_READY";

/// Returns `true` when `REFORGED_CONTROL=stdin` is set in the process environment.
pub fn controlled_by_stdin() -> bool {
    std::env::var(CONTROL_ENV).is_ok_and(|val| val.trim() == "stdin")
}

/// Reads lines from `input` until `"stop"` is received or EOF is reached, then invokes `on_stop`.
pub fn watch_stop_input(mut input: impl BufRead, on_stop: impl FnOnce()) {
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => {
                on_stop();
                return;
            }
            Ok(_) => {
                if line.trim().eq_ignore_ascii_case("stop") {
                    on_stop();
                    return;
                }
            }
            Err(_) => {
                on_stop();
                return;
            }
        }
    }
}

struct ControlInner {
    asked: Arc<AtomicBool>,
    lock: Mutex<bool>,
    cvar: Condvar,
}

impl ControlInner {
    fn new() -> Self {
        Self {
            asked: Arc::new(AtomicBool::new(false)),
            lock: Mutex::new(false),
            cvar: Condvar::new(),
        }
    }

    fn trigger(&self) {
        self.asked.store(true, Ordering::SeqCst);
        if let Ok(mut stopped) = self.lock.lock() {
            *stopped = true;
            self.cvar.notify_all();
        }
    }
}

fn subscribers() -> &'static Mutex<Vec<Weak<ControlInner>>> {
    static SUBSCRIBERS: OnceLock<Mutex<Vec<Weak<ControlInner>>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn ensure_ctrlc_registered() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let _ = std::thread::Builder::new().name("reforged-ctrlc".to_owned()).spawn(|| {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                return;
            };
            rt.block_on(async {
                while tokio::signal::ctrl_c().await.is_ok() {
                    if let Ok(mut list) = subscribers().lock() {
                        list.retain(|weak| {
                            if let Some(inner) = weak.upgrade() {
                                inner.trigger();
                                true
                            } else {
                                false
                            }
                        });
                    }
                }
            });
        });
    });
}

/// Coordinates graceful shutdown from Ctrl+C/SIGTERM or `REFORGED_CONTROL=stdin`.
#[derive(Clone)]
pub struct Control {
    inner: Arc<ControlInner>,
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Control {
    /// Creates an isolated `Control` handle without registering process-wide signal handlers.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ControlInner::new()),
        }
    }

    /// Registers signal handling (and an optional stdin `"stop"` / EOF watcher thread) and
    /// returns a `Control` handle.
    pub fn begin(stdin: bool) -> Result<Self, HostError> {
        ensure_ctrlc_registered();
        let ctrl = Self::new();
        if let Ok(mut list) = subscribers().lock() {
            list.push(Arc::downgrade(&ctrl.inner));
        }

        if stdin {
            let inner = Arc::clone(&ctrl.inner);
            std::thread::Builder::new()
                .name("reforged-stdin-control".to_owned())
                .spawn(move || {
                    let stdin_handle = std::io::stdin();
                    let reader = std::io::BufReader::new(stdin_handle);
                    watch_stop_input(reader, move || {
                        inner.trigger();
                    });
                })
                .map_err(|e| HostError::message(format!("failed to spawn stdin control thread: {e}")))?;
        }

        Ok(ctrl)
    }

    /// Returns the shared cancellation flag set when a stop signal or stdin stop/EOF is received.
    #[must_use]
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.inner.asked)
    }

    /// Returns `true` once a stop signal or stdin stop/EOF has been received.
    pub fn asked(&self) -> bool {
        self.inner.asked.load(Ordering::SeqCst)
    }

    /// Triggers shutdown and wakes any thread waiting on [`Self::wait`].
    pub fn stop(&self) {
        self.inner.trigger();
    }

    /// Blocks the calling thread until [`Self::stop`] is triggered.
    pub fn wait(&self) {
        if self.asked() {
            return;
        }
        if let Ok(mut guard) = self.inner.lock.lock() {
            while !*guard && !self.asked() {
                match self.inner.cvar.wait(guard) {
                    Ok(g) => guard = g,
                    Err(_) => break,
                }
            }
        }
    }

    /// Waits up to `timeout` for shutdown to be requested, returning `true` if triggered.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        if self.asked() {
            return true;
        }
        if let Ok(guard) = self.inner.lock.lock()
            && !*guard
            && !self.asked()
        {
            let _ = self.inner.cvar.wait_timeout(guard, timeout);
        }
        self.asked()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::io::Cursor;

    #[test]
    fn watch_stop_input_triggers_on_stop_line() {
        let input = Cursor::new(b"hello\nstop\nlater\n");
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        watch_stop_input(input, move || {
            flag.store(true, Ordering::SeqCst);
        });
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn watch_stop_input_triggers_on_eof() {
        let input = Cursor::new(b"keep going\n");
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        watch_stop_input(input, move || {
            flag.store(true, Ordering::SeqCst);
        });
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn control_stop_wakes_waiter() {
        let ctrl = Control::begin(false).unwrap();
        assert!(!ctrl.asked());
        let clone = ctrl.clone();
        let handle = std::thread::spawn(move || {
            clone.wait();
            clone.asked()
        });
        ctrl.stop();
        assert!(handle.join().unwrap());
    }
}
