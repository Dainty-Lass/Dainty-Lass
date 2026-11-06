//! The launcher's log (row L29) over launcher-kit's `platform::log` (RL3), which writes and redacts. Kept
//! here: the library layer, which also mutes `egs-api` events that `log` forwarded (their target is then in
//! a field, so the MIT filter's plain target check misses them).

use std::path::PathBuf;

use launcher_kit::platform::log as kit;

pub use kit::describe_error;

pub fn info(message: &str) {
    kit::info(message);
}

pub fn warn(message: &str) {
    kit::warn(message);
}

pub fn error(message: &str) {
    kit::error(message);
}
#[cfg(test)]
pub use kit::redact;

/// Registers a value that must never reach the log; a server-made key without its `RRK_` too.
pub fn add_secret(value: &str) {
    kit::register_secret(value);
}

pub fn remove_secret(value: &str) {
    kit::unregister_secret(value);
}

/// Writes from now on to `<dir>/launcher.log`.
pub fn log_to_file(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    kit::set_folder(&dir);
}

/// Whether an event comes from `egs-api`, also when `log` forwarded it (its target is then in a field).
pub(crate) fn is_muted(event: &tracing::Event<'_>) -> bool {
    use tracing_log::NormalizeEvent as _;
    event.normalized_metadata().map_or_else(
        || epic::is_egs_api_target(event.metadata().target()),
        |meta| epic::is_egs_api_target(meta.target()),
    )
}

/// Library events (`tracing`) at INFO and above, into the same masked log.
pub(crate) struct Layer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Layer {
    /// False drops the event for the whole subscriber, so a layer stacked with this one never sees it either (the
    /// server GUI's channel).
    fn event_enabled(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) -> bool {
        !is_muted(event)
    }

    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        // Its failed requests are logged with `{:?}` (docs/security/epic.md)
        if is_muted(event) {
            return;
        }
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{value:?}");
                } else {
                    self.0.push_str(&format!(" {}={value:?}", field.name()));
                }
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        let level = *event.metadata().level();
        let text = format!("{}: {}", event.metadata().target(), message.0);
        match level {
            tracing::Level::ERROR => error(&text),
            tracing::Level::WARN => warn(&text),
            tracing::Level::INFO => info(&text),
            _ => {}
        }
    }
}

/// Sends library events and panics to the log. Once, at start.
pub fn install() {
    use tracing_subscriber::layer::SubscriberExt as _;
    let subscriber = tracing_subscriber::registry().with(Layer);
    let _ = tracing::subscriber::set_global_default(subscriber);
    std::panic::set_hook(Box::new(|panic| error(&format!("panic: {panic}"))));
}

/// Tests that point the log at a file of their own take this, since the file is the process's.
#[cfg(test)]
pub(crate) static FILE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The MIT log has no way back to stderr; later lines go to a folder of the test process's own.
#[cfg(test)]
pub(crate) fn stop_file_for_tests() {
    kit::set_folder(&std::env::temp_dir().join(format!("launcher-tests-{}", std::process::id())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Cases {
        secrets: Vec<String>,
        removed: String,
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        line: String,
        redacted: String,
    }

    #[test]
    fn the_shared_table() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test/cases/log.json");
        let cases: Cases = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for secret in &cases.secrets {
            add_secret(secret);
        }
        let (last, before) = cases.cases.split_last().unwrap();
        for case in before {
            assert_eq!(redact(&case.line), case.redacted, "{:?}", case.line);
        }
        remove_secret(&cases.removed);
        assert_eq!(redact(&last.line), last.redacted);
    }

    #[test]
    fn a_registered_key_never_reaches_the_file() {
        let _file = FILE_TESTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        log_to_file(dir.path().to_path_buf());
        let key = concat!("RRK_", "fedcba9876543210", "fedcba9876543210", "fedcba9876543210");
        add_secret(key);
        info(&format!("launching with -AUTH_PASSWORD={key} and {}", &key[4..]));
        tracing::subscriber::with_default(
            tracing_subscriber::layer::SubscriberExt::with(tracing_subscriber::registry(), Layer),
            || {
                tracing::warn!("library saw {}", &key[4..]);
                tracing::error!(target: "egs_api::api", "request to https://x/?sid=egs-api-secret failed");
            },
        );
        let text = std::fs::read_to_string(dir.path().join("launcher.log")).unwrap();
        assert!(!text.contains(&key[4..]), "{text}");
        assert!(!text.contains("egs-api-secret"), "{text}");
        assert!(text.contains(" INFO launching with -AUTH_PASSWORD=<hidden>"), "{text}");
        assert!(text.contains(" WARN "), "{text}");
        stop_file_for_tests();
    }
}
