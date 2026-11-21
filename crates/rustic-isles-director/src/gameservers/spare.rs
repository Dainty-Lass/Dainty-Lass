//! The warm spare hunt server (prototype, `GAMESERVER_WARM_SPARE=1`): one hunt server started before anyone asks,
//! with a placeholder hunt and `RUSTIC_ISLES_SPARE=1`, whose runtime holds it just before its first map load
//! (`rustic_isles_runtime_core::spare`). The next hunt request hands it the real hunt through
//! `rustic_isles_spare_port<port>.txt` and waits for its ready marker as for any hunt; then a new spare starts. The
//! spare takes one of the `MAX_CONCURRENT_HUNT_SERVERS` slots.

use super::{
    Director, GameServer, ServerKind, ServerSpec, ServerState, ServerStatus, StartError, label_for, lock, now_ms,
    roster_argument,
};
use crate::launcher::{Command, OrNull};
use crate::process::{self, Labels, Proc};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// The placeholder the spare answers before its hunt: only the engine's early reads of the default map see it.
const SPARE_MAP: &str = "/Game/Maps/islands/adventure/Moss_Triforce/adventure_moss_triforce";
/// Tells the runtime to hold (`rustic_isles_runtime_core::spare::VARIABLE`).
const SPARE_VARIABLE: &str = "RUSTIC_ISLES_SPARE";

/// A booted spare waiting for its hunt.
pub(super) struct Spare {
    port: u16,
    launch_id: String,
    proc: Arc<Proc>,
    ready: oneshot::Receiver<()>,
    log_path: PathBuf,
    reservation: Option<u64>,
}

/// The spare slot in [`super::Inner`].
#[derive(Default)]
pub(super) struct SpareSlot {
    pub(super) spare: Option<Spare>,
    pub(super) starting: bool,
    /// A spare listened before its hunt (the runtime did not hold): no more spares this run.
    failed: bool,
}

impl SpareSlot {
    /// Hunt slots the spare takes: one while it starts or waits.
    pub(super) fn slots(&self) -> usize {
        usize::from(self.starting || self.spare.is_some())
    }
}

impl Director {
    /// Starts a spare when spares are on, none is up or starting, and a hunt slot is free.
    pub(crate) fn ensure_spare(self: &Arc<Self>) {
        if !self.config.warm_spare || self.is_stopping() {
            return;
        }
        {
            let mut inner = lock(&self.inner);
            if inner.spare.failed || inner.spare.slots() > 0 {
                return;
            }
            let hunts = inner.servers.iter().filter(|s| s.is_hunt()).count() + inner.launching;
            if (hunts + 1) as f64 > self.config.max_hunts {
                return;
            }
            inner.spare.starting = true;
        }
        let director = Arc::clone(self);
        tokio::spawn(async move {
            let spawned = director.spawn_spare();
            let mut inner = lock(&director.inner);
            inner.spare.starting = false;
            match spawned {
                Ok(spare) => {
                    let proc = Arc::clone(&spare.proc);
                    let launch_id = spare.launch_id.clone();
                    inner.spare.spare = Some(spare);
                    drop(inner);
                    let weak = Arc::downgrade(&director);
                    tokio::spawn(async move {
                        let exit = proc.exited().await;
                        if let Some(director) = weak.upgrade() {
                            director.on_spare_exit(&launch_id, &exit);
                        }
                    });
                }
                Err(error) => tracing::warn!("[WarmSpare] no spare: {error}"),
            }
        });
    }

