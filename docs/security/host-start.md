<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the host kit's `setup`, `start`, `serve` and `update` (RX3)

The checklist for how a host's server is configured and brought up: `dainty-host setup` reads `settings.toml` and
`secrets.env` and writes one env file per service, `start` and `serve` compose the services into one process
(RD26), and `update` replaces the release in place and reruns `setup`. The code is `crates/dainty-hostkit`
(`hostconfig.rs` and `hostconfig/`, `settingstoml.rs`, `envfile.rs`, `commands/setup*`, `commands/start/`,
`serve.rs`, `services.rs`, `tls.rs`, `commands/update.rs`, `selfupdate.rs`) and its composition in `crates/dainty-host`.
Written from the MIT code; `SECURITY_REVIEW.md` indexes it.

How to use it: before a release, copy this list into the release commit, check each item in the code (not from
memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed.

## What it is and where the boundary is

- **In, from the host (trusted, but typed by hand):** `settings.toml` (or a `host.json`, `.yaml`, `.yml`), its
  `[env.<service>]` passthrough tables and `[services.<name>]` libraries, `setup`'s options, the Discord bot token and
  OAuth client secret on stdin, and the host's own process environment, which wins over the env files (RD24).
- **In, from disk:** `secrets.env`, the gateway certificate and key in `.workspace/tls`, the env files a previous
  `setup` wrote.
- **In, from the network (`update` only, untrusted):** the release list, the checksums file and the archive, from
  GitHub or the source `DAINTY_SERVER_RELEASE` names.
- **Out:** `secrets.env` and the env files (every secret the services hold), the gateway certificate and key, the
  firewall script and systemd units, the services' child environments, and an installed release.

The services' own listeners are each service's checklist (`gateway.md`, `allowlist.md`, `content.md`,
`admin-api.md`, `controller.md`); this one covers what the host kit tells them.

Fuzz target in `fuzz`: `host_config` (a `settings.toml` through `settingstoml::document` and
`HostConfig`, and env values through `format_value` and `parse_env`: no panic, an unchanged file reads back the same
settings, a value written to an env file reads back as itself on its own line, never as a second variable).

## Checklist

### `settings.toml`

- [ ] A file over `MAX_HOST_CONFIG_BYTES` (256 KiB) is refused before it is parsed (`read_host_config`).
- [ ] An unknown key is refused with its TOML name rather than ignored (`settingstoml::unknown`; test
  `unknown_keys_are_refused_with_the_right_name`).
- [ ] `edition = "legacy"` or `"both"` is refused with `EDITION_REMOVED`: this server no longer runs 1.4.4.
- [ ] A Postgres host off this machine is refused unless `database.allowRemote = true`, since the connection is not
  encrypted (`is_local_database_host`).
- [ ] A `[services.<name>]` library needs a path and a 64-lowercase-hex `sha256`; a name the core composes itself is
  refused, and so is any key besides `library`, `sha256` and `serves` (`hostconfig/services.rs`; test
  `what_services_holds_is_checked`). Loading against that digest is `plugin-host.md`.
- [ ] Rewriting the file patches only changed values (`toml_edit`), so a host's comments stay; a secret is never a
  `settings.toml` field.

### `secrets.env`

- [ ] Every generated secret is 32 bytes from `aws_lc_rs::rand::SystemRandom` as hex; the backend's token key is a
  fresh RSA-4096 pair (`commands/setup/secrets.rs`).
- [ ] The file is made once and kept: a missing or empty key is filled, every present one is left as it is, and a
  key under its old name is renamed with the same value, never a fresh one (`load_or_create_secrets`,
  `RENAMED_SECRET_KEYS`, DX4).
- [ ] The two host-provided secrets (`DISCORD_BOT_TOKEN`, `DAINTY_OAUTH_CLIENT_SECRET`) come only from stdin, are
  checked before anything is written (the token by `BotToken::new`; the client secret one line, at most 512 bytes,
  no whitespace or control character), and are never a command-line value.
- [ ] `secrets.env` and every env file are written through `write_secret_file`: created `0600` on Unix and set to
  `0600` again after the write.
- [ ] **Known gap:** on Windows `write_secret_file` is a plain `std::fs::write`; the files inherit the folder's ACL.
  Decide whether the server folder's ACL is enough.
- [ ] `setup` refuses to run as root on Linux (`NOT_AS_ROOT`), so the secrets are owned by the user that runs the
  server.

### The env files

- [ ] Each file starts with `ENV_FILE_HEADER` (secret, never share or commit; rerun setup instead of editing).
- [ ] A value with CR or LF is refused, so no value can add a second variable; anything beyond the plain set is
  double-quoted with `\` and `"` escaped (`envfile::format_value`, fuzzed by `host_config`).
- [ ] Every service but the gateway is told `127.0.0.1`: `BIND_HOST` for the backend, Director, content and admin,
  `ALLOWLIST_BIND` for the allowlist helper, and every service URL another is given is a loopback URL.
- [ ] The gateway binds `0.0.0.0` with `GATEWAY_TLS=1` and the kit's certificate in public mode, and the host's game
  address, with TLS and the allowlist off, in private mode.
- [ ] A `[env.<service>]` passthrough holds at most 64 keys of `[A-Z][A-Z0-9_]{0,63}` and values of strings, numbers
  or booleans; a secret, a key with an `AUTH_`, `DAINTY_DB_` or `DAINTY_OAUTH_` prefix, a `_SECRET`, `_KEY`, `_KEYS`,
  `_TOKEN`, `_PASSWORD`, `_B64`, `_PORT` or `_BIND` suffix, and the safety keys in `DENIED` (server mode, approved
  executable hash, multipliers, registration mode, bind and port) are refused (`hostconfig/env.rs`; test
  `secrets_ports_and_safety_keys_are_refused`).
- [ ] The passthrough only appends: a key setup already wrote is refused, and `setup` fails while any entry is
  refused (`env::apply`; test `apply_appends_and_never_overrides_what_setup_wrote`).
- [ ] The gateway's, the allowlist's and content's own loaders run on the environment each would start with before
  any file is written, so a bad value stops `setup` (`hostconfig/check.rs`).
- [ ] Stale env files from the 1.4.4 layout are removed.

### The gateway certificate

- [ ] A certificate and key that are both there are kept and never regenerated, since every invite's `fp` is the
  certificate's SHA-256 (`tls.rs`, `commands/setup.rs`).
- [ ] `tls::write_gateway_cert_if_missing` writes the key through `write_secret_file`.
- [ ] **Known gap:** `RealSetupEnv::make_certificate` (`commands/setup.rs`), the path `setup` takes, writes
  `gateway-key.pem` with a plain `std::fs::write`, so on Unix the key gets the umask's mode (usually `0644`), not
  `0600`.
- [ ] The fingerprint is printed after setup and handed to the controller for invites; the key never is.

### `start` and `serve`

- [ ] The preflight fails the start on a gateway port already in use, any failed network, Linux or game-executable
  check, an unfinished game install and a bad store (`commands/start/mod.rs`, `preflight_with`).
- [ ] Missing env files are written from `settings.toml` and `secrets.env` before the start (`ensure_server_files`).
- [ ] A child service starts with `env_clear()` and then exactly the merged environment: the inherited variables, its
  env file over them, and the supervisor's own over both (`services.rs`, `service_environment`).
- [ ] For a setup-written service, a host variable that overrides the file is logged on every start, with the value
  redacted for a secret, an `AUTH_` key, a secret suffix or a safety setting; the generic names (`PORT`, `BIND_HOST`,
  `NODE_ENV`, `LOG_LEVEL`, `MY_IP`, `BACKEND_URL`) are dropped unless prefixed `DAINTY_<SERVICE>_`, and the
  supervisor's own variables are never overridden (`hostconfig/overrides.rs`; tests
  `a_secret_is_overridden_but_never_shown`, `the_supervisors_own_variables_are_never_overridden`).
