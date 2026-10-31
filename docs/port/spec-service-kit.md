# Clean-room Behaviour Specification: `service-kit` (`crates/service-kit`)

## 1. Purpose and Scope

`service-kit` (`rust/crates/service-kit`) provides shared HTTP server plumbing, Express-compatible request body parsing, ICU root collation, environment variable loading, JavaScript value/JSON semantics, Pino/line logging, IETF Draft-8 rate limiting, Express-compatible routing, and graceful server shutdown used across DaintyLass services.

Two of its fourteen source files (`src/context.rs` and `src/telemetry.rs`) were authored from scratch for DaintyLass (`DR3`) and remain untouched. This specification covers the twelve remaining source files and three integration test files so the entire `service-kit` crate is licensed under MIT:
- `src/lib.rs`
- `src/body.rs`
- `src/collate.rs`
- `src/env.rs`
- `src/js.rs`
- `src/jsjson.rs`
- `src/json.rs`
- `src/linelog.rs`
- `src/log.rs`
- `src/ratelimit.rs`
- `src/routing.rs`
- `src/serve.rs`
- `tests/body.rs`, `tests/ratelimit.rs`, `tests/routing.rs`

- **Package name**: `service-kit`
- **Edition**: `2024`
- **Licence**: `MIT` (`Copyright (c) 2026 Silent Spud`)

---

## 2. Crate Root (`src/lib.rs`)

```rust
pub mod body;
pub mod collate;
pub mod context;
pub mod env;
pub mod js;
pub mod jsjson;
pub mod json;
pub mod linelog;
pub mod log;
pub mod ratelimit;
pub mod routing;
pub mod serve;
pub mod telemetry;
```

---

## 3. Module `body` (`service_kit::body`)

Parses `application/json` and `application/x-www-form-urlencoded` request bodies before route matching, matching Express `body-parser` (`express.json({limit: "50mb"})` and `express.urlencoded({extended: true})`).

### Public Signatures

```rust
pub const DEFAULT_LIMIT: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyError {
    Malformed,
    TooLarge,
    TooManyParameters,
    UnsupportedCharset,
    UnsupportedEncoding,
    Aborted,
}

impl BodyError {
    pub fn status(self) -> axum::http::StatusCode;
}

impl axum::response::IntoResponse for BodyError { ... }

pub fn error_page(status: axum::http::StatusCode) -> axum::response::Response;

pub type OnBodyError =
    dyn Fn(&axum::http::request::Parts, BodyError) -> Option<axum::response::Response> + Send + Sync;

#[derive(Clone)]
pub struct BodyConfig {
    pub limit: usize,
    pub on_error: Arc<OnBodyError>,
}

impl Default for BodyConfig { ... }

#[derive(Debug, Clone, PartialEq)]
pub struct ExpressBody(pub Option<serde_json::Value>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ExpressBody {
    type Rejection = std::convert::Infallible;
    ...
}

pub async fn parse_bodies(
    axum::extract::State(config): axum::extract::State<BodyConfig>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response;

pub fn parse_json(text: &str) -> Result<serde_json::Value, BodyError>;
pub fn parse_form(text: &str) -> Result<serde_json::Value, BodyError>;
```

### Observable Behaviour

1. **`BodyError::status(self)`**:
   - `Malformed | Aborted` -> `StatusCode::BAD_REQUEST` (`400`)
   - `TooLarge | TooManyParameters` -> `StatusCode::PAYLOAD_TOO_LARGE` (`413`)
   - `UnsupportedCharset | UnsupportedEncoding` -> `StatusCode::UNSUPPORTED_MEDIA_TYPE` (`415`)
2. **`error_page(status)`**:
   - Body is `<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Error</title>\n</head>\n<body>\n<pre>{canonical_reason}</pre>\n</body>\n</html>\n`.
   - Sets headers `content-security-policy: default-src 'none'`, `x-content-type-options: nosniff`, and `content-type: text/html; charset=utf-8`.
