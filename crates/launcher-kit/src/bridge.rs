// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! What crosses the bridge between the page and Rust: the commands, their argument checks and their answers.

pub mod checks;
#[cfg(test)]
mod generate;
pub mod modules;
pub mod progress;
pub mod spec;
pub mod types;
