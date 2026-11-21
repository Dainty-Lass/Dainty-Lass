// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::{error::Error, fmt};

/// Which part of rifts (or the module itself) an answer comes from.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Component {
    Admission,
    Relay,
    Realms,
    Guard,
    Host,
    Isles,
}

impl Component {
    // The name a refusal shows: the crate's, or the module's for `Isles`.
    const fn label(self) -> &'static str {
        match self {
            Self::Isles => "isles",
            Self::Admission => "rifts-admission",
            Self::Guard => "rifts-guard",
            Self::Host => "rifts-host",
            Self::Realms => "rifts-realms",
            Self::Relay => "rifts-relay",
        }
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// An operation with no implementation yet; clients see it as a 501.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Unimplemented {
    pub component: Component,
    pub operation: &'static str,
}

impl Unimplemented {
    pub const fn new(component: Component, operation: &'static str) -> Self {
        Self { component, operation }
    }
}

impl fmt::Display for Unimplemented {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { component, operation } = self;
        write!(f, "{component}: {operation} is not implemented yet")
    }
}

impl Error for Unimplemented {}

/// A refusal with the code the module's status table maps to an HTTP status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub code: String,
    pub message: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl Error for Refusal {}

/// Either kind of answer a hook gives instead of a value; shows and sources as the one it holds.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SeamError {
    Unimplemented(Unimplemented),
    Refused(Refusal),
}

impl SeamError {
    pub const fn unimplemented(component: Component, operation: &'static str) -> Self {
        Self::Unimplemented(Unimplemented::new(component, operation))
    }

    pub fn is_unimplemented(&self) -> bool {
        matches!(self, Self::Unimplemented(_))
    }

    fn inner(&self) -> &(dyn Error + 'static) {
        match self {
            Self::Unimplemented(missing) => missing,
            Self::Refused(refusal) => refusal,
        }
    }
}

impl fmt::Display for SeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.inner(), f)
    }
}

impl Error for SeamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner().source()
    }
}

impl From<Unimplemented> for SeamError {
    fn from(missing: Unimplemented) -> Self {
        Self::Unimplemented(missing)
    }
}

impl From<Refusal> for SeamError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_texts() {
        let expected = [
            (Component::Admission, "rifts-admission"),
            (Component::Relay, "rifts-relay"),
            (Component::Realms, "rifts-realms"),
            (Component::Guard, "rifts-guard"),
            (Component::Host, "rifts-host"),
            (Component::Isles, "isles"),
        ];
        for (component, text) in expected {
            assert_eq!(component.to_string(), text);
        }

        let missing = SeamError::unimplemented(Component::Realms, "realms.join");
        let refused: SeamError = Refusal {
            code: "REALMS_DISABLED".into(),
            message: "No.".into(),
        }
        .into();
        let cases = [
            (&missing, true, "rifts-realms: realms.join is not implemented yet"),
            (&refused, false, "REALMS_DISABLED: No."),
        ];
        for (error, unimplemented, text) in cases {
            assert_eq!(error.is_unimplemented(), unimplemented);
            assert_eq!(error.to_string(), text);
            assert!(error.source().is_none());
        }
    }
}
