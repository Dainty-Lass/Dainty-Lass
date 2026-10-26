//! `reforged-host`: the host kit's command line (MIGRATION_PLAN.md RD3, RH rows), the same binary as
//! `reforged-server-console` under its old name.

fn main() -> std::process::ExitCode {
    host::host_main()
}
