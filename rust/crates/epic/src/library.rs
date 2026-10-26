//! Whether the signed-in account owns Dauntless (DD22): nothing is downloaded for one that doesn't, even though the
//! CDN would serve it. A stub until DE3, which asks Epic's library through `egs-api` and caches the answer a while.

use crate::NotImplemented;
use crate::auth::Session;

/// Dauntless's app name in Epic's library and manifests.
pub const APP_NAME: &str = "Jackal";

/// Why the library could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LibraryError {
    #[error(transparent)]
    NotImplemented(#[from] NotImplemented),
}

/// Whether [`APP_NAME`] is in the account's library (DE3).
pub async fn owns_dauntless(session: &Session) -> Result<bool, LibraryError> {
    let _ = session;
    Err(NotImplemented { row: "DE3" }.into())
}
