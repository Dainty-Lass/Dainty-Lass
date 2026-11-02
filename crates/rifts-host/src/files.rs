// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The files the host PC keeps: hub limits per server and control cursors per account, each one JSON file written
//! atomically, a damaged one set aside as `.bad` before it is replaced.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rifts_seam::AccountId;
use serde::{Deserialize, Serialize};

use crate::HostError;
use crate::session::HubLimits;

/// In the session folder, in the order they are written.
pub const SESSION_FILES: [&str; 4] = ["bridge.conf", "transport.secret", "capability.secret", "ticket.secret"];
/// In the game folder (Guard role `ADMISSION_KEY`).
pub const ADMISSION_PUBLIC_KEY_FILE: &str = "p2p-admission-public.xy";
/// The file name the launcher still pins a digest for; nothing loads it since the transport is linked in (D1).
pub const TRANSPORT_DLL: &str = "RusticTransport.dll";
pub const HOST_SETTINGS_FILE: &str = "p2p-host-settings.json";
pub const HUB_LIMITS_FILE: &str = "realm-host-limits.json";
pub const DISABLE_COOP_FLAG_FILE: &str = "DISABLE_COOP_TRANSPORT.flag";

const MAX_SERVER_ID: usize = 64;
const MAX_ACCOUNT_ID: usize = 128;

pub fn validate_hub_limits(limits: &HubLimits) -> Result<(), HostError> {
    let refuse = |text: &str| Err(HostError::InvalidInput(text.to_owned()));
    if !(1..=64).contains(&limits.max_players) {
        return refuse("the player limit must be from 1 to 64");
    }
    if !(1..=8).contains(&limits.max_servers) {
        return refuse("the server limit must be from 1 to 8");
    }
    if limits.afk_seconds > 7200 {
        return refuse("the AFK limit must be from 0 to 7200 seconds");
    }
    Ok(())
}

pub trait HostState: Send + Sync {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError>;
    fn save_hub_limits(&self, limits: &HubLimits) -> Result<(), HostError>;
    fn control_cursor(&self, account: &AccountId) -> Result<Option<u64>, HostError>;
    fn set_control_cursor(&self, account: &AccountId, sequence: u64) -> Result<(), HostError>;
}

#[derive(Debug)]
pub struct FileHostState {
    config_dir: PathBuf,
    server_id: String,
    lock: Mutex<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredLimits {
    max_players: u8,
    max_servers: u8,
    #[serde(default)]
    afk_seconds: u32,
}

impl From<StoredLimits> for HubLimits {
    fn from(s: StoredLimits) -> Self {
        Self {
            max_players: s.max_players,
            max_servers: s.max_servers,
            afk_seconds: s.afk_seconds,
        }
    }
}

impl From<&HubLimits> for StoredLimits {
    fn from(l: &HubLimits) -> Self {
        Self {
            max_players: l.max_players,
            max_servers: l.max_servers,
            afk_seconds: l.afk_seconds,
        }
    }
}

// Unknown keys are refused so the older single-object layout is not taken for an empty map.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsFileIn {
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<u64>,
    #[serde(default)]
    servers: Option<BTreeMap<String, StoredLimits>>,
}

#[derive(Serialize)]
struct LimitsFileOut<'a> {
    version: u32,
    servers: &'a BTreeMap<String, StoredLimits>,
}

#[derive(Deserialize)]
struct SettingsIn {
    #[serde(default, alias = "control_cursors", rename = "controlCursors")]
    control_cursors: BTreeMap<String, u64>,
}

#[derive(Serialize)]
struct SettingsOut<'a> {
    #[serde(rename = "controlCursors")]
    control_cursors: &'a BTreeMap<String, u64>,
}

struct Read<T> {
    value: T,
    damaged: bool,
}

fn valid_server_id(id: &str) -> bool {
    (1..=MAX_SERVER_ID).contains(&id.len())
}

/// `None` when the file is absent.
fn read_text(path: &Path) -> Result<Option<String>, HostError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(HostError::Io(format!("could not read {}: {e}", path.display()))),
    }
}

/// `realm-host-limits.json.bad` from `realm-host-limits.json`.
fn sibling(path: &Path, extension: &str) -> PathBuf {
    path.with_extension(format!("json.{extension}"))
}

fn atomic_write(path: &Path, damaged: bool, contents: &[u8]) -> Result<(), HostError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| HostError::Io(format!("could not create {}: {e}", dir.display())))?;
    }
    if damaged && path.exists() {
        fs::rename(path, sibling(path, "bad"))
            .map_err(|e| HostError::Io(format!("could not set aside damaged {}: {e}", path.display())))?;
    }
    let tmp = sibling(path, "tmp");
    let _ = fs::remove_file(&tmp);
    let mut file =
        create_private(&tmp).map_err(|e| HostError::Io(format!("could not create {}: {e}", tmp.display())))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|e| HostError::Io(format!("could not write {}: {e}", tmp.display())))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| HostError::Io(format!("could not commit {}: {e}", path.display())))
}

