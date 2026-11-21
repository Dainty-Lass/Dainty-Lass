// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host`: the host kit's command line (MIGRATION_PLAN.md RD3, RH rows), the same binary as
//! `dainty-server` under the name installs run (DD1).

fn main() -> std::process::ExitCode {
    dainty_host::host_main()
}
