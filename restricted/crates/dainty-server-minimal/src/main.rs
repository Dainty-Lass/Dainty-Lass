//! `dainty-server-minimal`: `serve` with no console and no dashboard (RD26).

fn main() -> std::process::ExitCode {
    dainty_host::serve::minimal_main()
}
