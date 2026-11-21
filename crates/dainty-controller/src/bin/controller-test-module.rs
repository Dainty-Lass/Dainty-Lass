// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! module-api's stub module as an executable, for the registry's tests (DF3). `--id <id>` renames it,
//! a second module for the same game version; `--pid-file <file>` writes its pid, so a test can kill it;
//! `--grandchild <file>` first starts a process of its own that listens on loopback and writes the port
//! to `<file>`, so a test can see the whole tree go; `--die-after <ms>` exits that long after it starts, a
//! module that keeps crashing.

use std::process::{ExitCode, Stdio};
use std::sync::Arc;

use dainty_module_api::stub::StubModule;
use dainty_module_api::{
    AdminCall, AdminResponse, BoxFuture, CommandOutput, CommandRequest, Event, GameAnswer, GameCall, GameModule,
    ModuleConfig, ModuleContext, ModuleError, ModuleId, ModuleManifest, ModuleStatus, Registrar,
};

struct Renamed {
    stub: Arc<StubModule>,
    id: ModuleId,
}

impl GameModule for Renamed {
    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id.clone(),
            ..self.stub.manifest()
        }
    }

    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
        self.stub.register(registrar)
    }

    fn start(&self, ctx: ModuleContext, config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        self.stub.start(ctx, config)
    }

    fn stop(&self, ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        self.stub.stop(ctx)
    }

    fn status(&self) -> ModuleStatus {
        self.stub.status()
    }

    fn command(
        &self,
        ctx: ModuleContext,
        request: CommandRequest,
    ) -> BoxFuture<'_, Result<CommandOutput, ModuleError>> {
        self.stub.command(ctx, request)
    }

    fn admin(&self, ctx: ModuleContext, call: AdminCall) -> BoxFuture<'_, Result<AdminResponse, ModuleError>> {
        self.stub.admin(ctx, call)
    }

    fn event(&self, ctx: ModuleContext, event: Event) -> BoxFuture<'_, Result<(), ModuleError>> {
        self.stub.event(ctx, event)
    }

    fn game(&self, ctx: ModuleContext, call: GameCall) -> BoxFuture<'_, Result<GameAnswer, ModuleError>> {
        self.stub.game(ctx, call)
    }
}

fn listen(file: &str) -> ExitCode {
    let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") else {
        return ExitCode::FAILURE;
    };
    let Ok(addr) = listener.local_addr() else {
        return ExitCode::FAILURE;
    };
    if std::fs::write(file, addr.port().to_string()).is_err() {
        return ExitCode::FAILURE;
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if let Some(file) = value("--listen") {
        return listen(&file);
    }
    if let Some(file) = value("--grandchild") {
        // Its output must not be the module's pipe
        let started = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .args(["--listen", &file])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        });
        if let Err(error) = started {
            eprintln!("could not start the grandchild: {error}");
            return ExitCode::FAILURE;
        }
    }
    if let Some(file) = value("--pid-file")
        && let Err(error) = std::fs::write(&file, std::process::id().to_string())
    {
        eprintln!("could not write {file}: {error}");
        return ExitCode::FAILURE;
    }
    if let Some(ms) = value("--die-after").and_then(|ms| ms.parse::<u64>().ok()) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            std::process::exit(3);
        });
    }
    let stub = Arc::new(StubModule::new());
    match value("--id").map(|id| id.parse::<ModuleId>()) {
        None => dainty_module_api::child::main(StubModule::new()),
        Some(Ok(id)) => dainty_module_api::child::main(Renamed { stub, id }),
        Some(Err(error)) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
