use std::process::ExitCode;

use protocol::edition::EditionSelection;

fn main() -> ExitCode {
    host::serve::minimal_main(EditionSelection::Reforged)
}
