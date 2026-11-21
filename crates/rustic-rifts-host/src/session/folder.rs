// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The per-session folder of config and secret files the transport opens.

use std::path::{Path, PathBuf};
use std::{fs, io};

use rand::TryRng as _;
use rustic_rifts_seam::SessionId;

use super::rules::{safe_config_value, wire_session_id};
use crate::HostError;
use crate::files::{SESSION_FILES, create_private};

/// `N` random bytes as lowercase hex.
pub(crate) fn random_hex<const N: usize>() -> Result<String, HostError> {
    let mut bytes = [0_u8; N];
    if rand::rngs::SysRng.try_fill_bytes(&mut bytes).is_err() {
        return Err(HostError::Io("could not generate session randomness".to_owned()));
    }
    Ok(hex::encode(bytes))
}

fn io_failure(action: &str, path: &Path) -> impl FnOnce(io::Error) -> HostError {
    let what = format!("could not {action} {}", path.display());
    move |e| HostError::Io(format!("{what}: {e}"))
}

#[cfg(unix)]
fn make_private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn make_private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().create(path)
}

/// `<app_local_data>/p2p-sessions/<wire>-<generation>-<16 hex>`, new, owner-only on Unix.
pub fn create_session_directory(
    app_local_data: &Path,
    session_id: &SessionId,
    generation: u64,
) -> Result<PathBuf, HostError> {
    let wire = wire_session_id(session_id);
    safe_config_value(&wire, 64)?;
    let sessions = app_local_data.join("p2p-sessions");
    fs::create_dir_all(&sessions).map_err(io_failure("create", &sessions))?;
    let folder = sessions.join(format!("{wire}-{generation}-{}", random_hex::<8>()?));
    make_private_dir(&folder).map_err(io_failure("create", &folder))?;
    Ok(folder)
}

/// Exclusive create, `0600` on Unix; on Windows the folder's inherited ACL applies.
pub fn write_session_file_0600(path: &Path, contents: &[u8]) -> Result<(), HostError> {
    use std::io::Write as _;
    let mut file = create_private(path).map_err(io_failure("create", path))?;
    let written = file.write_all(contents).and_then(|()| file.sync_all());
    written.map_err(io_failure("write", path))
}

/// Removes the session files and then the folder if nothing else is in it; errors are ignored.
pub fn cleanup_directory(path: &Path) {
    SESSION_FILES
        .iter()
        .for_each(|name| drop(fs::remove_file(path.join(name))));
    drop(fs::remove_dir(path));
}
