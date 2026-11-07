// No console window behind the launcher in release builds on Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // The game console's reader is this executable too (launcher-kit's platform/game_console.rs)
    if let Some(code) = launcher_kit::platform::game_console::helper_main() {
        return code;
    }
    // dainty-manager, or dainty-launcher: the feature decides, the config names the file
    if let Err(error) = launcher_app::run(launcher::options()) {
        eprintln!("DaintyLass could not start: {error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
