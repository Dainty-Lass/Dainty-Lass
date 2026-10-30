// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Static port assignments, pinned game build constants, repository root discovery, and
//! canonical filesystem paths relative to the server root directory.

use std::path::{Path, PathBuf};

use crate::error::HostError;

pub const RUNTIME_FILE_NAMES: [&str; 2] = ["MysticParadox.dll", "winmm.dll"];
pub const GAME_CHANGELIST: u32 = 647472;
pub const GAME_VERSION_TEXT: &str = "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472";
pub const PINNED_EXE_SHA256: &str = "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b";
pub const MAX_HUNTS: u32 = 10;
pub const DEFAULT_LEGACY_PUBLIC_PORT: u16 = 4443;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ports {
    pub backend: u16,
    pub director: u16,
    pub content: u16,
    pub admin: u16,
    pub allowlist: u16,
    pub public_gateway: u16,
    pub private_gateway: u16,
    pub relay: u16,
    pub mongo: u16,
    pub game_first: u16,
    pub game_last: u16,
    pub game_lowest: u16,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyPorts {
    pub metagame: u16,
    pub deploy: u16,
    pub content: u16,
    pub allowlist: u16,
    pub private_gateway: u16,
    pub chat: u16,
    pub game_first: u16,
    pub game_last: u16,
}

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

pub fn game_exe_relative() -> PathBuf {
    Path::new("Archon")
        .join("Binaries")
        .join("Win64")
        .join("Dauntless-Win64-Shipping.exe")
}

pub fn game_port_range(max_hunts: u32) -> (u16, u16) {
    let hunts = max_hunts.clamp(1, MAX_HUNTS) as u16;
    let first = PORTS.game_first.min(PORTS.game_last - 1 - hunts);
    (first, PORTS.game_last)
}

fn is_repo_root(dir: &Path) -> bool {
    dir.join("rust")
        .join("services")
        .join("host")
        .join("Cargo.toml")
        .exists()
        || dir.join("release.json").exists()
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub repo: PathBuf,
    pub preset: PathBuf,
    pub host_kit: PathBuf,
    pub gateway_dir: PathBuf,
    pub content_dir: PathBuf,
    pub workspace: PathBuf,
    pub paradox: PathBuf,
    pub isles: PathBuf,
    pub revived: PathBuf,
    pub old_upstream: PathBuf,
    pub backend_dir: PathBuf,
    pub director_dir: PathBuf,
    pub runtime_source: PathBuf,
    pub sdk_header: PathBuf,
    pub sdk_dir: PathBuf,
    pub runtime_dll: PathBuf,
    pub loader_dir: PathBuf,
    pub loader_dll: PathBuf,
    pub exporter_dir: PathBuf,
    pub exporter_dll: PathBuf,
    pub backend_game_data: PathBuf,
    pub director_game_data: PathBuf,
    pub raw_game_data: PathBuf,
    pub exports: PathBuf,
    pub store_captures: PathBuf,
    pub captures_pin: PathBuf,
    pub runtime_publish: PathBuf,
    pub config: PathBuf,
    pub host_toml: PathBuf,
    pub host_json: PathBuf,
    pub tls: PathBuf,
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
    pub mongo_data: PathBuf,
    pub logs: PathBuf,
    pub gameserver_logs: PathBuf,
    pub branding: PathBuf,
    pub news_file: PathBuf,
    pub bin: PathBuf,
    pub gameserver_wrapper: PathBuf,
    pub wine_prefix: PathBuf,
    pub proton_data: PathBuf,
    pub systemd: PathBuf,
}

impl Paths {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        let repo = repo.into();
        let preset = repo.join("preset");
        let host_kit = repo.join("rust").join("services").join("host");
        let gateway_dir = repo.join("services").join("gateway");
        let content_dir = repo.join("services").join("content");
        let workspace = repo.join(".workspace");
        let isles = repo.join("modules").join("rustic-isles");
        let paradox = isles.join("reference").join("1.14.7");
        let revived = repo.join("modules").join("legacy-revived");
        let old_upstream = workspace.join("Mystic-Paradox");
        let backend_dir = paradox.join("ParadoxBackend");
        let director_dir = paradox.join("ParadoxDirector");
        let runtime_source = isles.join("native").join("runtime");
        let sdk_header = runtime_source.join("SDK.hpp");
        let sdk_dir = runtime_source.join("SDK");
        let runtime_dll = runtime_source.join("x64").join("Release").join("MysticParadox.dll");
        let loader_dir = repo.join("native").join("runtime-loader");
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
        let host_json = config.join("host.json");
        let tls = workspace.join("tls");
        let cert_file = tls.join("gateway-cert.pem");
        let key_file = tls.join("gateway-key.pem");
        let mongo_data = workspace.join("data").join("mongo");
        let logs = workspace.join("logs");
        let gameserver_logs = logs.join("gameservers");
        let branding = workspace.join("branding");
        let news_file = branding.join("news.json");
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
            revived,
            old_upstream,
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
            host_json,
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]

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
        assert_eq!(p.host_json, Path::new("/srv/server/.workspace/config/host.json"));
        assert_eq!(p.cert_file, Path::new("/srv/server/.workspace/tls/gateway-cert.pem"));
        assert_eq!(p.key_file, Path::new("/srv/server/.workspace/tls/gateway-key.pem"));
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