3. **Media type and charset check**:
   - Inspects `Content-Type`:
     - `"application/json"` -> JSON
     - `"application/x-www-form-urlencoded"` -> Form
     - Missing or any other essence -> `ExpressBody(None)` is stored in extensions and the request passes through.
   - If a `charset` parameter is present on `Content-Type` and is not `"utf-8"` (case-insensitive, stripping optional quotes), returns `BodyError::UnsupportedCharset`.
4. **Compression & size limits**:
   - Reads raw body up to `config.limit` bytes (`LengthLimitError` -> `BodyError::TooLarge`; other stream error -> `BodyError::Aborted`).
   - Checks `Content-Encoding` (default `"identity"`):
     - `"identity"`: uses raw bytes.
     - `"gzip"` / `"deflate"`: inflates up to `config.limit` bytes (exceeding `config.limit` -> `BodyError::TooLarge`; corrupt stream -> `BodyError::Malformed`).
     - Any other encoding -> `BodyError::UnsupportedEncoding`.
   - Validates UTF-8 (`BodyError::Malformed` if not UTF-8) and re-inserts the raw bytes into the downstream `Request` body while storing `ExpressBody(Some(value))` in request extensions.
5. **`parse_json(text)`**:
   - Empty `text` (`""`) -> `Ok(Value::Object(Map::new()))`.
   - In strict mode, the first non-whitespace character (after stripping leading `' '`, `'\t'`, `'\n'`, `'\r'`) must be `'{'` or `'['`; otherwise `Err(BodyError::Malformed)`.
   - Parses with `serde_json::from_str(text)` (`Err(BodyError::Malformed)` on error).
6. **`parse_form(text)`**:
   - Splits `text` on `'&'`, ignoring empty segments. If more than `1000` (`PARAMETER_LIMIT`) non-empty pairs exist -> `Err(BodyError::TooManyParameters)`.
   - Splits each pair on `'='` (missing `'='` means value `""`).
   - Replaces `'+'` with `' '` and percent-decodes valid UTF-8 sequences; if percent-decoding produces invalid UTF-8, keeps the `+`-spaced raw string as-is.
   - Empty keys are skipped. Repeated keys accumulate into a `Value::Array`.

---

## 4. Module `collate` (`service_kit::collate`)

Implements ICU root collation (`String.prototype.localeCompare` with default locale) on printable ASCII strings.

### Public Signatures

```rust
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering;
```

### Observable Behaviour

- Ignores ASCII control characters (`is_ascii_control()`).
- Primary collation order for printable ASCII (case-folded) is:
  `" _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz"`
  (Characters outside this table sort after `'z'` by Unicode scalar value).
- When primary weights of `a` and `b` are equal, compares case at the tertiary level: lowercase sorts before uppercase at the first character position where case differs.

---

## 5. Module `env` (`service_kit::env`)

Loads configuration variables from an explicit `.env` file overridden by the process environment (`std::env::vars()`).

### Public Signatures

```rust
#[derive(Debug, Clone, Default)]
pub struct Env { /* private fields */ }

#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("{path}: {message}")]
    File { path: String, message: String },
    #[error("{name}: {message}")]
    Value { name: String, message: String },
}

impl Env {
    pub fn load(file: Option<&Path>) -> Result<Self, EnvError>;
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self;
    pub fn strip_prefix(&self, prefix: &str) -> Self;
    pub fn vars(&self) -> impl Iterator<Item = (&str, &str)>;
    pub fn raw(&self, name: &str) -> Option<&str>;
    pub fn get(&self, name: &str) -> Option<&str>;
    pub fn parse<T>(&self, name: &str) -> Result<Option<T>, EnvError>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display;
    pub fn flag(&self, name: &str, default: bool) -> Result<bool, EnvError>;
}
```

### Observable Behaviour

