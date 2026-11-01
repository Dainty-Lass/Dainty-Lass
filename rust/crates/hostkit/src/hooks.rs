// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Runtime hook boundary (`HostHooks`) allowing `hostkit` to remain 100% MIT with zero
//! dependencies on AGPL service or game-module crates while `services/host` wires in the
//! compiled-in services, generators and database migration tables.

use std::any::Any;
use std::path::Path;
use std::process::ExitCode;
use std::sync::OnceLock;

use crate::error::HostError;
use crate::gamedata::Step;
use crate::hostconfig::HostConfig;
use crate::paths::Paths;

pub type AnySlot = Box<dyn Any + Send + Sync>;

pub trait HostHooks: Send + Sync {
    fn built_in_services(&self) -> &'static [&'static str] {
        crate::BUILT_IN_SERVICES
    }

    fn run_generator(&self, _paths: &Paths, step: &Step<'_>) -> Result<crate::run::RunResult, HostError> {
        Err(HostError::message(format!(
            "in-process generator for {} is not registered in this build",
            step.title
        )))
    }

    fn mongo_has_documents(&self, _uri: &str) -> Result<bool, String> {
        Err("MongoDB check is not registered in this build".to_owned())
    }

    #[allow(clippy::type_complexity)]
    fn db_migrate(
        &self,
        _source_uri: &str,
        _db_cfg: &store::DatabaseConfig,
        _dry_run: bool,
    ) -> Result<(Vec<(String, usize)>, Vec<(String, String)>), HostError> {
        Err(HostError::message("database migration is not registered in this build"))
    }

    fn db_verify(
        &self,
        _source_uri: &str,
        _db_cfg: &store::DatabaseConfig,
    ) -> Result<Vec<(String, String)>, HostError> {
        Err(HostError::message(
            "database verification is not registered in this build",
        ))
    }

    fn db_export_mongo(&self, _uri: &str, _out: &Path) -> Result<Vec<(String, usize)>, HostError> {
        Err(HostError::message("MongoDB export is not registered in this build"))
    }

    fn run_db_command(
        &self,
        _paths: &Paths,
        _args: &crate::cli::DbArgs,
        _out: &mut dyn FnMut(&str),
    ) -> Option<crate::Outcome> {
        None
    }

    fn install_director_runner(&self, _paths: &Paths) -> Result<(), HostError> {
        Ok(())
    }

    fn compose_in_process_allowlist(
        &self,
        _controller: &mut controller::ServerController,
        _paths: &Paths,
        _registry: &controller::ModuleRegistry,
    ) -> Result<Option<AnySlot>, HostError> {
        Ok(None)
    }

    fn compose_in_process_content(
        &self,
        _controller: &mut controller::ServerController,
        _paths: &Paths,
        _registry: &controller::ModuleRegistry,
    ) -> Result<(), HostError> {
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_in_process_network_and_gateway(
        &self,
        _controller: &mut controller::ServerController,
        _paths: &Paths,
        _config: &HostConfig,
        _registry: &controller::ModuleRegistry,
        _slot: Option<AnySlot>,
        _in_process_gateway: bool,
        _public: bool,
        _guarded: bool,
    ) -> Result<(), HostError> {
        Ok(())
    }

    fn ensure_gateway_cert(&self, _paths: &Paths, _config: &HostConfig) -> Result<(), HostError> {
        Ok(())
    }

    fn game_modules(&self) -> Vec<controller::ModuleFactory> {
        controller::modules::game_modules()
    }

    fn run_service(&self, _name: &str, _args: &[String]) -> Option<ExitCode> {
        None
    }
}

struct DefaultHooks;
impl HostHooks for DefaultHooks {}

static HOOKS: OnceLock<&'static dyn HostHooks> = OnceLock::new();
static DEFAULT_HOOKS: DefaultHooks = DefaultHooks;

pub fn set_hooks(hooks: &'static dyn HostHooks) {
    let _ = HOOKS.set(hooks);
}

pub fn get() -> &'static dyn HostHooks {
    *HOOKS.get_or_init(|| &DEFAULT_HOOKS)
}
