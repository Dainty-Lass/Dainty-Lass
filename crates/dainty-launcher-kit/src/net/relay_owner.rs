// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The relay's owner check (RD21), `dainty-relay`'s, over the launcher's table source: netstat.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use tokio::io::AsyncReadExt;

pub use dainty_relay::owner::*;

use crate::platform::system::{run_hidden, system_root};

/// How long one table read may take.
pub const TABLE_TIMEOUT: Duration = Duration::from_secs(2);
/// The most output one table read keeps.
pub const TABLE_LIMIT: u64 = 4 * 1024 * 1024;

/// `System32\netstat.exe -ano -p tcp`, run hidden.
#[derive(Debug, Clone, Copy, Default)]
pub struct Netstat;

impl TableSource for Netstat {
    fn read(&self) -> BoxFuture<'static, Option<String>> {
        Box::pin(async {
            let mut cmd = run_hidden(system_root().join("System32").join("netstat.exe"));
            cmd.args(["-ano", "-p", "tcp"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let run = async {
                let mut child = cmd.spawn().ok()?;
                let mut out = Vec::new();
                child
                    .stdout
                    .take()?
                    .take(TABLE_LIMIT + 1)
                    .read_to_end(&mut out)
                    .await
                    .ok()?;
                let status = child.wait().await.ok()?;
                (status.success() && out.len() as u64 <= TABLE_LIMIT).then_some(out)
            };
            let out = tokio::time::timeout(TABLE_TIMEOUT, run).await.ok()??;
            // The state column is in the console's code page; only ASCII fields are read
            Some(String::from_utf8_lossy(&out).into_owned())
        })
    }
}

/// The owner check for the game whose pid `pid()` answers: netstat on Windows, none elsewhere (RD21).
pub fn game_owner_check(pid: impl Fn() -> Option<u32> + Send + Sync + 'static) -> Option<Arc<dyn OwnerCheck>> {
    if cfg!(windows) {
        Some(Arc::new(GameOwnerCheck::new(pid, Netstat)))
    } else {
        drop(pid);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_has_a_check() {
        assert_eq!(game_owner_check(|| Some(1)).is_some(), cfg!(windows));
    }
}
