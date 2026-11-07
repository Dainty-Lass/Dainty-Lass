// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The relay's owner check (RD21): on Windows a loopback connection is served only when the TCP table shows the
//! game the launcher started owns its client end. Any local process can reach 127.0.0.1, and the relay carries
//! the account key, so a loopback peer alone is not enough. Elsewhere there is no check.

pub mod table;

use std::net::{SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use tokio::io::AsyncReadExt;

use self::table::Owner;
use crate::platform::system::{run_hidden, system_root};

/// More checks than this waiting at once are refused, so a flood of connections can't queue table reads.
pub const MAX_WAITING: usize = 64;
/// The wait before a second read when the first had no row: the table can lag the connect.
pub const RETRY_DELAY: Duration = Duration::from_millis(25);
/// How long one table read may take.
pub const TABLE_TIMEOUT: Duration = Duration::from_secs(2);
/// The most output one table read keeps.
pub const TABLE_LIMIT: u64 = 4 * 1024 * 1024;

/// Decides once per connection whether the relay serves it.
pub trait OwnerCheck: Send + Sync {
    /// Whether the connection from `peer` to the relay's `local` address belongs to the game.
    fn allows(&self, peer: SocketAddr, local: SocketAddr) -> BoxFuture<'_, bool>;
}

/// Reads the TCP table as text; `None` when it could not be read.
pub trait TableSource: Send + Sync + 'static {
    /// One fresh read.
    fn read(&self) -> BoxFuture<'static, Option<String>>;
}

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

type SharedRead = Shared<BoxFuture<'static, Option<Arc<str>>>>;

/// The check against the game's pid, over any [`TableSource`].
pub struct GameOwnerCheck<S> {
    pid: Box<dyn Fn() -> Option<u32> + Send + Sync>,
    source: Arc<S>,
    waiting: AtomicUsize,
    in_flight: Mutex<Option<SharedRead>>,
}

impl<S: TableSource> GameOwnerCheck<S> {
    /// A check that passes connections owned by `pid()`, which answers the running game's pid.
    pub fn new(pid: impl Fn() -> Option<u32> + Send + Sync + 'static, source: S) -> Self {
        Self {
            pid: Box::new(pid),
            source: Arc::new(source),
            waiting: AtomicUsize::new(0),
            in_flight: Mutex::new(None),
        }
    }

    /// Joins the read in flight, or starts one. Nothing is kept after it ends, so each batch reads afresh.
    async fn read_table(&self) -> Option<Arc<str>> {
        let read = {
            let mut slot = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
            match slot.as_ref() {
                Some(read) => read.clone(),
                None => {
                    let source = Arc::clone(&self.source);
                    let read: BoxFuture<'static, Option<Arc<str>>> =
                        Box::pin(async move { source.read().await.map(Arc::from) });
                    let read = read.shared();
                    *slot = Some(read.clone());
                    read
                }
            }
        };
        let table = read.clone().await;
        let mut slot = self.in_flight.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(|current| current.ptr_eq(&read)) {
            *slot = None;
        }
        table
    }

    /// Whether the connection `peer` → `local` is the game's.
    pub async fn check(&self, peer: SocketAddr, local: SocketAddr) -> bool {
        let Some(pid) = (self.pid)().filter(|pid| *pid > 0) else {
            return false;
        };
        let (Some(client), Some(server)) = (loopback_v4(peer), loopback_v4(local)) else {
            return false;
        };
        let _waiting = match Waiting::enter(&self.waiting) {
            Some(guard) => guard,
            None => return false,
        };
        for attempt in 0..2 {
            let Some(table) = self.read_table().await else {
                return false;
            };
            match table::owner(&table, client, server) {
                Owner::Pid(owner) => return owner == pid,
                Owner::Refused => return false,
                Owner::NotFound if attempt == 0 => tokio::time::sleep(RETRY_DELAY).await,
                Owner::NotFound => {}
            }
        }
        false
    }
}

impl<S: TableSource> OwnerCheck for GameOwnerCheck<S> {
    fn allows(&self, peer: SocketAddr, local: SocketAddr) -> BoxFuture<'_, bool> {
        Box::pin(self.check(peer, local))
    }
}

struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn enter(count: &'a AtomicUsize) -> Option<Self> {
        if count.fetch_add(1, Ordering::AcqRel) >= MAX_WAITING {
            count.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Self(count))
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A loopback IPv4 end with a port (IPv4-mapped counts).
#[must_use]
pub fn loopback_v4(addr: SocketAddr) -> Option<SocketAddrV4> {
    let ip = match addr {
        SocketAddr::V4(v4) => *v4.ip(),
        SocketAddr::V6(v6) => v6.ip().to_ipv4_mapped()?,
    };
    (ip.is_loopback() && addr.port() != 0).then(|| SocketAddrV4::new(ip, addr.port()))
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
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::sync::atomic::AtomicU32;

    use super::*;

    /// Answers queued tables in turn (the last one repeats), counting reads, each after `delay`.
    struct FakeTable {
        tables: Mutex<Vec<Option<String>>>,
        reads: Arc<AtomicUsize>,
        delay: Duration,
    }

    impl FakeTable {
        fn new(tables: &[Option<&str>], delay: Duration) -> (Self, Arc<AtomicUsize>) {
            let reads = Arc::new(AtomicUsize::new(0));
            let fake = Self {
                tables: Mutex::new(tables.iter().rev().map(|t| t.map(str::to_owned)).collect()),
                reads: Arc::clone(&reads),
                delay,
            };
            (fake, reads)
        }
    }

    impl TableSource for FakeTable {
        fn read(&self) -> BoxFuture<'static, Option<String>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let mut tables = self.tables.lock().unwrap();
            let table = if tables.len() > 1 {
                tables.pop().unwrap()
            } else {
                tables[0].clone()
            };
            let delay = self.delay;
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                table
            })
        }
    }

    const PEER: &str = "127.0.0.1:52000";
    const LOCAL: &str = "127.0.0.1:61100";
    const ROW: &str = "  TCP    127.0.0.1:52000        127.0.0.1:61100        ESTABLISHED     4242\r\n";

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[tokio::test]
    async fn the_games_row_passes_and_anothers_does_not() {
        let (table, _) = FakeTable::new(&[Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(check.check(addr(PEER), addr(LOCAL)).await);
        let (table, _) = FakeTable::new(&[Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4243), table);
        assert!(!check.check(addr(PEER), addr(LOCAL)).await);
    }

    #[tokio::test]
    async fn no_game_or_no_loopback_v4_refuses_without_a_read() {
        for pid in [None, Some(0)] {
            let (table, reads) = FakeTable::new(&[Some(ROW)], Duration::ZERO);
            let check = GameOwnerCheck::new(move || pid, table);
            assert!(!check.check(addr(PEER), addr(LOCAL)).await);
            assert_eq!(reads.load(Ordering::SeqCst), 0);
        }
        let (table, reads) = FakeTable::new(&[Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(!check.check(addr("[::1]:52000"), addr(LOCAL)).await);
        assert!(!check.check(addr("10.0.0.2:52000"), addr(LOCAL)).await);
        assert!(!check.check(addr("127.0.0.1:0"), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        // Mapped addresses count
        assert!(check.check(addr("[::ffff:127.0.0.1]:52000"), addr(LOCAL)).await);
    }

    #[tokio::test]
    async fn a_missing_row_gets_one_more_read() {
        let (table, reads) = FakeTable::new(&[Some(""), Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(check.check(addr(PEER), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        let (table, reads) = FakeTable::new(&[Some(""), Some(""), Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(!check.check(addr(PEER), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_unreadable_table_or_duplicate_rows_refuse() {
        let (table, _) = FakeTable::new(&[None], Duration::ZERO);
        assert!(
            !GameOwnerCheck::new(|| Some(4242), table)
                .check(addr(PEER), addr(LOCAL))
                .await
        );
        let twice = format!("{ROW}{ROW}");
        let (table, _) = FakeTable::new(&[Some(&twice)], Duration::ZERO);
        assert!(
            !GameOwnerCheck::new(|| Some(4242), table)
                .check(addr(PEER), addr(LOCAL))
                .await
        );
    }

    #[tokio::test]
    async fn checks_waiting_together_share_one_read_and_later_ones_read_afresh() {
        let (table, reads) = FakeTable::new(&[Some(ROW)], Duration::from_millis(100));
        let check = Arc::new(GameOwnerCheck::new(|| Some(4242), table));
        let runs: Vec<_> = (0..10)
            .map(|_| {
                let check = Arc::clone(&check);
                tokio::spawn(async move { check.check(addr(PEER), addr(LOCAL)).await })
            })
            .collect();
        for run in runs {
            assert!(run.await.unwrap());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(check.check(addr(PEER), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), 2, "nothing is cached");
    }

    #[tokio::test]
    async fn the_sixty_fifth_waiting_check_is_refused() {
        let (table, _) = FakeTable::new(&[Some(ROW)], Duration::from_millis(300));
        let check = Arc::new(GameOwnerCheck::new(|| Some(4242), table));
        let runs: Vec<_> = (0..MAX_WAITING)
            .map(|_| {
                let check = Arc::clone(&check);
                tokio::spawn(async move { check.check(addr(PEER), addr(LOCAL)).await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!check.check(addr(PEER), addr(LOCAL)).await);
        for run in runs {
            assert!(run.await.unwrap());
        }
        assert!(check.check(addr(PEER), addr(LOCAL)).await, "the count goes back down");
    }

    #[tokio::test]
    async fn the_pid_is_read_per_check() {
        let pid = Arc::new(AtomicU32::new(4242));
        let seen = Arc::clone(&pid);
        let (table, _) = FakeTable::new(&[Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(move || Some(seen.load(Ordering::SeqCst)), table);
        assert!(check.check(addr(PEER), addr(LOCAL)).await);
        pid.store(0, Ordering::SeqCst);
        assert!(!check.check(addr(PEER), addr(LOCAL)).await);
    }

    #[test]
    fn only_windows_has_a_check() {
        assert_eq!(game_owner_check(|| Some(1)).is_some(), cfg!(windows));
    }
}
