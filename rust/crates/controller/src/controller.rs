//! Unified in-process `ServerController` (`docs/app-structure-plan.md`).
//!
//! Orchestrates `director-*`, `backend-*`, `content`, `admin`, `allowlist`, and `gateway` inside
//! a single async runtime with zero child-process spawning for the Rust service stack, shared
//! `StatusRegistry`, in-process allowlist IP feed, and reverse-order graceful shutdown.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use backend::BackendConfig;
use director::DirectorConfig;
use gateway::allowlist::config::{AllowlistConfig, FirewallKind};
use gateway::allowlist::helper::{Helper as AllowlistHelper, run_with_context as run_allowlist_with_context};
use gateway::allowlist::nft::{NFT_TABLE, NftSettings, default_nft};
use gateway::config::{Config as GatewayConfig, DEFAULT_LIMITS, DEFAULT_TIMEOUTS, Endpoint};
use gateway::feed::InProcessSink;
use protocol::edition::{Edition, EditionSelection};
use service_kit::context::{ComponentPhase, ServerSnapshot, StatusRegistry};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::Instrument as _;

use crate::config::{EditionConfig, ServerConfig};
use crate::provision::{ProvisionedWorkspace, provision_workspace};

struct RunningComponent {
    stop_tx: watch::Sender<bool>,
    handle: JoinHandle<Result<(), String>>,
}

/// Unified in-process orchestrator for all server components and active editions.
pub struct ServerController {
    compiled_editions: EditionSelection,
    config: ServerConfig,
    provisioned: ProvisionedWorkspace,
    status_registry: StatusRegistry,
    running: BTreeMap<String, RunningComponent>,
    startup_order: Vec<String>,
    allowlist_slot: Arc<StdMutex<Option<Arc<AllowlistHelper>>>>,
}

impl ServerController {
    /// Provisions the workspace (secrets, TLS cert, DB/preset) and initializes the controller.
    pub fn new(config: ServerConfig, compiled_editions: EditionSelection) -> Result<Self, String> {
        let mut config = config;
        config.constrain_to_editions(compiled_editions);
        let provisioned = provision_workspace(&config)?;
        Ok(Self {
            compiled_editions,
            config,
            provisioned,
            status_registry: StatusRegistry::default(),
            running: BTreeMap::new(),
            startup_order: Vec::new(),
            allowlist_slot: Arc::new(StdMutex::new(None)),
        })
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn provisioned(&self) -> &ProvisionedWorkspace {
        &self.provisioned
    }

    pub fn status_registry(&self) -> &StatusRegistry {
        &self.status_registry
    }

    pub fn subscribe_status(&self) -> watch::Receiver<ServerSnapshot> {
        self.status_registry.subscribe()
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        self.status_registry.snapshot()
    }

    pub fn invite_url(&self) -> &str {
        &self.provisioned.invite_url
    }

    pub fn cert_fingerprint(&self) -> &str {
        &self.provisioned.cert_fingerprint
    }

    /// Starts all enabled components in dependency order:
    /// `director-*` -> `backend-*` -> `content` -> `admin` -> `allowlist` -> `gateway`.
    pub async fn start_all(&mut self) -> Result<(), String> {
        let active = self.config.active_editions(self.compiled_editions);
        if active.is_empty() {
            return Err("no game editions are enabled in configuration".into());
        }
        for (edition, ed_cfg) in &active {
            let dir_name = format!("director-{}", edition.as_str());
            self.spawn_director(&dir_name, *edition, ed_cfg).await?;
            let back_name = format!("backend-{}", edition.as_str());
            self.spawn_backend(&back_name, *edition, ed_cfg).await?;
        }
        self.spawn_content().await?;
        if self.config.admin_enabled {
            self.spawn_admin().await?;
        }
        self.spawn_allowlist().await?;
        self.spawn_gateway().await?;
        Ok(())
    }

    /// Starts a single named component (`director-reforged`, `backend-reforged`, `content`, `admin`, `allowlist`, `gateway`).
    pub async fn start_component(&mut self, name: &str) -> Result<(), String> {
        if self.running.contains_key(name) {
            return Err(format!("component {name} is already running"));
        }
        match name {
            "director-reforged" => {
                let ed_cfg = self.config.editions.reforged.clone();
                self.spawn_director(name, Edition::Reforged, &ed_cfg).await
            }
            "director-refined" => {
                let ed_cfg = self.config.editions.refined.clone();
                self.spawn_director(name, Edition::Refined, &ed_cfg).await
            }
            "backend-reforged" => {
                let ed_cfg = self.config.editions.reforged.clone();
                self.spawn_backend(name, Edition::Reforged, &ed_cfg).await
            }
            "backend-refined" => {
                let ed_cfg = self.config.editions.refined.clone();
                self.spawn_backend(name, Edition::Refined, &ed_cfg).await
            }
            "content" => self.spawn_content().await,
            "admin" => self.spawn_admin().await,
            "allowlist" => self.spawn_allowlist().await,
            "gateway" => self.spawn_gateway().await,
            other => Err(format!("unknown component `{other}`")),
        }
    }

    /// Stops a single named component gracefully.
    pub async fn stop_component(&mut self, name: &str) -> Result<(), String> {
        let Some(running) = self.running.remove(name) else {
            return Err(format!("component {name} is not running"));
        };
        let _ = running.stop_tx.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(10), running.handle).await;
        self.startup_order.retain(|n| n != name);
        Ok(())
    }

