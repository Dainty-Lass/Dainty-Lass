//! Unified multi-format (`TOML`, `YAML`, `JSON`) server configuration (`docs/app-structure-plan.md`).

use std::path::{Path, PathBuf};

use protocol::edition::{Edition, EditionSelection};
use serde::{Deserialize, Serialize};

use crate::yaml::parse_yaml_to_json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Toml,
    Yaml,
    Json,
}

impl ConfigFormat {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "toml" => Some(Self::Toml),
            "yaml" | "yml" => Some(Self::Yaml),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Per-edition (`Refined` or `Reforged`) stack configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EditionConfig {
    pub enabled: bool,
    pub backend_port: u16,
    pub director_port: u16,
    pub udp_port_begin: u16,
    pub udp_port_end: u16,
    pub max_hunt_servers: u16,
    pub server_root_dir: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
}

impl EditionConfig {
    pub fn default_for(edition: Edition, enabled: bool) -> Self {
        let (udp_port_begin, udp_port_end) = edition.default_udp_port_range();
        Self {
            enabled,
            backend_port: edition.default_backend_port(),
            director_port: edition.default_director_port(),
            udp_port_begin,
            udp_port_end,
            max_hunt_servers: 4,
            server_root_dir: None,
            runtime_dir: None,
        }
    }

    pub fn udp_ports(&self) -> Vec<u16> {
        if self.udp_port_end < self.udp_port_begin {
            return Vec::new();
        }
        (self.udp_port_begin..=self.udp_port_end).collect()
    }
}

impl Default for EditionConfig {
    fn default() -> Self {
        Self::default_for(Edition::Reforged, true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EditionsMap {
    pub reforged: EditionConfig,
    pub refined: EditionConfig,
}

impl Default for EditionsMap {
    fn default() -> Self {
        Self {
            reforged: EditionConfig::default_for(Edition::Reforged, true),
            refined: EditionConfig::default_for(Edition::Refined, true),
        }
    }
}

/// Top-level server configuration deserializable from `.toml`, `.yaml`/`.yml`, or `.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerConfig {
    pub server_name: String,
    pub server_mode: String,
    pub registration_mode: String,
    pub bind_host: String,
    pub public_host: String,
    pub game_address: String,
    pub gateway_port: u16,
    pub content_port: u16,
    pub admin_port: u16,
    pub admin_enabled: bool,
    pub firewall_mode: String,
    pub workspace_dir: PathBuf,
    pub preset_dir: Option<PathBuf>,
    pub editions: EditionsMap,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::default_for(EditionSelection::Both, PathBuf::from(".workspace"))
    }
}

impl ServerConfig {
    pub fn default_for(selection: EditionSelection, workspace_dir: PathBuf) -> Self {
        Self {
            server_name: "Reforged Revival Server".into(),
            server_mode: "private".into(),
            registration_mode: "open".into(),
            bind_host: "127.0.0.1".into(),
            public_host: "127.0.0.1".into(),
            game_address: "127.0.0.1".into(),
            gateway_port: 8443,
            content_port: 8446,
            admin_port: 8447,
            admin_enabled: false,
            firewall_mode: "off".into(),
            workspace_dir,
            preset_dir: None,
            editions: EditionsMap {
                reforged: EditionConfig::default_for(Edition::Reforged, selection.includes(Edition::Reforged)),
                refined: EditionConfig::default_for(Edition::Refined, selection.includes(Edition::Refined)),
            },
        }
    }

    pub fn from_str(text: &str, format: ConfigFormat) -> Result<Self, String> {
        let mut cfg: Self = match format {
            ConfigFormat::Toml => toml::from_str(text).map_err(|e| format!("TOML config error: {e}"))?,
            ConfigFormat::Json => serde_json::from_str(text).map_err(|e| format!("JSON config error: {e}"))?,
            ConfigFormat::Yaml => {
                let value = parse_yaml_to_json(text)?;
                serde_json::from_value(value).map_err(|e| format!("YAML config error: {e}"))?
            }
        };
        // If refined kept serde's struct-level default (61000) because the user omitted port fields
        // in their `[editions.refined]` section, normalize to Refined's distinct default ports.
        if cfg.editions.refined.backend_port == Edition::Reforged.default_backend_port()
            && cfg.editions.reforged.backend_port == Edition::Reforged.default_backend_port()
        {
            cfg.editions.refined.backend_port = Edition::Refined.default_backend_port();
        }
        if cfg.editions.refined.director_port == Edition::Reforged.default_director_port()
            && cfg.editions.reforged.director_port == Edition::Reforged.default_director_port()
        {
            cfg.editions.refined.director_port = Edition::Refined.default_director_port();
        }
        if cfg.editions.refined.udp_port_begin == Edition::Reforged.default_udp_port_range().0
            && cfg.editions.reforged.udp_port_begin == Edition::Reforged.default_udp_port_range().0
        {
            let (begin, end) = Edition::Refined.default_udp_port_range();
            cfg.editions.refined.udp_port_begin = begin;
            cfg.editions.refined.udp_port_end = end;
        }
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let format = ConfigFormat::from_path(path).ok_or_else(|| {
            format!(
                "unsupported config file extension for {} (expected .toml, .yaml, .yml, or .json)",
                path.display()
            )
        })?;
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("could not read config {}: {e}", path.display()))?;
        Self::from_str(&text, format)
    }

