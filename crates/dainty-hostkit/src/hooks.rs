// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Runtime hook boundary (`HostHooks`) allowing `hostkit` to remain 100% MIT with zero
//! dependencies on AGPL service or game-module crates while `crates/dainty-host` wires in the
//! compiled-in services, generators and database migration tables.

use std::any::Any;
use std::process::ExitCode;
use std::sync::OnceLock;

use crate::error::HostError;
use crate::gamedata::Step;
use crate::hostconfig::HostConfig;
use crate::paths::Paths;

/// What the allowlist hook hands the gateway hook: the allowlist's handle, typed by the host that made it.
pub type AnySlot = Box<dyn Any + Send + Sync>;

/// What the host that builds this kit plugs in: the services, generators and database tools it compiles in.
/// Every method's default is what a build without them does.
pub trait HostHooks: Send + Sync {
    /// The front services this build runs in its own process.
    fn built_in_services(&self) -> &'static [&'static str] {
        crate::BUILT_IN_SERVICES
    }

    /// Runs one game-data generation step; the default fails, having no generators.
    fn run_generator(&self, _paths: &Paths, step: &Step<'_>) -> Result<crate::run::RunResult, HostError> {
        Err(HostError::message(format!(
            "in-process generator for {} is not registered in this build",
            step.title
        )))
    }

    /// Migrates the MongoDB at `_source_uri` into the store (or checks it, with `_dry_run`).
    /// Returns the documents per collection and the mismatches; the default fails.
    #[allow(
        clippy::type_complexity,
        reason = "the per-collection counts and the mismatches, spelled out for implementors"
    )]
    fn db_migrate(
        &self,
        _paths: &Paths,
        _source_uri: &str,
        _db_cfg: &dainty_store::DatabaseConfig,
        _dry_run: bool,
    ) -> Result<(Vec<(String, usize)>, Vec<(String, String)>), HostError> {
        Err(HostError::message("database migration is not registered in this build"))
    }

    /// Compares the store with the MongoDB at `_source_uri`; returns the mismatches. The default fails.
    fn db_verify(
        &self,
        _paths: &Paths,
        _source_uri: &str,
        _db_cfg: &dainty_store::DatabaseConfig,
    ) -> Result<Vec<(String, String)>, HostError> {
        Err(HostError::message(
            "database verification is not registered in this build",
        ))
    }

    /// Runs a `db` command in place of the host kit's own; `None` leaves it to the kit.
    fn run_db_command(
        &self,
        _paths: &Paths,
        _args: &crate::cli::DbArgs,
        _out: &mut dyn FnMut(&str),
    ) -> Option<crate::Outcome> {
        None
    }

    /// Puts the Director's game server runner in place; the default does nothing.
    fn install_director_runner(&self, _paths: &Paths) -> Result<(), HostError> {
        Ok(())
    }

    /// Adds the allowlist to the controller in this process; returns its handle for the gateway, or `None` when not
    /// built in.
    fn compose_in_process_allowlist(
        &self,
        _controller: &mut dainty_controller::ServerController,
        _paths: &Paths,
        _registry: &dainty_controller::ModuleRegistry,
    ) -> Result<Option<AnySlot>, HostError> {
        Ok(None)
    }

    /// Adds the content server to the controller in this process; the default adds nothing.
    fn compose_in_process_content(
        &self,
        _controller: &mut dainty_controller::ServerController,
        _paths: &Paths,
        _registry: &dainty_controller::ModuleRegistry,
    ) -> Result<(), HostError> {
        Ok(())
    }

    /// Adds the network module and the gateway to the controller in this process, with the allowlist's handle.
    /// The default adds nothing.
    #[allow(
        clippy::too_many_arguments,
        reason = "the hook receives every part of the composed services"
    )]
    fn compose_in_process_network_and_gateway(
        &self,
        _controller: &mut dainty_controller::ServerController,
        _paths: &Paths,
        _config: &HostConfig,
        _registry: &dainty_controller::ModuleRegistry,
        _slot: Option<AnySlot>,
        _in_process_gateway: bool,
        _public: bool,
        _guarded: bool,
    ) -> Result<(), HostError> {
        Ok(())
    }

    /// A service `[services.<name>]` loads as a library (RD27). A build with a compose function of its own for the
    /// service (its env, its options) puts it in here.
    fn compose_service_library(
        &self,
        controller: &mut dainty_controller::ServerController,
        paths: &Paths,
        config: &HostConfig,
        registry: &dainty_controller::ModuleRegistry,
        library: dainty_controller::ServiceLibrary,
    ) -> Result<(), HostError> {
        crate::serve::add_service_library(controller, paths, config, registry, library)
    }

    /// Makes the gateway's certificate when it is missing; the default does nothing.
    fn ensure_gateway_cert(&self, _paths: &Paths, _config: &HostConfig) -> Result<(), HostError> {
        Ok(())
    }

    /// The game modules compiled into this build, for the server `_config` describes; an error refuses to serve.
    fn game_modules(
        &self,
        _paths: &Paths,
        _config: &HostConfig,
    ) -> Result<Vec<dainty_controller::ModuleFactory>, HostError> {
        Ok(dainty_controller::modules::game_modules())
    }

    /// Runs the service `_name` with its command line; `None` when this build has no such service.
    fn run_service(&self, _name: &str, _args: &[String]) -> Option<ExitCode> {
        None
    }
}

struct DefaultHooks;
impl HostHooks for DefaultHooks {}

static HOOKS: OnceLock<&'static dyn HostHooks> = OnceLock::new();
static DEFAULT_HOOKS: DefaultHooks = DefaultHooks;

/// Installs the host's hooks; only the first call counts. Call it before anything reads them.
pub fn set_hooks(hooks: &'static dyn HostHooks) {
    let _ = HOOKS.set(hooks);
}

/// The installed hooks, or the defaults when none were installed.
pub fn get() -> &'static dyn HostHooks {
    *HOOKS.get_or_init(|| &DEFAULT_HOOKS)
}