- `Env::load(file)`: if `Some(path)`, loads key-value pairs via `dotenvy::from_path_iter(path)` (mapping errors to `EnvError::File`), then overlays `std::env::vars()`.
- `strip_prefix(prefix)`: returns a new `Env` containing only variables whose names started with `prefix`, with `prefix` stripped.
- `raw(name)`: returns `Some(val)` even if `val` is `""`.
- `get(name)`: returns `Some(val)` only when `val` is present and non-empty (`!val.is_empty()`).
- `parse::<T>(name)`: parses `self.get(name)` after `.trim()`, returning `Ok(None)` when unset/empty or `Err(EnvError::Value { name, message })` on parse failure.
- `flag(name, default)`: returns `Ok(default)` when `get(name)` is `None`, `Ok(true)` for `"1"`, `Ok(false)` for `"0"`, and `Err(EnvError::Value { name, message: format!("{other:?} is not 0 or 1") })` for anything else.

---

## 6. Module `js` (`service_kit::js`)

Implements ECMAScript value coercions and string/date helpers on `Option<&serde_json::Value>` (where `None` represents `undefined`).

### Public Signatures

```rust
pub fn number(value: Option<&Value>) -> f64;
pub fn string(value: Option<&Value>) -> String;
pub fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool;
pub fn is_nullish(value: Option<&Value>) -> bool;
pub fn truthy(value: Option<&Value>) -> bool;
pub fn is_space(c: char) -> bool;
pub fn trim(text: &str) -> &str;
pub fn is_number(value: Option<&Value>) -> bool;
pub fn safe_integer(value: Option<&Value>) -> Option<i64>;
pub fn date_parse(text: &str) -> Option<i64>;
pub fn decode_uri_component(text: &str) -> Option<String>;
```

### Observable Behaviour

- `number`: `None` -> `NaN`; `Null` -> `0.0`; `Bool(b)` -> `1.0`/`0.0`; `Number(n)` -> `n.as_f64()`; `String(s)` -> trimmed (`""` -> `0.0`, `"0x..."`/`"0X..."` -> hex integer, `"Infinity"`/`"+Infinity"`/`"-Infinity"`, decimal float, else `NaN`); `Array([])` -> `0.0`, `Array([one])` -> `number(Some(String(string(Some(one)))))`, longer arrays or `Object` -> `NaN`.
- `string`: `None` -> `"undefined"`; `Null` -> `"null"`; `Bool(b)` -> `"true"`/`"false"`; `Number(n)` -> `jsjson::number(...)`; `String(s)` -> `s.clone()`; `Array(items)` -> comma-joined `string` of elements (where ` null ` elements become `""`); `Object(_)` -> `"[object Object]"`.
- `strict_eq(a, b)`: primitives compared by value (`None == None`, `Null == Null`, `Bool`, `Number` as `f64`, `String`); objects/arrays always return `false`.
- `is_nullish`: `matches!(value, None | Some(Value::Null))`.
- `truthy`: `false` for `None`, `Null`, `Bool(false)`, `Number(0.0 | NaN)`, `String("")`; `true` otherwise.
- `is_space(c)`: `(c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'`.
- `trim(text)`: `text.trim_matches(is_space)`.
- `is_number`: `matches!(value, Some(Value::Number(_)))`.
- `safe_integer`: if `value` is a `Value::Number` whose `f64` has `.fract() == 0.0` and `abs() <= 9_007_199_254_740_991.0`, returns `Some(n as i64)`.
- `date_parse(text)`: parses RFC 3339, RFC 2822, local ISO-8601 (`%Y-%m-%dT%H:%M:%S%.f`, `%Y-%m-%dT%H:%M:%S`, `%Y-%m-%dT%H:%M` in `chrono::Local`), or bare UTC date (`%Y-%m-%d` at `00:00:00Z`), returning Unix epoch milliseconds (`Option<i64>`).
- `decode_uri_component(text)`: decodes `%XX` hex escapes and validates UTF-8, returning `None` if any `%` is not followed by two ASCII hex digits or if the resulting bytes are not valid UTF-8.