    /// Loads `explicit_path` if given, or searches `.workspace/config/` for `host.toml`, `host.yaml`,
    /// `host.yml`, or `host.json`, or falls back to zero-config defaults for `default_editions`.
    pub fn load_or_default(
        explicit_path: Option<&Path>,
        default_editions: EditionSelection,
        workspace_dir: PathBuf,
    ) -> Result<Self, String> {
        if let Some(path) = explicit_path {
            let mut cfg = Self::load(path)?;
            cfg.constrain_to_editions(default_editions);
            return Ok(cfg);
        }
        let config_dir = workspace_dir.join("config");
        for candidate in ["host.toml", "host.yaml", "host.yml", "host.json"] {
            let path = config_dir.join(candidate);
            if path.is_file() {
                let mut cfg = Self::load(&path)?;
                cfg.constrain_to_editions(default_editions);
                return Ok(cfg);
            }
        }
        Ok(Self::default_for(default_editions, workspace_dir))
    }

    pub fn constrain_to_editions(&mut self, allowed: EditionSelection) {
        if !allowed.includes(Edition::Reforged) {
            self.editions.reforged.enabled = false;
        }
        if !allowed.includes(Edition::Refined) {
            self.editions.refined.enabled = false;
        }
    }

    /// Returns the enabled `(Edition, EditionConfig)` pairs permitted by `compiled`.
    pub fn active_editions(&self, compiled: EditionSelection) -> Vec<(Edition, EditionConfig)> {
        let mut out = Vec::new();
        if compiled.includes(Edition::Reforged) && self.editions.reforged.enabled {
            out.push((Edition::Reforged, self.editions.reforged.clone()));
        }
        if compiled.includes(Edition::Refined) && self.editions.refined.enabled {
            out.push((Edition::Refined, self.editions.refined.clone()));
        }
        out
    }

    /// Union of UDP ports across all active editions (for the in-process allowlist).
    pub fn all_udp_ports(&self, compiled: EditionSelection) -> Vec<u16> {
        let mut ports = Vec::new();
        for (_, ed_cfg) in self.active_editions(compiled) {
            for port in ed_cfg.udp_ports() {
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
        ports
    }

    /// Primary backend URL (first active edition's backend, defaulting to Reforged).
    pub fn primary_backend_url(&self, compiled: EditionSelection) -> String {
        let port = self
            .active_editions(compiled)
            .first()
            .map_or(self.editions.reforged.backend_port, |(_, cfg)| cfg.backend_port);
        format!("http://127.0.0.1:{port}")
    }

    /// Primary director URL (first active edition's director, defaulting to Reforged).
    pub fn primary_director_url(&self, compiled: EditionSelection) -> String {
        let port = self
            .active_editions(compiled)
            .first()
            .map_or(self.editions.reforged.director_port, |(_, cfg)| cfg.director_port);
        format!("http://127.0.0.1:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_yaml_and_json_identically() {
        let toml_text = r#"
server_name = "Dual Edition Server"
server_mode = "public"
gateway_port = 8443

[editions.reforged]
enabled = true
backend_port = 61000
director_port = 61001

[editions.refined]
enabled = true
backend_port = 61010
director_port = 61011
"#;
        let yaml_text = r#"
server_name: "Dual Edition Server"
server_mode: "public"
gateway_port: 8443
editions:
  reforged:
    enabled: true
    backend_port: 61000
    director_port: 61001
  refined:
    enabled: true
    backend_port: 61010
    director_port: 61011
"#;
        let json_text = r#"{
  "server_name": "Dual Edition Server",
  "server_mode": "public",
  "gateway_port": 8443,
  "editions": {
    "reforged": {
      "enabled": true,
      "backend_port": 61000,
      "director_port": 61001
    },
    "refined": {
      "enabled": true,
      "backend_port": 61010,
      "director_port": 61011
    }
  }
}"#;
        let from_toml = ServerConfig::from_str(toml_text, ConfigFormat::Toml).unwrap();
        let from_yaml = ServerConfig::from_str(yaml_text, ConfigFormat::Yaml).unwrap();
        let from_json = ServerConfig::from_str(json_text, ConfigFormat::Json).unwrap();
        assert_eq!(from_toml, from_yaml);
        assert_eq!(from_yaml, from_json);
        assert_eq!(from_toml.active_editions(EditionSelection::Both).len(), 2);
        assert_eq!(from_toml.all_udp_ports(EditionSelection::Both).len(), 16);
    }
}
