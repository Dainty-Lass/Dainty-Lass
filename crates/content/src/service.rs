// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Content as a service library the core loads (RD27, `service_plugin`): the same server, with the module
//! table it owns. Each module's folder is recorded before the module reaches the table, so the table's registry
//! view can find what the module registered.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use module_api::manifest::ModuleManifest;
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, CorePlugin as _, ModuleError, ModuleId};
use service_kit::context::ServiceContext;
use service_kit::env::Env;

use crate::config::Config;
use crate::modules::{ContentModules, RegistryView};
use crate::server;

type Dirs = Arc<Mutex<BTreeMap<ModuleId, PathBuf>>>;

struct RecordedDirs(Dirs);

impl RegistryView for RecordedDirs {
    fn module_dir(&self, module: &ModuleId) -> Option<PathBuf> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module)
            .cloned()
    }
}

/// Content as a service library: the server and the module table the controller attaches modules to.
pub struct ContentService {
    modules: Arc<ContentModules>,
    dirs: Dirs,
}

impl ContentService {
    /// A service with an empty module table.
    pub fn new() -> Self {
        let dirs = Dirs::default();
        Self {
            modules: Arc::new(ContentModules::new(Arc::new(RecordedDirs(dirs.clone())))),
            dirs,
        }
    }

    /// The module table, for the controller to attach modules to.
    pub fn modules(&self) -> &Arc<ContentModules> {
        &self.modules
    }

    fn dirs(&self) -> std::sync::MutexGuard<'_, BTreeMap<ModuleId, PathBuf>> {
        self.dirs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for ContentService {
    fn default() -> Self {
        Self::new()
    }
}

impl service_plugin::Service for ContentService {
    fn name(&self) -> &'static str {
        "content"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::ContentFiles]
    }

    fn attach(
        &self,
        module: &ModuleManifest,
        registration: &Registration,
        dir: Option<&Path>,
    ) -> Result<(), ModuleError> {
        match dir {
            Some(dir) => self.dirs().insert(module.id.clone(), dir.to_owned()),
            None => self.dirs().remove(&module.id),
        };
        self.modules.attach(module, registration)
    }

    fn detach(&self, module: &ModuleId) {
        self.modules.detach(module);
        self.dirs().remove(module);
    }

    fn run(&self, env: Env, ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        let modules = self.modules.clone();
        Box::pin(async move {
            let config = Config::from_env(&env)?;
            server::run_with_modules(config, ctx, modules).await
        })
    }
}
