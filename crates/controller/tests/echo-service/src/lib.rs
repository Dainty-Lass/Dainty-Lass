//! A service that writes what the core asks of it to `ECHO_FILE`, for controller's tests (RD27): built as a library
//! here, and compiled into the test itself through `#[path]`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use module_api::manifest::ModuleManifest;
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, ModuleError, ModuleId};
use service_kit::context::ServiceContext;
use service_kit::env::Env;

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

impl service_plugin::Service for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::ContentFiles]
    }

    fn attach(&self, module: &ModuleManifest, _: &Registration, dir: Option<&Path>) -> Result<(), ModuleError> {
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

service_plugin::export_service!(Echo::default());
