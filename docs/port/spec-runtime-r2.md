<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R2, backend HTTP (RD27, plan item 11)

R2 moves hook **HTTP-1** (surface section 3.2) into the MIT runtime: the game's outgoing HTTP requests are redirected
to the Dainty Lass backend, a dedicated server's progression grants get an idempotent grant id, and a DeployServer
gameserver's requests carry its key. It is the first slice that changes game behaviour, and the first to consume R1's
parsed configuration.

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md` and `spec-runtime-r1.md` alone,
and never reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472),
the request's layout offsets and virtual slots, the engine allocator's entry points, header names, the host allowlist,
exact log texts and behaviour. Conventions are the surface page's: an RVA is `+0x0XXXXXXX` from the executable's image
base; **C** client, **S** server, **B** both. **(call)** marks a decision made here, **(departure)** a deliberate
difference from the AGPL runtime, **(ambiguity)** something the source did not settle; all are collected in section 11.

---

## 1. Scope

| R2 owns | Section |
| --- | --- |
| Hook HTTP-1 on `FCurlHttpRequest::ProcessRequest`, installed through R0's broker (B) | 3 |
| The redirect of allowlisted hosts to `<API base>/__origin/<host><tail>` (B) | 4 |
| The progression grant id (S: DeployServer and player host) | 5 |
| The gameserver key header (S: DeployServer) | 6 |
| Feature id `HTTP-1@R2`; RT-7 for the chat address | 2, 8 |

Not R2: the player host's capability header (`HTTP-1@R10`, slice R10; it stays in the AGPL runtime until R10 lands),
the backend address itself (R1 section 4), the logging-only probes around HTTP (RT-3, section 9).

### 1.1 Where the code goes

R0 confines `unsafe` to the DLL's `ffi` module (R0 section 2), so R2 is split three ways:

| File | Holds |
| --- | --- |
| `crates/rustic-isles-runtime-core/src/http.rs` (new) | everything pure, on UTF-16 slices or `&str`: the URL split (4.2), the allowlist (4.3), the redirect builder (4.4), the grant path rule (5.2), the grant id format (5.4), the key-header decision (6), the log line texts (10) |
| `crates/rustic-isles-runtime/src/ffi/http.rs` (new) | the HTTP-1 detour, the request reads and writes, the engine allocator and virtual calls, the CSPRNG call |
| `crates/rustic-isles-runtime-core/src/offsets.rs` | the R2 lines of section 3.1 (the `TARGETS` line and the `layout`/`data` constants), with a comment naming R2 |
| `crates/rustic-isles-runtime-core/src/registry.rs` | `LANDED` gains `"R2"`; the R0 test that asserts `HTTP-1` unowned is updated (section 10) |
| one-line edits | `pub mod http;` in the core crate's `lib.rs` and in `ffi/mod.rs`; one install entry per role in `src/lifecycle.rs` (3.3) |

`lifecycle.rs`, both `lib.rs`/`mod.rs` files and `registry.rs` are R0's files; R2's edits there are the one-line
additions R0 section 2 allows for a slice (its install entry, its module line, its `LANDED` name). The registry already
holds R2's lines (`HTTP-1` kind chain, `HTTP-1@R2` and `HTTP-1@R10` kind feature): R2 adds no id.

### 1.2 What R2 reads from R0 and R1

All through the existing `rustic-isles-runtime` API, read-only:

| Value | From | Use |
| --- | --- | --- |
| `config::backend_url().api_base()` | R1 4.4 | the redirect's base (4.4). Resolved in Init before any hook exists, so the detour never sees the "not resolved" fallback; R2 converts it to UTF-16 once (a `OnceLock`) on first use |
| `config::is_server()` | R0 3.2 | grant id and key header are server-only |
| `config::auth_mode()` | R1 5 | `GlobalGameserver` or `PlayerHost` for the grant id; `GlobalGameserver` for the key |
| `config::launch()` → `Launch::api_key` | R1 6.1 | the gameserver key (`Some` only for a full launch in `GlobalGameserver` mode; never logged) |
| `ffi::broker::hook`, `NextCell` | R0 7.2 | installing the wrapper |
| `ffi::memory::readable`, `read` | R0 5.1 | every game memory read |
| `ffi::log::line`, `ffi::detaching()`, `lifecycle::write_status_once` | R0 3.3, 4.3 | logging and the detach rule |

R2 parses nothing itself: a host, port or scheme in the backend address is R1's answer.

---

## 2. Ownership and the AGPL side

With `"R2"` in `LANDED`, `rir_owns` answers 1 for `HTTP-1` and `HTTP-1@R2`, and 0 for `HTTP-1@R10`. The status line
reads `owns=5 ids`.

The AGPL runtime's contract with seam patch 0041 already covers R2; **no further AGPL patch is needed (no 0042)**:

- HTTP-1 is a chain id. The AGPL runtime always joins its chain with `rir_chain_add_inner` (band `Agpl`, innermost),
  whether or not the MIT owns it, so its wrapper keeps running.
- Inside that wrapper every block R2 replaces is gated on `HTTP-1@R2`: the URL redirect, the grant id and the key
  header. With R2 landed, the AGPL wrapper does none of them.
- The player host's capability header is gated on `HTTP-1@R10` and stays AGPL. It is set by the AGPL wrapper only in
  `PlayerHost` mode, so it never overlaps the key header (which is `GlobalGameserver` only), and it runs after R2's
  work, exactly as before (it came last in the AGPL's own order).
- No state crosses: nothing the AGPL keeps reading is set by R2's blocks. R0 section 7.5's cross-slice table gains no
  row.
- The AGPL runtime still resolves the backend address for itself and logs its own `[BackendAddress]` line; the MIT
  writes its own (R1 4.4). Both read the same variables, so they agree. Harmless; it goes with R21.

**With the MIT DLL absent** the AGPL runtime does everything as today (seam 0041).

---

## 3. The hook

### 3.1 Facts

| Name | Value | Notes |
| --- | --- | --- |
| HTTP-1 target, `FCurlHttpRequest::ProcessRequest` | `+0x032C5FE0` | found by RVA (the AGPL checks no prologue). Signature `(request: *mut c_void) -> u8` (a C++ `bool`); the request is the only argument |
| Request: readable span | `0xD0` bytes | the span every R2 access stays inside, checked before any field read |
| Request: URL | `FString` at `+0x70` | `{ data: *mut u16, num: i32, max: i32 }`; `num` counts the terminating NUL. `ProcessRequest` reads this field before it configures the native request, so a rewrite here is what curl uses |
| Request: in-flight word | `i32` at `+0xC4` | `1` = the request is already in flight: the native call fails it, and the engine ignores header changes |
| Request: vtable | the first pointer of the request | readable for 17 slots before use |
| Vtable slot 2: `GetHeader` | `(this, out: *mut FString, name: *const FString) -> *mut FString` | hidden-return form: the callee zeroes `*out`, looks `name` up in the request's header map and copies the value into an engine allocation |
| Vtable slot 16: `SetHeader` | `(this, name: *const FString, value: *const FString)` | byte offset `0x80`; copies both strings, so the caller's buffers need live only for the call (the AGPL passes buffers it owns, not engine memory). Ignored while in flight |
| `FMemory::Realloc` | `+0x02564AB0` | `(ptr, size: usize, alignment: u32) -> ptr`; `ptr` null acts as a malloc. Lazily sets up and dispatches through `GMalloc` |
| `GMalloc` | data `+0x06E1EB58` | a pointer to the engine allocator object. Its vtable slot 6 (byte `+0x30`) is `Free(this, ptr)`; a null `GMalloc` means nothing was allocated through it |

**(ambiguity, corrected here)** The surface page gives `FMemory::Realloc` as `+0x023BB590`. That is the 1.12 address
the AGPL's comment carries; the call it makes on 1.14.7 is to `+0x02564AB0`, and `GMalloc` on 1.14.7 is
`+0x06E1EB58`. Use this page's values.

`offsets.rs` lines (shape as R0 6.1):

- `TARGETS`: `{ id: "HTTP-1", rva: 0x032C_5FE0, found_by: Rva, what: "FCurlHttpRequest::ProcessRequest" }`.
- `layout`: `HTTP_REQUEST_SIZE = 0xD0`, `HTTP_REQUEST_URL = 0x70`, `HTTP_REQUEST_IN_FLIGHT = 0xC4`,
  `HTTP_VTABLE_SLOTS = 17`, `HTTP_GET_HEADER_SLOT = 2`, `HTTP_SET_HEADER_SLOT = 16`, `FSTRING_MAX_NUM = 32_768`,
  `MALLOC_FREE_SLOT = 6`.
- `data`: `FMEMORY_REALLOC = 0x0256_4AB0`, `GMALLOC = 0x06E1_EB58`.

### 3.2 The wrapper

One MIT wrapper on HTTP-1 through `broker::hook(base + 0x032C5FE0, wrapper, order = 0, &NEXT)`. Each call, on whatever
thread issued the request (the game thread or an HTTP worker; nothing here may assume the game thread):

1. If detaching: return `next(request)`.
2. `write_status_once()` (the first call into any MIT detour writes R0's status, R0 4.3).
3. Inside `catch_unwind`, in this order:
   1. the redirect (4), both roles;
   2. server only: the grant id (5);
   3. server only: the key header (6).
4. Return `next(request)` (the AGPL wrapper when the AGPL runtime is loaded, then the original), passing its return
   value through unchanged.

A panic in step 3 logs `[Panic] HTTP-1 ...` once (R0 2) and still reaches step 4 with the original argument. R2 never
absorbs a call: every request reaches the game. **(call)** `order = 0`; R10's capability header, when it moves, takes a
larger `order` so the R2 steps keep running first, as in the AGPL.

Every request field is read through R0's `readable`/`read`; a failed read ends that step (and the steps that need it)
for this call and nothing is logged for it.

### 3.3 Install

In Init after the dispatch chains: client `SP-1`, then `HTTP-1`; server `SP-2`, `SP-3`, then `HTTP-1`. Its result goes
into the status as `[MitRuntime] install HTTP-1 ok|<reason>`. **Settled (RT-17 for R2):** HTTP-1 has no order
dependency on any other id: it reads the backend address, the role and the launch, which Init resolves before any
install, and nothing else. The surface's install list puts it among the first dozen hooks on both roles; any
position after `MH_Initialize` is equivalent.

---

## 4. The redirect (B)

Always on, both roles. No flag turns it off (a stray file must never break every player's connection).

### 4.1 Reading the URL

The request must be readable for `0xD0` bytes, and the URL `FString` at `+0x70` must satisfy all of: `data` non-null,
`num >= 2`, `max >= num`, `num <= 32768`, and `data` readable for `num × 2` bytes. Then, if the in-flight word is `1`,
stop (the native call fails such a request; its URL is left as it is). The URL text is the first `num − 1` UTF-16 units
(the NUL excluded).

### 4.2 The split

`split_url(url) -> Option<(host, tail)>`, on UTF-16 text:

1. Find the **first** `://`; none: no split.
2. The scheme is the text before it; it must equal `http` or `https` ignoring ASCII case; otherwise no split.
3. The authority starts after `://` and ends at the first of `/`, `?` or `#` after it, or at the end of the text.
4. The tail: no delimiter found: `/`. The delimiter is `/`: the text from it to the end, verbatim. The delimiter is
   `?` or `#`: `/` followed by the text from it to the end (a URL may carry a query with no path).
