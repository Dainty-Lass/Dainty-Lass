// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One running game server process (spec §5.5-§5.7): both pipes always drained into its log file (the 2026-07-19
//! freeze: a paused stdout filled the pipe and the game thread blocked holding the CRT's stdio lock), the readiness
//! marker scanned from stdout, and the exit.

use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch};

use crate::launcher::{Command, Exit, Launcher, Pipe, TreeControl};

const CHUNK: usize = 64 * 1024;
const SCAN_LIMIT: usize = 65_536;
const SCAN_KEEP: usize = 4_096;
/// The pipes are read to their end after the exit; a stray holder is killed with the tree first
const PIPE_DRAIN: Duration = Duration::from_secs(5);
pub const READY_MARKER: &str = "RUSTIC_ISLES_GAMESERVER_READY";

/// `RUSTIC_ISLES_GAMESERVER_READY launchId={id} port={port}` (JS-trimmed): the id and port when well-formed.
pub fn parse_marker(line: &str) -> Option<(&str, &str)> {
    let rest = service_kit::js::trim(line).strip_prefix(READY_MARKER)?.strip_prefix(" launchId=")?;
    let (id, port) = rest.split_once(" port=")?;
    let id_ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-');
    let port_ok = !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
    (id_ok && port_ok).then_some((id, port))
}

/// Scans stdout line by line for this launch's marker.
#[derive(Debug)]
pub struct ReadinessScanner {
    launch_id: String,
    port: String,
    buffer: Vec<u8>,
}

impl ReadinessScanner {
    pub fn new(launch_id: &str, port: u16) -> Self {
        Self {
            launch_id: launch_id.to_owned(),
            port: port.to_string(),
            buffer: Vec::new(),
        }
    }

    /// True once this launch's marker was seen; another launch's is warned and ignored.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]);
            if let Some((id, port)) = parse_marker(&line) {
                if id == self.launch_id && port == self.port {
                    self.buffer.clear();
                    return true;
                }
                tracing::warn!(
                    "Ignoring mismatched gameserver readiness marker on port {}: {}",
                    self.port,
                    service_kit::js::trim(&line)
                );
            }
        }
        if self.buffer.len() > SCAN_LIMIT {
            let cut = self.buffer.len() - SCAN_KEEP;
            self.buffer.drain(..cut);
        }
        false
    }

    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }
}

/// A process and its exit, shared by the server, the registry and the exit watcher.
pub struct Proc {
    pub pid: Option<u32>,
    pub tree: Arc<dyn TreeControl>,
    exit: watch::Receiver<Option<Exit>>,
}

impl Proc {
    pub fn alive(&self) -> bool {
        self.exit.borrow().is_none()
    }

    pub fn exit(&self) -> Option<Exit> {
        self.exit.borrow().clone()
    }

    pub async fn exited(&self) -> Exit {
        let mut exit = self.exit.clone();
        match exit.wait_for(Option::is_some).await {
            Ok(exit) => exit.clone().unwrap_or_default(),
            Err(_) => Exit::default(),
        }
    }
}

pub(crate) struct Spawned {
    pub proc: Arc<Proc>,
    /// Ok once ready; closed when stdout ended without the marker
    pub ready: oneshot::Receiver<()>,
}

/// One writer per log file; a failed write stops writing but never blocks or fails the readers (DS5).
fn log_writer(file: std::fs::File) -> mpsc::UnboundedSender<Vec<u8>> {
    let (sender, mut receiver) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        let mut file = Some(tokio::fs::File::from_std(file));
        while let Some(chunk) = receiver.recv().await {
            if let Some(writer) = file.as_mut()
                && writer.write_all(&chunk).await.is_err()
            {
                file = None;
            }
        }
        if let Some(mut writer) = file {
            let _ = writer.flush().await;
        }
    });
    sender
}

