<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the module host and its frame protocol (DX2)

The checklist for loading game modules and plugins out of process (DD5): the frame protocol in
`crates/dainty-module-api/src/frame.rs`, the host side in `host.rs`, the module side in `child.rs`, the call plumbing
both share in `peer.rs`, and the checks on what a module declares (`manifest.rs`, `registrar.rs`). The controller
that decides which modules to load and restarts them (DF3) keeps `controller.md`. `SECURITY_REVIEW.md` indexes it.

How to use it: before the first external module ships to hosts, copy this list into the cutover PR, check each item
in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the commit
reviewed. An item that does not hold is a bug to fix before the cutover, not a box to leave open.

## What it is and where the boundary is

A game module or plugin can run as its own executable: the controller starts it, and the two talk over the child's
stdin and stdout in length-prefixed JSON frames. The child is code the host chose to run, but not code the host
compiled: it comes from a module repository's release, or from a plugin such as rustic-freeza. A child that is
broken, out of date or replaced on disk must only ever be refused or killed, never trusted with more than a
compiled-in module gets.

- **In, the executable:** the file `host.toml` names for an external module, with its SHA-256 (DD5). Nothing else
  is ever started: the admin API and the console load and unload only modules `host.toml` lists (DF3).
- **In, the child's stdout:** the hello (protocol, API version, manifest), its calls to the host (emit an event, log a
  line, ask an account's home) and its replies (a registration, command output, admin answers, chat answers, game
  answers: availability, game servers, an invite code).
- **In, the child's stderr:** free text, passed into the host's logs line by line.
- **Out, the child's stdin:** the welcome (node identity), the host's calls (register, start with the module's
  config, stop, status, commands with their actor, admin actions, events, chat queries, game calls) and replies to
  its calls.
- **Out, the process:** arguments from `host.toml`, the environment the controller passes (DF3), and the process
  group (Linux) or creation flags (Windows) the controller kills it by.

What a module never gets, in or out of process: a listener off loopback (its routes go behind the gateway, which
still refuses host-only routes from outside), a raw database connection (DD23: `store`'s write path only), the
account key (DD17) or the admin API's keys. A registration is data the core plugins choose to serve.

Fuzz target: `module_frame` in `restricted/fuzz` (frames in both directions at both size limits, the hello check, a
registration checked against a manifest, game calls and answers with their reason codes, and the names inside: game versions, module ids, global ids, JIDs, event
names, SHA-256 digests). It checks that a length over the limit is refused before the body is read, that whatever
decodes writes back as the same message, and that every name has exactly one spelling.

## Checklist

### Loading

- [ ] `ChildModule::spawn` hashes the whole file and compares it with the pinned SHA-256 before anything is started;
      a mismatch or an unreadable file starts nothing (`LoadError::HashMismatch`, `LoadError::Read`). Test:
      `an_executable_that_does_not_match_its_pin_is_never_started`.
- [ ] Digests are exactly 64 lowercase hex characters (`Sha256Hex`); `host.toml`'s value is parsed, never compared
      as text. Test: `digests_are_exact_lowercase_hex`.
- [ ] The child's hello must name the module `host.toml` listed for that file (`ExternalModule::expected`,
      `Refusal::WrongModule`). Test: `the_child_is_refused_when_it_is_not_the_module_named`.
- [ ] stdin, stdout and stderr are pipes, the child is killed if its handle is dropped (`kill_on_drop`), it runs in
      its own process group on Linux and with `CREATE_NO_WINDOW` on Windows.
- [ ] The file is opened once (`open_pinned`) and the bytes hashed are the bytes started: on Windows the handle
      shares reads only, so nobody can write, rename or delete the file until the child runs; on Linux the child is
      started through the handle (`/proc/self/fd/<n>`), so a file renamed over it is not the one that runs.
- [ ] The child gets only the environment the controller gives it (`ChildOptions::env`, `env_clear`): the host kit
      passes the shell's whole environment, as `start` does (`service_environment`: the clean-room host kit has no
      `SERVER_PREFIXES`/`SERVER_NAMES` filter, so a server setting in the shell reaches the child), and the
      registry's own default is an allowlist of the OS's variables (`system_environment`). Test:
      `children_get_the_os_variables_and_no_server_setting`.
- [ ] The child's whole tree is adopted at spawn (a job object with `KILL_ON_JOB_CLOSE` on Windows, its process
      group on Linux, `tree.rs`) and killed on unload, on a broken pipe, when the child is seen to have exited, and
      when the handle is dropped. Tests: `nothing_is_left_running_after_stop`,
      `a_module_killed_mid_run_shows_failed_and_restarts_on_command` (controller).

### The handshake

- [ ] The host reads nothing before the hello, waits at most `handshake_timeout` (10 s) for it, and reads it with
      the 64 KiB `MAX_HELLO_FRAME` limit. Test: `a_silent_child_times_out_and_a_newer_one_is_refused_with_a_reason`.
- [ ] `check_hello` refuses any frame protocol but `PROTOCOL_VERSION`, any module API the host's `API_VERSION`
      doesn't accept (another major, or a newer minor), a manifest whose `api` differs from the hello's, and a
      manifest that fails `validate`. The refusal is sent to the child with its reason, then the pipe closes. Test:
      `the_handshake_refuses_other_versions_and_modules`.
- [ ] The child refuses a welcome with another frame protocol, and stops on a refusal (`ServeError::Refused`).
- [ ] A second hello, or a welcome after the first, breaks the protocol and ends the connection.

### Frames

