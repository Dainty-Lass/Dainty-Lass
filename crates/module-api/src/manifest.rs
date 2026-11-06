//! [`ModuleManifest`]: what a module says about itself before anything runs (DD4). The controller keys
//! game modules by [`GameVersion`], the UI draws a module's screens from it (DD11) and the launcher's client
//! comes from its [`ClientProfile`] (DD12, DD22).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ids::{ModuleId, Sha1Hex, Sha256Hex, check_name};
use crate::version::{ApiVersion, GameVersion};

/// Bounds on what a manifest may hold. It arrives from an external module's process (DD5), so every list
/// and string has a ceiling.
pub mod limits {
    /// Longest display name, in bytes.
    pub const DISPLAY_NAME: usize = 64;
    /// Most invite aliases.
    pub const ALIASES: usize = 8;
    /// Most default ports.
    pub const PORTS: usize = 32;
    /// The widest port range a module may ask for
    pub const PORT_RANGE: u32 = 1024;
    /// Most client arguments, and most environment variables.
    pub const ARGUMENTS: usize = 64;
    /// Longest argument, environment value or ini text, in bytes.
    pub const TEXT: usize = 1024;
    /// Most ini edits.
    pub const INI_EDITS: usize = 256;
    /// Most Epic manifest sources.
    pub const SOURCES: usize = 8;
    /// Longest manifest source URL, in bytes.
    pub const URL: usize = 2048;
    /// Longest relative path, in bytes (Windows' `MAX_PATH`).
    pub const PATH: usize = 260;
    /// Most entries in a patch set's lists and pinned files.
    pub const LIST: usize = 256;
    /// Most palette colours.
    pub const PALETTE: usize = 64;
}

/// Game module or plugin (DD3, DD25). Both implement `GameModule` and load the same way; a plugin extends
/// the game module of its `game_version` instead of serving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    /// Serves a game version.
    Game,
    /// Extends the game module of its version.
    Plugin,
}

/// A port's protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// An inclusive port range; one port is `first == last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortRange {
    /// First port.
    pub first: u16,
    /// Last port, inclusive.
    pub last: u16,
}

impl PortRange {
    /// The one port `port`.
    pub fn single(port: u16) -> Self {
        Self {
            first: port,
            last: port,
        }
    }

    /// Number of ports in the range.
    pub fn width(self) -> u32 {
        u32::from(self.last).saturating_sub(u32::from(self.first)) + 1
    }

    /// Starts above 0, is not reversed, and is at most [`limits::PORT_RANGE`] wide.
    pub fn is_valid(self) -> bool {
        self.first != 0 && self.first <= self.last && self.width() <= limits::PORT_RANGE
    }

    /// The two ranges share a port.
    pub fn overlaps(self, other: PortRange) -> bool {
        self.first <= other.last && other.first <= self.last
    }
}

/// A port a module listens on by default; the controller moves it on a clash (DD4) and tells the module
/// what it got in `ModuleConfig::ports`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortDefault {
    /// Name the module and `host.toml` use for it.
    pub name: String,
    /// Its protocol.
    pub transport: Transport,
    /// The ports it takes unless moved.
    pub range: PortRange,
}

/// A plugin's set of gameplay patches, or a game module's willingness to take one (DD25). A stub: RZ2 fills
/// what applying one means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PatchSetCapability {
    /// A game module that lets a plugin patch its game servers
    Accepts,
    /// A plugin with this set of patches.
    Provides(PatchSet),
}

/// A plugin's gameplay patches, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSet {
    /// Patch set id, a name.
    pub id: String,
    /// Name shown in the UI.
    pub display_name: String,
    /// Runtime patches the game servers' runtime is built with, by name
    #[serde(default)]
    pub runtime_patches: Vec<String>,
    /// Backend rules switched on, by name
    #[serde(default)]
    pub backend_rules: Vec<String>,
}

