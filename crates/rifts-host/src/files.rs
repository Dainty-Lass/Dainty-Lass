// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The files the host PC keeps: hub limits per server and control cursors per account, each one JSON file written
//! atomically, a damaged one set aside as `.bad` before it is replaced.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write as _},
    ops::RangeInclusive,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
};

use serde::{Deserialize, Serialize};

use crate::{HostError, session::HubLimits};

// In the launcher's config folder.
pub const HUB_LIMITS_FILE: &str = "realm-host-limits.json";
pub const HOST_SETTINGS_FILE: &str = "p2p-host-settings.json";

/// In the session folder, in the order they are written.
pub const SESSION_FILES: [&str; 4] = ["bridge.conf", "transport.secret", "capability.secret", "ticket.secret"];

// In the game folder.
/// Guard role `ADMISSION_KEY`.
pub const ADMISSION_PUBLIC_KEY_FILE: &str = "p2p-admission-public.xy";
pub const DISABLE_COOP_FLAG_FILE: &str = "DISABLE_COOP_TRANSPORT.flag";
/// The file name the launcher still pins a digest for; nothing loads it since the transport is linked in (D1).
pub const TRANSPORT_DLL: &str = "RusticTransport.dll";

const SERVER_ID_LENGTHS: RangeInclusive<usize> = 1..=64;
const ACCOUNT_ID_LENGTHS: RangeInclusive<usize> = 1..=128;

pub fn validate_hub_limits(limits: &HubLimits) -> Result<(), HostError> {
    let players = (1..=64).contains(&limits.max_players);
    let servers = (1..=8).contains(&limits.max_servers);
    let afk = limits.afk_seconds <= 7200;
    let refusal = [
        (players, "the player limit must be from 1 to 64"),
        (servers, "the server limit must be from 1 to 8"),
        (afk, "the AFK limit must be from 0 to 7200 seconds"),
    ]
    .into_iter()
    .find_map(|(fine, text)| (!fine).then_some(text));
    refusal.map_or(Ok(()), |text| Err(HostError::InvalidInput(text.to_owned())))
}

pub trait HostState: Send + Sync {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError>;
    fn save_hub_limits(&self, limits: &HubLimits) -> Result<(), HostError>;
    fn control_cursor(&self, account: &rifts_seam::AccountId) -> Result<Option<u64>, HostError>;
    fn set_control_cursor(&self, account: &rifts_seam::AccountId, sequence: u64) -> Result<(), HostError>;
}

/// The two files in one folder, for one server id; one lock serialises every read-modify-write.
#[derive(Debug)]
pub struct FileHostState {
    folder: PathBuf,
    server: String,
    busy: Mutex<()>,
}

/// One server's limits as stored; a file from before the AFK limit has none.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StoredLimits {
    #[serde(rename = "maxPlayers")]
    players: u8,
    #[serde(rename = "maxServers")]
    servers: u8,
    #[serde(rename = "afkSeconds", default)]
    afk: u32,
}

impl From<StoredLimits> for HubLimits {
    fn from(stored: StoredLimits) -> Self {
        Self {
            max_players: stored.players,
            max_servers: stored.servers,
            afk_seconds: stored.afk,
        }
    }
}

impl StoredLimits {
    fn of(limits: &HubLimits) -> Self {
        Self {
            players: limits.max_players,
            servers: limits.max_servers,
            afk: limits.afk_seconds,
        }
    }

    fn valid(self) -> bool {
        validate_hub_limits(&self.into()).is_ok()
    }
}

type ServerLimits = BTreeMap<String, StoredLimits>;
type Cursors = BTreeMap<String, u64>;

/// `realm-host-limits.json`, read as `<Option<u64>, Option<map>>` and written as `<u32, &map>`. Unknown keys are
/// refused so the older single-object layout is not taken for an empty map.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LimitsFile<V, S> {
    #[serde(default)]
    version: V,
    #[serde(default)]
    servers: S,
}

/// `p2p-host-settings.json`; the snake_case key is read from older files.
#[derive(Deserialize, Serialize)]
struct SettingsFile<C> {
    #[serde(default, alias = "control_cursors", rename = "controlCursors")]
    cursors: C,
}

/// A file's content, and whether it was damaged and must be set aside before the next write.
struct Loaded<T> {
    value: T,
    damaged: bool,
}

impl<T: Default> Loaded<T> {
    fn intact(value: T) -> Self {
        Self { value, damaged: false }
    }

    fn unreadable() -> Self {
        Self {
            value: T::default(),
            damaged: true,
        }
    }
}

fn io_failure(action: &str, path: &Path) -> impl FnOnce(io::Error) -> HostError {
    let what = format!("could not {action} {}", path.display());
    move |e| HostError::Io(format!("{what}: {e}"))
}

/// `None` when the file is absent.
fn text_of(path: &Path) -> Result<Option<String>, HostError> {
    match fs::read(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        read => read
            .map(|bytes| Some(String::from_utf8_lossy(&bytes).into_owned()))
            .map_err(io_failure("read", path)),
    }
}

/// `realm-host-limits.json.bad` from `realm-host-limits.json`.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    path.with_extension(["json", suffix].join("."))
}

