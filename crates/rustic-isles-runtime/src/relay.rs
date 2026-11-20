//! The in-game relay: what the launcher ran while the game played, now on a thread of the game's own, so the game
//! keeps talking to its server after the launcher closes. The launcher names the server in the environment
//! (`config::relay`); without that, it runs the relay itself and this does nothing.

use std::sync::Arc;

use dainty_relay::Level;
use dainty_relay::endpoint::Endpoint;
use dainty_relay::owner::{GameOwnerCheck, OwnerCheck};
use dainty_relay::relay::{DEFAULT_QOS_PORT, LogSink, PinSink, Relay, RelayConfig};
use rustic_isles_runtime_core::config::relay::{CAPABILITY_MARKER, RelayTarget};

use crate::config;
use crate::ffi::log;
use crate::ffi::tcp_table::{IpHelperTable, is_wine};

/// Starts the relay's thread when the launcher asked for it. Runs under the loader lock: the thread does
/// everything, once the lock is released.
pub fn start() {
    let target = match config::relay_target() {
        None => return,
        Some(Ok(target)) => target,
        Some(Err(refusal)) => {
            log::defer(format!("relay: not started: {}", refusal.text()));
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("dainty-relay".to_owned())
        .spawn(move || run(target));
    if spawned.is_err() {
        log::defer("relay: not started: no thread".to_owned());
    }
}

fn run(target: RelayTarget) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            log::line(&format!("relay: not started: {error}"));
            return;
        }
    };
    runtime.block_on(serve(target));
}

async fn serve(target: RelayTarget) {
    let described = format!("{}:{} ({})", target.host, target.port, mode(&target));
    // Wine can't tell the game's connections from another program's; the launcher has no check there either (RD21)
    let owner: Option<Arc<dyn OwnerCheck>> = if is_wine() {
        None
    } else {
        Some(Arc::new(GameOwnerCheck::new(
            || Some(std::process::id()),
            IpHelperTable,
        )))
    };
    let sink: LogSink = Arc::new(|level, line| {
        let level = match level {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        };
        log::line(&format!("relay {level}: {line}"));
    });
    let on_pin: PinSink = Arc::new(|seen| {
        log::line(&format!(
            "relay error: the server presented certificate {seen}, not the invite's"
        ));
    });
    let config = RelayConfig {
        port: target.listen,
        qos_port: Some(DEFAULT_QOS_PORT),
        target: Endpoint {
            host: target.host,
            port: target.port,
            pin: target.pin,
        },
        owner,
        log: sink,
        on_pin_mismatch: on_pin,
    };
    match Relay::start(config).await {
        Ok(relay) => {
            log::line(&format!(
                "{CAPABILITY_MARKER}: relay on 127.0.0.1:{} to {described}",
                relay.port()
            ));
            // The relay lives as long as the game; the process ending closes it
            std::future::pending::<()>().await;
            drop(relay);
        }
        Err(error) => log::line(&format!("relay: not started: {error}")),
    }
}

fn mode(target: &RelayTarget) -> &'static str {
    if target.pin.is_some() { "public" } else { "private" }
}