/// A plugin's look for the UI (DD25): drawn when the plugin is loaded and the host turned it on. A stub:
/// RZ3 and DU2 fill what the UI reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeCapability {
    /// Theme id, a name.
    pub id: String,
    /// Name shown in the UI.
    pub display_name: String,
    /// Colour tokens by name, as the UI's CSS variables take them
    #[serde(default)]
    pub palette: BTreeMap<String, String>,
    /// A layout the UI offers, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// A file the module registers with content
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

/// What a module does beyond serving its game, which decides what the UI shows for it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// Player-hosted realms (DD14): the UI shows the hosting flow
    #[serde(default)]
    pub realms: bool,
    /// Joins the network (DD16)
    #[serde(default)]
    pub network: bool,
    /// Gives the chat plugin JIDs and room rules (DD21)
    #[serde(default)]
    pub chat: bool,
    /// Patch sets accepted or provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_set: Option<PatchSetCapability>,
    /// The plugin's theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeCapability>,
}

/// Which folder an [`IniEdit`]'s relative `file` is joined to (DD12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IniRoot {
    /// The game's install folder
    #[default]
    Install,
    /// `%LOCALAPPDATA%` (Windows)
    LocalAppData,
}

impl IniRoot {
    /// The file is relative to the install folder.
    pub fn is_install(&self) -> bool {
        matches!(self, Self::Install)
    }
}

/// Another file inside the install folder that the launcher checks by SHA-256 before launch (DD12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedFile {
    /// Relative to the install folder, `/`-separated
    pub path: String,
    /// Its pinned SHA-256.
    pub sha256: Sha256Hex,
}

/// One line the launcher writes into a game ini before launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IniEdit {
    /// The folder `file` is relative to.
    #[serde(default, skip_serializing_if = "IniRoot::is_install")]
    pub root: IniRoot,
    /// The ini file, a relative `/`-separated path.
    pub file: String,
    /// Section, without brackets.
    pub section: String,
    /// Key in the section.
    pub key: String,
    /// Value written.
    pub value: String,
}

/// The Epic build a version installs from (DE4, DD22): its manifest pinned by SHA-1, where to fetch the
/// manifest, and the executable's SHA-256 checked after install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpicBuild {
    /// Epic's build version string for it
    pub version: String,
    /// The manifest's SHA-1, which the fetched manifest must hash to.
    pub manifest_sha1: Sha1Hex,
    /// `https` URLs to try in order, after the joined server's copy (DD22)
    pub manifest_sources: Vec<String>,
    /// The shipping executable's SHA-256 after install.
    pub executable_sha256: Sha256Hex,
}

/// What the player's launcher needs for this game (DD12), served by content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientProfile {
    /// The executable, relative to the install folder, `/`-separated
    pub executable: String,
    /// The executable's SHA-256, checked before launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<Sha256Hex>,
    /// Other files inside the install folder pinned by SHA-256
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PinnedFile>,
    /// Command-line arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Environment variables set for the game.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, String>,
    /// Ini lines written before launch.
    #[serde(default)]
    pub ini_edits: Vec<IniEdit>,
    /// None for a version that can't come from Epic (1.4.4: locate your folder)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic: Option<EpicBuild>,
}

/// What a module says about itself, checked by [`ModuleManifest::validate`] before it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleManifest {
    /// Module id.
    pub id: ModuleId,
    /// Game module or plugin.
    pub kind: ModuleKind,
    /// "Rustic Isles 1.14.7", "Dauntless Revived" (DD4)
    pub display_name: String,
    /// The version served, or for a plugin the version extended
    pub game_version: GameVersion,
    /// Other names an invite's `version` may give this game (`reforged`, `revived`)
    #[serde(default)]
    pub invite_aliases: Vec<String>,
    /// The module API it was built against.
    pub api: ApiVersion,
    /// Ports it listens on by default.
    #[serde(default)]
    pub ports: Vec<PortDefault>,
    /// What it does beyond serving its game.
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Built but not checked against recorded traffic (DD9); the UI says so
    #[serde(default)]
    pub unverified: bool,
    /// What the launcher needs to start the game, for a game module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientProfile>,
}

