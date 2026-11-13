<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R1, config (RD27, plan item 11)

R1 is the runtime's configuration: every value the game runtime takes from its environment, its command line and
the number files next to the executable, parsed into typed values the other slices read. It is pure (a module of
`rustic-isles-runtime-core`, tested on every platform) plus a few thin Windows readers in `rustic-isles-runtime`.
It installs no hook and changes no game behaviour: it is inert until R2 (backend URL), R8 (pacing, hub cap, AFK), R9
(launch arguments, map URL) and R18 (trials) consume it.

An implementer builds from this page, `spec-runtime-r0.md` and `spec-runtime-surface.md` alone, never from
`restricted/`. Exact texts (refusal reasons, log lines) and exact grammars below are facts the operator and the tests
see. **(call)** marks a decision made here, **(departure)** a deliberate difference from the AGPL runtime, both
collected in section 12. Conventions are the surface page's.

---

## 1. Scope

| R1 owns | Section | Consumed by |
| --- | --- | --- |
| The `MYSTICPARADOX_` to `RUSTIC_ISLES_` environment alias | 3 | R0's Init, before every other read |
| The backend address: variables, grammar, refusal reasons, the fallback, the log line | 4 | R2 (HTTP-1), R0's status lines |
| The server auth mode | 5 | R9, R10, R2 |
| The server launch arguments, full and manual | 6 | R0 (port, for the log), R9, R8, R2 (the gameserver key) |
| The expected-player roster count and the map URL | 7 | R8, R9 (SRV-1) |
| The frame-pacing settings (patch 0020) | 8 | R8 |
| The trials rotation and week | 9 | R18 (ESC-1) |
| The hub cap and AFK timeouts (files and, per RT-11, the environment) | 10 | R8, R20 |

R1 owns no flag file and no hook id. It lands with R0 (R0 section 1).

**Where the code goes.** `rustic-isles-runtime-core/src/config/` holds every parser as a function of its inputs (text,
a list of arguments, a clock value), returning a typed result. `rustic-isles-runtime/src/config/` holds the Windows
readers: `GetEnvironmentStringsW`/`SetEnvironmentVariableW` for the alias, `GetEnvironmentVariableW` (through R0's
helpers), `GetCommandLineW` and `CommandLineToArgvW` (shell32), and a bounded file read next to the executable. Every
value is read at most once per process and kept in a `OnceLock` (or written once in Init before any reader runs).

---

## 2. Reading rules shared by every section

- **Environment.** R0's `env_value(name, max)` (unset, empty, over `max`, NUL/CR/LF or non-ASCII give `None`) unless a
  section says it reads **raw**: `env_raw(name, cap) -> Raw` = `Unset` (absent or empty), `TooLong(len)` (more than
  `cap` UTF-16 units; `len` is the value's length in units) or `Value(String)` (lossless from UTF-16; an unpaired
  surrogate becomes U+FFFD, which every R1 grammar then refuses).
- **Decimal (`strtol` rule).** Where a section says "a `strtol` integer", the text must be: optional ASCII whitespace
  (space, `\t`, `\n`, `\v`, `\f`, `\r`), an optional `+` or `-`, one or more ASCII digits, **and nothing after**. The
  value is computed in `i64`; a value outside `i32` is out of range for every setting here. (The AGPL used C's
  `strtol` with a 32-bit `long`; the outcomes agree for every in-range value, and every out-of-range one is refused by
  each setting's range anyway.)
- **Decimal (stream rule).** Where a section says "a stream integer" (the number files): skip leading ASCII whitespace,
  then an optional sign and one or more digits; **stop at the first non-digit and ignore the rest of the file**. No
  digits, or a value outside the type's range (`i32` or `i64`, named per section), is "unreadable". A UTF-8 BOM is not
  whitespace, so a file that starts with one is unreadable (as in the AGPL).

---

## 3. The `MYSTICPARADOX_` alias

Runs first in R0's Init, before any other environment read in this DLL, so every later read sees the new names. (The
AGPL runtime runs the same pass after it; with this one done first, it finds nothing to do.)

