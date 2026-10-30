# Clean-room Behaviour Specification: `proxy` (`crates/proxy`)

## 1. Purpose and Scope

`proxy` (`rust/crates/proxy`) provides RFC 9110 hop-by-hop HTTP header filtering and idle-timed bidirectional stream tunnelling shared by the server TLS gateway and the launcher local proxy.

- **Package name**: `proxy`
- **Edition**: `2024`
- **Licence**: `MIT` (`Copyright (c) 2026 Silent Spud`)
- **Normal dependencies**:
  - `http`
  - `tokio` (`io-util`, `net`, `time`)

---

## 2. Public Interface (`src/lib.rs`)

```rust
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use http::HeaderMap;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Sleep;

pub const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub fn is_hop_by_hop(name: &str) -> bool;
pub fn connection_tokens(headers: &HeaderMap) -> BTreeSet<String>;
pub fn end_to_end(headers: &HeaderMap, drop: impl Fn(&str) -> bool) -> HeaderMap;

pub struct Idle<S> { /* private fields */ }

impl<S> Idle<S> {
    pub fn new(inner: S, limit: Duration) -> Self;
}

impl<S: AsyncRead + Unpin> AsyncRead for Idle<S> { ... }
impl<S: AsyncWrite + Unpin> AsyncWrite for Idle<S> { ... }

pub async fn tunnel<A, B>(a: A, b: B, idle: Duration) -> (u64, u64)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin;
```

---

## 3. Observable Behaviour

### 3.1 Hop-by-hop Header Filtering (`is_hop_by_hop`, `connection_tokens`, `end_to_end`)

1. **`is_hop_by_hop(name)`**:
   - Returns `true` if `name` matches any entry in `HOP_BY_HOP` using ASCII case-insensitive comparison (`eq_ignore_ascii_case`).

2. **`connection_tokens(headers)`**:
   - Inspects every `connection` (`http::header::CONNECTION`) header value in `headers`.
   - Values that are valid UTF-8 are split on `','`; each comma-separated part is trimmed of surrounding whitespace and converted to ASCII lowercase (`to_ascii_lowercase`).
   - Empty tokens are ignored; non-empty tokens are returned in order.

3. **`end_to_end(headers, extra)`**:
   - Collects `connection_tokens(headers)`.
   - Returns an iterator over `(name, value)` pairs in `headers` excluding any header whose name:
     - satisfies `is_hop_by_hop(name.as_str())`, or
     - matches any entry in `extra` (`eq_ignore_ascii_case`), or
     - matches any token returned by `connection_tokens(headers)` (`eq_ignore_ascii_case`).

### 3.2 Idle-timed Stream (`Idle<S>`) and `tunnel`

1. **`Idle<S>`**:
   - Wraps an async stream `inner: S` with an inactivity timeout `limit: Duration`.
   - `AsyncRead::poll_read`:
     - Records the buffer's `filled().len()` before polling `inner`.
     - Polls `inner`.
     - If `Poll::Ready(Ok(()))` and `buf.filled().len() > before` (at least one byte was read), resets the sleep deadline to `Instant::now() + limit` and returns `Poll::Ready(Ok(()))`.
     - If `Poll::Ready(Ok(()))` with 0 new bytes (EOF) or `Poll::Ready(Err(_))`, returns that result immediately.
     - If `Poll::Pending`, polls the sleep timer: if the timer has elapsed (`Poll::Ready(())`), returns `Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "idle stream timed out")))`; otherwise returns `Poll::Pending`.
   - `AsyncWrite`:
     - `poll_write`: polls `inner`. When `Poll::Ready(Ok(n))` with `n > 0`, resets the sleep deadline to `Instant::now() + limit`. When `Poll::Pending`, polls the sleep timer and returns `TimedOut` (`"idle stream timed out"`) if elapsed.
     - `poll_flush` and `poll_shutdown`: delegate directly to `inner`.

2. **`tunnel(a, b, idle)`**:
   - Wraps both `a` and `b` in `Idle::new(..., idle)` and copies bytes bidirectionally (`tokio::io::copy_bidirectional`), returning the `(a_to_b, b_to_a)` byte counts or the first I/O error (including `io::ErrorKind::TimedOut` if either side goes idle for `idle`).

---

## 4. Test Expectations

Unit tests must verify:
1. `end_to_end` drops standard hop-by-hop headers (`connection`, `keep-alive`, `transfer-encoding`, `te`, etc.), headers named dynamically inside `Connection` (e.g. `Connection: x-hop, keep-alive, X-Second` drops `x-hop` and `x-second`), and caller-supplied `extra` headers (`host`), while preserving end-to-end headers (`x-kept`).
2. `tunnel` copies data in both directions when active and fails with `io::ErrorKind::TimedOut` when neither side transfers bytes within `idle`.