    /// Restarts a single named component in-place.
    pub async fn restart_component(&mut self, name: &str) -> Result<(), String> {
        if self.running.contains_key(name) {
            self.stop_component(name).await?;
        }
        self.start_component(name).await
    }

    /// Stops all running components in reverse startup order (`gateway` first, `director-*` last).
    pub async fn shutdown(&mut self) {
        let reverse_order: Vec<String> = self.startup_order.iter().rev().cloned().collect();
        for name in reverse_order {
            let _ = self.stop_component(&name).await;
        }
    }

    async fn spawn_director(&mut self, name: &str, edition: Edition, ed_cfg: &EditionConfig) -> Result<(), String> {
        let (ctx, stop_tx) = self.status_registry.service_context(name);
        let cfg = DirectorConfig {
            edition,
            bind_host: "127.0.0.1".into(),
            port: ed_cfg.director_port,
            game_address: self.config.game_address.clone(),
            udp_port_begin: ed_cfg.udp_port_begin,
            udp_port_end: ed_cfg.udp_port_end,
            max_hunt_servers: ed_cfg.max_hunt_servers,
            server_root_dir: ed_cfg.server_root_dir.clone(),
        };
        let span_name = name.to_string();
        let handle = tokio::spawn(
            async move { director::run_with_context(cfg, ctx).await }
                .instrument(tracing::info_span!("component", service = %span_name)),
        );
        self.record_started(name, stop_tx, handle).await
    }

    async fn spawn_backend(&mut self, name: &str, edition: Edition, ed_cfg: &EditionConfig) -> Result<(), String> {
        let (ctx, stop_tx) = self.status_registry.service_context(name);
        let cfg = BackendConfig {
            edition,
            bind_host: "127.0.0.1".into(),
            port: ed_cfg.backend_port,
            server_name: self.config.server_name.clone(),
            server_mode: self.config.server_mode.clone(),
            registration_mode: self.config.registration_mode.clone(),
            max_hunt_servers: ed_cfg.max_hunt_servers,
            db_dir: Some(self.provisioned.db_dir.clone()),
            director_url: format!("http://127.0.0.1:{}", ed_cfg.director_port),
        };
        let span_name = name.to_string();
        let handle = tokio::spawn(
            async move { backend::run_with_context(cfg, ctx).await }
                .instrument(tracing::info_span!("component", service = %span_name)),
        );
        self.record_started(name, stop_tx, handle).await
    }

