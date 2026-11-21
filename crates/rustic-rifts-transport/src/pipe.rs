// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Guard events from the hosted game server: `RGE1\t<type>\t<actor>\t<payload>` messages on an inbound named pipe.
//! The reader runs on the transport's runtime and only queues; the session takes the queue each loop.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::text::safe_str;

/// Umbrella spec D3: `RGE1` and the `RusticHuntGuard.events.` name the launcher gives the pipe.
pub const MESSAGE_MAGIC: &str = "RGE1";
/// The pipe's name prefix the launcher uses (umbrella spec D3).
pub const PIPE_PREFIX: &str = "RusticHuntGuard.events.";
/// The largest pipe message read, in bytes.
pub const MAX_MESSAGE: usize = 8192;
// Only the Windows pipe server reads messages; elsewhere the pipe stays empty.
#[cfg(windows)]
const MAX_QUEUED: usize = 256;

/// One Guard event read from the pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeEvent {
    /// The event type.
    pub kind: String,
    /// The actor account, empty when none is named.
    pub actor: String,
    /// The payload, a JSON object's text.
    pub payload: String,
}

/// Kept as observed: the pipe takes `.`, `:` and `-` in types that the batch check later refuses (spec §7).
pub fn parse_message(message: &[u8]) -> Option<PipeEvent> {
    let text = std::str::from_utf8(message).ok()?;
    let mut parts = text.splitn(4, '\t');
    let (magic, kind, actor, payload) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let valid = magic == MESSAGE_MAGIC
        && safe_str(kind, 64, b"_-.:")
        && (actor.is_empty() || safe_str(actor, 96, b"_-.:"))
        && (1..=4096).contains(&payload.len())
        && payload.starts_with('{')
        && payload.ends_with('}');
    valid.then(|| PipeEvent {
        kind: kind.to_owned(),
        actor: actor.to_owned(),
        payload: payload.to_owned(),
    })
}

#[derive(Debug, Default)]
struct Shared {
    events: VecDeque<PipeEvent>,
    dropped: u64,
    stopped: bool,
}

#[cfg(windows)]
impl Shared {
    fn push(&mut self, message: &[u8]) {
        match parse_message(message) {
            Some(event) if self.events.len() < MAX_QUEUED => self.events.push_back(event),
            _ => self.dropped += 1,
        }
    }
}

/// The inbound Guard event pipe; dropping it stops the reader.
pub struct GuardPipe {
    shared: Arc<Mutex<Shared>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

/// Checks the pipe name and that a game server process id is given.
pub fn check_config(name: &str, expected_pid: u32) -> Result<(), String> {
    if safe_str(name, 96, b"_-.:") && expected_pid != 0 {
        Ok(())
    } else {
        Err("invalid local Guard pipe configuration".to_owned())
    }
}

impl GuardPipe {
    /// Opens the named pipe and starts reading it on `runtime`.
    pub fn create(name: &str, expected_pid: u32, runtime: &tokio::runtime::Handle) -> Result<Self, String> {
        check_config(name, expected_pid)?;
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let task = platform::spawn(name, runtime, shared.clone())?;
        Ok(Self {
            shared,
            task: Some(task),
        })
    }

    /// The queued events and the count dropped since the last take.
    pub fn take(&self) -> (Vec<PipeEvent>, u64) {
        self.shared.lock().map_or_else(
            |_| (Vec::new(), 0),
            |mut shared| (shared.events.drain(..).collect(), std::mem::take(&mut shared.dropped)),
        )
    }
}

impl Drop for GuardPipe {
    fn drop(&mut self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.stopped = true;
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::io::ErrorKind;
    use std::sync::{Arc, Mutex};

    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

    use super::{MAX_MESSAGE, Shared};

    // ERROR_MORE_DATA: a message larger than the read buffer.
    const MORE_DATA: i32 = 234;

    /// Inbound only, message mode, one instance, remote clients refused. Without `unsafe` the pipe keeps Windows'
    /// default security descriptor and the client's process id is not checked (recorded in the provenance file).
    pub(super) fn spawn(
        name: &str,
        runtime: &tokio::runtime::Handle,
        shared: Arc<Mutex<Shared>>,
    ) -> Result<tokio::task::JoinHandle<()>, String> {
        let path = format!(r"\\.\pipe\{name}");
        let _entered = runtime.enter();
        let server = ServerOptions::new()
            .access_inbound(true)
            .access_outbound(false)
            .first_pipe_instance(true)
            .pipe_mode(PipeMode::Message)
            .reject_remote_clients(true)
            .max_instances(1)
            .in_buffer_size(MAX_MESSAGE as u32)
            .out_buffer_size(0)
            .create(&path)
            .map_err(|_| "local Guard pipe creation failed".to_owned())?;
        Ok(runtime.spawn(async move {
            let mut buffer = vec![0_u8; MAX_MESSAGE + 1];
            loop {
                if server.connect().await.is_err() {
                    let _ = server.disconnect();
                    continue;
                }
                loop {
                    if server.readable().await.is_err() {
                        break;
                    }
                    match server.try_read(&mut buffer) {
                        Ok(size) if size > 0 && size <= MAX_MESSAGE => {
                            if let Ok(mut shared) = shared.lock() {
                                shared.push(&buffer[..size]);
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) if e.raw_os_error() == Some(MORE_DATA) => {
                            if let Ok(mut shared) = shared.lock() {
                                shared.dropped += 1;
                            }
                            break;
                        }
                        _ => {
                            if let Ok(mut shared) = shared.lock() {
                                shared.dropped += 1;
                            }
                            break;
                        }
                    }
                }
                let _ = server.disconnect();
                if shared.lock().map_or(true, |s| s.stopped) {
                    return;
                }
            }
        }))
    }
}

/// A stand-in that never receives: Guard's producer runs only on Windows.
#[cfg(not(windows))]
mod platform {
    use std::sync::{Arc, Mutex};

    use super::Shared;

    pub(super) fn spawn(
        _name: &str,
        runtime: &tokio::runtime::Handle,
        _shared: Arc<Mutex<Shared>>,
    ) -> Result<tokio::task::JoinHandle<()>, String> {
        Ok(runtime.spawn(std::future::pending::<()>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        assert_eq!(
            parse_message(b"RGE1\tCOMPLETION_REQUESTED\tacc-1\t{\"a\":1}"),
            Some(PipeEvent {
                kind: "COMPLETION_REQUESTED".to_owned(),
                actor: "acc-1".to_owned(),
                payload: "{\"a\":1}".to_owned(),
            })
        );
        assert!(parse_message(b"RGE1\tA.b:c\t\t{\"x\":\"a\tb\"}").is_some());
        assert!(parse_message(b"XGE1\tA\t\t{}").is_none());
        assert!(parse_message(b"RGE1\tA\t\t[]").is_none());
        assert!(parse_message(b"RGE1\tA B\t\t{}").is_none());
        assert!(parse_message(b"RGE1\tA\t\t").is_none());
        assert_eq!(
            check_config("RusticHuntGuard.events.x", 0),
            Err("invalid local Guard pipe configuration".to_owned())
        );
    }
}