5. In the authority, drop everything up to and including its **first** `@` (user information), then cut at its
   **first** `:` (the port).
6. What remains is the host; empty: no split.

The host keeps its case and is not validated further here. (A bracketed IPv6 host is cut at its first `:`; it then
never matches the allowlist, which is the intended outcome.)

### 4.3 The allowlist

`is_redirect_host(host)`: true when, for one of the apexes `steelyard.ca`, `steelyard.online`, `ol.epicgames.com`,
`api.epicgames.dev`, either the host equals the apex, or the host is longer than the apex, the character just before
its last `len(apex)` characters is `.`, and those last characters equal the apex. Comparisons ignore ASCII case.
**(call)** ASCII case folding; the AGPL used the C runtime's locale-dependent comparison, which agrees for every host
that can match. Never a substring match.

The backend accepts `/__origin/<host>` only for the same apexes (lower-cased, `[a-z0-9.-]`, no leading, trailing or
double dot, at most 253 characters) and answers 400 otherwise; R2 does not repeat those checks.

### 4.4 The rewrite

When the split succeeds and the host is on the allowlist:

1. `replacement = <API base> + "/__origin/" + host + tail`, `host` exactly as split (its case kept, user information
   and port gone). With the built-in address this is `https://backend.invalid/__origin/...`: it fails closed.
