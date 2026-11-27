<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the server manager's module panels (RX3)

The checklist for the module panels of the server manager (the `server-gui` build of the Tauri shell): what the page
may see of a module, and what it may ask of one. A module ships no UI; the page draws everything from the module's
manifest and its registration. Written from the MIT code: the checks in
`crates/dainty-launcher-kit/src/bridge/modules.rs`, the registry side in `crates/dainty-manager-server/src/modules.rs`,
the manifest and registration rules in `crates/dainty-module-api` (`manifest.rs`, `registrar.rs`, `ids.rs`), the
command table in `crates/dainty-launcher-kit/src/bridge/spec.rs` and the Tauri commands in
`crates/dainty-launcher-app/src/bridge/commands.rs`.
`SECURITY_REVIEW.md` indexes it.

How to use it: before a release that ships the server manager, copy this list into the release commit, check each
item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed.

## What it is and where the boundary is

- **In, from the page (untrusted as any command's caller):** module ids, command names, a command's JSON input,
  settings JSON, load and unload requests, and moderation requests (`admin_action`). The isolation hook
  (`launcher/isolation/hook.js`, generated from `spec.rs`) checks each command's argument shapes first; the Rust
  side checks again.
- **In, from a module (untrusted when it is an external process):** its manifest, its registration (commands,
  arguments, settings sections and defaults, theme), command answers and error messages.
- **Out:** a module's settings written to the workspace's `settings.toml` (`[modules.<id>.<section>]`), commands and
  admin calls into the controller's `ModuleRegistry`, and the bounded views the page draws.

Nothing here takes a path or a program from the page (DD5): the registry only knows the modules `settings.toml`
lists, and ids are module ids, never paths. The panel exists only in the `server-gui` build; elsewhere
`Backend::panel` is `None` and every module command answers `not_in_this_build`.

## Checklist

### Naming a module

- [ ] Every id the page names goes through `module_id` (`ModuleId::from_str`: 1–64 lower-case ASCII letters, digits
  and `-`, starting with a letter, not ending in `-`); anything else is `ModuleUnknown`, whatever it was
  (`dainty-module-api` `ids.rs` tests, including `"../x"`).
- [ ] An id that parses but is not in the registry is `ModuleUnknown` from `RegistryError::NotListed`; the registry
  never loads anything `settings.toml` did not list.
- [ ] With no server running the registry slot is empty and every call is `ServerStopped`
  (`ServerModules::registry`).
- [ ] A command name is matched against the module's registered `CommandSpec`s only; an unregistered name is
  `ModuleInputInvalid:command`. A module that is not `Ready` is refused (`not_loaded`).

### A command's input and output

- [ ] The hook bounds `module` 64, `command` 64 and `input` 8192 UTF-16 units (`MODULE_ID_MAX`,
  `MODULE_COMMAND_MAX`, `MODULE_JSON_MAX`).
- [ ] `command_args` reads the input as JSON (`null` or an object of strings only), refuses an argument the command
  did not register, a non-string value, a control character, more than 1024 characters (`ARG_MAX`), a missing
  required argument and a gap before a given argument; it yields positional text in the registered order (tests
  `a_form_becomes_positional_arguments`, `input_that_does_not_fit_the_command_is_refused_with_why`).
- [ ] A refusal's detail names an argument only when it is a manifest-style name (`is_name`), so page text is never
  echoed back.
- [ ] Commands run as `Actor::Console` (DH1: whoever has the window has the server) and are logged by module and
  command name, not by argument.
- [ ] Every command and admin call has a deadline (`MODULE_TIMEOUT`, 120 s); past it the answer is dropped and the
  page gets `timeout`.
- [ ] A command's text answer loses every control character but line breaks and tabs and is cut to 16 KiB
  (`output_text`); its data answer is dropped when its JSON is over 64 KiB (`OUTPUT_DATA_MAX`).
- [ ] A module's error message and a registry failure reach the page through `plain` at 300 characters
  (`DETAIL_MAX`), control characters turned to spaces.
- [ ] `admin_action` is checked by `checks::admin_request` behind the hook: a listed action, `player` 1–64 and
  `reason` up to 500 characters with no control characters, `until` and `module` bounded; a named module must be
  `Ready`. Player names reach logs and messages cut to 64 characters (`PLAYER_MAX`).

### Settings

- [ ] `merge_settings` lays the page's JSON over the saved settings only for a section and key the module
  registered, only with the type of the key's default (an integer default takes an integer, a number must be
  finite), and strings at most 4096 characters with no NUL (`VALUE_MAX`, the same bound as
  `dainty-hostkit`'s `hostconfig/modules.rs` `MAX_VALUE`). Nothing is written unless every value fits (test
  `settings_are_checked_against_what_the_module_registered`).
