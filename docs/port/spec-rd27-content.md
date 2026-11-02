# Clean-Room Behaviour Specification: `services/content` (RD27)

> **Scope & Purpose (RD27):** This is the clean-room behaviour specification for reimplementing DaintyLass's
> content server under MIT in dainty-lass. Today the server lives in dainty-restricted (`AGPL-3.0-only`) as
> `services/content` and its library wrapper `services/content-plugin`. Section 1 classifies every file of both
> crates; section 2 specifies everything in Category B from observable contracts (the HTTP replies recorded from the
> Node service, the env files the host kit writes, the log lines the tests read, the APIs other crates call); section 3
> lists the tests that define the behaviour; section 4 is the target design.
>
> The implementer works from this document, the MIT crates of dainty-lass (`protocol`, `service-kit`, `module-api`,
> `service-plugin`, `controller`) and the Category A files of section 1.2, and never opens dainty-restricted or
> Dauntless Revived. Where this spec names a type or function, it is either MIT API in dainty-lass or a public name
> another crate calls (section 2.1); every other name in the rewrite is the implementer's own.
>
> Baseline read for this spec: dainty-restricted `feat/rd27-plugin-abi` at `849f45e` (the two crates last changed in
> `ee7d90b`), dainty-lass `feat/rd27-plugin-abi` at `92e797a`.

---

## 1. Provenance & Licensing Classification

### 1.1 Evidence

