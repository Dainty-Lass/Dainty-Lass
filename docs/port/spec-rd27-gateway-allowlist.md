<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: the allowlist helper (RD27)

> **Ports moved 2026-11-10.** The loopback services' defaults are now backend 41000, Director 41001, content
> 41002, admin 41003, allowlist 41005, chat 41006 and QoS 41010, below the dynamic range Windows lends to Hyper-V,
> WSL and Docker. host.toml's `[ports]` moves any of them, and on Windows setup and start move one Windows has lent
> out (a private gateway's 61000 to 41080). The numbers below are the ones recorded upstream: read each 610xx
> service port as its 410xx counterpart. A private gateway's default stays 61000.

Part of [`spec-rd27-gateway.md`](spec-rd27-gateway.md). The gateway is
[`spec-rd27-gateway-front.md`](spec-rd27-gateway-front.md), whose section 15 has the address rules this page uses
(`check_allowlist_ip`, the canonical form, the classes).

The helper keeps the game's UDP ports open only to the addresses of players who logged in, as the gateway reports
them. It runs with the most rights of any server part (administrator on Windows, root or `CAP_NET_ADMIN` on Linux), so
everything it accepts is checked twice, it listens on loopback only, wants a shared secret, never runs a shell, and
only ever changes its own firewall rule or table. Anything that can make it run a command or open the ports to
everyone defeats the allowlist.

Quoted text in `code` is exact.

---

## 1. Settings

Read once at start from the env file; a bad one stops the start (`Configuration: <error>`). Optional values are
trimmed and empty means unset. Whole numbers are read as Node reads a number:
`<NAME> must be a whole number from <min> to <max>`. Only these `ALLOWLIST_*` names are read; Dauntless Revived's
`DR_NFT_BINARY`, `DR_NFT_TABLE` and `DR_FIREWALL_INTERFACE` are not, and `ALLOWLIST_POWERSHELL` (from an older
`.env`) is accepted and ignored.

| Setting | Default | Rule and error text |
| --- | --- | --- |
| `ALLOWLIST_DRY_RUN` | none, required | `1` (only log what would change) or `0` (really change the firewall); anything else, unset included: `ALLOWLIST_DRY_RUN must be set: 1 only logs what it would do (any test machine), 0 really changes the firewall (the server)` |
| `ALLOWLIST_BIND` | `127.0.0.1` | Exactly `127.0.0.1` or `::1`, else `ALLOWLIST_BIND must be 127.0.0.1 or ::1: the helper runs with administrator rights and only the gateway on this machine may reach it` (`localhost` is refused too) |
| `ALLOWLIST_FIREWALL` | `windows` | `windows` or `nftables`, else `ALLOWLIST_FIREWALL must be windows (Windows Firewall) or nftables (Linux), not <JSON string>` |
| `ALLOWLIST_NFT` | first existing of `/usr/sbin/nft`, `/sbin/nft`; else `/usr/sbin/nft` | Must start with `/`: `ALLOWLIST_NFT must be the absolute path of nft (the helper never looks it up in PATH)` |
| `ALLOWLIST_NFT_TABLE` | `reforged_revival` | Section 9.1 |
| `ALLOWLIST_INTERFACE` | none (every interface but `lo`) | Section 9.1 |
| `ALLOWLIST_PORT` | 61005 | 1 to 65535 |
| `ALLOWLIST_SECRET` | required | The gateway's secret rule (front page, section 1): `ALLOWLIST_SECRET is not set`, or 32 to 256 printable characters without spaces |
| `ALLOWLIST_ALLOW_PRIVATE` | off | Exactly `1` allows private, loopback and link-local addresses (tests and LAN parties) |
| `ALLOWLIST_TTL_SECONDS` | 600 | 1 to 86400 |
| `ALLOWLIST_MIN_INTERVAL_MS` | 3000 | 50 to 600000 |
| `ALLOWLIST_MAX_ENTRIES` | 256 | 1 to 1000 |
| `ALLOWLIST_PORTS` | `8770-8777` | Section 2 (the host kit widens it for `--max-hunts`) |
| `ALLOWLIST_AUDIT_LOG` | `allowlist-audit.log` | Made absolute |
| `ALLOWLIST_STATE_FILE` | `allowlist-state.json` | Made absolute |
| `ALLOWLIST_RULE_NAME` | `Dainty Lass game ports (allowlist)` | Below |

Order of the checks: dry run, bind, firewall, nft path, table, interface, then port, secret, TTL, interval, max
entries, ports, rule name.

**The rule name** (`check_rule_name`): the Windows Firewall rule this helper finds and changes; a second helper names
its own. It must start with `Dainty Lass ` or `Dauntless Reforged Revival ` (each with its trailing space), be at most
80 bytes, and hold only ASCII letters, digits, spaces and `(`, `)`, `-`, `.`, `_`. Else
`ALLOWLIST_RULE_NAME must start with "Dainty Lass " (or legacy "Dauntless Reforged Revival ") and hold only letters,
digits, spaces and ()-._, at most 80 (got <JSON string>)`. So `Dainty Lass`, `Dainty Lass *`, a name with a quote, a
newline or `ö`, and 81 characters are refused; `Dainty Lass 1.4.4 game ports (allowlist)` passes.

## 2. Ports

- **`check_ports(text)`**: `<digits>` or `<digits>-<digits>`, each 1 to 5 ASCII digits and nothing else (no sign, no
  space, no list), else `ALLOWLIST_PORTS must be a port or a range like 8770-8777 (got <JSON string>)`. Then within
  1-65535 and low to high, else `ALLOWLIST_PORTS must be within 1-65535, low to high`. Returns the normalised text:
  leading zeros gone, a single port kept single. `08770-08777` is `8770-8777`; `8777` stays; `1-65535` passes;
  `8777-8770`, `0-5`, `0`, `8770-70000`, `8770,8771`, `any`, empty, `-`, `8770-`, `-8777`, `123456`, ` 8777`,
  `8770-8777 accept`, `+8777` fail.
- **`check_port_list(text)`**: a comma-separated list of such items, each checked, rejoined with `,`; more than 32
  items: `ALLOWLIST_PORTS holds at most 32 ranges`. Empty items fail (`,`, `8770,`, `8770,,8771`, `8770, 8771` fail).
  `08770-8777,64000,64010-64019` is `8770-8777,64000,64010-64019`. `ALLOWLIST_PORTS` itself is one item; the list is
  that plus the game modules' ranges (section 12).

## 3. The firewall plan

`plan(ips, ports)`: the ports pass `check_port_list`; each address must pass `check_allowlist_ip` with private
allowed and already be in canonical form, else `refusing to change the firewall with <JSON string>`. The plan is the
addresses (as given, in order) and the checked ports; it is "enabled" when it has an address. Nothing reaches a
firewall without passing through `plan` again, whoever the caller. Refused: `2001:DB8::1` (not canonical),
`1.2.3.4/32`, `0.0.0.0`, `any`, empty, `010.1.1.1`, `224.0.0.1`, anything with a quote or a command.

## 4. The loopback endpoint

HTTP/1.1 on `ALLOWLIST_BIND:ALLOWLIST_PORT`. At most 64 connections at once (more are accepted and dropped); the head
within 5 s; each connection at most 30 s. Answers are JSON with `content-type: application/json`, `content-length`
and `cache-control: no-store`, and `connection: close` where noted. For each request, in order:

1. The peer must be a loopback address (defence in depth, the bind is loopback already): else 403
   `{"error":"forbidden"}`, close.
2. The path (exact, no query considered): `/allow` takes POST, `/status` takes GET; any other path is 404
   `{"error":"not_found"}`. Another method is 405 `{"error":"method_not_allowed"}` with `allow: POST` or `allow: GET`.
   These two need no secret.
3. The `x-allowlist-secret` header must equal the secret. Both sides are hashed with SHA-256 and the digests compared
   without an early exit, so the time says nothing about the secret. Missing or wrong: 401 `{"error":"unauthorized"}`,
   close; audited as `unauthorized` `{path, suppressed}`, at most once a minute.
4. `GET /status`: 200 with the status (section 6).
5. `POST /allow`: a `Content-Length` that read as a number is over 1024 is 413 `{"error":"body_too_large"}`, close;
   the body is read up to 1024 bytes, more is the same 413. A body that is not JSON is 400 `{"error":"bad_request"}`.
   Then the `ip` member of a JSON object is judged (a JSON value that is not an object, or has no `ip`, or whose `ip`
   is not a string, is `not_a_string`).
6. The address must pass `check_allowlist_ip` with `ALLOWLIST_ALLOW_PRIVATE`: else 400
   `{"error":"invalid_ip","reason":"<reason>"}`, audited as `refused` `{value, reason, suppressed}` at most once a
   minute per reason and shown value. The shown value is a string's first 64 characters with anything outside
   printable ASCII as `?`, or `number`, `boolean`, `object` (null, objects, arrays) or `undefined` (missing).
7. The live set is refreshed (section 5). Full: 503 `{"error":"allowlist_full"}`, audited as `full`
   `{ip, max, suppressed}` at most once a minute. Added: audited `added` `{ip}`, the state saved, the firewall sync
   asked for the new list. Either way the expiry timer is re-armed.
8. 200 `{"ip":"<canonical>","added":true|false,"ttlSeconds":<ttl>}`.

## 5. The live set

- Entries are canonical addresses, each with an expiry. `refresh(address, now)`: an existing entry's expiry becomes
  now plus the TTL (refreshed); a new one is added with that expiry (added) unless the set holds the maximum (full).
- `expire(now)`: removes and returns, sorted as text, every entry whose expiry is at or before now. Example: added at
  0 with a 600 s TTL, refreshed at 1 s: still there at 600.999 s, gone at 601 s.
- `list()`: IPv4 addresses before IPv6, each group by numeric value of the address bytes, so the same set always
  gives the same rule (`9.0.0.1`, `10.0.0.1`, `203.0.113.7`, `2001:db8::1`, `2001:db8::2`).
- The expiry loop sleeps until the earliest expiry plus 5 ms (or until woken by a change), then expires: each gone
  address audited `expired` `{ip}`, the state saved, the sync asked for the new list.
- **The state file**: `{"version":1,"entries":[{"ip":"<canonical>","expiresAt":<ms since epoch>},...]}` in list
  order. Saved when an address is added, when one expires, at stop, and every 30 s when something changed (a refresh
  only marks it changed). Written to `<file>.tmp` (mode 0600 where the file system has modes) and renamed into place.
  A failure warns `allowlist state file not saved` `{file, error}`.
- **Restoring at start**: a missing file is nothing; an unreadable or unparseable one warns `allowlist state file
  unreadable; starting empty` `{file, error}`. From `entries` (anything else than a list restores nothing), each item
  is skipped unless its `ip` is a string that parses, its `expiresAt` is a finite number after now, and the address
  passes `check_allowlist_ip` with the configured private setting; loading stops at the maximum; a restored expiry is
  never later than now plus one TTL. Example (TTL 600 s, now 1 s, public only): of `203.0.113.7` at 5000 s,
  `198.51.100.2` at 0.999 s, `10.0.0.1`, `0.0.0.0/0`, a number and `null`, only the first is restored, expiring at
  601 s.

## 6. Status

`GET /status` answers `{"backend": <name>, "dryRun", "allowPrivate", "ttlSeconds", "ports": <the guarded ports
text>, "entries": [{"ip", "expiresAt": <ISO>}...] in list order, "pending": <bool>, "lastApply": null or {"ok",
"enabled", "ips", "at": <ISO>, "error" only when there is one}}`, plus `"modulePorts": {<module>: [<range text>...]}`
only when a module guards any. The backend name is `windows-firewall` or `linux-nftables` (by the configured
firewall, dry run included). Nothing about the secret or the settings files.

## 7. The audit log

Append-only JSON lines in `ALLOWLIST_AUDIT_LOG` (its folder made at start): `{"t": <ISO now>, "event": <name>,
...fields}`. Each line is also logged as an info line `allowlist <event>` with the fields. A write failure is an error
line `audit log write failed` `{file, error}`. Never the secret. Events: `start` `{dryRun, restored, ttlSeconds,
ports, allowPrivate, backend}`; `added` `{ip}`; `expired` `{ip}`; `refused`; `full`; `unauthorized`; `rule_applied`
`{enabled, ips, dryRun, ms}`; `rule_failed` `{enabled, ips, error}`; `dry_run` (section 11); `ports_changed`
`{ports, modules}`; `stop` `{saved}`; `closed_ports` `{dryRun}`.

## 8. The firewall sync

Brings the firewall to the desired list, never more often than once per `ALLOWLIST_MIN_INTERVAL_MS`.

- The state: the desired list, the applied list (unknown at start and after a failed run, so the next run always
  applies), when the last run started, failures in a row, stopped, running, the run count, the last result, an epoch
  bumped by a re-apply, and an urgent flag.
- A request sets the desired list and wakes the scheduler. A list equal to the applied one causes no run.
- The scheduler waits until desired differs from applied, then waits the delay: the larger of the backoff (none
  without failures; else the interval times 2 to the power failures minus one, at most 60 s) and the spacing (the
  rest of the interval since the last run started, unless urgent). During the wait only an urgent request
  recomputes the delay; others leave the timer alone. Then, with the run lock held and not stopped, one run.
- A run applies the desired list as it is now, recording start time and duration. Success: applied becomes that list,
  unless the epoch changed during the run (then unknown, so it runs again); failures reset. Failure: applied unknown,
  failures plus one. The result `{ok, ips, at, ms, error}` is kept, reported (audited `rule_applied` or
  `rule_failed`), and only then is the run marked finished.
- `reapply()`: bumps the epoch, makes applied unknown, marks urgent, wakes: the same list runs again at once (a
  module's ranges changed and its servers are about to start). A re-apply during a run is not lost.
- `pending`: running, or applied differs from desired. `settled(timeout)` waits, polling every 10 ms, until not
  pending or stopped.
- `finish(list)` (shutdown): marks stopped, waits for a run in progress, sets the desired list and runs once at once
  unless it is already applied. Later requests are ignored.
- Behaviour that must hold: the first change after a quiet period applies at once; changes arriving during a run or
  within the interval fold into one later run, at least the interval after the previous start; a failed run is
  retried with backoff and each result reported (`[false, true]`).

## 9. nftables (Linux)

### 9.1 Names

- **Table** (`check_nft_table`): an ASCII letter, then up to 31 letters, digits or `_`, else
  `ALLOWLIST_NFT_TABLE must be a letter then up to 31 letters, digits or _ (got <JSON string>)`. `1table`,
  `my-table`, `_t`, 33 characters fail.
- **Interface** (`check_interface`): an ASCII letter or digit, then letters, digits or `_`, `.`, `@`, `-`, at most 15
  in all, else `ALLOWLIST_INTERFACE must be a network interface name like eth0 (got <JSON string>)`. `eth0`, `ens3`,
  `wg0`, `br-lan`, `eth0.100`, `0abc` pass; a quote, a space, `../x`, `-eth0`, `eth0:1`, 16 characters fail.

### 9.2 The script

`nft_script(plan, settings)` re-runs `plan` and both name checks, so no caller can put anything else in it. IPv4
addresses (no `:`) go in set `players4`, IPv6 in `players6`, each keeping the plan's order. A single port item is
written as it is; a list becomes `{ a, b, c }` (items joined with `, `). With an interface, the three game-port rules
start with `iifname "<name>" `. Lines (a tab is shown as `\t`), ending with a newline:

```text
table inet <T> {}
delete table inet <T>
table inet <T> {
\tset players4 {
\t\ttype ipv4_addr
\t\telements = { <v4 addresses joined by ", "> }      (only when there is one)
\t}
\tset players6 {
\t\ttype ipv6_addr
\t\telements = { <v6 addresses joined by ", "> }      (only when there is one)
\t}
\tchain input {
\t\ttype filter hook input priority -5; policy accept;
\t\tiifname "lo" accept
\t\t<iif>ip saddr @players4 udp dport <ports> accept
\t\t<iif>ip6 saddr @players6 udp dport <ports> accept
\t\t<iif>udp dport <ports> drop
\t}
}
```

nft applies a file as one transaction: the first line makes sure the table exists, the second deletes it, the rest
defines it again. So there is never a moment with the ports open to everyone or the table missing, and a script nft
refuses changes nothing. The chain's policy is accept: the helper cannot cut the host off anything else. Loopback is
never filtered. No addresses: both sets empty, the ports drop everything (closed).

Example, addresses `203.0.113.7`, `2001:db8::1`, `198.51.100.2`, ports `8770-8777`, interface `eth0`: `players4` has
`elements = { 203.0.113.7, 198.51.100.2 }`, `players6` has `elements = { 2001:db8::1 }`, and the three rules read
`iifname "eth0" ip saddr @players4 udp dport 8770-8777 accept` and so on. Ports `8770-8777,64000-64009,64020` read
`udp dport { 8770-8777, 64000-64009, 64020 }`, with the same number of lines as a single range.

### 9.3 Running nft

- nft runs at the configured absolute path with a fixed argument list, never through PATH or a shell: `-f -` with the
  script on stdin, `-f <file>`, or `list tables`. stdout is discarded, stderr kept, the child killed if dropped.
- A run has 30 s; past it: `nft did not finish within <ms> ms`. Spawning fails: `could not run <path>: <error>`. A
  non-zero exit: `nft exited with <code or "a signal">`, followed by `: <detail>` when stderr had any, where the
  detail is the first 1000 characters (trimmed) of stderr's last 4000 bytes. A broken pipe on stdin is ignored (the
  exit code says what happened).
- How the script is fed: unknown at first. The first run tries stdin; success settles on stdin. Failure tries a file;
  success warns `nft refused the script on stdin; using a private file from now on` `{detail}` and settles on files
  (stdin is not tried again). Both failing is the file's error, and the feed stays unknown.
- The private file: a new temporary folder named with the prefix `reforged-nft-`, mode 0700, holding `rules.nft`
  created with create-new, mode 0600; both removed after the run.
- `can_manage_nft(path)` (read-only, at start in real mode): the path is a file and `list tables` succeeds within
  15 s.

## 10. Windows Firewall

- Through COM (`INetFwPolicy2`), never PowerShell. Every call runs off the async threads, inside a COM apartment
  (multithreaded) set up and torn down around it; COM errors read `COM: <error>` and `Windows Firewall: <error>`.
- The rule constants: id `DaintyLass-GamePorts-Allowlist`, display name `Dainty Lass game ports (allowlist)` (or
  `ALLOWLIST_RULE_NAME`), grouping `Dainty Lass`, description `Opens the game ports to players who logged in through the
  gateway. Managed by the allowlist helper.` Rules are found by display name.
- `windows_change(plan, name)`: a plan without addresses is "close"; otherwise "open" with display name, description,
  grouping, direction `in`, action `allow`, protocol `udp`, local ports the plan's text as it is (`8770-8777` or
  `8770-8777,64000-64009`), remote addresses the plan's joined with `,`, profiles `all`. As JSON (for the dry run):
  close is `{"enabled":false}`; open is `{"enabled":true,"displayName","direction","action","protocol","localPorts",
  "remoteAddresses","profiles"}`.
