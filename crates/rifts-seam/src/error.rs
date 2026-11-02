// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fmt;

/// Which part of rifts (or the module itself) an answer comes from.
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

/// An operation with no implementation yet; clients see it as a 501.
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

/// A refusal with the code the module's status table maps to an HTTP status.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_texts() {
        let names: Vec<String> = [
            Component::Admission,
            Component::Relay,
            Component::Realms,
            Component::Guard,
            Component::Host,
            Component::Isles,
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            names,
            [
                "rifts-admission",
                "rifts-relay",
                "rifts-realms",
                "rifts-guard",
                "rifts-host",
                "isles"
            ]
        );

        let missing = SeamError::unimplemented(Component::Realms, "realms.join");
        assert!(missing.is_unimplemented());
        assert_eq!(missing.to_string(), "rifts-realms: realms.join is not implemented yet");

        let refused = SeamError::from(Refusal {
            code: "REALMS_DISABLED".into(),
            message: "No.".into(),
        });
        assert!(!refused.is_unimplemented());
        assert_eq!(refused.to_string(), "REALMS_DISABLED: No.");
    }
}
