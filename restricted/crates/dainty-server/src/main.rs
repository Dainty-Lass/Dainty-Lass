//! `dainty-server`: the server in one binary, `serve` when no command is given (RD26).

fn main() -> std::process::ExitCode {
    dainty_host::console_main()
}
