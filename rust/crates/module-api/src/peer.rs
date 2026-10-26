//! What both ends of the pipe share: one writer for every task, and the calls waiting for a reply.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tokio::io::AsyncWrite;
use tokio::sync::oneshot;

use crate::frame::write_frame;
use crate::module::{ErrorKind, ModuleError};

/// How many calls from the other side run at once; past it, reading waits, which backs the pipe up.
pub(crate) const MAX_IN_FLIGHT: usize = 32;

/// The most bytes of one log message or stderr line passed on from a child.
pub(crate) const MAX_LOG_LINE: usize = 4096;

/// None once closed: a child's stdin closes only when its handle is dropped, not on shutdown.
pub(crate) type Writer = Arc<tokio::sync::Mutex<Option<Pin<Box<dyn AsyncWrite + Send>>>>>;

pub(crate) fn writer(inner: impl AsyncWrite + Send + 'static) -> Writer {
    Arc::new(tokio::sync::Mutex::new(Some(Box::pin(inner))))
}

pub(crate) async fn send<T: Serialize>(writer: &Writer, message: &T, max: usize) -> Result<(), ModuleError> {
    let mut guard = writer.lock().await;
    let Some(inner) = guard.as_mut() else {
        return Err(ModuleError::new(ErrorKind::Unavailable, "the pipe is closed"));
    };
    write_frame(inner, message, max)
        .await
        .map_err(|error| ModuleError::new(ErrorKind::Unavailable, format!("the pipe failed: {error}")))
}

/// Closes the pipe, which is the child adapter's signal to exit.
pub(crate) async fn close(writer: &Writer) {
    if let Some(mut inner) = writer.lock().await.take() {
        _ = tokio::io::AsyncWriteExt::shutdown(&mut inner).await;
    }
}

/// What a reply to a call id means.
pub(crate) enum Resolved {
    Delivered,
    /// A call that timed out or was abandoned: dropped
    Late,
    /// An id never issued: the other side is broken
    Unknown,
}

struct State<A> {
    next: u64,
    waiting: HashMap<u64, oneshot::Sender<Result<A, ModuleError>>>,
    closed: Option<String>,
}

/// The calls this side made that wait for the other side's reply.
pub(crate) struct Pending<A> {
    state: Mutex<State<A>>,
}

impl<A> Pending<A> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                next: 0,
                waiting: HashMap::new(),
                closed: None,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<A>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn open(&self) -> Result<(u64, oneshot::Receiver<Result<A, ModuleError>>), ModuleError> {
        let mut state = self.lock();
        if let Some(reason) = &state.closed {
            return Err(ModuleError::new(ErrorKind::Unavailable, reason.clone()));
        }
        let id = state.next;
        state.next += 1;
        let (tx, rx) = oneshot::channel();
        state.waiting.insert(id, tx);
        Ok((id, rx))
    }

    pub(crate) fn forget(&self, id: u64) {
        self.lock().waiting.remove(&id);
    }

    pub(crate) fn resolve(&self, id: u64, result: Result<A, ModuleError>) -> Resolved {
        let mut state = self.lock();
        match state.waiting.remove(&id) {
            Some(tx) => {
                _ = tx.send(result);
                Resolved::Delivered
            }
            None if id < state.next => Resolved::Late,
            None => Resolved::Unknown,
        }
    }

    /// Fails every waiting call and every later one with `reason`.
    pub(crate) fn close(&self, reason: impl Into<String>) {
        let mut state = self.lock();
        let reason = state.closed.get_or_insert_with(|| reason.into()).clone();
        for (_, tx) in state.waiting.drain() {
            _ = tx.send(Err(ModuleError::new(ErrorKind::Unavailable, reason.clone())));
        }
    }

    pub(crate) fn closed(&self) -> Option<String> {
        self.lock().closed.clone()
    }
}

/// A child's text made safe for one log line: control characters replaced, length bounded.
pub(crate) fn log_safe(text: &str) -> String {
    let mut out: String = text
        .trim_end_matches(['\r', '\n'])
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect();
    if out.len() > MAX_LOG_LINE {
        let mut end = MAX_LOG_LINE;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replies_reach_their_call_and_closing_fails_the_rest() {
        let pending = Pending::<u8>::new();
        let (a, rx_a) = pending.open().unwrap();
        let (b, rx_b) = pending.open().unwrap();
        assert!(matches!(pending.resolve(a, Ok(1)), Resolved::Delivered));
        assert_eq!(rx_a.await.unwrap(), Ok(1));
        assert!(matches!(pending.resolve(a, Ok(1)), Resolved::Late));
        assert!(matches!(pending.resolve(99, Ok(1)), Resolved::Unknown));
        pending.close("gone");
        assert_eq!(rx_b.await.unwrap().unwrap_err().kind, ErrorKind::Unavailable);
        assert!(matches!(pending.resolve(b, Ok(1)), Resolved::Late));
        assert_eq!(pending.open().unwrap_err().message, "gone");
        assert_eq!(pending.closed().as_deref(), Some("gone"));
    }

    #[test]
    fn log_lines_lose_control_characters_and_length() {
        assert_eq!(log_safe("a\u{1b}[31mb\r\n"), "a\u{fffd}[31mb");
        assert_eq!(log_safe("x\ny"), "x\u{fffd}y");
        let long = "é".repeat(MAX_LOG_LINE);
        let safe = log_safe(&long);
        assert!(safe.len() <= MAX_LOG_LINE && safe.chars().all(|c| c == 'é'));
    }
}