- Applying: every rule with that display name is found. Close disables them all (Windows then drops the game ports'
  traffic by default). Open sets the first one (description, grouping, inbound, allow, UDP before the ports since ports
  need a protocol, local ports, remote addresses, all profiles, enabled) and disables any others, so exactly one is in
  effect; with none, a new rule is made with that name, set the same way and added. A rule an older helper made with
  the same display name is taken over as it is.
- `read_rules(name)` (read-only: enabled, local ports, remote addresses, protocol, direction, action of each rule with
  the name) and `remove_rules(name)` (every rule with the name, one at a time; only the live test uses it).
- `is_elevated()`: a read-only query of the process token's elevation.

## 11. The backends and dry run

- **Real**, by `ALLOWLIST_FIREWALL`: `nftables` needs Linux (`ALLOWLIST_FIREWALL=nftables needs Linux`) and
  `can_manage_nft` (`<path> could not list the firewall: install nftables (apt install nftables) and run the helper as
  root or with CAP_NET_ADMIN (the reforged-allowlist systemd unit does)`); `windows` needs Windows
  (`ALLOWLIST_FIREWALL=windows needs Windows (Windows Firewall); on Linux use ALLOWLIST_FIREWALL=nftables`) and an
  elevated process (`ALLOWLIST_DRY_RUN=0 needs administrator rights: run it as a scheduled task with "Run with highest
  privileges"`).
