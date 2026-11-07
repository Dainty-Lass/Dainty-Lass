// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Online backups of a SQLite store.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::OpenFlags;
use rusqlite::backup::{Backup, StepResult};

use crate::database::run_blocking;
use crate::{Error, Store};

const PAGES_PER_STEP: i32 = 128;

impl Store {
    /// Performs an online backup of the store's database file to `destination` within `deadline`.
    pub async fn backup(&self, destination: &Path, deadline: Duration) -> Result<(), Error> {
        let source = self.path().to_path_buf();
        let destination = destination.to_path_buf();
        let busy_timeout = self.busy_timeout();
        run_blocking(move || backup_file(&source, &destination, busy_timeout, deadline)).await
    }
}

/// Copies a consistent point-in-time snapshot of `source` into `destination` using SQLite's online backup API.
pub fn backup_file(source: &Path, destination: &Path, busy_timeout: Duration, deadline: Duration) -> Result<(), Error> {
    if !source.is_file() {
        return Err(Error::Backup(format!(
            "source database {} is not an existing file",
            source.display()
        )));
    }
    if destination.exists() {
        return Err(Error::Backup(format!(
            "backup destination {} already exists",
            destination.display()
        )));
    }

    let start = Instant::now();
    let step_timeout = busy_timeout.min(deadline);

    let src = rusqlite::Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| Error::Backup(e.to_string()))?;
    src.busy_timeout(step_timeout)
        .map_err(|e| Error::Backup(e.to_string()))?;

    let mut dst = rusqlite::Connection::open(destination).map_err(|e| Error::Backup(e.to_string()))?;

    let result = (|| -> Result<(), Error> {
        dst.busy_timeout(step_timeout)
            .map_err(|e| Error::Backup(e.to_string()))?;

        // Hold a read snapshot on the source so concurrent WAL writes do not restart the backup.
        src.execute_batch("BEGIN DEFERRED;")
            .map_err(|e| Error::Backup(e.to_string()))?;
        let _: i64 = src
            .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
            .map_err(|e| Error::Backup(e.to_string()))?;

        if start.elapsed() >= deadline {
            return Err(Error::Backup("backup deadline expired".into()));
        }

        {
            let backup = Backup::new(&src, &mut dst).map_err(|e| Error::Backup(e.to_string()))?;
            loop {
                if start.elapsed() >= deadline {
                    return Err(Error::Backup("backup deadline expired".into()));
                }
                match backup.step(PAGES_PER_STEP).map_err(|e| Error::Backup(e.to_string()))? {
                    StepResult::Done => break,
                    StepResult::More => {}
                    StepResult::Busy | StepResult::Locked => {
                        if start.elapsed() >= deadline {
                            return Err(Error::Backup("backup deadline expired".into()));
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    other => {
                        return Err(Error::Backup(format!("unexpected backup step result: {other:?}")));
                    }
                }
            }
        }

        let _ = src.execute_batch("COMMIT;");

        // Convert the backup into a self-contained single-file database in DELETE journal mode.
        let _: String = dst
            .query_row("PRAGMA journal_mode = DELETE;", [], |row| row.get(0))
            .map_err(|e| Error::Backup(e.to_string()))?;
        Ok(())
    })();

    drop(dst);
    drop(src);

    if result.is_err() {
        let _ = std::fs::remove_file(destination);
        let mut wal = destination.as_os_str().to_owned();
        wal.push("-wal");
        let _ = std::fs::remove_file(wal);
        let mut shm = destination.as_os_str().to_owned();
        shm.push("-shm");
        let _ = std::fs::remove_file(shm);
    }

    result
}
