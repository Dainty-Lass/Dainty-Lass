// No console window behind the launcher in release builds on Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // reforged-server-gui, or reforged-client: the feature decides, the config names the file
    #[cfg(feature = "server-gui")]
    let started = launcher::run_server_gui();
    #[cfg(not(feature = "server-gui"))]
    let started = launcher::run_client();
    if let Err(error) = started {
        eprintln!("Dauntless Reforged Revival could not start: {error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