    fn spawn_spare(&self) -> Result<Spare, String> {
        let reservation = self.memory.reserve().map_err(|refused| refused.reason)?;
        let give_back = |reservation: Option<u64>| {
            if let Some(id) = reservation {
                self.memory.give_back(id);
            }
        };
        let launch_id = rustic_isles::replay::random_uuid();
        let Some(port) = lock(&self.inner).pool.take(&launch_id) else {
            give_back(reservation);
            return Err("No free ports left!".to_owned());
        };
        let fail = |message: String| {
            lock(&self.inner).pool.release(port, &launch_id);
            give_back(reservation);
            message
        };
        let (Some(program), Some(key)) = (self.config.binary_path.clone(), self.config.api_key()) else {
            return Err(fail(
                "GAMESERVER_BINARY_PATH and METAGAME_API_KEY must be set".to_owned(),
            ));
        };
        let folder = &self.config.log_dir;
        let log_path = folder.join(format!("port{port}_hunt.log"));
        let mut file = std::fs::create_dir_all(folder)
            .and_then(|()| std::fs::OpenOptions::new().create(true).append(true).open(&log_path))
            .map_err(|e| fail(format!("{}: {e}", log_path.display())))?;
        let header = format!(
            "=== Gameserver launch {} (local time), warm spare ===\nport={port} map={SPARE_MAP} (placeholder until a \
             hunt arrives)\n\n",
            chrono::Local::now().format("%-m/%-d/%Y, %-I:%M:%S %p"),
        );
        let _ = file.write_all(header.as_bytes());
        let mut args = vec![
            key.to_owned(),
            port.to_string(),
            SPARE_MAP.to_owned(),
            "NO_BEHEMOTH".to_owned(),
            "NO_MM_HUNTID".to_owned(),
            roster_argument(None),
            format!("{}:{port}", self.config.my_ip_text()),
        ];
        args.extend(self.config.launch_args.iter().cloned());
        let mut env = self.config.child_env(&launch_id, None);
        env.push((SPARE_VARIABLE.to_owned(), "1".to_owned()));
        let inject = crate::runtime::for_start(self.config.runtime_dir.as_deref(), &program, &mut env)
            .map_err(|e| fail(format!("the game runtime: {e}")))?;
        let command = Command {
            program,
            args,
            env,
            windows_hide: self.config.windows_hide,
            inject,
        };
        let labels = Labels {
            port,
            label: "hunt".to_owned(),
            launch_id: launch_id.clone(),
        };
        let spawned =
            process::launch(self.launcher.as_ref(), &command, file, labels).map_err(|e| fail(format!("spawn: {e}")))?;
        let proc = spawned.proc;
        {
            let mut inner = lock(&self.inner);
            if let Some(pid) = proc.pid {
                inner.pool.attach_pid(port, &launch_id, pid);
            }
            inner.procs.push(Arc::clone(&proc));
        }
        tracing::info!(
            "[WarmSpare] spare hunt server spawned on port {port} (pid={}); it waits before its map for a hunt",
            proc.pid.map_or_else(|| "unknown".to_owned(), |p| p.to_string())
        );
        Ok(Spare {
            port,
            launch_id,
            proc,
            ready: spawned.ready,
            log_path,
            reservation,
        })
    }

    fn on_spare_exit(self: &Arc<Self>, launch_id: &str, exit: &crate::launcher::Exit) {
        let spare = {
            let mut inner = lock(&self.inner);
            match inner.spare.spare.as_ref() {
                Some(spare) if spare.launch_id == launch_id => inner.spare.spare.take(),
                _ => None,
            }
        };
        let Some(spare) = spare else {
            return;
        };
        tracing::warn!(
            "[WarmSpare] the spare on port {} exited while waiting (code={}, signal={})",
            spare.port,
            OrNull(&exit.code),
            OrNull(&exit.signal)
        );
        self.drop_spare(&spare);
    }

    fn drop_spare(&self, spare: &Spare) {
        lock(&self.inner).pool.release(spare.port, &spare.launch_id);
        if let Some(id) = spare.reservation {
            self.memory.give_back(id);
        }
    }

