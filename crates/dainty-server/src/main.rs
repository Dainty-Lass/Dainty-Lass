// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-server`: the server in one binary, `serve` when no command is given (RD26).

fn main() -> std::process::ExitCode {
    dainty_host::console_main()
}