- [ ] Each child runs with stdin closed and is held in a job object with `KILL_ON_JOB_CLOSE` on Windows, or its own
  process group on Unix, so a stop takes everything it started.
- [ ] Service libraries load with the `sha256` from `[services.<name>]` (`serve::service_libraries`); an external
  module's ports cannot take a reserved service port (`serve::new_registry`, `reserved_ports`).
- [ ] Under the launcher (`DAINTY_CONTROL=stdin`) a `stop` line or EOF stops the server, and `DAINTY_SERVER_READY` is
  printed only after every service started.

### `update`

- [ ] The archive's SHA-256 must match its line in the release's checksums file; on a mismatch, a missing line or a
  failed download nothing downloaded is kept (`selfupdate::fetch`).
- [ ] Build provenance is checked with `gh attestation verify --repo <repository>` (`verify_attestation`).
- [ ] **Known gap:** the attestation is skipped, not failed, when the release has no repository (a URL or folder
  source) or `gh` is not installed. The checksums come from the same place as the archive, so without the
  attestation a hostile source or a man-in-the-middle on an `http://` source can install any build. The source is
  `DAINTY_SERVER_RELEASE` from the host's environment, which accepts `http://`.
- [ ] Extraction refuses an absolute path, a `..` component, a drive prefix, an empty path, a symlink and (in a
  tar.gz) a hard link (`validate_archive_entry_path`, `extract_zip`, `extract_tar_gz`).
- [ ] The install checks every staged file against the release manifest and refuses to overwrite a file the host
  changed (test `install_verifies_staged_files_and_refuses_local_changes`); an interrupted install leaves a marker
  so the next `update` finishes it.
- [ ] The new binary reruns `setup --skip-install` on the host's existing `settings.toml`, so secrets and the
  certificate are kept.

## Open points

- `load_or_create_secrets` fills a missing `AUTH_SIGNING_PRIVKEY_B64` or `AUTH_SIGNING_PUBKEY_B64` on its own, from a
  fresh pair, so a file that lost only one half ends up with a key and a public key that don't match. That breaks
  sign-in rather than weakening it, but setup should make both or neither.
- `settings.toml` is written in place (`write_host_config`), not through a temporary file and a rename; a crash mid
  write leaves a truncated file that the next read refuses.
- The archive's decompressed size is not bounded.

## Sign-off

| Date | Commit reviewed | Reviewer | Notes |
| --- | --- | --- | --- |
| | | | |
