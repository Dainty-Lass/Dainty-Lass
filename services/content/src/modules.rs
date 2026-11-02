// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! STAND-IN, written clean-room from the spec's summary (section 2.14) so the crate builds and its routes can be
//! tested. The integration pass replaces this file with the Category A `modules.rs` (DF4b) moved from
//! dainty-restricted; the calls the rest of the crate makes into it are listed in
//! `docs/port/clean-room/content.md`.
//!
//! Content as a core plugin keyed by game version: each attached game module's client profile, its Epic
//! manifest and its runtime folder names.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::UNIX_EPOCH;

use bytes::Bytes;
use module_api::manifest::{ClientProfile, ModuleKind, ModuleManifest};
use module_api::registrar::Registration;
use module_api::{CoreCapability, CorePlugin, ErrorKind, GameVersion, ModuleError, ModuleId};
use serde::Serialize;
use serde_json::json;
use sha1::Digest as _;

use crate::log;

/// The version content serves when a request names none.
pub const DEFAULT_VERSION: &str = "1.14.7";
/// Where a module registers its Epic manifest (`ContentFile::path`).
pub const EPIC_MANIFEST_PATH: &str = "epic/manifest";
pub const MAX_EPIC_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// The folder each module's files are relative to, as the registry knows it.
pub trait RegistryView: Send + Sync {
    fn module_dir(&self, module: &ModuleId) -> Option<PathBuf>;
}

/// A registry that knows no folders: nothing a module registered can be read.
pub struct NoModuleDirs;

impl RegistryView for NoModuleDirs {
    fn module_dir(&self, _: &ModuleId) -> Option<PathBuf> {
        None
    }
}

/// `GET /content/v1/versions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionsBody {
    pub default_version: String,
    pub versions: Vec<VersionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionSummary {
    pub game_version: GameVersion,
    pub module: ModuleId,
    pub display_name: String,
    pub invite_aliases: Vec<String>,
    pub unverified: bool,
    pub client: bool,
    pub epic_manifest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpicManifestError {
    UnknownVersion,
    NotServed,
    Unavailable(String),
    Mismatch { expected: String, found: String },
}

/// A served Epic manifest: its bytes and the SHA-1 the client profile pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicManifest {
    pub bytes: Bytes,
    pub sha1: String,
}

struct Entry {
    manifest: ModuleManifest,
    /// The registered `epic/manifest` file, relative to the module's folder
    epic_source: Option<String>,
}

type EpicCache = BTreeMap<GameVersion, (u64, u128, EpicManifest)>;

pub struct ContentModules {
    view: Arc<dyn RegistryView>,
    entries: RwLock<BTreeMap<GameVersion, Entry>>,
    epic_cache: Mutex<EpicCache>,
}

fn reforged() -> GameVersion {
    GameVersion::release(1, 14, 7)
}

fn revived() -> GameVersion {
    GameVersion::release(1, 4, 4)
}

fn invalid(message: impl Into<String>) -> ModuleError {
    ModuleError::new(ErrorKind::Invalid, message)
}

impl ContentModules {
    pub fn new(view: Arc<dyn RegistryView>) -> Self {
        Self {
            view,
            entries: RwLock::default(),
            epic_cache: Mutex::default(),
        }
    }

