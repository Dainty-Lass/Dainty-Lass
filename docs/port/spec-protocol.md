# Clean-room Behaviour Specification: `protocol` (`crates/dainty-protocol`)

## 1. Purpose and Scope

`protocol` (`crates/dainty-protocol`) is a pure, synchronous validation and wire-contract crate shared by the launcher, host kit, controller, and module seams. It has no network, filesystem, or database side effects.

- **Package name**: `protocol`
- **Edition**: `2024`
- **Licence**: `MIT` (`Copyright (c) 2026 Silent Spud`)
- **Normal dependencies**:
  - `module-api` (workspace, MIT) — re-exports `module_api::GameVersion`
  - `serde` (`derive`)
  - `serde_json`
  - `ts-rs` (optional, enabled by the `ts` Cargo feature)
- **Cargo features**:
  - `ts = ["dep:ts-rs"]`: derives `ts_rs::TS` on wire types exported to the UI bridge (`UsernameCheck`, `ServerMode`, `Invite`, `InviteError`, `RegistrationMode`, `PlayerWhere`, `StatusPlayer`, `InstanceKind`, `StatusInstance`, `GameDataKind`, `StatusGame`, `ServerStatus`).
  - `legacy = []`: enables parsing and formatting of legacy `1.4.4` (`dauntless-revived://`) invites instead of rejecting them with `InviteError::Revived`.

### Crate Root Exports (`src/lib.rs`)

- Public modules: `pub mod account;`, `pub mod invite;`, `pub mod manifest;`, `pub mod status;`, `pub mod text;`
- Re-export: `pub use module_api::GameVersion;`

---

## 2. Module `text` (`protocol::text`)

Provides ECMAScript-compatible string trimming and control/format character stripping used across protocol parsers.

### Public Signatures

```rust
pub fn is_js_whitespace(ch: char) -> bool;
pub fn js_trim(s: &str) -> &str;
pub fn strip_control_and_format(s: &str) -> String;
pub fn has_control_or_format(s: &str) -> bool;
```

### Observable Behaviour

1. **ECMAScript whitespace (`is_js_whitespace`, `js_trim`)**:
   - Matches ECMAScript `String.prototype.trim` (`White_Space` + `LineTerminator` + `FEFF`):
     - `'\u{0009}'..='\u{000D}'` (TAB, LF, VT, FF, CR)
     - `'\u{0020}'` (SPACE)
     - `'\u{00A0}'` (NO-BREAK SPACE)
     - `'\u{1680}'` (OGHAM SPACE MARK)
     - `'\u{2000}'..='\u{200A}'` (EN QUAD through HAIR SPACE)
     - `'\u{2028}'` (LINE SEPARATOR)
     - `'\u{2029}'` (PARAGRAPH SEPARATOR)
     - `'\u{202F}'` (NARROW NO-BREAK SPACE)
     - `'\u{205F}'` (MEDIUM MATHEMATICAL SPACE)
     - `'\u{3000}'` (IDEOGRAPHIC SPACE)
     - `'\u{FEFF}'` (ZERO WIDTH NO-BREAK SPACE / BOM)
   - Note: unlike Rust's `char::is_whitespace`, `'\u{0085}'` (NEXT LINE) is **not** ECMAScript whitespace and must not be trimmed by `js_trim`.
   - `js_trim(s)` strips leading and trailing characters satisfying `is_js_whitespace`.

2. **Control and directional/format characters (`strip_control_and_format`, `has_control_or_format`)**:
   - A character is classified as control or format if its Unicode scalar value falls in any of these ranges:
     - `0x0000..=0x001F` (C0 controls)
     - `0x007F..=0x009F` (DEL and C1 controls)
     - `0x061C` (ARABIC LETTER MARK)
     - `0x200B..=0x200F` (zero-width spaces and LTR/RTL marks)
     - `0x202A..=0x202E` (bidirectional overrides/embeddings, including RLO `\u{202E}`)
     - `0x2060..=0x2069` (word joiner, invisible operators, bidirectional isolates)
     - `0xFEFF` (BOM)
     - `0xFFF9..=0xFFFB` (interlinear annotation controls)
   - `strip_control_and_format(s)` returns a `String` with all such characters removed, followed by `js_trim` on the result.
   - `has_control_or_format(s)` returns `true` if `s` contains at least one such character.

---

## 3. Module `account` (`protocol::account`)

Validates player display names (`username`) and account keys (`RRK_...`).

### Public Signatures

