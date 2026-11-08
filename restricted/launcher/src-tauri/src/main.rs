// No console window behind the launcher in release builds on Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // The game console's reader is this executable too (dainty-launcher-kit's platform/game_console.rs)
    if let Some(code) = dainty_launcher_kit::platform::game_console::helper_main() {
        return code;
    }
    // dainty-manager, or dainty-launcher: the feature decides, the config names the file
    if let Err(error) = dainty_launcher_app::run(dainty_launcher::options()) {
        eprintln!("Dainty Lass could not start: {error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
