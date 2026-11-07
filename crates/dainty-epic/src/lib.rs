//! Installing Dauntless from the player's own Epic library (docs/dainty-lass-plan.md DD22), for the launcher and
//! the host kit. DE1 lays it down: `manifest` and `chunk` read Epic's formats; `auth` signs in through `egs-api`
//! (DE2); `library` checks ownership (DE3); `download` installs a version (DE5). `docs/port/spike-de1-epic.md` says
//! why the formats are read here and not by `egs-api`, and `docs/security/epic.md` is the checklist.
//!
//! Nothing here logs. Tokens have no `Debug` or `Display` (`auth::Token`), and a manifest is parsed only once its
//! SHA-1 matches the pin (`manifest::Manifest::parse`). `egs-api` does log, under [`EGS_API_LOG_TARGET`]: every
//! subscriber that could see it drops that target.

pub mod auth;
pub mod builds;
pub mod chunk;
pub mod download;
pub mod library;
pub mod manifest;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

/// The `log` target of `egs-api`'s events. Its failed requests are logged with `{:?}`, so they never reach a log
/// file (docs/security/epic.md, "Signing in").
pub const EGS_API_LOG_TARGET: &str = "egs_api";

/// Whether an event's target is `egs-api`'s, or one of its modules'.
pub fn is_egs_api_target(target: &str) -> bool {
    target
        .strip_prefix(EGS_API_LOG_TARGET)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

/// The row that will fill in a stub, so a caller can say what is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not implemented yet ({row})")]
pub struct NotImplemented {
    /// The plan row that implements it.
    pub row: &'static str,
}

#[cfg(test)]
mod tests {
    #[test]
    fn egs_api_targets_are_recognised() {
        for target in ["egs_api", "egs_api::api", "egs_api::facade::auth"] {
            assert!(super::is_egs_api_target(target), "{target}");
        }
        for target in ["egs_apis", "dainty_launcher", "dainty_epic", "reqwest::connect", ""] {
            assert!(!super::is_egs_api_target(target), "{target}");
        }
    }
}