```rust
pub const KEY_PREFIX: &str = "RRK_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum UsernameCheck {
    Ok,
    Empty,
    TooShort,
    TooLong,
    BadChars,
}

impl UsernameCheck {
    pub const fn as_str(self) -> &'static str;
}

pub fn check_username(name: &str) -> UsernameCheck;
pub fn is_valid_username(name: &str) -> bool;
pub fn is_plausible_account_key(key: &str) -> bool;
pub fn extract_account_key(text: &str) -> Option<&str>;
```

### Observable Behaviour

1. **Username validation (`check_username`, `is_valid_username`)**:
   - Does **not** trim the input string (leading or trailing spaces fail character validation).
   - Evaluates in order:
     1. If `name.is_empty()` -> `UsernameCheck::Empty` (`"empty"`).
     2. If `name.len() < 3` -> `UsernameCheck::TooShort` (`"too_short"`).
     3. If `name.len() > 16` -> `UsernameCheck::TooLong` (`"too_long"`).
     4. Every byte must be ASCII alphanumeric (`A-Z`, `a-z`, `0-9`) or underscore (`_`). Otherwise -> `UsernameCheck::BadChars` (`"bad_chars"`).
     5. Otherwise -> `UsernameCheck::Ok` (`"ok"`).
   - `is_valid_username(name)` returns `check_username(name) == UsernameCheck::Ok`.

2. **Plausible account key (`is_plausible_account_key`)**:
   - Returns `true` if and only if:
     - `key.len()` is in `16..=128`;
     - `key` starts with `"RRK_"` (`KEY_PREFIX`);
     - every byte in `key` is printable ASCII in `0x21..=0x7E` excluding `"` (`b'"'`), `'` (`b'\''`), and `\` (`b'\\'`).

3. **Account key extraction (`extract_account_key`)**:
   - First trims `text` with `js_trim(text)`. If the trimmed slice satisfies `is_plausible_account_key`, returns `Some(trimmed)`.
   - Otherwise searches `text` for a server-minted key token of the exact form `RRK_` followed by at least 24 lowercase hex characters (`0-9`, `a-f`), up to 120 hex characters, bounded on both sides by non-word characters (where a word character is ASCII alphanumeric or `_`).
   - If found and the matched slice satisfies `is_plausible_account_key`, returns `Some(matched)`. Otherwise returns `None`.

---

## 4. Module `invite` (`protocol::invite`)

Parses and formats `dauntless://join?...`, `reforged-revival://join?...`, and `dauntless-revived://join?...` server invite URLs.

### Public Signatures

```rust
pub const SCHEME: &str = "dauntless";
pub const REFORGED_SCHEME: &str = "reforged-revival";
pub const REVIVED_SCHEME: &str = "dauntless-revived";
pub const MAX_INVITE_LENGTH: usize = 2048;
pub const DEFAULT_PUBLIC_PORT: u16 = 443;

pub const fn reforged_version() -> GameVersion; // GameVersion::new(1, 14, 7)
pub const fn revived_version() -> GameVersion;  // GameVersion::new(1, 4, 4)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ServerMode {
    Private,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Invite {
    pub mode: ServerMode,
    pub host: String,
    pub port: u16,
    pub code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub share: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub fp: Option<String>,
    #[serde(default = "reforged_version")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub version: GameVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InviteError {
    Empty,
    TooLong,
    Revived,
    NotInvite,
    Params,
    Version,
    Mode,
    Host,
    Port,
    Fp,
    Code,
    Name,
    Share,
}

impl InviteError {
    pub const fn as_str(self) -> &'static str;
}

impl std::fmt::Display for InviteError { ... }
impl std::error::Error for InviteError {}

pub fn parse_invite(raw: &str) -> Result<Invite, InviteError>;
pub fn format_invite(invite: &Invite) -> Result<String, InviteError>;
pub fn is_valid_host(host: &str) -> bool;
pub fn is_private_mode_host(host: &str) -> bool;
pub fn is_tailscale_ipv4(host: &str) -> bool;
pub fn is_valid_fingerprint(fp: &str) -> bool;
```

### Observable Behaviour