1. Take the process environment block (`GetEnvironmentStringsW`): entries `NAME=VALUE`, in block order.
2. For each entry whose text starts with `MYSTICPARADOX_` (**case-sensitive**) and contains a `=` after the prefix:
   `suffix` = the text between the prefix and the first `=`; `value` = the text after that `=`. (Entries such as the
   `=C:=C:\...` drive entries never start with the prefix.)
3. The new name is `RUSTIC_ISLES_` + `suffix`. If no variable of that name **exists** (`GetEnvironmentVariableW(name,
   NULL, 0)` returns 0; an existing empty variable counts as existing), plan to set it to `value`.
4. After walking the whole block (never while walking it), set each planned variable. Free the block.

Pure part: `alias_plan(entries: &[&str], exists: impl Fn(&str) -> bool) -> Vec<(String, String)>`.

This alias is for launchers and Directors built before the Rustic Isles rename. RT-8 renames the remaining Mystic names
in one joint cutover with the launcher and the Director (slice R10); that cutover deletes this section.

---

## 4. The backend address (infra; realms S1; patch 0040)

### 4.1 Variables

Read **raw** with a cap of 511 units, in this order: `RUSTIC_ISLES_BACKEND_URL`, then `DAINTY_BACKEND_URL`. The
**first one that is set (non-empty) is the only one used**; the second is never consulted when the first is set, even
if the first is refused (fail closed, as the AGPL). Both unset: the built-in address (4.4).

- `TooLong(len)`: refused with reason `value too long`, length `len`.
- `Value(text)`: parsed by 4.2. Refused: its reason, length = `text`'s length in characters.

Resolved once, in R0's Init (before any hook that builds a backend URL), and readable from any thread afterwards.

### 4.2 Grammar: `parse_backend_url(text) -> Result<BackendUrl, Reason>`

Accepted, exactly: `scheme://host[:port]`. The checks run **in this order**, and the first that fails gives the
reason; the reason texts are fixed and **never echo the input** (a refused value may carry credentials):

| # | Check | Reason |
| --- | --- | --- |
| 1 | `text` is empty | `empty value` |
| 2 | more than 300 characters | `value too long` |
| 3 | any character is U+0000–U+0020 or U+007F or above (space, control, non-ASCII) | `contains a space, control or non-ASCII character` |
| 4 | no `://` in `text` | `missing scheme://` |
| 5 | the text before the **first** `://`, lower-cased, is neither `http` nor `https` | `scheme must be http or https` |
| 6 | the authority (everything after that `://`) is empty | `missing host` |
| 7 | the authority contains `@` | `user information is not allowed` |
| 8 | the authority contains any of `/` `?` `#` `\` | `path, query and fragment are not allowed` |

Then, if the authority starts with `[` (an IPv6 literal):

| # | Check | Reason |
| --- | --- | --- |
| 9 | no `]` in the authority | `unterminated [IPv6] literal` |
| 10 | the text between `[` and the first `]` is not an IPv6 address (4.3) | `malformed IPv6 address` |
| 11 | text follows the `]` and does not start with `:` | `unexpected text after the IPv6 literal` |

The host is `[` + the inner text lower-cased + `]`; a `:` after `]` starts the port (the rest of the authority).

Otherwise:

| # | Check | Reason |
| --- | --- | --- |
| 12 | the authority holds two or more `:` | `more than one ':' outside [brackets]` |
| 13 | the host (the authority up to its `:`, or all of it) is empty | `missing host` |
| 14 | the host holds only digits and dots, and is not a dotted quad (4.3) | `malformed IPv4 address` |
| 15 | otherwise the host is not a DNS name (4.3) | `malformed host name` |

The host is lower-cased. A `:` gives the port text (everything after it).

Port, when a `:` was present:

| # | Check | Reason |
| --- | --- | --- |
| 16 | the port text is empty, longer than 5, starts with `0`, or has a non-digit | `port must be 1-65535` |
| 17 | its value is 0 or above 65535 | `port must be 1-65535` |

No `:`: the port is 443 for `https`, 80 for `http`.

`BackendUrl { secure: bool (https), host: String, port: u16 }`.

### 4.3 Host forms

- **Dotted quad:** exactly four parts separated by `.`; each part 1 to 3 digits, value at most 255, no leading zero
  (`0` alone is fine, `01` is not). `1.2.3`, `1.2.3.4.5`, `256.0.0.1`, `1..2.3` are refused.
- **DNS name:** total length 1 to 253; labels separated by `.`; no empty label (no leading dot, no `..`, **no trailing
  dot**); a label is 1 to 63 characters of ASCII letters, digits and `-`, and neither starts nor ends with `-`; the
  **last label is not all digits** (`example.123` is refused; `123.example` is fine). Checked before lower-casing;
  letters of either case are allowed.
- **IPv6** (the inner text, no brackets, no zone id): length 2 to 45; only hex digits, `:` and `.`.
  - No `::`: split on `:`; every token non-empty; a token with a `.` must be the **last** token and a dotted quad, and
    counts as two groups; any other token is 1 to 4 hex digits and counts as one; the total is exactly **8**.
  - One `::` (a second `::` anywhere after the first, which includes `:::`, is refused): the sides before and after it
    are counted the same way, an empty side being zero groups; a dotted quad is allowed only as the last token of the
    **right** side; left + right is at most **7**. So `::`, `::1`, `1::`, `fe80::1`, `::ffff:1.2.3.4` pass;
    `1:2:3:4:5:6:7:8:9`, `1::2::3`, `:1::`, `1.2.3.4::` and `12345::` are refused.

### 4.4 Results, the fallback, the log line

- **API base** (`api_base()`), what R2's redirect prepends `/__origin/<host><path>` to: `http://` or `https://`, the
  host, and `:<port>` **only when the port is not the scheme's default** (`https://h:443` gives `https://h`).