/// A new file, owner-only on Unix; on Windows it inherits the folder's ACL.
pub(crate) fn create_private(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

impl FileHostState {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self::for_server(config_dir, "default")
    }

    pub fn for_server(config_dir: impl Into<PathBuf>, server_id: impl Into<String>) -> Self {
        Self {
            config_dir: config_dir.into(),
            server_id: server_id.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    fn locked(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn limits_path(&self) -> PathBuf {
        self.config_dir.join(HUB_LIMITS_FILE)
    }

    fn settings_path(&self) -> PathBuf {
        self.config_dir.join(HOST_SETTINGS_FILE)
    }

    fn read_limits(&self) -> Result<Read<BTreeMap<String, StoredLimits>>, HostError> {
        let Some(text) = read_text(&self.limits_path())? else {
            return Ok(Read {
                value: BTreeMap::new(),
                damaged: false,
            });
        };
        if let Ok(file) = serde_json::from_str::<LimitsFileIn>(&text) {
            let all = file.servers.unwrap_or_default();
            let total = all.len();
            let kept: BTreeMap<String, StoredLimits> = all
                .into_iter()
                .filter(|(id, l)| valid_server_id(id) && validate_hub_limits(&HubLimits::from(*l)).is_ok())
                .collect();
            let damaged = kept.is_empty() && total > 0;
            return Ok(Read { value: kept, damaged });
        }
        if let Ok(single) = serde_json::from_str::<StoredLimits>(&text)
            && validate_hub_limits(&single.into()).is_ok()
        {
            return Ok(Read {
                value: BTreeMap::from([(self.server_id.clone(), single)]),
                damaged: false,
            });
        }
        Ok(Read {
            value: BTreeMap::new(),
            damaged: true,
        })
    }

    fn read_settings(&self) -> Result<Read<BTreeMap<String, u64>>, HostError> {
        let Some(text) = read_text(&self.settings_path())? else {
            return Ok(Read {
                value: BTreeMap::new(),
                damaged: false,
            });
        };
        Ok(match serde_json::from_str::<SettingsIn>(&text) {
            Ok(settings) => Read {
                value: settings.control_cursors,
                damaged: false,
            },
            Err(_) => Read {
                value: BTreeMap::new(),
                damaged: true,
            },
        })
    }
}

impl HostState for FileHostState {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError> {
        let _guard = self.locked();
        Ok(self
            .read_limits()?
            .value
            .get(&self.server_id)
            .map(|l| HubLimits::from(*l)))
    }

    fn save_hub_limits(&self, limits: &HubLimits) -> Result<(), HostError> {
        validate_hub_limits(limits)?;
        if !valid_server_id(&self.server_id) {
            return Err(HostError::InvalidInput("invalid server id".to_owned()));
        }
        let _guard = self.locked();
        let Read { mut value, damaged } = self.read_limits()?;
        let wanted = StoredLimits::from(limits);
        if !damaged && value.get(&self.server_id) == Some(&wanted) {
            return Ok(());
        }
        value.insert(self.server_id.clone(), wanted);
        let text = serde_json::to_string_pretty(&LimitsFileOut {
            version: 1,
            servers: &value,
        })
        .map_err(|e| HostError::Io(format!("could not encode hub limits: {e}")))?;
        atomic_write(&self.limits_path(), damaged, text.as_bytes())
    }

    fn control_cursor(&self, account: &AccountId) -> Result<Option<u64>, HostError> {
        if account.0.is_empty() {
            return Err(HostError::InvalidInput("account id must not be empty".to_owned()));
        }
        let _guard = self.locked();
        Ok(self.read_settings()?.value.get(&account.0).copied())
    }

    fn set_control_cursor(&self, account: &AccountId, sequence: u64) -> Result<(), HostError> {
        if !(1..=MAX_ACCOUNT_ID).contains(&account.0.len()) {
            return Err(HostError::InvalidInput("invalid account id".to_owned()));
        }
        let _guard = self.locked();
        let Read { mut value, damaged } = self.read_settings()?;
        let stored = value.get(&account.0).copied();
        // A cursor only moves forward: a late or replayed ack never re-delivers what was acked.
        let next = stored.map_or(sequence, |s| s.max(sequence));
        if !damaged && stored == Some(next) {
            return Ok(());
        }
        value.insert(account.0.clone(), next);
        let text = serde_json::to_string_pretty(&SettingsOut {
            control_cursors: &value,
        })
        .map_err(|e| HostError::Io(format!("could not encode host settings: {e}")))?;
        atomic_write(&self.settings_path(), damaged, text.as_bytes())
    }
}
