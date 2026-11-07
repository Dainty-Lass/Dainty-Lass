//! Errors name what is wrong and where, never a value from the data: a key, a secret or a player's row stays out
//! of every message, since they reach the host's console and logs.

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The source is not what it says it is, or not something this importer accepts
    #[error("{0}")]
    Invalid(String),
    /// The 1.14.7 database to import into
    #[error("target database: {0}")]
    Target(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Usage(String),
}

impl ImportError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub fn io(path: &std::path::Path, error: impl std::fmt::Display) -> Self {
        Self::Io(format!("{}: {error}", path.display()))
    }
}

/// A source database's failures: SQLite's own words for a file-system failure stay I/O, anything else is the
/// source's fault (DR `offline_save.rs`).
impl From<diesel::result::Error> for ImportError {
    fn from(error: diesel::result::Error) -> Self {
        let text = error.to_string();
        if ["disk I/O error", "database or disk is full", "unable to open"]
            .iter()
            .any(|io| text.contains(io))
        {
            Self::Io(text)
        } else {
            Self::Invalid(format!("not a readable 1.4.4 database ({text})"))
        }
    }
}

impl From<dainty_store::Error> for ImportError {
    fn from(error: dainty_store::Error) -> Self {
        match error {
            dainty_store::Error::Query(error) => error.into(),
            dainty_store::Error::Connection(error) => Self::Io(error.to_string()),
            dainty_store::Error::Backup(reason) => Self::Io(reason),
            other => Self::Invalid(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, ImportError>;
