// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The relay's owner check (RD21): on Windows a loopback connection is served only when the TCP table shows the
//! game owns its client end. Any local process can reach 127.0.0.1, and the relay carries the player's session,
//! so a loopback peer alone is not enough. The table comes from a [`TableSource`]: netstat in the launcher, the
//! IP helper in the runtime.

pub mod table;

use std::net::{SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};

use self::table::Owner;

/// More checks than this waiting at once are refused, so a flood of connections can't queue table reads.
pub const MAX_WAITING: usize = 64;
/// The waits before each further read while the client end has no row. Windows can leave an open loopback
/// connection's client row out of the table (or list it as `LISTENING` to port 0) for most of a second while
/// its server row shows, more often in a burst of connects like the game's login: 230 to 715 ms measured.
pub const RETRY_DELAYS: [Duration; 7] = [
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
    Duration::from_millis(1600),
];

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
        let mut delays = RETRY_DELAYS.iter();
        loop {
            let Some(table) = self.read_table().await else {
                return false;
            };
            match table::owner(&table, client, server) {
                Owner::Pid(owner) => return owner == pid,
                Owner::Refused => return false,
                Owner::NotFound => match delays.next() {
                    Some(delay) => tokio::time::sleep(*delay).await,
                    None => return false,
                },
            }
        }
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
    async fn a_missing_row_is_read_again_until_the_delays_run_out() {
        let (table, reads) = FakeTable::new(&[Some(""), Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(check.check(addr(PEER), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        let (table, reads) = FakeTable::new(&[Some(""), Some(""), Some(ROW)], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(check.check(addr(PEER), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), 3);

        let (table, reads) = FakeTable::new(&[Some("")], Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(!check.check(addr(PEER), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), RETRY_DELAYS.len() + 1);
    }

    /// What Windows showed for an open loopback connection in a burst of 30 (`netstat -ano -p tcp`): the server
    /// row, and the client end as a bound socket or not at all, for up to 715 ms. Read six times, the old check
    /// (one retry after 25 ms) refused it; the game lost a login request this way.
    #[tokio::test]
    async fn a_client_row_windows_lists_late_still_passes() {
        const SERVER_ROW: &str = "  TCP    127.0.0.1:61100        127.0.0.1:52000        ESTABLISHED     4000
";
        const BOUND: &str = "  TCP    127.0.0.1:52000        127.0.0.1:0            LISTENING       4242
";
        let late = format!("{SERVER_ROW}{BOUND}");
        let found = format!("{SERVER_ROW}{ROW}");
        let tables = [
            Some(SERVER_ROW),
            Some(late.as_str()),
            Some(SERVER_ROW),
            Some(late.as_str()),
            Some(SERVER_ROW),
            Some(found.as_str()),
        ];
        let (table, reads) = FakeTable::new(&tables, Duration::ZERO);
        let check = GameOwnerCheck::new(|| Some(4242), table);
        assert!(check.check(addr(PEER), addr(LOCAL)).await);
        assert_eq!(reads.load(Ordering::SeqCst), 6);
        // The bound row is never taken for the connection, even with the game's pid
        let (table, _) = FakeTable::new(&[Some(late.as_str())], Duration::ZERO);
        assert!(
            !GameOwnerCheck::new(|| Some(4242), table)
                .check(addr(PEER), addr(LOCAL))
                .await
        );
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
}