1. **Pre-checks and scheme recognition (`parse_invite`)**:
   - Trims `raw` using `js_trim`.
   - If empty -> `Err(InviteError::Empty)`.
   - If `trimmed.encode_utf16().count() > 2048` (`MAX_INVITE_LENGTH`, matching JS `String.length` in UTF-16 code units) -> `Err(InviteError::TooLong)`.
   - Matches prefix case-insensitively (`^scheme://join(?:\?|\?.*|/\?.*)$`):
     - `dauntless://join?...` or `dauntless://join/?...`: unified scheme (default game version `1.14.7`, overridden by optional `version=` query param).
     - `reforged-revival://join?...` or `reforged-revival://join/?...`: alias pinned to `1.14.7`.
     - `dauntless-revived://join?...` or `dauntless-revived://join/?...`: alias pinned to `1.4.4`. When Cargo feature `legacy` is **not** enabled, any URL matching `^dauntless-revived://join(?:[/?]|$)` (case-insensitively) immediately returns `Err(InviteError::Revived)` before inspecting query parameters.
     - Anything else (including `dauntless-revived://register?...` or missing query `?`) -> `Err(InviteError::NotInvite)`.

2. **Query parameter parsing**:
   - If the query string after `?` is empty or contains `#` -> `Err(InviteError::Params)`.
   - Splits on `&`. Each part must contain `=` with a non-empty key; keys must be unique (duplicate keys -> `Err(InviteError::Params)`).
   - Keys and values are decoded with `application/x-www-form-urlencoded` rules (`+` -> space, `%XX` -> UTF-8 bytes; malformed percent-encoding or invalid UTF-8 -> `Err(InviteError::Params)`).
   - Allowed keys:
     - `v = "1"`: `v`, `host`, `port`, `code`, `name`, optional `share`, optional `version`.
     - `v = "2"`: `v`, `mode`, `host`, `fp`, `code`, `name`, optional `port`, optional `version`.
     - Any unknown key (or `fp`/`mode` on `v=1`, or `share` on `v=2`) -> `Err(InviteError::Params)`.
   - `v` parameter:
     - Must be `"1"` or `"2"`; missing or any other value -> `Err(InviteError::Version)`.
   - `version` parameter (optional):
     - Accepted values: `"1.14.7"` or `"reforged"` -> `GameVersion::new(1, 14, 7)`; `"1.4.4"` or `"revived"` -> `GameVersion::new(1, 4, 4)`. Any other value -> `Err(InviteError::Params)`.
     - If the scheme was an edition-specific alias (`reforged-revival` or `dauntless-revived`) and `version` resolves to a different `GameVersion` -> `Err(InviteError::Params)`.
     - If the resolved `GameVersion` is `1.4.4` and Cargo feature `legacy` is **not** enabled -> `Err(InviteError::Revived)`.

3. **Field validation rules**:
   - **Mode (`v=2` only)**: `mode` must be `"public"`; missing or anything else -> `Err(InviteError::Mode)`. (`v=1` is always `ServerMode::Private`).
   - **Fingerprint (`v=2` only, `is_valid_fingerprint`)**: `fp` must be exactly 64 lowercase ASCII hex characters (`0-9`, `a-f`); missing or invalid -> `Err(InviteError::Fp)`.
   - **Host (`is_valid_host`, `is_private_mode_host`, `is_tailscale_ipv4`)**:
     - `parse_invite` trims `host` with `js_trim` and lowercases ASCII characters (`to_ascii_lowercase`) before calling `is_valid_host`.
     - `is_valid_host(host)` requires `1..=253` bytes, already lowercase:
       - Either a strict dotted-quad IPv4 address (`A.B.C.D`, four base-10 octets `0..=255` with no leading zeros), or
       - A DNS name whose labels (split on `.`) are each `1..=63` bytes of `[a-z0-9-]`, neither starting nor ending with `-`, and whose last label (TLD) is not all digits.
     - `is_tailscale_ipv4(host)` returns `true` iff `host` is a strict IPv4 address in `100.64.0.0/10` (`first == 100 && (64..=127).contains(&second)`).
     - `is_private_mode_host(host)` returns `true` iff `is_valid_host(host)` and (`is_tailscale_ipv4(host)` or `host.ends_with(".ts.net")` or `host == "127.0.0.1"` or `host == "localhost"`).
     - For `v=1`, `host` must satisfy `is_private_mode_host(&host)`; for `v=2`, `host` must satisfy `is_valid_host(&host)`. Otherwise -> `Err(InviteError::Host)`.
   - **Port**:
     - On `v=1`, `port` is required. On `v=2`, `port` is optional and defaults to `443` (`DEFAULT_PUBLIC_PORT`), though if `port` is present as `""` it is rejected.
     - Must be base-10 digits `1..=5` characters long with no leading `'0'`, parsing to `1..=65535`. Otherwise -> `Err(InviteError::Port)`.
   - **Code**:
     - Trimmed with `js_trim`. Must be `4..=64` bytes of ASCII alphanumeric characters or `-` (`[A-Za-z0-9-]`). Otherwise -> `Err(InviteError::Code)`.
   - **Name**:
     - Trimmed with `js_trim`. Let `len = name.encode_utf16().count()`. Must satisfy `1..=64` UTF-16 code units and `!has_control_or_format(name)`. Otherwise -> `Err(InviteError::Name)`.
   - **Share (`v=1` optional)**:
     - If omitted or empty after `js_trim`, `share` is `None`.
     - Otherwise must have `share.encode_utf16().count() <= 512`, start with `"https://login.tailscale.com/"`, and after `"https://login.tailscale.com/"` contain only printable ASCII (`0x21..=0x7E`, no whitespace or control characters). Otherwise -> `Err(InviteError::Share)`.