- [ ] Every frame's length is checked against the limit (1 MiB after the handshake) before its body is allocated or
      read, on both sides (`check_len`, shared by `decode` and `read_frame`); an empty frame is refused too. Tests:
      `bad_frames_are_refused_before_their_body_is_read`, `streams_end_cleanly_only_between_frames`.
- [ ] Every envelope and message type has `deny_unknown_fields` and closed enums: an unknown message, field or
      variant is a decode error, never ignored.
- [ ] A frame that doesn't decode, a stream that ends inside a frame and a reply to an id never issued each close the
      connection, fail every waiting call with `unavailable`, and kill the child (`read_loop`). Test:
      `a_child_that_breaks_the_protocol_is_dropped`.
- [ ] A reply to a call that timed out is dropped, not taken for a newer call (`Resolved::Late`). Test:
      `a_slow_answer_times_out_and_its_late_reply_is_ignored`.
- [ ] Every host call waits at most its timeout (30 s; start 120 s; stop the stop grace) and is forgotten after it.
- [ ] At most 32 calls from the other side run at once (`MAX_IN_FLIGHT`); past that, reading waits, so a flood backs up
      in the pipe instead of in memory.
- [ ] JSON nesting is bounded by `serde_json`'s recursion limit (128).

### What a module declares

- [ ] `ModuleManifest::validate` bounds every list and string, checks ids and aliases as names, port ranges (from 1,
      ordered, at most 1024 wide, no overlap within a transport), the client executable and ini files as relative
      paths that can't leave the install folder (no root, drive, `\`, `:`, `.` or `..`), Epic manifest sources as
      `https` URLs, and which capabilities fit a game module and which a plugin. Test:
      `validation_catches_what_serde_cannot`.
- [ ] The host checks every registration against the manifest itself (`ModuleDriverExt::register`), compiled in or
      not: route prefixes are absolute paths of unreserved characters with no empty, `.` or `..` segment and name a
      manifest TCP port; UDP ports name manifest UDP ports; content paths and sources are relative paths; command,
      argument and config names follow their rules; nothing is given twice. Test: `bad_entries_are_refused`.
- [ ] A route a module registers never opens a listener: the gateway (DF4a) serves it, refuses `host_only` routes
      from outside, and refuses everything host-only whatever a module says.
- [ ] A plugin extends only the game module of its exact game version, and supplies a patch set only to a module
      that accepts one (`plugin_fits`). Test: `a_plugin_fits_only_its_version_and_a_module_that_accepts_patches`.

### Game calls (MS1)

- [ ] A game answer is data the caller checks, not a decision: `CreateInvite` with `require_up` is refused by the
      module itself, under the same write, when the hub isn't up (`UnavailableReason::error`), so a caller's earlier
      `Availability` is never the guarantee. Test: `the_stub_answers_the_same_in_and_out_of_process` (`invite
      before`).
- [ ] An `UnavailableReason` crosses as one of four codes; any other text in `reason` is a decode error, and
      `from_error` reads a reason only from an `unavailable` error whose message is exactly a code. Test:
      `reasons_are_codes_and_cross_as_errors`.
- [ ] An invite code is a credential: `GameCall` and `GameAnswer` leave it out of `Debug`, which the adapters'
      errors and logs use, and no event carries one. So is `AccountForKey`'s account key (N-1). Test:
      `invite_codes_never_show_in_debug`.
- [ ] A registration's `game_calls` is checked like the rest (nothing given twice) and left off the wire when empty,
      so a module without game calls writes the 1.1 form. Test: `the_stub_registers_cleanly_and_its_defaults_follow`.

### Logs

- [ ] A child's log calls and stderr lines reach the host's logs with control characters replaced and at most 4 KiB
      each (`log_safe`, `forward_stderr`), so a module can't forge log lines. Test:
      `log_lines_lose_control_characters_and_length`.
- [ ] Nothing in a frame is logged whole by the adapters; errors name the problem, not the bytes.

### Unloading

- [ ] `unload` asks the module to stop, closes its stdin (dropping the handle, which is what closes a pipe), waits
      the stop grace and then kills it. Test: `the_stub_answers_the_same_in_and_out_of_process` (ends with an
      unload; the child exits at once).
- [ ] Every call after an unload or a broken pipe fails with `unavailable`; nothing waits forever.

## Open points

These do not block DF2 but must be settled, or accepted in writing, before external modules ship:

- **A write into the file between the hash and the start, on Linux.** The handle closes a swap (rename), not a
  write into the same file by a process allowed to write it; once the child runs, the kernel refuses writes to it
  (`ETXTBSY`). The folder must be writable only by the host's account (hosting.md says so). Copying the checked
  bytes to a private file and starting that would close it, at the cost of the module's own path. Without
  `/proc` (some containers) the path is started, as before.
- **A process the child starts before it is adopted** escapes the job on Windows: the window is between the spawn
  and the next line, before the child's hello.
- **Stdout discipline.** A module that prints to stdout breaks its own pipe and is killed. That is safe, but a
  module author will meet it as a crash; the module side's docs say so.
- **Fuzzing depth.** `module_frame` starts from nothing; a seed corpus of real frames (the stub's hello and a
  registration) would reach the manifest and registration checks sooner.

## Fuzzing

A short run of `module_frame` happens on every PR (CI's `fuzz` job runs every target in `cargo fuzz list`). The run
before external modules ship, from `restricted/fuzz`:

```bash
cargo +nightly fuzz run module_frame -- -max_total_time=86400
```

The first byte picks the size limit (even: 1 MiB, odd: the 64 KiB hello limit); the rest is the stream.

Signed: ________ (date, commit)
