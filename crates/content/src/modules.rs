// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Content as a core plugin (DD3, DF4b): what each loaded game module registers with content, keyed by its
//! game version (DD4). The controller (DF3) hands a module's manifest and registration to
//! [`ContentModules::attach`] as it loads, and content then serves, per version:
//!
//! - the module's client profile (DD12), from its manifest;
//! - its Epic manifest (DD22), the file it registered at [`EPIC_MANIFEST_PATH`], served only while its
//!   SHA-1 is the one the profile pins;
//! - the runtime, from `<runtime dir>/<version or alias>` (`runtime.rs`).
//!
//! A version, an alias or an edition name resolves to a version here; nothing resolved means the server's
//! own runtime, 1.14.7, as before modules.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::SystemTime;

use bytes::Bytes;
use module_api::registrar::ContentFile;
use module_api::{
    ClientProfile, CoreCapability, CorePlugin, ErrorKind, GameVersion, ModuleError, ModuleId, ModuleKind,
    ModuleManifest, Registration, Sha1Hex,
};
use serde::Serialize;
use serde_json::json;
use sha1::{Digest, Sha1};

use service_kit::linelog;

/// What a request without a version gets (DF4b): the runtime this server always served.
pub const DEFAULT_VERSION: &str = "1.14.7";

/// Where a module registers its pinned Epic manifest (`Registrar::content_file`).
pub const EPIC_MANIFEST_PATH: &str = "epic/manifest";

/// The largest Epic manifest served: the epic crate's own ceiling on a manifest's data.
pub const MAX_EPIC_MANIFEST_BYTES: u64 = 64 << 20;

/// Versions and aliases known before any module registers one; DF3 removes `protocol::game::Game`, which
/// held them.
const BUILT_IN: [(&str, &str); 2] = [("1.14.7", "reforged"), ("1.4.4", "revived")];

/// `edition=`: the names `Edition` and `Game` gave the two games (DF4b keeps them working).
const EDITIONS: [(&str, &str); 2] = [("reborn", "1.14.7"), ("legacy", "1.4.4")];

/// What content needs from the controller's registry (DF3) that a registration does not carry.
pub trait RegistryView: Send + Sync {
    /// The folder a loaded module's `ContentFile::source` paths are relative to, None when it has none.
    fn module_dir(&self, module: &ModuleId) -> Option<PathBuf>;
}

/// A registry with no module folders: content without a controller.
pub struct NoModuleDirs;

impl RegistryView for NoModuleDirs {
    fn module_dir(&self, _module: &ModuleId) -> Option<PathBuf> {
        None
    }
}

struct Cached {
    len: u64,
    modified: Option<SystemTime>,
    bytes: Bytes,
}

struct Entry {
    manifest: ModuleManifest,
    epic_file: Option<ContentFile>,
    /// The last Epic manifest that matched its SHA-1, with the size and time it was read at
    epic_cache: Mutex<Option<Cached>>,
}

/// One version's row in `/content/v1/versions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionSummary {
    pub game_version: GameVersion,
    pub module: ModuleId,
    pub display_name: String,
    pub invite_aliases: Vec<String>,
    pub unverified: bool,
    pub client: bool,
    /// This server serves the pinned Epic manifest (DD22: the launcher tries a joined server's copy first)
    pub epic_manifest: bool,
}

/// The body of `/content/v1/versions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionsBody {
    pub default_version: &'static str,
    pub versions: Vec<VersionSummary>,
}

/// Why a version's Epic manifest is not served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpicManifestError {
    /// No module serves the version (404 `unknown_version`)
    UnknownVersion,
    /// The module pins no Epic build or registered no manifest (404 `no_epic_manifest`)
    NotServed,
    /// Registered but not readable as a regular file under the cap (503 `epic_manifest_unavailable`)
    Unavailable(String),
    /// The file is not the pinned one (503 `epic_manifest_invalid`)
    Mismatch { expected: Sha1Hex, found: Sha1Hex },
}

/// The version-keyed table content serves from: the [`CorePlugin`] the controller attaches modules to.
pub struct ContentModules {
    view: Arc<dyn RegistryView>,
    by_version: RwLock<BTreeMap<GameVersion, Arc<Entry>>>,
}

impl Default for ContentModules {
    fn default() -> Self {
        Self::new(Arc::new(NoModuleDirs))
    }
}

fn invalid(message: impl Into<String>) -> ModuleError {
    ModuleError::new(ErrorKind::Invalid, message)
}

/// A folder name a version or alias may give the runtime: never a path, `.` or `..`.
fn is_folder_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}

