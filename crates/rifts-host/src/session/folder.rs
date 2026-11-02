// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The per-session folder of config and secret files the transport opens.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use rand::TryRng as _;
use rifts_seam::SessionId;

use super::rules::{safe_config_value, wire_session_id};
use crate::HostError;
use crate::files::{SESSION_FILES, create_private};

pub(crate) fn random_hex<const N: usize>() -> Result<String, HostError> {
    let mut bytes = [0_u8; N];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| HostError::Io("could not generate session randomness".to_owned()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// `<app_local_data>/p2p-sessions/<wire>-<generation>-<16 hex>`, new, owner-only on Unix.
pub fn create_session_directory(
    app_local_data: &Path,
    session_id: &SessionId,
    generation: u64,
) -> Result<PathBuf, HostError> {
    let wire = wire_session_id(session_id);
    safe_config_value(&wire, 64)?;
    let parent = app_local_data.join("p2p-sessions");
    fs::create_dir_all(&parent).map_err(|e| HostError::Io(format!("could not create {}: {e}", parent.display())))?;
    let path = parent.join(format!("{wire}-{generation}-{}", random_hex::<8>()?));
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt as _;
        fs::DirBuilder::new().mode(0o700).create(&path)
    };
    #[cfg(not(unix))]
    let created = fs::DirBuilder::new().create(&path);
    created.map_err(|e| HostError::Io(format!("could not create {}: {e}", path.display())))?;
    Ok(path)
}

/// Exclusive create, `0600` on Unix; on Windows the folder's inherited ACL applies.
pub fn write_session_file_0600(path: &Path, contents: &[u8]) -> Result<(), HostError> {
    let mut file =
        create_private(path).map_err(|e| HostError::Io(format!("could not create {}: {e}", path.display())))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|e| HostError::Io(format!("could not write {}: {e}", path.display())))
}

/// Removes the session files and then the folder if nothing else is in it; errors are ignored.
pub fn cleanup_directory(path: &Path) {
    for name in SESSION_FILES {
        let _ = fs::remove_file(path.join(name));
    }
    let _ = fs::remove_dir(path);
}