/// Why a manifest fails [`ModuleManifest::validate`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("manifest: {0}")]
pub struct ManifestError(pub String);

fn fail<T>(message: impl Into<String>) -> Result<T, ManifestError> {
    Err(ManifestError(message.into()))
}

/// Free text a person reads: bounded, one line.
pub(crate) fn is_text(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}

/// A relative `/`-separated path that stays inside the folder it is joined to, on Windows as well: no
/// root, drive, `\`, `:` (alternate streams), `.` or `..` segment.
pub(crate) fn is_relative_path(path: &str) -> bool {
    is_text(path, limits::PATH)
        && !path.contains(['\\', ':'])
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn check_list(list: &[String], what: &str) -> Result<(), ManifestError> {
    if list.len() > limits::LIST || !list.iter().all(|item| is_text(item, 128)) {
        return fail(format!("{what}: at most {} names of 1 to 128 characters", limits::LIST));
    }
    Ok(())
}

impl ModuleManifest {
    /// Whether an invite's `version` names this module's game: the version itself or an alias.
    pub fn answers_invite(&self, version: &str) -> bool {
        self.game_version.to_string() == version || self.invite_aliases.iter().any(|alias| alias == version)
    }

    /// The default port named `name`.
    pub fn port(&self, name: &str) -> Option<&PortDefault> {
        self.ports.iter().find(|port| port.name == name)
    }

    /// Everything serde can't check: bounds, names, paths, and which capabilities fit which kind.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if !is_text(&self.display_name, limits::DISPLAY_NAME) {
            return fail(format!(
                "the display name is 1 to {} characters on one line",
                limits::DISPLAY_NAME
            ));
        }
        if self.invite_aliases.len() > limits::ALIASES {
            return fail(format!("at most {} invite aliases", limits::ALIASES));
        }
        let mut aliases = BTreeSet::new();
        for alias in &self.invite_aliases {
            check_name(alias, "an invite alias").map_err(|e| ManifestError(e.to_string()))?;
            if !aliases.insert(alias) {
                return fail(format!("invite alias {alias} given twice"));
            }
        }
        self.validate_ports()?;
        let game = self.kind == ModuleKind::Game;
        let caps = &self.capabilities;
        if !game && (caps.realms || caps.network || caps.chat || self.client.is_some()) {
            return fail("realms, network, chat and a client profile belong to game modules");
        }
        match &caps.patch_set {
            Some(PatchSetCapability::Accepts) if !game => return fail("a plugin provides a patch set"),
            Some(PatchSetCapability::Provides(set)) => {
                if game {
                    return fail("a game module accepts a patch set; a plugin provides one");
                }
                check_name(&set.id, "a patch set id").map_err(|e| ManifestError(e.to_string()))?;
                if !is_text(&set.display_name, limits::DISPLAY_NAME) {
                    return fail("the patch set's display name is 1 to 64 characters");
                }
                check_list(&set.runtime_patches, "runtime patches")?;
                check_list(&set.backend_rules, "backend rules")?;
            }
            _ => {}
        }
        if let Some(theme) = &caps.theme {
            if game {
                return fail("a theme belongs to a plugin");
            }
            Self::validate_theme(theme)?;
        }
        if let Some(client) = &self.client {
            Self::validate_client(client)?;
        }
        Ok(())
    }

    fn validate_ports(&self) -> Result<(), ManifestError> {
        if self.ports.len() > limits::PORTS {
            return fail(format!("at most {} ports", limits::PORTS));
        }
        for (i, port) in self.ports.iter().enumerate() {
            check_name(&port.name, "a port name").map_err(|e| ManifestError(e.to_string()))?;
            if !port.range.is_valid() {
                return fail(format!(
                    "port {}: a range from 1, first <= last, at most {} wide",
                    port.name,
                    limits::PORT_RANGE
                ));
            }
            for other in &self.ports[..i] {
                if other.name == port.name {
                    return fail(format!("port {} given twice", port.name));
                }
                if other.transport == port.transport && other.range.overlaps(port.range) {
                    return fail(format!("ports {} and {} overlap", other.name, port.name));
                }
            }
        }
        Ok(())
    }

    fn validate_theme(theme: &ThemeCapability) -> Result<(), ManifestError> {
        check_name(&theme.id, "a theme id").map_err(|e| ManifestError(e.to_string()))?;
        if !is_text(&theme.display_name, limits::DISPLAY_NAME) {
            return fail("the theme's display name is 1 to 64 characters");
        }
        if theme.palette.len() > limits::PALETTE {
            return fail(format!("a palette has at most {} colours", limits::PALETTE));
        }
        for (name, value) in &theme.palette {
            check_name(name, "a palette name").map_err(|e| ManifestError(e.to_string()))?;
            if !is_text(value, 64) {
                return fail(format!("palette {name}: 1 to 64 characters"));
            }
        }
        if theme
            .layout
            .as_deref()
            .is_some_and(|layout| check_name(layout, "a layout").is_err())
        {
            return fail("the theme's layout is a name");
        }
        if theme.logo.as_deref().is_some_and(|logo| !is_relative_path(logo)) {
            return fail("the theme's logo is a relative path");
        }
        Ok(())
    }

    fn validate_client(client: &ClientProfile) -> Result<(), ManifestError> {
        if !is_relative_path(&client.executable) {
            return fail("the client executable is a relative path inside the install folder");
        }
        if client.files.len() > limits::LIST || !client.files.iter().all(|f| is_relative_path(&f.path)) {
            return fail("pinned files must be relative paths inside the install folder");
        }
        if client.arguments.len() > limits::ARGUMENTS
            || !client
                .arguments
                .iter()
                .all(|arg| arg.len() <= limits::TEXT && !arg.contains('\0'))
        {
            return fail(format!(
                "at most {} arguments of {} bytes",
                limits::ARGUMENTS,
                limits::TEXT
            ));
        }
        if client.environment.len() > limits::ARGUMENTS
            || !client.environment.iter().all(|(k, v)| {
                is_text(k, 128) && !k.contains('=') && v.len() <= limits::TEXT && !v.chars().any(char::is_control)
            })
        {
            return fail("environment variables: 1 to 128 character names and one-line values");
        }
        if client.ini_edits.len() > limits::INI_EDITS {
            return fail(format!("at most {} ini edits", limits::INI_EDITS));
        }
        for edit in &client.ini_edits {
            let ok = is_relative_path(&edit.file)
                && is_text(&edit.section, limits::TEXT)
                && is_text(&edit.key, limits::TEXT)
                && edit.value.len() <= limits::TEXT
                && !edit.value.chars().any(char::is_control);
            if !ok {
                return fail("an ini edit names a relative file and one-line section, key and value");
            }
        }
        if let Some(epic) = &client.epic {
            if !is_text(&epic.version, 128) {
                return fail("the Epic build version is 1 to 128 characters");
            }
            let sources_ok = !epic.manifest_sources.is_empty()
                && epic.manifest_sources.len() <= limits::SOURCES
                && epic.manifest_sources.iter().all(|url| {
                    is_text(url, limits::URL)
                        && url.starts_with("https://")
                        && url.len() > "https://".len()
                        && !url.contains(char::is_whitespace)
                });
            if !sources_ok {
                return fail(format!("1 to {} https manifest sources", limits::SOURCES));
            }
        }
        Ok(())
    }
}