fn pump(mut pipe: Pipe, log: mpsc::UnboundedSender<Vec<u8>>, mut scan: Option<(ReadinessScanner, oneshot::Sender<()>)>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = vec![0u8; CHUNK];
        loop {
            let n = match pipe.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let _ = log.send(buffer[..n].to_vec());
            if let Some((scanner, _)) = scan.as_mut()
                && scanner.feed(&buffer[..n])
                && let Some((_, ready)) = scan.take()
            {
                let _ = ready.send(());
            }
        }
    })
}

/// What the exit line and its warning name.
pub(crate) struct Labels {
    pub port: u16,
    pub label: String,
    pub launch_id: String,
}

/// Starts the process, writes `pid=` to its log and drains both pipes; on exit what is left of the tree is killed
/// at once (a stray Xvfb would hold the port) and the exit line closes the log.
pub(crate) fn launch(
    launcher: &dyn Launcher,
    command: &Command,
    mut file: std::fs::File,
    labels: Labels,
) -> std::io::Result<Spawned> {
    let started = launcher.start(command)?;
    let pid_text = started.pid.map_or_else(|| "unknown".to_owned(), |p| p.to_string());
    let _ = file.write_all(format!("pid={pid_text}\n\n").as_bytes());
    let log = log_writer(file);
    let (ready_tx, ready_rx) = oneshot::channel();
    let scanner = ReadinessScanner::new(&labels.launch_id, labels.port);
    let out = pump(started.stdout, log.clone(), Some((scanner, ready_tx)));
    let err = pump(started.stderr, log.clone(), None);
    let (exit_tx, exit_rx) = watch::channel(None);
    let tree = Arc::clone(&started.tree);
    let pid = started.pid;
    let wait = started.wait;
    tokio::spawn(async move {
        let exit = wait.await;
        let _ = exit_tx.send(Some(exit.clone()));
        tree.kill();
        let _ = tokio::time::timeout(PIPE_DRAIN, async {
            let _ = out.await;
            let _ = err.await;
        })
        .await;
        let pid = pid.map_or_else(|| "undefined".to_owned(), |p| p.to_string());
        let text = format!(
            "Gameserver process exited port={} label={} pid={pid} code={} signal={}",
            labels.port,
            labels.label,
            exit.code_text(),
            exit.signal_text()
        );
        tracing::warn!("{text}");
        let _ = log.send(format!("\n=== {text} ===\n").into_bytes());
    });
    Ok(Spawned {
        proc: Arc::new(Proc {
            pid,
            tree: started.tree,
            exit: exit_rx,
        }),
        ready: ready_rx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers() {
        let id = "0e1a7c2b-3f4d-4b5a-9c8d-112233445566";
        assert_eq!(
            parse_marker(&format!("  RUSTIC_ISLES_GAMESERVER_READY launchId={id} port=8780\r")),
            Some((id, "8780"))
        );
        assert_eq!(parse_marker("RUSTIC_ISLES_GAMESERVER_READY launchId=ABC port=1"), None);
        assert_eq!(parse_marker("RUSTIC_ISLES_GAMESERVER_READY launchId=abc port="), None);
        assert_eq!(parse_marker("MYSTICPARADOX_GAMESERVER_READY launchId=abc port=1"), None);
        let mut scanner = ReadinessScanner::new(id, 8780);
        assert!(!scanner.feed(b"LogNet: listening\nRUSTIC_ISLES_GAMESERVER_READY launchId=00000000-0000-4000-8000-000000000000 port=8780\n"));
        assert!(!scanner.feed(format!("RUSTIC_ISLES_GAMESERVER_READY launchId={id} port=8781\nRUSTIC_ISLES_GAMESERVER_READY launchId={id} ").as_bytes()));
        assert!(scanner.feed(b"port=8780\n"));
        let mut long = ReadinessScanner::new(id, 8780);
        long.feed(&vec![b'x'; 70_000]);
        assert_eq!(long.buffered(), 4096);
    }
}
