# The launcher's oracle tests (`dainty-launcher`, test-only)

Dainty Lass Launcher and Dainty Lass Manager are the root's MIT Tauri shell, `crates/dainty-launcher-app`, over
`crates/dainty-launcher-kit`, with the manager's embedded server in `crates/dainty-manager-server`. The release
(`launcher-release.yml`) and CI's installers build them there, and nothing here is built into an app or shipped.

What stays is the oracle (spec-tauri-shell §12.1), AGPL because it holds DR's recorded tables:

- `src-tauri/tests`: the tests that hold dainty-launcher-kit to DR's tables (`test/cases`) and drive its controller,
  relay and modules, and the oracle for `dainty-manager-server` (`manager_server.rs`, `controller.rs`'s `server-gui`
  cases). The package has no library and one binary, `fake-host-kit`, the kit's stand-in for `dainty-host` that the
  Host page's tests start.
- `test/electron-keystore`: the Electron key-store harness (`keystore.rs`'s `an_electron_profile_reads_back`).
- `src-tauri/icons` and `brand/`: today's icons (L2), which the MIT shell's placeholders stand in for until new ones
  are drawn. No build reads them.

The bridge's command table and generated files are the kit's (`crates/dainty-launcher-kit/generated`;
`RR_UPDATE_GENERATED=1 cargo test -p dainty-launcher-kit generated` at the root), and `bun run restricted-data` in
`ui/` and `launcher/` copies them from there.

## Running the tests

From `restricted/` (`-j 6` or fewer on the maintainer's PC):

```bash
cargo test -p dainty-launcher -j 6
cargo test -p dainty-launcher -j 6 --features server-gui,realms
```

CI's restricted Rust family runs the first with the rest of the workspace, and the second on Linux.
