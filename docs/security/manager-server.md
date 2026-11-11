<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the manager's embedded server (RX3)

The checklist for `crates/dainty-manager-server`, the Dainty Lass Manager's in-process server
(`docs/port/spec-tauri-shell.md` §4–8, RD26, RD27). It has no fuzz target of its own (T-8): the page's input is
parsed and checked by the launcher kit's `bridge::checks` and `bridge::modules`, which the `launcher_input` target
covers.

How to use it: before the manager's first MIT release, check each item in the code (not from memory), link the code
or test that shows it, and sign at the end with the date and the commit reviewed. An item that does not hold is a bug
to fix before the release, not a box to leave open.

## What it is and where the boundary is

The manager's page calls the bridge commands; the kit checks every argument and hands the result to this crate,
which runs the host kit, the services and the module registry in the manager's process.

- **In:** the page's checked values (module ids, command names and their JSON input, settings JSON, history
  filters, revert plan ids, the server config form, remote management requests). Whoever has the manager's window
  has the server (DH1): every module call and admin action runs as `Actor::Console`.
- **In, from modules:** what a module answers (text, data, failure words), which for an external module is another
  process's output.
- **Out:** the workspace's files (`host.toml`, the env files, `secrets.env`, the databases) and the services the
  host kit starts, which bind as `host.toml` says (only the gateway off loopback).

Code: `crates/dainty-manager-server/src/` (`lib.rs`, `engine.rs`, `modules.rs`, `history.rs`, `version_data.rs`).

## Checklist

### The page's input

- [ ] No call takes a path or a program from the page (DD5): the workspace is the controller's, module ids go
      through `module_id`, and the registry only loads what `host.toml` lists (`modules.rs`).
- [ ] A command runs only when the module registered it, with the arguments `command_args` made from the input;
      settings are saved only after `merge_settings` accepted every value, and a refusal leaves `host.toml` as it was
      (`tests/modules.rs`).
- [ ] A revert applies only a plan this process showed in a dry run, under the engine's write lock, once
      (`history.rs`, `tests/history.rs`).
- [ ] `run_kit` runs only `setup`, `invite`, `multipliers`, `gamedata`, `publish-runtime` and `update`; anything else
      is refused, and unknown flags are dropped rather than passed on (`engine.rs`).

### What reaches the page

- [ ] A module's words are bounded and lose their control characters (`plain`, `output_text`); data longer than
      `OUTPUT_DATA_MAX` is dropped.
- [ ] A module that does not answer in 120 s gives `timeout`; the page is never held.
- [ ] Only the server's own log lines are forwarded (the host, its services, the modules in the snapshot); the
      launcher's own events, which can name the account key's file or the Epic session, never are (`is_server_line`).
- [ ] History rows go out through `redacted_text` and `redacted_json`: an `admin_keys` row never shows its key hash.
- [ ] Invite links and remote enrolment URIs are answers to the page only, never logged.

### The version data

- [ ] The 1.14.7 version data is read only from the manager's own install (beside the executable, or Tauri's
      `../lib/Dainty Lass Manager/` on Linux) and written only to the workspace's `data/1.14.7/version_data.json`;
      a host's own `version_data` in `host.toml` is never overwritten (`version_data.rs`).

### Secrets

- [ ] `secrets.env` is made once by `load_or_create_secrets` and never rewritten from the page; no secret is a field
      of any answer.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