- **Chat address** (`chat_address()`): `ws://` or `wss://` matching the scheme, the host, **no port and no path**; the
  port travels separately (`chat_port()`). (A client that appends `":" + port` to the address would otherwise produce
  two ports.)
- **Built-in (fallback):** used when both variables are unset or the one used is refused. API base
  `https://backend.invalid`, chat `wss://backend.invalid`, port 443. **(call)** The MIT runtime has no deployment
  header: the built-in host is always `backend.invalid`, a reserved name that never resolves, so a game started without
  an address fails closed instead of reaching a stranger.
- **Log line** (R0 writes it with its first status lines, tagged `[MitRuntime]`; R9 may write it again):
  `[BackendAddress] source=env|built-in api=<api base> ws=<chat address>:<chat port>`, and when a value was refused,
  ` (<VARIABLE> ignored: <reason>, length=<n>)` appended, `<VARIABLE>` being the variable actually read. **(departure)**
  The AGPL always names `RUSTIC_ISLES_BACKEND_URL` there and logs the API's "needed size" (length + 1) for a value of
  512 or more; the MIT runtime names the variable and logs the length.

**RT-15 settled.** The host kit writes both `DAINTY_BACKEND_URL` and `RUSTIC_ISLES_BACKEND_URL`
(`http://127.0.0.1:<backend port>`) into the Director's env file (`crates/dainty-hostkit/src/hostconfig.rs`), and the
Director hands its whole environment to every game server it starts (`rustic-isles-director` `Config::child_env`). So
every DeployServer gets an address; the runtime's older note that a DeployServer sets none is stale. A server started
by hand without either variable fails closed on every redirected request, as intended.

---

## 5. The server auth mode

Server role only (R0 3.2). `RUSTIC_ISLES_AUTH_MODE`, read **raw** with a cap of 32:

| Read | Mode |
| --- | --- |
| `Unset` | `GlobalGameserver` (a DeployServer gameserver) |
| `Value("GLOBAL_GAMESERVER")` | `GlobalGameserver` |
| `Value("HUNT_CAPABILITY")` | `PlayerHost` |
| any other `Value`, or `TooLong` | `Invalid` |

