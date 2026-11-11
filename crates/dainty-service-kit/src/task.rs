// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Named tasks, so tokio-console groups a module's work under its name (docs/tokio-console.md).

use std::future::Future;

use tokio::task::JoinHandle;

#[cfg(all(feature = "tokio-console", not(tokio_unstable)))]
compile_error!("the tokio-console feature needs RUSTFLAGS=\"--cfg tokio_unstable\" (docs/tokio-console.md)");

/// Spawns `future` on the current runtime as `tokio::spawn` does, named `name` where tokio can name tasks.
pub fn spawn_named<F>(name: &str, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    #[cfg(tokio_unstable)]
    {
        // Never Err today; an error would leave nothing to fall back to, the future being moved
        match tokio::task::Builder::new().name(name).spawn(future) {
            Ok(handle) => handle,
            Err(_) => unreachable!(),
        }
    }
    #[cfg(not(tokio_unstable))]
    {
        _ = name;
        tokio::spawn(future)
    }
}
