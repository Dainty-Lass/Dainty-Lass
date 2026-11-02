//! Content as a service of the core (RD27): the one implementation `content-plugin` exports as a library and the core
//! runs compiled in, behind `service_plugin`'s protocol either way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use crate::modules::{ContentModules, RegistryView};
use module_api::manifest::ModuleManifest;
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, CorePlugin, ModuleError, ModuleId};
use service_kit::context::ServiceContext;
use service_kit::env::Env;

/// The module folders the core sends with each attach: the library can't ask the core's registry.
#[derive(Default)]
struct Dirs(RwLock<BTreeMap<ModuleId, PathBuf>>);

impl RegistryView for Dirs {
    fn module_dir(&self, module: &ModuleId) -> Option<PathBuf> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module)
            .cloned()
    }
}

/// Content as a service (`service_plugin::Service`): what `content-plugin` exports as a library and the core runs
/// compiled in, the same code either way (RD27).
pub struct ContentService {
    dirs: Arc<Dirs>,
    modules: Arc<ContentModules>,
}

impl Default for ContentService {
    fn default() -> Self {
        let dirs = Arc::new(Dirs::default());
        Self {
            modules: Arc::new(ContentModules::new(dirs.clone())),
            dirs,
        }
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
        {
            let mut dirs = self.dirs.0.write().unwrap_or_else(PoisonError::into_inner);
            match dir {
                Some(dir) => dirs.insert(module.id.clone(), dir.to_path_buf()),
                None => dirs.remove(&module.id),
            };
        }
        self.modules.attach(module, registration)
    }

    fn detach(&self, module: &ModuleId) {
        self.modules.detach(module);
        self.dirs
            .0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(module);
    }

    fn run(&self, env: Env, ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        let modules = self.modules.clone();
        Box::pin(async move {
            let config = crate::config::Config::from_env(&env)?;
            crate::server::run_with_modules(config, ctx, modules).await
        })
    }
}
