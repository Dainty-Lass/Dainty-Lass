//! `dainty-server-minimal`: `serve` with no console and no dashboard (RD26).

fn main() -> std::process::ExitCode {
    host::serve::minimal_main()
}
