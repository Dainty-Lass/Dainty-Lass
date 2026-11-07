// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Static port assignments, pinned game build constants, repository root discovery, and
//! canonical filesystem paths relative to the server root directory.

use std::path::{Path, PathBuf};

use crate::error::HostError;

/// The game runtime's DLLs `publish-runtime` publishes: the runtime and its loader.
pub const RUNTIME_FILE_NAMES: [&str; 2] = ["Rustic-Isles.dll", "winmm.dll"];
/// The pinned Dauntless 1.14.7 build's changelist.
pub const GAME_CHANGELIST: u32 = 647472;
/// The pinned build's version string, as the game reports it.
pub const GAME_VERSION_TEXT: &str = "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472";
/// The SHA-256 of the pinned build's `Dauntless-Win64-Shipping.exe`, lowercase hex.
pub const PINNED_EXE_SHA256: &str = "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b";
/// The most hunts a server runs at once.
pub const MAX_HUNTS: u32 = 10;
/// The public port a 1.4.4 edition's gateway listened on.
pub const DEFAULT_LEGACY_PUBLIC_PORT: u16 = 4443;

/// The server's fixed ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ports {
    /// The backend's TCP port.
    pub backend: u16,
    /// The Director's TCP port.
    pub director: u16,
    /// The content server's TCP port.
    pub content: u16,
    /// The admin service's TCP port.
    pub admin: u16,
    /// The allowlist's TCP port.
    pub allowlist: u16,
    /// A public server's gateway port, unless host.toml sets another.
    pub public_gateway: u16,
    /// A private server's gateway port.
    pub private_gateway: u16,
    /// The relay's TCP port, which serves `/QoS`.
    pub relay: u16,
    /// MongoDB's TCP port.
    pub mongo: u16,
    /// The first UDP game port, unless more hunts need the range to start lower.
    pub game_first: u16,
    /// The last UDP game port.
    pub game_last: u16,
    /// The lowest UDP game port any hunt count gives.
    pub game_lowest: u16,
}

/// The ports the server uses.
pub const PORTS: Ports = Ports {
    backend: 61000,
    director: 61001,
    content: 61002,
    admin: 61003,
    allowlist: 61005,
    public_gateway: 443,
    private_gateway: 61000,
    relay: 61100,
    mongo: 27017,
    game_first: 8770,
    game_last: 8777,
    game_lowest: 8766,
};

/// The ports a 1.4.4 edition used beside 1.14.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyPorts {
    /// Its metagame's TCP port.
    pub metagame: u16,
    /// Its deploy service's TCP port.
    pub deploy: u16,
    /// Its content server's TCP port.
    pub content: u16,
    /// Its allowlist's TCP port.
    pub allowlist: u16,
    /// Its private gateway's TCP port.
    pub private_gateway: u16,
    /// Its chat's TCP port.
    pub chat: u16,
    /// Its first UDP game port.
    pub game_first: u16,
    /// Its last UDP game port.
    pub game_last: u16,
}

/// The ports a 1.4.4 edition used when both ran.
pub const BOTH_LEGACY_PORTS: LegacyPorts = LegacyPorts {
    metagame: 61010,
    deploy: 61011,
    content: 61012,
    allowlist: 61015,
    private_gateway: 61010,
    chat: 61099,
    game_first: 8780,
    game_last: 8787,
};

/// The game executable's path inside a Dauntless folder.
pub fn game_exe_relative() -> PathBuf {
    Path::new("Archon")
        .join("Binaries")
        .join("Win64")
        .join("Dauntless-Win64-Shipping.exe")
}

/// The first and last UDP port the game servers use for `max_hunts` (clamped to 1 to [`MAX_HUNTS`]).
pub fn game_port_range(max_hunts: u32) -> (u16, u16) {
    let hunts = max_hunts.clamp(1, MAX_HUNTS) as u16;
    let first = PORTS.game_first.min(PORTS.game_last - 1 - hunts);
    (first, PORTS.game_last)
}

fn is_repo_root(dir: &Path) -> bool {
    dir.join("services").join("host").join("Cargo.toml").exists()
        || (dir.join("Cargo.toml").exists()
            && (dir.join("modules").join("rustic-isles").exists()
                || dir.join("restricted").join("modules").join("rustic-isles").exists()))
        || dir.join("release.json").exists()
}

/// The first folder from `start` upward (at most 8 levels) that is a checkout or a release; an error when none is.
pub fn find_repo_root(start: &Path) -> Result<PathBuf, HostError> {
    let resolved = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut current: Option<&Path> = Some(resolved.as_path());
    for _ in 0..=8 {
        let Some(dir) = current else {
            break;
        };
        if is_repo_root(dir) {
            return Ok(dir.to_path_buf());
        }
        current = dir.parent();
    }
    Err(HostError::message(format!(
        "could not find the project root from {}",
        start.display()
    )))
}

