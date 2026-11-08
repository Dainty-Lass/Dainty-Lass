//! dainty-launcher-kit's owner check for the relay (spec §2.6.5, RD21), as the shell's `net/relay_owner.rs` tested it: the
//! TCP table's rows, fresh shared snapshots that fail closed, and a real socket on Windows.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dainty_launcher_kit::net::relay_owner::table::{Owner, owner};
use dainty_launcher_kit::net::relay_owner::{GameOwnerCheck, TableSource, game_owner_check, loopback_v4};
use futures_util::future::BoxFuture;

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

const ROW: &str = " TCP 127.0.0.1:52123 127.0.0.1:61100 ESTABLISHED 123\r\n";

/// The shell's `tcp_table_owns_peer`: the client's row names exactly `pid`.
fn owns(table: &str, local: SocketAddr, remote: SocketAddr, pid: u32) -> bool {
    let (Some(server), Some(client)) = (loopback_v4(local), loopback_v4(remote)) else {
        return false;
    };
    pid > 0 && owner(table, client, server) == Owner::Pid(pid)
}

#[test]
fn the_client_row_and_the_exact_pid_never_the_server_row() {
    let (local, remote) = (addr("127.0.0.1:61100"), addr("127.0.0.1:52123"));
    assert!(owns(ROW, local, remote, 123));
    for table in [
        ROW.replace("123\r", "456\r"),
        format!("{ROW}{ROW}"),
        "TCP 127.0.0.1:61100 127.0.0.1:52123 ESTABLISHED 123".into(),
        ROW.replace("52123", "52124"),
        ROW.replace("127.0.0.1:52123", "127.0.0.2:52123"),
    ] {
        assert!(!owns(&table, local, remote, 123), "{table:?}");
    }
    assert!(owns(ROW, local, addr("[::ffff:127.0.0.1]:52123"), 123));
    assert!(!owns(ROW, local, remote, 0));
    assert!(owns(&ROW.replace("ESTABLISHED", "HERGESTELLT"), local, remote, 123));
}

/// The shell's counting reader: each read takes the next table, the last one stays.
#[derive(Clone)]
struct Counting {
    tables: Arc<Mutex<Vec<String>>>,
    reads: Arc<AtomicUsize>,
}

impl Counting {
    fn new(tables: &[&str], reads: &Arc<AtomicUsize>) -> Self {
        Self {
            tables: Arc::new(Mutex::new(tables.iter().map(|t| (*t).to_owned()).collect())),
            reads: Arc::clone(reads),
        }
    }
}

impl TableSource for Counting {
    fn read(&self) -> BoxFuture<'static, Option<String>> {
        let this = self.clone();
        Box::pin(async move {
            this.reads.fetch_add(1, Ordering::SeqCst);
            let mut tables = this.tables.lock().unwrap();
            Some(if tables.len() > 1 {
                tables.remove(0)
            } else {
                tables[0].clone()
            })
        })
    }
}

struct Failing;

impl TableSource for Failing {
    fn read(&self) -> BoxFuture<'static, Option<String>> {
        Box::pin(async { None })
    }
}

#[tokio::test]
async fn checks_share_fresh_snapshots_and_fail_closed() {
    let (local, remote) = (addr("127.0.0.1:61100"), addr("127.0.0.1:52123"));
    let reads = Arc::new(AtomicUsize::new(0));
    let check = GameOwnerCheck::new(|| Some(123), Counting::new(&[ROW], &reads));
    assert!(check.check(remote, local).await);
    assert!(check.check(remote, local).await);
    assert_eq!(reads.load(Ordering::SeqCst), 2, "no snapshot is reused");

    // Only Windows has the check; elsewhere the relay serves loopback without one
    assert_eq!(game_owner_check(|| Some(123)).is_some(), cfg!(windows));
    let no_game = GameOwnerCheck::new(|| None, Counting::new(&[ROW], &Arc::default()));
    assert!(!no_game.check(remote, local).await);
    assert!(!GameOwnerCheck::new(|| Some(123), Failing).check(remote, local).await);

    // A row that appears late gets one retry; one that never does is refused after it
    let reads = Arc::new(AtomicUsize::new(0));
    let appears = GameOwnerCheck::new(|| Some(123), Counting::new(&["", ROW], &reads));
    assert!(appears.check(remote, local).await);
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    let reads = Arc::new(AtomicUsize::new(0));
    let wrong = GameOwnerCheck::new(|| Some(456), Counting::new(&[ROW], &reads));
    assert!(!wrong.check(remote, local).await);
    assert_eq!(reads.load(Ordering::SeqCst), 1, "a wrong owner is not retried");
}

#[tokio::test]
async fn simultaneous_sockets_share_one_snapshot() {
    struct Slow {
        table: String,
        reads: Arc<AtomicUsize>,
    }
    impl TableSource for Slow {
        fn read(&self) -> BoxFuture<'static, Option<String>> {
            let (reads, table) = (Arc::clone(&self.reads), self.table.clone());
            Box::pin(async move {
                reads.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Some(table)
            })
        }
    }
    let table: String = (0..32)
        .map(|i| ROW.replace("52123", &(52123 + i).to_string()))
        .collect();
    let reads = Arc::new(AtomicUsize::new(0));
    let check = Arc::new(GameOwnerCheck::new(
        || Some(123),
        Slow {
            table,
            reads: Arc::clone(&reads),
        },
    ));
    let local = addr("127.0.0.1:61100");
    let first = {
        let check = Arc::clone(&check);
        tokio::spawn(async move { check.check(addr("127.0.0.1:52123"), local).await })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    let rest =
        futures_util::future::join_all((1..32).map(|i| check.check(addr(&format!("127.0.0.1:{}", 52123 + i)), local)))
            .await;
    assert!(first.await.unwrap() && rest.iter().all(|ok| *ok));
    assert!(reads.load(Ordering::SeqCst) <= 2, "{}", reads.load(Ordering::SeqCst));
}

#[cfg(windows)]
#[tokio::test]
async fn a_real_loopback_socket_resolves_this_process() {
    use dainty_launcher_kit::net::relay_owner::Netstat;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, remote) = listener.accept().await.unwrap();
    let local = accepted.local_addr().unwrap();
    let me = std::process::id();
    assert!(
        GameOwnerCheck::new(move || Some(me), Netstat)
            .check(remote, local)
            .await
    );
    assert!(
        !GameOwnerCheck::new(move || Some(me + 1), Netstat)
            .check(remote, local)
            .await
    );
    drop(client);
}
