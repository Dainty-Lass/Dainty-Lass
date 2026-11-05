//! What a realm host's PC keeps (RF1, section 6). The launcher keeps no database (DD15): these are files in its
//! config directory, and the session folder's secrets die with the session.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rifts_seam::AccountId;
use serde::{Deserialize, Serialize};

use crate::HostError;
use crate::session::HubLimits;

/// Written into `<app local data>/p2p-sessions/<sessionId>-<generation>-<16 hex>/`.
pub const SESSION_FILES: [&str; 4] = ["bridge.conf", "transport.secret", "capability.secret", "ticket.secret"];

/// In the game folder, from the runtime feed's extra files (Guard role `ADMISSION_KEY`).
pub const ADMISSION_PUBLIC_KEY_FILE: &str = "p2p-admission-public.xy";

/// In the game folder (Guard roles `TRANSPORT_MODULE`, `TRANSPORT_RUNTIME`); DD1 keeps the name.
pub const TRANSPORT_DLL: &str = "ParadoxTransport.dll";

/// Saved control cursors per account in the launcher's config directory (`p2p/control.rs`).
pub const HOST_SETTINGS_FILE: &str = "p2p-host-settings.json";

/// Saved hub limits in the launcher's config directory (`p2p/realm.rs`).
pub const HUB_LIMITS_FILE: &str = "realm-host-limits.json";

/// Local flag file that turns off co-op transport (`p2p/mod.rs`).
pub const DISABLE_COOP_FLAG_FILE: &str = "DISABLE_COOP_TRANSPORT.flag";

const LIMITS_FILE_VERSION: u32 = 1;
const DEFAULT_SERVER_KEY: &str = "default";

/// Validates [`HubLimits`] against the runtime's bounds (`p2p/realm.rs:validate_options`).
pub fn validate_hub_limits(limits: &HubLimits) -> Result<(), HostError> {
    if !(1..=64).contains(&limits.max_players) {
        return Err(HostError::InvalidInput(
            "the player limit must be from 1 to 64".to_owned(),
        ));
    }
    if !(1..=8).contains(&limits.max_servers) {
        return Err(HostError::InvalidInput(
            "the server limit must be from 1 to 8".to_owned(),
        ));
    }
    if limits.afk_seconds > 7200 {
        return Err(HostError::InvalidInput(
            "the AFK limit must be from 0 to 7200 seconds".to_owned(),
        ));
    }
    Ok(())
}

