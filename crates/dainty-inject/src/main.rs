// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `dainty-inject.exe`: Proton runs it in place of the game (`dainty_inject::helper`).

fn main() {
    let code = dainty_inject::helper::run(std::env::args_os().skip(1));
    std::process::exit(code.cast_signed());
}
