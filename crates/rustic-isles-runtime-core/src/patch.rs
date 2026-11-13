//! Verified byte patches (R0 5.4): what to do given the bytes at the site.

/// `patch_bytes`'s outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchResult {
    /// The bytes were `expected` and are now the replacement.
    Applied,
    /// The bytes already were the replacement: the other DLL or an earlier call did it (R0 call 8).
    AlreadyApplied,
    /// The bytes were neither.
    Mismatch,
    /// The site is not committed, readable memory inside one region.
    Unreadable,
    /// `VirtualProtect` refused.
    ProtectFailed,
}

/// What the current bytes call for: `Some(Applied)` means "write", anything else is the final answer.
pub fn decide(current: &[u8], expected: &[u8], replacement: &[u8]) -> PatchResult {
    if expected.len() != replacement.len() || current.len() != expected.len() {
        PatchResult::Mismatch
    } else if current == expected {
        PatchResult::Applied
    } else if current == replacement {
        PatchResult::AlreadyApplied
    } else {
        PatchResult::Mismatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions() {
        let expected = [0xBA, 0x60, 0xEA, 0x00, 0x00];
        let replacement = [0xBA, 0x88, 0x13, 0x00, 0x00];
        assert_eq!(decide(&expected, &expected, &replacement), PatchResult::Applied);
        assert_eq!(
            decide(&replacement, &expected, &replacement),
            PatchResult::AlreadyApplied
        );
        assert_eq!(
            decide(&[0xBA, 0, 0, 0, 0], &expected, &replacement),
            PatchResult::Mismatch
        );
        assert_eq!(decide(&expected, &expected, &replacement[..4]), PatchResult::Mismatch);
    }
}
