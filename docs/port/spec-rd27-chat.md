# Clean-Room Behaviour Specification: `crates/dainty-chat` (RD27)

> **Ports moved 2026-11-10.** The loopback services' defaults are now backend 41000, Director 41001, content
> 41002, admin 41003, allowlist 41005, chat 41006 and QoS 41010, below the dynamic range Windows lends to Hyper-V,
> WSL and Docker. settings.toml's `[ports]` moves any of them, and on Windows setup and start move one Windows has lent
> out (a private gateway's 61000 to 41080). The numbers below are the ones recorded upstream: read each 610xx
> service port as its 410xx counterpart. A private gateway's default stays 61000.

> **Scope and purpose (RD27, DD26):** this is the clean-room behaviour specification for relicensing Dainty Lass's
> chat core plugin to MIT. Chat lives in dainty-restricted (AGPL) at `crates/dainty-chat` (crate `chat`) and
> `crates/dainty-chat-plugin` (crate `chat-plugin`, the `cdylib` the core loads). Part of it is a port of Dauntless
> Revived's (DR) metagame chat at `e1c5fec`; that part is rewritten here from this document alone by an implementer
> who never sees the AGPL code. The rest was written for Dainty Lass and moves as it is.
>
> Everything below is stated from observable contracts: the XMPP stanzas a client sends and gets, the HTTP routes,
> the database rows, the node-link wire format, the env settings, the public Rust API other crates and the moved
> Category A files call, the error texts and the test suite. No implementation code is reproduced, no private helper
> is named, and the Mystic Paradox realtime gateway, which DR and Dainty Lass read for the 1.14.7 client's subset, is
> cited only for the names it gives (paths, room prefixes), never for code.
>
> Spec written against dainty-restricted `849f45e` (branch `feat/rd27-plugin-abi`) and dainty-lass `92e797a`.

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| dainty-restricted `NOTICE.md` (lines 79-81) | `crates/dainty-chat` is DN10a, a port of DR's metagame chat at `e1c5fec`, with behaviour (no code) from Mystic Paradox's realtime gateway |
| `docs/port/agpl-inventory.md` §1.3.13 | 8 DR-derived source files (`lib.rs`, `connection.rs`, `muc.rs`, `nick.rs`, `redact.rs`, `server.rs`, `settings.rs`, `xmpp.rs`); `federation.rs`, `history.rs`, `hooks.rs`, `naming.rs` and the three test files born here |
| `docs/port/migration-guide.md` §2 row `crates/dainty-chat` | Source: DR `services/metagame/src/chat/` at `e1c5fec` |
| Commit `0a3017b` (DN10a) body | "keeps its structure: one task owns all of chat, DR's limits, throttles, pings, ghost replacement, nickname rules, frame parser and redaction. `xmpp.rs`, `nick.rs` and `redact.rs` are DR's unchanged but for helper paths and `NS_DELAY`" |
| Commits `023b6d6` (DN10b), `8346fd4`, `ac5cf47`, `d4d8d5c` (RD27) | Federation, the dashboard moderation, the env reader and the library written here |
| File headers (`//!`) | Each DR-derived file opens "From Dauntless Revived's metagame chat at `e1c5fec`"; each born file says "New here" or names its row |
| 3+-line similarity (the method of `clean-room/similarity.md` §1) of each file against DR's chat at legacy-revived `cfcad3eb` (`rust/services/metagame/src/chat/*.rs`, `redact.rs`, `moderation.rs`, `config.rs`, `limits.rs`, `server.rs`, `db.rs`, `tests/m31/chat*.rs`, `tests/chatparity.rs`) | Table below |

| File | Code lines | Best DR match | Matched | % |
| --- | ---: | --- | ---: | ---: |
| `src/xmpp.rs` | 482 | `chat/xmpp.rs` | 480 | 99.6 |
| `src/nick.rs` | 125 | `chat/nick.rs` | 125 | 100.0 |
| `src/redact.rs` | 132 | `redact.rs` | 127 | 96.2 |
| `src/server.rs` | 1659 | `chat/mod.rs` | 1098 | 66.2 |
| `src/muc.rs` | 749 | `chat/muc.rs` | 366 | 48.9 |
| `src/settings.rs` | 80 | `chat/mod.rs` | 24 | 30.0 |
| `src/lib.rs` | 219 | `chat/connection.rs` | 51 | 23.3 |
| `src/connection.rs` | 848 | `chat/connection.rs` | 166 | 19.6 |
| `src/hooks.rs` | 98 | `chat/muc.rs` | 9 | 9.2 |
| `src/env.rs` | 66 | `chat/mod.rs` | 6 | 9.1 |
| `src/naming.rs` | 121 | `chat/mod.rs` | 6 | 5.0 |
| `src/federation.rs` | 1378 | `moderation.rs` (metagame's, unrelated) | 63 | 4.6 |
| `src/moderation.rs` | 818 | `config.rs` | 35 | 4.3 |
| `src/history.rs` | 499 | — | 0 | 0.0 |
| `tests/*.rs`, `chat-plugin/**/*.rs` | — | — | ≤ 22 | ≤ 4.9 |

The born files' few matches are imports, `tracing` calls and closing braces. `connection.rs` and `lib.rs` are low
because about half of each was added here (the `/__chat/*` loopback API in `bc2711f`, the start-up wiring), but both
keep DR's structure and DR's functions, so the whole file is rewritten (§1.5).

### 1.2 Category A: born here, moves to MIT as it is (22 files)

| Path (dainty-restricted) | Born in | What it is |
| --- | --- | --- |
| `crates/dainty-chat/Cargo.toml` | `0a3017b` | The manifest. Moves with three edits: `license = "MIT"`, a description without "from Dauntless Revived", and no `gateway` dev-dependency (§6.3) |
| `crates/dainty-chat/migrations/sqlite/0001_chat/{up,down}.sql` | `0a3017b` | Chat's schema (§3.2) |
| `crates/dainty-chat/migrations/postgres/0001_chat/{up,down}.sql` | `0a3017b` | The same in Postgres |
| `crates/dainty-chat/migrations/sqlite/0002_federation/{up,down}.sql` | `023b6d6` | `mid` columns and `chat_outbox` |
| `crates/dainty-chat/migrations/postgres/0002_federation/{up,down}.sql` | `023b6d6` | The same in Postgres |
| `crates/dainty-chat/src/env.rs` | `d4d8d5c` | Chat's settings from its env file; moved from host's `compose.rs`, itself MIT (§3.3) |
| `crates/dainty-chat/src/federation.rs` | `0a3017b` (seam), `023b6d6` (body) | `Federation`, `LocalOnly`, `ChatFederation`, the node-link wire format (§3.1) |
| `crates/dainty-chat/src/history.rs` | `0a3017b` | `History`: chat's database on `store` |
| `crates/dainty-chat/src/hooks.rs` | `0a3017b` | `ChatHost`, `Login`, `Player`, `LoginRefusal`, `Drivers`, the defaults (§3.4) |
| `crates/dainty-chat/src/moderation.rs` | `8346fd4`, `ac5cf47` | The staff dashboard's filter, chat log and reports |
| `crates/dainty-chat/src/naming.rs` | `0a3017b` | Room names to `RoomKind` (DD21) |
| `crates/dainty-chat/tests/chat.rs` | `0a3017b` | One-node black-box tests (§5) |
| `crates/dainty-chat/tests/federation.rs` | `023b6d6` | Two-node tests; one of them needs the AGPL gateway (§6.3) |
| `crates/dainty-chat/tests/history.rs` | `0a3017b` | `History` on SQLite and Postgres |
| `crates/dainty-chat/tests/dashboard_live.rs` | `ac5cf47` | `Moderation` against a live dashboard (ignored) |
| `crates/dainty-chat-plugin/Cargo.toml` | `ee7d90b` | The library's manifest; moves with `license = "MIT"` |
| `crates/dainty-chat-plugin/src/lib.rs` | `d4d8d5c` | `ChatService`: chat as a `service_plugin::Service`, `LibraryChatHost` over the `ChatLink` |
| `crates/dainty-chat-plugin/tests/load.rs` | `d4d8d5c` | Loads the built library through `controller` |

Outside the two folders, three more born-here files go with chat: `fuzz/fuzz_targets/chat_stanza.rs` (`0a3017b`),
`fuzz/fuzz_targets/chat_federation.rs` (`023b6d6`) and `docs/security/chat.md` (`0a3017b`, `8346fd4`).

### 1.3 Category B: derived from DR, rewritten clean-room (8 files)

1. `crates/dainty-chat/src/lib.rs`: the crate root, `ChatPlugin`, `Running`, `StartError` (§2.1, §2.3)
2. `crates/dainty-chat/src/settings.rs`: `Settings`, `NickCheck`, the port and paths (§2.2)
3. `crates/dainty-chat/src/connection.rs`: the listener, the WebSocket connections, the gateway check, the loopback API
   (§2.4, §2.5)
4. `crates/dainty-chat/src/server.rs`: the chat task, sessions, login, bind, IQs, presence, whispers, limits (§2.6-§2.12)
5. `crates/dainty-chat/src/muc.rs`: rooms (§2.13)
6. `crates/dainty-chat/src/nick.rs`: nickname rules (§2.14)
7. `crates/dainty-chat/src/xmpp.rs`: the frame parser, escaping, JIDs, the token bucket, trace redaction (§2.15-§2.17)
8. `crates/dainty-chat/src/redact.rs`: credential redaction for logs (§2.18)

### 1.4 Dependencies between the categories

The crate is one unit, so the arrows run both ways. What each Category A file needs from Category B is the API §2.1
pins; nothing else of B may be assumed.

| Category A file | Uses from Category B |
| --- | --- |
| `federation.rs` | `server::{BODY_LIMIT, Clock, Event, is_authcid_shape, random_hex}` (with `Event::{Muted, RemoteDirect, RemoteRoom}`), `muc::MAX_NICK_BYTES`, `xmpp::{DEFAULT_DOMAIN, has_control_character}` |
| `env.rs` | `settings::{Settings, NickCheck}` (fields and `problems`) |
| `chat-plugin/src/lib.rs` | `ChatPlugin::{start, attach, detach, modules}`, `Running::{port, stop}`, `Options`, `StartError` (Display), the root re-exports |
| `tests/chat.rs`, `tests/federation.rs`, `chat-plugin/tests/load.rs` | `ChatPlugin`, `Running::{port, history, mute, unmute, tick, stop}`, `Settings`, `Options`, `server::random_hex`, the root re-exports |
| `fuzz/chat_stanza.rs` | `xmpp::{Element, Node, parse_frame, has_markup_declaration, escape, log_text, parse_jid, redact_frame}`, `nick::{check_nickname, name_part_of}`, `server::is_authcid_shape` |
| `fuzz/chat_federation.rs` | `muc::MAX_NICK_BYTES`, `server::{BODY_LIMIT, is_authcid_shape}` |

| Category B file | Uses from Category A |
| --- | --- |
| `server.rs`, `muc.rs` | `hooks::{ChatHost, Login, LoginRefusal, Player, default_jid, default_rules, is_unsupported}`; `history::{History, Mute, NewDirect, NewRoom}` and `History::{remember_account, account_of, mute_of, pending_for, mark_delivered, add_direct_once, add_room_once, room_history}`; `naming::{room_kind_of, room_key, room_class}`; `federation::{Federation, Envelope, DirectWire, RoomWire}`; `moderation::{Moderation, parse_report, Channel, channel_of, Line, Person, Verdict, Report, TargetPerson, ReportContext}` |
| `lib.rs` | `history::History` and `History::{set_mute, clear_mute}`, `federation::Federation`, `hooks::ChatHost`, the root re-exports of A items (§2.1) |
| `connection.rs` | `History::{account_of, set_mute, clear_mute, direct_between, pending_for}`, `Federation::{resolve, set_mute}`, `ChatHost::account_of` |
| `settings.rs`, `nick.rs`, `xmpp.rs` | none |
| `redact.rs` | `protocol::account::ACCOUNT_KEY_PREFIX` (MIT, dainty-lass) |

`hooks.rs`, `naming.rs`, `history.rs` and `moderation.rs` use nothing of Category B. The A files must not be edited to
fit the rewrite: if a B item they name cannot be built as §2.1 says, stop and report it.

### 1.5 Mixed files

`connection.rs`, `server.rs`, `muc.rs` and `lib.rs` hold code written here inside DR's structure: the `/__chat/*`
routes and the loopback client (`bc2711f`), the offline-delivery and federation parts of the whisper path, the
`/report` command, `remote_direct` and `remote_room`. Splitting them out would need someone to edit AGPL files before
the implementer starts. They are simpler to rewrite with the rest, and §2 specifies them with the rest.

---

## 2. Behaviour of the Category B code

### 2.0 Conventions

- **Times** are Unix milliseconds (`i64`) from the chat's clock (`Options::clock`) unless a line says "real clock".
- **Lengths:** "bytes" is UTF-8 length; "chars" is Unicode scalar values; "UTF-16 units" is the length JavaScript
  reports. The rules use all three on purpose, matching the Node chat DR ported.
- **Hex:** "N random hex" is N random bytes written as 2N lower-case hex digits.
- **Escaping** in a template is §2.16's escape of the value. Every stanza template below is exact: attribute order,
  quoting (double quotes) and the absence of whitespace between elements are part of the contract, because the tests
  and the game's own parsing look for these substrings.
- `NS_*` names are the constants of §2.1 (`xmpp`). In templates, `{framing}` is `urn:ietf:params:xml:ns:xmpp-framing`,
  `{client}` is `jabber:client`, `{stanzas}` is `urn:ietf:params:xml:ns:xmpp-stanzas`.
- **"Full JID"** of a session is `uid@domain/resource`, where a missing `uid` or `resource` is the text `undefined`.
- **"MUC domain"** of a session: `muc.<session domain>` or `conference.<session domain>`, compared case-insensitively
  (Unicode lower-casing of both sides).

### 2.1 Public API (what must exist, by name)

Module tree of the crate `chat` (all `pub mod`): `connection`, `env`, `federation`, `history`, `hooks`, `moderation`,
`muc`, `naming`, `nick`, `redact`, `server`, `settings`, `xmpp`.

Crate-root re-exports: `federation::{ChatFederation, ChatFederationConfig, DirectWire, Envelope, Federation,
LocalOnly, MuteWire, RoomWire}`, `history::{DirectMessage, History, Mute, RoomMessage}`, `hooks::{ChatHost, Drivers,
Login, LoginRefusal, Player}`, `moderation::{DashboardSettings, Moderation}`, `server::Options`,
`settings::{NickCheck, Settings}`.

**Crate root (`lib.rs`):**

| Item | Contract |
| --- | --- |
| `pub enum StartError` (`Debug`, `thiserror::Error`) | `Settings(String)` displays `chat settings: {0}`; `Listen { address: SocketAddr, source: std::io::Error }` displays `chat could not listen on {address} ({source})` |
| `pub struct ChatPlugin` (`Default`) | `new() -> Self`; `modules(&self) -> BTreeSet<ModuleId>`; `async start(&self, settings: &Settings, node: NodeIdentity, host: Arc<dyn ChatHost>, federation: Arc<dyn Federation>, history: History, options: Options) -> Result<Running, StartError>`; implements `module_api::CorePlugin` (§2.3) |
| `pub struct Running` | `pub port: u16`; `pub largest_backlog: Arc<AtomicUsize>`; `history(&self) -> &History`; `async mute(&self, account: &GlobalId, until: Option<i64>, reason: Option<String>, actor: store::Actor) -> Result<(), store::Error>`; `async unmute(&self, account: &GlobalId, actor: store::Actor) -> Result<bool, store::Error>`; `async tick(&self)`; `async stop(self)` |

**`settings`:** `pub const DEFAULT_PORT: u16 = 61006`; `pub const WS_PATHS: [&str; 3]` = `["//", "/", "/__ws/xmpp"]`;
`pub enum NickCheck { Enforce, Log }` (`Debug, Clone, Copy, PartialEq, Eq`) with `as_str` giving `enforce` / `log`;
`pub struct Settings` (`Debug, Clone, PartialEq, Eq`, all fields `pub`): `host: IpAddr`, `port: u16`,
`gateway_secret: Option<String>`, `nick_check: NickCheck`, `trace: bool`, `history_on_join: i64`,
`offline_batch: i64`; `Default`; `problems(&self) -> Vec<String>` (§2.2).

**`server`:**

| Item | Contract |
| --- | --- |
| `pub const FRAME_LIMIT: usize = 32768` | Largest WebSocket frame and message, bytes |
| `pub const BODY_LIMIT: usize = 2048` | Largest message body, chars |
| `pub const OUTPUT_LIMIT: usize = 256 * 1024` | Default unsent-output limit, bytes |
| `pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>` | The clock chat reads |
| `pub type Observer = Arc<dyn Fn(Option<&str>, &str) + Send + Sync>` | Sees every stanza handed to a connection, with the receiving session's JID local part |
| `pub struct Options` (`Clone`) | `pub clock: Clock`, `pub output_limit: usize`, `pub auto_tick: bool`, `pub observe: Option<Observer>`, `pub moderation: Moderation`; `Default` = real clock (`now_ms`), `OUTPUT_LIMIT`, `true`, `None`, `Moderation::off()` |
| `pub enum Event` (`Debug`) | Whatever the chat task is asked. It must have at least `Muted { account: GlobalId, mute: Option<Mute> }`, `RemoteDirect { message: DirectWire, reply: tokio::sync::oneshot::Sender<bool> }` and `RemoteRoom { message: RoomWire }`, which `federation.rs` sends (§2.11, §2.13.9); the other variants are the implementer's |
| `pub fn now_ms() -> i64` | The real clock: milliseconds since the epoch, 0 before it, saturating at `i64::MAX` |
| `pub fn random_hex(n: usize) -> String` | `n` random bytes as `2n` lower-case hex digits |
| `pub fn stamp(at: i64) -> String` | XEP-0203 stamp: UTC RFC 3339 with milliseconds and `Z` (`1970-01-01T00:00:00.000Z` for 0; `2026-10-26T07:33:20.123Z` for `1793000000123`); an out-of-range value gives the epoch's stamp |
| `pub fn is_authcid_shape(text: &str) -> bool` | 1 to 128 bytes, and no `@`, `/`, white-space char (Unicode `White_Space`) or control char (§2.8) |

**`muc`:** `pub const MAX_NICK_BYTES: usize = 1023`, and `MAX_ROOMS_PER_SESSION = 8`, `MAX_OCCUPANTS = 128`,
`MAX_ROOMS = 1000`, `JOIN_BURST: u32 = 10`, `MESSAGE_BURST: u32 = 8` (public for reference; §2.13 uses them);
`pub fn is_muc_domain(account_domain: &str, domain: &str) -> bool` (§2.0's MUC domain).

**`nick`:** `pub const FALLBACK_NAME: &str = "InvalidMCPUser"`; `pub enum NickReason { Account, Resource, Format,
Name }` with `as_str` = `nick-account`, `nick-resource`, `nick-format`, `nick-name`; `pub fn
url_encode_like_client(&str) -> String`; `pub fn decode_name_part(&str) -> Option<String>`; `pub fn
name_part_of(nick: &str) -> Option<String>`; `pub fn check_nickname(nick: &str, uid: &str, resource: &str, names:
&[Option<&str>]) -> Result<String, NickReason>` (§2.14).

**`xmpp`:** the constants `NS_FRAMING`, `NS_SASL` (`urn:ietf:params:xml:ns:xmpp-sasl`), `NS_BIND`
(`urn:ietf:params:xml:ns:xmpp-bind`), `NS_SESSION` (`urn:ietf:params:xml:ns:xmpp-session`), `NS_STREAMS`
(`http://etherx.jabber.org/streams`), `NS_STREAM_ERRORS` (`urn:ietf:params:xml:ns:xmpp-streams`), `NS_STANZAS`,
`NS_CLIENT`, `NS_PING` (`urn:xmpp:ping`), `NS_MUC` (`http://jabber.org/protocol/muc`), `NS_MUC_USER`
(`http://jabber.org/protocol/muc#user`), `NS_DELAY` (`urn:xmpp:delay`), and `DEFAULT_DOMAIN` =
`prod.ol.epicgames.com` (the client's built-in XMPP domain). Types: `pub enum Node { Element(Element), Text(String) }`
and `pub struct Element { pub name: String, pub attrs: Vec<(String, String)>, pub children: Vec<Node> }` (both `Debug,
Clone, PartialEq, Eq`; `Element` also `Default`), with `Element::attr(&self, &str) -> Option<&str>`, `text(&self) ->
String`, `child(&self, &str) -> Option<&Element>`, `elements(&self) -> impl Iterator<Item = &Element>`;
`pub struct Jid { pub local: String, pub domain: String, pub resource: Option<String> }` (`Debug, Clone, PartialEq,
Eq`); `pub struct Bucket` (`Debug, Clone, Copy, PartialEq`) with `new(burst: u32, now: i64)` and `take(&mut self,
burst: u32, refill_ms: f64, now: i64) -> bool`. Functions: `text_of(Option<&Element>) -> String`, `parse_frame(&str) ->
Option<Element>`, `has_markup_declaration(&str) -> bool`, `escape(&str) -> String`, `local_name(&str) -> String`,
`log_text(&str, max: usize) -> String`, `is_host_name(&str) -> bool`, `is_xmpp_domain(&str) -> bool`,
`parse_jid(&str) -> Option<Jid>`, `has_control_character(&str) -> bool`, `redact_frame(&str) -> String`.

**`redact`:** `pub fn redact(text: &str) -> String` (§2.18).

**`connection`:** `pub const GATEWAY_HEADER: &str = "x-reforged-gateway"`; `pub fn client_address(peer: SocketAddr,
headers: &axum::http::HeaderMap, secret: Option<&str>) -> (IpAddr, &'static str)` (§2.4). Everything else in
`connection`, and the rest of `server` and `muc` (sessions, the task, the room table), is the implementer's, `pub` or
not.

### 2.2 Settings

Defaults: `host` 127.0.0.1, `port` 61006, `gateway_secret` none, `nick_check` `Enforce`, `trace` false,
`history_on_join` 20, `offline_batch` 100. `port` 0 binds any free port (tests).

`problems()` returns, in this order, each that applies (texts are a contract; `env.rs` and `StartError` join them
with `"; "`):

1. host not loopback: `chat must listen on 127.0.0.1 or ::1 (the gateway forwards chat there), not {host}`
2. `gateway_secret` is `Some("")`: `the gateway secret is empty`
3. `history_on_join` outside 0..=1000 or `offline_batch` outside 1..=1000: `history_on_join is 0 to 1000 and
   offline_batch 1 to 1000`

### 2.3 Plugin, start and stop

- `ChatPlugin` as a `CorePlugin`: `name()` is `chat`; `serves()` is `[CoreCapability::Chat]`; `attach(module, _)`
  adds `module.id` to its set when `module.capabilities.chat`, and always returns `Ok`; `detach(id)` removes it.
  `modules()` is a copy of the set. The set is shared with every chat started from this plugin and read on each
  query (a module attached after start counts at once). A module "answers chat" when it is in this set.
- `start`:
  1. non-empty `problems()` refuses with `StartError::Settings`.
  2. Binds TCP `settings.host:settings.port`; a failure is `StartError::Listen` with that address. `Running::port`
     is the port actually bound.
  3. Logs that chat listens on `host:port` and the nick check (`enforce` or `log`).
  4. Calls `federation.attach(events, history.clone(), host.clone(), options.clock.clone())`, where `events` is the
     sending side of the chat task's queue.
  5. Spawns the chat task (§2.6) and the HTTP server (§2.4, §2.5), the latter with graceful shutdown.
- `Running::mute(account, until, reason, actor)`: builds `Mute { account: account.to_string(), until, reason, by:
  actor.as_text(), at: now_ms() }` (real clock), stores it with `History::set_mute(mute, actor)` (an error is
  returned and nothing else happens), then tells the task (`Event::Muted` with the mute; every live session of that
  account takes it at once), then `federation.set_mute(account, Some(mute), actor)`.
- `Running::unmute(account, actor)`: `History::clear_mute(account string, actor)` gives whether there was one (an
  error is returned), then `Event::Muted` with none, then `federation.set_mute(account, None, actor)`; returns the
  flag.
- `Running::tick()`: one tick (§2.12) now, returning when it is done (also when `auto_tick` is off).
- `Running::stop()`: the shutdown of §2.12, then waits for the task, then stops the HTTP listener and waits for it.

### 2.4 The listener and WebSocket connections

HTTP/1.1 on the bound loopback address. Every connection gets the peer address; `TCP_NODELAY` is set (use
`service_kit::serve::{serve, NoDelay, ConnectionInfo}` or equivalent).

**Who is calling (`client_address`).** The request is from the gateway when all hold: a secret is configured, the
peer's IP (IPv4-mapped IPv6 taken as IPv4) is loopback, the request carries `x-reforged-gateway`, and that header's
value equals the secret, compared in constant time (compare SHA-256 digests of both in constant time, so the lengths
leak nothing). Then the player's address is the right-most comma-separated entry over all `x-forwarded-for` headers,
trimmed and parsed as an IP (mapped addresses canonicalised); if that does not parse, the peer's. The second value is
`"gateway"`. Otherwise the address is the peer's IP and the second value `"direct"`. Tests:
`only_the_gateway_names_the_player` (connection.rs, rewritten: a loopback peer with the right secret and
`1.2.3.4, 203.0.113.7` gives `203.0.113.7`/gateway; a wrong secret, a LAN peer, no secret configured, or no header
gives the peer/direct).

**Routing.** The `/__chat/*` routes of §2.5 first; anything else goes to the upgrade handler:

1. Not a valid WebSocket upgrade (any method, any path): `404` with an empty body.
2. A path (exact match, no query considered) outside `WS_PATHS`: `404`.
3. Asks the chat task to admit the address (§2.7). Refused, or the task gone: `503` with headers `connection: close`
   and `content-length: 0`.
4. Accepts the upgrade. The chosen subprotocol is the first non-empty, trimmed, comma-separated entry of
   `sec-websocket-protocol`, whatever it is (the game offers `xmpp`); none offered, none chosen. Maximum message and
   frame size `FRAME_LIMIT`. A failed upgrade is logged as a warning.

**A connection.** Once upgraded, the connection registers with the chat task (address and how it came) and gets a
session id. It then runs a reader and a writer:

- **Reader:** a text frame is handed to the task as one frame, in order. A binary frame is decoded as UTF-8 lossily
  (invalid sequences become U+FFFD) and handed over the same way. Ping and pong frames are not frames (the WebSocket
  layer answers pings). A close frame or the end of the stream tells the task the connection is gone. A read error
  tells it gone, flagged "too large" when the error is the message or frame size limit.
- **Writer:** sends what the task hands it, in order: a text frame; or a close frame with a code and an empty reason,
  after which nothing more is written and the connection is cut if the client has not finished closing within 30 s;
  or "cut now". The writer keeps a shared count of bytes handed to it and not yet written (Node's `bufferedAmount`),
  increased when the task hands a text over and decreased when it has been written. When the writer stops it flushes
  for at most 200 ms (so a close the client sent is answered).
- When the reader stops (or a cut is signalled), the connection waits up to 30 s for the writer, then drops the
  socket and marks itself closed (the shutdown waits on that mark, §2.12).

### 2.5 The loopback control API (`/__chat/*`)

Added here (`bc2711f`); no caller in either repository yet, kept for parity. Every route answers `403` with an empty
body unless the peer is loopback **and** the request has no `x-reforged-gateway` header (so nothing the gateway
forwards reaches it). JSON bodies are read as JSON (a malformed body is the framework's 4xx). Answers are JSON with
the fields in the order given.

**Resolving an account** from text `t`: `t` parsed as a `GlobalId`; else `History::account_of(t)` parsed; else
`ChatHost::account_of(t)`; else `Federation::resolve(t)`; else none.

| Route | Request | Answers |
| --- | --- | --- |
| `POST /__chat/mute` | `{"account"?, "local"?, "until"?: i64, "reason"?, "actor"?}` | key = `account`, else `local`, else `""`; unresolved: `404 {"error":"account not found"}`. Actor `Admin(actor or "host")`. Stores a `Mute` (now = real clock) as `Running::mute` does; a store error is `500 {"error":"<error display>"}`; then `Event::Muted`, then `Federation::set_mute`. `200 {"ok":true,"account":"<id>"}` |
| `POST /__chat/unmute` | same body | As `Running::unmute`. `200 {"ok":true,"account":"<id>","had":<bool>}`; `404`/`500` as above |
| `GET /__chat/history/direct?a=&b=&limit=` | | missing `a`: `400 {"error":"missing a"}`; missing `b`: `400 {"error":"missing b"}`. `limit` an `i64`, default 50 when absent or unparsable, clamped to 1..=200. Unresolved: `404 {"error":"account a not found"}` / `{"error":"account b not found"}`. `200 {"a":"<id>","b":"<id>","messages":[DirectMessage...]}` from `History::direct_between`; `500 {"error":...}` |
| `GET /__chat/history/waiting?account=` (or `local=`) | | neither: `400 {"error":"missing account"}`; unresolved `404 {"error":"account not found"}`; `200 {"account":"<id>","messages":[...]}` from `History::pending_for(id, 50)`; `500` |
| `POST /__chat/tick` | | runs one tick and waits for it; `200 {"ok":true}` |
| `POST /__chat/client/login` | `{"handle","authcid","token","resource"?}` | Opens a test client (below) and keeps it under `handle` (replacing one of that name). `200 {"ok":true,"handle":..,"frames":[..]}`; on failure `400 {"ok":false,"error":"<text>"}` |
| `POST /__chat/client/send` | `{"handle","xml"?,"whisper_to"?,"join_room"?,"name"?,"say_room"?,"body"?,"wait_ms"?}` | unknown handle `404 {"error":"unknown client handle"}`. Stanza: `xml` as given; else `whisper_to` gives `<message type="chat" to="{whisper_to}@prod.ol.epicgames.com" id="w1"><body>{body}</body></message>`; else `join_room` gives `<presence to="{room}@muc.prod.ol.epicgames.com/{nick}"><x xmlns="http://jabber.org/protocol/muc"/></presence>` with nick `{name or authcid}:{authcid}:{resource}`; else `say_room` gives `<message type="groupchat" to="{room}@muc.prod.ol.epicgames.com" id="m1"><body>{body}</body></message>`; else `400 {"error":"missing stanza action"}`. Only `body` and the nick are escaped (room and target are inserted raw). A write error is `500 {"error":..}`. Then waits `wait_ms` (default 200, clamped 10..=5000) and answers `200 {"ok":true,"frames":[..]}` with every frame received so far, taken |
| `GET /__chat/client/drain?handle=&expect=&timeout_ms=&wait_ms=` | | no `handle`: `400 {"error":"missing handle"}`; unknown `404`. With non-empty `expect`: waits until some received frame contains it (timeout default 5000, clamped 50..=15000): `200 {"ok":true,"matched":true,"frames":[..]}`, or on timeout `200 {"ok":false,"matched":false,"error":"<text>","frames":[..]}`. Without: waits `wait_ms` (default 200, 10..=5000), `200 {"ok":true,"frames":[..]}` |
| `POST /__chat/client/close` | `{"handle"}` | removes it; if there was one, sends `<close xmlns="{framing}"/>` and waits 50 ms. Always `200 {"ok":true}` |

**The test client** connects to `127.0.0.1:<this port>`, asks `GET //` with subprotocol `xmpp` and a fixed key; a
status line other than `HTTP/1.1 101` fails with `upgrade refused: <status line>`. It sends masked text frames and
reads text and binary frames as lossy UTF-8 (a close frame ends reading). It opens a stream to `DEFAULT_DOMAIN`, waits
up to 5 s for `<stream:features`, sends SASL PLAIN (`\0authcid\0token`, standard base64 with padding), waits up to 10 s
for `<success` or `<failure` (failure: `SASL authentication failed: <frames>`), restarts the stream, binds `resource`
(default `V0:Win:dauntless:0.1`, escaped) with iq id `b1`, waits up to 10 s for `<jid>` or `type="error"` (error: `bind
failed: <frames>`), sends `<presence/>`, waits 250 ms; the frames it answers are the bind answer and everything after.
A wait that times out fails with `timed out waiting for <needles>; got <frames>`. These error texts are informative,
not a contract.

### 2.6 The chat task

One task owns all of chat state and handles one event at a time, in arrival order, awaiting each to the end (the
Node chat's single thread). Events: admit an address; a connection registered; a frame; a connection gone (with the
"too large" flag); a mute set or lifted; a remote direct message (answers whether it was delivered); a remote room
message; shutdown; tick-now. With `auto_tick` the task also ticks every second (missed ticks delayed, not bunched);
events and ticks never run concurrently, and pending events win over the timer. The task ends after a shutdown or
when every sender is gone.

Session ids start at 1 and only grow; "in order" for sessions means by id. A session's state: id, its writer, whether
it is still open for sending, the address and how it came, when it opened, its stream domain (initially
`DEFAULT_DOMAIN`), and once logged in the uid (JID local part), account, module, display name at login and login
time; once bound the resource; flags for "ended", "a login was refused here" and "backlogged"; frames seen before
login; last inbound time; the pending ping id and deadline; three token buckets (stanzas, room joins, messages); the
times of recent dropped stanzas and of the last drop log; the IQ namespaces already logged; "available"; the rooms it
holds as `(room JID as the client spelled it, nickname)` in join order; and its mute. A per-uid list of bound sessions
is kept oldest first.

**Sending to a session.** Nothing goes to a session that is gone or no longer open. If the session has not ended and
is backlogged, or its writer's unsent count exceeds `output_limit`, it is marked backlogged and the stanza dropped
(it is ended at its next frame or tick, never mid fan-out). Otherwise: with `trace`, an info log of the redacted
stanza (§2.17); the observer, if any, is called with the session's uid and the stanza; the unsent count grows by the
stanza's byte length and `largest_backlog` takes the maximum of itself and the new count; the stanza goes to the
writer (a closed writer marks the session not open).

**Seen message ids.** The task remembers the last 4096 message ids (`mid`) it handled, oldest evicted first. "Remember
a mid" answers whether it was new; the empty mid is always new and never stored.

**Log-once.** A keyed throttle answers "log now?" for a key and a window (default 10 minutes): yes when the key was not
seen within the window, recording now. At most 10000 keys (the oldest recorded is evicted first); each tick forgets
keys recorded 10 minutes ago or more. The `/report` rate limit (§2.10.3) shares it.

### 2.7 Connection admission and limits

| Limit | Value | Effect |
| --- | --- | --- |
| Sessions | 512 | admission refused (`503`) |
| Unbound sessions per address (not logged in, or logged in and not bound) | 8 | admission refused |
| Frames before login | 4 | the 5th: a SASL failure counted for the address, session ended `refused` |
| Login deadline | 15 s from open | ended `timeout` |
| Bind deadline | 10 s from login | the uid's bind timeout counted, ended `timeout` |
| Bind timeouts per uid | 3 in 10 min | the uid's logins wait 60 s |
| SASL failures per address | 10 in 10 min | the address's logins wait 10 min |
| Stanzas after login | burst 60, one more every 1000/30 ms | dropped (`rate`) |
| Dropped stanzas | more than 100 in 60 s | ended `abuse` |
| Idle before a ping | 50 s without an inbound frame | ping, 100 s to answer |
| Ghost ping | 10 s | §2.9 |
| Connections per uid (bound or not) | 3 | at a successful login, the oldest unbound ones of that uid go (`replaced`) |
| Bound sessions per uid | 2 | at a bind (§2.9) |
| IQ id, stanza id kept | 128 UTF-16 units | longer: IQ dropped; message id ignored |
| Resource | 256 UTF-16 units, no control char | `bad-request` |
| Room sweep | every 60 s | §2.13.7 |

Admission refusal is logged once per address per 10 minutes, naming the cause ("too many connections" or "too many
logins in progress from this address").

**Ending a session** (once; a second end is nothing), with a reason among `close`, `socket`, `ping-timeout`,
`replaced`, `abuse`, `size`, `backlog`, `shutdown`, `timeout`, `refused`:

1. Marks it ended; it leaves every room (§2.13.6) with leave reason `replaced` when the end reason is `replaced`, else
   `disconnect`.
2. If it was bound: it leaves the uid's bound list; if none is left, the uid entry goes and
   `Federation::presence(account, uid, false)` then `ChatHost::presence(account, uid, false)` are awaited. If the
   reason is `replaced`, `abuse`, `size` or `backlog`, the uid's logins wait 60 s (an unbound session never sets this).
3. Logs the close with the uid (or `-`), the reason and the session's age in whole seconds (rounded).
4. If still open: `close`, `shutdown`, `replaced` send `<close xmlns="{framing}"/>` (sent even when backlogged) then a
   WebSocket close 1000; `size` sends a close 1009 alone; every other reason cuts the connection without a close
   frame.
5. Removes the session.

A connection gone without "too large" is first marked not open, then ended `socket` (nothing sent). With "too large":
a warning, a SASL failure counted for its address, then ended `size`.

**Dropping a stanza** (with a reason: `rate`, `parse`, `unknown`, `refused`): records the time in the session's
60-second window; logs `dropped` with the reason at most once a minute per session; past 100 in the window ends the
session `abuse`. Room join refusals count (§2.13.2); room message errors and dropped presences do not.

### 2.8 Frames, the stream and login

**A frame** for session `id` (ignored when the session is gone or ended):

1. `last_inbound` = now; the ping deadline and id are cleared (any frame answers a ping).
2. With `trace`, info-log the redacted frame.
3. Backlogged (flag, or unsent count over `output_limit`): end `backlog`, stop.
4. Not logged in: count the frame; past 4, count a SASL failure for the address and end `refused`, stop. Logged in:
   take a stanza token; none, drop `rate` and stop.
5. A frame containing `<!` or `<?` anywhere: logged in, drop `parse`; not logged in, send
   `<stream:error xmlns:stream="http://etherx.jabber.org/streams"><restricted-xml xmlns="urn:ietf:params:xml:ns:xmpp-streams"/></stream:error>`
   and end `refused`. Stop.
6. Parse (§2.15). Failure: logged in, drop `parse`; not logged in, end `refused` (nothing sent). Stop.
7. By the root's local name: `open` is the stream open below; `close` ends the session `close`.
8. Not logged in: §2.8.2.
9. `iq`: §2.8.4 (also before the bind).
10. Not bound: drop `unknown`.
11. `presence` §2.10.1, `message` §2.10.2, anything else drop `unknown`.

#### 2.8.1 Stream open

Before the bind, the `to` attribute, when it passes `is_xmpp_domain` (§2.16), becomes the session's domain as written
(case kept). An absent or bad `to` resets the domain to `DEFAULT_DOMAIN` only when not logged in yet. After the bind the
domain never changes. The answer is two stanzas:

- `<open xmlns="{framing}" from="{domain}" id="{8 random hex}" version="1.0" xml:lang="en"/>`
- logged in: `<stream:features xmlns:stream="http://etherx.jabber.org/streams"><bind xmlns="urn:ietf:params:xml:ns:xmpp-bind"/></stream:features>`;
  otherwise `<stream:features xmlns:stream="http://etherx.jabber.org/streams"><mechanisms xmlns="urn:ietf:params:xml:ns:xmpp-sasl"><mechanism>PLAIN</mechanism></mechanisms></stream:features>`

#### 2.8.2 Before login

- `auth`: §2.8.3.
- `iq` (the client's legacy `jabber:iq:auth` after a refused SASL): answer
  `<iq xmlns="jabber:client" type="error" id="{id}"><error type="auth"><not-authorized xmlns="{stanzas}"/></error></iq>`,
  where ` id="{id}"` is present only when the `id` attribute is non-empty and at most 128 UTF-16 units; then end
  `refused`.
- Anything else: end `refused`, nothing sent.

#### 2.8.3 SASL PLAIN

A refusal sends `<failure xmlns="urn:ietf:params:xml:ns:xmpp-sasl"><{condition}/></failure>`, marks that this
connection had a refused login, counts a SASL failure for the address unless the reason is `throttled` or
`unavailable`, and logs once per (uid, or the address when no uid is known, and reason) per 10 minutes. The
connection stays open. In order:

1. This connection already had a refused login: end `refused`, nothing sent (one attempt per connection).
2. The address is throttled: refuse `temporary-auth-failure`, reason `throttled`.
3. `mechanism` attribute not exactly `PLAIN`: refuse `invalid-mechanism`, reason `bad-mechanism`.
4. The payload: the element's own text, trimmed as JavaScript's `trim()` (use `service_kit::js::trim`). It must be
   non-empty, at most 12000 bytes, end in at most two `=`, and before them be non-empty and only `A-Z a-z 0-9 + /`.
   It is decoded as Node's `Buffer.from(text, "base64")`: each alphabet char gives 6 bits (`-` counts as `+`, `_` as
   `/`), decoding stops at the first other char, whole bytes are kept and a trailing partial byte is dropped
   (`AGEAYg==` and `AGEAYg` give `\0a\0b`; `QQ` gives `A`; `Q` gives nothing). The bytes are decoded as UTF-8
   lossily and split on NUL. There must be exactly three fields; the first (authzid) empty or equal to the second;
   the second (authcid) `is_authcid_shape`; the third (token) non-empty and at most 8192 UTF-16 units. Otherwise
   refuse `not-authorized`, reason `bad-format`.
5. The uid (authcid) is throttled: refuse `temporary-auth-failure`, reason `throttled`.
6. `ChatHost::login(Login { authcid, token, domain: session domain })`. A refusal `r`: refuse
   `temporary-auth-failure` when `r` is `Unavailable`, else `not-authorized`, with reason `r.as_str()`.
7. The JID: for a module that does not answer chat, `hooks::default_jid(authcid, domain)`. Otherwise
   `ChatHost::ask(module, ChatQuery::Jid { account })`: `ChatAnswer::Jid(j)` is `j`; another answer is none; an
   `Unsupported` error (`hooks::is_unsupported`) is the default JID; any other error is none with a warning. None:
   refuse `temporary-auth-failure`, reason `unavailable`.
8. The JID's local part differs from the authcid: refuse `not-authorized`, reason `uid-mismatch`.
9. The mute: `History::mute_of(account string)`; an error refuses `temporary-auth-failure`, reason `unavailable`
   (logged as an error). None here: `Federation::mute_of(account)`.
10. `History::remember_account(uid, account, module, jid domain, now)`; a failure is only a warning.
11. Other non-ended sessions with this uid, bound or not, number `n`: if `n + 1 > 3`, the first `n + 1 - 3` of the
    unbound ones (in session order) are ended `replaced`.
12. The session takes uid, account, module, display name, login time and mute; logs `login ok`; sends
    `<success xmlns="urn:ietf:params:xml:ns:xmpp-sasl"/>`.

The token is never logged at any level.

#### 2.8.4 IQs (logged in)

- `type` `result` or `error`: ignored (no drop).
- `type` not `get`/`set`, or `id` (absent counts as empty) over 128 UTF-16 units: drop `unknown`.
- `set` with a child whose local name is `bind`: §2.9, with the requested resource the text of its `resource` child
  (empty when none).
- Anything else (pings, the session IQ, the unknown): the first child element's `xmlns` (empty when absent), unless it
  is `NS_PING` or `NS_SESSION`, is debug-logged once per session (at most 32 namespaces, first 80 chars). Answer
  `<iq xmlns="jabber:client" type="result" id="{id}"/>` (the attribute is always there, empty when the id is).

### 2.9 Bind, ghosts and pings

**Bind** (iq id `i`, requested resource `r`):

1. Already bound: answer the bound result with the current full JID; nothing else.
2. `r` over 256 UTF-16 units or holding a control char: answer
   `<iq xmlns="jabber:client" type="error" id="{i}"><error type="modify"><bad-request xmlns="{stanzas}"/></error></iq>`;
   still unbound.
3. Empty `r` becomes `srv-{16 random hex}` (32 hex digits).
4. A bound session of the same uid with the same resource is ended `replaced`.
5. If the uid still has two or more bound sessions, the one heard from least recently (smallest `last_inbound`; the
   oldest-bound first among equals) is ended `replaced`.
6. The session takes the resource and joins the end of the uid's bound list; info log (uid, resource, domain, count).
7. Answers `<iq xmlns="jabber:client" type="result" id="{i}"><bind xmlns="urn:ietf:params:xml:ns:xmpp-bind"><jid>{full JID}</jid></bind></iq>`.
8. Every other bound session of the uid whose ping deadline is absent or later than now + 10 s is pinged with a 10 s
   deadline (a ghost that does not answer ends `ping-timeout`).
9. `Federation::presence(account, uid, true)` then `ChatHost::presence(account, uid, true)` (at every bind, not only
   the first).
10. Waiting direct messages are delivered (§2.11).

**A ping** takes the next id `sp<n>` (one counter for the whole task, from 1), sets the session's ping id and deadline
(now + timeout), and sends `<iq xmlns="jabber:client" type="get" id="sp{n}" from="{domain}" to="{full JID}"><ping xmlns="urn:xmpp:ping"/></iq>`.

### 2.10 Presence and messages

#### 2.10.1 Presence

- No `to`: `type="unavailable"` leaves every room (leave reason `left`, §2.13.6) and marks the session not available;
  no `type` marks it available; any other type does nothing. Nothing is echoed to anyone.
- A `to` over 2048 UTF-16 units, or one `parse_jid` refuses, is dropped (debug log only).
- It is for a room when the `to` domain is the session's MUC domain, or starts with `muc.` or `conference.` (ASCII
  case-insensitive), or the first child with local name `x` has `xmlns` exactly `NS_MUC`. Otherwise (directed
  presence, subscriptions, probes) it is dropped (debug log only).
- For a room: no `type` joins (§2.13.2); `unavailable` leaves (§2.13.5); other types do nothing.

#### 2.10.2 Messages

`to` (empty when absent) is parsed when at most 2048 UTF-16 units. Then:

- `type="groupchat"` with a parsed `to`: §2.13.8.
- `type` `chat`, `normal` or absent, with a parsed `to` whose domain is **not** the session's MUC domain: a whisper
  (§2.11) to `to.local`, naming `to.resource` if any.
- Anything else: dropped with a debug log naming the type (first 16 chars, `undefined` when absent). Not counted.

#### 2.10.3 `/report`

A whisper or room body that `moderation::parse_report` recognises is a report command and is never delivered,
stored or federated. The answer goes to the sender alone as
`<message xmlns="jabber:client" type="{kind}" from="{from}" to="{full JID}"><body>{answer}</body></message>`, where
`kind` is `chat` for a whisper (from = the session's domain) and `groupchat` in a room (from = the room JID as the
client spelled it). `answer` (server text, English, a contract):

| Case | Answer |
| --- | --- |
| `parse_report` gave `Err` | `Usage: /report <player> [category] <what happened>` |
| `Moderation::is_on()` false | `Reports are not set up on this server.` |
| log-once key `report|{uid}` with a 30 s window says no | `Please wait a little before sending another report.` |
| `Moderation::report(...)` true | `Report sent. Thank you.` |
| `Moderation::report(...)` false | `The report could not be sent. Please try again later.` |

The report is `Report { id: "report-" + 16 random hex, source: "game", reporter: Person { user_id: uid, name:
display name at login or uid }, target: TargetPerson { user_id: the uid of any session whose uid equals the target or
whose display name equals it ASCII-case-insensitively, else none; name: the target as typed }, category,
description, context: ReportContext { room_id: the room key in a room, none for a whisper, at: now } }`. Logged with
the category and whether it was queued.

### 2.11 Whispers and offline delivery

**A whisper** from session `s` to local part `target` (resource `res` optional):

1. Body = text of the first `body` child (empty when none); `length` in chars; the message id is the `id` attribute
   when at most 128 UTF-16 units. The target shown in logs is `target` when `is_authcid_shape`, else `?`.
2. Takes a message token (burst 8, one per second). None: logs reason `limit` at most once a minute per session, and
   stops.
3. A refused whisper sends nothing back, ever; it is logged with sender uid, shown target, length and the reason. In
   order: length 0 or over `BODY_LIMIT` (`size`); a report command (§2.10.3, before the remaining checks, so a muted
   player can still report); `target` equal to the sender's uid (`self`); sender muted now (`muted`); `target` not
   `is_authcid_shape` (`unknown`).
4. The recipient account: the account of the first bound session of `target`; else `History::account_of(target)`
   parsed; else `ChatHost::account_of(target)`; else `Federation::resolve(target)`; none is `unknown`. A history read
   error is logged and treated as none.
5. `ChatHost::blocked(sender, recipient)` true, or the sender's module answers chat and
   `ChatQuery::MayWhisper { from, to }` answers exactly `MayWhisper(false)` (any error or other answer allows):
   `blocked`.
6. `mid` = `dm-` + 16 random hex. `Moderation::rate(body, Channel::Direct)` and `Moderation::record(Line { id: mid,
   at: now, channel: "direct", room_id: None, sender: Person { uid, display name or uid }, recipient: Some(Person {
   target, target }), body: original body, verdict })`. Verdict `Blocked`: logged as refused `filter`, stop (looks
   sent).
7. The body from here on is the filter's shown text. Recipients: the bound sessions of `target` whose account is the
   recipient; if some of them have resource exactly `res`, only those, else all.
8. Delivered = recipients non-empty; if so, remember the mid. `delivered_at` = now when delivered, or when federation
   is on (`Federation::is_federated()`) and the recipient's home (`GlobalId::owner`) is not this node; else none.
9. Stored before it is sent: `History::add_direct_once(NewDirect { sender: sender account, sender_local: uid,
   recipient: recipient account, body, stanza_id: message id, at: now, delivered_at, mid: Some(mid) })`; an error is
   logged and delivery continues.
10. Each recipient gets `<message xmlns="jabber:client" type="chat" from="{sender full JID}" to="{recipient full JID}" id="{message id}"><body>{body}</body></message>`
    (` id=...` only with a message id).
11. Info log with the delivered count; then `Federation::publish(Envelope::Direct { mid, from: sender, from_local:
    uid, to: recipient, body, id: message id, at: now, delivered })`, an error only a warning.

**Waiting messages at a bind:**

1. `History::pending_for(account, offline_batch)` (oldest first; a read error is logged and nothing is delivered).
   For each: a message whose mid is set and already seen is only marked delivered; otherwise it is sent as
   `<message xmlns="jabber:client" type="chat" from="{sender_local}@{domain}" to="{full JID}" id="{stanza id}"><body>{body}</body><delay xmlns="urn:xmpp:delay" from="{domain}" stamp="{stamp(at)}"/></message>`
   (` id=...` only with a stanza id; `domain` is the receiving session's). Then `History::mark_delivered(account, ids,
   now)` for all of them (an error is logged).
2. When federation is on and the account's home is another node: `Federation::pull_waiting(account,
   offline_batch)`. For each pulled message: an already-seen mid is only acknowledged; otherwise it is stored with
   `add_direct_once` (delivered now, its own mid), its stanza built as above (sender `from_local`), and acknowledged.
   Then `Federation::ack_waiting(account, mids, now)`, then the stanzas are sent.
3. A non-zero total is logged.

**A remote direct message** (`Event::RemoteDirect`, from `federation.rs`), answering "delivered":

1. Recipients: every non-ended bound session whose account is the message's `to` (any uid). None: `false`.
2. Mid already seen: `true`, nothing sent. Else `ChatHost::blocked(from, to)`: `true`, nothing sent.
3. Each recipient gets `<message xmlns="jabber:client" type="chat" from="{from_local}@{its domain}" to="{its full JID}" id="{stanza_id}"><body>{body}</body></message>`
   (` id=...` only with one). `true`.

**A mute set or lifted** (`Event::Muted`): every session with that account takes the new mute (or none) at once.
`Mute::holds(now)` (A) decides whether it is in force.

### 2.12 The tick and shutdown

**Tick** (`now` from the clock), for each session in order that has not ended:

- backlogged: end `backlog`;
- else not logged in and `now - opened ≥ 15 s`: end `timeout`;
- else logged in and unbound, and `now - login time ≥ 10 s`: count a bind timeout for the uid (3 in 10 minutes makes
  its logins wait 60 s), end `timeout`;
- else bound: with a ping deadline, `now ≥ deadline` ends `ping-timeout`; without one, `now - last_inbound ≥ 50 s`
  pings with a 100 s deadline.

Then, when 60 s have passed since the last sweep (the first tick always sweeps), the room sweep (§2.13.7); then
`Federation::tick(now)`; then expired throttles, log-once keys, bind-timeout lists whose times are all older than 10
minutes and SASL failure times older than 10 minutes are forgotten.

**Shutdown:** every session is ended `shutdown`; then the task polls every 20 ms, for at most 1 s, until every
connection has marked itself closed; then every connection is cut; then the task returns.

### 2.13 Rooms (MUC)

#### 2.13.1 What a room is

A room is a `RoomKind` (module-api): `naming::room_kind_of(local, this node, this node's region)` (A) maps the room
JID's local part; when that gives none and the session's module answers chat, `ChatQuery::RoomOf { name: local }` is
asked and an `Ok(ChatAnswer::RoomOf(k))` gives `k` (errors ignored). By `naming`, `General` and `General<id>` are the
network room; `City-<id>` is **this node's region room** (DD21, the 1.14.7 client's patch 0025 City rooms); `Region-
<name>` a region by name; `Party-<id>` / `Hunt-<id>` the instance's global id.

**"Two City rooms as one."** `City-a@muc.d` and `City-b@muc.d` on one node are the same room. Each occupant keeps the
room JID it joined with (local part and domain as its client spelled them), and every stanza that occupant gets about
the room names that JID: its own presences, others' presences (`{its room JID}/{their nick}`), messages (`{its room
JID}/{sender nick}`), history and errors. The rooms are kept in creation order and found by kind. A session that joins
a second JID of a room it already holds leaves under the first JID first (§2.13.2 step 11).

An occupant is `(session, nick, room JID as spelled, rules)`, kept in join order.

Two sessions of one account (same uid) never see each other in a room: "the others" of a session in a room are the
occupants that are neither it nor a session with its uid.

#### 2.13.2 Joining

Presence to room JID `local@domain/nick` (nick = the resource, empty when none; room JID = `local@domain` as sent):

1. Takes a join token (burst 10, one more every 6 s). None: refuse `limit`.
2. The kind (§2.13.1). None, or `domain` not the session's MUC domain: refuse `not-allowed`.
3. `held` = this session's occupant entry in that room, if any.
4. The rules: for a module that does not answer chat, `hooks::default_rules(kind)`; else `ChatQuery::Room { room,
   account }`: `Room(r)` is `r`; another answer none; `Unsupported` the default; another error none with a warning.
   None: refuse `unavailable`.
5. `!may_join`: if `held`, the session is removed (`evicted`, §2.13.6); refuse `not-member`.
6. `held` with exactly this nick and this room JID: a rejoin. The session alone gets the room again: each other's
   presence (from `{held JID}/{their nick}`), then its own with status 110. Its stored rules are not changed, no
   history is sent; logged as a rejoin. Stop.
7. Nick empty or over `MAX_NICK_BYTES` bytes: refuse `nick-format`.
8. `nick::check_nickname(nick, uid, resource, [display name at login])`. An error refuses with that reason when the
   nick check is `Enforce` or the reason is `Account`; under `Log` other reasons are admitted with a warning logged
   once per (session, room key) per 10 minutes.
9. `held` (other nick or JID): the session is removed (`left`).
10. Occupants of other accounts in the room (not this session, not its uid): one with a byte-equal nick refuses
    `conflict`.
11. The session holds 8 rooms already, or there are 128 occupants of other accounts, or the room does not exist and
    1000 rooms do: refuse `limit`.
12. Ghosts: each other session of this uid in the room is removed (`replaced`: the others are told, the ghost is not).
13. The room is created if it does not exist (appended).
14. The joiner gets each other's presence (`from="{joiner's room JID}/{their nick}"`); then each other gets the
    joiner's (`from="{their room JID}/{joiner nick}"`); then the joiner is added (occupant and session room list) and
    gets its own presence with status 110.
15. `pulled = Federation::room_joined(kind, rules.history ? history_on_join : 0)` (always called). If the rules keep
    history: each pulled message's mid is remembered and it is stored with `add_room_once(NewRoom { room: room key,
    sender, nick, body, stanza_id, mid: Some(mid), at })`; then the history is sent (§2.13.4).
16. Logged with the room class, uid, the decoded name (or `?`) and the count of others.

**A refused join** sends
`<presence xmlns="jabber:client" type="error" from="{room JID}/{nick}" to="{full JID}"><x xmlns="http://jabber.org/protocol/muc"/><error type="{type}"><{condition} xmlns="{stanzas}"/></error></presence>`,
logs once per (session, reason) per 10 minutes (room local part via `log_text(local, 80)`, uid, reason), and drops a
stanza (`refused`).

| Reason (log) | `type` | `condition` |
| --- | --- | --- |
| `nick-account`, `nick-resource`, `nick-format`, `nick-name`, `not-member` | `auth` | `forbidden` |
| `not-allowed` | `cancel` | `not-allowed` |
| `conflict` | `cancel` | `conflict` |
| `limit`, `unavailable` | `wait` | `service-unavailable` |

#### 2.13.3 Occupant presence

The presence of occupant `o` (as nick `n`) for session `t`, from the room as `t`'s client names it (`J`):
`<presence xmlns="jabber:client"[ type="unavailable"] from="{J}/{n}" to="{t full JID}"><x xmlns="http://jabber.org/protocol/muc#user"><item affiliation="none" role="{role}" jid="{o full JID}"/>[<status code="110"/>][<status code="307"/>]</x></presence>`,
with `role` `none` when unavailable, else `participant`; 110 when `o` is `t`; 307 when evicted.

#### 2.13.4 History on join

When `history_on_join` is 0, nothing. Else `History::room_history(room key, history_on_join)` (oldest first; an error
is logged and nothing sent), each row as
`<message xmlns="jabber:client" type="groupchat" id="{stanza id}" from="{room JID}/{row nick}" to="{full JID}"><body>{body}</body><delay xmlns="urn:xmpp:delay" from="{room JID domain}" stamp="{stamp(at)}"/></message>`
(` id=...` only with one; note it comes before `from`). Sent after the joiner's own presence.

#### 2.13.5 Leaving

Presence `unavailable` to `local@domain/r`: if the session holds a room under exactly that room JID, it is removed
(`left`). If not (after a refused join, a move to another JID, a restart) and the domain is its MUC domain and `r` is
non-empty, it still gets
`<presence xmlns="jabber:client" type="unavailable" from="{room JID}/{r}" to="{full JID}"><x xmlns="http://jabber.org/protocol/muc#user"><item affiliation="none" role="none" jid="{full JID}"/><status code="110"/></x></presence>`
so its client stops waiting.

#### 2.13.6 Removal

Removing session `s` from a room with leave reason `left`, `disconnect`, `evicted` or `replaced`:

1. Each of the others gets `s`'s unavailable presence (from `{their JID}/{s nick}`, 307 when `evicted`).
2. `s` leaves the occupants and its room list.
3. If `s` has not ended and the reason is `left` or `evicted`, `s` gets its own unavailable presence (110, and 307 when
   evicted).
4. An emptied room is dropped and `Federation::room_left(kind)` runs in the background (not awaited).
5. Logged with room class, uid and reason.

Leaving all rooms removes the session from each room holding it, then clears its room list.

#### 2.13.7 The sweep

Every 60 s (§2.12), for each room and each occupant, the rules are asked again (§2.13.2 step 4): `may_join` updates
the stored rules; `!may_join` removes the occupant (`evicted`); none (the module could not say) keeps the last rules.

#### 2.13.8 Room messages

A `groupchat` message to room JID `local@domain` (as sent):

1. Message id: the `id` attribute when at most 128 UTF-16 units.
2. No `body` child but a `subject` child: ignored.
3. Not held under that exact room JID, or body length (chars) outside 1..=2048: message error `modify` /
   `not-acceptable`.
4. Sender muted now, or its stored rules do not allow speaking: error `auth` / `forbidden`.
5. A message token (shared with whispers): none, a warning at most once a minute per session, and error `modify` /
   `not-acceptable`.
6. A report command: §2.10.3 with the room key. Stop.
7. `mid` = `rm-` + 16 random hex; `Moderation::rate(body, channel_of(kind))`; `record(Line { id: mid, at, channel:
   "city" or "party", room_id: Some(room key), sender, recipient: None, body: original, verdict })`.
8. Verdict `Blocked`: only the sender gets
   `<message xmlns="jabber:client" type="groupchat" id="{id}" from="{own JID}/{own nick}" to="{full JID}"><body>{shown}</body></message>`;
   logged as blocked by the filter; not stored, not federated.
9. Recipients: every occupant that is the sender or not of the sender's uid (the sender gets its own echo). The mid is
   remembered. If the sender's rules keep history, stored first: `add_room_once(NewRoom { room key, sender account,
   own nick, shown body, stanza id, mid, now })` (an error is logged).
10. Each recipient other than the sender is skipped when `ChatHost::blocked(sender, its account)`; the rest get
    `<message xmlns="jabber:client" type="groupchat" id="{id}" from="{its room JID}/{own nick}" to="{its full JID}"><body>{shown}</body></message>`.
11. Logged with class, uid, length, delivered count and, if any, the blocked count; then `Federation::publish(Envelope::
    Room { mid, room: kind, from: sender, nick: own nick, body: shown, id, at: now, history: own rules' history })`.

**A message error** is
`<message xmlns="jabber:client" type="error" id="{id}" from="{room JID}" to="{full JID}"><error type="{type}"><{condition} xmlns="{stanzas}"/></error></message>`
(` id=...` only with one). It is not a dropped stanza.

#### 2.13.9 Remote room messages

`Event::RemoteRoom { message }` (from `federation.rs`): an already-seen mid is ignored; a room this node does not hold
is ignored. When the message says history or any local occupant's rules keep history, it is stored with
`add_room_once` (the room key, `from`, `nick`, `body`, `stanza_id`, `mid`, `at`). Every occupant, except one whose
account `ChatHost::blocked(from, account)` names, gets `<message xmlns="jabber:client" type="groupchat" id="{stanza_id}" from="{its room JID}/{nick}" to="{its full JID}"><body>{body}</body></message>`
(no same-uid filter).

### 2.14 Nicknames

The client joins every room as `UrlEncode(DisplayName):AccountId:OwnResource`. Its encoder writes UTF-8, keeps the
unreserved bytes `A-Z a-z 0-9 - _ . ~`, writes every other byte as `%XX` with upper-case hex and drops NUL
(`url_encode_like_client`; `"é a"` gives `%C3%A9%20a`). The name part therefore never holds `:`.

`decode_name_part(p)`: `%` must be followed by two hex digits (either case) giving one byte; unreserved bytes are
kept; any other byte fails; the bytes must be valid UTF-8 (`%c3%A9` gives `é`; `%FF` fails; `A%2` fails).
`name_part_of(nick)` decodes the text before the first `:` (the whole nick without one).

`check_nickname(nick, uid, resource, names)`, the first rule that fails:

1. No `:`: `Account`.
2. What follows the first `:` must equal `{uid}:{resource}`; if not, `Account` when its first `:`-separated segment is
   not `uid`, else `Resource`.
3. Remove every occurrence of `uid` from the nick; if what remains contains `UID-`: `Account` (no other account id
   anywhere, resource included).
4. The name part must decode: else `Format`.
5. The decoded name must be `InvalidMCPUser` (the client's fallback) or equal one of the non-none `names`: else `Name`.

Success gives the decoded name. The server never rewrites a nickname.

### 2.15 The frame parser

`parse_frame` behaves exactly as the Node chat's parser (ltx 3.1.2's SAX parser and tree builder), so a frame parses
here as it did there: lenient where ltx is, strict where it is. It is linear in the input, never recursive on depth.
Callers refuse `<!` and `<?` before parsing (`has_markup_declaration` is the plain substring test for either).

It scans the input left to right in one of seven modes, tracking the position where the current token began ("the
mark"; cleared when a token is taken), a tag name, the pending attribute list, an end-tag flag, a self-closing flag,
the current quote char and the current attribute name. It keeps a stack of open elements and the finished root.

- **Text** (initial; mark at 0): jumps to the next `<`; with none left the scan stops (trailing text is never
  emitted). At `<`: the text from the mark to here, if non-empty, is unescaped (failure fails the whole frame) and
  appended as a text child of the innermost open element (dropped when none is open, so text before or after the
  root is lost). Mode becomes tag-name, mark at the next position, attribute list emptied.
- **Tag-name:** `/` exactly at the mark sets the end-tag flag and moves the mark past it. `!` or `?` fails the
  frame. A byte ≤ 32, `/` or `>` takes the name (mark to here) and switches to in-tag **without consuming** that
  byte. Other bytes continue the name.
- **In-tag:** `>` finishes the tag: an end tag closes the innermost open element only if its name is exactly this name
  (otherwise the end tag is ignored); a start tag opens an element with the collected attributes and, if the
  self-closing flag is set, closes it at once. Flags reset, mode text, mark after `>`. `/` sets the self-closing flag.
  Another byte > 32 starts an attribute name (mark here). Bytes ≤ 32 are skipped.
- **Attr-name:** a byte ≤ 32 or `=` takes the name and switches to attr-eq without consuming the byte.
- **Attr-eq:** `=` switches to attr-quote; other bytes are skipped.
- **Attr-quote:** `"` or `'` becomes the quote, mode attr-value, mark after it; other bytes are skipped.
- **Attr-value:** jumps to the next byte equal to the quote; there the value (mark to here) is unescaped (failure fails
  the frame). If the attribute name is already in the list its value is replaced in place (first position kept, last
  value wins), else the pair is appended. Mode in-tag.

Closing an element appends it to its parent, or, when no element is open, makes it the root if there is no root yet
(the first complete root wins; later roots are dropped). The result is the root, or none when no root element was
completed. All mode boundaries are ASCII bytes, so every slice is on a char boundary.

Consequences the tests pin: `<a><b></c></b></a>` has `b` as a child; `<a b c="1"/>` has the single attribute `b="1"`
(an attribute without a value takes the next one's); `<a><b/>` is none; `text only` is none; `<a/><b/>` is `a`; a
stream prefix stays in the name (`stream:features`).

**Unescaping** a token: scanning for `&`; for each, the first `;` after it ends an entity (no `;` after it: the rest
is kept literally and scanning stops). The entity must be `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, or a character
reference; anything else fails the token. A reference `&#x...;` reads its body after `x` as hexadecimal, `&#...;` as
decimal, both as JavaScript's `parseInt`: leading white space trimmed (JavaScript's white-space set), an optional sign,
for hexadecimal an optional `0x`/`0X` prefix, then the longest run of digits of that radix; no digits is a failure;
only the first 15 digits count. The code must be a legal XML char (`0x9`, `0xA`, `0xD`, `0x20..=0xD7FF`,
`0xE000..=0xFFFD`, `0x10000..=0x10FFFF`), else failure. `&#X41;` (upper-case X) is read as decimal and fails. Because
the first `;` ends the entity, `a & b; c` fails while `x & y` stays as it is.

**Reading the tree:** `attr(n)` is the value of the attribute named exactly `n`; `text()` concatenates the element's own
text children; `child(n)` is the first child element whose local name equals `n`; `elements()` are the child elements
in order; `text_of(None)` is empty. `local_name(name)` is the part after the last `:` (the whole name without one),
Unicode lower-cased.

### 2.16 Text helpers

- `escape`: `&` `<` `>` `"` `'` become `&amp;` `&lt;` `&gt;` `&quot;` `&apos;`; nothing else changes.
- `log_text(text, max)`: the first `max` chars, each char outside `0x21..=0x7E` replaced by `?`, followed by `...`
  when the text had more than `max` chars (`log_text("ab cé", 3)` is `ab?...`).
- `is_host_name`: 1 to 253 bytes, all ASCII letters, digits, `.` or `-`.
- `is_xmpp_domain`: non-empty, at most 259 bytes; split at the first `:`; the host part `is_host_name`; a port part,
  when present, 1 to 5 ASCII digits with a value 1..=65535 (`127.0.0.1:61000` yes; `host:0`, `host:123456`, `host:`,
  `bad host` no).
- `parse_jid`: the resource is everything after the first `/` (possibly empty, possibly holding `/`); the rest needs
  an `@` neither first nor last; local is before the first `@`, domain after it (it may hold another `@`).
  `City-1@muc.dom/Name:UID-a:V2:x/y` gives local `City-1`, domain `muc.dom`, resource `Name:UID-a:V2:x/y`; `@dom` and
  `a@` give none.
- `has_control_character`: any char of general category Cc.
- **Token bucket:** starts full (`burst` tokens) at `now`; taking refills `elapsed / refill_ms` tokens (elapsed
  clamped at 0, the total capped at `burst`), moves its time to `now`, and succeeds by spending one whole token when
  at least one is there (burst 2, refill 1000 ms: two at time 0, not a third, one more at 1000).

### 2.17 Trace redaction (`redact_frame`)

With `trace` on, every frame in and stanza out is logged through `redact_frame`, which:

1. For `auth`, then `password`, then `body`: finds `<tag` case-insensitively (ASCII) where the next char is not a
   letter, digit or `_` (a longer name such as `authx` is skipped), then the next `>`, then the next `</tag>`, and
   replaces the content between `>` and `</tag>` with `[redacted]` (`auth`, `password`) or `[N chars]` (`body`, N
   counted in chars). An occurrence with no `>` or no closing tag ends that tag's pass, leaving the rest as it is.
2. Applies `redact` (§2.18).
3. Over 2048 UTF-16 units: keeps the longest prefix of whole chars within 2048 units and appends `...`.

`<auth mechanism='PLAIN'>c2VjcmV0</auth>` gives `<auth mechanism='PLAIN'>[redacted]</auth>`;
`<message><body>héllo</body></message>` gives `<message><body>[5 chars]</body></message>`.

### 2.18 Credential redaction (`redact`)

Four passes, in this order, each linear and byte-wise on ASCII patterns:

1. **JWTs:** at each `eyJ`: a run of `[A-Za-z0-9_-]` right after it (at least one char), `.`, a second run (at least
   one), `.`, a third run (at least one). Then `eyJ` through the end of the third run becomes `<token>` and scanning
   continues after it. Otherwise `eyJ` is kept and scanning continues after it.
2. **Account keys** with prefix `RRK_` (`protocol::account::ACCOUNT_KEY_PREFIX`): the prefix followed by exactly 48 hex
   digits not followed by another hex digit becomes `<redacted>`; otherwise the prefix is kept and scanning continues
   after it.
3. The same for `UUK_` (a Dauntless Revived database's keys).
4. **Long runs:** every maximal run of 64 or more `[A-Za-z0-9_-]` becomes `<redacted>`.

Pinned cases: `eyJ.a.b`, `eyJa..b`, `eyJ.only`, `eyJabc.def` unchanged; `xeyJa.b.c.d` gives `x<token>.d`;
`eyJa.b.c` + 70 `z` gives `<token>`; `/account/api/oauth/sessions/kill/eyJhbGc.eyJ1c2Vy.c2ln` ends in `/<token>`;
`UUK_` + 48 `a` gives `<redacted>`, + 49 `a` unchanged; `key=RRK_<48 b>&x=1` gives `key=<redacted>&x=1`; 64 `x` give
`<redacted>`, 63 stay; `/character/UID-1234` unchanged.

### 2.19 Logging rules

Server logs are English and a host's tool, not a contract, but these hold: no token, SASL payload, password, message
body or `GATEWAY_SECRET` is logged at any level; client text in a log goes through `log_text` or the uid; `trace` logs
only `redact_frame` output. Every line starts `chat: `, and the session lines carry `c=<session id>`, `uid=`, `reason=`
in `key=value` form, which hosts grep: connect (`from=`, `via=`), login refused, login ok, bound, closed (`after=<s>s`),
dropped, join, join refused, leave, message, whisper, offline messages delivered, report. Metrics are `moderation.rs`'s
(A).

---

## 3. Contracts kept by Category A that the rewrite must honour

These are not rewritten. They are stated so that the implementer can check the rewrite against them without reading
the A code for anything but the API.

### 3.1 Federation between nodes (`federation.rs`, DN10b)

Over the node link's `Service::Chat` (`/__node/chat`), each call one JSON request and one JSON reply, at most
`network::link::MAX_MESSAGE` (1 MiB) each, `deny_unknown_fields` everywhere.

- Request `{"version":1,"op":{"op":"<name>",...}}`; reply `{"version":1,"reply":{"reply":"<name>",...}}`. Another
  version is refused.
- Ops: `resolve {local}`; `presence {account, local, online}`; `store_direct {message}`; `route_direct {message}`;
  `deliver_direct {message}`; `pull_waiting {account, limit}` (1..=100); `ack_waiting {account, mids, at}` (1..=200
  mids); `room_join {room, history_limit}` (≤ 100); `room_leave {room}`; `room_post {message}`; `room_fanout
  {message}`; `get_mute {account}`; `set_mute {account, mute?, actor}`.
- Replies: `ack`; `resolved {account?}`; `direct {delivered}`; `waiting {messages}` (≤ 100); `room_history
  {messages}` (≤ 100); `mute {mute?}`.
- `DirectWire {mid, from, from_local, to, body, stanza_id?, at, delivered}`; `RoomWire {mid, room, from, nick, body,
  stanza_id?, at, history}`; `MuteWire {account, until?, reason?, by, at}`. `GlobalId`s are their text
  (`account:<ULID>@<node>`); `RoomKind` is `"network"` or `{"region":"<r>"}`, `{"party":"<id>"}`, `{"hunt":"<id>"}`.
- Bounds: mid 1..=128 chars with no white space or control char; `from_local` and `local` `is_authcid_shape`; body
  1..=`BODY_LIMIT` chars; stanza id 1..=128 UTF-16 units, no control char; nick 1..=`MAX_NICK_BYTES` bytes, no control
  char; `at ≥ 0`; mute reason ≤ 512 chars, actor and `by` 1..=256 bytes, no control char.
- Error texts (`ChatWireError` display): `chat federation message is {n} bytes; max is {MAX}`, `invalid JSON: {e}`,
  `unsupported version {v}`, `invalid field: {what}`; `FederationError::Unreachable`: `the node that owns it did not
  answer: {e}`.
- Routing: a room's owner is the leader for the network room, the first node (by id) of the region for a region room
  (the leader when none), `id.owner()` for party and hunt rooms. Direct messages are stored at both homes and routed
  through the recipient's home; waiting ones wait there until pulled at login. Unreachable calls queue in
  `chat_outbox` (at most 4096 rows, 64 per tick).

The rewrite meets this through the `Federation` calls of §2.9-§2.13 and the three `Event` variants of §2.1: the A code
sends `Event::RemoteDirect` and awaits its answer, `Event::RemoteRoom`, and `Event::Muted` (for a mute pushed from a
peer).

### 3.2 Chat's database (`chat.db`)

`History::open(config)` (A) migrates it; on SQLite `History::beside(game)` puts `chat.db` next to the game's file, on
Postgres a `chat` schema in the same database. Tables after both migrations:

| Table | Columns |
| --- | --- |
| `chat_accounts` | `jid_local TEXT PRIMARY KEY`, `account TEXT NOT NULL`, `module TEXT NOT NULL`, `domain TEXT NOT NULL`, `seen_at BIGINT NOT NULL` |
| `chat_room_messages` | `id` (autoincrement / identity), `room`, `sender`, `nick`, `body TEXT NOT NULL`, `stanza_id TEXT`, `at BIGINT NOT NULL`, `mid TEXT`; index `(room, id)`; unique `(room, mid)` |
| `chat_direct_messages` | `id`, `sender`, `sender_local`, `recipient`, `body TEXT NOT NULL`, `stanza_id TEXT`, `at BIGINT NOT NULL`, `delivered_at BIGINT`, `mid TEXT`; index `(recipient, delivered_at, id)`; unique `(mid)` |
| `chat_mutes` | `account TEXT PRIMARY KEY`, `until_at BIGINT`, `reason TEXT`, `muted_by TEXT NOT NULL`, `at BIGINT NOT NULL` |
| `chat_outbox` | `id`, `peer TEXT`, `call TEXT` (the request JSON), `tries BIGINT`, `next_at BIGINT`, `created_at BIGINT` (all `NOT NULL`); index `(next_at, id)` |

`room` holds `naming::room_key` (`network`, `region:<name>`, or the instance's `GlobalId` text); `sender` and
`recipient` the account's `GlobalId` text. Every write goes through `store`'s change log (DD23), under the player for
messages, the admin actor for mutes, `system:chat-federation` for replicated room messages.

### 3.3 Env settings

Chat reads the gateway's env file (`gateway.env`, which the host kit writes); in the library, the core hands it the
same file plus the node database. `env.rs` (A) reads:

| Variable | Effect |
| --- | --- |
| `GATEWAY_SECRET` | trimmed; non-empty becomes `Settings::gateway_secret` |
| `CHAT_PORT` | trimmed, a `u16` becomes the port; unparsable is ignored (the default stays) |
| `CHAT_NICK_CHECK` | `log` (any case, trimmed) is `NickCheck::Log`; anything else `Enforce` |
| `CHAT_TRACE` | `1`, `true` or `TRUE` (trimmed) turns `trace` on |
| `CHAT_DASHBOARD_URL`, `CHAT_DASHBOARD_INGEST_KEY` | the staff dashboard; none without a URL |
| `REFORGED_DB_ENGINE`, `REFORGED_DB_FILE`, `REFORGED_DB_*` (library only) | the node database, `store::DatabaseConfig::from_lookup` |

Errors: `chat settings: <problems joined by "; ">`; the library's `chat: REFORGED_DB_ENGINE is not set`, `chat:
REFORGED_DB_FILE is not set`, `chat: no link to the core`, `chat: the core sent no setup`, `chat history database:
{e}`. The dashboard's problems are `the dashboard URL is http:// or https://` and `the dashboard ingest key is at
least 32 characters`.

### 3.4 The host seam (`ChatHost`) and the library messages

`hooks.rs` (A): `ChatHost` has `login(Login) -> Result<Player, LoginRefusal>`, `ask(module, ChatQuery) ->
Result<ChatAnswer, ModuleError>`, and defaulted `blocked(sender, recipient) -> bool` (false), `account_of(local) ->
Option<GlobalId>` (none), `presence(account, local, online)` (nothing). `LoginRefusal::as_str` is `expired`,
`bad-token`, `no-account`, `unavailable`. `default_jid(authcid, domain)` is `authcid@domain`; `default_rules(kind)` is
join and speak everywhere, history for party and hunt rooms only. `Drivers` answers `ask` from a map of
`ModuleDriver`s (an unknown module answers `Unsupported`).

In-process, restricted's `crates/dainty-host/src/compose.rs` implements `ChatHost` over the controller (`ControllerChatHost`).
As a library, `chat-plugin` (A) implements it over dainty-lass's `service_plugin::chat::ChatLink`
(`crates/dainty-service-plugin/src/chat.rs`, MIT, unchanged by this work): the core sends `ToChat::{Setup, Loaded,
Unloaded, Answer, HomeLoggedIn, Account, PeerAnswer, PeerServe}` and chat sends `FromChat::{Ask, HomeLogin,
FindAccount, Presence, PeerCall, PeerServed}`, tagged `op` in snake case, each request with an id the asker chose, at
most 256 pending a side, 30 s for an answer, strings bounded (`MAX_LOCAL` 256, `MAX_TOKEN` 16 KiB, `MAX_REASON` 512,
`MAX_PEERS` 1024). Login there tries `ChatQuery::Login` on each loaded chat module (the attached ones, or the core's
loaded list when none are attached), then, for an account `Federation::resolve` finds homed elsewhere,
`ChatLink::home_login`; `Unavailable` when a module was unavailable, else `BadToken`. `blocked` is the first
`MayWhisper` answer negated; `account_of` the first `AccountOf(Some)`, then `ChatLink::find_account`; `presence`
emits `chat.presence` (`PresenceEvent`). The rewrite calls `ChatHost` only as §2 says and never assumes which
implementation is behind it.

---

## 4. Error texts and wire strings that are a contract

| Where | Text |
| --- | --- |
| `StartError` | `chat settings: {0}`; `chat could not listen on {address} ({source})` |
| `Settings::problems` | §2.2's three texts |
| SASL failures | `<failure xmlns="urn:ietf:params:xml:ns:xmpp-sasl"><not-authorized/></failure>`, `<temporary-auth-failure/>`, `<invalid-mechanism/>` |
| Stream error | `restricted-xml` (§2.8 step 5) |
| IQ errors | `auth`/`not-authorized` before login; `modify`/`bad-request` at bind |
| Join refusals | §2.13.2's table |
| Message errors | `modify`/`not-acceptable`; `auth`/`forbidden` |
| `/report` answers | §2.10.3's five texts |
| HTTP | `404`/`503` of §2.4 (empty bodies); §2.5's JSON error texts |
| Federation, env | §3.1, §3.3 (A) |

---

## 5. Tests that define the behaviour

Every test below is Category A or is rewritten here as a black-box test. The implementer may read and run all of the
A tests once they are in dainty-lass; they exercise the B code only through §2.1's API and real sockets.

### 5.1 Black-box suites to reuse as they are

| Test | What it fixes |
| --- | --- |
| `tests/chat.rs::players_log_in_join_a_room_and_talk` | wrong token and unknown account give `not-authorized`; `City-a` and `City-b` are one room, each client seeing its own JID; presence `jid` is the full JID; 110; escaped bodies; the sender's echo; region rooms keep no history by default; someone else's nick is `forbidden`; `Guild-x` is `not-allowed`; another path fails the upgrade |
| `tests/chat.rs::a_whisper_to_someone_offline_arrives_at_their_next_login` | nothing comes back to the sender; the waiting whisper comes with `from="ada@dom"`, a `<delay xmlns="urn:xmpp:delay"`, ` id="w1"`, once; a live whisper comes from the full JID without delay; both stored and delivered |
| `tests/chat.rs::party_rooms_keep_history_for_the_next_joiner` | two history messages, after the joiner's 110, in order, from the room JID with the sender's nick; stored under the party id with the sender's account |
| `tests/chat.rs::a_muted_player_is_not_heard` | `Running::mute` refuses room messages (`forbidden`) and drops whispers; `by` is `admin:mod`; the mute holds across logins; `unmute` answers true and lifts it |
| `tests/chat.rs::a_module_gives_the_jid_and_the_rules` | a module's JID local part must match the authcid (`impostor` refused); its read-only network room answers `forbidden`; `attach`/`detach` |
| `tests/chat.rs::the_plugin_serves_chat` | name and capability |
| `tests/chat.rs::the_dashboards_filter_applies_and_chat_and_reports_reach_it`, `without_a_dashboard_reports_say_so_and_chat_is_untouched` | moderation through the whisper and room paths, the report answers |
| `tests/federation.rs` (5 tests over `network::inprocess`) | DMs across nodes once, both histories; offline DMs wait at the recipient's home; rooms and room history across nodes; a mute at one node holds at another; the outbox catches up after a partition |
| `chat-plugin/tests/load.rs::players_chat_through_the_library_the_core_loads` | the same login, offline whisper and room scenarios through the library and the core |
| `tests/history.rs`, `tests/dashboard_live.rs` | A code only (`History`, `Moderation`) |

### 5.2 Unit tests to rewrite with the B code

The B files carry unit tests that fix §2's pinned cases; write equivalent tests (names may stay, they describe
behaviour):

- `settings`: `chat_stays_on_loopback` (§2.2).
- `connection`: `only_the_gateway_names_the_player` (§2.4).
- `server`: `sasl_payloads_decode_like_buffer_from`, `log_once_keeps_a_window` (key `a`, window 1000: yes at 0, no at
  999, yes at 1000), `authcids_are_jid_local_parts` (`UID-1234`, a UUID, `player` yes; empty, `a@b`, `a/b`, `a b`,
  `a\0`, 129 bytes no), `stamps_are_utc_with_milliseconds` (§2.1).
- `muc`: `muc_domains_follow_the_account_domain` (`Dom` with `MUC.dom` and `conference.dom` yes, `muc.other` no).
- `nick`: `nicknames_are_checked_like_node`, with the cases of §2.14 (resource `V2:app:WIN::0123`: the right nick for
  `Ada Lovelace`; the fallback name with no known name; `Bo` against `Ada` is `Name`; `Ada` is `Account`;
  `Ada:UID-b:x` is `Account`; `Ada:UID-a:other` is `Resource`; `A%2:...` and `%FF:...` are `Format`;
  `UID-b:UID-a:...` is `Account`).
- `xmpp`: `frames_parse_as_ltx_parses_them`, `helpers_match_node`, `the_trace_is_redacted` (§2.15-§2.17).
- `redact`: `matches_node`, `credentials_never_reach_the_log` (§2.18).

### 5.3 Fuzz targets

`chat_stanza` (any bytes, read lossily as UTF-8): unless it has a markup declaration, a parsed tree's every name,
attribute and text is no longer than the input and its depth no deeper; `text()`, `child("body")`, `redact_frame`,
`log_text`, `escape` never panic; an `is_authcid_shape` input holds no `@` or `/` and is at most 128 bytes; a parsed JID
whose local part names a party or hunt room has a JID-safe id, a region room a non-empty name, the network room a
`General` prefix; `check_nickname` and `name_part_of` never panic. `chat_federation`: the decoders never panic and
every accepted request and reply round-trips (A code; see §7 item 2). Both move to dainty-lass `fuzz/` with chat.

---

## 6. The target MIT design

### 6.1 Where it goes

| dainty-restricted today | dainty-lass after |
| --- | --- |
| `crates/dainty-chat/` (crate `chat`) | `crates/dainty-chat/`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud` and `// SPDX-License-Identifier: MIT` on every source file. Category A files moved as they are; Category B files written from §2 |
| `crates/dainty-chat-plugin/` (crate `chat-plugin`, `cdylib`) | `crates/dainty-chat-plugin/`, MIT, moved as it is. Whether the MIT core builds chat in (as it does admin and remote) or keeps loading the library is the maintainer's call (RD27 row); the library form needs no core change |
| `fuzz/fuzz_targets/chat_{stanza,federation}.rs` | dainty-lass `fuzz/fuzz_targets/`, with their `[[bin]]` entries |
| `docs/security/chat.md` | dainty-lass `docs/security/chat.md` (indexed in `SECURITY_REVIEW.md` by the integration pass) |

Every dependency is already in dainty-lass's workspace and MIT: `module-api`, `network`, `protocol`
(`ACCOUNT_KEY_PREFIX`), `service-kit` (`js::trim`, `serve`, `env`, `metrics`), `store` (`Database`, `portable`,
`Actor`), `service-plugin`; the external crates (`axum` with `ws`, `diesel`, `diesel_migrations`, `regex`, `sha2`,
`subtle`, `ulid`, `rand`, `chrono`, `hyper*`, `rustls`, `tokio-tungstenite` for tests) are workspace dependencies
already. Adding `crates/dainty-chat` and `crates/dainty-chat-plugin` to the workspace `members` and `chat` to
`[workspace.dependencies]` are seam changes for the integration pass.

### 6.2 What dainty-restricted needs afterwards

Restricted deletes both folders and takes `chat` and `chat-plugin` from dainty-lass by git rev, like the other MIT
crates. Its remaining callers use only this public API:

- `crates/dainty-host/src/compose.rs` (in-process chat; MIT code that stays there): `chat::settings::DEFAULT_PORT`,
  `chat::env::{settings, dashboard}`, `ChatPlugin::{new, start, modules}` and its `CorePlugin` impl, `Drivers::{default,
  insert, remove, module_ids, ask}`, `ChatHost`, `Login`, `Player`, `LoginRefusal`, `History::{open, beside}`,
  `Federation`, `ChatFederation::new` (and its `NodeService` impl), `ChatFederationConfig::{new, with_leader}`,
  `LocalOnly`, `Options` (`moderation`, `Default`), `Moderation::{off, start}`, `Running::{port, stop}`.
- The release and Docker builds ship the chat library from dainty-lass's build instead of restricted's (RD27 row
  owners).
- The one test that needs the AGPL gateway (§6.3).

No game module calls the chat crate: `rustic-isles` talks to chat only through `module-api` (`GameModule::chat`,
`ChatQuery`), which does not change.

### 6.3 The gateway test

`tests/federation.rs::federation_works_end_to_end_over_mutual_tls_gateways` runs two chats behind two real
`gateway::Gateway`s (AGPL) with mutual TLS. It cannot stay in an MIT crate's tests, and `gateway` must leave chat's
`[dev-dependencies]`. Move that one test to restricted (for example `crates/dainty-gateway/tests/`) against `chat` as a git
dependency, or rewrite it on `network`'s `TlsPeerLink` and `ServiceMux` without the gateway. The file's other five tests
use `network::inprocess` and move with it.

### 6.4 The clean-room procedure

As `docs/port/clean-room/hostkit.md` records for DL3b:

1. Before the implementer starts, move the Category A files of §1.2 into dainty-lass (with the MIT headers and the two
   manifest edits) and remove `gateway` from the dev-dependencies.
2. The implementer works in a worktree whose sparse checkout excludes every AGPL tree and has no dainty-restricted
   checkout reachable; it reads this spec, the moved A files, the MIT workspace crates and `docs/`.
3. Record the session, the files read and written, and the test, clippy and `cargo tree -e normal -p dainty-chat` results in
   `docs/port/clean-room/chat.md`, and add the files to `docs/port/clean-room/similarity.md` measured against
   restricted `849f45e` `crates/dainty-chat/src/` and DR's `services/metagame/src/chat/` (legacy-revived `cfcad3eb`). Explain
   every file at or above 25 % as §3 of that report does (wire templates, public signatures and the pinned constants
   are expected to match).
4. The integration pass records chat under "Clean-room MIT rewrites" in dainty-lass `NOTICE.md` and removes it from
   restricted's NOTICE, updates `agpl-inventory.md` and the migration guide's §2 row.

---

## 7. Risks and ambiguities

1. **Mixed files.** `connection.rs`, `server.rs`, `muc.rs` and `lib.rs` hold code written here (the `/__chat/*` API,
   offline and federated delivery, `/report`, remote messages) inside DR's structure. All four are rewritten whole
   (§1.5); nothing is salvaged from them.
2. **`chat_federation` fuzz target asserts more than the validator checks.** `validate_envelope` accepts an empty
   `mid` (it checks a mid only when non-empty), and bounds the body in chars, while the target asserts a non-empty mid
   and `body.len() <= BODY_LIMIT` in bytes. An envelope with no `mid`, or a 2048-char body with multi-byte chars,
   should crash it. This is A code: fix the target (or the validator) in the move, not in the rewrite.
3. **`BODY_LIMIT` counts chars** everywhere chat and the wire check it; only the `chat_federation` target reads it as
   bytes. The rewrite keeps chars (§2.0), as the Node chat did.
4. **The `/__chat/*` routes** have no caller and are reachable by any local process that omits the gateway header:
   `client/send` with `xml` injects raw stanzas as a logged-in test client, and `mute` needs no key. They are kept for
   parity; the maintainer may want them behind a flag or removed (a behaviour change, so a decision row).
5. **`federation.rs`'s actor parsing** maps `host:<x>` to `system:host:<x>` (it knows only `admin:`, `player:`,
   `system:`). A code, noted only.
6. **Clock use is mixed.** Mutes take the real clock (`now_ms`) for their `at`, everything else `Options::clock`; a
   test with a fake clock sees real times on mutes. Kept as it is.
7. **Presence at every bind.** `presence(true)` is sent at each bind, not only the first session's; `presence(false)`
   only when the last goes. Consumers must be idempotent. Kept.
8. **Mystic Paradox.** The 1.14.7 subset (the three paths, room prefixes, SASL PLAIN with the game token, the
   `name:id:resource` nick, `normal`/absent message types) is behaviour read from Paradox's realtime gateway; this
   spec names it and copies nothing, so Paradox's additional terms do not reach the MIT crate. The implementer must not
   open `modules/rustic-isles/reference/`.
9. **The XML parser is ltx's behaviour, not XML's.** The quirks in §2.15 (the first `;` ending an entity, an
   attribute without a value taking the next one's, mismatched end tags ignored) are deliberate; a "correct" parser
   would change what the game sees. ltx itself is MIT, and §2.15 describes its observable behaviour.
10. **Release shape.** Moving `chat-plugin` to dainty-lass moves a shipped library from restricted's release to
    dainty-lass's, or into the core; either is an RD27 decision for the maintainer, not this spec's.