- **Dry run**: never touches the firewall. At start it warns `DRY RUN: the firewall is never changed; every change is
  only written to the audit log`. Each run writes an audit `dry_run` event: `{firewall: "windows"|"nftables", rule,
  enabled, ips, ports, backend}` where for nftables `rule` is `table inet <T>` and `script` is the exact script, and
  for Windows `rule` is the rule's display name and `windows` the change's JSON. The real backends can describe a plan
  the same way.
- Applying a plan always goes through `plan` with the ports read afresh for every run (the guard's text, section 12).

## 12. Game modules' UDP ranges (Category A, moves as it is)

`allowlist/guard.rs` and `allowlist/modules.rs` are Category A. What they rely on from the rewrite: `check_ports` and
`MAX_RANGES` (section 2), and the helper's use of the guard:

- The guard's text (`ALLOWLIST_PORTS` plus every attached module's ranges, sorted, adjacent and overlapping ranges
  joined, `a-b` or `a`, comma-joined) is the plan's ports at every run, and `/status`'s `ports`.
- Every change to the guard calls back once: the helper audits `ports_changed` `{ports, modules}` and calls
  `reapply()` on the sync.
- `admit_udp(ip, port)`: the guard covers the port and `allow_ip(ip)` succeeds. `allow_ip(ip)`: the same as a
  `POST /allow` with that string (sections 4.6 to 4.8), true on 200. `is_admitted(ip, port)`: the address passes the
  check, the guard covers the port, and its canonical form is in the live list.

