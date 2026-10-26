use std::process::ExitCode;

use protocol::edition::EditionSelection;

fn main() -> ExitCode {
    controller::run_minimal_cli(EditionSelection::Both)
}