Exact, case-sensitive matches. **(departure)** The AGPL treats a value it cannot read (over 32, or holding CR, LF or
NUL) as unset, so as `GlobalGameserver`; the MIT runtime treats it as `Invalid` (fail closed). `Invalid` ends the server
before it listens with exit code `0xE102` (R9 owns that step). Whether `PlayerHost` is honoured depends on R10 (the
player host); R1 only classifies.

---

## 6. Server launch arguments (surface 2.4)

The arguments are `CommandLineToArgvW(GetCommandLineW())`, kept as UTF-16 (`Vec<u16>` each) in storage that lives for
the process: every value is **copied** (a pointer into the argv block crashed the AGPL late in the map-URL hook). The
pure parser takes the list and the narrow command line (for the manual mode, 6.2).

### 6.1 Full launch: more than 8 entries

Positional (entry 0 is the program):

| Entry | Field | Use |
| --- | --- | --- |
| 1 | `api_key` | the gameserver key, kept only in `GlobalGameserver` mode (R2 sends it as a header). In `PlayerHost` mode R1 keeps the raw entry for R10, which requires it to be `-`, and `api_key` is `None` |
| 2 | `port` | a decimal port (below) |
| 3 | `map_path` | verbatim |
| 4 | `behemoth` | a class path, or any text containing `NO_BEHEMOTH` |
| 5 | `matchmaker_hunt_id` | an id, or text containing `NO_MM_HUNTID` |
| 6 | `expected_players` | `accountId:hunterId, accountId:hunterId...` or text containing `NO_EXPECTED_PLAYERS` |
| 7 | `own_address` | the server's own `ip:port`, verbatim |

Entries from 8 on are ignored. Derived: `watchdog = true`, `logging = true`.

**Port (departure).** The AGPL parses entry 2 with C++'s `stoi` (leading whitespace and a sign accepted, stops at the
first non-digit, and **throws** on no digits, which ends the process inside the loader lock). The MIT parser requires 1
to 5 ASCII digits, no leading zero, value 1 to 65535. Anything else is a launch error: R0's Init ends the process with
exit code **`0xE106`** before installing anything, and nothing is logged (the loader lock). **(call)** The Director and
the launcher always pass a plain decimal port, so no real launch changes.

### 6.2 Manual test mode: 8 or fewer entries

- `port`: if the **narrow** command line (`GetCommandLineA`, or the UTF-16 one narrowed) contains `Port=`
  (case-sensitive, anywhere, first occurrence), the digits immediately after it, 1 to 5 of them, value 1 to 65535;
  otherwise, or when they are not a valid port, **8777**. **(departure)** The AGPL's `stoi` there accepted leading
  whitespace and a sign and threw on no digits; the MIT falls back to 8777 instead.
- `map_path`: argv entry 1 when it exists and does not start with `-`, cut at its first `?`; otherwise `Ramsgate`.
  **(departure)** The AGPL scans the raw command line from its first space or tab (which a quoted program path
  containing a space breaks) and ignores a map name that runs to the end of the line. The MIT uses the parsed argv
  entry.
- `api_key`, `behemoth`, `matchmaker_hunt_id`, `expected_players`, `own_address`: `None`.
- `watchdog = false`, `logging = true`.

### 6.3 The result

`Launch { mode: Full | Manual, api_key, port: u16, map_path, behemoth, matchmaker_hunt_id, expected_players,
own_address, raw_entry_1, watchdog: bool }`. On a client, R1 parses nothing and the port is 0. Never logged: the
command line, `api_key`, `raw_entry_1`. The map path and the port may be logged.

---

## 7. The roster count and the map URL

### 7.1 Expected players

`expected_player_count(expected_players, hub_cap) -> u32`:

- `None`, empty, or containing `NO_EXPECTED_PLAYERS` anywhere: **0**.
- Otherwise split on `,`. Walk the entries from the start; stop after the last one, and a trailing `,` ends the walk
  without an empty entry (`a:b,` counts 1). Each entry must hold a `:` (its first one) that is neither its first
  character nor its last; **any entry that fails makes the whole roster 0** (so does an empty entry between two commas).
  Entries are not trimmed: `a:b, c:d` counts 2.
