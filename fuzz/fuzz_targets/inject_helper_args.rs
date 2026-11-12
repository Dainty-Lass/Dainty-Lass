//! The `dainty-inject.exe` helper's command line (docs/security/launcher-injection.md): any arguments parse or are
//! refused without a panic, and what parses names one to four DLLs, each with a lowercase SHA-256, and a program;
//! the launcher's own line reads back as it was built.
#![no_main]

use std::ffi::OsString;
use std::path::PathBuf;

use dainty_inject::helper::{command_line, parse, wine_path};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|args: Vec<String>| {
    if let Some(asked) = parse(args.iter().map(OsString::from)) {
        assert!((1..=4).contains(&asked.dlls.len()));
        for dll in &asked.dlls {
            assert!(
                dll.sha256.len() == 64
                    && dll
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
        }
        let dlls: Vec<(String, String)> = asked
            .dlls
            .iter()
            .filter_map(|d| Some((d.path.to_str()?.to_owned(), d.sha256.clone())))
            .collect();
        if let Some(program) = asked.program.to_str().filter(|_| dlls.len() == asked.dlls.len()) {
            let again = parse(command_line(&dlls, program, &asked.args)).unwrap();
            assert_eq!(again, asked);
        }
    }
    for arg in &args {
        if let Some(wine) = wine_path(&PathBuf::from(arg)) {
            assert!(wine.starts_with("Z:\\") && !wine.contains('/'));
        }
    }
});
