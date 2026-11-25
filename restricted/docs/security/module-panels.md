<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the server GUI's module panels (RX3)

> **Moved (RL13, RL14):** this code is now the MIT `crates/dainty-launcher-kit` (spec-rd27-launcher.md); the
> `launcher/src-tauri` paths below are the AGPL shell's, deleted in RB15, kept as the record.

The checklist for what the server GUI's page may see of the server's game modules and plugins, and ask of them
(`docs/dainty-lass-plan.md` DU3, DD5, DD11, DD25): `module_list`, `module_get`, `module_run_command`,
`module_set_settings`, `module_load` and `module_unload`. `SECURITY_REVIEW.md` indexes it. The frame protocol and the
loading of external modules are `module-host.md`'s; the Host page's other commands are `host-page.md`'s.

How to use it: before the server GUI's release, copy this list into the review, check each item in the code (not from
memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed. An item that
does not hold is a bug to fix before the release, not a box to leave open.

## What it is and where the boundary is

The server GUI runs the server, and so the controller and its module registry, inside the launcher. The page lists the
modules `settings.toml` names, loads and unloads them, runs the commands they registered and edits the settings they
registered. A module ships no UI (DD11): the page draws every card from the module's manifest and registration, which
for an external module come from its own process (DD5) and are untrusted text.

- **In from the page:** a module id, a command name, the command's form as JSON text and a settings patch as JSON text.
  The isolation hook and the commands check the same length limits (`spec.rs`); the checks below are the backend's own.
- **In from a module:** its manifest, its registration (commands, argument names and help, settings keys and defaults,
  the plugin theme) and a command's output. All of it is bounded by `module-api` when it registers
  (`ModuleManifest::validate`, `Registration::check`) and again, as text for the page, in `bridge/modules.rs`.
- **Out to the page:** `ModuleView`s and a command's text and JSON. The page sets a plugin's palette as CSS variables.
- **Out to disk:** `settings.toml`'s `[modules.<id>.<section>]` tables, written by the host kit's own writer.
- **Who acts:** whoever has the window has the server, as with the console: a command runs as `Actor::Console` (DH1).

Code: `launcher/src-tauri/src/bridge/modules.rs` (the checks and the views),
`server_gui/modules.rs` (the registry behind them), `bridge/commands.rs` (`module_*`), `bridge/spec.rs`.

## Checklist

### Naming a module

- [ ] A module is only ever named by its manifest id (`module_id`: `ModuleId`'s rule, 1 to 64 of `a-z`, `0-9`, `-`). A
      path, a drive, `..` or an upper-case name is `module_unknown` before the registry is asked
      (`only_listed_ids_are_modules`).
- [ ] Only modules `settings.toml` lists exist: the registry answers `module_unknown` for any other id, and no command
  adds
      an entry or takes an executable, a path or a hash (DD5).
- [ ] A load or unload goes through `ModuleRegistry::load` and `unload`, which check the SHA-256 of an external module
      as DD5 says; the page cannot skip that.
- [ ] Loading a loaded module and unloading one that is not are refused (`module_bad_state`), not repeated.

### A command's input and output

- [ ] A command must be one the module registered and that the module is running; the input is JSON text of at most
      8 KiB, an object of the command's own argument names or null (`command_args`). An unknown name, a value that is
      not text, a control character, more than 1024 characters, a missing required argument or a gap before a filled
      one is `module_input_invalid` and nothing is sent (`input_that_does_not_fit_the_command_is_refused_with_why`).
- [ ] The arguments reach the module as positional text, as the console passes them, never through a shell.
- [ ] A command is given up on after 120 s; its answer is cut to 16 KiB of text and 64 KiB of JSON, control
      characters other than line breaks and tabs removed, and shown by the page as text (React escapes it).
- [ ] The failure text a module gives is bounded to 300 characters on one line before it is shown.
- [ ] The command's arguments are not logged; the log says who ran which command on which module.

### Settings

- [ ] A patch is accepted only whole: every section and key must be one the module registered, every value must have
      the type of the key's default (an integer default takes an integer), text at most 4096 characters without a NUL
      (`merge_settings`, `settings_are_checked_against_what_the_module_registered`).
- [ ] A module that has not loaded has registered nothing, so nothing is accepted for it.
- [ ] The patch is merged into what `settings.toml` has and written by the host kit's writer (`save_host_config`), which
      checks the whole file again and keeps the rest of it; a failure leaves the file as it was and the page is told
      `settings_toml` without the file's text.
- [ ] A running module keeps the values it started with; the registry hands the new ones to the next load only
      (`ModuleRegistry::update_settings`).

### What the page draws

- [ ] Every string from a module is plain text in the page (no HTML is built from it) and has control characters
      removed (`plain`).
- [ ] A plugin's palette entry reaches the page only with a name of `a-z`, `0-9` and `-` and a colour that is a hex
      colour or one of `rgb`, `hsl`, `oklch`, `oklab`, `lab` and `lch` with plain numbers in it: no nested function,
      `url(`, `var(`, quote, `;`, `{` or backslash (`is_colour`, `colours_cannot_carry_anything_else`). The page checks
      it again (`safePalette`) and sets it through the CSSOM, which the page's CSP allows; the page has no inline
      style.
- [ ] A plugin's logo is shown as the file name it registered; no image is fetched from a module's text.
- [ ] The "unverified" mark comes from the manifest and is always shown on the card (DD9).

### The build

- [ ] The module commands are served only with the `server-gui` feature; in any other build they answer
      `not_in_this_build` (`module_*` in `commands.rs`) and the page never shows the server screens
      (`snapshot.app.serverGui`).
- [ ] The commands take no arguments the spec does not list, and the generated hook list and permission set were
      regenerated from the spec.

## Open items

- **A settings change reaches a running module only when it restarts.** There is no live reload; the page says so.
- **The registered settings are known once a module has loaded.** The editor is empty for one that has not, since the
  registry keeps the registration only while a module is loaded.
- **No fuzz target for the form and patch parsers.** They sit beside the bridge's types, which need the launcher's
  build (the generated `Language`); `module_frame` covers the registration they are checked against.

Signed: ________ (date, commit)