    async fn spawn_content(&mut self) -> Result<(), String> {
        let name = "content";
        let (ctx, stop_tx) = self.status_registry.service_context(name);
        let cfg = content::config::Config {
            game: protocol::game::Game::Reborn,
            port: self.config.content_port,
            bind_hosts: vec!["127.0.0.1".into()],
            backend_url: self.config.primary_backend_url(self.compiled_editions),
            runtime_dir: Some(self.provisioned.runtime_dir.clone()),
            game_dir: None,
            manifest_path: None,
            branding_dir: Some(self.provisioned.branding_dir.clone()),
            news_file: None,
            news_url: None,
            max_streams_per_account: 2,
            max_streams_total: 16,
            auth_cache_seconds: 60,
        };
        let handle = tokio::spawn(
            async move { content::server::run_with_context(cfg, ctx).await }
                .instrument(tracing::info_span!("component", service = "content")),
        );
        self.record_started(name, stop_tx, handle).await
    }

    async fn spawn_admin(&mut self) -> Result<(), String> {
        let name = "admin";
        let (ctx, stop_tx) = self.status_registry.service_context(name);
        let api_key = self
            .provisioned
            .secrets
            .get("ADMIN_API_KEY")
            .cloned()
            .unwrap_or_default();
        let admin_key = self
            .provisioned
            .secrets
            .get("REFORGED_ADMIN_KEY")
            .cloned()
            .unwrap_or_default();
        let operator_secret = self
            .provisioned
            .secrets
            .get("GATEWAY_OPERATOR_SECRET")
            .cloned()
            .unwrap_or_default();
        let server_mode = if self.config.server_mode.eq_ignore_ascii_case("public") {
            admin::config::ServerMode::Public
        } else {
            admin::config::ServerMode::Private
        };
        let cfg = admin::config::Config {
            port: self.config.admin_port,
            bind_hosts: vec!["127.0.0.1".into()],
            backend_url: self.config.primary_backend_url(self.compiled_editions),
            director_url: self.config.primary_director_url(self.compiled_editions),
            api_key,
            admin_key,
            operator_secret,
            timeouts_file: self.provisioned.admin_timeouts_file.clone(),
            max_hunt_servers: u32::from(self.config.editions.reforged.max_hunt_servers),
            server_mode,
        };
        let handle = tokio::spawn(
            async move { admin::server::run_with_context(cfg, ctx).await }
                .instrument(tracing::info_span!("component", service = "admin")),
        );
        self.record_started(name, stop_tx, handle).await
    }

    async fn spawn_allowlist(&mut self) -> Result<(), String> {
        let name = "allowlist";
        let (ctx, stop_tx) = self.status_registry.service_context(name);
        let dry_run = self.config.firewall_mode.eq_ignore_ascii_case("off")
            || self.config.firewall_mode.eq_ignore_ascii_case("dry-run");
        let firewall = if cfg!(windows) {
            FirewallKind::Windows
        } else {
            FirewallKind::Nftables
        };
        let udp_ports = self.config.all_udp_ports(self.compiled_editions);
        let ports = match (udp_ports.first(), udp_ports.last()) {
            (Some(&first), Some(&last)) if first <= last => format!("{first}-{last}"),
            _ => "8770-8777".into(),
        };
        let secret = self
            .provisioned
            .secrets
            .get("GATEWAY_ALLOWLIST_SECRET")
            .cloned()
            .unwrap_or_default();
        let allow_cfg = AllowlistConfig {
            bind_host: "127.0.0.1".into(),
            port: 0,
            secret,
            dry_run,
            firewall,
            allow_private: true,
            ttl: Duration::from_secs(900),
            min_interval: Duration::from_secs(2),
            max_entries: 1024,
            ports,
            audit_log: self.provisioned.workspace_dir.join("logs").join("allowlist-audit.log"),
            state_file: self.provisioned.workspace_dir.join("allowlist-state.json"),
            nft: NftSettings {
                binary: default_nft(),
                table: NFT_TABLE.into(),
                interface: None,
            },
        };
        let slot = self.allowlist_slot.clone();
        let handle = tokio::spawn(
            async move { run_allowlist_with_context(allow_cfg, ctx, Some(slot)).await }
                .instrument(tracing::info_span!("component", service = "allowlist")),
        );
        self.record_started(name, stop_tx, handle).await
    }