2. If `replacement` has 32 767 or more UTF-16 units: stop, URL unchanged.
3. `new_num = len(replacement) + 1`. If `max < new_num`: `data = FMemory::Realloc(data, new_num × 2, 0)`; a null
   result: stop, URL unchanged (the old buffer is still the request's).
4. Copy `replacement` and a NUL into `data`; store `data`, `num = new_num`, and `max = new_num` when `max` was smaller.

Never allocate the URL buffer with Rust's allocator: the engine later frees or reallocates it through `GMalloc`.

The first rewrite in the process logs `[Http] redirect active host=<host>` (once, an atomic flag). Nothing else about
URLs is logged: no path, no query (it may carry an exchange code), no header.

Hosts that are not on the allowlist are left alone, and their requests still get the grant id (5) and the key (6) as
those sections say.

---

## 5. The progression grant id (S)

### 5.1 When

Server role, auth mode `GlobalGameserver` or `PlayerHost` (an `Invalid` mode never gets one), after the redirect, on
every request. The backend's `POST /progression/<account>/<track>/<amount>` applies an XP grant once per id; the game
sends none (no grant or idempotency string exists in the executable). UE's HTTP retry calls `ProcessRequest` again on
the **same** request object, so the id is read back first and a retry keeps the id of its first attempt: the backend
then replays the grant instead of applying it twice. Without an id the backend mints one per request for a DeployServer
(so a retried grant would count twice) and refuses the grant for a player host (a realm member's kill XP was never
saved before the AGPL added this).

### 5.2 The grant path

Read the URL again exactly as 4.1 (it may have just been rewritten), without the in-flight stop. Then
`grant_path(url) -> Option<path>`:

1. `split_url` (4.2); no split: none.
2. `path` = the tail up to its first `?` or `#`.
3. If `path` starts with `/__origin/`: find the next `/` after that prefix; none, or right after the prefix (an empty
   host): none. Otherwise `path` = the text from that `/` on.
4. `path` must start with `/progression/`. After it: a first segment up to the next `/`, non-empty; a second segment up
   to the next `/`, non-empty; then the amount, the rest of `path`, which must be 1 to 9 characters, all ASCII digits.
   Anything else: none.

The result is the whole `path` (`/progression/<account>/<track>/<amount>`), used only in the log line. No grant path:
nothing more for this request.

### 5.3 Setting it

1. Count this grant request against a process budget (an atomic counter); the log lines below are written only while
   the count before this request is below **2000**.
2. In-flight word `1`: log `[Http] grant path=<path> in flight: header untouched (the native call fails it)` and stop.
3. The vtable must be readable for 17 slots; else stop.
4. `GetHeader(request, &out, &name)` with `name` = `x-rustic-isles-grant-id` and `out` a zeroed `FString` in R2's
   memory.
5. **Reused** when `out.data` is non-null and `out.num > 1`. Its text is the first `out.num − 1` units when
   `out.num <= 256` and the buffer is readable, else `<unreadable>`. Any non-empty value already on the request is kept,
   even one R2 cannot read: replacing it on a retry would turn one grant into two ids.
6. If `out.data` is non-null, free it through `GMalloc` (3.1: read `GMalloc`, check it and its vtable readable for 7
   slots, call slot 6). It was allocated by the engine; never free it any other way. A null or unreadable `GMalloc`:
   skip the free.
7. Not reused: draw 16 bytes from the OS CSPRNG (`ProcessPrng` from `bcryptprimitives.dll`, or `RtlGenRandom` =
   `advapi32!SystemFunction036`; both are imports R0 section 2 allows). On failure log
   `[Http] grant path=<path> no random bytes: no grant id set, the backend will refuse this grant` and stop (fail
   closed: better refused than applied twice). Otherwise build the id (5.4) and call `SetHeader(request, &name, &id)`.
8. Log `[Http] grant path=<path> grant=<id, first 128 characters> reused=0|1`.

### 5.4 The id

`gs-` followed by the 16 bytes as 32 lowercase hexadecimal digits, high nibble first (35 characters). The backend's
grammar is `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`, so the id always passes it. `grant_id(bytes: [u8; 16]) -> String`
is pure.

---

## 6. The gameserver key header (S, DeployServer)

On every request, after the grant id, when: server role, auth mode `GlobalGameserver`, `Launch::api_key` is `Some`
(a full launch; R1 6.1), the in-flight word is not `1`, the vtable is readable for 17 slots, **and the URL (read as
4.1, after the redirect) starts with the API base followed by `/`** **(departure)**: call
`SetHeader(request, "x-rustic-isles-gameserver-apikey", <key>)`, the key's UTF-16 units as R1 kept them (an empty
key is sent empty, as the AGPL does).