---

## 7. Module `jsjson` (`service_kit::jsjson`)

Serialises and parses JSON values matching V8's `JSON.stringify` and `JSON.parse` (including ECMAScript own-property key ordering where canonical array-index keys `0..2^32-1` sort ascending before string keys in insertion order, and ECMAScript `Number.prototype.toString` formatting).

### Public Signatures

```rust
pub fn stringify(value: &Value) -> String;
pub fn parse(text: &str) -> Result<Value, serde_json::Error>;
pub fn to_js(value: &Value) -> Value;
pub fn number(value: f64) -> String;
pub fn deep_equal(a: &Value, b: &Value) -> bool;
```

### Observable Behaviour

- `stringify(value)`: formats `value` as compact JSON where object keys are ordered by `js_order` (canonical `u32` array indices `< u32::MAX` ascending first, then remaining keys in insertion order), control characters `< 0x20` are escaped (`\b`, `\f`, `\n`, `\r`, `\t`, `\uXXXX`), and numbers are formatted with `number(f64)`.
- `parse(text)`: parses `text` with `serde_json::from_str` and transforms the result with `to_js(&value)` (converting numbers through `f64` and ordering object keys).
- `number(value)`: returns `"null"` for non-finite numbers, `"0"` for `0.0` / `-0.0`, fixed-point notation when `1e-6 <= |value| < 1e21`, and scientific notation (`1e+21`, `1.5e-7`) outside that range.
- `deep_equal(a, b)`: matches Node's `util.isDeepStrictEqual` on JSON values (distinguishing `+0.0` from `-0.0`, equating `NaN` with `NaN`, and comparing objects independent of key order).

---

## 8. Module `json` (`service_kit::json`)

Provides an Axum `Json<T>` response wrapper using `jsjson::stringify` and `application/json; charset=utf-8`, serde number helpers, and `not_found`.

### Public Signatures

```rust
pub const JSON_UTF8: &str = "application/json; charset=utf-8";

pub struct Json<T>(pub T);

impl<T: serde::Serialize> axum::response::IntoResponse for Json<T> { ... }

pub fn js_number<S: serde::Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>;
pub fn js_number_opt<S: serde::Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error>;
pub fn not_found(_request: &axum::extract::Request) -> axum::response::Response;
```

### Observable Behaviour

- `Json<T>::into_response`: converts `self.0` to `serde_json::Value`, serialises with `jsjson::stringify`, and sets `content-type: application/json; charset=utf-8` (or `500 Internal Server Error` if serialisation fails).
- `js_number` / `js_number_opt`: serialises whole numbers `< 1e21` without `.0` (`serialize_i64` / `serialize_i128`), non-finite numbers as `null`, and fractional numbers as `f64`.
- `not_found(_request)`: returns `StatusCode::NOT_FOUND.into_response()` (`404` with empty body).

---

## 9. Module `linelog` (`service_kit::linelog`)

Writes structured single-line JSON logs (`{t, level, msg, ...fields}`) for the gateway and content server, with optional test sink capture, tracing bridge forwarding, and per-key log throttling.

### Public Signatures

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 10,
    Info = 20,
    Warn = 30,
    Error = 40,
}

pub type Sink = Box<dyn Fn(&str, Level) + Send + Sync>;

pub fn set_sink(sink: Option<Sink>);
pub fn set_level(level: Level);
pub fn log(level: Level, msg: &str, fields: Value);
pub fn info(msg: &str, fields: Value);
pub fn warn(msg: &str, fields: Value);
pub fn error(msg: &str, fields: Value);
pub fn debug(msg: &str, fields: Value);

pub struct Throttle { /* private fields */ }

