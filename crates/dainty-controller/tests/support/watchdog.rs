//! The library tests' watchdog, shared by tests/service_library.rs and tests/chat_library.rs.

use std::time::Duration;

/// Aborts the test binary if a test outlives `limit`: a library stuck in its process would otherwise hold CI forever.
pub struct Watchdog(
    #[allow(
        dead_code,
        reason = "never read: dropping it at the end of the test is what ends the wait"
    )]
    std::sync::mpsc::Sender<()>,
);

pub fn watchdog(test: &'static str, limit: Duration) -> Watchdog {
    let (done, wait) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(limit) {
            eprintln!("{test} ran past {limit:?}: aborting the test binary");
            std::process::abort();
        }
    });
    Watchdog(done)
}