- The count, clamped to `0..=hub_cap` (10).

### 7.2 The map URL (SRV-1's text; R9 writes it into the engine)

`map_url(launch, hub_cap, listen: bool) -> UTF-16 string`, the concatenation of:

1. `map_path` (`Ramsgate` in manual mode).
2. `?MonsterClass=` + `behemoth`, unless `behemoth` is `None` or contains `NO_BEHEMOTH`.
3. `?HuntId=` + `matchmaker_hunt_id`, unless `None` or containing `NO_MM_HUNTID`.
4. `?PlayerHuntIds=` + `expected_players`, unless `None` or containing `NO_EXPECTED_PLAYERS`.
5. `?MaxPlayers=` + `hub_cap` in decimal, **always** (without it the game defaults its player cap to 1).
6. `?listen` when `listen` is true (R9 decides: `MAP_URL_LISTEN.flag`, never on a player host).

Values go in verbatim, unescaped. **(ambiguity)** In manual mode the AGPL builds this string from absent fields (an
undefined read); the MIT treats them as absent (rules 2 to 4 skip them).

---

## 8. Frame pacing settings (patch 0020)

Two variables, read raw once (at the first server tick, by R8), each with a buffer of 15 characters:

| Variable | Default | Accepted | 0 means |
| --- | --- | --- | --- |
| `DAINTY_SERVER_MAX_FPS` | 30 | 0, or 10 to 240 | no cap |
| `DAINTY_SERVER_IDLE_FPS` | 0 | 0, or 1 to 240 | the same as the active cap |

`fps_setting(name, raw, default, min, max) -> (i32, Option<String>)`:

- `Unset`: the default, no note.
- `TooLong` (16 or more characters): the default, note `" <NAME> too long (default used)"`.
- `Value(text)`: a `strtol` integer (2) that is 0 or within `min..=max`: that value, no note. Anything else: the
  default, note `" <NAME>='<text>' invalid (default used)"`.

Notes are appended in the order MAX, IDLE to R8's line
`[FramePace] server frame cap: <N fps|off>, idle (no client for 15 s): <N fps|same> (DAINTY_SERVER_MAX_FPS /
DAINTY_SERVER_IDLE_FPS)<notes>`, where the active part is `off` for 0 and the idle part `same` for 0. (The value is
echoed in the note; it is a number setting, never a secret.)

---

## 9. Trials rotation and week (R18's inputs)

- `TRIALS_ROTATION_MINUTES`: raw, 31-character buffer; a `strtol` integer from 1 to 525 600; else, or unset, or too
  long, **10 080** (seven days). Read once.
- `RUSTIC_ISLES_TRIALS_WEEK`: raw, 15-character buffer; a `strtol` integer from 1 to 181; else `None`. The Director
  sets it for an arena hunt, so a trial stays on the week it launched with.
- **Rows:** 181, named `Scheduled_Arena_Hunt_NNN` with `NNN` the week zero-padded to three digits (`..._001` to
  `..._181`). `parse_week_row(name)`: exactly the prefix plus three ASCII digits, value 1 to 181; anything else none.
- **Clock:** `now` in .NET ticks (100 ns since 0001-01-01 UTC) = the Windows `FILETIME` + 504 911 232 000 000 000.
  The Unix epoch is 621 355 968 000 000 000 ticks; a rotation is `minutes × 600 000 000` ticks.
  `bucket = (now − unix_epoch) / rotation` (integer division), `current_week = bucket % 181 + 1`,
  `bucket_start = unix_epoch + bucket × rotation`. The pure function takes `now` and the minutes.

The Director computes the same week for the hunts it launches (`rustic-isles-director`, `hunt` module); the vectors in
11 pin both to the same answers.

---

## 10. Hub cap and AFK timeouts (RT-11)

### 10.1 Where the values come from

RT-11 makes the number files `host.toml` settings passed in the environment. **(call)** R1 defines the runtime side;
until the host kit writes the variables, the files are still read, so nothing changes for a host mid-transition. For
each value the first source that gives one wins:

