// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 1.14.7 module as a library (spec §9.4). Its id is a constant that parses, so the one construction failure
//! cannot happen; if it did, the panic unwinds inside module-api's open guard and the core sees a failed load, never
//! undefined behaviour (the workspace's `panic = "unwind"`).

#![allow(
    missing_docs,
    reason = "the exported entry symbol is the macro's and carries no doc; an attribute on the invocation is ignored"
)]

fn module() -> rustic_isles::IslesModule {
    match rustic_isles_1_14_7::module() {
        Ok(module) => module,
        Err(error) => panic!("rustic-isles-1-14-7: {error}"),
    }
}

dainty_module_api::export_plugin!(module());
