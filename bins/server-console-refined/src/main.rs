use std::process::ExitCode;

use protocol::edition::EditionSelection;

fn main() -> ExitCode {
    host::console_main(EditionSelection::Refined)
}