1. The player host's launcher override (R10's environment, already validated there): `RUSTIC_ISLES_MAX_PLAYERS`
   (1 to 64) for the cap; `RUSTIC_ISLES_HUB_AFK_SECONDS` (0 to 7200) for the hub timeout. Only in `PlayerHost` mode.
2. The new variables **(call: names)**: `DAINTY_HUB_MAX_PLAYERS`, `DAINTY_HUB_AFK_SECONDS`, `DAINTY_HUNT_AFK_SECONDS`
   (read with R0's `env_value`, max 16; the text is a `strtol` integer).
3. The files next to the executable: `HUB_MAX_PLAYERS.txt`, `HUB_AFK_TIMEOUT_SECONDS.txt`,
   `HUNT_AFK_TIMEOUT_SECONDS.txt` (read at most 64 bytes **(call)**; a stream integer, 2).
4. The default.

Each value is read once (the cap at the first map URL, the timeouts at listen, as R8 says).

### 10.2 Rules

| Value | Type | Rule | Default |
| --- | --- | --- | --- |
| Hub cap | `i32` | 1 to 64 used; anything else (or unreadable) falls to the next source | 4 (a Dauntless party) |
| Hub AFK seconds | `i64` | negative: **native** (no override); 0: never kick (R8 warns); 1 to 59: 60; above 604 800: 604 800; else the value. Unreadable falls to the next source | 1200 |
| Hunt AFK seconds | `i64` | the same | native |
| Launcher hub AFK (source 1) | from R10 | 0: never kick; 1 to 59: 60; else the value | — |

`AfkSetting { seconds: Option<i64> /* None = native */, source: &'static str }` with the source texts R8 logs:
`default`, `file`, `file (negative = native)`, `file (0 = never kick)`, `file (clamped to 60)`,
`file (clamped to 604800)`, `file unreadable -> default`, and the same with `env` in place of `file` for source 2, and
`launcher role`, `launcher role (0 = never kick)`, `launcher role (clamped to 60)` for source 1.
**(departure)** In the AGPL an unreadable file is the end (the default, `file unreadable -> default`); here an
unreadable environment value falls through to the file, then the default, with that source text.

### 10.3 What the other rows need (not R1)

- The host kit: `host.toml` fields `hub_max_players`, `hub_afk_seconds`, `hunt_afk_seconds` (in `HostConfig` and the
  key table, default unset = today's behaviour), written by `setup` into the Director's env file as the three
  `DAINTY_` names; a round-trip test. The Director already forwards its environment.
- The docs: the settings table in `restricted/docs/hosting.md` (and the Linux and Docker pages where a host sets it).
- Patches 0024 and 0029 (also RT-11) are R15's to name.
- A later row removes source 3 once the release that writes the variables is out.

---

## 11. Tests (all in CI: `cargo test -p rustic-isles-runtime-core`)

- **Alias:** the plan for a block with `MYSTICPARADOX_A=1`, `MYSTICPARADOX_B=` (empty value is copied), an existing
  `RUSTIC_ISLES_A` (not overwritten), `mysticparadox_c=1` (lower case: ignored), `=C:=C:\x` (ignored), a value
  holding `=`.
- **Backend grammar:** every row of 4.2 with an input that reaches it and the exact reason; a valid case for each host
  form; the 4.3 examples; `HTTPS://Example.COM` gives host `example.com`, secure; ports `:1`, `:65535` pass and `:0`,
  `:65536`, `:080`, `:`, `:8a` fail; `http://a://b` fails at 8; `https://[::1]:8443` gives host `[::1]`, port 8443;
  `https://[::1]x` fails at 11; `https://user@h` fails at 7; 300 characters pass the length check and 301 fail.
  No reason text contains any part of the input.
- **Results:** `api_base` with and without default ports for both schemes; `chat_address` never has a port; the
  built-in triple.
- **Variables:** the first set wins; an empty first falls to the second; a refused first never falls to the second; a
  512-unit value is `value too long` with length 512; the log line for env, built-in and refused cases.
- **Auth mode:** the table of 5.
- **Launch:** a 9-entry list (every field); a 12-entry list (extra entries ignored); port `0`, `65536`, `+80`, ` 80`,
  `80x`, `` are launch errors; manual mode with and without `Port=`, `Port=x` (8777), argv 1 `-log` (map `Ramsgate`),
  `Ramsgate_Hub?listen` (map `Ramsgate_Hub`); the program path with a space.
- **Roster:** `NO_EXPECTED_PLAYERS`; `a:b`; `a:b,c:d`; `a:b,` (1); `a:b,,c:d` (0); `:b` (0); `a:` (0); `a` (0); a
  roster longer than the cap is clamped.
- **Map URL:** every sentinel combination; `MaxPlayers` always; `?listen` only when asked; manual-mode absent fields.
- **Pacing:** each variable unset, `0`, at both ends of its range, one past each end, ` 30` (accepted), `30 ` (refused),
  `+30`, `-0`, `x`, 15 and 16 characters; the notes text.
- **Trials:** minutes `0`, `1`, `525600`, `525601`, unset; week `0`, `1`, `181`, `182`; row names `_001`, `_181`,
  `_000`, `_182`, `_1`, `_0011`; buckets at fixed instants: 1970-01-01T00:00:00Z gives week 1, bucket start the epoch;
  2026-11-12T00:00:00Z (Unix 1 794 441 600 s) gives bucket 2 967 and week **72** with 10 080 minutes, bucket
  996 912 and week **146** with 30 minutes.
- **Files and env for cap and AFK:** each row of 10.2; ` 12\nx` is 12 (stream rule); `12x` is 12 for the file but
  refused for the environment (`strtol` rule); `x12`, empty, a BOM; `99999999999` for `i32` unreadable; source order
  1 to 4 and the source texts.

**Never in CI:** nothing here touches the game. R1 has no play-test of its own (surface section 8: inert).

---

## 12. Calls, departures and settled questions

Calls:

1. The built-in backend host is always `backend.invalid` (no deployment header).
2. A full launch's port must be a plain decimal port; anything else ends the process with `0xE106` in Init.
3. RT-11 variable names `DAINTY_HUB_MAX_PLAYERS`, `DAINTY_HUB_AFK_SECONDS`, `DAINTY_HUNT_AFK_SECONDS`, read before the
   files; the files stay until the host kit writes the variables.
4. Number files are read up to 64 bytes.
5. Values are kept as UTF-16 where the engine takes them back (the launch fields, the map URL).

Departures from the AGPL (each fails closed or removes a crash; none changes a launch the launcher or Director makes):

1. The backend log line names the variable read and logs the value's length.
2. An unreadable `RUSTIC_ISLES_AUTH_MODE` is `Invalid`, not `GlobalGameserver`.
3. Ports: strict parse in full mode; fall back to 8777 in manual mode instead of throwing.
4. Manual mode's map comes from argv entry 1.
5. An unreadable environment value for the cap or the timeouts falls through to the file.

Ambiguity: the AGPL's manual-mode map URL reads absent fields; the MIT treats them as absent.

Settled here:

- **RT-15:** yes, every DeployServer gets `DAINTY_BACKEND_URL` and `RUSTIC_ISLES_BACKEND_URL` (4.4).
- **RT-13:** `RUSTIC_ISLES_INTERNAL_VERSION` (`"0.0.3"`) is read by nothing in this repository (the launcher, the
  Director and the host kit never look for it); it is **not ported**. The MIT DLL reports its crate version in R0's
  status line instead.
- **RT-14 for R1:** R1 owns no flag file.
- RT-12, RT-16 and RT-18 are not R1's (R7, R4, and R0 for the matcher API).

No open question for the maintainer.

## 13. Size

Rust without tests about 650 lines (alias 50, backend 220, auth and launch 150, roster and map URL 60, pacing 40,
trials 50, cap and AFK 80) plus 60 lines of Windows readers. Tests about 600 lines.