Never logged: the key, or that a given request carried it.

**Departure (needs the maintainer's ruling, call 4).** The AGPL sets the key on **every** request the server makes,
whatever its host. Every backend request is rewritten to the API base (4.4), so limiting the key to URLs under the API
base changes nothing the backend sees, and stops the key from leaving with a request to any other host (AGENTS.md:
a key goes only to the server it was made for). If the maintainer prefers the AGPL behaviour, drop the URL condition;
nothing else changes. The in-flight condition is equivalent to the AGPL (the engine ignores the call then).

The player host's header (`x-mystic-host-capability`, renamed by D3 in R10) is not R2's (section 2).

---

## 7. Threads and allocation

- The detour runs on any thread, concurrently. R2 keeps no per-request state; its shared state is the UTF-16 API base
  (written once), the "redirect logged" flag and the grant log counter (atomics).
- Every buffer handed to the engine to **keep** (the URL) comes from `FMemory::Realloc`. Buffers handed to `GetHeader`
  as a name and to `SetHeader` as name and value are R2's own, valid for the call. Every buffer the engine hands back
  (`GetHeader`'s copy) goes back through `GMalloc`'s `Free`.
- No lazily constructed statics that allocate on first use from an HTTP worker under a lock the engine holds: use
  `OnceLock`/atomics (the AGPL crashed once on a lazily built container first touched from a curl worker thread).

---

## 8. RT-7: where the client's chat connects (settled)

On 1.14.7 the AGPL runtime does **not** install its XMPP config redirect (the `GetConfigString` target `+0x025E6890`
has no verified signature; it logs that it skipped it). The client's chat address therefore comes from the game's own
config, and the launcher writes it: on every launch it rewrites `Engine.ini` with
`[OnlineSubsystemMcp.XMPP] ServerAddr="ws://<relay host>"`, `ServerPort=<relay port>`, `bUseSSL=false`
(`crates/dainty-launcher-kit/src/platform/game_config.rs`), pointing chat at its loopback relay, which carries it to the
gateway (`docs/architecture.md`, "Play"). **The MIT runtime does not port the XMPP redirect.** R1's `chat_address()`
and `chat_port()` stay unused by R2; a later slice may drop them, or use them if a launch without the launcher ever
needs chat. The patches page's note that `bUseSSL` follows the address's scheme describes the uninstalled redirect.

---

## 9. Flag files (RT-14 for R2)

Appendix A of the surface gave R2 three flags. **R2 keeps none**:

| Flag | AGPL use | In the MIT runtime |
| --- | --- | --- |
| `URL_LOG` | a deduplicated observation log of hosts and paths (development profile only) | **dropped**: its logger is never called on 1.14.7, and it is logging only (RT-3) |
| `URL_PATH_DIAG` | logs host and path of every request, rewritten or not | **dropped**: logging only (RT-3) |
| `CHARACTER_ASSOC_PROBE` | request-to-issuer probes | **dropped**: the probes are compiled out or never installed on 1.14.7 |

The AGPL's grant log line also carries a native call chain from the `[GrantTrace]` diagnostics (R19, not ported); the
MIT line does not. `GRANT_TRACE_OFF` and `KILL_XP_TRACE_OFF` stay R19's. The surface page's `PlayerExperienceComponent.*`
"grant inputs" on SP-2 are traces only (they change nothing); they are R19's and are not ported: the grant id needs
nothing from ProcessEvent.

RT-18 does not arise for R2: it registers no dispatch handler.

---

## 10. Tests

### 10.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`), pure, on every platform:

- **Split:** `https://auth-prod.steelyard.ca/tags?x=1` gives host `auth-prod.steelyard.ca`, tail `/tags?x=1`;
  `HTTPS://X.steelyard.ca` gives tail `/`; `https://x.steelyard.ca?a=1` gives tail `/?a=1`; `https://h#f` gives `/#f`;
  `https://user:pw@h.steelyard.ca:8443/p` gives host `h.steelyard.ca`, tail `/p`; `ftp://h/x`, `h/x`, `https:///x`,
  `https://@/x` give no split; `https://[::1]:80/x` gives host `[` (and no match).
- **Allowlist:** each apex exact and as a subdomain (`a.b.steelyard.ca`), mixed case; refused: `steelyard.ca.evil.com`,
  `evilsteelyard.ca`, `xol.epicgames.com`, `epicgames.com`, `api.epicgames.com`, `steelyard.co`, empty.
- **Rewrite text:** with API bases `https://backend.invalid`, `http://127.0.0.1:61100`, `https://[::1]:8443`; the host's
  case kept; user information and port dropped; a replacement of 32 766 units is allowed and 32 767 refused.
- **Grant path:** `/progression/a/b/10`; the same under `/__origin/x.steelyard.ca`; with `?q` and `#f` (cut);
  refused: `/__origin/` alone, `/__origin//progression/a/b/1`, `/progression/a/b`, `/progression//b/1`,
  `/progression/a//1`, `/progression/a/b/`, `/progression/a/b/1x`, `/progression/a/b/1234567890` (10 digits),
  `/progression/a/b/1/2`, `/Progression/a/b/1`; the full URL form (`https://backend.invalid/__origin/...`).
- **Grant id:** fixed bytes `00 01 … 0f` give `gs-000102030405060708090a0b0c0d0e0f`; every id matches the backend's
  grammar.
- **Key decision:** the table of 6 (role, mode, key present, in flight, URL under the API base or not, including a URL
  that only shares a prefix with the base, such as `https://backend.invalidx/...`).
- **Log texts:** the lines of 4.4 and 5.3, and that none contains a query, a header value other than the grant id, or
  the key.
- **Registry:** with `LANDED = ["R0", "R1", "R2"]`, the owned ids are `SP-1`, `SP-2`, `SP-3`, `HTTP-1`, `HTTP-1@R2`
  (count 5); `HTTP-1@R10` is not owned. The offsets table test finds HTTP-1's line (R0 6.1 checks every landed
  detour).

DLL (`cargo test -p rustic-isles-runtime`, Windows): build a fake request in test memory (a buffer of `0xD0` bytes, a
URL `FString`, a fake vtable whose slots 2 and 16 are test functions recording their arguments, a fake `GMalloc`) and
drive the steps directly (not through the game's RVAs): the rewrite into a buffer that is large enough, one that must
grow (through an injected realloc), a failed realloc (URL untouched); the in-flight stop; the grant id set once and
reused on a second call with the same request; the free of `GetHeader`'s copy; the key header on and off per 6. The
detour itself is checked through the broker on a test function, as R0's broker test does.

### 10.2 Never in CI

The real `ProcessRequest`, the engine allocator, the backend's answers, and both DLLs together in the game.

### 10.3 The maintainer's play-test (surface section 8, R2 row)

Build and deploy both DLLs as R0 11.1 step 1. Then:

1. **Login and the store.** Start the client through the launcher. In `rustic_isles_dll_port0.log`:
   `[MitRuntime] ... owns=5 ids`, `[MitRuntime] install HTTP-1 ok`, `[MitRuntime] [BackendAddress] source=env ...`,
   `[Http] redirect active host=<an allowlisted host>`, and **no** AGPL `[UrlRedirect] ProcessRequest active` line.
   Log in, reach Ramsgate, open the store; the backend's log shows `/__origin/...` requests.
2. **Kill XP.** Let the Director start a hunt; finish it. The hunt server's log shows
   `[Http] grant path=/progression/... grant=gs-... reused=0` lines (and `reused=1` if a grant was retried); the
   backend applies each grant once; the XP is there after returning to Ramsgate. The server's requests still carry the
   key (the backend accepts the gameserver's calls; a missing key shows as refused game-server routes).
3. **A realm hunt.** Host a realm and finish a hunt in it: grant lines appear on the hosted server and a member's XP is
   saved (the capability header is still AGPL's and must still be sent).
4. **Fail closed.** Start the client with `DAINTY_BACKEND_URL=https://bad host` (any value R1 refuses): the log shows
   `(DAINTY_BACKEND_URL ignored: contains a space, control or non-ASCII character, length=...)`, `source=built-in`
   (`api=https://backend.invalid`), the redirect line still appears (its requests now go to `backend.invalid`), and
   login fails with a connection error instead of
   reaching any other server. The launcher sets the variable itself, so this step is run by hand (start the game
   through `dainty-inject.exe` as in R0 11.1 with the variable in its environment).
5. **Without the MIT DLL**, step 1 behaves as before (the AGPL's own `[UrlRedirect]` line appears).

Only CI passed until these steps are done.

---

## 11. Calls, departures, ambiguities and settled questions

Calls:

1. HTTP-1's MIT wrapper has `order = 0`; R10 takes a larger one so R2's steps stay first.
2. ASCII case folding for the scheme and the allowlist.
3. MIT log tag `[Http]` for R2's lines (R0 call 6), texts as in 4.4 and 5.3.
4. **The key header only on URLs under the API base** (6). **Needs the maintainer's ruling.**
5. R2 keeps none of its three flags (9).
6. The CSPRNG is the OS's (`ProcessPrng` or `RtlGenRandom`), the implementer's choice of the two.

Departures from the AGPL:

1. The key header is limited to the API base (call 4).
2. The grant log line drops the native call chain (an R19 diagnostic) and the thread id, and is tagged `[Http]`.

Ambiguities:

- The surface page's `FMemory::Realloc` RVA (`+0x023BB590`) is 1.12's; 1.14.7's is `+0x02564AB0` (3.1).
- HTTP-1 is found by RVA alone; the AGPL checks no prologue, so neither does R2. A wrong executable is caught by R0's
  name self-check only for the dispatch points, not here. **(call)** Not adding a prologue check: the RVA has been
  stable through every 1.14.7 release the project has run, and a check would need bytes nobody has recorded.

Settled here:

- **RT-7:** chat connects to the address the launcher writes into `Engine.ini`; the XMPP redirect is not ported (8).
- **RT-14 for R2:** no flags (9).
- **RT-17 for R2:** no order dependency (3.3).
- **RT-18:** not applicable (no dispatch handler).
- **Patch 0042:** not needed; seam 0041 already gates every R2 block (2).
- RT-12, RT-13, RT-15, RT-16 are not R2's (R7; R1; R1; R4).

Open for the maintainer: **call 4** only.

## 12. Size

Rust without tests about 330 lines: the core module about 150 (split, allowlist, redirect, grant path, id, key
decision, log texts), the `ffi` module about 180 (the detour, URL read and write, realloc, `GetHeader`/`SetHeader`,
the free, the CSPRNG). Offsets and registry edits about 25. Tests about 350.
