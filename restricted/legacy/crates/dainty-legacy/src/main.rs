//! The plugin as an executable. With no arguments it is a module (DD5): stdout is the frame pipe, so nothing here
//! prints. With `import <kind> ...` it runs one import from the command line and prints its report.

use std::process::ExitCode;

use dainty_legacy_import::cli::{self, Kind};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return dainty_module_api::child::main(dainty_legacy::module());
    }
    let (Some("import"), Some(kind)) = (
        args.first().map(String::as_str),
        args.get(1).and_then(|k| Kind::parse(k)),
    ) else {
        eprintln!("{}", cli::USAGE);
        return ExitCode::from(2);
    };
    let result = cli::parse(kind, &args[2..]).and_then(|request| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| dainty_legacy_import::ImportError::Io(format!("the async runtime: {e}")))?;
        // Outside a network a server is its own accounts' home, `local` (as `dainty-host db migrate`)
        let local = "local"
            .parse()
            .map_err(|e| dainty_legacy_import::ImportError::Usage(format!("{e}")))?;
        runtime.block_on(cli::run(&request, &local))
    });
    match result {
        Ok(report) => {
            println!("{}", report.text());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("dainty-legacy: {error}");
            ExitCode::FAILURE
        }
    }
}
