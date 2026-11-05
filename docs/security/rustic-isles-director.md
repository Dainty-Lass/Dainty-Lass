<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles Director (RX3)

The checklist for `crates/rustic-isles-director`, the MIT Director of the Rustic Isles module (RD27,
`docs/port/spec-rustic-isles-director.md` §15). `SECURITY_REVIEW.md` indexes it once the integration pass adds the row.

How to use it: before restricted's module switches to this crate, copy this list into the change, check each item in
the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix first, not a box to leave open.

## What it is and where the boundary is

The Director starts, watches and stops the game servers (Ramsgate, the Training Grounds, one process per hunt) on the
module's `director` port (default 61001). It binds 127.0.0.1 only, and no gateway route reaches it. It has no
authentication: anything that can open a loopback connection on the host can ask it to start game servers (bounded
by the hunt cap, the memory guard and the port pool) and read the hub status with pids.

- **Trusted in:** the host's settings (the module's env files and `[switches]`) and the hunt tables in
  `[game_data] director_dir`.
- **Untrusted in:** the matchmaking body (`POST /api/matchmaker/handle-matchmaking-for-player`, or the in-process
  slot), which the backend builds partly from what a player sent (the travel URL's map, `MonsterClass` and `HuntID`,
  the `HuntId`) and partly from server-side ids; the game servers' stdout and stderr; the runtime's occupancy log;
  a CatalogExporter capture (the importer, a host tool).
- **Out:** child processes (`GAMESERVER_BINARY_PATH`), their log files under `GAMESERVER_LOG_DIR`, the importer's
  table files under the vendor folder.

Code: `crates/rustic-isles-director/src/` (`api.rs`, `matchmaker.rs`, `gameservers.rs`, `process.rs`, `launcher.rs`,
`os.rs`, `cmdline.rs`, `config.rs`, `port_pool.rs`, `memory.rs`, `hub.rs`, `occupancy.rs`, `hunt.rs`,
`hunt_tables.rs`).

## Checklist

### What reaches a command line or a file name

- [ ] The program is `GAMESERVER_BINARY_PATH`, never a shell: one `argv` entry per argument on Unix, and on Windows
      a command line the MSVC runtime splits back into the same list, the program quoted whole (`cmdline.rs`,
      fuzz target `rustic_isles_director_cmdline`).
- [ ] A NUL in the program, an argument or the environment refuses the spawn on every platform (`launcher.rs`,
      `OsLauncher::start`; a departure that fixes restricted's cut Windows command line).
- [ ] Argument 1 is the gameserver key; it is never logged and `Command` has no `Debug` (`launcher.rs`).
- [ ] The map (argument 3), Behemoth (4), matchmaker hunt id (5), roster (6) and address (7) are each one argument,
      whatever they contain (fuzz target `rustic_isles_director_matchmaking`).
- [ ] The switches are the version's constants; `GAMESERVER_LOG_CMDS` is used only when it is `[A-Za-z0-9_, ]{1,512}`,
      so a host typo or a quote cannot add a switch (`lib.rs`, `GameserverArgs`).
- [ ] The log file's label has no `/` and stops at the first `.` (`label_for`).
- [ ] `RUSTIC_ISLES_TRIALS_WEEK` is set only through the fixed arena pattern (three ASCII digits), and the launch id
      is a random UUID (`config.rs`, `child_env`; `hunt::arena_week`).

### Process trees

- [ ] Windows: the server is created inside a new job object from its first instruction (the job attribute at
      creation), the job kill-on-close, so the tree ends with the Director's process; only its three standard handles
      are inherited (the handle-list attribute) (`os.rs`).
- [ ] Unix: the server leads a new process group; stop is SIGTERM to the group, kill SIGKILL; on Linux the child gets
      SIGTERM when its starting thread goes (`os.rs`).
- [ ] When the leader exits, what is left of its tree is killed at once (`process.rs`); a stop that the leader ignores
      for 10 s is a kill (`gameservers.rs`, `stop_proc`); shutdown kills what is left after 11 s.
- [ ] The only `unsafe` code is the per-platform module in `os.rs`, under a scoped `#[allow(unsafe_code)]`, with a
      `SAFETY` comment per call.

### Resource bounds

- [ ] The hunt cap (`MAX_CONCURRENT_HUNT_SERVERS`, checked after the stagger and counting hunts still launching), the
      memory guard and the port pool bound what a loopback caller can start.
- [ ] Both pipes are always read to their end (the 2026-07-19 freeze), and a log write failure never blocks them
      (`process.rs`).
- [ ] The readiness scan keeps at most 65 536 bytes without a newline (then the last 4 096), and only this launch's
      well-formed marker counts (`ReadinessScanner`; fuzz target `rustic_isles_director_lines`).
- [ ] The occupancy follower reads at most 4 MiB a poll and keeps a partial line of at most 65 536 chars
      (`occupancy.rs`).
- [ ] A tag query stops at the end of its stream whatever its counts say, and nesting past 64 is an overflow
      (`hunt::evaluate_tag_query`; fuzz target `rustic_isles_director_tag_query`).
- [ ] The body limit is 100 KiB; body errors are Express's pages (`api.rs`).

### Secrets in logs and `Debug`

- [ ] `Config`'s `Debug` shows only the port range, the binary path and `MY_IP` (`config.rs`).
- [ ] `Settings`' `Debug` is names only (core §6.1); no log line carries a setting's value, the arguments or the
      environment.

## Known weak spots (kept for parity, spec §15 and §16 item 16)

Each is a candidate decision row for the maintainer; none is fixed in this port.

- [ ] Player-influenced text (map, `MonsterClass`, `HuntID`, `HuntId`) reaches the game server's command line.
      Quoting keeps each argument whole, but Unreal's parser looks for `-Switch` tokens and `Key=` values across the
      whole line, so a map such as `-ExecCmds=...` or a Behemoth containing `Key=` may be read as an engine switch.
      Whether the backend validates them first is the matchmaking unit's. Proposed: refuse a map that is not
      `/Game/...` and Behemoth or hunt ids outside a path-safe set.
- [ ] The gameserver key is an argument, so any local user can read it from the process list. Proposed: hand it to
      the runtime another way (a runtime patch).
- [ ] Every setting of the merged env files (`backend.env` with `admin.env` and `director.env`: signing keys,
      `API_KEY_HASH_SECRET`, SMTP and Discord secrets, the database URL) and the controller's whole environment reach
      every game server's environment. Proposed: an allowlist of variables.
- [ ] Player-supplied hunt ids and travel arguments are logged as they are (control characters included), and the
      missing-hunt registry grows without bound, one entry per distinct unknown id. Proposed: escape control
      characters, cap the registry.
- [ ] The importer writes `{vendor}/{sourceTable}.json` for whatever `sourceTable` the capture names (a host's own
      file, but a `..` leaves the folder). Proposed: refuse a `sourceTable` that is not a plain name.
- [ ] On Windows the log file name takes the label as is, so a `\` or `:` in a Behemoth path reaches the file name
      (a missing sub-folder fails the start; `:` names an NTFS stream). Proposed: sanitize the label.
- [ ] No authentication on the port: loopback is the boundary.

## Fuzz targets

`fuzz/fuzz_targets/`: `rustic_isles_director_cmdline`, `rustic_isles_director_matchmaking`,
`rustic_isles_director_tag_query`, `rustic_isles_director_lines`, `rustic_isles_director_hunt_import`.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