/// `p2p-host-settings.json` and `realm-host-limits.json`.
pub trait HostState: Send + Sync {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError>;
    /// Written through a `.tmp` beside it; a file that fails to parse is kept as `.bad`.
    fn save_hub_limits(&self, limits: &HubLimits) -> Result<(), HostError>;
    /// `control_cursors`: the last control sequence seen per account.
    fn control_cursor(&self, account: &AccountId) -> Result<Option<u64>, HostError>;
    fn set_control_cursor(&self, account: &AccountId, sequence: u64) -> Result<(), HostError>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WireHubLimits {
    max_players: u8,
    max_servers: u8,
    #[serde(default)]
    afk_seconds: u32,
}

impl From<&HubLimits> for WireHubLimits {
    fn from(limits: &HubLimits) -> Self {
        Self {
            max_players: limits.max_players,
            max_servers: limits.max_servers,
            afk_seconds: limits.afk_seconds,
        }
    }
}

impl From<WireHubLimits> for HubLimits {
    fn from(wire: WireHubLimits) -> Self {
        Self {
            max_players: wire.max_players,
            max_servers: wire.max_servers,
            afk_seconds: wire.afk_seconds,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredLimits {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    servers: BTreeMap<String, WireHubLimits>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredHostSettings {
    #[serde(default, alias = "control_cursors")]
    control_cursors: BTreeMap<String, u64>,
}

/// File-backed [`HostState`] storing `realm-host-limits.json` and `p2p-host-settings.json` in the launcher's config
/// folder (`ParadoxLauncher/src-tauri/src/p2p/{realm.rs,control.rs}`).
#[derive(Debug)]
pub struct FileHostState {
    config_dir: PathBuf,
    server_id: String,
    lock: Mutex<()>,
}

impl FileHostState {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self::for_server(config_dir, DEFAULT_SERVER_KEY)
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

    fn limits_path(&self) -> PathBuf {
        self.config_dir.join(HUB_LIMITS_FILE)
    }

    fn settings_path(&self) -> PathBuf {
        self.config_dir.join(HOST_SETTINGS_FILE)
    }

    fn read_limits_map(&self) -> Result<(BTreeMap<String, HubLimits>, bool), HostError> {
        let path = self.limits_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((BTreeMap::new(), false)),
            Err(err) => return Err(HostError::Io(format!("could not read {}: {err}", path.display()))),
        };
        if let Ok(stored) = serde_json::from_str::<StoredLimits>(&text) {
            let mut valid = BTreeMap::new();
            let mut had_invalid = false;
            for (id, wire) in stored.servers {
                let limits = HubLimits::from(wire);
                if !id.is_empty() && id.len() <= 64 && validate_hub_limits(&limits).is_ok() {
                    valid.insert(id, limits);
                } else {
                    had_invalid = true;
                }
            }
            let damaged = had_invalid && valid.is_empty();
            return Ok((valid, damaged));
        }
        if let Ok(single) = serde_json::from_str::<WireHubLimits>(&text) {
            let limits = HubLimits::from(single);
            if validate_hub_limits(&limits).is_ok() {
                let mut map = BTreeMap::new();
                map.insert(self.server_id.clone(), limits);
                return Ok((map, false));
            }
        }
        Ok((BTreeMap::new(), true))
    }

    fn read_settings_map(&self) -> Result<(BTreeMap<String, u64>, bool), HostError> {
        let path = self.settings_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((BTreeMap::new(), false)),
            Err(err) => return Err(HostError::Io(format!("could not read {}: {err}", path.display()))),
        };
        match serde_json::from_str::<StoredHostSettings>(&text) {
            Ok(stored) => Ok((stored.control_cursors, false)),
            Err(_) => Ok((BTreeMap::new(), true)),
        }
    }
}

fn write_atomic_json(path: &Path, damaged: bool, contents: &[u8]) -> Result<(), HostError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| HostError::Io(format!("could not create {}: {err}", parent.display())))?;
    }
    if damaged && path.exists() {
        let bad = path.with_extension("json.bad");
        fs::rename(path, &bad)
            .map_err(|err| HostError::Io(format!("could not set aside damaged {}: {err}", path.display())))?;
    }
    let tmp = path.with_extension("json.tmp");
    let _ = fs::remove_file(&tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|err| HostError::Io(format!("could not create {}: {err}", tmp.display())))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|err| HostError::Io(format!("could not write {}: {err}", tmp.display())))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|err| HostError::Io(format!("could not commit {}: {err}", path.display())))
}

impl HostState for FileHostState {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError> {
        let _guard = self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut servers, _) = self.read_limits_map()?;
        Ok(servers.remove(&self.server_id))
    }

    fn save_hub_limits(&self, limits: &HubLimits) -> Result<(), HostError> {
        validate_hub_limits(limits)?;
        if self.server_id.is_empty() || self.server_id.len() > 64 {
            return Err(HostError::InvalidInput("invalid server id".to_owned()));
        }
        let _guard = self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut servers, damaged) = self.read_limits_map()?;
        if !damaged && servers.get(&self.server_id) == Some(limits) {
            return Ok(());
        }
        servers.insert(self.server_id.clone(), *limits);
        let stored = StoredLimits {
            version: LIMITS_FILE_VERSION,
            servers: servers
                .iter()
                .map(|(k, v)| (k.clone(), WireHubLimits::from(v)))
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&stored)
            .map_err(|err| HostError::Io(format!("could not encode hub limits: {err}")))?;
        write_atomic_json(&self.limits_path(), damaged, &bytes)
    }

    fn control_cursor(&self, account: &AccountId) -> Result<Option<u64>, HostError> {
        if account.0.is_empty() {
            return Err(HostError::InvalidInput("account id must not be empty".to_owned()));
        }
        let _guard = self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (cursors, _) = self.read_settings_map()?;
        Ok(cursors.get(account.0.as_str()).copied())
    }

    fn set_control_cursor(&self, account: &AccountId, sequence: u64) -> Result<(), HostError> {
        if account.0.is_empty() || account.0.len() > 128 {
            return Err(HostError::InvalidInput("invalid account id".to_owned()));
        }
        let _guard = self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut cursors, damaged) = self.read_settings_map()?;
        let current = cursors.get(account.0.as_str()).copied();
        let next = current.map_or(sequence, |prev| prev.max(sequence));
        if !damaged && current == Some(next) {
            return Ok(());
        }
        cursors.insert(account.0.clone(), next);
        let stored = StoredHostSettings {
            control_cursors: cursors,
        };
        let bytes = serde_json::to_vec_pretty(&stored)
            .map_err(|err| HostError::Io(format!("could not encode host settings: {err}")))?;
        write_atomic_json(&self.settings_path(), damaged, &bytes)
    }
}