/// The server folder found from the current folder, else from the executable's.
pub fn locate_repo_root() -> Result<PathBuf, HostError> {
    let cwd = std::env::current_dir().map_err(|e| HostError::io(".", e))?;
    if let Ok(root) = find_repo_root(&cwd) {
        return Ok(root);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
        && let Ok(root) = find_repo_root(parent)
    {
        return Ok(root);
    }
    find_repo_root(&cwd)
}

/// Where everything the host kit reads and writes is, from the server folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The server folder: a checkout or an unpacked release.
    pub repo: PathBuf,
    /// `preset/`: the game-data preset a checkout ships.
    pub preset: PathBuf,
    /// `services/host`.
    pub host_kit: PathBuf,
    /// `services/gateway`.
    pub gateway_dir: PathBuf,
    /// `services/content`.
    pub content_dir: PathBuf,
    /// `.workspace/`: everything the server makes and keeps.
    pub workspace: PathBuf,
    /// Rustic Isles' 1.14.7 reference tree.
    pub paradox: PathBuf,
    /// `modules/rustic-isles`, in this checkout or its `restricted/`.
    pub isles: PathBuf,
    /// The reference backend, `ParadoxBackend`.
    pub backend_dir: PathBuf,
    /// The reference Director, `ParadoxDirector`.
    pub director_dir: PathBuf,
    /// The game runtime's sources.
    pub runtime_source: PathBuf,
    /// The generated SDK header, `SDK.hpp`.
    pub sdk_header: PathBuf,
    /// The generated SDK folder.
    pub sdk_dir: PathBuf,
    /// The built runtime DLL.
    pub runtime_dll: PathBuf,
    /// The runtime loader's sources.
    pub loader_dir: PathBuf,
    /// The built loader, `winmm.dll`.
    pub loader_dll: PathBuf,
    /// CatalogExporter's sources.
    pub exporter_dir: PathBuf,
    /// The built CatalogExporter DLL.
    pub exporter_dll: PathBuf,
    /// The backend's game data.
    pub backend_game_data: PathBuf,
    /// The Director's game data.
    pub director_game_data: PathBuf,
    /// The backend's raw game data, the generators' input.
    pub raw_game_data: PathBuf,
    /// The game exports, `Items_Analysis`.
    pub exports: PathBuf,
    /// Where the pinned store captures are fetched to.
    pub store_captures: PathBuf,
    /// The store-capture pin, `store-captures.json`.
    pub captures_pin: PathBuf,
    /// `.workspace/runtime`: the published runtime.
    pub runtime_publish: PathBuf,
    /// `.workspace/config`: host.toml, secrets.env and the env files.
    pub config: PathBuf,
    /// `.workspace/config/host.toml`.
    pub host_toml: PathBuf,
    /// `.workspace/tls`: the gateway's and the node's certificates.
    pub tls: PathBuf,
    /// The gateway's certificate, which every invite's fingerprint depends on.
    pub cert_file: PathBuf,
    /// The gateway's private key.
    pub key_file: PathBuf,
    /// `.workspace/mongodb`: a managed mongod's data.
    pub mongo_data: PathBuf,
    /// `.workspace/logs`.
    pub logs: PathBuf,
    /// The game servers' logs.
    pub gameserver_logs: PathBuf,
    /// `.workspace/branding`.
    pub branding: PathBuf,
    /// `.workspace/config/news.json`.
    pub news_file: PathBuf,
    /// `.workspace/bin`.
    pub bin: PathBuf,
    /// The Linux game server wrapper, `reforged-gameserver`.
    pub gameserver_wrapper: PathBuf,
    /// `.workspace/wine`: the Wine prefix.
    pub wine_prefix: PathBuf,
    /// `.workspace/proton`: Proton's compat data.
    pub proton_data: PathBuf,
    /// `.workspace/systemd`: the generated units and their install script.
    pub systemd: PathBuf,
}

