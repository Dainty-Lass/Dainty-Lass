// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Consistent single-file online backup of a live SQLite database.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::OpenFlags;
use rusqlite::backup::{Backup, StepResult};

use crate::{Error, Store, run_blocking};

impl Store {
    pub async fn backup(&self, destination: &Path, deadline: Duration) -> Result<(), Error> {
        let source = self.path.clone();
        let destination = destination.to_path_buf();
        let busy_timeout = self.busy_timeout;
        run_blocking(move || backup_file(&source, &destination, busy_timeout, deadline)).await
    }
}

pub fn backup_file(source: &Path, destination: &Path, busy_timeout: Duration, deadline: Duration) -> Result<(), Error> {
    if !source.is_file() {
        return Err(Error::Backup(format!("no database at {}", source.display())));
    }
    if destination.exists() {
        return Err(Error::Backup(format!("{} exists already", destination.display())));
    }

    let result = perform_backup(source, destination, busy_timeout, deadline);
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

fn perform_backup(source: &Path, destination: &Path, busy_timeout: Duration, deadline: Duration) -> Result<(), Error> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let src = rusqlite::Connection::open_with_flags(source, flags).map_err(|e| Error::Backup(e.to_string()))?;
    src.busy_timeout(busy_timeout)
        .map_err(|e| Error::Backup(e.to_string()))?;
    src.execute_batch("BEGIN; SELECT count(*) FROM sqlite_schema;")
        .map_err(|e| Error::Backup(e.to_string()))?;

    let mut dst = rusqlite::Connection::open(destination).map_err(|e| Error::Backup(e.to_string()))?;

    let start = Instant::now();
    {
        let backup = Backup::new(&src, &mut dst).map_err(|e| Error::Backup(e.to_string()))?;
        loop {
            match backup.step(4096).map_err(|e| Error::Backup(e.to_string()))? {
                StepResult::Done => break,
                StepResult::More => {
                    if Instant::now() >= start + deadline {
                        return Err(Error::Backup("the backup deadline passed".into()));
                    }
                }
                StepResult::Busy | StepResult::Locked => {
                    if Instant::now() >= start + deadline {
                        return Err(Error::Backup("the backup deadline passed".into()));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    if Instant::now() >= start + deadline {
                        return Err(Error::Backup("the backup deadline passed".into()));
                    }
                }
            }
        }
    }

    let _ = src.execute_batch("COMMIT;");
    dst.execute_batch("PRAGMA journal_mode = DELETE;")
        .map_err(|e| Error::Backup(e.to_string()))?;
    Ok(())
}
