//! A service that writes what the core asks of it to `ECHO_FILE`, for controller's tests (RD27): built as a library
//! here, and compiled into the test itself through `#[path]`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dainty_module_api::manifest::ModuleManifest;
use dainty_module_api::registrar::Registration;
use dainty_module_api::{BoxFuture, CoreCapability, ModuleError, ModuleId};
use dainty_service_kit::context::ServiceContext;
use dainty_service_kit::env::Env;

#[derive(Default)]
pub struct Echo {
    file: Mutex<Option<PathBuf>>,
}

impl Echo {
    fn write(&self, line: &str) {
        let file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(file) = file
            && let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(file)
        {
            _ = writeln!(out, "{line}");
        }
    }
}

impl dainty_service_plugin::Service for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::ContentFiles]
    }

    fn attach(&self, module: &ModuleManifest, _: &Registration, dir: Option<&Path>) -> Result<(), ModuleError> {
        let file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // A test turns the service against modules with a `.refuse` file beside its own
        if file.is_some_and(|file| file.with_extension("refuse").exists()) {
            self.write(&format!("refuse {}", module.id));
            return Err(ModuleError::new(dainty_module_api::ErrorKind::Invalid, "refused"));
        }
        self.write(&format!("attach {} dir={}", module.id, dir.is_some()));
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        self.write(&format!("detach {module}"));
    }

    fn run(&self, env: Env, mut ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        *self.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = env.get("ECHO_FILE").map(PathBuf::from);
        self.write("start");
        let greeting = env.get("ECHO_GREETING").unwrap_or_default().to_owned();
        let file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Box::pin(async move {
            tracing::info!(target: "echo", "echo is up");
            ctx.status().ready(None, Some(format!("greeting={greeting}")));
            ctx.wait_stop().await;
            if let Some(file) = file
                && let Ok(mut out) = std::fs::OpenOptions::new().append(true).open(file)
            {
                _ = writeln!(out, "stop");
            }
            Ok(())
        })
    }
}

dainty_service_plugin::export_service!(Echo::default());