- [ ] Before a module has registered, its schema is empty and every value is refused.
- [ ] Settings are written through `load_host_config` and `save_host_config` on a blocking thread; a missing or
  unreadable `settings.toml` is `ModuleFailed:settings_toml`, with the reason in the host's log only.
- [ ] A running module keeps the settings it started with; `update_settings` applies them at its next load.
- [ ] A registration is checked by `Registration::check` for every module, compiled in or not: snake_case section
  and key names of at most 64 bytes, one-line help, bounded counts of sections, keys, commands and arguments, and
  unique names (`registrar.rs`).

### What the page draws

- [ ] Every module string the page sees passes `plain` with a bound: display name 64, help 256, detail 300, port
  names 64, theme id and name 64, theme layout 64, logo 260 (test
  `module_words_are_bounded_and_lose_their_control_characters`).
- [ ] A setting is shown only when its default is a bool, a number or a string of at most 4096 characters without
  NUL; a saved value of another type than the default's shows the default.
- [ ] A theme's palette keeps only entries whose name is a token (1–32 lower-case letters, digits and `-`, starting
  with a letter) and whose value passes `is_colour`: `#rgb[a]`/`#rrggbb[aa]` or one of eight colour functions with
  plain numbers, at most 64 bytes, so no `url(`, `var(`, quote, `;`, `{` or backslash reaches a CSS variable (tests
  `colours_cannot_carry_anything_else`, `a_plugin_theme_is_drawn_with_the_colours_that_are_safe`). The manifest
  also refuses a theme logo that is not a relative path (`validate_theme`).
- [ ] `unverified` modules (DD9) are flagged in the view so the page can say so.

### The build

- [ ] The module commands are in the `ServerGui` group of `spec.rs`; `Build::current` serves them only with the
  `server-gui` feature, and the permission set and hook list are generated from that table (checked by the
  `generated` test).
- [ ] `ServerModules` is attached only by the embedded server's start and detached on its stop.

## Fuzz targets

`launcher_modules`: `command_args` gives only bounded plain arguments, `merge_settings` merges only registered keys
of their default's type, `is_colour` passes nothing a CSS value could escape with, and `plain` stays plain.
`launcher_bridge` fuzzes the hook shapes of every command in `COMMANDS` (the module commands included) and the
bridge's `checks`; `host_console` fuzzes the console's own `module run` parser.

## Open points

- [ ] **Fixed 2026-11-26:** `launcher_modules` fuzzes `command_args`, `merge_settings`, `is_colour` and `plain`.
- [ ] **The page does not call these commands yet:** `module_*` and `admin_action` are in `launcher/test/api.test.ts`'s
  `NO_PAGE_METHOD`. Review "What the page draws" again against the page when its methods land, in particular how
  it sets palette values and whether it ever renders `logo`.
- [ ] **Fixed 2026-11-26:** `command_args` and `merge_settings` refuse JSON text past `MODULE_JSON_MAX` (8192 UTF-16
  units, the hook's cap) with `too_long`, so the cap holds behind the hook too; `module` and `command` were already
  bounded by the id and registered-name checks.
- [ ] Not a security item: `done_message` and the admin result messages are English made in Rust (marked T-4), against the DU2 rule that
  Rust returns a code. Not a security issue; noted so it is not mistaken for reviewed text.

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
