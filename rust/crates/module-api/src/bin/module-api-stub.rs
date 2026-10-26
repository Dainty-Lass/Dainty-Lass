//! The stub module as an executable: what the out-of-process tests start.

use std::process::ExitCode;

fn main() -> ExitCode {
    module_api::child::main(module_api::stub::StubModule::new())
}