/// Staged in `.tmp` and renamed over the file, after moving a damaged one to `.bad`.
fn replace_file(path: &Path, damaged: bool, contents: &[u8]) -> Result<(), HostError> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(io_failure("create", folder))?;
    }
    if damaged && path.exists() {
        fs::rename(path, beside(path, "bad")).map_err(io_failure("set aside damaged", path))?;
    }
    let staging = beside(path, "tmp");
    let _ = fs::remove_file(&staging);
    let mut file = create_private(&staging).map_err(io_failure("create", &staging))?;
    let written = file.write_all(contents).and_then(|()| file.sync_all());
    written.map_err(io_failure("write", &staging))?;
    drop(file);
    fs::rename(&staging, path).map_err(io_failure("commit", path))
}

/// A new file, owner-only on Unix; on Windows it inherits the folder's ACL.
pub(crate) fn create_private(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

fn pretty<T: Serialize>(value: &T, what: &str) -> Result<String, HostError> {
    serde_json::to_string_pretty(value).map_err(|e| HostError::Io(format!("could not encode {what}: {e}")))
}

fn invalid(text: &str) -> HostError {
    HostError::InvalidInput(text.to_owned())
}

impl FileHostState {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self::for_server(config_dir, "default")
    }

    pub fn for_server(config_dir: impl Into<PathBuf>, server_id: impl Into<String>) -> Self {
        let (folder, server) = (config_dir.into(), server_id.into());
        Self {
            folder,
            server,
            busy: Mutex::default(),
        }
    }

    pub fn config_dir(&self) -> &Path {
        self.folder.as_path()
    }

    pub fn server_id(&self) -> &str {
        self.server.as_str()
    }

    fn hold(&self) -> MutexGuard<'_, ()> {
        self.busy.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The per-server map; an older single-object file counts as this server's entry.
    fn load_limits(&self) -> Result<Loaded<ServerLimits>, HostError> {
        let Some(text) = text_of(&self.folder.join(HUB_LIMITS_FILE))? else {
            return Ok(Loaded::intact(ServerLimits::new()));
        };
        if let Ok(file) = serde_json::from_str::<LimitsFile<Option<u64>, Option<ServerLimits>>>(&text) {
            let mut servers = file.servers.unwrap_or_default();
            let had_entries = !servers.is_empty();
            servers.retain(|id, limits| SERVER_ID_LENGTHS.contains(&id.len()) && limits.valid());
            // Entries that were all bad make the file damaged; an empty map does not.
            let damaged = had_entries && servers.is_empty();
            return Ok(Loaded {
                value: servers,
                damaged,
            });
        }
        Ok(match serde_json::from_str::<StoredLimits>(&text) {
            Ok(single) if single.valid() => Loaded::intact(ServerLimits::from([(self.server.clone(), single)])),
            _ => Loaded::unreadable(),
        })
    }

    fn load_cursors(&self) -> Result<Loaded<Cursors>, HostError> {
        let Some(text) = text_of(&self.folder.join(HOST_SETTINGS_FILE))? else {
            return Ok(Loaded::intact(Cursors::new()));
        };
        let parsed = serde_json::from_str::<SettingsFile<Cursors>>(&text);
        Ok(parsed.map_or_else(|_| Loaded::unreadable(), |file| Loaded::intact(file.cursors)))
    }
}

impl HostState for FileHostState {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError> {
        let _held = self.hold();
        let servers = self.load_limits()?.value;
        Ok(servers.get(&self.server).map(|stored| (*stored).into()))
    }

    fn save_hub_limits(&self, limits: &HubLimits) -> Result<(), HostError> {
        validate_hub_limits(limits)?;
        if !SERVER_ID_LENGTHS.contains(&self.server.len()) {
            return Err(invalid("invalid server id"));
        }
        let _held = self.hold();
        let mut loaded = self.load_limits()?;
        let wanted = StoredLimits::of(limits);
        if !loaded.damaged && loaded.value.get(&self.server) == Some(&wanted) {
            return Ok(());
        }
        loaded.value.insert(self.server.clone(), wanted);
        let file = LimitsFile {
            version: 1_u32,
            servers: &loaded.value,
        };
        let text = pretty(&file, "hub limits")?;
        replace_file(&self.folder.join(HUB_LIMITS_FILE), loaded.damaged, text.as_bytes())
    }

    fn control_cursor(&self, account: &rifts_seam::AccountId) -> Result<Option<u64>, HostError> {
        let id = account.0.as_str();
        if id.is_empty() {
            return Err(invalid("account id must not be empty"));
        }
        let _held = self.hold();
        Ok(self.load_cursors()?.value.get(id).copied())
    }

    fn set_control_cursor(&self, account: &rifts_seam::AccountId, sequence: u64) -> Result<(), HostError> {
        let id = account.0.as_str();
        if !ACCOUNT_ID_LENGTHS.contains(&id.len()) {
            return Err(invalid("invalid account id"));
        }
        let _held = self.hold();
        let mut loaded = self.load_cursors()?;
        let stored = loaded.value.get(id).copied();
        // A cursor only moves forward: a late or replayed ack never re-delivers what was acked.
        let next = stored.map_or(sequence, |kept| kept.max(sequence));
        if !loaded.damaged && stored == Some(next) {
            return Ok(());
        }
        loaded.value.insert(id.to_owned(), next);
        let text = pretty(&SettingsFile { cursors: &loaded.value }, "host settings")?;
        replace_file(&self.folder.join(HOST_SETTINGS_FILE), loaded.damaged, text.as_bytes())
    }
}