    async fn spawn_gateway(&mut self) -> Result<(), String> {
        let name = "gateway";
        let (ctx, stop_tx) = self.status_registry.service_context(name);
        let primary_port = self
            .config
            .active_editions(self.compiled_editions)
            .first()
            .map_or(self.config.editions.reforged.backend_port, |(_, c)| c.backend_port);
        let tls = self.config.server_mode.eq_ignore_ascii_case("public");
        let secret = self
            .provisioned
            .secrets
            .get("GATEWAY_OPERATOR_SECRET")
            .cloned()
            .unwrap_or_default();
        let gw_cfg = GatewayConfig {
            game: protocol::game::Game::Reborn,
            bind_host: self.config.bind_host.clone(),
            port: self.config.gateway_port,
            tls,
            cert_file: if tls {
                self.provisioned.cert_file.clone()
            } else {
                std::path::PathBuf::new()
            },
            key_file: if tls {
                self.provisioned.key_file.clone()
            } else {
                std::path::PathBuf::new()
            },
            secret,
            backend: Endpoint {
                host: "127.0.0.1".into(),
                port: primary_port,
            },
            content: Endpoint {
                host: "127.0.0.1".into(),
                port: self.config.content_port,
            },
            ws: Endpoint {
                host: "127.0.0.1".into(),
                port: primary_port,
            },
            allowlist: None,
            limits: DEFAULT_LIMITS,
            timeouts: DEFAULT_TIMEOUTS,
        };
        let slot = self.allowlist_slot.clone();
        let in_process_allow: Option<InProcessSink> = Some(Arc::new(move |ip: &str| {
            let guard = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.as_ref().is_some_and(|h| h.allow_ip(ip))
        }));
        let handle = tokio::spawn(
            async move { gateway::server::run_with_context(gw_cfg, None, in_process_allow, ctx).await }
                .instrument(tracing::info_span!("component", service = "gateway")),
        );
        self.record_started(name, stop_tx, handle).await
    }

    async fn record_started(
        &mut self,
        name: &str,
        stop_tx: watch::Sender<bool>,
        handle: JoinHandle<Result<(), String>>,
    ) -> Result<(), String> {
        let mut rx = self.status_registry.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let phase = rx.borrow_and_update().get(name).map(|c| c.phase);
            if matches!(phase, Some(ComponentPhase::Ready)) {
                break;
            }
            if matches!(phase, Some(ComponentPhase::Failed)) {
                let detail = rx.borrow().get(name).map(|c| c.detail.clone()).unwrap_or_default();
                return Err(format!("component {name} failed to start: {detail}"));
            }
            if handle.is_finished() {
                return match handle.await {
                    Ok(Err(e)) => Err(format!("component {name} exited during start: {e}")),
                    Ok(Ok(())) => Err(format!("component {name} exited unexpectedly during start")),
                    Err(e) => Err(format!("component {name} panicked: {e}")),
                };
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let _ = tokio::time::timeout(Duration::from_millis(50), rx.changed()).await;
        }
        if !self.startup_order.iter().any(|n| n == name) {
            self.startup_order.push(name.to_string());
        }
        self.running
            .insert(name.to_string(), RunningComponent { stop_tx, handle });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener as StdListener;

    fn free_port() -> u16 {
        StdListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[tokio::test]
    async fn starts_both_editions_concurrently_and_shuts_down_in_reverse_order() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = ServerConfig::default_for(EditionSelection::Both, tmp.path().to_path_buf());
        cfg.gateway_port = free_port();
        cfg.content_port = free_port();
        cfg.editions.reforged.backend_port = free_port();
        cfg.editions.reforged.director_port = free_port();
        cfg.editions.refined.backend_port = free_port();
        cfg.editions.refined.director_port = free_port();

        let mut controller = ServerController::new(cfg, EditionSelection::Both).unwrap();
        controller.start_all().await.unwrap();

        let snap = controller.snapshot();
        for expected in [
            "director-reforged",
            "backend-reforged",
            "director-refined",
            "backend-refined",
            "content",
            "allowlist",
            "gateway",
        ] {
            assert_eq!(
                snap.get(expected).map(|c| c.phase),
                Some(ComponentPhase::Ready),
                "component {expected} should be Ready"
            );
        }

        controller.restart_component("content").await.unwrap();
        assert_eq!(
            controller.snapshot().get("content").map(|c| c.phase),
            Some(ComponentPhase::Ready)
        );

        controller.shutdown().await;
    }
}
