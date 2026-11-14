// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Off Windows there are no player host pipes: a native launcher can neither open a Wine pipe nor have the Windows
//! pid the control pipe checks (spec-runtime-r10 17.4), so hosting refuses before any server starts.

use std::time::Duration;

use super::PipeFailure;

/// Never made.
pub(super) struct ReadyListener;

impl ReadyListener {
    pub(super) fn create(_name: &str) -> Result<Self, PipeFailure> {
        Err(PipeFailure::Unsupported)
    }

    pub(super) fn accept(self, _: u32, _: Duration, _: &dyn Fn() -> bool) -> Result<Vec<u8>, PipeFailure> {
        Err(PipeFailure::Unsupported)
    }
}

pub(super) fn fence(_: &str, _: u32, _: &[u8], _: Duration) -> Result<Vec<u8>, PipeFailure> {
    Err(PipeFailure::Unsupported)
}
