use std::fmt;

/// Who answered: a rifts crate, or isles on the other side of [`crate::IslesCore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Component {
    Admission,
    Relay,
    Realms,
    Guard,
    Host,
    Isles,
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Admission => "rifts-admission",
            Self::Relay => "rifts-relay",
            Self::Realms => "rifts-realms",
            Self::Guard => "rifts-guard",
            Self::Host => "rifts-host",
            Self::Isles => "isles",
        })
    }
}

/// What every stub answers (RD18): the operation exists in the surface, its port has not landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{component}: {operation} is not implemented yet")]
pub struct Unimplemented {
    pub component: Component,
    pub operation: &'static str,
}

impl Unimplemented {
    pub const fn new(component: Component, operation: &'static str) -> Self {
        Self { component, operation }
    }
}

/// A launcher error code Node throws as `LauncherApiError` (`REALMS_DISABLED`, `REALM_*`, `GUARD_*`); isles writes
/// the `{error:{code,message,requestId}}` envelope with the status the code is registered under.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Refusal {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SeamError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    #[error(transparent)]
    Refused(#[from] Refusal),
}

impl SeamError {
    pub const fn unimplemented(component: Component, operation: &'static str) -> Self {
        Self::Unimplemented(Unimplemented::new(component, operation))
    }

    pub fn is_unimplemented(&self) -> bool {
        matches!(self, Self::Unimplemented(_))
    }
}