- **Git history (dainty-restricted, which carries dainty-lass's history since the new root of 2026-10-26):** every
  file except `modules.rs` and the `content-plugin` crate was added by `7338b10` ("a new root without the Dauntless
  Reforged Revival line"), i.e. it predates the new root and came over from the RS2 port. `modules.rs` was added by
  `2296a8e` (DF4b). `content-plugin/*`, `tests/parity/harness.rs` and the rewritten `tests/parity.rs` were added by
  `ee7d90b` (RD27). Later commits (`9829d9a` removing 1.4.4, `75084b3` the DX4 rename, `eace7f5`, `62d56c6`) edited
  files without changing their origin.
- **NOTICE.md (dainty-restricted):** "`services/gateway` and `services/content` ... are modified versions of its
  [Dauntless Revived's] `rust/` folder at commit `e1c5fec`".
- **Migration guide (dainty-lass `docs/port/migration-guide.md` §2.4):** maps `ReforgedContent/src/{app,auth,config,
  server,branding,limits,log,news,runtime}.ts` and DR's `UndauntedContent` to `services/content/src/*.rs`; `runtime.rs`
  is new in Reforged (`runtime.ts`), the rest are DR's Rust port fitted to Reforged.
- **AGPL inventory (dainty-lass `docs/port/agpl-inventory.md` §1.3.15):** "Born here (1 file): `src/modules.rs`
  (DF4b)"; everything else DR-derived or Reforged-derived.
- **File headers:** the module comments of `app.rs`, `auth.rs`, `branding.rs`, `config.rs`, `limits.rs`, `news.rs`,
  `runtime.rs`, `server.rs` name their Node source (`src/app.ts` etc.); `tests/app.rs` names `test/app.test.ts` and
  "Dauntless Revived's streams test"; `harness.rs` and `record.cjs` name `legacy/ReforgedContent`.
- **Line similarity against DR** (`legacy-revived` at `24b7624`, the commit the AGPL inventory names; 3+-line runs of
  non-blank, non-comment lines, the method of `clean-room/similarity.md`), shown in the tables below.

### 1.2 Category A — written for DaintyLass from no external project (may move to MIT as is)

| Path (in dainty-restricted) | Origin | DR similarity | Depends on Category B |
| --- | --- | --- | --- |
| `services/content/src/modules.rs` | DF4b (`2296a8e`): content as a core plugin keyed by game version | 2.2% (closing braces) | `crate::log` only (a re-export of `service_kit::linelog`; replace with the MIT crate directly) |
| `services/content-plugin/Cargo.toml` | RD27 (`ee7d90b`) | — | depends on the `content` crate |
| `services/content-plugin/src/lib.rs` | RD27 (`ee7d90b`) | 8.3% (`use` lines, braces) | `content::config::Config::from_env`, `content::server::run_with_modules` (B); `content::modules` (A) |
| `services/content-plugin/tests/parity.rs` | RD27 (`ee7d90b`) | 3.5% | includes `services/content/tests/parity/harness.rs` (B) by `#[path]`; reads the fixtures |
| `services/content/tests/parity/content.requests.json` | Request script authored for the RS2 port | 11.4% (repeated JSON punctuation) | none (data) |
| `services/content/tests/parity/content-none.requests.json` | idem | 16.7% | none (data) |
| `services/content/tests/parity/content-invalid.requests.json` | idem | 22.1% | none (data) |
| `services/content/tests/parity/content-index.requests.json` | idem | 20.6% | none (data) |
| `services/content/tests/parity/content.expected.json` | Replies recorded from the Node service (`legacy/ReforgedContent`) on those scripts | (recorded output) | none (data) |
| `services/content/tests/parity/content-none.expected.json` | idem | — | none (data) |
| `services/content/tests/parity/content-invalid.expected.json` | idem | — | none (data) |
| `services/content/tests/parity/content-index.expected.json` | idem | — | none (data) |

**12 files.** The eight parity JSON files are test data, not program code: the scripts were written for this port in
the scripts' own format (described in section 3.1, so the implementer needs no code to read them), and the
`*.expected.json` files are what the Node server answered, i.e. observable facts about its behaviour. They are treated
exactly as `crates/protocol/tests/fixtures/runtime-manifest*.json`, which were recorded from the same Node service and
kept under MIT in the protocol rewrite (`clean-room/protocol.md`). See section 5, item 1.

### 1.3 Category B — derived from DR or another AGPL source (rewrite clean-room)

| Path (in dainty-restricted) | Derived from | DR similarity | Disposition |
| --- | --- | --- | --- |
| `services/content/Cargo.toml` | DR `rust/services/content/Cargo.toml` | — | Write a new manifest (section 4.4) |
| `services/content/data/dauntless-1.4.4.json` | DR's 1.4.4 game manifest data | — | **Drop.** Nothing references it since `9829d9a` removed the 1.4.4 game download |
| `services/content/src/lib.rs` | DR `lib.rs` | 47.4% | Rewrite |
| `services/content/src/app.rs` | DR `app.rs` + Reforged `app.ts` | 50.9% | Rewrite (sections 2.5–2.7, 2.14) |
| `services/content/src/auth.rs` | DR `auth.rs` + Reforged `auth.ts` | 47.7% | Rewrite (section 2.8) |
| `services/content/src/branding.rs` | DR `branding.rs` (Node `branding.ts` identical in DR and Reforged) | 62.1% | Rewrite (section 2.12) |
| `services/content/src/cli.rs` | DR `main.rs` | 66.7% | Rewrite (section 2.3) |
| `services/content/src/client.rs` | DR `client.rs` | 98.3% | Rewrite (section 2.9) |
| `services/content/src/config.rs` | DR `config.rs` + Reforged `config.ts` | 35.8% | Rewrite (section 2.2) |
| `services/content/src/input.rs` | Functions split out of DR's `app.rs`/`news.rs` and Reforged's `runtime.ts` | 30.4% | Rewrite (sections 2.5, 2.11, 2.13) |
| `services/content/src/limits.rs` | DR `limits.rs` | 78.5% | Rewrite (section 2.10) |
| `services/content/src/log.rs` | DR `log.rs` (a doc line and the `linelog` re-export; the test capture helper was added later) | 0.0% (one-line file in DR) | Rewrite; trivially a re-export (section 2.1) |
| `services/content/src/news.rs` | DR `news.rs` | 51.2% | Rewrite (section 2.13) |
| `services/content/src/runtime.rs` | Reforged `ReforgedContent/src/runtime.ts` (AGPL, Freeza's line) | 11.3% | Rewrite (section 2.11) |
| `services/content/src/server.rs` | DR `server.rs` + Reforged `server.ts` | 63.2% | Rewrite (section 2.4) |
| `services/content/tests/app.rs` | Reforged `test/app.test.ts` + DR `tests/streams.rs` | 16.1% | Rewrite from the invariants of section 3.3 |
| `services/content/tests/parity.rs` | DR `tests/parity.rs` | 26.3% | Rewrite (section 3.1) |
| `services/content/tests/parity/harness.rs` | DR `tests/parity.rs` (split out in RD27) | 57.1% | Rewrite from the harness contract of section 3.1 |
| `services/content/tests/parity/record.cjs` | DR `tests/parity/record.cjs` | 65.0% | **Drop.** It drives `legacy/ReforgedContent`, which was removed on 2026-10-26, so it cannot run; the fixtures are frozen |

**19 files** (17 rewritten, 2 dropped).

### 1.4 Category A files that depend on Category B code

- `src/modules.rs` calls the crate's `log` module for two info lines. Point it at `service_kit::linelog` (MIT) when it
  moves; no other change.
- `content-plugin/src/lib.rs` calls `content::config::Config::from_env` and `content::server::run_with_modules`. The
  rewrite keeps both names and signatures (section 2.1), so the file moves unchanged.
- `content-plugin/tests/parity.rs` includes `../../content/tests/parity/harness.rs` and uses its `Server` and `run`.
  The rewritten harness keeps that path and interface (section 3.1).
- The `*.expected.json` fixtures are only meaningful with a harness that builds the same fixture folders; the rewritten
  harness follows section 3.1.

### 1.5 Outside the two crates, for reference

Not classified here, but they touch the content API and must keep compiling: dainty-restricted's
`services/host/src/compose.rs` (`content`, `content_module`, `content_library`, `ContentView`; MIT header),
`services/host/src/lib.rs` (the hidden `content` subcommand), and the fuzz targets `fuzz/fuzz_targets/content_request.rs`,
`content_runtime_index.rs`, `content_news.rs`. dainty-restricted's `docs/security/content.md` is the AGPL checklist for
today's code; the rewrite writes its own (section 4.6).

---

## 2. Behaviour Specification (Category B)

### 2.1 Crate shape and the public API other crates call

The crate is a library named `content`. No binary of its own: the host runs it through `content::cli::main_from`
(the hidden `dainty-lass-host content` subcommand), the controller in process through `content::server`, and the core
as a loaded library through `content-plugin`.

Names other crates call today (dainty-restricted's `host`, `content-plugin`, the fuzz targets and the parity tests).
These must exist with these paths and shapes; everything else is the implementer's choice:

| Path | Shape | Caller |
| --- | --- | --- |
| `content::cli::main_from` | `fn main_from<I, T>(args: I) -> std::process::ExitCode where I: IntoIterator<Item = T>, T: Into<OsString> + Clone` | host `lib.rs` (service dispatch) |
| `content::config::Config` | `#[derive(Debug, Clone, PartialEq, Eq)]` struct with `pub fn from_env(env: &service_kit::env::Env) -> Result<Config, String>` | host `compose.rs`, `content-plugin`, parity tests |
| `content::server::run_with_modules` | `async fn(config: Config, ctx: service_kit::context::ServiceContext, modules: Arc<ContentModules>) -> Result<(), String>`; the future is `Send` | host `compose.rs`, `content-plugin` |
| `content::server::run`, `run_with_context` | `run(config)` = standalone context named `"content"`; `run_with_context(config, ctx)` = `run_with_modules` with an empty table | kept for callers of the previous API |
| `content::server::prepare` | `fn(config: &Config) -> AppState`: everything up to listening, with an empty module table | `tests/parity.rs` |
| `content::server::serve` | `async fn(listener: tokio::net::TcpListener, app: AppState, stop: tokio::sync::watch::Receiver<bool>)`: serves until `stop` turns true | `tests/parity.rs` |
| `content::app::AppState` | the type `prepare` returns and `serve` takes; `Clone` | `tests/parity.rs` (by inference only) |
| `content::log::set_sink` | re-export of `service_kit::linelog::set_sink` (the whole `linelog` API re-exported as `content::log`) | parity tests |
| `content::modules::*` | Category A, moves as is: `ContentModules`, `RegistryView`, `NoModuleDirs`, `VersionsBody`, `VersionSummary`, `EpicManifestError`, `DEFAULT_VERSION`, `EPIC_MANIFEST_PATH`, `MAX_EPIC_MANIFEST_BYTES` | host `compose.rs`, `content-plugin` |
| `content::input::KEY_HEADER` | `&str` = `"x-reforged-user-api-key"` | fuzz |
| `content::input::Route` | enum, `Debug, Clone, Copy, PartialEq, Eq`, with exactly the variants `RuntimeManifest`, `Branding`, `News`, `BrandingImage(&str)`, `RuntimeFile(&str)`, `Versions`, `ClientProfile(&str)`, `EpicManifest(&str)`, `Unknown`, and `Route::of(path_only: &str) -> Route<'_>` | fuzz |
| `content::input::header_text` | `fn(&http::HeaderMap, name: &str) -> Option<String>` | fuzz |
| `content::input::if_none_match_hits` | `fn(header: Option<&str>, etag: &str) -> bool` | fuzz |
| `content::input::is_plausible_key` | `fn(&str) -> bool` | fuzz |
| `content::input::is_runtime_file_name` | `fn(&str) -> bool` | fuzz |
| `content::input::is_version_segment` | `fn(&str) -> bool` | fuzz |
| `content::input::version_query` | `fn(query: Option<&str>) -> Option<(VersionKey, String)>`, `VersionKey` an enum with `Version` and `Edition` | fuzz |
| `content::input::parse_runtime_json` | `fn(&[u8]) -> Result<protocol::manifest::RuntimeManifest, String>` | fuzz |
| `content::input::parse_news` | `fn(&serde_json::Value) -> Result<Vec<NewsItem>, String>`, `NewsItem { pub date: String, pub title: String, pub body: String }` | fuzz |

`input` holds every function that reads outside input as a pure function, so the fuzz targets can drive them; keep
that split.

Shared MIT building blocks the rewrite uses (all in dainty-lass): `protocol::manifest` (`RUNTIME_MANIFEST_PATH`,
`RUNTIME_FILE_NAMES`, `RUNTIME_DLL`, `LEGACY_RUNTIME_DLL`, `DEFAULT_CHANGELIST`, `MAX_RUNTIME_FILE_BYTES`,
`RUNTIME_BUILD_NAME`, `RuntimeManifest::new`/`to_json`, `runtime_file_slot`); `protocol::invite::reforged_version`;
`service_kit::env::Env`, `service_kit::js::{trim, number, safe_integer, date_parse, string}`,
`service_kit::jsjson::{stringify, parse}`, `service_kit::linelog`, `service_kit::context::ServiceContext`;
`module_api` (manifest, registration, `CorePlugin`, `GameVersion`, `ClientProfile`).

**JSON objects keep insertion order everywhere** (`serde_json` with `preserve_order`; `service-kit` already turns the
feature on, and the crate should name it itself). Every JSON text the server writes is `service_kit::jsjson::stringify`
of an object built in the field order this spec gives, except the two module-api bodies of section 2.6.7–2.6.8.

### 2.2 Configuration (`content.env`)

The host kit writes `content.env` (dainty-lass `hostkit::hostconfig`): `PORT=61002`, `BIND_HOST=127.0.0.1`,
`BACKEND_URL=http://127.0.0.1:61000`, `CONTENT_RUNTIME_DIR=<repo>/.workspace/runtime`,
`CONTENT_BRANDING_DIR=<repo>/.workspace/branding`, `CONTENT_NEWS_FILE=<repo>/.workspace/config/news.json`. The other
settings are read only if a host adds them. `Config::from_env` reads the `Env` it is given (process environment over
file, `service_kit::env`) once; the first bad setting is an `Err(String)` and stops the start.

Settings are read in this order, which decides which error is reported first:

| # | Name | Default | Rule | Error text |
| --- | --- | --- | --- | --- |
| 1 | `CONTENT_GAME` | `reborn` | Raw value trimmed and ASCII-lowercased. Unset, empty or `reborn` is the 1.14.7 game (`protocol::invite::reforged_version()`). | `legacy`: `CONTENT_GAME=legacy (Dauntless Revived's 1.4.4) is no longer served; this content server serves reborn only`. Anything else: `CONTENT_GAME must be reborn (got <raw>)`, `<raw>` the untrimmed value written as a JSON string |
| 2 | `PORT` | `61002` | Whole number 1–65535 (the number rule below) | `PORT must be a whole number from 1 to 65535` |
| 3 | `BIND_HOST` | `127.0.0.1` | Value trimmed; empty means the default. Split on `,`, each part JS-trimmed, empty parts dropped. Each must be a loopback host (below). | No parts left: `BIND_HOST is empty`. A non-loopback part: `BIND_HOST must be a loopback address (got "<part>"): players reach the content server through the gateway` (the part as a JSON string) |
| 4 | `BACKEND_URL` | `http://127.0.0.1:61000` | Value trimmed, empty means the default; every trailing `/` removed; the result is kept as text (not normalised). It must parse as a WHATWG URL with scheme `http` and a host that, with any `[`/`]` removed, is a loopback host. | Does not parse: `BACKEND_URL is not a URL`. Other scheme or host: `BACKEND_URL must be http:// on this machine (the account keys travel in it)` |
| 5 | `CONTENT_NEWS_URL` | none | Value trimmed, empty means none. See section 2.13.3 for the rule. Kept as the URL's normalised serialisation. | `CONTENT_NEWS_URL must be a raw.githubusercontent.com HTTPS file URL` |
| 6 | `CONTENT_RUNTIME_DIR` | none | Trimmed, empty means none; made absolute against the working folder (`std::path::absolute`, falling back to the text as given). None is not an error: no runtime is offered. | — |
| 7 | `CONTENT_BRANDING_DIR` | none | As 6 | — |
| 8 | `CONTENT_NEWS_FILE` | none | As 6 | — |
| 9 | `CONTENT_MAX_STREAMS_PER_ACCOUNT` | `4` | Whole number 1–64 | `CONTENT_MAX_STREAMS_PER_ACCOUNT must be a whole number from 1 to 64` |
| 10 | `CONTENT_MAX_STREAMS_TOTAL` | `48` | Whole number 1–1024 | `CONTENT_MAX_STREAMS_TOTAL must be a whole number from 1 to 1024` |
| 11 | `CONTENT_AUTH_CACHE_SECONDS` | `300` | Whole number 0–3600 | `CONTENT_AUTH_CACHE_SECONDS must be a whole number from 0 to 3600` |

- **The number rule:** an unset value, or one that is empty after trimming whitespace, takes the default. Otherwise the
  raw value is converted as JavaScript's `Number(string)` does (`service_kit::js::number` of the value as a JSON
  string: `" 5 "` is 5, `"1e1"` is 10, `"0x10"` is 16, `"1.5"` and `"abc"` are not whole) and must be a whole number
  within the bounds.
- **A loopback host** is exactly `localhost`; or a dotted IPv4 address in strict form (four decimal parts, no leading
  zeros, as Rust's `Ipv4Addr` parses) that starts with `127.`; or exactly `::1`. Nothing else (not `[::1]`, not
  `0.0.0.0`, not `::`, not a name).
- `CONTENT_GAME_DIR`, `CONTENT_MANIFEST`, `CONTENT_DOWNLOAD_BASE_URL` and `CONTENT_ALLOW_ANY_BIND` (DR's settings) are
  ignored without error.
- `Config` carries, publicly: `version` (`GameVersion`), `port` (`u16`), `bind_hosts` (`Vec<String>`, as written
  after trimming), `backend_url` (`String`), `runtime_dir`, `branding_dir`, `news_file` (`Option<PathBuf>`),
  `news_url` (`Option<String>`), `max_streams_per_account`, `max_streams_total` (`usize`), `auth_cache_seconds` (`u64`).
- The address a bind host listens on: `localhost` is `127.0.0.1` only; anything else is parsed as an IP address.

### 2.3 Command line (`content serve`)

- `content [serve] [--env-file <path>]`; `serve` is the only subcommand and the default. clap metadata: name
  `content`, the crate version, about `DaintyLass content server`; `serve`'s help is `Serves the game runtime, news
  and art pack.`
- Without `--env-file`, a file named `.env` in the working folder is read if it is a regular file (as
  `node --env-file-if-exists=.env` did). See section 5, item 5.
- The environment is loaded with `service_kit::env::Env::load`. A load error or a `Config::from_env` error is logged
  at `error` level as `Configuration: <error>` with no other fields, and the exit code is failure.
- A multi-threaded tokio runtime runs `server::run(config)`; a runtime build error is logged at `error` with its text;
  `run` returning `Err(text)` logs `text` at `error`. Success exits 0.

### 2.4 Start, listeners, connections and stop

`run_with_modules(config, ctx, modules)`:

1. `ctx.status().report_starting("preparing content server")`.
2. Prepare the app (below).
3. For each bind host in order, bind a TCP listener on `<address>:<port>`. Failure returns
   `Err("Could not listen on <host>:<port>: <io error>")`; a host that is not an address,
   `Err("Could not listen on <host>:<port>: not an address")`. Each listener is served on its own task.
4. `ctx.status().report_ready(Some(port), "listening")`, then log `info` `content server ready` with fields, in order:
   `listen` (array of `"<host>:<port>"` for each bind host), `backend` (the backend URL), `runtime` (the runtime folder
   as displayed, or `null`), `branding` (idem), `news` (the news file as displayed, else the news URL, else `null`),
   `maxStreamsPerAccount`, `maxStreamsTotal`.
5. Wait for `ctx.wait_stop()`. Then `report_stopping("stopping")`, log `info` `stopping` with `{"signal":"stop"}`, tell
   every listener to stop and wait for them, `report_stopped("stopped")`, return `Ok(())`.

**Preparing** (also `prepare(config)`, with an empty `ContentModules`): a clock of milliseconds since 1970; the runtime
store for `runtime_dir` (section 2.11), checked once now. If that first status is not OK, log `warn` `no runtime is
offered to players yet; the launcher shows them that the host has to publish one` with `dir` (the folder as displayed,
or `null`) and `reason`. Then the account check (section 2.8) against `backend_url` with a 5 s lookup timeout and
`auth_cache_seconds`; the download slots (section 2.10); branding (section 2.12); news (section 2.13); and the module
table given.

**Serving a listener** (`serve`):

- HTTP/1.1 only. Accept until `stop` is true; an accept error is skipped.
- At most 512 open connections per listener. A connection accepted beyond that is closed at once (Node's
  `maxConnections`).
- `TCP_NODELAY` on. The request head must arrive within 20 s.
- A connection on which nothing could be read or written for 120 s fails with a time-out (Node's
  `server.setTimeout(120000)`), so a client that stops reading a download frees its slot.
- Each request is passed to the handler with the peer address. When `stop` turns true, the accept loop ends and every
  open connection is aborted.

### 2.5 Requests: rules common to every route

**Matching.** The request target's path and query, split at the first `?` into the path and the query. The path is
matched byte for byte: no percent-decoding, no dot-segment removal, no case folding, no trailing-slash tolerance. The
route is decided **before** the method:

| Path | Route |
| --- | --- |
| exactly `/content/v1/runtime/manifest` | runtime manifest |
| exactly `/content/v1/branding` | branding |
| exactly `/content/v1/news` | news |
| starts with `/content/v1/branding/` | branding image; the rest of the path is the name (may be empty) |
| starts with `/content/v1/runtime/files/` | runtime file; the rest of the path is the name (may be empty, may contain `/`) |
| exactly `/content/v1/versions` | versions |
| `/content/v1/versions/<v>/client` | client profile, when `<v>` is a version segment |
| `/content/v1/versions/<v>/epic/manifest` | Epic manifest, when `<v>` is a version segment |
| anything else | unknown |

The tests are made in the order of the table. A **version segment** is 1–64 bytes, each a lowercase ASCII letter, a
digit, `.` or `-` (so no `/`, `%`, uppercase). `/content/v1/versions/` followed by anything that is not one of the two
shapes is unknown.

**Methods.** An unknown route is `404 not_found` whatever the method. A known route with a method other than `GET` or
`HEAD` is `405 method_not_allowed` with `allow: GET, HEAD`. `HEAD` is answered as `GET` with the same status and
headers (including `content-length`) and no body, unless a route says otherwise.

**Headers on every reply:** `x-content-type-options: nosniff`. No CORS headers, no `accept-ranges`, no
`content-range`: `range` and `if-range` are ignored (always the whole file, status 200).

**JSON replies** (every error, and the JSON routes): status as given, `content-type: application/json;
charset=utf-8`, `content-length: <bytes of the body>`, `cache-control: no-cache`, then any extra header the case names
(`allow`, `retry-after`, `etag`). The body is the JSON text as specified. An **error** is the JSON reply
`{"error":"<code>"}`.

**304 replies** carry only `x-content-type-options`, `etag` and, for a branding image, `cache-control`. No
`content-type`, no `content-length` (also for `HEAD`), no body.

**Request headers** are read as Node reads `req.headers`: all lines of that name, each byte taken as one Latin-1
character, joined with `", "` (`header_text`). Absent is `None`.

**Conditional requests** (`if_none_match_hits(header, etag)`): no header is a miss. A header whose JS-trimmed whole value
is `*` is a hit. Otherwise the header is split on `,`; a part hits when, JS-trimmed and with one leading `W/` removed,
it equals `etag` exactly. (`"*, \"x\""` is not a `*` and misses; a repeated header line is joined first, so
`"nope"` + `<etag>` on two lines hits.) Each route says which ETag it compares against.

**The client address** for the log is the `x-forwarded-for` header text as is (the gateway sets it), else the peer IP.

**The version query** (`version_query`): the query is decoded as `application/x-www-form-urlencoded`; the first pair
whose key is `version` or `game` gives `(Version, value)`, or whose key is `edition` gives `(Edition, value)`; later
pairs are ignored; anything else in the query is ignored. A requested version is then resolved through the module
table: a `Version` value with `ContentModules::resolve`, an `Edition` value with `resolve_edition`. A value that resolves
to nothing is ignored, i.e. the server's own runtime is used.

### 2.6 Routes

#### 2.6.1 `GET|HEAD /content/v1/runtime/manifest` (route name `runtime-manifest`; no key)

1. Take the runtime status for the requested version (section 2.11.4), off the async threads (it may hash files). A
   failed background task counts as invalid with reason `runtime check failed`.
2. Nothing published: `404 no_runtime` (log reason = the status reason).
3. Published but not checking out: `503 runtime_invalid` (log reason = the status reason).
4. OK: if `if-none-match` hits the runtime ETag (`"<sha256 hex of the body>"`), `304` with that `etag`.
5. Else `200`, the JSON reply whose body is the runtime body byte for byte (section 2.11.3), plus `etag`.

#### 2.6.2 `GET|HEAD /content/v1/runtime/files/<name>` (route name `runtime-file`)

In this order:

1. **Version prefix.** If `<name>` contains `/`, split at the first `/`; if the first part resolves through
   `ContentModules::resolve`, that is the requested version and the second part is the file name. Otherwise the whole
   `<name>` is the file name (and, containing `/`, it fails step 2).
2. **Name.** The file name must be exactly `Rustic-Isles.dll`, `winmm.dll` or `MysticParadox.dll` (the runtime's name
   before DX4), letter case included (`protocol::manifest::runtime_file_slot` is `Some`). Else `404 not_found`, log
   reason `not_a_runtime_file`. From here the log's `path` is the file name.
3. **Key** (section 2.8): no `x-reforged-user-api-key` header or an empty one is `401 unauthorized` (reason
   `no_key`); a refused key is `401 unauthorized` (reason `bad_key`); a backend that cannot answer is
   `503 auth_unavailable` with `retry-after: 5` (reason = the lookup's reason). The log records the account id and
   username from here on.
4. **Listed file.** Find the file in the checked list of the requested version's runtime (section 2.11.4), off the async
   threads: the list entry whose slot equals the requested name's slot (so `Rustic-Isles.dll` and `MysticParadox.dll`
   name the same entry). The path served is the runtime folder joined with the **listed** name, never the requested
   one. No OK runtime, or no such entry: `404 not_found`, reason `no_runtime`.
5. **Condition.** ETag `"<listed sha256>"`. A hit is `304` with `etag`.
6. **HEAD:** `200` with `etag`, `content-type: application/octet-stream`, `content-length: <listed size>`, no
   `cache-control`, no body, no slot taken.
7. **Slot** for the account id (section 2.10): account limit is `429 too_many_streams`, `retry-after: 2`, reason
   `account_limit`; server limit is `503 server_busy`, `retry-after: 10`, reason `total_limit`.
8. **Open** the file. Failure: forget the runtime check (section 2.11.4), `503 file_unavailable`, reason `open_failed`,
   no `retry-after`. Reading the open file's metadata failing: `503 file_unavailable`, no reason, no `retry-after`.
9. **Changed on disk:** not a regular file, or its length differs from the listed size: forget the runtime check,
   `503 file_unavailable`, `retry-after: 10`, reason `changed_on_disk`.
10. `200`: `etag`, `content-type: application/octet-stream`, `content-length: <listed size>`, `cache-control: private,
    no-transform`; the body is the file's first `<listed size>` bytes, streamed in reads of 256 KiB. The slot taken in
    step 7 is held from step 7 until the reply body is finished or dropped (also for the 503s of steps 8–9).

The query string is ignored on this route.

#### 2.6.3 `GET|HEAD /content/v1/branding` (route name `branding`; no key)

`200`, the JSON reply of the current branding body (section 2.12.4). No ETag.

#### 2.6.4 `GET|HEAD /content/v1/branding/<name>` (route name `branding-image`; no key)

1. `<name>` must be a safe image name (section 2.12.1) and a file of the current branding scan, exactly; else
   `404 not_found`. From here the log's `path` is the name.
2. Open the folder's file of that name and read its metadata; failure, not a regular file, length 0 or above 25 MiB:
   `404 not_found`.
3. ETag `W/"<length in lowercase hex>-<modification time in whole ms since 1970, lowercase hex>"` (0 when the time is
   unknown). Set `etag` and `cache-control: public, max-age=300`.
4. Condition: compare `if-none-match` against the ETag **without** its `W/` prefix (so `W/"…"` and `"…"` both hit). A
   hit is `304` (with `etag` and `cache-control`).
5. `200`: also `content-type` (by extension, section 2.12.1) and `content-length`; body the file streamed in 256 KiB
   reads (none for `HEAD`). No key, no slot.

With the fixture mtime of 1 700 000 000 s, an 8-byte file's ETag is `W/"8-18bcfe56800"`.

#### 2.6.5 `GET|HEAD /content/v1/news` (route name `news`; no key)

`200`, the JSON reply of the current news body (section 2.13.4). No ETag. Each call may start the background GitHub
refresh.

#### 2.6.6 `GET|HEAD /content/v1/versions` (route name `versions`; no key)

`200`, JSON reply (no ETag) whose body is `serde_json::to_string` of `ContentModules::versions()`: field names are
module-api's / `VersionsBody`'s own snake_case names, e.g. with nothing attached
`{"default_version":"1.14.7","versions":[]}` and with one module
`{"default_version":"1.14.7","versions":[{"game_version":"1.12.0","module":"isles-1-12","display_name":"Rustic Isles 1.12","invite_aliases":["isles-112"],"unverified":true,"client":true,"epic_manifest":true}]}`.
If serialisation fails, `500 internal`.

#### 2.6.7 `GET|HEAD /content/v1/versions/<v>/client` (route name `client-profile`; no key)

The log's `path` is `<v>`. `<v>` resolves through `ContentModules::resolve`; no version, or a version with no attached
module (a built-in one such as `1.14.7` included), is `404 unknown_version`; an attached module without a client
profile is `404 no_client_profile`. Otherwise the body is `serde_json::to_string` of the module's
`module_api::ClientProfile` (module-api's field names and order), ETag `"<sha256 hex of that body>"`; a hit is `304`
with `etag`, else `200` JSON reply plus `etag`. Serialisation failure is `500 internal`.

#### 2.6.8 `GET|HEAD /content/v1/versions/<v>/epic/manifest` (route name `epic-manifest`)

The log's `path` is `<v>`.

1. `<v>` must resolve (`ContentModules::resolve`), else `404 unknown_version` **before** any key check.
2. Key, exactly as section 2.6.2 step 3 (so a built-in version without a module, e.g. `1.14.7`, answers 401 without a
   key and 404 with one).
3. `ContentModules::epic_manifest(version)`, off the async threads (a failed task is `Unavailable("manifest check
   failed")`): `UnknownVersion` is `404 unknown_version`; `NotServed` is `404 no_epic_manifest`; `Unavailable(reason)`
   is `503 epic_manifest_unavailable` (log reason = reason); `Mismatch{expected, found}` is `503 epic_manifest_invalid`
   with log reason `its SHA-1 is <found>, the client profile pins <expected>`.
4. ETag `"<pinned sha1 hex>"`; a hit is `304` with `etag`.
5. HEAD: `200` with `etag`, `content-type: application/octet-stream`, `content-length`, no body, no slot.
6. Slot as section 2.6.2 step 7 (same codes and `retry-after`).
7. `200` with `etag`, `content-type: application/octet-stream`, `content-length`, `cache-control: private,
   no-transform`; body the bytes. The slot is held until the body is done or dropped.

### 2.7 The request log

Each request writes exactly one line, when its reply body is finished or dropped (Node's `res.on("close")`), through
`service_kit::linelog` (so the line starts `t`, `level`, `msg`). Fields in this order:

| Field | Value |
| --- | --- |
| `method` | the request method as given |
| `route` | `runtime-manifest`, `branding`, `news`, `branding-image`, `runtime-file`, `versions`, `client-profile`, `epic-manifest`; `other` for an unknown route **and for a 405** (the method is checked before the route is named) |
| `path` | what the route set (section 2.6), else the request path without the query, cut to its first 200 UTF-16 units |
| `status` | the reply status |
| `ms` | whole milliseconds from the request's arrival to this line |
| `ip` | section 2.5 |
| `reason` | only when the route set one |
| `account`, `user` | only once a key was accepted: the account id and username |
| `bytes`, `expected`, `complete` | only for a streamed download (the 200 of 2.6.2 or 2.6.8): bytes handed to the connection, the size promised, and whether they are equal |

Level `warn` when the status is 400, 401 or ≥ 500; `info` otherwise (429 is `info`). Message `download` when the route
is `runtime-file` or `epic-manifest`, the method is `GET` and the line carries `bytes`; `request` otherwise.

The account key never appears in any log line (test: section 3.3, item L1).

### 2.8 Account check (`x-reforged-user-api-key`)

**Plausible key:** 1–256 bytes, every byte 0x21–0x7E (printable ASCII, no space). Anything else is refused without
asking the backend (a joined repeated header contains `", "` and is therefore refused).

**The lookup** (the backend): `GET <BACKEND_URL with trailing slashes removed>/reforged/api/GetUserInfo` with headers
`x-reforged-user-api-key: <key>` and `accept: application/json`, through the client of section 2.9 with a 5 s timeout
and a 1 MiB body limit.

| Backend answer | Result | Unavailable reason |
| --- | --- | --- |
| Transport error, time-out or redirect | unavailable | `backend unreachable: <client error text>` |
| 401 or 403 | refused | — |
| Any other status but 200 | unavailable | `backend answered <status>` |
| 200, body over the limit or not JSON | unavailable | `backend answered with bad JSON` |
| 200, `UserId` missing, not a string or empty | unavailable | `backend answer has no UserId` |
| 200 with a `UserId` | account: id = `UserId`; username = `Username` if a string, else `""`; admin = `IsAdmin` is JSON `true` | — |

**The cache:**

- Keyed by the lowercase hex SHA-256 of the key; the key itself is never stored.
- An entry is used while its expiry is later than now (ms). An expired entry found on lookup is removed.
- An account answer lives `CONTENT_AUTH_CACHE_SECONDS` (0 means it is never served from the cache); a refused key
  30 s; an unavailable answer is never cached. The expiry is computed when the lookup finishes.
- At most 2000 entries; inserting beyond that drops the oldest-inserted first (a re-inserted key counts as newest).
- Concurrent checks of the same key share one backend lookup; only one of them stores the result.
- A lookup that panics is unavailable with reason `lookup failed`, and is not cached.

### 2.9 Outgoing HTTP client

Used for the account lookup and the GitHub news. HTTP/1.1 over plain HTTP or HTTPS (rustls with the aws-lc-rs
provider and the webpki roots; if that provider cannot be set up, rustls's default with webpki roots). One shared
client per process.

- GET with the given headers and an empty body. The whole exchange, body included, must finish within the timeout,
  else the error is `The operation was aborted due to timeout`.
- A connection or protocol error is `fetch failed: <error>`.
- Statuses 301, 302, 303, 307 and 308 are the error `fetch failed: unexpected redirect` (as `fetch` with
  `redirect: "error"`). A 304 is an answer, not a redirect.
- The reply gives the status, the headers, and the body read up to the limit; a body over the limit is a body error
  (the status is still returned).

### 2.10 Download slots

- Per account at most `CONTENT_MAX_STREAMS_PER_ACCOUNT` streams at once; the server at most
  `CONTENT_MAX_STREAMS_TOTAL`. The per-account limit is checked first.
- Acquiring checks both and counts one for the account and one in total; refusal names `account_limit` or
  `total_limit`. Releasing (when the slot is dropped) counts both down once; an account with zero is forgotten.
- Only a `GET` that reached the slot step takes one (never `HEAD`, never a 304 or an earlier error).

### 2.11 The runtime (`CONTENT_RUNTIME_DIR`)

#### 2.11.1 `runtime.json` (`parse_runtime_json`)

Bytes → text with invalid UTF-8 replaced, one leading U+FEFF removed, parsed with `service_kit::jsjson::parse`. A
parse error is `runtime.json: <parser error text>`; a shape error is `runtime.json: <reason>` with the reasons below,
checked in this order:

1. Not an object: `runtime.json must be an object`.
2. `version`: a string, JS-trimmed, 1–48 bytes of `A–Z a–z 0–9 . _ + -`. A missing or non-string version counts as
   empty. Else `version must be 1-48 letters, digits, dots, dashes, plus signs or underscores`.
3. `build.changelist`: only read when `build` is a JSON object (an array is not) that has the key. Present, it must be
   a JavaScript safe integer greater than 0 (`service_kit::js::safe_integer`: a JSON number that is whole, so `5.0`
   counts; `null`, strings and 2^53 do not). Else `build.changelist must be a positive whole number`. Absent: 647472
   (`DEFAULT_CHANGELIST`). Any `build.name` the host wrote is ignored.
4. `files` must be an array: `files must be a list`.
5. Each entry, in order (an entry that is not an object is treated as an empty object):
   - `name` must be a string that is a runtime file name (section 2.6.2 step 2), else
     `unknown file <name> (only Rustic-Isles.dll and winmm.dll)`, `<name>` the value as `JSON.stringify` writes it
     (`null`, `5`, `"dxgi.dll"`), or `undefined` when missing.
   - Its slot must not already be listed: `<name> is listed twice` (`MysticParadox.dll` and `Rustic-Isles.dll` are one
     slot).
   - `size`: a safe integer from 1 to 67108864 (`MAX_RUNTIME_FILE_BYTES`): else `<name>: size must be 1 to 67108864
     bytes`.
   - `sha256`: a string of exactly 64 lowercase hex characters: else `<name>: sha256 must be 64 lowercase hex
     characters`.
6. Both slots present: for `Rustic-Isles.dll` then `winmm.dll`, a missing slot is `<name> is missing from the list`.

The result is `RuntimeManifest::new(trimmed version, changelist, files)`: files sorted by name, build name always
`rel-1.14.7-Archon`. Its body (section 2.11.3) is `RuntimeManifest::to_json()`.

#### 2.11.2 Checking a folder

For a folder `dir`:

1. `dir/runtime.json` (following links): it exists but is not a regular file, or is larger than 65536 bytes:
   **invalid**, `runtime.json is not a regular file under 64 KB`. Missing or unreadable: **none**, `no runtime.json yet
   (the host has not published a runtime)`.
2. Parse it (2.11.1); an error is **invalid** with that text.
3. For each listed file, in list order, at `dir/<listed name>` **without following links**: cannot be read →
   `<name> is missing`; not a regular file (a link or a folder is not) → `<name> is not a regular file`; another
   length → `<name> is <actual> bytes, runtime.json says <listed>`; the SHA-256 of its bytes (read in 256 KiB blocks;
   a read error → `<name> is missing`) differs → `<name> does not match its sha256 in runtime.json`. The first failure
   is **invalid** with that text.
4. Otherwise **OK**, with the manifest, its body and its ETag.

#### 2.11.3 The body and ETag

The body is the manifest's compact JSON, field order `version`, `build` (`name`, `changelist`), `files` (each `name`,
`size`, `sha256`). The ETag is `"` + lowercase hex SHA-256 of the body + `"`. The same `runtime.json` always gives the
same body and ETag. `protocol::test_data::RUNTIME_MANIFESTS` holds two input/body pairs recorded from Node.

#### 2.11.4 The server's runtime and versioned runtimes

**The server's own runtime** (no version, or one with no folder):

- No `CONTENT_RUNTIME_DIR`: always **none** with reason `CONTENT_RUNTIME_DIR is not set`, never logged by the store.
- Otherwise the status is cached. A request checks again only if 10 s or more passed since the last check, or the
  check was forgotten. Before the first check the status is none, `not checked yet`.
- A check first looks at the existence, length and modification time (ns) of `runtime.json`, `Rustic-Isles.dll`,
  `winmm.dll` and `MysticParadox.dll` in the folder. If none of those changed since the last load, the cached status
  stands (no re-hash). Otherwise the folder is loaded again (2.11.2).
- After a load:
  - OK, and the previous status was not OK or had another version: log `info` `runtime ready` with `version` and
    `files` (array of `{name, size, sha256}` in list order).
  - Not OK, and the previous status was OK or had another reason text: log `no runtime is offered to players` with
    `dir` (displayed) and `reason`, at `warn` for invalid and `info` for none.
- **Forgetting the check** (on a served file that failed to open or changed, section 2.6.2) clears both the time and
  the remembered file signature, so the next request loads the folder again.

**A version's runtime:** for a resolved version, its folder names are tried in order: the version's own text, then the
attached module's invite aliases, then the built-in alias (`reforged` for 1.14.7, `revived` for 1.4.4); only names of
lowercase letters, digits, `.` and `-` not starting with `.` count; duplicates are dropped (this is
`ContentModules::runtime_folders`). The first `<runtime dir>/<name>` that is a directory is checked **now**, every
time (no cache, no log), and serves both the manifest and files for that request. If none exists, or there is no runtime
dir, the server's own runtime answers. So `?game=revived` reads `<dir>/revived/` when present, and
`files/reforged/winmm.dll` with no `reforged/` folder serves the server's own file.

### 2.12 Branding (`CONTENT_BRANDING_DIR`)

#### 2.12.1 Image names and types

- **Safe image name:** at most 100 bytes; split at the last `.` into stem and extension; the extension, any case, is
  `jpg`, `jpeg`, `png` or `webp`; the stem, split on `.`, has only non-empty parts of `A–Z a–z 0–9 _ -`, and its first
  part starts with a letter or digit. (`Harbour-Dusk_2.JPEG`, `bg.v2.webp` are safe; `.a.jpg`, `a..jpg`, `a b.jpg`,
  `a/b.jpg`, `a.jpg.exe`, `a.gif` are not.)
- **Content type** by lowercase extension: `png` → `image/png`, `webp` → `image/webp`, otherwise `image/jpeg`.
- **Signature** (the first bytes): JPEG `FF D8 FF`; PNG `89 50 4E 47 0D 0A 1A 0A`; WebP `RIFF` at 0–3 and `WEBP` at
  8–11. A file whose first 12 bytes do not match its type is never served.

#### 2.12.2 The scan

No folder configured: no art. Otherwise:

1. List the folder (names that are valid UTF-8). Failure: log `warn` `branding folder unreadable` with `dir`, `error`;
   no art.
2. `branding.json`, if it exists: read as UTF-8, one leading BOM removed, parsed with `serde_json`. An object is the
   config; a non-object logs `warn` `branding.json is not an object; ignoring it` (`dir`); a read or parse error logs
   `warn` `branding.json is not valid JSON; ignoring it` (`dir`, `error`). There is no size limit on it.
3. The wanted list: if the config's `backgrounds` is an array, each item in order: a string is a file with no credit;
   an object gives `file` and `credit`; anything else is an entry with neither. Otherwise every safe image name in the
   folder, sorted by bytes, no credits.
4. For each wanted entry, stopping once 50 images are accepted:
   - The file must be a string, a safe image name, present in the folder listing exactly, and not already accepted.
     Else log `warn` `branding image skipped (bad name, missing or listed twice)` with `file`: the value shown as
     JavaScript's `String()` would (`undefined` when missing, the string itself, `[object Object]` for an object,
     `5`, `null`…), cut to 120 UTF-16 units.
   - Its metadata **without following links**: unreadable → `warn` `branding image skipped (unreadable)` (`file`,
     `error`); not a regular file, empty, or over 25 MiB (26214400 bytes) → `warn` `branding image skipped (not a
     regular file, empty or over 25 MB)` (`file`).
   - Its first 12 bytes: unreadable → the `unreadable` warning; signature mismatch → `warn` `branding image skipped
     (contents don't match the extension)` (`file`).
   - Accepted: name, cleaned credit, content type, length, modification time.
5. Accent: if the config has `accent` and it does not clean, log `warn` `branding.json accent ignored: use #rgb or
   #rrggbb` (no fields).

**Cleaning:**

- *Accent:* a string, JS-trimmed, `#` followed by exactly 3 or 6 hex digits; the result is the trimmed text in
  lowercase. Anything else is none.
- *Credit:* a string only; every character below U+0020 and U+007F becomes a space; every run of JavaScript `\s`
  whitespace (Unicode whitespace except U+0085, plus U+FEFF) becomes one space; JS-trimmed; cut to 200 UTF-16 units
  (section 2.13.2); empty is none.

#### 2.12.3 Re-reading

The scan is kept and redone when a request finds no scan yet, or 10 s or more since the last one.

#### 2.12.4 The body

`{"backgrounds":[{"url":"/content/v1/branding/<file>","credit":<string or null>},…],"accent":<string or null>}`.
With no art: `{"backgrounds":[],"accent":null}`.

### 2.13 News

#### 2.13.1 The document (`parse_news`)

- Accepted shapes: an array, or an object whose `items` is an array. Anything else is the error
  `news file must be {"items": [...]} or an array`.
- Each entry in order, with a `warn` line for each one skipped:
  1. Not an object: `news item skipped: not an object` (no fields).
  2. `date` and `title` not both strings: `news item skipped: needs a date and a title` (no fields).
  3. `date` not understood by JavaScript's `Date.parse` (`service_kit::js::date_parse`), or outside the range a UTC
     date-time can show: `news item skipped: date not understood` with `date` = its first 40 UTF-16 units.
  4. The title, cleaned (below), empty: `news item skipped: empty title` (no fields).
  5. Accepted: `date` as UTC `YYYY-MM-DDTHH:MM:SS.mmmZ`; `title` cleaned; `body` cleaned if a string, else `""`.
- Sort by `date` text descending; equal dates keep their order (stable). Keep the first 50.

#### 2.13.2 Text cleaning

- `\r\n` and then any remaining `\r` become `\n`.
- Title: every character below U+0020, and U+007F, becomes a space (so `"  Newest\n title "` → `Newest  title`; runs
  are **not** collapsed).
- Body: characters U+0000–U+0008, U+000B–U+001F and U+007F are removed; `\t` and `\n` stay.
- Then JS-trim, then cut to the limit (title 200, body 10000 UTF-16 units). The cut keeps whole characters: a
  character whose UTF-16 units would cross the limit is dropped with everything after it.

#### 2.13.3 Sources

**The file (`CONTENT_NEWS_FILE`):**

- Checked when a request finds no check yet, or 5 s or more since the last.
- Metadata (following links) fails: unless already in the missing state, log `warn` `news file not found; serving no
  news` (`file`), clear the items, enter the missing state.
- If length and modification time (ns) are unchanged since the last look, nothing happens. Otherwise remember them;
  then a non-regular file or one over 1 MiB (1048576 bytes) logs `warn` `news file skipped: not a regular file or over
  1 MB` (`file`) and keeps the items; else read as UTF-8, remove one leading BOM, parse with `serde_json`, then
  `parse_news`. Success logs `info` `news loaded` (`file`, `items`: the count) and replaces the items; any error logs
  `warn` `news file could not be read; keeping the last good version` (`file`, `error`) and keeps the last good items.

**The GitHub file (`CONTENT_NEWS_URL`):**

- Valid URL: parses as a WHATWG URL; scheme `https`; host exactly `raw.githubusercontent.com`; no explicit port in the
  serialised URL (the default `:443` is dropped by the parser and therefore accepted); no user name or password; no
  query and no fragment, not even empty ones; the path, split on `/`, has at least 5 parts (`/owner/repo/branch/file`).
- Each `GET /content/v1/news` starts one background refresh (on the tokio runtime) and returns at once with what is
  already known; the reply never waits for GitHub.
- A refresh does nothing if one is running or the last attempt began less than 60 s ago. It records the attempt time
  first, then GETs the URL (5 s, 1 MiB limit) with `If-None-Match: <stored ETag>` when one is stored.
- 304: no change, no log. Another non-2xx status: error `GitHub news HTTP <status>`. A body over the limit:
  `GitHub news exceeds 1 MB`. The body (UTF-8 lossy, BOM removed) parsed with `serde_json`: its error text;
  `parse_news`: its error. If the document listed at least one entry (the array's length, or `items`'s length) and none
  was usable: `GitHub news contains no usable entries`.
- Success replaces the remote items (an empty list included) and the stored ETag (the reply's `etag`, or none).
- Any error logs `warn` `GitHub news unavailable; keeping last good news` with `error` and keeps the last good remote
  items.

#### 2.13.4 The body

`{"items":[{"date":…,"title":…,"body":…},…]}`: the remote items once any remote fetch succeeded, else the file's
items, else `{"items":[]}`.

### 2.14 ContentModules (Category A, moves as is) — the contract the routes rely on

Summarised so the implementer of the routes knows what the table does; the file itself is not rewritten.

- It is the `CorePlugin` named `content` serving `[CoreCapability::ContentFiles]`. The controller (or `content-plugin`)
  attaches each loaded module's manifest and registration; the table is keyed by game version.
- `attach`: validates the manifest and checks the registration against it (an external module's manifest is
  untrusted), as `ErrorKind::Invalid` errors with their texts. A `Plugin`-kind module is accepted and ignored. A
  registered content file at `epic/manifest` without a client profile pinning an Epic build is refused (`epic/manifest
  is registered, but the client profile pins no Epic build to check it against`). Another module id for a version
  already served is refused (`content already serves <version> for <other id>`); the same id replaces its entry. Logs
  `info` `module content attached` (`module`, `version`, `client`, `epicManifest`).
- `detach(id)` removes that module's entries and logs `info` `module content detached` (`module`) if any were removed.
- `resolve(name)`: the version of the first attached module (in version order) whose version text or an invite alias
  equals `name`; else the built-ins `1.14.7`/`reforged` → 1.14.7 and `1.4.4`/`revived` → 1.4.4. `resolve_edition`:
  exactly `reborn` → 1.14.7, `legacy` → 1.4.4.
- `epic_manifest(version)`: the file the module registered at `epic/manifest`, under the module folder the registry
  view gives (refused if a link leads it out), a regular file of 1 byte to 64 MiB, served only while its SHA-1 equals the
  profile's pinned `manifest_sha1`; cached by (length, modification time).

### 2.15 The service library (`content-plugin`, Category A, moves as is)

A `cdylib` named `content_plugin` exporting one `service_plugin::Service` (`export_service!`):

- `name()` is `content`; `serves()` is `[ContentFiles]`.
- `attach(module, registration, dir)` records the module's folder (or forgets it when `dir` is none) **before**
  passing the module to the `ContentModules` it owns; that table's registry view answers from those recorded folders.
- `detach(id)` detaches from the table, then forgets the folder.
- `run(env, ctx)` = `Config::from_env(&env)?` then `server::run_with_modules(config, ctx, table)`. A config error is
  the run's `Err`, which the core reports as the component's failure.

The core loads it through `controller::service_library` with `ServiceLibrary { name: "content", sha256, path, serves:
&[ContentFiles] }` (dainty-restricted's `host::compose::content_library`), as an optional component that must be ready
within 60 s, on the same `content.env`.

### 2.16 Error catalogue

| Status | `error` | Extra headers | When |
| --- | --- | --- | --- |
| 404 | `not_found` | — | Unknown route; bad or unlisted runtime file name; no runtime file; any branding image refusal |
| 405 | `method_not_allowed` | `allow: GET, HEAD` | Known route, other method |
| 404 | `no_runtime` | — | Manifest: nothing published |
| 503 | `runtime_invalid` | — | Manifest: published, does not check out |
| 401 | `unauthorized` | — | No key, empty key, implausible or refused key |
| 503 | `auth_unavailable` | `retry-after: 5` | Backend cannot answer |
| 429 | `too_many_streams` | `retry-after: 2` | Account's slots full |
| 503 | `server_busy` | `retry-after: 10` | Server's slots full |
| 503 | `file_unavailable` | `retry-after: 10` (only when changed on disk) | Runtime file failed to open, stat, or changed |
| 404 | `unknown_version` | — | Version route: unknown version or no module |
| 404 | `no_client_profile` | — | Module without a client profile |
| 404 | `no_epic_manifest` | — | Module pins or registers no Epic manifest |
| 503 | `epic_manifest_unavailable` | — | Registered file not readable as required |
| 503 | `epic_manifest_invalid` | — | Registered file is not the pinned one |
| 500 | `internal` | — | A module-api body failed to serialise |

### 2.17 Constants

| What | Value |
| --- | --- |
| Default port / bind / backend | 61002 / `127.0.0.1` / `http://127.0.0.1:61000` |
| Connections per listener | 512 |
| Header time-out / idle time-out | 20 s / 120 s |
| Read chunk for file bodies | 256 KiB |
| Backend lookup time-out / body limit | 5 s / 1 MiB |
| Auth cache: refused TTL / max entries | 30 s / 2000 |
| `runtime.json` max / runtime file max | 64 KiB / 64 MiB |
| Runtime re-check | 10 s |
| Branding re-scan / max images / max image / max credit | 10 s / 50 / 25 MiB / 200 UTF-16 units |
| News file re-check / max file / GitHub re-fetch / GitHub time-out | 5 s / 1 MiB / 60 s / 5 s |
| News: max items / title / body | 50 / 200 / 10000 UTF-16 units |
| Logged path cut | 200 UTF-16 units |

---

## 3. Tests that Define the Behaviour

### 3.1 The parity fixtures (Category A data, kept)

`services/content/tests/parity/<name>.requests.json` and `<name>.expected.json` for `content`, `content-none`,
`content-invalid` and `content-index`. The expected replies were recorded from the Node server on the same scripts; a
failing parity test means the Rust side is wrong (AGENTS.md, "Parity tests are the contract"). They cannot be
re-recorded any more (the Node server is gone).

**Script format** (what the new harness must do with it):

- `env`: extra env pairs. `runtime`: `null` for no runtime folder, or `{files, index?}`. `branding`: `{files, json?}`.
  `news`: a document. `keys`: key → `{status, body}` for the fake backend. `requests`: the steps.
- File contents (`files` values, `spec`): `{"text": s}` is its UTF-8 bytes, `{"hex": h}` the decoded bytes,
  `{"pattern": n}` the `n` bytes `(i*7+3) & 0xff` for `i` in `0..n`.
- Every file the harness writes gets modification time 1 700 000 000 s after 1970 (so branding ETags match).
- `runtime` → folder `<tmp>/runtime` with the files; `runtime.json` is `index` if given (an object is written as
  `JSON.stringify` writes it, a string as is, with every `$sha:<file>` replaced by that file's SHA-256 hex), else
  `{"version":"20261014-1a2b3c4d","build":{"changelist":647472},"files":[{"name","size","sha256"} per file, in the
  script's order]}` compact. Env `CONTENT_RUNTIME_DIR=<tmp>/runtime` only when `runtime` is not null.
- `branding` → `<tmp>/art` with its files and `branding.json` from `json` (text as above);
  `CONTENT_BRANDING_DIR=<tmp>/art`. `news` → `<tmp>/news.json`; `CONTENT_NEWS_FILE` set.
- A fake backend on `127.0.0.1:0`: for any request, if the path is exactly `/reforged/api/GetUserInfo` and
  `x-reforged-user-api-key` is a key in `keys`, answer that `status` with `content-type: application/json` and the
  body (object as JSON text, string as is); else 401 with body `{}`. `BACKEND_URL` is its `http://` address.
- Steps: `{"write": name, "spec"}` rewrites `<tmp>/runtime/<name>`. Otherwise one raw request on a new connection:
  `<method or GET> <path exactly as written> HTTP/1.1\r\nhost: content\r\n`, then each `[name, value]` header with the
  first `$etag` in the value replaced by the last ETag any reply carried, then `connection: close\r\n\r\n`; read to
  end of stream.
- Each reply becomes `{"request": "<METHOD> <path>", "status", "headers", "body"}`: `headers` only those of
  `content-type`, `content-length`, `content-range`, `etag`, `accept-ranges`, `cache-control`, `retry-after`, `allow`,
  `access-control-allow-origin`, `cross-origin-resource-policy`, `x-content-type-options` that are present (names
  lowercased, values trimmed; compared as a set, order free); `body` the text if at most 4096 bytes of UTF-8 with no
  byte ≤ 0x08, else `<N bytes sha256 HEX>`. The list must equal `results` in the expected file, entry for entry.
- The whole script runs well within 10 s, so the runtime re-check of section 2.11.4 does not fire on its own between
  steps.

**Harness interface kept for `content-plugin`'s test:** a module at `services/content/tests/parity/harness.rs` with
`pub struct Server { pub port: u16, pub stop: Pin<Box<dyn Future<Output = ()> + Send>> }` and
`pub async fn run<F: Future<Output = Server>>(name: &str, launch: impl FnOnce(Vec<(String, String)>) -> F)`: it builds
the fixture, calls `launch` with the env pairs, runs the steps against `port`, awaits `stop`, then compares. It finds the
fixtures at `CARGO_MANIFEST_DIR/../content/tests/parity`, so it works from either crate.

**What each script checks:**

- `content` (runtime published with the pre-DX4 name `MysticParadox.dll` of 70000 bytes and `winmm.dll` of 3000; an
  art pack; news): unknown paths and case/trailing-slash variants 404; the manifest with and without a query, HEAD with
  `content-length`; `if-none-match` as the ETag, `W/`-prefixed, in a list, `*`, a miss, and split over two header lines;
  405 with `allow` for POST/DELETE/PUT on known routes and 404 for POST on an unknown one; DR's old routes
  (`/content/v1/files/x`, `/content/v1/manifest`) 404; every key case (none, empty, the old `x-undaunted-` header,
  403, 500, bad JSON, no `UserId`, unknown, a space, a repeated header) → 401 or 503 `auth_unavailable` with
  `retry-after: 5`; the DLL streamed, HEAD without `cache-control`, a query ignored; `winmm.dll` with 304 on GET and
  HEAD; `range`/`if-range` ignored (200, whole file); wrong case, `runtime.json`, `..`, `%2e%2e`, a sub-path, an
  empty name, `%2E`, `%00` all 404; branding body (credit collapsed, `fake.png` and `missing.jpg` and `5` skipped,
  accent lowercased); images with weak ETag and `public, max-age=300`, 304 keeping `cache-control`, refused names 404;
  news newest first with CRLF normalised and the bad date dropped; `x-forwarded-for` accepted; then `winmm.dll`
  rewritten to 2999 bytes: the file is `503 file_unavailable` `retry-after: 10`, the manifest then `503
  runtime_invalid`, the file `404`; rewritten back to 3000 bytes within 10 s: still `503`/`404` (the re-check
  interval).
- `content-none` (no runtime dir, no art, no news): manifest `404 no_runtime` (HEAD too); a file without a key 401,
  with a key 404 (key before lookup), HEAD 404; an unknown file name 404; `{"backgrounds":[],"accent":null}`;
  `{"items":[]}`.
- `content-invalid` (a wrong `winmm.dll` hash): manifest `503 runtime_invalid` (HEAD too), both files 404 with a key.
- `content-index` (a `runtime.json` written by hand: BOM, keys out of order, a build name, files reversed, `15.0` as a
  size, a padded `"  1.0+rr_2  "` version, changelist 651234): the normalised body and its ETag, 304 on it, the file
  served.

### 3.2 Black-box tests the implementer may reuse

| Path | Repo | What it checks |
| --- | --- | --- |
| `services/content/tests/parity/*.requests.json`, `*.expected.json` (8 files) | moved into dainty-lass with the crate | HTTP parity, through the new harness (section 3.1) |
| `crates/protocol/tests/fixtures/runtime-manifest*.json` via `protocol::test_data::RUNTIME_MANIFESTS` | dainty-lass (MIT) | `parse_runtime_json(input).to_json()` equals the recorded body, for both pairs |
| `services/content-plugin/tests/parity.rs` | moves with `content-plugin` (Category A) | The same four scripts through the built library and `controller::service_library` (needs `cargo build -p content-plugin` first) |
| `services/host/src/lib.rs` tests that start, stop and restart the `content` component on a `content.env` | dainty-restricted (runs there after the switch) | The compiled-in module starts, reports ready and stops |
| `fuzz/fuzz_targets/content_request.rs`, `content_runtime_index.rs`, `content_news.rs` | dainty-restricted | API compatibility of section 2.1 (they must still build) and the invariants of 3.3 R/N |

`services/content/tests/app.rs` and the in-file unit tests of the Category B files are **not** reusable: they are
Category B code and call internal APIs. Rewrite them from section 3.3.

### 3.3 Invariants the rewritten tests must cover

Config (C):

- C1 Defaults: port 61002, bind `["127.0.0.1"]`, backend `http://127.0.0.1:61000`, no runtime/branding/news, 4/48/300.
- C2 `0.0.0.0`, `100.101.102.103`, `203.0.113.7`, `::`, `my-host.example` refused ("loopback"); `127.0.0.1,::1` and
  `" localhost , 127.0.0.2 "` accepted as `["127.0.0.1","::1"]` and `["localhost","127.0.0.2"]`.
- C3 `BACKEND_URL` `http://203.0.113.7:61000` and `https://127.0.0.1:61000` refused ("this machine"); `not a url` →
  "not a URL"; `ftp://127.0.0.1` refused; `http://127.0.0.1:61000///` kept as `http://127.0.0.1:61000`, `[::1]`,
  `localhost` and `127.5.6.7` accepted.
- C4 `CONTENT_GAME` ` REBORN ` accepted; `1.4.4` → "CONTENT_GAME must be reborn"; `legacy` → "no longer served".
- C5 `PORT=0` refused; `CONTENT_MAX_STREAMS_PER_ACCOUNT=65` gives the full error text; `CONTENT_MAX_STREAMS_TOTAL=1.5`
  refused; `CONTENT_AUTH_CACHE_SECONDS=" 0 "` is 0; `CONTENT_RUNTIME_DIR=" runtime "` is absolute, ending in
  `runtime`.
- C6 `CONTENT_NEWS_URL`: `https://raw.githubusercontent.com/o/r/main/news.json` accepted; `https://example.com/feed`,
  `http://…`, `https://raw.githubusercontent.com@127.0.0.1/a/b/c/d`, a `?token=` query, a 3-part path, port 444 and
  a `#x` fragment refused.

Request input (R):

- R1 `if_none_match_hits`: none misses; ` * ` hits; `"x", "abc"` hits `"abc"`; `W/"abc"` hits; `"abcd"` misses;
  `*, "x"` misses.
- R2 Routes on the raw path: trailing slash unknown; `files/%2e%2e/runtime.json` is a runtime-file route whose name
  fails; `branding/` is an image route with an empty name; `/content/v1/files/x` unknown; `winmm.DLL` not a runtime
  name.
- R3 Version routes: `1.14.7/client` and `reforged/epic/manifest` match; `versions/`, `versions/client`,
  `versions//client`, `a/b/client`, `%2e%2e/client`, `1.14.7/Client`, `1.14.7/client/`, `1.14.7/epic`, `1.14.7`
  are unknown.
- R4 `version_query`: `game=revived` → (Version, revived); `x=1&version=1.12.0` → (Version, 1.12.0);
  `edition=reborn&game=revived` → (Edition, reborn); the first of two `game=` wins; `x=1` and none → none.
- R5 Two `x-reforged-user-api-key` lines join as `RRK_a, RRK_b`; an absent header is none.

Runtime (T):

- T1 Two files in any order come out sorted (`Rustic-Isles.dll`, `winmm.dll`), build `rel-1.14.7-Archon`, 647472.
- T2 Refused: `null`, `[]`, empty or spaced version, `files` not a list, one file, three files, `..\Rustic-Isles.dll`,
  `dxgi.dll`, uppercase sha, size 0, size 1e9, changelist -1, changelist null/1.5/"1"/2^53.
- T3 A BOM, `" v1 "`, `build: []`, `5.0` and `1e1` sizes are read as JavaScript reads them; the body starts
  `{"version":"v1","build":{"name":"rel-1.14.7-Archon","changelist":647472},"files":[{"name":"Rustic-Isles.dll","size":10,`.
- T4 Exact texts: `unknown file null (only Rustic-Isles.dll and winmm.dll)`, `unknown file undefined (only
  Rustic-Isles.dll and winmm.dll)` for a non-object entry, `MysticParadox.dll is listed twice`.
- T5 Store: none without a folder or before publishing; OK once published; `winmm.dll` found, `WINMM.DLL` and
  `runtime.json` not; a file with a wrong hash makes it invalid (reason names `winmm.dll`) after 10 s; a new publish
  is picked up after 10 s; a publish within 9.999 s is not seen until the check is forgotten.
- T6 A runtime published as `MysticParadox.dll` is listed under that name and served for both names, not for
  `mysticparadox.dll`.
- T7 A `runtime.json` of 64 KiB + 1 is "64 KB" invalid; a folder in place of `winmm.dll` is exactly `winmm.dll is not a
  regular file`.

Auth (A): one lookup then cached; cache holds the hash, never the key; expiry at the TTL boundary; unavailable not
cached, refused cached 30 s; implausible keys (`""`, a space, a newline, 257 bytes, `ä`) refused with no lookup;
three concurrent checks share one lookup; a panicking lookup is unavailable and not cached; at most N entries, oldest
dropped; the backend mapping of section 2.8 against a fake backend on `127.0.0.1:0`, and a free port is unavailable.

Slots (S): per-account limit (another account unaffected, release frees one); server-wide limit.

Branding (B): safe and unsafe names of 2.12.1 (including 104-byte names); signatures; accent and credit cleaning
(`#C8A24A` → `#c8a24a`, `" #abc "` → `#abc`, `red`, `#abcd`, `#12345g`, `url(x)`, `5`, null refused; credit
`"  Photo\nby   a friend "` → `Photo by a friend`, `""` and `7` none, 500 chars → 200); no folder means
`{"backgrounds":[],"accent":null}`; without `branding.json` only valid images in name order (fake content, `.txt`,
empty file and a folder named `sub.jpg` skipped); `branding.json` order and credits, `../../secret.jpg`,
missing and repeated entries skipped with at least three warnings; broken `branding.json` still lists folder images; a
missing folder gives no art; a new image appears only after 10 s.

News (N): the parse example of 2.13.2 (newest first, cleaning, skipped entries); a bare array; `{"news":[]}` and
`"x"` refused; the file: none, missing, loaded after a change, a broken edit keeps the last good version, an array
form replaces it; GitHub: newest first, one fetch per minute, a 304 is not an error and sends the stored ETag, 503 /
invalid JSON / `{"items":[{}]}` / over 1 MiB keep the last good news, `{"items":[]}` replaces it with nothing;
concurrent polls share one background fetch.

HTTP (H, against a real listener on `127.0.0.1:0`): the manifest without a key with an ETag; a file only with a known
key; nothing but the two files; news and an empty art pack; 404 `no_runtime` before publishing; stream limits (two
stalled 48 MiB downloads fill an account of 2, the third is 429 `retry-after: 2`, HEAD takes no slot, a third account's
download fills the server of 3 so another is 503 `server_busy`, dropping a stalled client frees its slot); a file
changed under the list is 503 `retry-after: 10`, the manifest then invalid, a new publish served after the check is
forgotten; versioned runtimes (section 2.11.4: `?game=`, `?edition=`, `files/<version>/`, an unknown version or alias
falls back, `files/nonsense/winmm.dll`, `files/revived/runtime.json` and `files/revived/../winmm.dll` 404, a key still
needed); module routes (`/content/v1/versions` empty then listing one module byte for byte as in 2.6.6, client profile
by version and alias with a 304, `unknown_version` cases, Epic manifest 401 without a key, 200 with ETag and type, HEAD
with length and no body, 304, 503 `epic_manifest_invalid` after the file changes, detach removes version and alias).

Log (L):

- L1 A download's line has exactly the keys `t, level, msg, method, route, path, status, ms, ip, account, user,
  bytes, expected, complete` in that order, `msg` `download`, `complete` true with `bytes = expected`, `ip` from
  `x-forwarded-for`; no line of the whole run contains a key.

---

## 4. Target Design for the MIT Rewrite

### 4.1 Where it goes

- **`services/content` in dainty-lass**, crate `content`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`, an
  `SPDX-License-Identifier: MIT` header on each new file. Category B files are written fresh from this spec; the
  Category A files of section 1.2 are copied in with an MIT header added.
- **`services/content-plugin` in dainty-lass**, crate `content-plugin` (`cdylib` `content_plugin`), MIT, moved as is.
  It then ships from the MIT repository; whether the core keeps loading it as a library or links `content` in directly
  (as it does `admin`) is the maintainer's call (section 5, item 9).
- `services/content/tests/parity/` keeps its path and the eight JSON files; `harness.rs` and `parity.rs` are new.
  `record.cjs` and `data/dauntless-1.4.4.json` are not carried.

### 4.2 Public API

Exactly section 2.1. dainty-restricted's `services/host/src/compose.rs`, `services/host/src/lib.rs`, the three fuzz
targets and `content-plugin` must build against the new crate with no source change.

### 4.3 What moves alongside

- `src/modules.rs` (Category A) into the new crate, its two log calls pointed at `service_kit::linelog`.
- `services/content-plugin/{Cargo.toml, src/lib.rs, tests/parity.rs}` (Category A).
- The eight parity JSON files (Category A data).

### 4.4 Dependencies

Normal: `protocol`, `module-api`, `service-kit` (all MIT in dainty-lass); `axum`, `hyper` (server and client, http1),
`hyper-util` (client-legacy, http1, server, tokio), `hyper-rustls`, `rustls` (aws-lc-rs), `http`, `http-body`,
`http-body-util`, `bytes`, `futures-util`, `tokio` (rt-multi-thread, net, fs, time, sync, signal, io-util),
`tokio-util` (io), `tower` (util), `url`, `chrono`, `clap` (derive), `serde`, `serde_json` (`preserve_order`),
`sha1`, `sha2`. Dev: `protocol` with `test-data`, `tempfile`, `tokio` with `macros`. `content-plugin` adds
`service-plugin` and, for its test, `controller`.

**Seam changes to report, not make** (AGENTS.md: the workspace `Cargo.toml` is the integration pass's): dainty-lass's
workspace `members` gains `services/content` and `services/content-plugin`; `[workspace.dependencies]` gains `bytes`,
`http`, `http-body` (today pinned per crate in dainty-restricted), a `content` path entry, and the features above where
the workspace entries lack them (`hyper-util`'s client and server features, `futures-util`). dainty-restricted's
workspace switches `content` from `{ path = "services/content" }` to the dainty-lass git entry its other MIT crates use,
and drops the two members; its fuzz `Cargo.toml` does the same. `NOTICE.md` in both repositories, the AGPL inventory and
the migration guide's §2.4 are updated by the integration pass.

### 4.5 Isolation, provenance and similarity

Follow `docs/port/clean-room/hostkit.md` and `similarity.md`:

- The implementer's worktree is a dainty-lass worktree only; dainty-restricted, `legacy-revived` and `rustic-freeza`
  are not readable from it, and the implementer subagent is told so. Record the isolation check.
- The provenance record goes in `docs/port/clean-room/content.md`: the spec commit, the Category A files moved, each
  implementer session, the files it read and wrote, and the test and clippy results.
- Similarity: the baseline is not in dainty-lass. Someone other than the implementer runs the `similarity.md` method
  against dainty-restricted's `services/content` at `ee7d90b` (and, as a second baseline, DR's `rust/services/content`
  at `24b7624`) and appends the per-file table to `similarity.md`.

### 4.6 Security deliverables (RX3)

Content is a trust-boundary component. With the code, the row delivers a new `docs/security/content.md` in dainty-lass
(structure of `docs/security/relay.md`), written from this spec, and fuzz targets in dainty-lass's `fuzz/` for the
request input, `runtime.json` and the news document, with the invariants of section 3.3 (R, T, N).

---

## 5. Open Points for the Maintainer

1. **The parity fixtures' status.** The `*.expected.json` files are replies recorded from Freeza's AGPL Node server.
   This spec treats them as data (observable behaviour), as was done for protocol's runtime-manifest fixtures. If that
   is not acceptable, they must be re-recorded from the new MIT server and checked by hand against section 2, which
   loses the independent Node reference.
2. **`modules.rs` is Category A** (DF4b, 2.2% similarity, no external source), but it was written inside an AGPL crate
   while looking at its neighbours. Its only dependency on Category B is the `log` re-export.
3. **`log.rs`, `lib.rs` and `Cargo.toml` are Category B only by origin:** in DR, `log.rs` was a doc line and one
   re-export. Rewriting them is trivial, and doing so keeps the rule simple.
4. **`data/dauntless-1.4.4.json` is orphaned** in dainty-restricted since `9829d9a`; it can be deleted there too.
5. **`.env` in the working folder.** `content serve` without `--env-file` reads `./.env` (Node's
   `--env-file-if-exists`). AGENTS.md says a service reads nothing from its working directory. The spec keeps today's
   behaviour; the host always passes `--env-file`, so dropping the fallback would not affect a host.
6. **Built-in 1.4.4 names** (`1.4.4`, `revived`, `legacy`) still resolve in `ContentModules` though 1.4.4 is no longer
   served; only a `<runtime dir>/revived` folder would ever answer for them. Kept, since the file moves as is.
7. **`CONTENT_NEWS_URL` with `:443`** is accepted, because the URL parser drops the default port before the port
   check. Kept for parity.
8. **The parity script depends on timing:** the 10 s runtime re-check must not fire mid-script. On a very slow CI host
   the `content` script could differ on its last two requests.
9. **Library or built in.** Once content is MIT, `dainty-lass-server` could link it like `admin` instead of loading
   `content_plugin`. That changes RD27's shape and the release layout, so it is a separate decision.