fn sha1_of(data: &[u8]) -> Sha1Hex {
    Sha1Hex::from_bytes(Sha1::digest(data).into())
}

impl ContentModules {
    pub fn new(view: Arc<dyn RegistryView>) -> Self {
        Self {
            view,
            by_version: RwLock::default(),
        }
    }

    fn entry(&self, version: &GameVersion) -> Option<Arc<Entry>> {
        let table = self.by_version.read().unwrap_or_else(PoisonError::into_inner);
        table.get(version).cloned()
    }

    /// The version a request's `version=`, `game=` or path segment names: a loaded module's version or
    /// alias, else a built-in one.
    pub fn resolve(&self, name: &str) -> Option<GameVersion> {
        {
            let table = self.by_version.read().unwrap_or_else(PoisonError::into_inner);
            if let Some((version, _)) = table.iter().find(|(_, entry)| entry.manifest.answers_invite(name)) {
                return Some(version.clone());
            }
        }
        BUILT_IN
            .iter()
            .find(|(version, alias)| *version == name || *alias == name)
            .and_then(|(version, _)| version.parse().ok())
    }

    /// `edition=`'s value: an edition name only.
    pub fn resolve_edition(&self, name: &str) -> Option<GameVersion> {
        EDITIONS
            .iter()
            .find(|(edition, _)| *edition == name)
            .and_then(|(_, version)| version.parse().ok())
    }

    /// The runtime folders a version may have, in the order tried: its version, then its aliases.
    pub fn runtime_folders(&self, version: &GameVersion) -> Vec<String> {
        let text = version.to_string();
        let mut names = vec![text.clone()];
        if let Some(entry) = self.entry(version) {
            names.extend(entry.manifest.invite_aliases.iter().cloned());
        }
        names.extend(
            BUILT_IN
                .iter()
                .filter(|(v, _)| *v == text)
                .map(|(_, alias)| (*alias).to_owned()),
        );
        let mut folders: Vec<String> = Vec::new();
        for name in names {
            if is_folder_name(&name) && !folders.contains(&name) {
                folders.push(name);
            }
        }
        folders
    }

    pub fn versions(&self) -> VersionsBody {
        let table = self.by_version.read().unwrap_or_else(PoisonError::into_inner);
        VersionsBody {
            default_version: DEFAULT_VERSION,
            versions: table
                .iter()
                .map(|(version, entry)| VersionSummary {
                    game_version: version.clone(),
                    module: entry.manifest.id.clone(),
                    display_name: entry.manifest.display_name.clone(),
                    invite_aliases: entry.manifest.invite_aliases.clone(),
                    unverified: entry.manifest.unverified,
                    client: entry.manifest.client.is_some(),
                    epic_manifest: entry.epic_file.is_some(),
                })
                .collect(),
        }
    }

    /// The loaded module's client profile for `version`: None for no module, Some(None) for a module
    /// without one.
    pub fn client_profile(&self, version: &GameVersion) -> Option<Option<ClientProfile>> {
        self.entry(version).map(|entry| entry.manifest.client.clone())
    }

    /// The pinned Epic manifest's bytes and SHA-1, read and checked now unless the file is unchanged since
    /// the last check. It reads and hashes the file: call it off the async threads.
    pub fn epic_manifest(&self, version: &GameVersion) -> Result<(Bytes, Sha1Hex), EpicManifestError> {
        let entry = self.entry(version).ok_or(EpicManifestError::UnknownVersion)?;
        let pinned = entry
            .manifest
            .client
            .as_ref()
            .and_then(|client| client.epic.as_ref())
            .map(|epic| epic.manifest_sha1);
        let (Some(file), Some(pinned)) = (&entry.epic_file, pinned) else {
            return Err(EpicManifestError::NotServed);
        };
        let dir = self
            .view
            .module_dir(&entry.manifest.id)
            .ok_or_else(|| EpicManifestError::Unavailable(format!("{} has no folder", entry.manifest.id)))?;
        let full = contained(&dir, &file.source).map_err(EpicManifestError::Unavailable)?;
        let meta = std::fs::symlink_metadata(&full)
            .map_err(|_| EpicManifestError::Unavailable(format!("{} is missing", file.source)))?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_EPIC_MANIFEST_BYTES {
            return Err(EpicManifestError::Unavailable(format!(
                "{} is not a regular file of 1 byte to 64 MiB",
                file.source
            )));
        }
        let modified = meta.modified().ok();
        let mut cache = entry.epic_cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cached) = cache.as_ref().filter(|c| c.len == meta.len() && c.modified == modified) {
            return Ok((cached.bytes.clone(), pinned));
        }
        *cache = None;
        let mut data = Vec::new();
        std::fs::File::open(&full)
            .and_then(|f| f.take(MAX_EPIC_MANIFEST_BYTES + 1).read_to_end(&mut data))
            .map_err(|_| EpicManifestError::Unavailable(format!("{} could not be read", file.source)))?;
        if data.len() as u64 != meta.len() {
            return Err(EpicManifestError::Unavailable(format!(
                "{} changed while it was read",
                file.source
            )));
        }
        let found = sha1_of(&data);
        if found != pinned {
            return Err(EpicManifestError::Mismatch {
                expected: pinned,
                found,
            });
        }
        let bytes = Bytes::from(data);
        *cache = Some(Cached {
            len: meta.len(),
            modified,
            bytes: bytes.clone(),
        });
        Ok((bytes, pinned))
    }
}

