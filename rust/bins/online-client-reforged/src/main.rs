#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use protocol::edition::EditionSelection;

fn main() -> ExitCode {
    if let Err(error) = launcher::run_client(EditionSelection::Reforged) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
