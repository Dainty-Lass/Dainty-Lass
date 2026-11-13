# The launcher from a terminal

`dainty-launcher play` and `dainty-launcher export` start the game the way the window's PLAY and the Host page's
"Start the game with CatalogExporter" do, so a host or developer can script them (the catalog exporter diff's runs,
for one). `crates/dainty-launcher-kit/src/controller/cli.rs` holds the commands; the shell's side is
`crates/dainty-launcher-app/src/cli.rs`.

```text
dainty-launcher play   [--server <id>] [--wait]
dainty-launcher export [--server <id>] [--exporter <dll>] [--wait]
dainty-launcher servers
```

| Option | Meaning |
| --- | --- |
| `--server <id>` | Switch to that saved server first, as the Servers page does. It stays the current server. `servers` lists the ids; `*` marks the current one. |
| `--exporter <dll>` | The CatalogExporter build to load. Default: the Host workspace's `CatalogExporter.dll`. A relative path is taken from the working directory. |
| `--wait` | Return when the game exits, with the game's exit code. |

## Design

- **The same controller, without the window.** The command builds the kit's `Controller` with the window's config
  (`controller_config` in the shell's `lib.rs`) over a `HeadlessPlatform` (no dialogs, nothing emitted, the game's
  console hidden as PLAY hides it), runs the window's start-up checks and one connect, then calls `play()` or
  `host_export()`, the methods the bridge commands call. Every check, the relay, the key, the injection, the
  leftover refusal and the clean-up are theirs. There is no second launch path.
- **The relay lives in the launcher process.** So the command runs until the game exits. Without `--wait` the
  launcher starts itself again, detached, with the hidden flag `--held`, prints the child's lines until the game is
  up, and returns 0. The child holds the relay until the game exits, and its log is the launcher's usual log.
- **One launcher at a time.** A command claims the single-launcher channel without handing over any arguments. If the
  window is open it comes to the front and the command fails: `the launcher is already open; close it and run the
  command again`. A window opened while a command runs is turned away, with a line in the log.
- **Output.** English (a host's tool), on stdout. A refusal is `error: <code>: <sentence> (<detail>)`, the
  controller's `ErrorCode` in snake case with its English sentence, and exits 1. Bad arguments print the usage and
  exit 2. The account key never reaches the command line, stdout or the log: the commands only call the
  controller, which keeps the key between the key store, the game's launch argument and the relay.
- **`export`** writes `export_flags.txt` and loads CatalogExporter as the Host page does, prints the DLL's path
  and SHA-256 (also logged), and once the game exits prints `exports: <Win64>\Items_Analysis`, or `no exports at
  ...` when CatalogExporter wrote nothing. Copying the files into the workspace stays the Host page's "Copy exports".
- **Not a `dainty-host` subcommand.** These are the launcher binary's own arguments, so the `clap` tree is untouched.

## Platforms

- **Windows.** Both commands. The release launcher is a window app: PowerShell neither waits for it nor shows its
  output unless the output is piped, so pipe it (`| Out-Host`). A debug build is a console app and needs nothing.
- **Linux.** `play` works as PLAY does there (Proton, with the inject helper or the loader). `export` is refused
  with `host_failed (windows_only)`, as the Host page refuses it. A detached `play` without `--wait` dies with the
  terminal's session (SIGHUP): use `--wait`, or `nohup`.

## PowerShell

```powershell
$launcher = "C:\Program Files\Dainty Lass Launcher\dainty-launcher.exe"   # wherever it is installed
& $launcher servers | Out-Host
& $launcher play --wait | Out-Host
"exit code: $LASTEXITCODE"
& $launcher export --exporter .\CatalogExporter.dll --wait | Out-Host
"exit code: $LASTEXITCODE"
```

If `$LASTEXITCODE` stays empty after a piped run, `Start-Process` gives the code for certain:

```powershell
$run = Start-Process $launcher -ArgumentList 'export','--wait' -NoNewWindow -Wait -PassThru -RedirectStandardOutput export.txt
Get-Content export.txt; "exit code: $($run.ExitCode)"
```

## Hands-on checks (maintainer, Windows PC with the game)

Not run. The tests drive the commands against the controller's fakes only (`tests/controller.rs`,
`the_command_lines_play_is_the_pages` and `the_command_lines_export_is_the_host_pages`).

1. Close the launcher window. `& $launcher servers | Out-Host` lists the saved servers, the current one starred.
2. `& $launcher play --wait | Out-Host`. It prints `server: ...` and `the game started (pid ...)`; the game reaches
   the menu signed in. Quit the game: it prints `the game exited with code 0`, and `$LASTEXITCODE` is 0. The
   launcher's log has the usual PLAY lines, with `-AUTH_PASSWORD=<hidden>`, and no account key appears anywhere in
   the terminal.
3. `& $launcher play | Out-Host` (no `--wait`) returns once the game is up; the game still reaches the menu (the
   detached launcher holds the relay), and that launcher exits with the game.
4. With the launcher window open, `& $launcher play | Out-Host` brings the window forward and prints the
   already-open error.
5. `& $launcher export --exporter <path to CatalogExporter.dll> --wait | Out-Host`. It prints the DLL's SHA-256 (the
   same as `Get-FileHash`). Once CatalogExporter is done, close the game if it stays open; the command prints `exports: ...\Items_Analysis` with
   the files in it.