## 13. Start, stop and the command line

- **Start** (`Helper::start`): restore the state (section 5), audit `start`, ask the sync for the restored list (an
  empty list closes the ports, so the rule never keeps addresses from before a crash), bind (`Could not listen on
  <bind>:<port>: <error>`), then run the expiry loop, the 30 s save loop and the endpoint. The info line `allowlist
  helper ready` `{listen, dryRun, firewall, ports, ttlSeconds, minIntervalMs, maxEntries, allowPrivate, auditLog,
  stateFile}`; the service status `ready` with the listen address and `ports <ports>`.
- **Stop**: info `stopping: closing the game ports` `{signal: "stop"}`; stop listening and the loops; save the live set;
  `finish([])` closes the ports; audit `stop` `{saved: <count>}`. A stop that takes more than 90 s fails with
  `Shutdown took longer than 90 s`. A crash leaves the last applied set, never wider.
- **`allowlist --close-ports`** (`close_ports_once`): the backend as configured (dry run or real, with real's start
  checks), then `plan([], ports)` and one apply of the empty list, audit `closed_ports` `{dryRun}`, exit 0; a failure
  `Could not disable the game-port rule: <error>`. For stop scripts: a supervisor that kills the helper never lets it
  close the ports itself.
- **`allowlist [--env-file <file>] [--close-ports]`**: without `--env-file`, a `.env` in the working folder if it is a
  file. Fatal errors are one `error` line and exit 1. Serving waits for the stop signal (`service_kit::serve`).
- **In-process** (`run_with_guard(config, ctx, slot, guard)`): the same start and stop under a service context, the
  helper placed in the slot so the gateway's feed and the core's admits reach it directly; a start failure fails the
  status; a guard shared with the core plugin (`run_with_context` makes a fresh one from `ALLOWLIST_PORTS`).