4. **Formatting (`format_invite`)**:
   - Validates `invite.host` (after `js_trim` + `to_ascii_lowercase`), `invite.port >= 1`, `invite.code`, and `invite.name` with the same rules.
   - Selects scheme:
     - `GameVersion::new(1, 14, 7)` -> `"reforged-revival"`
     - `GameVersion::new(1, 4, 4)` -> `"dauntless-revived"` when feature `legacy` is enabled, or `Err(InviteError::Revived)` when disabled
     - Any other `GameVersion` -> `Err(InviteError::Version)`.
   - Percent-encodes query values using ECMAScript `encodeURIComponent` rules (unreserved: `A-Z`, `a-z`, `0-9`, `-`, `_`, `.`, `!`, `~`, `*`, `'`, `(`, `)` — all other bytes encoded as uppercase `%XX`).
   - For `ServerMode::Public`: requires `invite.fp` satisfying `is_valid_fingerprint`; emits `{scheme}://join?v=2&mode=public&host={host}&port={port}&fp={fp}&code={code}&name={name}`.
   - For `ServerMode::Private`: requires `is_private_mode_host(&host)`; emits `{scheme}://join?v=1&host={host}&port={port}&code={code}&name={name}` plus `&share={share}` if `invite.share` is `Some` (validated by the share rule).

---

## 5. Module `status` (`protocol::status`)

Sanitises untrusted JSON payloads from a server's `/status` endpoint into bounded, HTML/control-safe structures for the launcher UI.

### Public Signatures

```rust
pub const MAX_PLAYERS_LISTED: usize = 500;
pub const MAX_INSTANCES_LISTED: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "UPPERCASE")]
pub enum RegistrationMode {
    Open,
    Invitecode,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum PlayerWhere {
    City,
    Hunt,
    Dojo,
    Menu,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StatusPlayer {
    pub name: String,
    #[serde(rename = "where")]
    pub location: PlayerWhere,
    pub instance: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum InstanceKind {
    City,
    Hunt,
    Dojo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StatusInstance {
    pub id: String,
    pub kind: InstanceKind,
    pub title: String,
    pub map: Option<String>,
    pub behemoth: Option<String>,
    pub players: u32,
    pub max_players: u32,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameDataKind {
    Real,
    Synthetic,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StatusGame {
    pub build: String,
    pub changelist: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub name: String,
    pub online: bool,
    pub version: String,
    pub commit: String,
    pub source_url: Option<String>,
    pub registration: Option<RegistrationMode>,
    pub players_online: u32,
    pub players: Vec<StatusPlayer>,
    pub instances: Vec<StatusInstance>,
    pub uptime_seconds: u32,
    pub content_port: Option<u16>,
    pub limited: bool,
    pub game: Option<StatusGame>,
    pub game_data: GameDataKind,
    pub chat: bool,
}

pub fn parse_server_status(raw: &serde_json::Value) -> Option<ServerStatus>;
pub fn limited_view(status: ServerStatus) -> ServerStatus;
```

### Observable Behaviour

1. **String sanitisation and truncation**:
   - Every string field is cleaned via `strip_control_and_format` and truncated to at most `max_len` UTF-16 code units (if slicing at `max_len` UTF-16 code units lands inside a surrogate pair, the incomplete surrogate is omitted).
   - If the cleaned string is empty, it is treated as missing (`None`).