impl Throttle {
    pub fn new(interval_ms: i64) -> Self;
    pub fn allow(&self, key: &str, now: i64) -> Option<u64>;
}
```

### Observable Behaviour

- Default threshold is initialised from `LOG_LEVEL` (`debug`, `warn`, `error`, else `info`).
- `log(level, msg, fields)`: ignores events below threshold. Builds a JSON object with `"t"` (ISO-8601 UTC millisecond timestamp `%Y-%m-%dT%H:%M:%S%.3fZ`), `"level"` (`"debug"` | `"info"` | `"warn"` | `"error"`), `"msg"`, and any entries from `fields` if `fields` is a `Value::Object`, formatted via `jsjson::stringify`.
  - If a custom `sink` is installed via `set_sink`, invokes `sink(&text, level)`.
  - Else if `crate::telemetry::tracing_bridge_active()` is `true`, emits a `tracing` event at the corresponding level (`"{msg} ({text})"`).
  - Else writes `{text}\n` to `stderr` for `Warn`/`Error` or `stdout` for `Debug`/`Info`.
- `Throttle::new(interval_ms)`: tracks up to `10_000` keys. `allow(key, now)` returns `None` (and increments the suppressed count for `key`) if fewer than `interval_ms` milliseconds have elapsed since `key` was last allowed; otherwise resets `key`'s timestamp to `now` and returns `Some(suppressed_count)`.

---

## 10. Module `log` (`service_kit::log`)

Provides a Pino-compatible JSON `tracing_subscriber::Layer` (`PinoLayer`) and the `fatal!` macro.

### Public Signatures

```rust
#[macro_export]
macro_rules! fatal {
    ($($arg:tt)+) => { ::tracing::error!(fatal = true, $($arg)+) };
}

pub fn parse_level(text: Option<&str>) -> Result<tracing_subscriber::filter::LevelFilter, String>;
pub fn init(level: tracing_subscriber::filter::LevelFilter, pretty: bool);

pub struct PinoLayer<W> { /* private fields */ }

impl<W> PinoLayer<W> {
    pub fn new(make: W) -> Self;
}

impl<S, W, O> tracing_subscriber::Layer<S> for PinoLayer<W>
where
    S: tracing::Subscriber,
    W: Fn() -> O + 'static,
    O: std::io::Write,
{ ... }
```

### Observable Behaviour

- `parse_level`: maps `None | Some("") | Some("info")` -> `INFO`, `"trace"` -> `TRACE`, `"debug"` -> `DEBUG`, `"warn"` -> `WARN`, `"error" | "fatal"` -> `ERROR`, `"silent"` -> `OFF`, and returns `Err` for any other string.
- `PinoLayer`: formats each `tracing::Event` as one JSON line with keys in Pino order: `"level"` (`10` trace, `20` debug, `30` info, `40` warn, `50` error, `60` error with `fatal = true` where the `fatal` field itself is omitted from output), `"time"` (epoch ms `u64`), `"pid"` (`u32`), `"hostname"` (`String`), recorded event fields (with `std::error::Error` values serialised as `{"type": "Error", "message": "..."}`), and `"msg"`.

---

## 11. Module `ratelimit` (`service_kit::ratelimit`)

Implements `express-rate-limit` v8 in-memory per-key rate limiting with IETF draft-8 `RateLimit` and `RateLimit-Policy` headers.

### Public Signatures

```rust
pub struct Limiter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub remaining: u64,
    pub reset_secs: u64,
    pub limited: bool,
}

impl Limiter {
    pub fn new(limit: u64, window: Duration, reply: serde_json::Value) -> Self;
    pub fn skip_successful(self) -> Self;
    pub fn name(&self) -> &str;
    pub fn hit(&self, key: &str, now: Instant) -> Decision;
    pub fn undo(&self, key: &str, decision: &Decision, now: Instant);
    pub fn sweep(&self, now: Instant);
    pub fn spawn_sweeper(self: &Arc<Self>) -> tokio::task::JoinHandle<()>;
    pub fn headers(&self, key: &str, decision: &Decision) -> [(axum::http::header::HeaderName, String); 2];
}