    /// No modules, and no folders to read their files from.
    pub fn none() -> Self {
        Self::new(Arc::new(NoModuleDirs))
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<GameVersion, Entry>> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The version `name` names: an attached module's version or invite alias, else a built-in name.
    pub fn resolve(&self, name: &str) -> Option<GameVersion> {
        if let Some(version) = self
            .read()
            .values()
            .find(|entry| entry.manifest.answers_invite(name))
            .map(|entry| entry.manifest.game_version.clone())
        {
            return Some(version);
        }
        match name {
            "1.14.7" | "reforged" => Some(reforged()),
            "1.4.4" | "revived" => Some(revived()),
            _ => None,
        }
    }

    /// `?edition=`: `reborn` or `legacy`.
    pub fn resolve_edition(&self, name: &str) -> Option<GameVersion> {
        match name {
            "reborn" => Some(reforged()),
            "legacy" => Some(revived()),
            _ => None,
        }
    }

    pub fn versions(&self) -> VersionsBody {
        let versions = self
            .read()
            .values()
            .map(|entry| VersionSummary {
                game_version: entry.manifest.game_version.clone(),
                module: entry.manifest.id.clone(),
                display_name: entry.manifest.display_name.clone(),
                invite_aliases: entry.manifest.invite_aliases.clone(),
                unverified: entry.manifest.unverified,
                client: entry.manifest.client.is_some(),
                epic_manifest: entry.epic_source.is_some(),
            })
            .collect();
        VersionsBody {
            default_version: DEFAULT_VERSION.to_owned(),
            versions,
        }
    }

    /// `None` when no module serves `version`; `Some(None)` when its module has no client profile.
    pub fn client_profile(&self, version: &GameVersion) -> Option<Option<ClientProfile>> {
        self.read().get(version).map(|entry| entry.manifest.client.clone())
    }

    /// The runtime folder names of `version`, in the order they are tried.
    pub fn runtime_folders(&self, version: &GameVersion) -> Vec<String> {
        let mut names = vec![version.to_string()];
        if let Some(entry) = self.read().get(version) {
            names.extend(entry.manifest.invite_aliases.iter().cloned());
        }
        if *version == reforged() {
            names.push("reforged".to_owned());
        } else if *version == revived() {
            names.push("revived".to_owned());
        }
        let mut folders: Vec<String> = Vec::new();
        for name in names {
            let ok = !name.starts_with('.')
                && !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
            if ok && !folders.contains(&name) {
                folders.push(name);
            }
        }
        folders
    }

    /// The module's registered Epic manifest, served only while its SHA-1 is the one the profile pins. Blocking.
    pub fn epic_manifest(&self, version: &GameVersion) -> Result<EpicManifest, EpicManifestError> {
        let (id, source, expected) = {
            let entries = self.read();
            let entry = entries.get(version).ok_or(EpicManifestError::UnknownVersion)?;
            let pinned = entry.manifest.client.as_ref().and_then(|c| c.epic.as_ref());
            let (Some(source), Some(epic)) = (&entry.epic_source, pinned) else {
                return Err(EpicManifestError::NotServed);
            };
            (
                entry.manifest.id.clone(),
                source.clone(),
                epic.manifest_sha1.to_string(),
            )
        };
        let unavailable = |why: &str| EpicManifestError::Unavailable(why.to_owned());
        let dir = self
            .view
            .module_dir(&id)
            .ok_or_else(|| unavailable("the module's folder is not known"))?;
        let root = fs::canonicalize(&dir).map_err(|_| unavailable("the module's folder is not readable"))?;
        let path = fs::canonicalize(dir.join(&source)).map_err(|_| unavailable("the file is missing"))?;
        if !path.starts_with(&root) {
            return Err(unavailable("the file leads out of the module's folder"));
        }
        let meta = fs::metadata(&path).map_err(|_| unavailable("the file is missing"))?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_EPIC_MANIFEST_BYTES {
            return Err(unavailable("not a regular file of 1 byte to 64 MB"));
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let mut cache = self.epic_cache.lock().unwrap_or_else(PoisonError::into_inner);
        let manifest = match cache.get(version) {
            Some((len, at, manifest)) if *len == meta.len() && *at == modified => manifest.clone(),
            _ => {
                let mut bytes = Vec::new();
                fs::File::open(&path)
                    .and_then(|file| file.take(MAX_EPIC_MANIFEST_BYTES).read_to_end(&mut bytes))
                    .map_err(|_| unavailable("the file is not readable"))?;
                let sha1 = crate::hex(&sha1::Sha1::digest(&bytes));
                let manifest = EpicManifest {
                    bytes: Bytes::from(bytes),
                    sha1,
                };
                cache.insert(version.clone(), (meta.len(), modified, manifest.clone()));
                manifest
            }
        };
        if manifest.sha1 != expected {
            return Err(EpicManifestError::Mismatch {
                expected,
                found: manifest.sha1,
            });
        }
        Ok(manifest)
    }
}

impl CorePlugin for ContentModules {
    fn name(&self) -> &'static str {
        "content"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::ContentFiles]
    }

    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        // An external module's manifest and registration are untrusted
        module.validate().map_err(|e| invalid(e.to_string()))?;
        registration.check(module).map_err(|e| invalid(e.to_string()))?;
        if module.kind == ModuleKind::Plugin {
            return Ok(());
        }
        let epic_source = registration
            .content
            .iter()
            .find(|file| file.path == EPIC_MANIFEST_PATH)
            .map(|file| file.source.clone());
        let pins_epic = module.client.as_ref().is_some_and(|c| c.epic.is_some());
        if epic_source.is_some() && !pins_epic {
            return Err(invalid(
                "epic/manifest is registered, but the client profile pins no Epic build to check it against",
            ));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(other) = entries.get(&module.game_version)
            && other.manifest.id != module.id
        {
            return Err(invalid(format!(
                "content already serves {} for {}",
                module.game_version, other.manifest.id
            )));
        }
        entries.insert(
            module.game_version.clone(),
            Entry {
                manifest: module.clone(),
                epic_source,
            },
        );
        drop(entries);
        self.epic_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&module.game_version);
        log::info(
            "module content attached",
            json!({
                "module": module.id.as_str(),
                "version": module.game_version.to_string(),
                "client": module.client.is_some(),
                "epicManifest": pins_epic,
            }),
        );
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        let before = entries.len();
        entries.retain(|_, entry| &entry.manifest.id != module);
        let removed = entries.len() != before;
        drop(entries);
        if removed {
            log::info("module content detached", json!({"module": module.as_str()}));
        }
    }
}