2. **Top-level fields (`parse_server_status`)**:
   - `raw` must be a JSON object (`Value::Object`); otherwise returns `None`.
   - `name`: required cleaned string (`max_len = 64`); if missing or empty -> returns `None`.
   - `online`: `raw["online"] == Value::Bool(true)`.
   - `version`: cleaned string (`max_len = 32`), default `"unknown"`.
   - `commit`: cleaned string (`max_len = 40`), default `"unknown"`.
   - `source_url`: optional cleaned string (`max_len = 256`) that must be a valid `https://` URL with host, no username/password, and only printable ASCII (`0x21..=0x7E`).
   - `registration`: `"OPEN"` -> `Some(Open)`, `"INVITECODE"` -> `Some(Invitecode)`, `"CLOSED"` -> `Some(Closed)`, else `None`.
   - `limited`: `raw["limited"] == Value::Bool(true)`.
   - When `limited` is `true`, `players_online = 0`, `players = vec![]`, `instances = vec![]` regardless of what the payload contains.
   - When `limited` is `false`:
     - `players`: up to `MAX_PLAYERS_LISTED` (`500`) valid entries from `raw["players"]` array. Each entry requires `name` (`max_len = 32`), `where` (`"city"` -> `City`, `"hunt"` -> `Hunt`, `"dojo"` -> `Dojo`, `"menu"` -> `Menu`, else `Unknown`), and optional `instance` (`max_len = 64`).
     - `instances`: up to `MAX_INSTANCES_LISTED` (`200`) valid entries from `raw["instances"]` array. Each entry requires `id` (`max_len = 64`), `kind` (`"city"` | `"hunt"` | `"dojo"` — entries with other kinds are skipped), `title` (`max_len = 80`, defaults to `id`), optional `map` (`max_len = 64`), optional `behemoth` (`max_len = 64`), `players` (clamped to `0..=1000`), `max_players` (`maxPlayers`, clamped to `0..=1000`), and optional `started_at` (`startedAt`, `max_len = 40`, matching ISO-8601 UTC `YYYY-MM-DDTHH:MM:SS(.mmm)Z` where month is `01..=12`, day is `01..=31`, hour is `00..=23`, minute is `00..=59`, second is `00..=60`).
     - `players_online`: `raw["playersOnline"]` clamped to `0..=10_000` (if missing or non-numeric, defaults to `players.len() as u32`).
   - `uptime_seconds`: `raw["uptimeSeconds"]` clamped to `0..=315_360_000` (defaults to `0`).
   - `content_port`: `raw["contentPort"]`: if missing or `null` or `<= 0` when rounded down -> `None`; otherwise clamped to `1..=65535`.
   - `game`: if `raw["game"]` is an object with a valid cleaned `build` string (`max_len = 64`), returns `Some(StatusGame { build, changelist })` where `changelist` is `raw["game"]["changelist"]` clamped to `0..=100_000_000` if numeric and `> 0`, else `None`.
   - `game_data`: `"real"` -> `Real`, `"synthetic"` -> `Synthetic`, else `Unknown`.
   - `chat`: `raw["chat"] == Value::Bool(true)`.
   - Note on numeric clamping: JSON numbers (`u64`, `i64`, finite `f64`) are truncated (`floor`) and clamped to `[0, max]`. Non-numbers or non-finite floats use the default.

3. **`limited_view(status)`**:
   - Returns `ServerStatus { limited: true, players_online: 0, players: Vec::new(), instances: Vec::new(), ..status }`.

---

## 6. Module `manifest` (`protocol::manifest`)

Defines the `/manifest/runtime.json` wire payload served by the content server and consumed by the launcher.

### Public Signatures

```rust
pub const RUNTIME_BUILD_NAME: &str = "rel-1.14.7-Archon";
pub const DEFAULT_CHANGELIST: u64 = 647_472;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub version: String,
    pub build: RuntimeBuild,
    pub files: Vec<RuntimeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBuild {
    pub name: String,
    pub changelist: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

impl RuntimeManifest {
    pub fn new(version: String, changelist: u64, files: Vec<RuntimeFile>) -> Self;
    pub fn to_json(&self) -> String;
}
```

### Observable Behaviour

- `RuntimeManifest::new(version, changelist, files)` sets `build.name = RUNTIME_BUILD_NAME.to_owned()` and `build.changelist = changelist`.
- `RuntimeManifest::to_json(&self)` serialises `self` as compact JSON (`{"version":...,"build":{"name":"rel-1.14.7-Archon","changelist":...},"files":[{"name":...,"size":...,"sha256":...},...]}`).

---

## 7. Test Expectations

The implementation must pass:
1. All unit and integration tests in `tests/invite.rs` (80 malformed invite vectors, v1/v2 round-trips, private-mode host rules, `dauntless-revived://` handling with and without `--features legacy`).
2. All golden fixture tests in `tests/manifest.rs` (`tests/fixtures/runtime-manifest*.json`).
3. All shared table tests in `tests/tables.rs` (`tests/cases/invite.json`, `tests/cases/status.json`, `tests/cases/username.json`) under both default features and `--features legacy`.
4. All username, key extraction, and hostile status tests in `tests/username_status.rs`.