pub fn ip_key(address: &str) -> String;

#[derive(Clone)]
pub struct Limited;

impl Limited {
    pub fn new(
        limiter: Arc<Limiter>,
        key: impl Fn(&axum::extract::Request) -> Option<String> + Send + Sync + 'static,
    ) -> Self;
}

pub async fn enforce(
    axum::extract::State(limited): axum::extract::State<Limited>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response;

pub async fn around(
    limiter: &Limiter,
    key: Option<&str>,
    handler: impl std::future::Future<Output = axum::response::Response>,
) -> axum::response::Response;
```

### Observable Behaviour

- Policy name (`Limiter::name`): `"{limit}-in-{window_name}"` where `window_name` is `"{s}sec"` (`< 60s`), `"{m}min"` (`< 60min`), `"{h}hr"` / `"{h}hrs"` (`< 24h`), or `"{d}day"` / `"{d}days"`.
- `Limiter::hit(key, now)` increments the hit counter for `key` within its active window (or starts a new window of duration `window` if none exists or the previous window expired at or before `now`). `remaining` is `limit.saturating_sub(count)`, `limited` is `count > limit`, and `reset_secs` is the ceiling in whole seconds (`>= 1`) until the window expires.
- `Limiter::undo(key, decision, now)` decrements the hit count for `key` if the window recorded when `decision` was issued is still the active window at `now` (removing the entry if its count reaches zero); if the window has already expired at or before `now`, `undo` is a no-op.
- `Limiter::sweep(now)` removes all entries whose window has expired at or before `now`. `spawn_sweeper` runs a background task that calls `sweep` once per window period until the `Limiter` is dropped.
- `ip_key(address)`: returns IPv4 addresses (including IPv4-mapped `::ffff:a.b.c.d` and IPv4-compatible `::a.b.c.d`) as dotted-quad IPv4 strings, IPv6 addresses masked to their `/56` prefix as `"{network}/56"`, and any non-IP string unchanged.
- Partition key `pk`: standard base64 of the first 12 ASCII hex characters of `SHA-256(key)`.
- `headers(key, decision)`:
  - `("ratelimit", "\"{name}\"; r={remaining}; t={reset_secs}")`
  - `("ratelimit-policy", "\"{name}\"; q={limit}; w={window_secs}; pk=:{pk}:")`
- `around` / `enforce`: if `key` is `None`, runs `handler` uncounted. Otherwise calls `limiter.hit(key, Instant::now())`. When `decision.limited` is `true`, returns `429 Too Many Requests` with `retry-after: {reset_secs}` and `Json(&limiter.reply)` without running `handler`. When not limited, runs `handler`, and if `skip_successful` is set and `response.status().as_u16() < 400`, calls `limiter.undo(key, &decision, Instant::now())`. Always appends both `ratelimit` and `ratelimit-policy` headers.

---

## 12. Module `routing` (`service_kit::routing`)

Implements an Express-compatible case-insensitive router that matches routes in registration order, ignores a single trailing slash, answers `HEAD` using `GET` routes, and percent-decodes `:name` parameters.

### Public Signatures

```rust
#[derive(Debug, Clone)]
pub struct Pattern;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadParam(pub String);

impl Pattern {
    pub fn parse(path: &str) -> Self;
    pub fn as_str(&self) -> &str;
    pub fn matches(&self, path: &str) -> Option<Result<Params, BadParam>>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params;

impl Params {
    pub fn get(&self, name: &str) -> Option<&str>;
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Params {
    type Rejection = std::convert::Infallible;
    ...
}

#[derive(Debug, Clone)]
pub struct MatchedPattern(pub Arc<str>);

pub struct Router<S = ()>;

impl<S: Clone + Send + Sync + 'static> Default for Router<S> { ... }

impl<S: Clone + Send + Sync + 'static> Router<S> {
    pub fn new() -> Self;
    pub fn route<H, T>(self, method: axum::http::Method, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, S>,
        T: 'static;
    pub fn get<H: axum::handler::Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self;
    pub fn post<H: axum::handler::Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self;
    pub fn put<H: axum::handler::Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self;
    pub fn patch<H: axum::handler::Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self;
    pub fn delete<H: axum::handler::Handler<T, S>, T: 'static>(self, path: &str, handler: H) -> Self;
    pub fn layer<L>(self, layer: L) -> Self
    where
        L: tower::Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<axum::http::Request<axum::body::Body>> + Clone + Send + Sync + 'static,
        <L::Service as tower::Service<axum::http::Request<axum::body::Body>>>::Response:
            axum::response::IntoResponse + 'static,
        <L::Service as tower::Service<axum::http::Request<axum::body::Body>>>::Error:
            Into<std::convert::Infallible> + 'static,
        <L::Service as tower::Service<axum::http::Request<axum::body::Body>>>::Future: Send + 'static;
    pub fn merge(self, other: Router<S>) -> Self;
    pub fn table(&self) -> Vec<(axum::http::Method, String)>;
    pub fn into_service(
        self,
        state: S,
        fallback: fn(&axum::http::Request<axum::body::Body>) -> axum::response::Response,
    ) -> RouterService;
}

#[derive(Clone)]
pub struct RouterService;

impl tower::Service<axum::http::Request<axum::body::Body>> for RouterService { ... }
```

### Observable Behaviour

- `Pattern::parse(path)` panics at build time if `path` does not start with `'/'`, contains empty segments (`"//"`), has an unnamed parameter (`"/:"`), or contains wildcard/regex characters (`'*'`, `'{'`, `'}'`, `'?'`, `'('`, `')'`).
- `Pattern::matches(path)` strips at most one trailing `'/'` (unless `path == "/"`), compares literal segments ASCII case-insensitively, requires parameter segments to be non-empty, and percent-decodes parameter values as UTF-8 (`Some(Err(BadParam(raw)))` if invalid UTF-8).
- `RouterService::call`: checks registered routes in registration order. A route matches if the registered HTTP method equals the request's method (or the request is `HEAD` and the registered method is `GET`) and `Pattern::matches(path)` returns `Some`.
  - On `Some(Err(_))`, returns `StatusCode::BAD_REQUEST.into_response()` (`400`).
  - On `Some(Ok(params))`, inserts `params` and `MatchedPattern` (holding the route's pattern text) into `request.extensions_mut()` and invokes the route's service (stripping the response body when a `HEAD` request is answered by a `GET` route).
  - If no route matches, invokes `fallback(&request)`.

---

## 13. Module `serve` (`service_kit::serve`)

Serves an Axum `Router` over a `TCP_NODELAY` listener with connection peer/local address extraction (`ConnectionInfo`), stopping on stdin `"shutdown"`, `Ctrl+C`, or `SIGTERM`/`Ctrl+Break` with a bounded drain timeout.

### Public Signatures

```rust
#[derive(Debug, Clone, Copy)]
pub struct ConnectionInfo {
    pub peer: std::net::SocketAddr,
    pub local: std::net::SocketAddr,
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, tokio::net::TcpListener>>
    for ConnectionInfo
{ ... }

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, NoDelay>>
    for ConnectionInfo
{ ... }

pub struct NoDelay(pub tokio::net::TcpListener);

impl axum::serve::Listener for NoDelay { ... }

pub async fn stop_requested();

pub async fn serve(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
    drain: std::time::Duration,
) -> std::io::Result<()>;
```

---

## 14. Test Expectations

The implementation must pass:
1. All unit tests in `body`, `collate`, `env`, `js`, `jsjson`, `linelog`, `log`, `ratelimit`, and `routing`.
2. Integration tests in `tests/body.rs`, `tests/ratelimit.rs`, and `tests/routing.rs`.
3. All downstream tests across the workspace (`cargo test --workspace`).