impl Paths {
    /// The layout under `repo`; Rustic Isles is looked for in `restricted/` first.
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        let repo = repo.into();
        let restricted = if repo.join("restricted").join("modules").join("rustic-isles").exists() {
            repo.join("restricted")
        } else {
            repo.clone()
        };
        let preset = repo.join("preset");
        let host_kit = repo.join("services").join("host");
        let gateway_dir = repo.join("services").join("gateway");
        let content_dir = repo.join("services").join("content");
        let workspace = repo.join(".workspace");
        let isles = restricted.join("modules").join("rustic-isles");
        let paradox = isles.join("reference").join("1.14.7");
        let backend_dir = paradox.join("ParadoxBackend");
        let director_dir = paradox.join("ParadoxDirector");
        let runtime_source = isles.join("native").join("runtime");
        let sdk_header = runtime_source.join("SDK.hpp");
        let sdk_dir = runtime_source.join("SDK");
        let runtime_dll = runtime_source.join("x64").join("Release").join("Rustic-Isles.dll");
        let loader_dir = restricted.join("native").join("runtime-loader");
        let loader_dll = loader_dir
            .join("target")
            .join("x86_64-pc-windows-msvc")
            .join("release")
            .join("winmm.dll");
        let exporter_dir = isles.join("native").join("catalog-exporter");
        let exporter_dll = exporter_dir.join("x64").join("Release").join("CatalogExporter.dll");
        let backend_game_data = backend_dir.join("game-data");
        let director_game_data = director_dir.join("game-data");
        let raw_game_data = backend_game_data.join("raw");
        let exports = paradox.join("Items_Analysis");
        let store_captures = paradox.join("DauntlessEndpointDocumentation");
        let captures_pin = backend_dir.join("store-captures.json");
        let runtime_publish = workspace.join("runtime");
        let config = workspace.join("config");
        let host_toml = config.join("host.toml");
        let tls = workspace.join("tls");
        let cert_file = tls.join("gateway-cert.pem");
        let key_file = tls.join("gateway-key.pem");
        let mongo_data = workspace.join("mongodb");
        let logs = workspace.join("logs");
        let gameserver_logs = logs.join("gameservers");
        let branding = workspace.join("branding");
        let news_file = config.join("news.json");
        let bin = workspace.join("bin");
        let gameserver_wrapper = bin.join("reforged-gameserver");
        let wine_prefix = workspace.join("wine");
        let proton_data = workspace.join("proton");
        let systemd = workspace.join("systemd");

        Self {
            repo,
            preset,
            host_kit,
            gateway_dir,
            content_dir,
            workspace,
            paradox,
            isles,
            backend_dir,
            director_dir,
            runtime_source,
            sdk_header,
            sdk_dir,
            runtime_dll,
            loader_dir,
            loader_dll,
            exporter_dir,
            exporter_dll,
            backend_game_data,
            director_game_data,
            raw_game_data,
            exports,
            store_captures,
            captures_pin,
            runtime_publish,
            config,
            host_toml,
            tls,
            cert_file,
            key_file,
            mongo_data,
            logs,
            gameserver_logs,
            branding,
            news_file,
            bin,
            gameserver_wrapper,
            wine_prefix,
            proton_data,
            systemd,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn game_port_range_scales_with_max_hunts() {
        assert_eq!(game_port_range(0), (8770, 8777));
        assert_eq!(game_port_range(1), (8770, 8777));
        assert_eq!(game_port_range(6), (8770, 8777));
        assert_eq!(game_port_range(7), (8769, 8777));
        assert_eq!(game_port_range(8), (8768, 8777));
        assert_eq!(game_port_range(9), (8767, 8777));
        assert_eq!(game_port_range(10), (8766, 8777));
        assert_eq!(game_port_range(99), (8766, 8777));
    }

    #[test]
    fn paths_layout_matches_workspace_contract() {
        let p = Paths::new("/srv/server");
        assert_eq!(p.repo, Path::new("/srv/server"));
        assert_eq!(p.workspace, Path::new("/srv/server/.workspace"));
        assert_eq!(p.config, Path::new("/srv/server/.workspace/config"));
        assert_eq!(p.host_toml, Path::new("/srv/server/.workspace/config/host.toml"));
        assert_eq!(p.cert_file, Path::new("/srv/server/.workspace/tls/gateway-cert.pem"));
        assert_eq!(p.key_file, Path::new("/srv/server/.workspace/tls/gateway-key.pem"));
        assert_eq!(p.mongo_data, Path::new("/srv/server/.workspace/mongodb"));
        assert_eq!(
            p.gameserver_wrapper,
            Path::new("/srv/server/.workspace/bin/reforged-gameserver")
        );
        assert_eq!(
            game_exe_relative(),
            Path::new("Archon")
                .join("Binaries")
                .join("Win64")
                .join("Dauntless-Win64-Shipping.exe")
        );
    }

    #[test]
    fn find_repo_root_walks_parents_or_errors() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(find_repo_root(&nested).is_err());

        std::fs::write(dir.path().join("release.json"), "{}").unwrap();
        let found = find_repo_root(&nested).unwrap();
        assert_eq!(found, std::fs::canonicalize(dir.path()).unwrap());
    }
}
