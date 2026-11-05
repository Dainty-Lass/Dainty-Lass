//! The Director's log lines, into the controller's `tracing` output (the host kit keeps each module's
//! output). Messages are Node's, kept word for word so a host's greps still match.

pub use crate::hub::Level;

pub fn info(message: &str) {
    tracing::info!("{message}");
}

pub fn warn(message: &str) {
    tracing::warn!("{message}");
}

pub fn error(message: &str) {
    tracing::error!("{message}");
}

/// A line with the error that caused it, as `logger.warn({ error }, message)`.
pub fn with_error(level: Level, message: &str, error: &str) {
    match level {
        Level::Info => tracing::info!(error, "{message}"),
        Level::Warn => tracing::warn!(error, "{message}"),
        Level::Error => tracing::error!(error, "{message}"),
    }
}

/// The supervisors' and the resolver's log, into this one.
pub struct HuntLog;

impl crate::hunt::HuntLog for HuntLog {
    fn info(&self, message: &str) {
        info(message);
    }

    fn warn(&self, message: &str) {
        warn(message);
    }
}
