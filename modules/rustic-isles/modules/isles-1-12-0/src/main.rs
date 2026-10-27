//! The module as an executable (DD5): stdout is the frame pipe, so nothing here prints.

use std::process::ExitCode;

fn main() -> ExitCode {
    module_api::child::main(isles_1_12_0::module())
}