    /// Hands the waiting spare `spec`'s hunt and waits for its ready marker; `None` when there is no usable spare,
    /// and the caller starts a server as before.
    pub(super) async fn start_from_spare(
        self: &Arc<Self>,
        spec: &ServerSpec,
    ) -> Option<Result<Arc<GameServer>, StartError>> {
        let mut spare = lock(&self.inner).spare.spare.take()?;
        match spare.ready.try_recv() {
            Err(oneshot::error::TryRecvError::Empty) if spare.proc.alive() => {}
            Ok(()) => {
                tracing::warn!(
                    "[WarmSpare] the spare on port {} listened before it had a hunt (its runtime did not hold); \
                     spares are off until the Director restarts",
                    spare.port
                );
                lock(&self.inner).spare.failed = true;
                let director = Arc::clone(self);
                let proc = Arc::clone(&spare.proc);
                tokio::spawn(async move {
                    let _ = director.stop_proc(&proc).await;
                });
                self.drop_spare(&spare);
                return None;
            }
            _ => {
                self.drop_spare(&spare);
                return None;
            }
        }
        let port = spare.port;
        if let Err(error) = self.write_spare_command(port, spec) {
            tracing::warn!("[WarmSpare] could not hand the spare on port {port} its hunt: {error}");
            let director = Arc::clone(self);
            let proc = Arc::clone(&spare.proc);
            tokio::spawn(async move {
                let _ = director.stop_proc(&proc).await;
            });
            self.drop_spare(&spare);
            return None;
        }
        let label = label_for(ServerKind::Hunt, spec.behemoth.as_deref());
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&spare.log_path) {
            let _ = writeln!(
                file,
                "\n=== Warm spare handed a hunt {} (local time) ===\nport={port} map={} behemoth={} mmHunt={} \
                 players={}\n",
                chrono::Local::now().format("%-m/%-d/%Y, %-I:%M:%S %p"),
                spec.map,
                spec.behemoth.as_deref().unwrap_or("NO_BEHEMOTH"),
                spec.matchmaker_hunt_id.as_deref().unwrap_or("NO_MM_HUNTID"),
                roster_argument(spec.roster.as_deref())
            );
        }
        let server = Arc::new(GameServer {
            launch_id: spare.launch_id.clone(),
            port,
            map: spec.map.clone(),
            behemoth: spec.behemoth.clone(),
            matchmaker_hunt_id: spec.matchmaker_hunt_id.clone(),
            kind: ServerKind::Hunt,
            label: label.clone(),
            log_path: spare.log_path.clone(),
            started_at_ms: now_ms(),
            proc: Arc::clone(&spare.proc),
            state: Mutex::new(ServerState {
                status: ServerStatus::Starting,
                ready_at_ms: None,
                ready_mono: None,
                roster: spec.roster.clone(),
            }),
        });
        lock(&self.inner).servers.push(Arc::clone(&server));
        let weak = Arc::downgrade(self);
        let watched = Arc::clone(&server);
        tokio::spawn(async move {
            let exit = watched.proc.exited().await;
            if let Some(director) = weak.upgrade() {
                director.on_process_exit(&watched, &exit);
            }
        });
        tracing::info!("[WarmSpare] {label} handed to the spare on port {port}");
        let result = self.await_ready(&server, spare.ready, spare.reservation).await;
        self.ensure_spare();
        Some(result)
    }

    /// The command file, written whole then renamed so the runtime never reads half of it.
    fn write_spare_command(&self, port: u16, spec: &ServerSpec) -> Result<(), String> {
        let dir = self
            .config
            .binary_path
            .as_deref()
            .and_then(|binary| std::path::Path::new(binary).parent())
            .ok_or("no GAMESERVER_BINARY_PATH")?;
        let mut text = format!(
            "map={}\nbehemoth={}\nhunt={}\nplayers={}\n",
            spec.map,
            spec.behemoth.as_deref().unwrap_or("NO_BEHEMOTH"),
            spec.matchmaker_hunt_id.as_deref().unwrap_or("NO_MM_HUNTID"),
            roster_argument(spec.roster.as_deref())
        );
        if let Some(week) = spec.matchmaker_hunt_id.as_deref().and_then(crate::hunt::arena_week) {
            text.push_str(&format!("week={week}\n"));
        }
        let path = dir.join(format!("rustic_isles_spare_port{port}.txt"));
        let partial = dir.join(format!("rustic_isles_spare_port{port}.partial"));
        std::fs::write(&partial, text)
            .and_then(|()| std::fs::rename(&partial, &path))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}
