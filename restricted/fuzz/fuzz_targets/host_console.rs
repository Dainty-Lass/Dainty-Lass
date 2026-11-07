//! A line typed at the server's console or the dashboard's `:` prompt (docs/security/controller.md).
//! The parser never panics, a service it names is a plain name, a level is one of the console's, and
//! what it says back never carries a control character typed into it. A service's line, as the console
//! and the dashboard print it, carries none either.
#![no_main]

use dainty_host::repl::{ConsoleCommand, LEVELS, MAX_LINE, is_service_name, parse_command, plain};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let line = String::from_utf8_lossy(data);
    let shown = plain(&line);
    assert!(!shown.chars().any(char::is_control) && shown.len() <= line.len());
    match parse_command(&line) {
        Ok(ConsoleCommand::Start(name) | ConsoleCommand::Stop(name) | ConsoleCommand::Restart(name)) => {
            assert!(is_service_name(&name));
        }
        Ok(ConsoleCommand::Log { service, level }) => {
            assert!(is_service_name(&service) && LEVELS.contains(&level));
        }
        Ok(
            ConsoleCommand::ModuleStatus(name)
            | ConsoleCommand::ModuleLoad(name)
            | ConsoleCommand::ModuleUnload(name)
            | ConsoleCommand::ModuleRestart(name),
        ) => assert!(is_service_name(&name)),
        Ok(ConsoleCommand::ModuleRun { module, command, args }) => {
            assert!(is_service_name(&module) && is_service_name(&command));
            assert!(args.iter().all(|arg| !arg.chars().any(char::is_control)));
        }
        Ok(_) => assert!(line.len() <= MAX_LINE),
        Err(said) => assert!(!said.chars().any(char::is_control) && said.len() <= 200),
    }
});