/// Why a plugin can't extend a game module.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FitError {
    /// The `plugin` manifest is not a plugin.
    #[error("{0} is not a plugin")]
    NotAPlugin(ModuleId),
    /// The `game` manifest is not a game module.
    #[error("{0} is not a game module")]
    NotAGame(ModuleId),
    /// The plugin is for another game version.
    #[error("the plugin extends {plugin}, the module serves {game}")]
    OtherVersion {
        /// The version the plugin extends.
        plugin: GameVersion,
        /// The version the game module serves.
        game: GameVersion,
    },
    /// The plugin provides a patch set the game module does not accept.
    #[error("{0} takes no patch set")]
    NoPatchSets(ModuleId),
}

/// Whether `plugin` can extend `game` (DD25): the same game version, and a patch set only where the game
/// module accepts one. A theme needs nothing from the game module.
pub fn plugin_fits(game: &ModuleManifest, plugin: &ModuleManifest) -> Result<(), FitError> {
    if plugin.kind != ModuleKind::Plugin {
        return Err(FitError::NotAPlugin(plugin.id.clone()));
    }
    if game.kind != ModuleKind::Game {
        return Err(FitError::NotAGame(game.id.clone()));
    }
    if plugin.game_version != game.game_version {
        return Err(FitError::OtherVersion {
            plugin: plugin.game_version.clone(),
            game: game.game_version.clone(),
        });
    }
    let provides = matches!(plugin.capabilities.patch_set, Some(PatchSetCapability::Provides(_)));
    let accepts = matches!(game.capabilities.patch_set, Some(PatchSetCapability::Accepts));
    if provides && !accepts {
        return Err(FitError::NoPatchSets(game.id.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::API_VERSION;

    type Case<T> = (&'static str, Box<dyn Fn(&mut T)>);

    fn game() -> ModuleManifest {
        ModuleManifest {
            id: "isles-1-14-7".parse().unwrap(),
            kind: ModuleKind::Game,
            display_name: String::from("Rustic Isles 1.14.7"),
            game_version: "1.14.7".parse().unwrap(),
            invite_aliases: vec![String::from("reforged")],
            api: API_VERSION,
            ports: vec![
                PortDefault {
                    name: String::from("backend"),
                    transport: Transport::Tcp,
                    range: PortRange::single(61000),
                },
                PortDefault {
                    name: String::from("game"),
                    transport: Transport::Udp,
                    range: PortRange {
                        first: 7777,
                        last: 7800,
                    },
                },
            ],
            capabilities: Capabilities {
                realms: true,
                network: true,
                chat: true,
                patch_set: Some(PatchSetCapability::Accepts),
                theme: None,
            },
            unverified: false,
            client: Some(ClientProfile {
                executable: String::from("Dauntless/Binaries/Win64/Dauntless-Win64-Shipping.exe"),
                executable_sha256: None,
                files: vec![],
                arguments: vec![String::from("-log")],
                environment: BTreeMap::new(),
                ini_edits: vec![],
                epic: Some(EpicBuild {
                    version: String::from("1.14.7"),
                    manifest_sha1: "aa70748e0000000000000000000000000031476f".parse().unwrap(),
                    manifest_sources: vec![String::from("https://example.invalid/1.14.7.manifest")],
                    executable_sha256: "ab".repeat(32).parse().unwrap(),
                }),
            }),
        }
    }

    fn plugin() -> ModuleManifest {
        ModuleManifest {
            id: "rustic-freeza".parse().unwrap(),
            kind: ModuleKind::Plugin,
            display_name: String::from("Rustic Freeza"),
            game_version: "1.14.7".parse().unwrap(),
            invite_aliases: vec![],
            api: API_VERSION,
            ports: vec![],
            capabilities: Capabilities {
                patch_set: Some(PatchSetCapability::Provides(PatchSet {
                    id: String::from("freeza"),
                    display_name: String::from("Freeza's tuning"),
                    runtime_patches: vec![String::from("0004")],
                    backend_rules: vec![],
                })),
                theme: Some(ThemeCapability {
                    id: String::from("freeza"),
                    display_name: String::from("Freeza"),
                    palette: BTreeMap::from([(String::from("accent"), String::from("#ff7a00"))]),
                    layout: None,
                    logo: Some(String::from("theme/logo.png")),
                }),
                ..Capabilities::default()
            },
            unverified: false,
            client: None,
        }
    }

    #[test]
    fn a_full_manifest_validates_and_round_trips() {
        for manifest in [game(), plugin()] {
            manifest.validate().unwrap();
            let json = serde_json::to_string(&manifest).unwrap();
            assert_eq!(serde_json::from_str::<ModuleManifest>(&json).unwrap(), manifest);
        }
        let game = game();
        assert!(game.answers_invite("1.14.7") && game.answers_invite("reforged"));
        assert!(!game.answers_invite("1.14") && !game.answers_invite("revived"));
        assert_eq!(game.port("game").unwrap().transport, Transport::Udp);
    }

    #[test]
    fn unknown_fields_and_bad_values_are_refused() {
        let mut json = serde_json::to_value(game()).unwrap();
        json["surprise"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ModuleManifest>(json).is_err());
        let mut json = serde_json::to_value(game()).unwrap();
        json["game_version"] = serde_json::json!("1.14.x");
        assert!(serde_json::from_value::<ModuleManifest>(json).is_err());
    }

    #[test]
    fn validation_catches_what_serde_cannot() {
        let cases: Vec<Case<ModuleManifest>> = vec![
            ("empty name", Box::new(|m| m.display_name.clear())),
            ("name with newline", Box::new(|m| m.display_name.push('\n'))),
            (
                "bad alias",
                Box::new(|m| m.invite_aliases.push(String::from("Reforged"))),
            ),
            (
                "twice alias",
                Box::new(|m| m.invite_aliases.push(String::from("reforged"))),
            ),
            ("port zero", Box::new(|m| m.ports[0].range = PortRange::single(0))),
            (
                "backwards",
                Box::new(|m| m.ports[1].range = PortRange { first: 9, last: 8 }),
            ),
            (
                "too wide",
                Box::new(|m| {
                    m.ports[1].range = PortRange {
                        first: 1000,
                        last: 3000,
                    }
                }),
            ),
            ("overlap", Box::new(|m| m.ports.push(m.ports[1].clone()))),
            (
                "escape",
                Box::new(|m| m.client.as_mut().unwrap().executable = String::from("../x.exe")),
            ),
            (
                "absolute",
                Box::new(|m| m.client.as_mut().unwrap().executable = String::from("/x.exe")),
            ),
            (
                "drive",
                Box::new(|m| m.client.as_mut().unwrap().executable = String::from("C:x.exe")),
            ),
            (
                "backslash",
                Box::new(|m| m.client.as_mut().unwrap().executable = String::from("a\\b.exe")),
            ),
            (
                "http",
                Box::new(|m| {
                    m.client.as_mut().unwrap().epic.as_mut().unwrap().manifest_sources[0] = String::from("http://x")
                }),
            ),
            (
                "no source",
                Box::new(|m| {
                    m.client
                        .as_mut()
                        .unwrap()
                        .epic
                        .as_mut()
                        .unwrap()
                        .manifest_sources
                        .clear()
                }),
            ),
            (
                "game theme",
                Box::new(|m| m.capabilities.theme = plugin().capabilities.theme),
            ),
            (
                "game provides",
                Box::new(|m| m.capabilities.patch_set = plugin().capabilities.patch_set),
            ),
        ];
        for (what, change) in cases {
            let mut manifest = game();
            change(&mut manifest);
            assert!(manifest.validate().is_err(), "{what}");
        }
        let mut manifest = plugin();
        manifest.capabilities.realms = true;
        assert!(manifest.validate().is_err());
        let mut manifest = plugin();
        manifest.capabilities.patch_set = Some(PatchSetCapability::Accepts);
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn a_plugin_fits_only_its_version_and_a_module_that_accepts_patches() {
        assert_eq!(plugin_fits(&game(), &plugin()), Ok(()));
        assert!(matches!(plugin_fits(&plugin(), &game()), Err(FitError::NotAPlugin(_))));
        let mut other = game();
        other.game_version = "1.12.0".parse().unwrap();
        assert!(matches!(
            plugin_fits(&other, &plugin()),
            Err(FitError::OtherVersion { .. })
        ));
        let mut closed = game();
        closed.capabilities.patch_set = None;
        assert!(matches!(plugin_fits(&closed, &plugin()), Err(FitError::NoPatchSets(_))));
        let mut theme_only = plugin();
        theme_only.capabilities.patch_set = None;
        assert_eq!(plugin_fits(&closed, &theme_only), Ok(()));
    }
}
