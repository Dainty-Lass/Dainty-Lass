//! What a host types at the server's console, and the service output it prints (docs/security/module-host.md).
//! None of it may panic; a name the console acts on is one it can show and pass on as it is, a module command's
//! arguments stay within their caps, and a service's line reaches the terminal with no control character.
#![no_main]

use dainty_hostkit::repl::{
    ConsoleCommand, LEVELS, MAX_ARG, MAX_ARGS, MAX_LINE, is_service_name, parse_command, plain,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|line: &str| {
    match parse_command(line) {
        Ok(command) => {
            assert!(line.len() <= MAX_LINE);
            match command {
                ConsoleCommand::Start(name) | ConsoleCommand::Stop(name) | ConsoleCommand::Restart(name) => {
                    assert!(is_service_name(&name));
                }
                ConsoleCommand::Log { service, level } => {
                    assert!(is_service_name(&service) && LEVELS.contains(&level));
                }
                ConsoleCommand::ModuleRun { module, command, args } => {
                    assert!(!module.is_empty() && !command.is_empty());
                    assert!(args.len() <= MAX_ARGS);
                    assert!(args.iter().all(|a| a.chars().count() <= MAX_ARG));
                }
                _ => {}
            }
        }
        Err(message) => assert!(!message.is_empty()),
    }
    // Service output can echo what a player sent: the host's terminal only ever prints it
    let shown = plain(line);
    assert!(!shown.chars().any(char::is_control), "{shown:?}");
    assert_eq!(plain(&shown), shown);
});