/// `dir` joined with a registered relative path, refused when a link leads it out of `dir`.
fn contained(dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = dir
        .canonicalize()
        .map_err(|_| format!("the module folder {} is missing", dir.display()))?;
    let full = root.join(relative);
    let real = full.canonicalize().map_err(|_| format!("{relative} is missing"))?;
    if real.starts_with(&root) {
        Ok(real)
    } else {
        Err(format!("{relative} leads out of the module folder"))
    }
}

impl CorePlugin for ContentModules {
    fn name(&self) -> &'static str {
        "content"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::ContentFiles]
    }

    /// Checked again here whatever the controller checked: an external module's manifest is untrusted.
    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        module.validate().map_err(|e| invalid(e.to_string()))?;
        registration.check(module).map_err(|e| invalid(e.to_string()))?;
        // A plugin's files (a theme's logo, DD25) wait for RZ3; it extends a version, it doesn't serve one
        if module.kind == ModuleKind::Plugin {
            return Ok(());
        }
        let epic_file = registration
            .content
            .iter()
            .find(|file| file.path == EPIC_MANIFEST_PATH)
            .cloned();
        let pinned = module.client.as_ref().is_some_and(|client| client.epic.is_some());
        if epic_file.is_some() && !pinned {
            return Err(invalid(format!(
                "{EPIC_MANIFEST_PATH} is registered, but the client profile pins no Epic build to check it against"
            )));
        }
        let mut table = self.by_version.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(other) = table.get(&module.game_version)
            && other.manifest.id != module.id
        {
            return Err(invalid(format!(
                "content already serves {} for {}",
                module.game_version, other.manifest.id
            )));
        }
        linelog::info(
            "module content attached",
            json!({
                "module": module.id.as_str(),
                "version": module.game_version.to_string(),
                "client": module.client.is_some(),
                "epicManifest": epic_file.is_some(),
            }),
        );
        table.insert(
            module.game_version.clone(),
            Arc::new(Entry {
                manifest: module.clone(),
                epic_file,
                epic_cache: Mutex::new(None),
            }),
        );
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        let mut table = self.by_version.write().unwrap_or_else(PoisonError::into_inner);
        let before = table.len();
        table.retain(|_, entry| entry.manifest.id != *module);
        if table.len() != before {
            linelog::info("module content detached", json!({"module": module.as_str()}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use module_api::stub::StubModule;
    use module_api::{GameModule, Registrar};

    struct Dirs(BTreeMap<String, PathBuf>);

    impl RegistryView for Dirs {
        fn module_dir(&self, module: &ModuleId) -> Option<PathBuf> {
            self.0.get(module.as_str()).cloned()
        }
    }

    /// The stub as a 1.12.0 module pinning `manifest`'s SHA-1, its manifest registered or not.
    fn module(manifest: &[u8], register: bool) -> (ModuleManifest, Registration) {
        let mut module = StubModule::new().manifest();
        module.game_version = "1.12.0".parse().unwrap();
        module.invite_aliases = vec![String::from("isles-112")];
        if let Some(epic) = module.client.as_mut().and_then(|c| c.epic.as_mut()) {
            epic.manifest_sha1 = sha1_of(manifest);
        }
        let mut registrar = Registrar::new(module.clone());
        if register {
            registrar.content_file(
                EPIC_MANIFEST_PATH,
                "manifests/1.12.0.manifest",
                "application/octet-stream",
            );
        }
        (module, registrar.finish().unwrap())
    }

    fn with_dir(dir: &Path) -> ContentModules {
        ContentModules::new(Arc::new(Dirs(BTreeMap::from([(
            String::from("stub"),
            dir.to_path_buf(),
        )]))))
    }

    #[test]
    fn versions_aliases_and_editions_resolve_and_nothing_else_does() {
        let modules = ContentModules::default();
        let v = |text: &str| Some(text.parse::<GameVersion>().unwrap());
        assert_eq!(modules.resolve("1.14.7"), v("1.14.7"));
        assert_eq!(modules.resolve("reforged"), v("1.14.7"));
        assert_eq!(modules.resolve("revived"), v("1.4.4"));
        for unknown in ["1.12.0", "isles-112", "reborn", "1.14", "nonsense", ""] {
            assert_eq!(modules.resolve(unknown), None, "{unknown}");
        }
        assert_eq!(modules.resolve_edition("reborn"), v("1.14.7"));
        assert_eq!(modules.resolve_edition("legacy"), v("1.4.4"));
        assert_eq!(modules.resolve_edition("reforged"), None);

        let (manifest, registration) = module(b"m", false);
        modules.attach(&manifest, &registration).unwrap();
        assert_eq!(modules.resolve("1.12.0"), v("1.12.0"));
        assert_eq!(modules.resolve("isles-112"), v("1.12.0"));
        assert_eq!(
            modules.runtime_folders(&"1.12.0".parse().unwrap()),
            ["1.12.0", "isles-112"]
        );
        assert_eq!(
            modules.runtime_folders(&"1.14.7".parse().unwrap()),
            ["1.14.7", "reforged"]
        );
        modules.detach(&manifest.id);
        assert_eq!(modules.resolve("isles-112"), None);
    }

    #[test]
    fn a_second_module_for_a_version_is_refused_and_the_same_one_replaced() {
        let modules = ContentModules::default();
        let (manifest, registration) = module(b"m", false);
        modules.attach(&manifest, &registration).unwrap();
        modules.attach(&manifest, &registration).unwrap();
        let mut other = manifest.clone();
        other.id = "isles-1-12".parse().unwrap();
        assert!(modules.attach(&other, &registration).is_err());
        assert_eq!(modules.versions().versions.len(), 1);
    }

    #[test]
    fn a_registration_that_does_not_check_out_is_refused() {
        let modules = ContentModules::default();
        let (mut manifest, registration) = module(b"m", true);
        // Registered with nothing to pin it
        manifest.client = None;
        assert!(modules.attach(&manifest, &registration).is_err());
        let (manifest, mut registration) = module(b"m", true);
        registration.content[0].source = String::from("../outside");
        assert!(modules.attach(&manifest, &registration).is_err());
        assert!(modules.versions().versions.is_empty());
    }

    #[test]
    fn the_epic_manifest_is_served_only_while_it_matches_its_pin() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"epic manifest bytes";
        std::fs::create_dir_all(dir.path().join("manifests")).unwrap();
        let file = dir.path().join("manifests/1.12.0.manifest");
        std::fs::write(&file, data).unwrap();
        let modules = with_dir(dir.path());
        let version: GameVersion = "1.12.0".parse().unwrap();
        assert_eq!(modules.epic_manifest(&version), Err(EpicManifestError::UnknownVersion));
        let (manifest, registration) = module(data, true);
        modules.attach(&manifest, &registration).unwrap();
        let (bytes, sha1) = modules.epic_manifest(&version).unwrap();
        assert_eq!((&bytes[..], sha1), (&data[..], sha1_of(data)));

        std::fs::write(&file, b"a different manifest").unwrap();
        assert!(matches!(
            modules.epic_manifest(&version),
            Err(EpicManifestError::Mismatch { .. })
        ));
        std::fs::remove_file(&file).unwrap();
        assert!(matches!(
            modules.epic_manifest(&version),
            Err(EpicManifestError::Unavailable(_))
        ));

        // Pinned but not registered: the launcher goes to the profile's sources
        let (manifest, registration) = module(data, false);
        modules.attach(&manifest, &registration).unwrap();
        assert_eq!(modules.epic_manifest(&version), Err(EpicManifestError::NotServed));
        // No folder known for the module
        let (manifest, registration) = module(data, true);
        let bare = ContentModules::default();
        bare.attach(&manifest, &registration).unwrap();
        assert!(matches!(
            bare.epic_manifest(&version),
            Err(EpicManifestError::Unavailable(_))
        ));
    }

    #[test]
    fn the_folder_names_are_never_paths() {
        for good in ["1.14.7", "reforged", "isles-112"] {
            assert!(is_folder_name(good), "{good}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", "C:", "Revived"] {
            assert!(!is_folder_name(bad), "{bad}");
        }
    }
}
