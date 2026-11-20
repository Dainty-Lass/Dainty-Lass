// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! TLS pinned to a certificate's SHA-256, `dainty-relay`'s.

pub use dainty_relay::pinned::*;

#[cfg(test)]
pub(crate) mod tests {
    pub use dainty_relay::pinned::test_tls::{acceptor, certificate};
}
