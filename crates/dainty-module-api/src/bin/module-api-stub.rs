//! The stub module as an executable: what the out-of-process tests start.

use std::process::ExitCode;

fn main() -> ExitCode {
    dainty_module_api::child::main(dainty_module_api::stub::StubModule::new())
}
